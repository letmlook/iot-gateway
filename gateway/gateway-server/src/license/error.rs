//! 授权相关错误类型。

use std::fmt;

#[derive(Debug)]
pub enum LicenseError {
    /// 授权文件不存在或不可读
    FileNotFound(std::io::Error),
    /// 文件格式无效（非 UTF-8 或 JSON 解析失败）
    InvalidFormat(String),
    /// 签名验证失败（篡改或错误密钥）
    SignatureInvalid,
    /// 机器码不匹配（非本机授权）
    MachineMismatch { expected: String, actual: String },
    /// 授权已过期
    Expired { expiry_date: String },
    /// 内部错误（解密/哈希等）
    Internal(String),
}

impl fmt::Display for LicenseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LicenseError::FileNotFound(e) => write!(f, "授权文件未找到或不可读: {}", e),
            LicenseError::InvalidFormat(s) => write!(f, "授权格式无效: {}", s),
            LicenseError::SignatureInvalid => write!(f, "授权签名验证失败"),
            LicenseError::MachineMismatch { expected, actual } => {
                write!(f, "机器码不匹配: 授权绑定 {}，当前设备 {}", expected, actual)
            }
            LicenseError::Expired { expiry_date } => write!(f, "授权已过期: {}", expiry_date),
            LicenseError::Internal(s) => write!(f, "授权校验内部错误: {}", s),
        }
    }
}

impl std::error::Error for LicenseError {}
