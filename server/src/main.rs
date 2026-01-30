mod chunked;
mod db;
mod routes;
mod storage;

use axum::{
    extract::DefaultBodyLimit,
    routing::{delete, get, post, put},
    Router,
};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::RwLock;
use tower_http::cors::{Any, CorsLayer};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

pub struct AppState {
    pub db: db::Database,
    pub storage: storage::Storage,
    pub chunked_uploads: RwLock<std::collections::HashMap<String, chunked::ChunkedUpload>>,
    pub config: Config,
}

#[derive(Clone)]
pub struct Config {
    pub max_file_size: u64,
    pub data_dir: std::path::PathBuf,
    pub host: String,
    pub port: u16,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            max_file_size: 5 * 1024 * 1024 * 1024, // 5GB
            data_dir: std::path::PathBuf::from("./data"),
            host: "127.0.0.1".to_string(),
            port: 3000,
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "spacetaxi_server=debug,tower_http=debug".into()),
        ))
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Load config from environment
    let config = Config {
        data_dir: std::env::var("SPACETAXI_DATA_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::path::PathBuf::from("./data")),
        host: std::env::var("SPACETAXI_HOST").unwrap_or_else(|_| "127.0.0.1".to_string()),
        port: std::env::var("SPACETAXI_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(3000),
        ..Default::default()
    };

    // Ensure data directory exists
    std::fs::create_dir_all(&config.data_dir)?;
    std::fs::create_dir_all(config.data_dir.join("files"))?;
    std::fs::create_dir_all(config.data_dir.join("chunks"))?;

    // Initialize database
    let db_path = config.data_dir.join("spacetaxi.db");
    let db = db::Database::new(&db_path).await?;

    // Initialize storage
    let storage = storage::Storage::new(config.data_dir.join("files"));

    // Create app state
    let state = Arc::new(AppState {
        db,
        storage,
        chunked_uploads: RwLock::new(std::collections::HashMap::new()),
        config: config.clone(),
    });

    // Start background cleanup task
    let cleanup_state = Arc::clone(&state);
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;
            if let Err(e) = cleanup_expired(&cleanup_state).await {
                tracing::error!("Cleanup error: {}", e);
            }
        }
    });

    // Build router
    let app = Router::new()
        // Simple upload
        .route("/upload", post(routes::upload))
        // Chunked upload
        .route("/upload/init", post(routes::chunked_init))
        .route("/upload/{upload_id}/chunk/{chunk_num}", put(routes::chunked_upload_chunk))
        .route("/upload/{upload_id}/status", get(routes::chunked_status))
        .route("/upload/{upload_id}/complete", post(routes::chunked_complete))
        // Download
        .route("/{id}", get(routes::download_page))
        .route("/{id}/blob", get(routes::download_blob))
        .route("/{id}/meta", get(routes::download_meta))
        // Delete
        .route("/{id}", delete(routes::delete_file))
        // Static assets
        .route("/assets/decrypt.js", get(routes::serve_decrypt_js))
        .layer(CorsLayer::new().allow_origin(Any).allow_methods(Any).allow_headers(Any))
        .layer(DefaultBodyLimit::disable()) // Disable body limit for large chunk uploads
        .with_state(state);

    let addr: SocketAddr = format!("{}:{}", config.host, config.port).parse()?;
    tracing::info!("Starting spacetaxi server on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

async fn cleanup_expired(state: &Arc<AppState>) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let expired = state.db.get_expired_files().await?;
    for id in expired {
        tracing::info!("Cleaning up expired file: {}", id);
        state.storage.delete(&id).await?;
        state.db.delete_file(&id).await?;
    }
    Ok(())
}
