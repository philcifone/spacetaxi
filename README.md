# spacetaxi

## AI DISCLAIMER

THIS IS AN LLM-ASSISTED PROJECT - USE AT YOUR OWN RISK!!!

NO WARRANTIES PROVIDED OR PRIVACY/SECURITY GUARANTEED

NOT RESPONSIBLE FOR ANY ILLEGAL OR MISUSE OF THIS PROJECT

## Project

End-to-end encrypted file sharing. Files are encrypted client-side before upload; the server never sees plaintext data or decryption keys.

## Features

- **Zero-knowledge encryption**: Files encrypted locally with XChaCha20-Poly1305 before upload
- **No server-side keys**: Decryption key lives only in the URL fragment, never sent to server
- **Password protection**: Optional Argon2id-derived encryption for additional security
- **Expiring links**: Configurable expiration time and download limits
- **One-time downloads**: Option to delete file after first download
- **Large file support**: Chunked, resumable uploads for files over 50MB (up to 5GB)
- **Browser decryption**: Recipients decrypt files directly in browser, no CLI needed

## Installation

### From source

Requires Rust 1.70+ and Node.js 18+.

```bash
# Clone repository
git clone https://github.com/philcifone/spacetaxi.git
cd spacetaxi

# Build CLI and server
cargo build --release

# Build web assets
cd web && npm install && npm run build && cd ..

# Install CLI (optional)
cp target/release/spacetaxi ~/.local/bin/
```

### Pre-built binaries

Check the releases page for pre-built binaries for your platform.

## CLI Usage

```bash
# Basic upload
spacetaxi myfile.txt

# Password-protected upload
spacetaxi -p mysecretpassword myfile.txt

# One-time download (deleted after first access)
spacetaxi -1 myfile.txt

# Set expiration and download limit
spacetaxi -e 7d -m 10 myfile.txt

# Use custom server
spacetaxi -s https://myserver.example.com myfile.txt
```

### Options

| Flag | Short | Description | Default |
|------|-------|-------------|---------|
| `--one-time` | `-1` | Delete after first download | false |
| `--password <pass>` | `-p` | Require password for decryption | none |
| `--max-downloads <n>` | `-m` | Max download count before deletion | unlimited |
| `--expires <duration>` | `-e` | Expiration time (e.g., "1h", "7d") | 24h |
| `--server <url>` | `-s` | Custom server URL | spacetaxi.cc |

### Output

After upload, the CLI outputs a URL like:

```
https://spacetaxi.cc/abc123#<base64-key>
```

The portion after `#` is the decryption key. It is never sent to the server.

## Server Deployment

### Native / LXC container

This is the simplest deployment method, suitable for bare metal, VMs, or LXC containers.

1. Build the server:

```bash
cargo build --release -p spacetaxi-server
```

2. Build web assets:

```bash
cd web && npm install && npm run build && cd ..
```

3. Create a configuration file:

```bash
cp config.example.toml config.toml
# Edit as needed
```

4. Create data directory:

```bash
mkdir -p /var/lib/spacetaxi/files /var/lib/spacetaxi/chunks
```

5. Run the server:

```bash
SPACETAXI_CONFIG=/etc/spacetaxi/config.toml \
SPACETAXI_DATA_DIR=/var/lib/spacetaxi \
./target/release/spacetaxi-server
```

For production, create a systemd service:

```ini
# /etc/systemd/system/spacetaxi.service
[Unit]
Description=spacetaxi encrypted file sharing
After=network.target

[Service]
Type=simple
User=spacetaxi
ExecStart=/usr/local/bin/spacetaxi-server
Environment=SPACETAXI_CONFIG=/etc/spacetaxi/config.toml
Environment=SPACETAXI_DATA_DIR=/var/lib/spacetaxi
Environment=RUST_LOG=spacetaxi_server=info
Restart=on-failure

[Install]
WantedBy=multi-user.target
```

### Docker

A Dockerfile and docker-compose.yml are provided if you prefer containerized deployment:

```bash
# Build web assets first
cd web && npm install && npm run build && cd ..

# Start server
docker compose up -d
```

The server will be available at `http://localhost:3000`. Data is persisted in a Docker volume.

### Configuration

Create `config.toml` from the example:

```toml
[server]
host = "127.0.0.1"
port = 3000

[storage]
data_dir = "./data"
max_file_size = 5368709120  # 5GB

[limits]
default_expiration = 86400   # 24 hours
max_expiration = 604800      # 7 days
```

Alternatively, use environment variables:

| Variable | Description | Default |
|----------|-------------|---------|
| `SPACETAXI_CONFIG` | Path to config file | config.toml |
| `SPACETAXI_DATA_DIR` | Data directory | ./data |
| `SPACETAXI_HOST` | Bind address | 127.0.0.1 |
| `SPACETAXI_PORT` | Bind port | 3000 |
| `RUST_LOG` | Log level | info |

### Reverse proxy

For production, run behind a reverse proxy with TLS. Example nginx configuration:

```nginx
server {
    listen 443 ssl http2;
    server_name spacetaxi.example.com;

    ssl_certificate /path/to/cert.pem;
    ssl_certificate_key /path/to/key.pem;

    client_max_body_size 5G;

    location / {
        proxy_pass http://127.0.0.1:3000;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;

        # For chunked uploads
        proxy_request_buffering off;
        proxy_http_version 1.1;
    }
}
```

## Security

- **URL fragment security**: The decryption key in the URL fragment (`#...`) is never sent to the server per the HTTP specification. This is the core security property.
- **Encryption**: XChaCha20-Poly1305 with 192-bit nonces (safe for random generation) and authenticated encryption.
- **Password derivation**: Argon2id with m=64MB, t=3, p=4 for password-protected files.
- **Delete tokens**: Each upload receives a token for owner-controlled deletion.
- **No logging of keys**: The server never has access to encryption keys and cannot decrypt files.

## Development

```bash
# Run server in dev mode
cargo run -p spacetaxi-server

# Run tests
cargo test --workspace

# Format and lint
cargo fmt
cargo clippy
```

## License

GPL-3.0
