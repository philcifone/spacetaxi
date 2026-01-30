# spacetaxi

Encrypted file sharing CLI and server. Files are encrypted client-side; the server never sees plaintext.

## Project Structure

```
spacetaxi/
├── cli/              # Rust CLI binary
│   ├── src/
│   │   ├── main.rs
│   │   ├── crypto.rs    # Encryption/key generation
│   │   ├── upload.rs    # Simple upload (<50MB)
│   │   └── chunked.rs   # Chunked upload (>50MB, resumable)
│   └── Cargo.toml
├── server/           # Rust server (Axum)
│   ├── src/
│   │   ├── main.rs
│   │   ├── routes.rs
│   │   ├── chunked.rs   # Chunked upload handling
│   │   ├── storage.rs
│   │   └── db.rs
│   ├── templates/    # Askama HTML templates
│   └── Cargo.toml
├── web/              # Browser decryption UI
│   ├── src/
│   │   ├── decrypt.ts
│   │   └── ui.ts
│   ├── package.json
│   └── build.mjs     # esbuild script
├── shared/           # Shared types/crypto between CLI and server
│   ├── src/
│   │   ├── lib.rs
│   │   ├── crypto.rs
│   │   └── types.rs
│   └── Cargo.toml
├── SPEC.md
└── Cargo.toml        # Workspace
```

## Build & Run

```bash
# Build Rust workspace
cargo build --release

# Build web assets
cd web && npm install && npm run build

# Run CLI
./target/release/spacetaxi myfile.txt

# Run server (dev)
cargo run -p spacetaxi-server

# Run tests
cargo test --workspace

# Docker (production)
docker compose up -d
```

## Key Dependencies

- **CLI**: clap, reqwest, chacha20poly1305, argon2, base64, indicatif
- **Server**: axum, tokio, sqlx (SQLite), tower-http, tower (rate limiting), askama
- **Shared**: serde, thiserror, chacha20poly1305, argon2
- **Web**: esbuild, typescript, @noble/ciphers, argon2-browser

## Code Style

- Use `thiserror` for error types, not strings
- Async everywhere in server code
- CLI uses tokio runtime for async uploads
- Run `cargo fmt` and `cargo clippy` before commits

## Crypto Notes

- NEVER log or store decryption keys on server
- URL fragment (#...) is never sent to server - this is intentional
- Use constant-time comparison for any auth tokens
- XChaCha20-Poly1305 chosen for: 192-bit nonce (safe random), AEAD, fast
- Password derivation uses Argon2id with m=64MB, t=3, p=4

## Testing

- Unit tests for crypto round-trips
- Integration tests with test server
- `cargo test -p spacetaxi-shared`
- `cargo test -p spacetaxi`
- `cargo test -p spacetaxi-server`

## Environment Variables

- `SPACETAXI_CONFIG`: Path to server config file (default: config.toml)
- `SPACETAXI_DATA_DIR`: Data directory for files and database (default: ./data)
- `SPACETAXI_HOST`: Server bind address (default: 127.0.0.1)
- `SPACETAXI_PORT`: Server bind port (default: 3000)
