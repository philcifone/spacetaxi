use crate::storage::ChunkStorage;
use std::collections::HashSet;
use std::path::PathBuf;

pub struct ChunkedUpload {
    pub upload_id: String,
    pub filename: String,
    pub total_size: u64,
    pub chunk_size: u64,
    pub total_chunks: u64,
    pub received_chunks: HashSet<u64>,
    pub one_time: bool,
    pub max_downloads: Option<u32>,
    pub expires_at: Option<i64>,
    pub has_password: bool,
    pub created_at: i64,
    pub storage: ChunkStorage,
}

impl ChunkedUpload {
    pub fn new(
        upload_id: String,
        filename: String,
        total_size: u64,
        chunk_size: u64,
        chunks_dir: PathBuf,
        one_time: bool,
        max_downloads: Option<u32>,
        expires_at: Option<i64>,
        has_password: bool,
    ) -> Self {
        let total_chunks = (total_size + chunk_size - 1) / chunk_size;
        Self {
            upload_id: upload_id.clone(),
            filename,
            total_size,
            chunk_size,
            total_chunks,
            received_chunks: HashSet::new(),
            one_time,
            max_downloads,
            expires_at,
            has_password,
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64,
            storage: ChunkStorage::new(chunks_dir),
        }
    }

    pub async fn save_chunk(&mut self, chunk_num: u64, data: &[u8]) -> Result<(), crate::storage::StorageError> {
        self.storage.save_chunk(&self.upload_id, chunk_num, data).await?;
        self.received_chunks.insert(chunk_num);
        Ok(())
    }

    pub fn is_complete(&self) -> bool {
        self.received_chunks.len() as u64 == self.total_chunks
    }

    pub fn get_received_chunks(&self) -> Vec<u64> {
        let mut chunks: Vec<_> = self.received_chunks.iter().copied().collect();
        chunks.sort();
        chunks
    }

    pub async fn cleanup(&self) -> Result<(), crate::storage::StorageError> {
        self.storage.cleanup(&self.upload_id).await
    }
}
