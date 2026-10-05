//! HTTP 客户端构造与响应状态码分类（feature = "http"）。
//!
//! 分类语义：
//! - `Delivered`：2xx → 可重试失败以外的全部情况，计成功
//! - `Retryable`：408 / 425 / 429 / 5xx / 超时 / 连接错误 → 进离线队列按序补发
//! - `Rejected`：其他 4xx → 语义性拒绝，丢弃计数不重试

use std::time::Duration;

#[cfg(feature = "http")]
use reqwest::Client;

/// HTTP 请求结果分类
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpClass {
    /// 2xx：成功
    Delivered,
    /// 4xx（不含 408/425/429）+ 其他：语义性拒绝，丢弃不重试
    Rejected,
    /// 408 / 425 / 429 / 5xx / 超时 / 连接错误：可重试，进离线队列
    Retryable,
}

impl HttpClass {
    /// 根据 HTTP 状态码分类
    pub fn from_status_code(code: u16) -> Self {
        if code == 408 || code == 425 || code == 429 {
            // 408 Request Timeout / 425 Too Early / 429 Too Many Requests → 可重试
            HttpClass::Retryable
        } else if (400..=499).contains(&code) {
            // 其他 4xx → 语义拒绝，不重试
            HttpClass::Rejected
        } else if (500..=599).contains(&code) {
            // 5xx → 服务端错误，可重试
            HttpClass::Retryable
        } else {
            // 1xx / 3xx / 以外 → 视为成功（不常见）
            HttpClass::Delivered
        }
    }
}

/// HTTP 客户端配置参数
#[derive(Debug, Clone)]
pub struct HttpClientConfig {
    pub timeout_ms: u64,
    pub insecure_skip_verify: bool,
    pub ca_file: Option<String>,
    pub user_agent: String,
}

impl Default for HttpClientConfig {
    fn default() -> Self {
        Self {
            timeout_ms: 5000,
            insecure_skip_verify: false,
            ca_file: None,
            user_agent: "iot-gateway-north/1.0".to_string(),
        }
    }
}

#[cfg(feature = "http")]
pub fn build_client(cfg: &HttpClientConfig) -> Result<Client, String> {
    let mut builder = Client::builder()
        .timeout(Duration::from_millis(cfg.timeout_ms))
        .user_agent(&cfg.user_agent);

    if cfg.insecure_skip_verify {
        builder = builder.danger_accept_invalid_certs(true);
    }

    if let Some(ca_path) = &cfg.ca_file {
        let ca_data = std::fs::read(ca_path).map_err(|e| format!("read ca_file failed: {}", e))?;
        let cert = reqwest::Certificate::from_pem(&ca_data)
            .or_else(|_| reqwest::Certificate::from_der(&ca_data))
            .map_err(|e| format!("parse ca_file failed: {}", e))?;
        builder = builder.add_root_certificate(cert);
    }

    builder
        .build()
        .map_err(|e| format!("build reqwest client failed: {}", e))
}

#[cfg(feature = "http")]
pub async fn send_and_classify(
    client: &Client,
    method: reqwest::Method,
    url: &str,
    headers: &[(String, String)],
    body: Option<Vec<u8>>,
) -> Result<HttpClass, String> {
    let mut req = client.request(method, url);
    for (k, v) in headers {
        req = req.header(k.as_str(), v.as_str());
    }
    if let Some(b) = body {
        req = req.body(b);
    }
    match req.send().await {
        Ok(resp) => Ok(HttpClass::from_status_code(resp.status().as_u16())),
        Err(e) => {
            if e.is_timeout() || e.is_connect() {
                Ok(HttpClass::Retryable)
            } else {
                // 其他错误（构建错误等）归类为拒绝
                Ok(HttpClass::Rejected)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_code_classification() {
        // Delivered
        assert_eq!(HttpClass::from_status_code(200), HttpClass::Delivered);
        assert_eq!(HttpClass::from_status_code(204), HttpClass::Delivered);
        // Retryable
        assert_eq!(HttpClass::from_status_code(408), HttpClass::Retryable);
        assert_eq!(HttpClass::from_status_code(425), HttpClass::Retryable);
        assert_eq!(HttpClass::from_status_code(429), HttpClass::Retryable);
        assert_eq!(HttpClass::from_status_code(500), HttpClass::Retryable);
        assert_eq!(HttpClass::from_status_code(502), HttpClass::Retryable);
        assert_eq!(HttpClass::from_status_code(503), HttpClass::Retryable);
        // Rejected
        assert_eq!(HttpClass::from_status_code(400), HttpClass::Rejected);
        assert_eq!(HttpClass::from_status_code(401), HttpClass::Rejected);
        assert_eq!(HttpClass::from_status_code(403), HttpClass::Rejected);
        assert_eq!(HttpClass::from_status_code(404), HttpClass::Rejected);
        // 1xx / 3xx → Delivered
        assert_eq!(HttpClass::from_status_code(100), HttpClass::Delivered);
        assert_eq!(HttpClass::from_status_code(301), HttpClass::Delivered);
    }
}
