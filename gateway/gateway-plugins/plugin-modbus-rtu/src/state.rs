//! Modbus RTU 插件运行时状态。

use gateway_sdk::{Group, Tag};

pub struct ModbusRtuState {
    pub port: String,
    pub baud_rate: u32,
    pub data_bits: u8,
    pub stop_bits: u8,
    pub parity: String,
    pub slave_id: u8,
    #[allow(dead_code)]
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
