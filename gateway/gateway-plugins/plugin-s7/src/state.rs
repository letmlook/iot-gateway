//! S7 插件运行时状态：每个节点的连接状态 + 指数退避。
//!
//! ConnectionState 遵循 plugin-modbus-tcp 的模式：
//! - 基数 500ms、封顶 30s、指数增长（2^n）
//! - 连接/读写成功 → on_success() 清零
//! - 连接/读写出错 → on_failure() 丢弃连接并退避

use gateway_sdk::{Group, Tag};

/// 单次连接失败后的退避上限（毫秒）
#[cfg(feature = "s7-client")]
const MAX_BACKOFF_MS: u64 = 30_000;
/// 退避基数（毫秒）
#[cfg(feature = "s7-client")]
const BACKOFF_BASE_MS: u64 = 500;

/// S7 长连接与重连退避状态（s7-client feature 开启时存在）
#[cfg(feature = "s7-client")]
pub struct ConnectionState {
    /// 复用的 S7Client 连接（使用 TcpTransport）
    pub client: Option<snap7_client::S7Client<snap7_client::transport::TcpTransport>>,
    /// 连续失败次数
    pub failures: u32,
    /// 退避期内最早可重试时间
    pub next_retry_at: Option<std::time::Instant>,
}

#[cfg(feature = "s7-client")]
impl ConnectionState {
    pub fn new() -> Self {
        Self {
            client: None,
            failures: 0,
            next_retry_at: None,
        }
    }

    /// 连接/读写成功：清空失败计数与退避
    pub fn on_success(&mut self) {
        self.failures = 0;
        self.next_retry_at = None;
    }

    /// 连接/读写出错：丢弃连接、累加失败计数并按指数退避
    pub fn on_failure(&mut self) {
        self.client = None;
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

#[cfg(feature = "s7-client")]
impl Default for ConnectionState {
    fn default() -> Self {
        Self::new()
    }
}

/// 每个 S7 节点的运行时状态
pub struct S7NodeState {
    /// PLC 地址
    pub host: String,
    /// 端口（默认 102）
    pub port: u16,
    /// 连接协议：s7comm 或 s7comm-plus
    pub protocol: String,
    /// 机架号（0-15）
    pub rack: u16,
    /// 槽位号（0-31）
    pub slot: u16,
    /// 连接超时（毫秒）
    pub connection_timeout_ms: u64,
    /// 读超时（毫秒）
    pub read_timeout_ms: u64,
    /// 默认采集组
    pub groups: Vec<Group>,
    /// 默认点位列表（open 时创建）
    pub tags: Vec<Tag>,
    /// 连接与退避状态
    #[cfg(feature = "s7-client")]
    pub conn: tokio::sync::Mutex<ConnectionState>,
}

#[cfg(feature = "s7-client")]
impl S7NodeState {
    #[allow(dead_code)]
    pub fn new(
        host: String,
        port: u16,
        protocol: String,
        rack: u16,
        slot: u16,
        connection_timeout_ms: u64,
        read_timeout_ms: u64,
    ) -> Self {
        Self {
            host,
            port,
            protocol,
            rack,
            slot,
            connection_timeout_ms,
            read_timeout_ms,
            groups: vec![Group {
                id: GroupId::new(),
                name: "default".to_string(),
                interval_ms: 1000,
                description: Some("默认采集组".to_string()),
            }],
            tags: vec![],
            conn: tokio::sync::Mutex::new(ConnectionState::new()),
        }
    }
}

// GroupId is re-exported from gateway_sdk
use gateway_sdk::GroupId;

#[cfg(all(test, feature = "s7-client"))]
mod tests {
    use super::*;

    #[test]
    fn failure_sets_backoff_and_success_clears() {
        let mut c = ConnectionState::new();
        assert_eq!(c.backoff_remaining_ms(), 0);

        c.on_failure();
        assert_eq!(c.failures, 1);
        let first = c.backoff_remaining_ms();
        assert!(first > 0, "first failure should introduce backoff");
        assert!(c.client.is_none(), "failed connection must be dropped");

        c.on_failure();
        assert_eq!(c.failures, 2);
        assert!(
            c.backoff_remaining_ms() > first,
            "backoff must grow with consecutive failures"
        );

        // 封顶测试
        for _ in 0..20 {
            c.on_failure();
        }
        assert!(
            c.backoff_remaining_ms() <= MAX_BACKOFF_MS + 100,
            "backoff is capped at MAX_BACKOFF_MS"
        );

        c.on_success();
        assert_eq!(c.failures, 0);
        assert_eq!(c.backoff_remaining_ms(), 0);
    }
}
