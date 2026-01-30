// Browser-side decryption for spacetaxi
// Uses @noble/ciphers for XChaCha20-Poly1305 and hash-wasm for Argon2id

import { xchacha20poly1305 } from '@noble/ciphers/chacha';
import { argon2id } from 'hash-wasm';

declare global {
  interface Window {
    FILE_ID: string;
    FILENAME: string;
    FILE_SIZE: number;
    HAS_PASSWORD: boolean;
  }
}

const NONCE_SIZE = 24;
const TAG_SIZE = 16;
const CHUNK_SIZE = 10 * 1024 * 1024; // 10MB - must match server

interface UrlFragment {
  key?: string;
  salt?: string;
  nonce?: string; // Base nonce for chunked files
}

interface FileMeta {
  filename: string;
  size: number;
  has_password: boolean;
  is_chunked: boolean;
  chunk_count?: number;
}

function base64UrlDecode(str: string): Uint8Array {
  // Handle URL-safe base64
  const base64 = str.replace(/-/g, '+').replace(/_/g, '/');
  const padding = '='.repeat((4 - (base64.length % 4)) % 4);
  const binary = atob(base64 + padding);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) {
    bytes[i] = binary.charCodeAt(i);
  }
  return bytes;
}

function parseFragment(): UrlFragment | null {
  const hash = window.location.hash.slice(1);
  console.log('[parseFragment] Raw hash:', hash);
  console.log('[parseFragment] Hash length:', hash.length);

  if (!hash) {
    console.error('[parseFragment] No hash found in URL');
    return null;
  }

  try {
    // Fragment is base64url-encoded JSON (no padding from CLI)
    const base64 = hash.replace(/-/g, '+').replace(/_/g, '/');
    const padding = '='.repeat((4 - (base64.length % 4)) % 4);
    const decoded = atob(base64 + padding);
    console.log('[parseFragment] Decoded JSON:', decoded);
    const parsed = JSON.parse(decoded);
    console.log('[parseFragment] Parsed fragment:', parsed);
    return parsed;
  } catch (err) {
    console.error('[parseFragment] Failed to parse fragment:', err);
    return null;
  }
}

function updateStatus(message: string) {
  const status = document.getElementById('status');
  if (status) status.textContent = message;
}

function showError(message: string) {
  const error = document.getElementById('error');
  if (error) {
    error.textContent = message;
    error.style.display = 'block';
  }
  updateStatus('Error');
}

function showProgress(percent: number) {
  const container = document.getElementById('progressContainer');
  const bar = document.getElementById('progress');
  if (container) container.style.display = 'block';
  if (bar) bar.style.width = `${percent}%`;
}

function showPasswordForm() {
  const form = document.getElementById('passwordForm');
  if (form) form.style.display = 'block';
  updateStatus('Enter password to decrypt');
}

function showDownloadButton(onClick: () => void) {
  const container = document.querySelector('.container');
  if (!container) return;

  const existingBtn = document.getElementById('downloadBtn');
  if (existingBtn) existingBtn.remove();

  const btn = document.createElement('button');
  btn.id = 'downloadBtn';
  btn.textContent = 'Download';
  btn.onclick = onClick;

  const status = document.getElementById('status');
  if (status) {
    container.insertBefore(btn, status);
  } else {
    container.appendChild(btn);
  }
}

async function deriveKeyFromPassword(password: string, salt: Uint8Array): Promise<Uint8Array> {
  const hash = await argon2id({
    password: password,
    salt: salt,
    iterations: 3,
    memorySize: 65536, // 64MB
    parallelism: 4,
    hashLength: 32,
    outputType: 'binary',
  });
  return hash;
}

// Generate chunk-specific nonce by XORing chunk index into base nonce
function deriveChunkNonce(baseNonce: Uint8Array, chunkIndex: number): Uint8Array {
  const nonce = new Uint8Array(baseNonce);
  // XOR the chunk index into the last 8 bytes (little-endian)
  const indexBytes = new Uint8Array(8);
  let idx = chunkIndex;
  for (let i = 0; i < 8; i++) {
    indexBytes[i] = idx & 0xff;
    idx = Math.floor(idx / 256);
  }
  for (let i = 0; i < 8; i++) {
    nonce[NONCE_SIZE - 8 + i] ^= indexBytes[i];
  }
  return nonce;
}

function toHex(bytes: Uint8Array): string {
  return Array.from(bytes.slice(0, 32)).map(b => b.toString(16).padStart(2, '0')).join('');
}

function decryptSimple(key: Uint8Array, encrypted: Uint8Array): Uint8Array {
  console.log('[decrypt] Key length:', key.length, 'bytes');
  console.log('[decrypt] Key (first 32 bytes hex):', toHex(key));
  console.log('[decrypt] Encrypted data length:', encrypted.length, 'bytes');

  if (encrypted.length < NONCE_SIZE + TAG_SIZE) {
    throw new Error(`Invalid encrypted data: only ${encrypted.length} bytes (need at least ${NONCE_SIZE + TAG_SIZE})`);
  }

  const nonce = encrypted.slice(0, NONCE_SIZE);
  const ciphertext = encrypted.slice(NONCE_SIZE);

  console.log('[decrypt] Nonce (hex):', toHex(nonce));
  console.log('[decrypt] Ciphertext length:', ciphertext.length, 'bytes');
  console.log('[decrypt] Ciphertext first 32 bytes (hex):', toHex(ciphertext));

  try {
    const cipher = xchacha20poly1305(key, nonce);
    return cipher.decrypt(ciphertext);
  } catch (err) {
    console.error('[decrypt] Decryption failed:', err);
    console.error('[decrypt] This usually means: wrong key, corrupted data, or format mismatch');
    throw err;
  }
}

function decryptChunked(key: Uint8Array, encrypted: Uint8Array, baseNonce: Uint8Array): Uint8Array {
  // Each encrypted chunk is: [24-byte nonce][ciphertext + 16-byte tag]
  // Full chunks (except last) have encrypted size = NONCE_SIZE + CHUNK_SIZE + TAG_SIZE
  const ENCRYPTED_FULL_CHUNK_SIZE = NONCE_SIZE + CHUNK_SIZE + TAG_SIZE;

  const chunks: Uint8Array[] = [];
  let offset = 0;
  let chunkIndex = 0;

  while (offset < encrypted.length) {
    // Determine this chunk's total encrypted size
    const remainingData = encrypted.length - offset;
    const isFullChunk = remainingData >= ENCRYPTED_FULL_CHUNK_SIZE;
    const encryptedChunkSize = isFullChunk ? ENCRYPTED_FULL_CHUNK_SIZE : remainingData;

    if (encryptedChunkSize < NONCE_SIZE + TAG_SIZE) {
      throw new Error(`Invalid chunk ${chunkIndex}: too small (${encryptedChunkSize} bytes)`);
    }

    // Extract nonce and ciphertext for this chunk
    const chunkNonce = encrypted.slice(offset, offset + NONCE_SIZE);
    const ciphertext = encrypted.slice(offset + NONCE_SIZE, offset + encryptedChunkSize);

    // Decrypt this chunk
    const cipher = xchacha20poly1305(key, chunkNonce);
    const plaintext = cipher.decrypt(ciphertext);
    chunks.push(plaintext);

    offset += encryptedChunkSize;
    chunkIndex++;
  }

  // Combine all decrypted chunks
  const totalSize = chunks.reduce((sum, chunk) => sum + chunk.length, 0);
  const result = new Uint8Array(totalSize);
  let pos = 0;
  for (const chunk of chunks) {
    result.set(chunk, pos);
    pos += chunk.length;
  }

  return result;
}

function triggerDownload(data: Uint8Array, filename: string) {
  const blob = new Blob([data], { type: 'application/octet-stream' });
  const url = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = url;
  a.download = filename;
  document.body.appendChild(a);
  a.click();
  document.body.removeChild(a);
  URL.revokeObjectURL(url);
}

async function fetchMeta(): Promise<FileMeta> {
  const response = await fetch(`/${window.FILE_ID}/meta`);
  if (!response.ok) {
    throw new Error(`Failed to fetch metadata: ${response.status}`);
  }
  return response.json();
}

async function fetchBlob(): Promise<Uint8Array> {
  updateStatus('Downloading encrypted file...');
  showProgress(0);

  const blobUrl = `/${window.FILE_ID}/blob`;
  console.log('[fetchBlob] Fetching:', blobUrl);

  const response = await fetch(blobUrl);
  console.log('[fetchBlob] Response status:', response.status);
  console.log('[fetchBlob] Content-Length header:', response.headers.get('Content-Length'));

  if (!response.ok) {
    throw new Error(`Failed to download: ${response.status}`);
  }

  const reader = response.body?.getReader();
  if (!reader) {
    throw new Error('Failed to read response');
  }

  const contentLength = parseInt(response.headers.get('Content-Length') || '0', 10);
  const chunks: Uint8Array[] = [];
  let received = 0;

  while (true) {
    const { done, value } = await reader.read();
    if (done) break;

    chunks.push(value);
    received += value.length;

    if (contentLength > 0) {
      showProgress((received / contentLength) * 100);
    }
  }

  // Combine chunks
  const result = new Uint8Array(received);
  let offset = 0;
  for (const chunk of chunks) {
    result.set(chunk, offset);
    offset += chunk.length;
  }

  return result;
}

async function decryptAndDownload(key: Uint8Array, baseNonce?: Uint8Array) {
  try {
    const encrypted = await fetchBlob();

    updateStatus('Decrypting...');
    showProgress(100);

    let decrypted: Uint8Array;
    if (baseNonce) {
      // Chunked file
      decrypted = decryptChunked(key, encrypted, baseNonce);
    } else {
      // Simple file
      decrypted = decryptSimple(key, encrypted);
    }

    updateStatus('Ready to download');

    // Show download button instead of auto-downloading
    showDownloadButton(() => {
      triggerDownload(decrypted, window.FILENAME);
      updateStatus('Complete!');
    });
  } catch (error) {
    showError(error instanceof Error ? error.message : 'Decryption failed');
  }
}

// Password submission handler
(window as any).submitPassword = async function() {
  const input = document.getElementById('password') as HTMLInputElement;
  const password = input?.value;

  if (!password) {
    showError('Please enter a password');
    return;
  }

  const fragment = parseFragment();
  if (!fragment?.salt) {
    showError('Invalid URL - missing salt');
    return;
  }

  try {
    updateStatus('Deriving key from password...');
    const salt = base64UrlDecode(fragment.salt);
    const key = await deriveKeyFromPassword(password, salt);

    // Check if chunked (nonce present)
    const baseNonce = fragment.nonce ? base64UrlDecode(fragment.nonce) : undefined;

    await decryptAndDownload(key, baseNonce);
  } catch (error) {
    showError('Invalid password or decryption failed');
  }
};

// Main initialization
async function init() {
  const fragment = parseFragment();

  if (!fragment) {
    showError('Invalid URL - no decryption key found');
    return;
  }

  // Check if this is chunked (has nonce in fragment)
  const baseNonce = fragment.nonce ? base64UrlDecode(fragment.nonce) : undefined;

  if (window.HAS_PASSWORD) {
    // Password protected - need user input
    if (!fragment.salt) {
      showError('Invalid URL - missing salt for password-protected file');
      return;
    }
    showPasswordForm();
  } else {
    // Direct decryption with key from URL
    if (!fragment.key) {
      showError('Invalid URL - missing decryption key');
      return;
    }

    try {
      console.log('[init] fragment.key (base64):', fragment.key);
      const key = base64UrlDecode(fragment.key);
      console.log('[init] Decoded key length:', key.length, 'bytes');
      await decryptAndDownload(key, baseNonce);
    } catch (error) {
      console.error('[init] Error during decryption:', error);
      showError(error instanceof Error ? error.message : 'Decryption failed');
    }
  }
}

// Start when DOM is ready
if (document.readyState === 'loading') {
  document.addEventListener('DOMContentLoaded', init);
} else {
  init();
}
