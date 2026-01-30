use crate::{chunked::ChunkedUpload, db::FileRecord, AppState};
use axum::{
    body::Body,
    extract::{Multipart, Path, State},
    http::{header, HeaderMap, StatusCode},
    response::{Html, IntoResponse, Response},
    Json,
};
use spacetaxi_shared::{
    ChunkUploadResponse, ChunkedUploadInitRequest, ChunkedUploadInitResponse, ChunkedUploadStatus,
    FileMeta, UploadResponse,
};
use std::sync::Arc;

// Include the decrypt.js at compile time
const DECRYPT_JS: &str = include_str!("../../web/dist/decrypt.js");

fn generate_id() -> String {
    use rand::Rng;
    let chars: Vec<char> = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789"
        .chars()
        .collect();
    let mut rng = rand::thread_rng();
    (0..8).map(|_| chars[rng.gen_range(0..chars.len())]).collect()
}

fn generate_token() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn parse_headers(headers: &HeaderMap) -> (bool, Option<u32>, Option<i64>, bool, String) {
    let one_time = headers
        .get("X-One-Time")
        .and_then(|v| v.to_str().ok())
        .map(|v| v == "true")
        .unwrap_or(false);

    let max_downloads = headers
        .get("X-Max-Downloads")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok());

    let expires_at = headers
        .get("X-Expires")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok());

    let has_password = headers
        .get("X-Has-Password")
        .and_then(|v| v.to_str().ok())
        .map(|v| v == "true")
        .unwrap_or(false);

    let filename = headers
        .get("X-Filename")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| {
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, v)
                .ok()
                .and_then(|bytes| String::from_utf8(bytes).ok())
        })
        .unwrap_or_else(|| "file".to_string());

    (one_time, max_downloads, expires_at, has_password, filename)
}

/// POST /upload - Simple file upload
pub async fn upload(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Result<Json<UploadResponse>, (StatusCode, String)> {
    let (one_time, max_downloads, expires_at, has_password, filename) = parse_headers(&headers);

    // Get the file from multipart
    let field = multipart
        .next_field()
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?
        .ok_or((StatusCode::BAD_REQUEST, "No file provided".to_string()))?;

    let data = field
        .bytes()
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    // Check file size
    if data.len() as u64 > state.config.max_file_size {
        return Err((StatusCode::PAYLOAD_TOO_LARGE, "File too large".to_string()));
    }

    let id = generate_id();
    let delete_token = generate_token();

    // Save file
    state
        .storage
        .save(&id, &data)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Save metadata
    let record = FileRecord {
        id: id.clone(),
        filename,
        file_size: data.len() as i64,
        created_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64,
        expires_at,
        download_count: 0,
        max_downloads: max_downloads.map(|m| m as i32),
        one_time,
        has_password,
        delete_token: delete_token.clone(),
        is_chunked: false,
        chunk_count: None,
    };

    state
        .db
        .insert_file(&record)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    tracing::info!("Uploaded file: {} ({} bytes)", id, data.len());

    Ok(Json(UploadResponse { id, delete_token }))
}

/// POST /upload/init - Initialize chunked upload
pub async fn chunked_init(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<ChunkedUploadInitRequest>,
) -> Result<Json<ChunkedUploadInitResponse>, (StatusCode, String)> {
    let (one_time, max_downloads, expires_at, has_password, _) = parse_headers(&headers);

    // Check file size
    if req.size > state.config.max_file_size {
        return Err((StatusCode::PAYLOAD_TOO_LARGE, "File too large".to_string()));
    }

    let upload_id = generate_token();
    let chunks_dir = state.config.data_dir.join("chunks");

    let upload = ChunkedUpload::new(
        upload_id.clone(),
        req.filename,
        req.size,
        req.chunk_size,
        chunks_dir,
        one_time,
        max_downloads,
        expires_at,
        has_password,
    );

    // Create chunk directory
    upload
        .storage
        .create_upload_dir(&upload_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    state.chunked_uploads.write().await.insert(upload_id.clone(), upload);

    tracing::info!("Initialized chunked upload: {}", upload_id);

    Ok(Json(ChunkedUploadInitResponse { upload_id }))
}

/// PUT /upload/{upload_id}/chunk/{chunk_num}
pub async fn chunked_upload_chunk(
    State(state): State<Arc<AppState>>,
    Path((upload_id, chunk_num)): Path<(String, u64)>,
    body: axum::body::Bytes,
) -> Result<Json<ChunkUploadResponse>, (StatusCode, String)> {
    let mut uploads = state.chunked_uploads.write().await;
    let upload = uploads
        .get_mut(&upload_id)
        .ok_or((StatusCode::NOT_FOUND, "Upload not found".to_string()))?;

    if chunk_num >= upload.total_chunks {
        return Err((StatusCode::BAD_REQUEST, "Invalid chunk number".to_string()));
    }

    upload
        .save_chunk(chunk_num, &body)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    tracing::debug!("Received chunk {} for upload {}", chunk_num, upload_id);

    Ok(Json(ChunkUploadResponse {
        received: body.len() as u64,
    }))
}

/// GET /upload/{upload_id}/status
pub async fn chunked_status(
    State(state): State<Arc<AppState>>,
    Path(upload_id): Path<String>,
) -> Result<Json<ChunkedUploadStatus>, (StatusCode, String)> {
    let uploads = state.chunked_uploads.read().await;
    let upload = uploads
        .get(&upload_id)
        .ok_or((StatusCode::NOT_FOUND, "Upload not found".to_string()))?;

    Ok(Json(ChunkedUploadStatus {
        chunks_received: upload.get_received_chunks(),
        total_chunks: upload.total_chunks,
    }))
}

/// POST /upload/{upload_id}/complete
pub async fn chunked_complete(
    State(state): State<Arc<AppState>>,
    Path(upload_id): Path<String>,
) -> Result<Json<UploadResponse>, (StatusCode, String)> {
    let mut uploads = state.chunked_uploads.write().await;
    let upload = uploads
        .remove(&upload_id)
        .ok_or((StatusCode::NOT_FOUND, "Upload not found".to_string()))?;

    if !upload.is_complete() {
        uploads.insert(upload_id.clone(), upload);
        return Err((StatusCode::BAD_REQUEST, "Upload incomplete".to_string()));
    }

    let id = generate_id();
    let delete_token = generate_token();

    // Assemble chunks into final file
    let file_size = upload
        .storage
        .assemble_chunks(&upload_id, upload.total_chunks, &state.storage, &id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Cleanup chunks
    let _ = upload.cleanup().await;

    // Save metadata
    let record = FileRecord {
        id: id.clone(),
        filename: upload.filename,
        file_size: file_size as i64,
        created_at: upload.created_at,
        expires_at: upload.expires_at,
        download_count: 0,
        max_downloads: upload.max_downloads.map(|m| m as i32),
        one_time: upload.one_time,
        has_password: upload.has_password,
        delete_token: delete_token.clone(),
        is_chunked: true,
        chunk_count: Some(upload.total_chunks as i32),
    };

    state
        .db
        .insert_file(&record)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    tracing::info!("Completed chunked upload: {} -> {} ({} bytes)", upload_id, id, file_size);

    Ok(Json(UploadResponse { id, delete_token }))
}

/// GET /{id} - Download page
pub async fn download_page(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Html<String>, (StatusCode, String)> {
    // Check if file exists and is valid
    let record = state
        .db
        .check_file_valid(&id)
        .await
        .map_err(|e| match e {
            crate::db::DbError::NotFound => (StatusCode::NOT_FOUND, "File not found".to_string()),
            crate::db::DbError::Expired => (StatusCode::GONE, "File expired or download limit reached".to_string()),
            _ => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        })?;

    // Increment download count
    state
        .db
        .increment_download_count(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Generate HTML page
    let html = format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>spacetaxi - Download {filename}</title>
    <style>
        * {{ margin: 0; padding: 0; box-sizing: border-box; }}
        body {{
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
            background: linear-gradient(135deg, #1a1a2e 0%, #16213e 100%);
            min-height: 100vh;
            display: flex;
            align-items: center;
            justify-content: center;
            color: #fff;
        }}
        .container {{
            background: rgba(255, 255, 255, 0.05);
            backdrop-filter: blur(10px);
            border-radius: 16px;
            padding: 2rem;
            max-width: 400px;
            width: 90%;
            text-align: center;
            border: 1px solid rgba(255, 255, 255, 0.1);
        }}
        .logo {{ font-size: 2rem; margin-bottom: 1rem; }}
        h1 {{ font-size: 1.5rem; margin-bottom: 0.5rem; word-break: break-all; }}
        .size {{ color: #888; margin-bottom: 1.5rem; }}
        .progress-container {{
            background: rgba(255, 255, 255, 0.1);
            border-radius: 8px;
            overflow: hidden;
            margin-bottom: 1rem;
            display: none;
        }}
        .progress {{
            height: 8px;
            background: linear-gradient(90deg, #00d9ff, #00ff88);
            width: 0%;
            transition: width 0.3s;
        }}
        .status {{ color: #888; margin-bottom: 1.5rem; }}
        .password-form {{
            margin-bottom: 1.5rem;
            display: none;
        }}
        .password-form input {{
            width: 100%;
            padding: 0.75rem;
            border-radius: 8px;
            border: 1px solid rgba(255, 255, 255, 0.2);
            background: rgba(255, 255, 255, 0.1);
            color: #fff;
            margin-bottom: 0.5rem;
        }}
        .password-form input::placeholder {{ color: #888; }}
        button {{
            background: linear-gradient(90deg, #00d9ff, #00ff88);
            border: none;
            padding: 0.75rem 2rem;
            border-radius: 8px;
            color: #1a1a2e;
            font-weight: bold;
            cursor: pointer;
            transition: transform 0.2s;
        }}
        button:hover {{ transform: scale(1.05); }}
        button:disabled {{
            opacity: 0.5;
            cursor: not-allowed;
            transform: none;
        }}
        .error {{
            color: #ff6b6b;
            margin-top: 1rem;
            display: none;
        }}
    </style>
</head>
<body>
    <div class="container">
        <div class="logo">🚀</div>
        <h1>{filename}</h1>
        <p class="size">{size}</p>
        <div class="password-form" id="passwordForm">
            <input type="password" id="password" placeholder="Enter password">
            <button onclick="submitPassword()">Decrypt</button>
        </div>
        <div class="progress-container" id="progressContainer">
            <div class="progress" id="progress"></div>
        </div>
        <p class="status" id="status">Initializing...</p>
        <p class="error" id="error"></p>
    </div>
    <script>
        window.FILE_ID = "{id}";
        window.FILENAME = "{filename}";
        window.FILE_SIZE = {file_size};
        window.HAS_PASSWORD = {has_password};
    </script>
    <script src="/assets/decrypt.js"></script>
</body>
</html>"#,
        filename = html_escape(&record.filename),
        size = format_size(record.file_size as u64),
        id = id,
        file_size = record.file_size,
        has_password = record.has_password,
    );

    Ok(Html(html))
}

/// GET /{id}/blob - Download raw encrypted blob
pub async fn download_blob(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Response, (StatusCode, String)> {
    // Check if file exists (but don't increment count - that happens on page load)
    state
        .db
        .get_file(&id)
        .await
        .map_err(|e| match e {
            crate::db::DbError::NotFound => (StatusCode::NOT_FOUND, "File not found".to_string()),
            _ => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        })?;

    let data = state
        .storage
        .load(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Response::builder()
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(header::CONTENT_LENGTH, data.len())
        .body(Body::from(data))
        .unwrap())
}

/// GET /{id}/meta - Get file metadata
pub async fn download_meta(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<FileMeta>, (StatusCode, String)> {
    let record = state
        .db
        .get_file(&id)
        .await
        .map_err(|e| match e {
            crate::db::DbError::NotFound => (StatusCode::NOT_FOUND, "File not found".to_string()),
            _ => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        })?;

    Ok(Json(FileMeta {
        filename: record.filename,
        size: record.file_size as u64,
        has_password: record.has_password,
        is_chunked: record.is_chunked,
        chunk_count: record.chunk_count.map(|c| c as u32),
    }))
}

/// DELETE /{id}
pub async fn delete_file(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<StatusCode, (StatusCode, String)> {
    let token = headers
        .get("X-Delete-Token")
        .and_then(|v| v.to_str().ok())
        .ok_or((StatusCode::UNAUTHORIZED, "Missing delete token".to_string()))?;

    let valid = state
        .db
        .verify_delete_token(&id, token)
        .await
        .map_err(|e| match e {
            crate::db::DbError::NotFound => (StatusCode::NOT_FOUND, "File not found".to_string()),
            _ => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        })?;

    if !valid {
        return Err((StatusCode::FORBIDDEN, "Invalid delete token".to_string()));
    }

    state
        .storage
        .delete(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    state
        .db
        .delete_file(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    tracing::info!("Deleted file: {}", id);

    Ok(StatusCode::NO_CONTENT)
}

/// GET /assets/decrypt.js
pub async fn serve_decrypt_js() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "application/javascript")],
        DECRYPT_JS,
    )
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} bytes", bytes)
    }
}
