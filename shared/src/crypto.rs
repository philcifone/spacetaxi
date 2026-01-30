use chacha20poly1305::{
    aead::{Aead, KeyInit},
    XChaCha20Poly1305, XNonce,
};
use rand::RngCore;
use thiserror::Error;

pub const KEY_SIZE: usize = 32;
pub const NONCE_SIZE: usize = 24;
pub const TAG_SIZE: usize = 16;
pub const SALT_SIZE: usize = 16;
pub const CHUNK_SIZE: usize = 10 * 1024 * 1024; // 10MB chunks

// Argon2id parameters
const ARGON2_M_COST: u32 = 65536; // 64MB
const ARGON2_T_COST: u32 = 3;
const ARGON2_P_COST: u32 = 4;

#[derive(Debug, Error)]
pub enum CryptoError {
    #[error("encryption failed: {0}")]
    EncryptionFailed(String),
    #[error("decryption failed: {0}")]
    DecryptionFailed(String),
    #[error("key derivation failed: {0}")]
    KeyDerivationFailed(String),
    #[error("invalid data length")]
    InvalidDataLength,
}

/// Generate a random 256-bit encryption key
pub fn generate_key() -> [u8; KEY_SIZE] {
    let mut key = [0u8; KEY_SIZE];
    rand::thread_rng().fill_bytes(&mut key);
    key
}

/// Generate a random salt for password derivation
pub fn generate_salt() -> [u8; SALT_SIZE] {
    let mut salt = [0u8; SALT_SIZE];
    rand::thread_rng().fill_bytes(&mut salt);
    salt
}

/// Derive an encryption key from a password using Argon2id
pub fn derive_key_from_password(password: &str, salt: &[u8]) -> Result<[u8; KEY_SIZE], CryptoError> {
    use argon2::{Algorithm, Argon2, Params, Version};

    let params = Params::new(ARGON2_M_COST, ARGON2_T_COST, ARGON2_P_COST, Some(KEY_SIZE))
        .map_err(|e| CryptoError::KeyDerivationFailed(e.to_string()))?;

    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);

    let mut key = [0u8; KEY_SIZE];
    argon2
        .hash_password_into(password.as_bytes(), salt, &mut key)
        .map_err(|e| CryptoError::KeyDerivationFailed(e.to_string()))?;

    Ok(key)
}

/// Generate a nonce for a specific chunk index
/// This allows deterministic nonce generation for chunked encryption
fn generate_chunk_nonce(base_nonce: &[u8; NONCE_SIZE], chunk_index: u64) -> [u8; NONCE_SIZE] {
    let mut nonce = *base_nonce;
    // XOR the chunk index into the last 8 bytes of the nonce
    let index_bytes = chunk_index.to_le_bytes();
    for i in 0..8 {
        nonce[NONCE_SIZE - 8 + i] ^= index_bytes[i];
    }
    nonce
}

/// Encrypt a chunk of data
/// Returns: [24-byte nonce][encrypted data][16-byte auth tag]
pub fn encrypt_chunk(
    key: &[u8; KEY_SIZE],
    plaintext: &[u8],
    chunk_index: u64,
    base_nonce: Option<&[u8; NONCE_SIZE]>,
) -> Result<(Vec<u8>, [u8; NONCE_SIZE]), CryptoError> {
    let cipher = XChaCha20Poly1305::new(key.into());

    // Generate or derive nonce
    let nonce = if let Some(base) = base_nonce {
        generate_chunk_nonce(base, chunk_index)
    } else {
        let mut n = [0u8; NONCE_SIZE];
        rand::thread_rng().fill_bytes(&mut n);
        n
    };

    let xnonce = XNonce::from_slice(&nonce);
    let ciphertext = cipher
        .encrypt(xnonce, plaintext)
        .map_err(|e| CryptoError::EncryptionFailed(e.to_string()))?;

    // Prepend nonce to ciphertext
    let mut result = Vec::with_capacity(NONCE_SIZE + ciphertext.len());
    result.extend_from_slice(&nonce);
    result.extend_from_slice(&ciphertext);

    Ok((result, nonce))
}

/// Decrypt a chunk of data
/// Input format: [24-byte nonce][encrypted data][16-byte auth tag]
pub fn decrypt_chunk(key: &[u8; KEY_SIZE], encrypted: &[u8]) -> Result<Vec<u8>, CryptoError> {
    if encrypted.len() < NONCE_SIZE + TAG_SIZE {
        return Err(CryptoError::InvalidDataLength);
    }

    let cipher = XChaCha20Poly1305::new(key.into());

    let nonce = XNonce::from_slice(&encrypted[..NONCE_SIZE]);
    let ciphertext = &encrypted[NONCE_SIZE..];

    cipher
        .decrypt(nonce, ciphertext)
        .map_err(|e| CryptoError::DecryptionFailed(e.to_string()))
}

/// Encrypt an entire file (for small files <50MB)
/// Returns the encrypted data with nonce prepended
pub fn encrypt_file(key: &[u8; KEY_SIZE], plaintext: &[u8]) -> Result<Vec<u8>, CryptoError> {
    let (encrypted, _) = encrypt_chunk(key, plaintext, 0, None)?;
    Ok(encrypted)
}

/// Decrypt an entire file
pub fn decrypt_file(key: &[u8; KEY_SIZE], encrypted: &[u8]) -> Result<Vec<u8>, CryptoError> {
    decrypt_chunk(key, encrypted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_key_generation() {
        let key1 = generate_key();
        let key2 = generate_key();
        assert_ne!(key1, key2);
        assert_eq!(key1.len(), KEY_SIZE);
    }

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        let key = generate_key();
        let plaintext = b"Hello, spacetaxi!";

        let encrypted = encrypt_file(&key, plaintext).unwrap();
        let decrypted = decrypt_file(&key, &encrypted).unwrap();

        assert_eq!(plaintext.as_slice(), decrypted.as_slice());
    }

    #[test]
    fn test_encrypt_decrypt_large_data() {
        let key = generate_key();
        let plaintext: Vec<u8> = (0..100_000).map(|i| (i % 256) as u8).collect();

        let encrypted = encrypt_file(&key, &plaintext).unwrap();
        let decrypted = decrypt_file(&key, &encrypted).unwrap();

        assert_eq!(plaintext, decrypted);
    }

    #[test]
    fn test_chunked_encryption() {
        let key = generate_key();
        let base_nonce = {
            let mut n = [0u8; NONCE_SIZE];
            rand::thread_rng().fill_bytes(&mut n);
            n
        };

        let chunk1 = b"First chunk of data";
        let chunk2 = b"Second chunk of data";

        let (enc1, _) = encrypt_chunk(&key, chunk1, 0, Some(&base_nonce)).unwrap();
        let (enc2, _) = encrypt_chunk(&key, chunk2, 1, Some(&base_nonce)).unwrap();

        let dec1 = decrypt_chunk(&key, &enc1).unwrap();
        let dec2 = decrypt_chunk(&key, &enc2).unwrap();

        assert_eq!(chunk1.as_slice(), dec1.as_slice());
        assert_eq!(chunk2.as_slice(), dec2.as_slice());
    }

    #[test]
    fn test_password_key_derivation() {
        let password = "my-secret-password";
        let salt = generate_salt();

        let key1 = derive_key_from_password(password, &salt).unwrap();
        let key2 = derive_key_from_password(password, &salt).unwrap();

        assert_eq!(key1, key2);

        // Different salt should produce different key
        let salt2 = generate_salt();
        let key3 = derive_key_from_password(password, &salt2).unwrap();
        assert_ne!(key1, key3);
    }

    #[test]
    fn test_password_encrypt_decrypt() {
        let password = "test-password-123";
        let salt = generate_salt();
        let key = derive_key_from_password(password, &salt).unwrap();

        let plaintext = b"Secret message encrypted with password";
        let encrypted = encrypt_file(&key, plaintext).unwrap();
        let decrypted = decrypt_file(&key, &encrypted).unwrap();

        assert_eq!(plaintext.as_slice(), decrypted.as_slice());
    }

    #[test]
    fn test_wrong_key_fails() {
        let key1 = generate_key();
        let key2 = generate_key();
        let plaintext = b"Some data";

        let encrypted = encrypt_file(&key1, plaintext).unwrap();
        let result = decrypt_file(&key2, &encrypted);

        assert!(result.is_err());
    }

    #[test]
    fn test_tampered_data_fails() {
        let key = generate_key();
        let plaintext = b"Original data";

        let mut encrypted = encrypt_file(&key, plaintext).unwrap();
        // Tamper with the ciphertext
        if let Some(byte) = encrypted.get_mut(NONCE_SIZE + 5) {
            *byte ^= 0xFF;
        }

        let result = decrypt_file(&key, &encrypted);
        assert!(result.is_err());
    }
}
