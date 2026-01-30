mod chunked;
mod upload;

use clap::Parser;
use indicatif::{ProgressBar, ProgressStyle};
use spacetaxi_shared::{UrlFragment, CHUNK_SIZE};
use std::path::PathBuf;
use thiserror::Error;

const DEFAULT_SERVER: &str = "https://spacetaxi.cc";
const SIMPLE_UPLOAD_THRESHOLD: u64 = 50 * 1024 * 1024; // 50MB

#[derive(Parser, Debug)]
#[command(name = "spacetaxi")]
#[command(about = "Encrypted file sharing - files are encrypted locally before upload")]
#[command(version)]
struct Args {
    /// File to upload
    file: PathBuf,

    /// Delete after first download
    #[arg(short = '1', long)]
    one_time: bool,

    /// Require password for decryption
    #[arg(short, long)]
    password: Option<String>,

    /// Max download count before deletion
    #[arg(short, long)]
    max_downloads: Option<u32>,

    /// Expiration time (e.g., "1h", "7d", "30m")
    #[arg(short, long, default_value = "24h")]
    expires: String,

    /// Custom server URL
    #[arg(short, long, default_value = DEFAULT_SERVER)]
    server: String,
}

#[derive(Debug, Error)]
pub enum CliError {
    #[error("file not found: {0}")]
    FileNotFound(PathBuf),
    #[error("failed to read file: {0}")]
    FileReadError(#[from] std::io::Error),
    #[error("encryption failed: {0}")]
    EncryptionError(#[from] spacetaxi_shared::crypto::CryptoError),
    #[error("upload failed: {0}")]
    UploadError(String),
    #[error("invalid duration format: {0}")]
    InvalidDuration(String),
    #[error("HTTP error: {0}")]
    HttpError(#[from] reqwest::Error),
}

fn parse_duration(s: &str) -> Result<std::time::Duration, CliError> {
    let s = s.trim();
    if s.is_empty() {
        return Err(CliError::InvalidDuration("empty duration".to_string()));
    }

    let (num_str, unit) = if s.ends_with("ms") {
        (&s[..s.len() - 2], "ms")
    } else {
        let split_pos = s
            .chars()
            .position(|c| !c.is_ascii_digit())
            .unwrap_or(s.len());
        (&s[..split_pos], &s[split_pos..])
    };

    let num: u64 = num_str
        .parse()
        .map_err(|_| CliError::InvalidDuration(s.to_string()))?;

    let multiplier = match unit {
        "s" | "sec" | "second" | "seconds" => 1,
        "m" | "min" | "minute" | "minutes" => 60,
        "h" | "hr" | "hour" | "hours" => 3600,
        "d" | "day" | "days" => 86400,
        "w" | "week" | "weeks" => 604800,
        "ms" => return Ok(std::time::Duration::from_millis(num)),
        "" => 1, // default to seconds
        _ => return Err(CliError::InvalidDuration(s.to_string())),
    };

    Ok(std::time::Duration::from_secs(num * multiplier))
}

fn create_progress_bar(total: u64, message: &str) -> ProgressBar {
    let pb = ProgressBar::new(total);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{msg} [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta})")
            .unwrap()
            .progress_chars("=>-"),
    );
    pb.set_message(message.to_string());
    pb
}

#[tokio::main]
async fn main() -> Result<(), CliError> {
    let args = Args::parse();

    // Verify file exists
    if !args.file.exists() {
        return Err(CliError::FileNotFound(args.file));
    }

    let file_size = std::fs::metadata(&args.file)?.len();
    let filename = args
        .file
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file")
        .to_string();

    // Parse expiration
    let duration = parse_duration(&args.expires)?;
    let expires_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
        + duration.as_secs() as i64;

    // Generate key or derive from password
    let (key, fragment) = if let Some(ref password) = args.password {
        let salt = spacetaxi_shared::generate_salt();
        let key = spacetaxi_shared::derive_key_from_password(password, &salt)?;
        (key, UrlFragment::new_with_salt(&salt))
    } else {
        let key = spacetaxi_shared::generate_key();
        (key, UrlFragment::new_with_key(&key))
    };

    let metadata = spacetaxi_shared::UploadMetadata {
        one_time: args.one_time,
        max_downloads: args.max_downloads,
        expires_at: Some(expires_at),
        has_password: args.password.is_some(),
        filename: filename.clone(),
    };

    println!("Encrypting {}...", filename);

    let (id, fragment) = if file_size <= SIMPLE_UPLOAD_THRESHOLD {
        // Simple upload for small files
        let pb = create_progress_bar(file_size, "Encrypting");
        let data = std::fs::read(&args.file)?;
        pb.set_position(file_size);
        pb.finish_with_message("Encrypted");

        let encrypted = spacetaxi_shared::crypto::encrypt_file(&key, &data)?;

        println!("Uploading {} ({} bytes encrypted)...", filename, encrypted.len());
        let pb = create_progress_bar(encrypted.len() as u64, "Uploading");

        let response = upload::simple_upload(&args.server, &encrypted, &metadata, &pb).await?;
        pb.finish_with_message("Uploaded");

        (response.id, fragment)
    } else {
        // Chunked upload for large files
        let total_chunks = (file_size + CHUNK_SIZE as u64 - 1) / CHUNK_SIZE as u64;
        println!(
            "Large file detected ({:.2} MB), using chunked upload ({} chunks)...",
            file_size as f64 / 1024.0 / 1024.0,
            total_chunks
        );

        let pb = create_progress_bar(file_size, "Encrypting & Uploading");

        let (response, base_nonce) =
            chunked::chunked_upload(&args.server, &args.file, &key, &metadata, &pb).await?;

        pb.finish_with_message("Uploaded");

        // Create fragment with nonce for chunked files
        let fragment = if args.password.is_some() {
            let salt = fragment.get_salt().unwrap();
            UrlFragment::new_chunked_with_password(&salt, &base_nonce)
        } else {
            UrlFragment::new_chunked(&key, &base_nonce)
        };

        (response.id, fragment)
    };

    // Build final URL
    let fragment_encoded =
        base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, fragment.encode());
    let url = format!(
        "{}/{}#{}",
        args.server.trim_end_matches('/'),
        id,
        fragment_encoded
    );

    println!();
    println!("Share this link:");
    println!("{}", url);
    println!();

    if args.one_time {
        println!("Note: This link will expire after the first download.");
    } else if let Some(max) = args.max_downloads {
        println!("Note: This link will expire after {} downloads.", max);
    }

    println!(
        "Note: This link will expire in {}.",
        humanize_duration(duration)
    );

    if args.password.is_some() {
        println!("Note: Password required to decrypt.");
    }

    Ok(())
}

fn humanize_duration(d: std::time::Duration) -> String {
    let secs = d.as_secs();
    if secs < 60 {
        format!("{} seconds", secs)
    } else if secs < 3600 {
        format!("{} minutes", secs / 60)
    } else if secs < 86400 {
        format!("{} hours", secs / 3600)
    } else {
        format!("{} days", secs / 86400)
    }
}
