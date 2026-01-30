use serde::{Deserialize, Serialize};

/// Response from POST /upload
#[derive(Debug, Serialize, Deserialize)]
pub struct UploadResponse {
    pub id: String,
    pub delete_token: String,
}

/// Response from POST /upload/init
#[derive(Debug, Serialize, Deserialize)]
pub struct ChunkedUploadInitResponse {
    pub upload_id: String,
}

/// Request body for POST /upload/init
#[derive(Debug, Serialize, Deserialize)]
pub struct ChunkedUploadInitRequest {
    pub size: u64,
    pub chunk_size: u64,
    pub filename: String,
}

/// Response from GET /upload/<id>/status
#[derive(Debug, Serialize, Deserialize)]
pub struct ChunkedUploadStatus {
    pub chunks_received: Vec<u64>,
    pub total_chunks: u64,
}

/// Response from PUT /upload/<id>/chunk/<n>
#[derive(Debug, Serialize, Deserialize)]
pub struct ChunkUploadResponse {
    pub received: u64,
}

/// Response from GET /<id>/meta
#[derive(Debug, Serialize, Deserialize)]
pub struct FileMeta {
    pub filename: String,
    pub size: u64,
    pub has_password: bool,
    #[serde(default)]
    pub is_chunked: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chunk_count: Option<u32>,
}

/// Upload metadata sent via headers
#[derive(Debug, Clone)]
pub struct UploadMetadata {
    pub one_time: bool,
    pub max_downloads: Option<u32>,
    pub expires_at: Option<i64>,
    pub has_password: bool,
    pub filename: String,
}

impl Default for UploadMetadata {
    fn default() -> Self {
        Self {
            one_time: false,
            max_downloads: None,
            expires_at: None,
            has_password: false,
            filename: String::from("file"),
        }
    }
}

/// URL fragment contents for key transmission
#[derive(Debug, Serialize, Deserialize)]
pub struct UrlFragment {
    /// Base64-encoded encryption key (for non-password protected files)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// Base64-encoded salt (for password-protected files)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub salt: Option<String>,
    /// Base64-encoded nonce for chunked files
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nonce: Option<String>,
}

impl UrlFragment {
    /// Create fragment for non-password protected file
    pub fn new_with_key(key: &[u8]) -> Self {
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
        Self {
            key: Some(URL_SAFE_NO_PAD.encode(key)),
            salt: None,
            nonce: None,
        }
    }

    /// Create fragment for password-protected file
    pub fn new_with_salt(salt: &[u8]) -> Self {
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
        Self {
            key: None,
            salt: Some(URL_SAFE_NO_PAD.encode(salt)),
            nonce: None,
        }
    }

    /// Create fragment for chunked file
    pub fn new_chunked(key: &[u8], nonce: &[u8]) -> Self {
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
        Self {
            key: Some(URL_SAFE_NO_PAD.encode(key)),
            salt: None,
            nonce: Some(URL_SAFE_NO_PAD.encode(nonce)),
        }
    }

    /// Create fragment for password-protected chunked file
    pub fn new_chunked_with_password(salt: &[u8], nonce: &[u8]) -> Self {
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
        Self {
            key: None,
            salt: Some(URL_SAFE_NO_PAD.encode(salt)),
            nonce: Some(URL_SAFE_NO_PAD.encode(nonce)),
        }
    }

    /// Encode to URL fragment string
    pub fn encode(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    /// Decode from URL fragment string
    pub fn decode(s: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(s)
    }

    /// Get the key bytes if present
    pub fn get_key(&self) -> Option<Vec<u8>> {
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
        self.key
            .as_ref()
            .and_then(|k| URL_SAFE_NO_PAD.decode(k).ok())
    }

    /// Get the salt bytes if present
    pub fn get_salt(&self) -> Option<Vec<u8>> {
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
        self.salt
            .as_ref()
            .and_then(|s| URL_SAFE_NO_PAD.decode(s).ok())
    }

    /// Get the nonce bytes if present
    pub fn get_nonce(&self) -> Option<Vec<u8>> {
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
        self.nonce
            .as_ref()
            .and_then(|n| URL_SAFE_NO_PAD.decode(n).ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_url_fragment_key() {
        let key = [0u8; 32];
        let fragment = UrlFragment::new_with_key(&key);
        let encoded = fragment.encode();
        let decoded = UrlFragment::decode(&encoded).unwrap();
        assert_eq!(decoded.get_key().unwrap(), key.to_vec());
        assert!(decoded.get_salt().is_none());
    }

    #[test]
    fn test_url_fragment_salt() {
        let salt = [1u8; 16];
        let fragment = UrlFragment::new_with_salt(&salt);
        let encoded = fragment.encode();
        let decoded = UrlFragment::decode(&encoded).unwrap();
        assert!(decoded.get_key().is_none());
        assert_eq!(decoded.get_salt().unwrap(), salt.to_vec());
    }

    #[test]
    fn test_url_fragment_chunked() {
        let key = [2u8; 32];
        let nonce = [3u8; 24];
        let fragment = UrlFragment::new_chunked(&key, &nonce);
        let encoded = fragment.encode();
        let decoded = UrlFragment::decode(&encoded).unwrap();
        assert_eq!(decoded.get_key().unwrap(), key.to_vec());
        assert_eq!(decoded.get_nonce().unwrap(), nonce.to_vec());
    }
}
