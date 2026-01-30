use crate::CliError;
use indicatif::ProgressBar;
use reqwest::multipart::{Form, Part};
use spacetaxi_shared::{UploadMetadata, UploadResponse};

pub async fn simple_upload(
    server: &str,
    encrypted_data: &[u8],
    metadata: &UploadMetadata,
    progress: &ProgressBar,
) -> Result<UploadResponse, CliError> {
    let client = reqwest::Client::new();

    // Create multipart form with the encrypted blob
    let part = Part::bytes(encrypted_data.to_vec())
        .file_name(metadata.filename.clone())
        .mime_str("application/octet-stream")
        .map_err(|e| CliError::UploadError(e.to_string()))?;

    let form = Form::new().part("file", part);

    // Build request with metadata headers
    let mut request = client
        .post(format!("{}/upload", server.trim_end_matches('/')))
        .multipart(form)
        .header("X-One-Time", metadata.one_time.to_string())
        .header("X-Has-Password", metadata.has_password.to_string());

    if let Some(max) = metadata.max_downloads {
        request = request.header("X-Max-Downloads", max.to_string());
    }

    if let Some(expires) = metadata.expires_at {
        request = request.header("X-Expires", expires.to_string());
    }

    // Encode filename as base64 to handle special characters
    let filename_b64 =
        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &metadata.filename);
    request = request.header("X-Filename", filename_b64);

    progress.set_position(encrypted_data.len() as u64);

    let response = request.send().await?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(CliError::UploadError(format!(
            "Server returned {}: {}",
            status, body
        )));
    }

    let upload_response: UploadResponse = response
        .json()
        .await
        .map_err(|e| CliError::UploadError(format!("Failed to parse response: {}", e)))?;

    Ok(upload_response)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metadata_defaults() {
        let meta = UploadMetadata::default();
        assert!(!meta.one_time);
        assert!(meta.max_downloads.is_none());
        assert!(!meta.has_password);
    }
}
