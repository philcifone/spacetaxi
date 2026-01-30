use std::path::PathBuf;
use thiserror::Error;
use tokio::io::AsyncWriteExt;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
    #[error("file not found: {0}")]
    NotFound(String),
}

#[derive(Clone)]
pub struct Storage {
    base_path: PathBuf,
}

impl Storage {
    pub fn new(base_path: PathBuf) -> Self {
        Self { base_path }
    }

    pub fn file_path(&self, id: &str) -> PathBuf {
        self.base_path.join(format!("{}.enc", id))
    }

    pub async fn save(&self, id: &str, data: &[u8]) -> Result<(), StorageError> {
        let path = self.file_path(id);
        tokio::fs::write(&path, data).await?;
        Ok(())
    }

    pub async fn save_stream<S>(&self, id: &str, mut stream: S) -> Result<u64, StorageError>
    where
        S: futures::Stream<Item = Result<bytes::Bytes, std::io::Error>> + Unpin,
    {
        use futures::StreamExt;

        let path = self.file_path(id);
        let mut file = tokio::fs::File::create(&path).await?;
        let mut total = 0u64;

        while let Some(chunk) = stream.next().await {
            let bytes = chunk?;
            file.write_all(&bytes).await?;
            total += bytes.len() as u64;
        }

        file.flush().await?;
        Ok(total)
    }

    pub async fn load(&self, id: &str) -> Result<Vec<u8>, StorageError> {
        let path = self.file_path(id);
        if !path.exists() {
            return Err(StorageError::NotFound(id.to_string()));
        }
        let data = tokio::fs::read(&path).await?;
        Ok(data)
    }

    pub async fn delete(&self, id: &str) -> Result<(), StorageError> {
        let path = self.file_path(id);
        if path.exists() {
            tokio::fs::remove_file(&path).await?;
        }
        Ok(())
    }

    pub async fn exists(&self, id: &str) -> bool {
        self.file_path(id).exists()
    }

    pub async fn size(&self, id: &str) -> Result<u64, StorageError> {
        let path = self.file_path(id);
        let metadata = tokio::fs::metadata(&path).await?;
        Ok(metadata.len())
    }
}

/// Storage for chunked uploads
pub struct ChunkStorage {
    base_path: PathBuf,
}

impl ChunkStorage {
    pub fn new(base_path: PathBuf) -> Self {
        Self { base_path }
    }

    pub fn chunk_dir(&self, upload_id: &str) -> PathBuf {
        self.base_path.join(upload_id)
    }

    pub fn chunk_path(&self, upload_id: &str, chunk_num: u64) -> PathBuf {
        self.chunk_dir(upload_id).join(format!("{}.chunk", chunk_num))
    }

    pub async fn create_upload_dir(&self, upload_id: &str) -> Result<(), StorageError> {
        let dir = self.chunk_dir(upload_id);
        tokio::fs::create_dir_all(&dir).await?;
        Ok(())
    }

    pub async fn save_chunk(&self, upload_id: &str, chunk_num: u64, data: &[u8]) -> Result<(), StorageError> {
        let path = self.chunk_path(upload_id, chunk_num);
        tokio::fs::write(&path, data).await?;
        Ok(())
    }

    pub async fn load_chunk(&self, upload_id: &str, chunk_num: u64) -> Result<Vec<u8>, StorageError> {
        let path = self.chunk_path(upload_id, chunk_num);
        let data = tokio::fs::read(&path).await?;
        Ok(data)
    }

    pub async fn get_received_chunks(&self, upload_id: &str) -> Result<Vec<u64>, StorageError> {
        let dir = self.chunk_dir(upload_id);
        if !dir.exists() {
            return Ok(vec![]);
        }

        let mut chunks = vec![];
        let mut entries = tokio::fs::read_dir(&dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            if let Some(name) = entry.file_name().to_str() {
                if let Some(num_str) = name.strip_suffix(".chunk") {
                    if let Ok(num) = num_str.parse::<u64>() {
                        chunks.push(num);
                    }
                }
            }
        }
        chunks.sort();
        Ok(chunks)
    }

    pub async fn assemble_chunks(&self, upload_id: &str, total_chunks: u64, target: &Storage, file_id: &str) -> Result<u64, StorageError> {
        let target_path = target.file_path(file_id);
        let mut file = tokio::fs::File::create(&target_path).await?;
        let mut total_size = 0u64;

        for chunk_num in 0..total_chunks {
            let chunk_data = self.load_chunk(upload_id, chunk_num).await?;
            file.write_all(&chunk_data).await?;
            total_size += chunk_data.len() as u64;
        }

        file.flush().await?;
        Ok(total_size)
    }

    pub async fn cleanup(&self, upload_id: &str) -> Result<(), StorageError> {
        let dir = self.chunk_dir(upload_id);
        if dir.exists() {
            tokio::fs::remove_dir_all(&dir).await?;
        }
        Ok(())
    }
}
