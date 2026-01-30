use sqlx::{sqlite::SqlitePoolOptions, Pool, Sqlite};
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum DbError {
    #[error("database error: {0}")]
    SqlxError(#[from] sqlx::Error),
    #[error("file not found")]
    NotFound,
    #[error("file expired or max downloads exceeded")]
    Expired,
}

#[derive(Debug, Clone)]
pub struct FileRecord {
    pub id: String,
    pub filename: String,
    pub file_size: i64,
    pub created_at: i64,
    pub expires_at: Option<i64>,
    pub download_count: i32,
    pub max_downloads: Option<i32>,
    pub one_time: bool,
    pub has_password: bool,
    pub delete_token: String,
    pub is_chunked: bool,
    pub chunk_count: Option<i32>,
}

#[derive(Clone)]
pub struct Database {
    pool: Pool<Sqlite>,
}

impl Database {
    pub async fn new(path: &Path) -> Result<Self, DbError> {
        let url = format!("sqlite:{}?mode=rwc", path.display());
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect(&url)
            .await?;

        // Run migrations
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS files (
                id TEXT PRIMARY KEY,
                filename TEXT NOT NULL,
                file_size INTEGER NOT NULL,
                created_at INTEGER NOT NULL,
                expires_at INTEGER,
                download_count INTEGER NOT NULL DEFAULT 0,
                max_downloads INTEGER,
                one_time INTEGER NOT NULL DEFAULT 0,
                has_password INTEGER NOT NULL DEFAULT 0,
                delete_token TEXT NOT NULL,
                is_chunked INTEGER NOT NULL DEFAULT 0,
                chunk_count INTEGER
            )
            "#,
        )
        .execute(&pool)
        .await?;

        // Index for expiration queries
        sqlx::query(
            r#"
            CREATE INDEX IF NOT EXISTS idx_files_expires_at ON files(expires_at)
            "#,
        )
        .execute(&pool)
        .await?;

        Ok(Self { pool })
    }

    pub async fn insert_file(&self, record: &FileRecord) -> Result<(), DbError> {
        sqlx::query(
            r#"
            INSERT INTO files (id, filename, file_size, created_at, expires_at, download_count,
                               max_downloads, one_time, has_password, delete_token, is_chunked, chunk_count)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&record.id)
        .bind(&record.filename)
        .bind(record.file_size)
        .bind(record.created_at)
        .bind(record.expires_at)
        .bind(record.download_count)
        .bind(record.max_downloads)
        .bind(record.one_time)
        .bind(record.has_password)
        .bind(&record.delete_token)
        .bind(record.is_chunked)
        .bind(record.chunk_count)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn get_file(&self, id: &str) -> Result<FileRecord, DbError> {
        let row = sqlx::query_as::<_, (
            String,
            String,
            i64,
            i64,
            Option<i64>,
            i32,
            Option<i32>,
            bool,
            bool,
            String,
            bool,
            Option<i32>,
        )>(
            r#"
            SELECT id, filename, file_size, created_at, expires_at, download_count,
                   max_downloads, one_time, has_password, delete_token, is_chunked, chunk_count
            FROM files
            WHERE id = ?
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(DbError::NotFound)?;

        Ok(FileRecord {
            id: row.0,
            filename: row.1,
            file_size: row.2,
            created_at: row.3,
            expires_at: row.4,
            download_count: row.5,
            max_downloads: row.6,
            one_time: row.7,
            has_password: row.8,
            delete_token: row.9,
            is_chunked: row.10,
            chunk_count: row.11,
        })
    }

    pub async fn increment_download_count(&self, id: &str) -> Result<(), DbError> {
        sqlx::query("UPDATE files SET download_count = download_count + 1 WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn check_file_valid(&self, id: &str) -> Result<FileRecord, DbError> {
        let record = self.get_file(id).await?;

        // Check expiration
        if let Some(expires_at) = record.expires_at {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64;
            if now > expires_at {
                return Err(DbError::Expired);
            }
        }

        // Check download count
        if let Some(max) = record.max_downloads {
            if record.download_count >= max {
                return Err(DbError::Expired);
            }
        }

        // Check one-time download
        if record.one_time && record.download_count > 0 {
            return Err(DbError::Expired);
        }

        Ok(record)
    }

    pub async fn delete_file(&self, id: &str) -> Result<(), DbError> {
        sqlx::query("DELETE FROM files WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn verify_delete_token(&self, id: &str, token: &str) -> Result<bool, DbError> {
        let record = self.get_file(id).await?;
        // Use constant-time comparison
        Ok(constant_time_eq(record.delete_token.as_bytes(), token.as_bytes()))
    }

    pub async fn get_expired_files(&self) -> Result<Vec<String>, DbError> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let rows = sqlx::query_as::<_, (String,)>(
            r#"
            SELECT id FROM files
            WHERE (expires_at IS NOT NULL AND expires_at < ?)
               OR (one_time = 1 AND download_count > 0)
               OR (max_downloads IS NOT NULL AND download_count >= max_downloads)
            "#,
        )
        .bind(now)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(|r| r.0).collect())
    }
}

// Constant-time comparison to prevent timing attacks
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut result = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        result |= x ^ y;
    }
    result == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constant_time_eq() {
        assert!(constant_time_eq(b"hello", b"hello"));
        assert!(!constant_time_eq(b"hello", b"world"));
        assert!(!constant_time_eq(b"hello", b"hell"));
    }
}
