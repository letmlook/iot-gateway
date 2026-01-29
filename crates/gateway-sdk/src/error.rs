//! 插件错误类型，对标 Neuron 驱动错误码与语义。

use serde::{Deserialize, Serialize};
use std::fmt;
use thiserror::Error;

pub type PluginResult<T> = Result<T, PluginError>;

/// 插件错误码（对标 Neuron，便于 API 与日志）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginErrorCode {
    /// 通用错误
    Unknown,
    /// 配置无效（格式、必填、范围等）
    ConfigInvalid,
    /// 点位校验失败（地址、类型、正则等）
    TagInvalid,
    /// 连接失败（设备/ Broker 不可达等）
    ConnectionFailed,
    /// 超时
    Timeout,
    /// 操作不支持（如写操作）
    NotSupported,
    /// 内部 / IO 错误
    Io,
    /// 校验失败（validate_tag 等）
    ValidationFailed,
}

impl fmt::Display for PluginErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PluginErrorCode::Unknown => write!(f, "unknown"),
            PluginErrorCode::ConfigInvalid => write!(f, "config_invalid"),
            PluginErrorCode::TagInvalid => write!(f, "tag_invalid"),
            PluginErrorCode::ConnectionFailed => write!(f, "connection_failed"),
            PluginErrorCode::Timeout => write!(f, "timeout"),
            PluginErrorCode::NotSupported => write!(f, "not_supported"),
            PluginErrorCode::Io => write!(f, "io"),
            PluginErrorCode::ValidationFailed => write!(f, "validation_failed"),
        }
    }
}

#[derive(Error, Debug)]
pub enum PluginError {
    #[error("{code}: {message}")]
    WithCode {
        code: PluginErrorCode,
        message: String,
    },
    #[error("plugin error: {0}")]
    Message(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

impl PluginError {
    pub fn msg(s: impl Into<String>) -> Self {
        PluginError::Message(s.into())
    }

    pub fn config_invalid(s: impl Into<String>) -> Self {
        PluginError::WithCode {
            code: PluginErrorCode::ConfigInvalid,
            message: s.into(),
        }
    }

    pub fn tag_invalid(s: impl Into<String>) -> Self {
        PluginError::WithCode {
            code: PluginErrorCode::TagInvalid,
            message: s.into(),
        }
    }

    pub fn connection_failed(s: impl Into<String>) -> Self {
        PluginError::WithCode {
            code: PluginErrorCode::ConnectionFailed,
            message: s.into(),
        }
    }

    pub fn timeout(s: impl Into<String>) -> Self {
        PluginError::WithCode {
            code: PluginErrorCode::Timeout,
            message: s.into(),
        }
    }

    pub fn not_supported(s: impl Into<String>) -> Self {
        PluginError::WithCode {
            code: PluginErrorCode::NotSupported,
            message: s.into(),
        }
    }

    pub fn validation_failed(s: impl Into<String>) -> Self {
        PluginError::WithCode {
            code: PluginErrorCode::ValidationFailed,
            message: s.into(),
        }
    }

    pub fn code(&self) -> PluginErrorCode {
        match self {
            PluginError::WithCode { code, .. } => *code,
            PluginError::Message(_) => PluginErrorCode::Unknown,
            PluginError::Io(_) => PluginErrorCode::Io,
        }
    }

    pub fn message(&self) -> String {
        match self {
            PluginError::WithCode { message, .. } => message.clone(),
            PluginError::Message(s) => s.clone(),
            PluginError::Io(e) => e.to_string(),
        }
    }
}
