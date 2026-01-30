//! 备份文件：gzip 压缩 + AES-256-GCM 加密，不直接可见内容。

use aes_gcm::{aead::Aead, Aes256Gcm, KeyInit};
use flate2::write::GzEncoder;
use flate2::Compression;
use gateway_core::Snapshot;
use sha2::{Digest, Sha256};
use std::io::Write;

const NONCE_LEN: usize = 12;

/// 从密钥字符串派生 32 字节 AES 密钥
fn derive_key(secret: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(secret.as_bytes());
    let out = hasher.finalize();
    let mut key = [0u8; 32];
    key.copy_from_slice(&out);
    key
}

/// 将快照压缩并加密，返回 [nonce 12][ciphertext]（ciphertext 含 GCM tag）
pub fn encrypt_backup(snap: &Snapshot, secret: &str) -> Result<Vec<u8>, String> {
    let json = serde_json::to_vec(snap).map_err(|e| e.to_string())?;
    let mut compressed = Vec::new();
    {
        let mut gz = GzEncoder::new(&mut compressed, Compression::default());
        gz.write_all(&json).map_err(|e| e.to_string())?;
        gz.finish().map_err(|e| e.to_string())?;
    }
    let key = derive_key(secret);
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|e| e.to_string())?;
    let mut nonce_bytes = [0u8; NONCE_LEN];
    use rand::RngCore;
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = aes_gcm::Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(nonce, compressed.as_ref())
        .map_err(|e| e.to_string())?;
    let mut out = Vec::with_capacity(NONCE_LEN + ciphertext.len());
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

/// 从 [nonce 12][ciphertext] 解密并解压，得到快照
pub fn decrypt_backup(data: &[u8], secret: &str) -> Result<Snapshot, String> {
    if data.len() < NONCE_LEN {
        return Err("backup data too short".to_string());
    }
    let (nonce_slice, ciphertext) = data.split_at(NONCE_LEN);
    let key = derive_key(secret);
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|e| e.to_string())?;
    let nonce = aes_gcm::Nonce::from_slice(nonce_slice);
    let compressed = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| "decrypt failed: invalid or tampered backup".to_string())?;
    let mut gz = flate2::read::GzDecoder::new(compressed.as_slice());
    let mut json = Vec::new();
    std::io::copy(&mut gz, &mut json).map_err(|e| e.to_string())?;
    let snap: Snapshot = serde_json::from_slice(&json).map_err(|e| e.to_string())?;
    Ok(snap)
}
