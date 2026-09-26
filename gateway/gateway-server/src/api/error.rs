//! 统一 API 错误响应：{ code, message }，便于前端按错误码做 i18n。

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;

/// 与 gateway_sdk::PluginErrorCode 及常见 HTTP 错误对应的错误码
const KNOWN_CODES: &[&str] = &[
    "unknown",
    "config_invalid",
    "tag_invalid",
    "connection_failed",
    "timeout",
    "not_supported",
    "io",
    "validation_failed",
];

#[derive(Serialize)]
pub struct ApiErrorBody {
    pub code: String,
    pub message: String,
}

pub struct ApiError {
    pub status: StatusCode,
    pub code: String,
    pub message: String,
}

impl ApiError {
    pub fn new(status: StatusCode, code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            status,
            code: code.into(),
            message: message.into(),
        }
    }

    /// 400：从业务错误字符串解析 code（插件错误格式为 "code: message"）
    pub fn bad_request(msg: impl Into<String>) -> Self {
        let msg = msg.into();
        let (code, message) = parse_plugin_error_message(&msg);
        Self::new(StatusCode::BAD_REQUEST, code, message)
    }

    pub fn not_found(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, "not_found", msg)
    }

    pub fn unauthorized() -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "unauthorized", "Unauthorized")
    }

    pub fn forbidden(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, "forbidden", msg)
    }

    pub fn internal(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", msg)
    }
}

/// 解析插件错误字符串 "code: message"，若前缀为已知 code 则拆分为 (code, message)
fn parse_plugin_error_message(s: &str) -> (String, String) {
    let s = s.trim();
    if let Some(colon) = s.find(": ") {
        let prefix = &s[..colon];
        if KNOWN_CODES.contains(&prefix) {
            return (prefix.to_string(), s[colon + 2..].trim().to_string());
        }
    }
    ("bad_request".to_string(), s.to_string())
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(ApiErrorBody {
                code: self.code,
                message: self.message.clone(),
            }),
        )
            .into_response()
    }
}
