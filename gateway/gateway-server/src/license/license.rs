//! License 文件读取与 RSA 验签校验。
//! 授权文件为加密的二进制格式，先用 AES-256-GCM 解密，再验签。

use crate::license::hardware;
use crate::license::error::LicenseError;
use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use base64::Engine;
use rsa::pkcs8::DecodePublicKey;
use rsa::sha2::Sha256;
use rsa::signature::Verifier;
use rsa::RsaPublicKey;
use serde::Deserialize;
use std::path::Path;

/// 授权文件名（放在数据目录下）
pub const LICENSE_FILENAME: &str = "license.dat";

/// AES-256-GCM 加密密钥（32 字节）- 用于授权文件加解密
/// 注意：此密钥需与 Python 脚本 gen_license.py 中的密钥保持一致
const LICENSE_AES_KEY: &[u8; 32] = b"IoTGateway@2024!SecretKey#Lic_32";

/// 内置 RSA 公钥（PEM）。与管理员签名脚本中的私钥成对，严禁泄露私钥。
/// 建议：发布前对公钥字符串做简单混淆或分段存储，增加逆向成本。
const BUILTIN_PUBLIC_KEY_PEM: &str = r#"-----BEGIN PUBLIC KEY-----
MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAvqddab7KV8EV0A2ypXfF
GpVtgPEQcFIUVIXOsxyNDmZraMOBSUL9nvm+eaX2vdUZgKHeQA2+D0hz6lDcC0nQ
Cv8gJXolr8Lfb9CwgKSFUbE/C0cAwnbyIQ3/QvVcBEDlzcVAoQc50+UiFOLnvgpE
udYL8/288MPaIO619Qs3TcSIfM3+R+djnadrDkmj9eN3LYiHZoUk5c3LfACEB4pq
jmK8KRJzqlI+wWFHI6iCwnXrhU51EdlVVPTpKTqiIc2EB6IPyy5XC6cLXq03Eyay
jdBim1jckfdk1hB6t2UsPsYkUxhSUmE7Z+hami8gh+L715R/Noeros6OjmUY+www
dQIDAQAB
-----END PUBLIC KEY-----"#;

/// 授权载荷（验签后解析出的内容）
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LicensePayload {
    pub machine_id: String,
    pub expiry_date: String, // ISO 日期，如 "2026-12-31"
    pub features: Vec<String>,
    /// 最大点位数限制，None 或 0 表示无限制
    #[serde(default)]
    pub max_tags: Option<u64>,
}

/// 磁盘上的授权文件结构（payload 与签名为 base64）
#[derive(Deserialize)]
struct LicenseFile {
    payload_b64: String,
    signature_b64: String,
}

/// 解密授权文件内容（AES-256-GCM）
/// 文件格式：nonce(12字节) + ciphertext
fn decrypt_license_file(encrypted: &[u8]) -> Result<Vec<u8>, LicenseError> {
    if encrypted.len() < 12 {
        return Err(LicenseError::InvalidFormat("license file too short".to_string()));
    }
    
    let (nonce_bytes, ciphertext) = encrypted.split_at(12);
    let cipher = Aes256Gcm::new_from_slice(LICENSE_AES_KEY)
        .map_err(|e| LicenseError::Internal(format!("AES key error: {}", e)))?;
    let nonce = Nonce::from_slice(nonce_bytes);
    
    cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| LicenseError::InvalidFormat("license decryption failed".to_string()))
}

/// 从 `license_path` 读取 license.dat，解密、验签并校验机器码与到期时间。
/// 完全离线，不发起任何网络请求。
pub fn load_and_verify_license(license_path: &Path) -> Result<LicensePayload, LicenseError> {
    let encrypted = std::fs::read(license_path).map_err(LicenseError::FileNotFound)?;
    
    // 先解密
    let decrypted = decrypt_license_file(&encrypted)?;
    let s = String::from_utf8(decrypted).map_err(|e| LicenseError::InvalidFormat(e.to_string()))?;
    let file: LicenseFile =
        serde_json::from_str(&s).map_err(|e| LicenseError::InvalidFormat(e.to_string()))?;

    let payload_bytes = base64::engine::general_purpose::STANDARD
        .decode(file.payload_b64.trim())
        .map_err(|e| LicenseError::InvalidFormat(format!("payload base64: {}", e)))?;
    let signature_bytes = base64::engine::general_purpose::STANDARD
        .decode(file.signature_b64.trim())
        .map_err(|e| LicenseError::InvalidFormat(format!("signature base64: {}", e)))?;

    let public_key = parse_public_key()?;
    verify_signature(&public_key, &payload_bytes, &signature_bytes)?;

    let payload: LicensePayload =
        serde_json::from_slice(&payload_bytes).map_err(|e| LicenseError::InvalidFormat(e.to_string()))?;

    let current_machine_id = hardware::machine_id().map_err(LicenseError::Internal)?;
    if payload.machine_id != current_machine_id {
        return Err(LicenseError::MachineMismatch {
            expected: payload.machine_id.clone(),
            actual: current_machine_id,
        });
    }

    // 到期判定：按日期解析比较（而非字符串比较），并对非法日期直接判为无效，
    // 避免 "2026-1-5" 这类非零填充日期被误判。
    let today = chrono::Utc::now().date_naive();
    match chrono::NaiveDate::parse_from_str(payload.expiry_date.trim(), "%Y-%m-%d") {
        Ok(expiry) => {
            if expiry < today {
                return Err(LicenseError::Expired {
                    expiry_date: payload.expiry_date.clone(),
                });
            }
        }
        Err(_) => {
            return Err(LicenseError::InvalidFormat(format!(
                "expiry_date 不是合法日期（期望 YYYY-MM-DD）: {}",
                payload.expiry_date
            )));
        }
    }

    Ok(payload)
}

fn parse_public_key() -> Result<RsaPublicKey, LicenseError> {
    let key = RsaPublicKey::from_public_key_pem(BUILTIN_PUBLIC_KEY_PEM)
        .map_err(|e| LicenseError::Internal(format!("公钥解析: {}", e)))?;
    Ok(key)
}

fn verify_signature(
    public_key: &RsaPublicKey,
    payload_bytes: &[u8],
    signature_bytes: &[u8],
) -> Result<(), LicenseError> {
    use rsa::pkcs1v15::Signature;
    let signature = Signature::try_from(signature_bytes)
        .map_err(|_| LicenseError::SignatureInvalid)?;
    let verifying_key = rsa::pkcs1v15::VerifyingKey::<Sha256>::new(public_key.clone());
    verifying_key
        .verify(payload_bytes, &signature)
        .map_err(|_| LicenseError::SignatureInvalid)?;
    Ok(())
}
