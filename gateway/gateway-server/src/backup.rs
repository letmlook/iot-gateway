//! 备份文件：gzip 压缩 + AES-256-GCM 加密，不直接可见内容。
//!
//! ## 文件格式
//! - **v2（当前写入格式）**：`"GWBK" | version=2 | salt[16] | nonce[12] | ciphertext`
//!   密钥由 **Argon2id** 派生（m=19MiB, t=2, p=1），带随机 salt，抗离线暴力破解。
//! - **v1（只读兼容）**：`nonce[12] | ciphertext`，密钥为单轮 `SHA-256(secret)`。
//!   历史备份仍可恢复，但不再用于新备份。

use aes_gcm::{aead::Aead, Aes256Gcm, KeyInit};
use flate2::write::GzEncoder;
use flate2::Compression;
use gateway_core::Snapshot;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};

const NONCE_LEN: usize = 12;
const SALT_LEN: usize = 16;
/// 备份文件魔数，用于区分新旧格式
const MAGIC: &[u8; 4] = b"GWBK";
const FORMAT_V2: u8 = 2;
/// 解压后明文上限（防「解压炸弹」把内存打满）
const MAX_PLAINTEXT_BYTES: u64 = 256 * 1024 * 1024;

/// v1 兼容：单轮 SHA-256 派生（弱，仅用于读取历史备份）
fn derive_key_sha256(secret: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(secret.as_bytes());
    let out = hasher.finalize();
    let mut key = [0u8; 32];
    key.copy_from_slice(&out);
    key
}

/// v2：Argon2id 派生（带 salt，抗离线爆破）
fn derive_key_argon2(secret: &str, salt: &[u8]) -> Result<[u8; 32], String> {
    use argon2::{Algorithm, Argon2, Params, Version};
    // OWASP 推荐的最小档：19 MiB 内存、2 次迭代、单并行度
    let params = Params::new(19 * 1024, 2, 1, Some(32)).map_err(|e| e.to_string())?;
    let a2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = [0u8; 32];
    a2.hash_password_into(secret.as_bytes(), salt, &mut key)
        .map_err(|e| e.to_string())?;
    Ok(key)
}

fn gzip(data: &[u8]) -> Result<Vec<u8>, String> {
    let mut compressed = Vec::new();
    {
        let mut gz = GzEncoder::new(&mut compressed, Compression::default());
        gz.write_all(data).map_err(|e| e.to_string())?;
        gz.finish().map_err(|e| e.to_string())?;
    }
    Ok(compressed)
}

/// 解压并限制明文大小（防解压炸弹）
fn gunzip_limited(compressed: &[u8]) -> Result<Vec<u8>, String> {
    let mut gz = flate2::read::GzDecoder::new(compressed);
    let mut json = Vec::new();
    let mut limited = (&mut gz).take(MAX_PLAINTEXT_BYTES);
    limited.read_to_end(&mut json).map_err(|e| e.to_string())?;
    Ok(json)
}

/// 将快照压缩并加密为 v2 格式：`"GWBK" | 2 | salt[16] | nonce[12] | ciphertext`
pub fn encrypt_backup(snap: &Snapshot, secret: &str) -> Result<Vec<u8>, String> {
    let json = serde_json::to_vec(snap).map_err(|e| e.to_string())?;
    let compressed = gzip(&json)?;

    let mut salt = [0u8; SALT_LEN];
    let mut nonce_bytes = [0u8; NONCE_LEN];
    {
        use rand::RngCore;
        let mut rng = rand::thread_rng();
        rng.fill_bytes(&mut salt);
        rng.fill_bytes(&mut nonce_bytes);
    }

    let key = derive_key_argon2(secret, &salt)?;
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|e| e.to_string())?;
    let ciphertext = cipher
        .encrypt(
            aes_gcm::Nonce::from_slice(&nonce_bytes),
            compressed.as_ref(),
        )
        .map_err(|e| e.to_string())?;

    let mut out = Vec::with_capacity(4 + 1 + SALT_LEN + NONCE_LEN + ciphertext.len());
    out.extend_from_slice(MAGIC);
    out.push(FORMAT_V2);
    out.extend_from_slice(&salt);
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

/// 解密并解压备份。自动识别 v2（Argon2id）与 v1（SHA-256）两种格式。
pub fn decrypt_backup(data: &[u8], secret: &str) -> Result<Snapshot, String> {
    let (key, nonce_slice, ciphertext) = if data.starts_with(MAGIC) {
        // v2：magic(4) + version(1) + salt(16) + nonce(12) + ciphertext
        if data.len() < 4 + 1 + SALT_LEN + NONCE_LEN {
            return Err("backup data too short".to_string());
        }
        let version = data[4];
        if version != FORMAT_V2 {
            return Err(format!("unsupported backup format version: {}", version));
        }
        let salt = &data[5..5 + SALT_LEN];
        let nonce = &data[5 + SALT_LEN..5 + SALT_LEN + NONCE_LEN];
        let ct = &data[5 + SALT_LEN + NONCE_LEN..];
        (derive_key_argon2(secret, salt)?, nonce, ct)
    } else {
        // v1 兼容：nonce(12) + ciphertext
        if data.len() < NONCE_LEN {
            return Err("backup data too short".to_string());
        }
        let (nonce, ct) = data.split_at(NONCE_LEN);
        (derive_key_sha256(secret), nonce, ct)
    };

    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|e| e.to_string())?;
    let compressed = cipher
        .decrypt(aes_gcm::Nonce::from_slice(nonce_slice), ciphertext)
        .map_err(|_| "decrypt failed: invalid or tampered backup".to_string())?;

    let json = gunzip_limited(&compressed)?;
    let snap: Snapshot = serde_json::from_slice(&json).map_err(|e| e.to_string())?;
    Ok(snap)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gateway_core::SNAPSHOT_VERSION;

    fn snap() -> Snapshot {
        let mut s = Snapshot::default();
        s.version = SNAPSHOT_VERSION;
        s
    }

    #[test]
    fn v2_roundtrip_with_strong_kdf() {
        let data = encrypt_backup(&snap(), "s3cret").expect("encrypt");
        assert!(
            data.starts_with(MAGIC),
            "new backups use the versioned format"
        );
        assert_eq!(data[4], FORMAT_V2);

        let back = decrypt_backup(&data, "s3cret").expect("decrypt");
        assert_eq!(back.version, SNAPSHOT_VERSION);
    }

    #[test]
    fn wrong_password_is_rejected() {
        let data = encrypt_backup(&snap(), "right").unwrap();
        assert!(decrypt_backup(&data, "wrong").is_err());
    }

    #[test]
    fn tampered_ciphertext_is_rejected() {
        let mut data = encrypt_backup(&snap(), "s3cret").unwrap();
        let last = data.len() - 1;
        data[last] ^= 0x01;
        assert!(
            decrypt_backup(&data, "s3cret").is_err(),
            "GCM tag must catch tampering"
        );
    }

    #[test]
    fn legacy_v1_backup_is_still_readable() {
        // 手工构造 v1（nonce + AES-GCM，密钥为单轮 SHA-256）以验证向后兼容
        let json = serde_json::to_vec(&snap()).unwrap();
        let compressed = gzip(&json).unwrap();
        let key = derive_key_sha256("old-secret");
        let cipher = Aes256Gcm::new_from_slice(&key).unwrap();
        let nonce_bytes = [7u8; NONCE_LEN];
        let ct = cipher
            .encrypt(
                aes_gcm::Nonce::from_slice(&nonce_bytes),
                compressed.as_ref(),
            )
            .unwrap();
        let mut legacy = Vec::new();
        legacy.extend_from_slice(&nonce_bytes);
        legacy.extend_from_slice(&ct);

        let back = decrypt_backup(&legacy, "old-secret").expect("legacy backup should open");
        assert_eq!(back.version, SNAPSHOT_VERSION);
    }

    #[test]
    fn unsupported_format_version_is_reported() {
        let mut data = encrypt_backup(&snap(), "s").unwrap();
        data[4] = 9;
        let err = decrypt_backup(&data, "s").unwrap_err();
        assert!(
            err.contains("unsupported backup format version"),
            "got: {}",
            err
        );
    }
}
