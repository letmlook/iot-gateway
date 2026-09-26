//! Modbus TCP 插件运行时状态。

use gateway_sdk::{Group, Tag};

/// 单次连接失败后的退避上限（毫秒）
#[cfg(feature = "modbus-client")]
const MAX_BACKOFF_MS: u64 = 30_000;
/// 退避基数（毫秒）：第 n 次连续失败等待 `min(BASE * 2^(n-1), MAX)`
#[cfg(feature = "modbus-client")]
const BACKOFF_BASE_MS: u64 = 500;

/// 长连接与重连退避。
///
/// 之前每轮采集都要重新 `TcpStream::connect`，100 点位 × 1s 周期会产生每秒上百次建连；
/// 现在复用同一条连接，仅在出错时丢弃并指数退避重建。
#[cfg(feature = "modbus-client")]
pub struct ConnectionState {
    /// 复用的连接；为 None 表示尚未建立或已失效
    pub conn: Option<tokio_modbus::client::Context>,
    /// 连续失败次数
    pub failures: u32,
    /// 退避期内最早可重试时间
    pub next_retry_at: Option<std::time::Instant>,
}

#[cfg(feature = "modbus-client")]
impl ConnectionState {
    pub fn new() -> Self {
        Self {
            conn: None,
            failures: 0,
            next_retry_at: None,
        }
    }

    /// 连接/读写成功：清空失败计数与退避
    pub fn on_success(&mut self) {
        self.failures = 0;
        self.next_retry_at = None;
    }

    /// 连接/读写出错：丢弃连接、累加失败计数并按指数退避推迟下次重试
    pub fn on_failure(&mut self) {
        self.conn = None;
        self.failures = self.failures.saturating_add(1);
        let backoff = BACKOFF_BASE_MS
            .saturating_mul(1u64 << self.failures.min(6))
            .min(MAX_BACKOFF_MS);
        self.next_retry_at =
            Some(std::time::Instant::now() + std::time::Duration::from_millis(backoff));
    }

    /// 退避剩余时间（毫秒）；为 0 表示可以立即重试
    pub fn backoff_remaining_ms(&self) -> u64 {
        match self.next_retry_at {
            Some(t) => t
                .saturating_duration_since(std::time::Instant::now())
                .as_millis() as u64,
            None => 0,
        }
    }
}

#[cfg(feature = "modbus-client")]
impl Default for ConnectionState {
    fn default() -> Self {
        Self::new()
    }
}

pub struct ModbusTcpState {
    pub host: String,
    pub port: u16,
    pub slave_id: u8,
    pub connection_timeout_ms: u64,
    pub send_interval_ms: u64,
    #[allow(dead_code)]
    pub max_retry_times: u32,
    #[allow(dead_code)]
    pub retry_interval_ms: u64,
    pub start_address: u8,
    pub groups: Vec<Group>,
    pub tags: Vec<Tag>,
    /// 长连接与退避状态（未启用 modbus-client feature 时不存在）
    #[cfg(feature = "modbus-client")]
    pub conn: tokio::sync::Mutex<ConnectionState>,
}

#[cfg(all(test, feature = "modbus-client"))]
mod tests {
    use super::*;

    #[test]
    fn failure_sets_exponential_backoff_and_success_resets() {
        let mut c = ConnectionState::new();
        assert_eq!(c.backoff_remaining_ms(), 0, "no backoff before any failure");

        c.on_failure();
        assert_eq!(c.failures, 1);
        let first = c.backoff_remaining_ms();
        assert!(first > 0, "first failure should introduce a wait");
        assert!(c.conn.is_none(), "a failed connection must be dropped");

        c.on_failure();
        assert_eq!(c.failures, 2);
        assert!(
            c.backoff_remaining_ms() > first,
            "backoff must grow with consecutive failures"
        );

        // 连续失败很多次后不会超过上限
        for _ in 0..20 {
            c.on_failure();
        }
        assert!(
            c.backoff_remaining_ms() <= MAX_BACKOFF_MS + 100,
            "backoff is capped"
        );

        c.on_success();
        assert_eq!(c.failures, 0);
        assert_eq!(c.backoff_remaining_ms(), 0, "success clears the backoff window");
    }
}
