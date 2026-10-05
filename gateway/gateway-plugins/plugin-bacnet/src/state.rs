//! BACnet 插件运行时状态。

use gateway_sdk::{Group, Tag};

/// UDP 无连接，用连续失败计数代替 ConnectionState。
/// 连续失败 ≥ RECONNECT_THRESHOLD 次后丢弃并重建客户端。
const RECONNECT_THRESHOLD: u32 = 3;

/// 每个节点的运行时状态
pub struct BacnetState {
    pub ip: String,
    pub port: u16,
    pub device_instance: Option<u32>,
    pub timeout_ms: u64,
    pub write_priority: u8,
    pub groups: Vec<Group>,
    pub tags: Vec<Tag>,
    /// 客户端连续失败计数（≥3 后丢弃重建）
    pub failure_count: u32,
    /// BacnetClient（Arc<Mutex<Option<...>>> 形式）
    /// 每次 FFI 调用时在 spawn_blocking 内取走，用完即放
    #[cfg(feature = "bacnet-client")]
    pub client: std::sync::Arc<std::sync::Mutex<Option<bacnet_rs::client::BacnetClient>>>,
}

impl BacnetState {
    /// 记录一次失败，连续 ≥3 次则触发重建
    pub fn record_failure(&mut self) -> bool {
        self.failure_count += 1;
        #[cfg(feature = "bacnet-client")]
        {
            // 丢弃客户端，下轮 poll 时重建
            if let Ok(mut guard) = self.client.lock() {
                *guard = None;
            }
        }
        self.failure_count >= RECONNECT_THRESHOLD
    }

    /// 成功时清零计数
    pub fn record_success(&mut self) {
        self.failure_count = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_count_increments() {
        let mut s = BacnetState {
            ip: "127.0.0.1".to_string(),
            port: 47808,
            device_instance: None,
            timeout_ms: 3000,
            write_priority: 8,
            groups: vec![],
            tags: vec![],
            failure_count: 0,
            #[cfg(feature = "bacnet-client")]
            client: std::sync::Arc::new(std::sync::Mutex::new(None)),
        };
        assert_eq!(s.failure_count, 0);
        s.record_failure();
        assert_eq!(s.failure_count, 1);
        s.record_failure();
        assert_eq!(s.failure_count, 2);
        // 第三次会触发重建
        let should_reconnect = s.record_failure();
        assert_eq!(s.failure_count, 3);
        assert!(should_reconnect);
    }

    #[test]
    fn success_resets_count() {
        let mut s = BacnetState {
            ip: "127.0.0.1".to_string(),
            port: 47808,
            device_instance: None,
            timeout_ms: 3000,
            write_priority: 8,
            groups: vec![],
            tags: vec![],
            failure_count: 2,
            #[cfg(feature = "bacnet-client")]
            client: std::sync::Arc::new(std::sync::Mutex::new(None)),
        };
        s.record_success();
        assert_eq!(s.failure_count, 0);
    }
}
