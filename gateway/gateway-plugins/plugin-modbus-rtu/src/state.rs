//! Modbus RTU 插件运行时状态。

use gateway_sdk::{Group, Tag};

/// 串口连接缓存（D 级审计 2026-11-22：此前每轮 poll_group 重开串口——
/// Unix 上 open() 有毫秒级开销，Windows 上 COM 口独占句柄反复分配还会泄漏风险）。
/// 持久化的 Context 失败即置 None，下一轮 poll 重建（自愈重连）。
#[cfg(feature = "modbus-client")]
pub struct SerialConn {
    pub ctx: tokio_modbus::client::Context,
    /// 建立时记录的连接参数；setting 变更后不匹配则强制重建
    pub port: String,
    pub baud_rate: u32,
    pub slave_id: u8,
}

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
