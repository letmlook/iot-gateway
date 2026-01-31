//! Modbus TCP 插件运行时状态。

use gateway_sdk::{Group, Tag};

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
}
