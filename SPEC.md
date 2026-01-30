# spacetaxi Specification

## Usage

```bash
spacetaxi [FLAGS] <file_name>
```

### Flags

| Flag | Short | Description | Default |
|------|-------|-------------|---------|
| `--one-time` | `-1` | Delete after first download | false |
| `--password <pass>` | `-p` | Require password for decryption | none |
| `--max-downloads <n>` | `-m` | Max download count before deletion | unlimited |
| `--expires <duration>` | `-e` | Expiration time (e.g., "1h", "7d") | 24h |
| `--server <url>` | `-s` | Custom server URL | spacetaxi.cc |

### Output

```
spacetaxi.cc/abc123#<base64-key>
```

## Encryption Scheme

- **Algorithm**: XChaCha20-Poly1305 (via `chacha20poly1305` crate)
- **Key derivation**:
  - Without password: Random 256-bit key, encoded in URL fragment
  - With password: Argon2id(password + random salt), salt in URL fragment
- **File format**: `[24-byte nonce][encrypted data][16-byte auth tag]`

## Server API

### `POST /upload`
- **Body**: Multipart form with encrypted blob
- **Headers**:
  - `X-One-Time: true/false`
  - `X-Max-Downloads: <n>`
  - `X-Expires: <unix-timestamp>`
  - `X-Has-Password: true/false`
  - `X-Filename: <original-filename>` (base64 encoded)
- **Response**: `{ "id": "abc123", "delete_token": "..." }`

### `GET /<id>`
- **Response**: HTML page with embedded decryption JS
- Increments download counter
- Returns 404/410 if expired or max downloads exceeded

### `GET /<id>/blob`
- **Response**: Raw encrypted blob
- Called by browser JS after page load

### `GET /<id>/meta`
- **Response**: `{ "filename": "...", "size": ..., "has_password": bool }`

### `DELETE /<id>` (optional, for CLI cleanup)
- Requires `X-Delete-Token` header with token returned at creation time

## Chunked Upload Protocol (files >50MB)

### `POST /upload/init`
- **Body**: `{ "size": <total-size>, "chunk_size": <size>, "filename": "..." }`
- **Headers**: Same metadata headers as `/upload`
- **Response**: `{ "upload_id": "..." }`

### `PUT /upload/<upload_id>/chunk/<n>`
- **Body**: Raw encrypted chunk data
- **Response**: `{ "received": <bytes> }`

### `GET /upload/<upload_id>/status`
- **Response**: `{ "chunks_received": [...], "total_chunks": n }`

### `POST /upload/<upload_id>/complete`
- **Response**: `{ "id": "abc123", "delete_token": "..." }`

Chunks are encrypted individually with same key but unique nonces (derived from chunk index).

## Storage

- Files stored as: `<storage_dir>/<id>.enc`
- Metadata in SQLite: id, filename, created_at, expires_at, download_count, max_downloads, one_time, has_password, file_size, delete_token

## Browser Decryption

The download page includes a minimal JS bundle that:
1. Fetches file metadata from `/<id>/meta`
2. Fetches encrypted blob from `/<id>/blob`
3. Extracts key from URL fragment
4. If password-protected: prompts user, derives key with Argon2id
5. Decrypts using @noble/ciphers for XChaCha20-Poly1305
6. Triggers browser download of decrypted file with original filename

## File Size Limits

- Default max: 5GB per file
- Configurable via server config
- Uses chunked upload for files >50MB

## Security Considerations

- URL fragment (#...) is never sent to server - this is the core security property
- Server only ever handles encrypted blobs
- Password-protected files use Argon2id with high memory cost
- All uploads receive a delete token for owner-controlled cleanup
- Constant-time comparison for all token verification
