use crate::CliError;
use indicatif::ProgressBar;
use spacetaxi_shared::{
    crypto, ChunkUploadResponse, ChunkedUploadInitRequest, ChunkedUploadInitResponse,
    ChunkedUploadStatus, UploadMetadata, UploadResponse, CHUNK_SIZE, NONCE_SIZE,
};
use std::path::Path;

pub async fn chunked_upload(
    server: &str,
    file_path: &Path,
    key: &[u8; 32],
    metadata: &UploadMetadata,
    progress: &ProgressBar,
) -> Result<(UploadResponse, [u8; NONCE_SIZE]), CliError> {
    let file_size = std::fs::metadata(file_path)?.len();
    let total_chunks = (file_size + CHUNK_SIZE as u64 - 1) / CHUNK_SIZE as u64;

    let client = reqwest::Client::new();
    let server = server.trim_end_matches('/');

    // Generate base nonce for all chunks
    let base_nonce: [u8; NONCE_SIZE] = {
        let mut n = [0u8; NONCE_SIZE];
        rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut n);
        n
    };

    // Initialize chunked upload
    let init_request = ChunkedUploadInitRequest {
        size: file_size,
        chunk_size: CHUNK_SIZE as u64,
        filename: metadata.filename.clone(),
    };

    let mut request = client
        .post(format!("{}/upload/init", server))
        .json(&init_request)
        .header("X-One-Time", metadata.one_time.to_string())
        .header("X-Has-Password", metadata.has_password.to_string());

    if let Some(max) = metadata.max_downloads {
        request = request.header("X-Max-Downloads", max.to_string());
    }

    if let Some(expires) = metadata.expires_at {
        request = request.header("X-Expires", expires.to_string());
    }

    let filename_b64 =
        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &metadata.filename);
    request = request.header("X-Filename", filename_b64);

    let response = request.send().await?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(CliError::UploadError(format!(
            "Failed to init upload: {} - {}",
            status, body
        )));
    }

    let init_response: ChunkedUploadInitResponse = response
        .json()
        .await
        .map_err(|e| CliError::UploadError(format!("Failed to parse init response: {}", e)))?;

    let upload_id = init_response.upload_id;

    // Check for any already uploaded chunks (resume support)
    let status_response = client
        .get(format!("{}/upload/{}/status", server, upload_id))
        .send()
        .await?;

    let chunks_already_uploaded: Vec<u64> = if status_response.status().is_success() {
        let status: ChunkedUploadStatus = status_response.json().await.unwrap_or(ChunkedUploadStatus {
            chunks_received: vec![],
            total_chunks,
        });
        status.chunks_received
    } else {
        vec![]
    };

    // Upload chunks
    let file = std::fs::File::open(file_path)?;
    let mut reader = std::io::BufReader::new(file);
    let mut buffer = vec![0u8; CHUNK_SIZE];
    let mut bytes_processed = 0u64;

    for chunk_index in 0..total_chunks {
        use std::io::Read;

        // Calculate chunk size
        let remaining = file_size - bytes_processed;
        let chunk_size = std::cmp::min(remaining, CHUNK_SIZE as u64) as usize;

        // Read chunk
        reader.get_mut().seek(std::io::SeekFrom::Start(bytes_processed))?;
        let bytes_read = reader.read(&mut buffer[..chunk_size])?;
        if bytes_read != chunk_size {
            return Err(CliError::UploadError(format!(
                "Failed to read chunk {}: expected {} bytes, got {}",
                chunk_index, chunk_size, bytes_read
            )));
        }

        bytes_processed += chunk_size as u64;

        // Skip if already uploaded
        if chunks_already_uploaded.contains(&chunk_index) {
            progress.set_position(bytes_processed);
            continue;
        }

        // Encrypt chunk
        let (encrypted, _) = crypto::encrypt_chunk(key, &buffer[..chunk_size], chunk_index, Some(&base_nonce))?;

        // Upload chunk
        let response = client
            .put(format!("{}/upload/{}/chunk/{}", server, upload_id, chunk_index))
            .body(encrypted)
            .header("Content-Type", "application/octet-stream")
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(CliError::UploadError(format!(
                "Failed to upload chunk {}: {} - {}",
                chunk_index, status, body
            )));
        }

        let _chunk_response: ChunkUploadResponse = response
            .json()
            .await
            .map_err(|e| CliError::UploadError(format!("Failed to parse chunk response: {}", e)))?;

        progress.set_position(bytes_processed);
    }

    // Finalize upload
    let response = client
        .post(format!("{}/upload/{}/complete", server, upload_id))
        .send()
        .await?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(CliError::UploadError(format!(
            "Failed to finalize upload: {} - {}",
            status, body
        )));
    }

    let upload_response: UploadResponse = response
        .json()
        .await
        .map_err(|e| CliError::UploadError(format!("Failed to parse finalize response: {}", e)))?;

    Ok((upload_response, base_nonce))
}

use std::io::Seek;
