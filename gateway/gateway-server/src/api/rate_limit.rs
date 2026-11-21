//! 写路径速率限制（T3，2026-11-21 批次）。
//!
//! 设计要点：
//! - **只限写方法**（POST/PUT/PATCH/DELETE）。探针与页面数据都是 GET，高频访问合法，
//!   不设限；读路径的惩罚应落在缓存/查询超时上，不在这里。
//! - **固定窗口计数**（分钟窗），实现简单、无外部依赖、判定 O(1)。
//!   管理台是低频人机交互场景，令牌桶的平滑性收益远小于复杂度成本。
//! - **身份维度**：认证请求按 `AuthContext` 的 username 计数；无身份的写路径
//!   （白名单中的 auth/login）按客户端 IP 计数——登录已有 per-username 账号锁，
//!   per-IP 限流是防爆破的第二道独立闸门。
//! - **fail-open 边界**：仅当限流器自身故障（理论上不会，纯内存）才放行；
//!   超限一律 429 + `Retry-After`，前端按 code=rate_limited 提示。
//! - 窗口条目惰性过期；键数超 `MAX_WINDOWS` 时全表清扫过期项，防匿名 IP 键膨胀。

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use axum::extract::{ConnectInfo, Request, State};
use axum::http::{header, HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

use crate::api::AuthContext;
use crate::state::AppState;

/// 窗口表上限：超过即触发一次全表过期清扫（正常远达不到：键=身份/IP 数）
const MAX_WINDOWS: usize = 16_384;

#[derive(Debug)]
struct Window {
    start_ms: i64,
    count: u32,
}

pub struct RateLimiter {
    /// 写路径限流（次/分钟/身份）；0 = 关闭
    writes_per_min: u32,
    /// 登录限流（次/分钟/IP）；0 = 关闭
    login_per_min: u32,
    windows: Mutex<HashMap<String, Window>>,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

impl RateLimiter {
    pub fn new(writes_per_min: u32, login_per_min: u32) -> Self {
        Self {
            writes_per_min,
            login_per_min,
            windows: Mutex::new(HashMap::new()),
        }
    }

    /// 固定窗口计数。返回 `Some(retry_after)` 表示超限（调用方应回 429）。
    /// `limit == 0` 表示该维度关闭，恒放行。
    fn check(&self, key: &str, limit: u32, now: i64) -> Option<Duration> {
        if limit == 0 {
            return None;
        }
        let mut windows = self.windows.lock().unwrap_or_else(|e| e.into_inner());
        let w = windows.entry(key.to_string()).or_insert(Window {
            start_ms: now,
            count: 0,
        });
        // 分钟窗口过期：重开新窗口
        if now - w.start_ms >= 60_000 {
            w.start_ms = now;
            w.count = 0;
        }
        w.count += 1;
        if w.count > limit {
            let retry_after = Duration::from_millis((60_000 - (now - w.start_ms)).max(0) as u64);
            return Some(retry_after);
        }
        // 键数超限：清扫已过期窗口（防止匿名 IP 键无限累积）
        if windows.len() > MAX_WINDOWS {
            windows.retain(|_, w| now - w.start_ms < 60_000);
        }
        None
    }

    /// 写路径（非登录）：按身份限流
    fn check_write(&self, identity: &str) -> Option<Duration> {
        let now = now_ms();
        self.check(&format!("write:{identity}"), self.writes_per_min, now)
    }

    /// 登录：按 IP 限流
    fn check_login(&self, ip: &str) -> Option<Duration> {
        let now = now_ms();
        self.check(&format!("login:{ip}"), self.login_per_min, now)
    }
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct RateLimitBody {
    pub code: String,
    pub message: String,
}

fn too_many_requests(retry_after: Duration) -> Response {
    let secs = retry_after.as_secs().max(1);
    (
        StatusCode::TOO_MANY_REQUESTS,
        [(header::RETRY_AFTER, HeaderValue::from(secs))],
        axum::Json(RateLimitBody {
            code: "rate_limited".into(),
            message: format!("too many requests, retry after {secs}s"),
        }),
    )
        .into_response()
}

/// 限流中间件：挂在 auth_middleware **内层**（洋葱序 auth → rate_limit → handler），
/// 认证后的请求可读 AuthContext.username 作身份；白名单的 auth/login 无身份，按 IP。
pub async fn rate_limit_middleware(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    let method = request.method().clone();
    // 读方法与未知方法直接放行
    if !matches!(
        method,
        Method::POST | Method::PUT | Method::PATCH | Method::DELETE
    ) {
        return next.run(request).await;
    }
    let path = crate::api::normalize_api_path(request.uri().path());
    let is_login = path == "auth/login" || path == "login";

    let retry_after = if is_login {
        let ip = request
            .extensions()
            .get::<ConnectInfo<std::net::SocketAddr>>()
            .map(|c| c.0.ip().to_string())
            .unwrap_or_else(|| "unknown".into());
        state.rate_limiter.check_login(&ip)
    } else {
        let identity = request
            .extensions()
            .get::<AuthContext>()
            .and_then(|ctx| ctx.username.clone())
            .unwrap_or_else(|| "anonymous".into());
        state.rate_limiter.check_write(&identity)
    };

    match retry_after {
        Some(d) => {
            tracing::warn!("rate limit exceeded: path={} login={}", path, is_login);
            too_many_requests(d)
        }
        None => next.run(request).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_after_limit_and_recovers_next_window() {
        let rl = RateLimiter::new(3, 10);
        for _ in 0..3 {
            assert!(
                rl.check_write("alice").is_none(),
                "within limit should pass"
            );
        }
        assert!(rl.check_write("alice").is_some(), "over limit should block");
        // 不同身份互不影响
        assert!(rl.check_write("bob").is_none());
        // 时间前移一个窗口后恢复
        let now = now_ms();
        assert!(rl.check("write:alice", 3, now + 61_000).is_none());
    }

    #[test]
    fn zero_limit_disables_checking() {
        let rl = RateLimiter::new(0, 0);
        for _ in 0..1000 {
            assert!(rl.check_write("alice").is_none());
        }
    }

    #[test]
    fn login_limit_is_per_ip() {
        let rl = RateLimiter::new(0, 2);
        assert!(rl.check_login("10.0.0.1").is_none());
        assert!(rl.check_login("10.0.0.1").is_none());
        assert!(rl.check_login("10.0.0.1").is_some());
        assert!(rl.check_login("10.0.0.2").is_none(), "other ip unaffected");
    }
}
