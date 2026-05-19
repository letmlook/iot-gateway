//! 备份文件：gzip 压缩 + AES-256-GCM 加密，不直接可见内容。

use aes_gcm::{aead::Aead, Aes256Gcm, KeyInit};
use axum::{extract::{Query, State}, http::header::CONTENT_DISPOSITION, response::IntoResponse, Json};
use flate2::write::GzEncoder;
use flate2::Compression;
use gateway_core::Snapshot;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

use crate::api::ApiError;
use crate::state::AppState;

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

// ---------- SQLite file-level backup/restore ----------

#[derive(Serialize)]
pub struct SqliteBackupResponse {
    pub path: String,
    pub size_bytes: u64,
}

#[derive(Deserialize)]
pub struct SqliteRestoreQuery {
    pub password: Option<String>,
}

/// POST /admin/sqlite-backup — trigger SQLite file-level backup
pub async fn sqlite_backup(
    State(state): State<AppState>,
) -> Result<Json<SqliteBackupResponse>, ApiError> {
    let backup_dir = PathBuf::from("data/backups");
    tokio::fs::create_dir_all(&backup_dir).await.ok();

    let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S").to_string();
    let backup_path = backup_dir.join(format!("gateway_sqlite_{}.db", timestamp));

    let db_path = &state.config.data_db();

    tokio::fs::copy(db_path, &backup_path)
        .await
        .map_err(|e| ApiError::internal(&format!("sqlite backup failed: {}", e)))?;

    let metadata = tokio::fs::metadata(&backup_path)
        .await
        .map_err(|e| ApiError::internal(&e.to_string()))?;

    Ok(Json(SqliteBackupResponse {
        path: backup_path.to_string_lossy().to_string(),
        size_bytes: metadata.len(),
    }))
}

/// GET /admin/sqlite-backup — download latest SQLite backup file
pub async fn sqlite_download_latest(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ApiError> {
    let backup_dir = PathBuf::from("data/backups");

    let mut entries = tokio::fs::read_dir(&backup_dir)
        .await
        .map_err(|e| ApiError::internal(&e.to_string()))?;

    let mut latest_entry: Option<tokio::fs::DirEntry> = None;
    let mut latest_modified: Option<std::time::SystemTime> = None;
    while let Some(entry) = entries.next_entry().await.map_err(|e| ApiError::internal(e.to_string()))? {
        if entry.path().extension().map_or(false, |ext| ext == "db") {
            if let Ok(meta) = entry.metadata().await {
                let modified = meta.modified().ok();
                let is_newer = latest_modified.is_none()
                    || (modified.is_some() && latest_modified.is_some() && modified > latest_modified);
                if is_newer {
                    latest_entry = Some(entry);
                    latest_modified = modified;
                }
            }
        }
    }

    let latest = latest_entry.ok_or_else(|| ApiError::not_found("no sqlite backup found"))?;

    let filename = latest.file_name().to_string_lossy().to_string();
    let bytes = tokio::fs::read(latest.path()).await
        .map_err(|e| ApiError::internal(&e.to_string()))?;

    Ok(([(CONTENT_DISPOSITION, format!("attachment; filename=\"{}\"", filename))], bytes))
}

/// POST /admin/sqlite-restore — restore SQLite database from backup
pub async fn sqlite_restore(
    State(state): State<AppState>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let backup_path = body.get("backup_path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ApiError::bad_request("backup_path required"))?;

    let dest = state.config.data_db();

    tokio::fs::copy(backup_path, &dest)
        .await
        .map_err(|e| ApiError::internal(&format!("sqlite restore failed: {}", e)))?;

    Ok(Json(serde_json::json!({ "status": "restored", "from": backup_path })))
}
