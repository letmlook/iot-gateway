//! DL/T645-2007 插件运行时状态。

use gateway_sdk::{Group, Tag};

/// DL/T645-2007 插件运行时状态
pub struct Dlt645State {
    /// 串口设备路径，如 /dev/ttyUSB0
    pub port: String,
    /// 波特率，默认 9600 或 2400（旧电表）
    pub baud: u32,
    /// 数据位，默认 8
    pub data_bits: u8,
    /// 停止位，默认 1
    pub stop_bits: u8,
    /// 校验位，"none" | "even" | "odd"，默认 "none"
    pub parity: String,
    /// 电表地址（6 字节 BCD，通信时低字节在前），如 "000000000000"
    pub meter_id: String,
    /// 读超时 ms
    pub read_timeout_ms: u64,
    /// 发送间隔 ms（电表要求帧间隔）
    pub send_interval_ms: u64,
    /// 最大重试次数
    pub max_retry: u32,
    pub groups: Vec<Group>,
    pub tags: Vec<Tag>,
}
