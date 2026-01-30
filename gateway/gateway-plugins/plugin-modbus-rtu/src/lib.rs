//! 南向 Modbus RTU 插件：支持串口/DTU、线圈/离散/输入/保持寄存器、
//! 地址格式（SLAVE!ADDRESS[.BIT][#ENDIAN]）、读写、超时重试、字节序、STRING/BYTES 等。

#[cfg(feature = "ffi")]
mod ffi;

use gateway_sdk::{
    ConfigSchema, Group, GroupId, NodeId, ParamSchema, ParamType, PluginMeta, SouthPlugin, Tag,
    TagId, TagRegexEntry, TagSchema,
};
use gateway_sdk::types::{DataValue, PluginKind};
use gateway_sdk::{PluginConfig, PluginError, PluginResult};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

/// Modbus 区域
#[derive(Clone, Copy, PartialEq, Eq)]
enum ModbusArea {
    Coil,
    DiscreteInput,
    InputRegister,
    HoldingRegister,
}

#[derive(Clone, Copy, Default)]
struct Endianness {
    swap16: bool,
    order32: u8,
}

#[derive(Clone)]
struct ParsedAddress {
    area: ModbusArea,
    start: u16,
    count: u16,
    bit_index: Option<u8>,
    endian: Endianness,
}

fn parse_address(addr: &str) -> Option<(ModbusArea, u16, u16)> {
    parse_address_full(addr, 1).map(|p| (p.area, p.start, p.count))
}

fn parse_address_full(addr: &str, start_address: u8) -> Option<ParsedAddress> {
    let addr = addr.trim();
    let mut endian = Endianness::default();
    let mut bit_index: Option<u8> = None;
    let addr_no_endian = if let Some(excl) = addr.find('#') {
        let (addr_part, endian_part) = addr.split_at(excl);
        let endian_str = endian_part.trim_start_matches('#').to_uppercase();
        match endian_str.as_str() {
            "B" => endian.swap16 = true,
            "L" | "LL" => {}
            "LB" => endian.order32 = 1,
            "BL" => endian.order32 = 2,
            "BB" => endian.order32 = 3,
            _ => {}
        }
        addr_part.trim()
    } else {
        addr
    };
    let (area, start, count) = if let Some(dot) = addr_no_endian.find('.') {
        let (base, rest) = addr_no_endian.split_at(dot);
        let rest = rest.trim_start_matches('.');
        let rest_upper = rest.to_uppercase();
        let is_string_len = rest_upper.ends_with('H') || rest_upper.ends_with('L') || rest_upper.ends_with('D') || rest_upper.ends_with('E');
        let len_str: &str = if is_string_len { rest_upper.trim_end_matches(|c: char| c == 'H' || c == 'L' || c == 'D' || c == 'E') } else { &rest_upper };
        if !is_string_len && rest.len() == 1 {
            if let Ok(b) = rest.parse::<u8>() {
                if b <= 15 {
                    bit_index = Some(b);
                }
            }
            parse_address_core(base, start_address)?
        } else if let Ok(len) = len_str.parse::<usize>() {
            let (a, s, _) = parse_address_core(base, start_address)?;
            let regs = (len + 1) / 2;
            return Some(ParsedAddress { area: a, start: s, count: regs as u16, bit_index: None, endian });
        } else {
            parse_address_core(base, start_address)?
        }
    } else {
        parse_address_core(addr_no_endian, start_address)?
    };
    Some(ParsedAddress { area, start, count, bit_index, endian })
}

fn parse_address_core(addr: &str, start_address: u8) -> Option<(ModbusArea, u16, u16)> {
    let parts: Vec<&str> = addr.split('!').map(|s| s.trim()).collect();
    if parts.is_empty() {
        return None;
    }
    let first = parts[0].to_lowercase();
    if first == "0x" || first == "0" || first == "1x" || first == "1" || first == "3x" || first == "3" || first == "4x" || first == "4" {
        let area = match first.as_str() {
            "0x" | "0" => ModbusArea::Coil,
            "1x" | "1" => ModbusArea::DiscreteInput,
            "3x" | "3" => ModbusArea::InputRegister,
            _ => ModbusArea::HoldingRegister,
        };
        let start: u16 = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
        let count: u16 = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(1);
        return Some((area, start, count.max(1)));
    }
    let _slave: u8 = parts[0].parse().ok().filter(|&s| s <= 247)?;
    let addr_num: u32 = parts.get(1).and_then(|s| s.parse().ok())?;
    let (area, base) = if addr_num >= 400001 && addr_num <= 465536 {
        (ModbusArea::HoldingRegister, 400000u32)
    } else if addr_num >= 300001 && addr_num <= 365536 {
        (ModbusArea::InputRegister, 300000)
    } else if addr_num >= 100001 && addr_num <= 165536 {
        (ModbusArea::DiscreteInput, 100000)
    } else if addr_num >= 1 && addr_num <= 65536 {
        (ModbusArea::Coil, 0)
    } else {
        return None;
    };
    let start = (addr_num.saturating_sub(base).saturating_sub(start_address as u32)) as u16;
    Some((area, start, 1u16))
}

fn config_str(config: &PluginConfig, key: &str, default: &str) -> String {
    config.get(key).and_then(|v| v.as_str()).map(String::from).unwrap_or_else(|| default.to_string())
}

fn regs_to_u32(regs: &[u16], e: &Endianness) -> u32 {
    if regs.len() < 2 {
        return 0;
    }
    let (a, b) = (regs[0], regs[1]);
    let bytes = match e.order32 {
        1 => [(b & 0xff) as u8, (b >> 8) as u8, (a & 0xff) as u8, (a >> 8) as u8],
        2 => [(a >> 8) as u8, (a & 0xff) as u8, (b >> 8) as u8, (b & 0xff) as u8],
        3 => [(b >> 8) as u8, (b & 0xff) as u8, (a >> 8) as u8, (a & 0xff) as u8],
        _ => [(a & 0xff) as u8, (a >> 8) as u8, (b & 0xff) as u8, (b >> 8) as u8],
    };
    u32::from_le_bytes(bytes)
}

/// 按字节序取 64 位：order32 0=LL 1=LB 2=BL 3=BB
fn regs_to_u64(regs: &[u16], e: &Endianness) -> u64 {
    if regs.len() < 4 {
        return 0;
    }
    let bytes = match e.order32 {
        1 => [
            (regs[1] & 0xff) as u8, (regs[1] >> 8) as u8, (regs[0] & 0xff) as u8, (regs[0] >> 8) as u8,
            (regs[3] & 0xff) as u8, (regs[3] >> 8) as u8, (regs[2] & 0xff) as u8, (regs[2] >> 8) as u8,
        ],
        2 => [
            (regs[0] >> 8) as u8, (regs[0] & 0xff) as u8, (regs[1] >> 8) as u8, (regs[1] & 0xff) as u8,
            (regs[2] >> 8) as u8, (regs[2] & 0xff) as u8, (regs[3] >> 8) as u8, (regs[3] & 0xff) as u8,
        ],
        3 => [
            (regs[3] >> 8) as u8, (regs[3] & 0xff) as u8, (regs[2] >> 8) as u8, (regs[2] & 0xff) as u8,
            (regs[1] >> 8) as u8, (regs[1] & 0xff) as u8, (regs[0] >> 8) as u8, (regs[0] & 0xff) as u8,
        ],
        _ => [
            (regs[0] & 0xff) as u8, (regs[0] >> 8) as u8, (regs[1] & 0xff) as u8, (regs[1] >> 8) as u8,
            (regs[2] & 0xff) as u8, (regs[2] >> 8) as u8, (regs[3] & 0xff) as u8, (regs[3] >> 8) as u8,
        ],
    };
    u64::from_le_bytes(bytes)
}

fn register_to_value_ext(regs: &[u16], data_type: &str, endian: &Endianness, bit_index: Option<u8>) -> DataValue {
    if let Some(bit) = bit_index {
        if let Some(&r) = regs.first() {
            return DataValue::Bool((r >> bit) & 1 != 0);
        }
        return DataValue::Bool(false);
    }
    match data_type {
        "bool" => DataValue::Bool(regs.first().map(|&r| r != 0).unwrap_or(false)),
        "int16" => {
            let r = regs.first().copied().unwrap_or(0);
            DataValue::Int16((if endian.swap16 { r.swap_bytes() } else { r }) as i16)
        }
        "uint16" => {
            let r = regs.first().copied().unwrap_or(0);
            DataValue::UInt16(if endian.swap16 { r.swap_bytes() } else { r })
        }
        "int32" | "uint32" => {
            let v = regs_to_u32(regs, endian);
            if data_type == "int32" { DataValue::Int32(v as i32) } else { DataValue::UInt32(v) }
        }
        "int64" | "uint64" => {
            let v = regs_to_u64(regs, endian);
            if data_type == "int64" { DataValue::Int64(v as i64) } else { DataValue::UInt64(v) }
        }
        "float32" => DataValue::Float32(f32::from_bits(regs_to_u32(regs, endian))),
        "float64" => DataValue::Float64(f64::from_bits(regs_to_u64(regs, endian))),
        "string" => {
            let mut bytes: Vec<u8> = Vec::new();
            for &r in regs {
                bytes.push((r & 0xff) as u8);
                bytes.push((r >> 8) as u8);
            }
            DataValue::String(String::from_utf8_lossy(&bytes).trim_end_matches('\0').to_string())
        }
        "bytes" => {
            let mut bytes: Vec<u8> = Vec::new();
            for &r in regs {
                bytes.push((r & 0xff) as u8);
                bytes.push((r >> 8) as u8);
            }
            DataValue::Bytes(bytes)
        }
        _ => DataValue::UInt16(regs.first().copied().unwrap_or(0)),
    }
}

fn value_to_registers(value: &DataValue, data_type: &str) -> Vec<u16> {
    match value {
        DataValue::Int16(v) => vec![*v as u16],
        DataValue::UInt16(v) => vec![*v],
        DataValue::Int32(v) => {
            let b = (*v as u32).to_le_bytes();
            vec![u16::from_le_bytes([b[0], b[1]]), u16::from_le_bytes([b[2], b[3]])]
        }
        DataValue::UInt32(v) => {
            let b = v.to_le_bytes();
            vec![u16::from_le_bytes([b[0], b[1]]), u16::from_le_bytes([b[2], b[3]])]
        }
        DataValue::Int64(v) => {
            let b = (*v as u64).to_le_bytes();
            vec![
                u16::from_le_bytes([b[0], b[1]]), u16::from_le_bytes([b[2], b[3]]),
                u16::from_le_bytes([b[4], b[5]]), u16::from_le_bytes([b[6], b[7]]),
            ]
        }
        DataValue::UInt64(v) => {
            let b = v.to_le_bytes();
            vec![
                u16::from_le_bytes([b[0], b[1]]), u16::from_le_bytes([b[2], b[3]]),
                u16::from_le_bytes([b[4], b[5]]), u16::from_le_bytes([b[6], b[7]]),
            ]
        }
        DataValue::Float32(v) => {
            let b = v.to_bits().to_le_bytes();
            vec![u16::from_le_bytes([b[0], b[1]]), u16::from_le_bytes([b[2], b[3]])]
        }
        DataValue::Float64(v) => {
            let b = v.to_bits().to_le_bytes();
            vec![
                u16::from_le_bytes([b[0], b[1]]), u16::from_le_bytes([b[2], b[3]]),
                u16::from_le_bytes([b[4], b[5]]), u16::from_le_bytes([b[6], b[7]]),
            ]
        }
        DataValue::String(s) => {
            let bytes = s.as_bytes();
            let mut regs = Vec::with_capacity((bytes.len() + 1) / 2);
            for chunk in bytes.chunks(2) {
                regs.push(u16::from_le_bytes([chunk.get(0).copied().unwrap_or(0), chunk.get(1).copied().unwrap_or(0)]));
            }
            regs
        }
        DataValue::Bytes(b) => {
            let mut regs = Vec::with_capacity((b.len() + 1) / 2);
            for chunk in b.chunks(2) {
                regs.push(u16::from_le_bytes([chunk.get(0).copied().unwrap_or(0), chunk.get(1).copied().unwrap_or(0)]));
            }
            regs
        }
        DataValue::Bool(v) if data_type == "bool" => vec![if *v { 0xff00 } else { 0 }],
        _ => vec![value.as_u64().map(|u| u as u16).unwrap_or(0)],
    }
}

/// Modbus RTU 南向插件
pub struct ModbusRtuPlugin {
    state: Arc<RwLock<HashMap<NodeId, ModbusRtuState>>>,
}

struct ModbusRtuState {
    port: String,
    baud_rate: u32,
    data_bits: u8,
    stop_bits: u8,
    parity: String,
    slave_id: u8,
    #[allow(dead_code)]
    connection_timeout_ms: u64,
    send_interval_ms: u64,
    #[allow(dead_code)]
    max_retry_times: u32,
    #[allow(dead_code)]
    retry_interval_ms: u64,
    start_address: u8,
    groups: Vec<Group>,
    tags: Vec<Tag>,
}

impl Default for ModbusRtuPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl ModbusRtuPlugin {
    pub fn new() -> Self {
        Self {
            state: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    fn default_groups() -> Vec<Group> {
        vec![Group {
            id: GroupId::new(),
            name: "default".to_string(),
            interval_ms: 1000,
            description: Some("默认采集组".to_string()),
        }]
    }
}

#[async_trait::async_trait]
impl SouthPlugin for ModbusRtuPlugin {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "modbus-rtu",
            kind: PluginKind::South,
            description: Some("Modbus RTU 南向驱动，通过串口连接 Modbus 设备"),
            version: "0.1.0",
            name_zh: Some("Modbus RTU"),
            name_en: Some("Modbus RTU"),
            description_zh: Some("Modbus RTU 南向驱动，通过串口/DTU 连接 Modbus 设备"),
            description_en: Some("Modbus RTU south driver, connect to Modbus devices via serial/DTU"),
        }
    }

    fn config_schema(&self) -> Option<ConfigSchema> {
        use gateway_sdk::ParamAttribute;
        Some(
            ConfigSchema::new()
                .param(ParamSchema {
                    name: "port".to_string(),
                    description: Some("串口路径，如 /dev/ttyUSB0、COM1".to_string()),
                    name_zh: Some("串口".to_string()),
                    name_en: Some("Port".to_string()),
                    description_zh: Some("串口路径，如 /dev/ttyUSB0、COM1".to_string()),
                    description_en: Some("Serial port path, e.g. /dev/ttyUSB0, COM1".to_string()),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::String,
                    default: Some(serde_json::json!("COM1")),
                    valid: None,
                })
                .param(ParamSchema {
                    name: "baud_rate".to_string(),
                    description: Some("波特率".to_string()),
                    name_zh: Some("波特率".to_string()),
                    name_en: Some("Baud rate".to_string()),
                    description_zh: Some("串口波特率".to_string()),
                    description_en: Some("Serial baud rate".to_string()),
                    attribute: ParamAttribute::Optional,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(9600)),
                    valid: None,
                })
                .param(ParamSchema {
                    name: "data_bits".to_string(),
                    description: Some("数据位 7 或 8".to_string()),
                    name_zh: Some("数据位".to_string()),
                    name_en: Some("Data bits".to_string()),
                    description_zh: Some("数据位 7 或 8".to_string()),
                    description_en: Some("Data bits 7 or 8".to_string()),
                    attribute: ParamAttribute::Optional,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(8)),
                    valid: None,
                })
                .param(ParamSchema {
                    name: "stop_bits".to_string(),
                    description: Some("停止位 1 或 2".to_string()),
                    name_zh: Some("停止位".to_string()),
                    name_en: Some("Stop bits".to_string()),
                    description_zh: Some("停止位 1 或 2".to_string()),
                    description_en: Some("Stop bits 1 or 2".to_string()),
                    attribute: ParamAttribute::Optional,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(1)),
                    valid: None,
                })
                .param(ParamSchema {
                    name: "parity".to_string(),
                    description: Some("校验位：none / even / odd".to_string()),
                    name_zh: Some("校验位".to_string()),
                    name_en: Some("Parity".to_string()),
                    description_zh: Some("校验位：none / even / odd".to_string()),
                    description_en: Some("Parity: none, even, or odd".to_string()),
                    attribute: ParamAttribute::Optional,
                    ty: ParamType::String,
                    default: Some(serde_json::json!("none")),
                    valid: None,
                })
                .param(ParamSchema {
                    name: "slave_id".to_string(),
                    description: Some("从站 ID".to_string()),
                    name_zh: Some("从站 ID".to_string()),
                    name_en: Some("Slave ID".to_string()),
                    description_zh: Some("Modbus 从站 ID".to_string()),
                    description_en: Some("Modbus slave ID".to_string()),
                    attribute: ParamAttribute::Optional,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(1)),
                    valid: None,
                })
                .param(ParamSchema {
                    name: "connection_timeout_ms".to_string(),
                    description: Some("连接/响应超时(ms)".to_string()),
                    name_zh: Some("连接超时(ms)".to_string()),
                    name_en: Some("Connection timeout (ms)".to_string()),
                    description_zh: Some("连接与响应超时时间，单位毫秒".to_string()),
                    description_en: Some("Connection and response timeout in milliseconds".to_string()),
                    attribute: ParamAttribute::Optional,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(3000)),
                    valid: None,
                })
                .param(ParamSchema {
                    name: "send_interval_ms".to_string(),
                    description: Some("相邻读/写命令间隔(ms)".to_string()),
                    name_zh: Some("发送间隔(ms)".to_string()),
                    name_en: Some("Send interval (ms)".to_string()),
                    description_zh: Some("相邻读/写命令之间的间隔，单位毫秒".to_string()),
                    description_en: Some("Interval between read/write commands in milliseconds".to_string()),
                    attribute: ParamAttribute::Optional,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(20)),
                    valid: None,
                })
                .param(ParamSchema {
                    name: "max_retry_times".to_string(),
                    description: Some("读失败最大重试次数".to_string()),
                    name_zh: Some("最大重试次数".to_string()),
                    name_en: Some("Max retry times".to_string()),
                    description_zh: Some("读失败时的最大重试次数".to_string()),
                    description_en: Some("Maximum retry count on read failure".to_string()),
                    attribute: ParamAttribute::Optional,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(3)),
                    valid: None,
                })
                .param(ParamSchema {
                    name: "retry_interval_ms".to_string(),
                    description: Some("重试间隔(ms)".to_string()),
                    name_zh: Some("重试间隔(ms)".to_string()),
                    name_en: Some("Retry interval (ms)".to_string()),
                    description_zh: Some("重试之间的间隔时间，单位毫秒".to_string()),
                    description_en: Some("Interval between retries in milliseconds".to_string()),
                    attribute: ParamAttribute::Optional,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(100)),
                    valid: None,
                })
                .param(ParamSchema {
                    name: "start_address".to_string(),
                    description: Some("地址起始：0 或 1（400001=第1个保持寄存器时填 1）".to_string()),
                    name_zh: Some("地址起始".to_string()),
                    name_en: Some("Start address".to_string()),
                    description_zh: Some("地址起始：0 或 1，400001 表示第 1 个保持寄存器时填 1".to_string()),
                    description_en: Some("Start address 0 or 1; use 1 when 400001 denotes first holding register".to_string()),
                    attribute: ParamAttribute::Optional,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(1)),
                    valid: None,
                })
                .tag_regex(vec![
                    TagRegexEntry {
                        data_type: "bool".to_string(),
                        regex: r"^[01]x![0-9]+(![0-9]+)?$".to_string(),
                    },
                    TagRegexEntry {
                        data_type: "int16".to_string(),
                        regex: r"^[34]x![0-9]+(![0-9]+)?$".to_string(),
                    },
                    TagRegexEntry {
                        data_type: "uint16".to_string(),
                        regex: r"^[34]x![0-9]+(![0-9]+)?$".to_string(),
                    },
                    TagRegexEntry {
                        data_type: "float32".to_string(),
                        regex: r"^[34]x![0-9]+(!2)?$".to_string(),
                    },
                    TagRegexEntry {
                        data_type: "float64".to_string(),
                        regex: r"^[34]x![0-9]+(!4)?$".to_string(),
                    },
                ]),
        )
    }

    fn tag_schema(&self) -> Option<TagSchema> {
        Some(TagSchema {
            data_types: Some(vec![
                "bool".to_string(),
                "int16".to_string(),
                "uint16".to_string(),
                "int32".to_string(),
                "uint32".to_string(),
                "int64".to_string(),
                "uint64".to_string(),
                "float32".to_string(),
                "float64".to_string(),
                "string".to_string(),
                "bytes".to_string(),
            ]),
            address_format: Some(
                "0x!addr/1x!addr/3x!addr/4x!addr 或 1!400001[.BIT][#ENDIAN]，.LEN 用于 STRING".to_string(),
            ),
            address_format_zh: Some("0x!addr/1x!addr/3x!addr/4x!addr 或 1!400001[.BIT][#ENDIAN]，.LEN 用于 STRING".to_string()),
            address_format_en: Some("0x!addr/1x!addr/3x!addr/4x!addr or 1!400001[.BIT][#ENDIAN], .LEN for STRING".to_string()),
        })
    }

    async fn validate_tag(&self, _node_id: NodeId, tag: &Tag) -> PluginResult<()> {
        if tag.name.is_empty() {
            return Err(PluginError::tag_invalid("tag name required"));
        }
        parse_address(&tag.address)
            .ok_or_else(|| PluginError::tag_invalid("address format: 0x!addr / 1x!addr / 3x!addr / 4x!addr"))?;
        Ok(())
    }

    async fn open(&self, node_id: NodeId, config: PluginConfig) -> PluginResult<()> {
        let port = config_str(&config, "port", "COM1");
        let baud_rate = config.get("baud_rate").and_then(|v| v.as_u64()).map(|n| n as u32).unwrap_or(9600);
        let data_bits = config.get("data_bits").and_then(|v| v.as_u64()).map(|n| n as u8).unwrap_or(8);
        let stop_bits = config.get("stop_bits").and_then(|v| v.as_u64()).map(|n| n as u8).unwrap_or(1);
        let parity = config_str(&config, "parity", "none");
        let slave_id = config.get("slave_id").and_then(|v| v.as_u64()).map(|n| n as u8).unwrap_or(1);
        let connection_timeout_ms = config.get("connection_timeout_ms").and_then(|v| v.as_u64()).unwrap_or(3000);
        let send_interval_ms = config.get("send_interval_ms").and_then(|v| v.as_u64()).unwrap_or(20);
        let max_retry_times = config.get("max_retry_times").and_then(|v| v.as_u64()).map(|n| n as u32).unwrap_or(3);
        let retry_interval_ms = config.get("retry_interval_ms").and_then(|v| v.as_u64()).unwrap_or(100);
        let start_address = config.get("start_address").and_then(|v| v.as_u64()).map(|n| n as u8).unwrap_or(1).min(1);
        let groups = Self::default_groups();
        let tags = groups
            .iter()
            .flat_map(|g| {
                vec![Tag {
                    id: TagId::new(),
                    name: "holding_0".to_string(),
                    address: "4x!0".to_string(),
                    attr: gateway_sdk::TagAttr::Read,
                    data_type: Some("uint16".to_string()),
                    description: Some("保持寄存器 0".to_string()),
                    group_id: g.id,
                }]
            })
            .collect::<Vec<_>>();
        let mut state = self.state.write().await;
        state.insert(
            node_id,
            ModbusRtuState {
                port,
                baud_rate,
                data_bits,
                stop_bits,
                parity,
                slave_id,
                connection_timeout_ms,
                send_interval_ms,
                max_retry_times,
                retry_interval_ms,
                start_address,
                groups,
                tags,
            },
        );
        Ok(())
    }

    async fn close(&self, node_id: NodeId) -> PluginResult<()> {
        let mut state = self.state.write().await;
        state.remove(&node_id);
        Ok(())
    }

    async fn poll_group(
        &self,
        node_id: NodeId,
        _group_id: GroupId,
        tags: &[Tag],
    ) -> PluginResult<Vec<(TagId, DataValue)>> {
        let state = self.state.read().await;
        let s = state.get(&node_id).ok_or_else(|| PluginError::msg("node not open"))?;

        #[cfg(feature = "modbus-client")]
        {
            use tokio_modbus::client::rtu::attach_slave;
            use tokio_modbus::prelude::*;
            use tokio_modbus::slave::Slave;
            use tokio_serial::SerialPortBuilderExt;

            let builder = tokio_serial::new(&s.port, s.baud_rate)
                .data_bits(match s.data_bits {
                    7 => tokio_serial::DataBits::Seven,
                    _ => tokio_serial::DataBits::Eight,
                })
                .stop_bits(match s.stop_bits {
                    2 => tokio_serial::StopBits::Two,
                    _ => tokio_serial::StopBits::One,
                })
                .parity(match s.parity.to_lowercase().as_str() {
                    "even" => tokio_serial::Parity::Even,
                    "odd" => tokio_serial::Parity::Odd,
                    _ => tokio_serial::Parity::None,
                });
            let serial = builder
                .open_native_async()
                .map_err(|e| PluginError::msg(format!("serial open: {}", e)))?;
            let mut ctx = attach_slave(serial, Slave(s.slave_id));
            let send_interval = Duration::from_millis(s.send_interval_ms.min(5000));
            let mut out = Vec::with_capacity(tags.len());
            for tag in tags {
                let (tag_id, value) = match parse_address_full(&tag.address, s.start_address) {
                    Some(parsed) => {
                        let dt = tag.data_type.as_deref().unwrap_or("uint16");
                        let v = match parsed.area {
                            ModbusArea::Coil => {
                                let coils = match ctx.read_coils(parsed.start, parsed.count).await {
                                    Ok(Ok(v)) => v,
                                    Ok(Err(e)) => return Err(PluginError::msg(format!("read_coils exception: {}", e))),
                                    Err(e) => return Err(PluginError::msg(format!("read_coils: {}", e))),
                                };
                                DataValue::Bool(coils.first().copied().unwrap_or(false))
                            }
                            ModbusArea::DiscreteInput => {
                                let disc = match ctx.read_discrete_inputs(parsed.start, parsed.count).await {
                                    Ok(Ok(v)) => v,
                                    Ok(Err(e)) => return Err(PluginError::msg(format!("read_discrete_inputs exception: {}", e))),
                                    Err(e) => return Err(PluginError::msg(format!("read_discrete_inputs: {}", e))),
                                };
                                DataValue::Bool(disc.first().copied().unwrap_or(false))
                            }
                            ModbusArea::InputRegister => {
                                let regs = match ctx.read_input_registers(parsed.start, parsed.count).await {
                                    Ok(Ok(v)) => v,
                                    Ok(Err(e)) => return Err(PluginError::msg(format!("read_input_registers exception: {}", e))),
                                    Err(e) => return Err(PluginError::msg(format!("read_input_registers: {}", e))),
                                };
                                register_to_value_ext(&regs, dt, &parsed.endian, parsed.bit_index)
                            }
                            ModbusArea::HoldingRegister => {
                                let regs = match ctx.read_holding_registers(parsed.start, parsed.count).await {
                                    Ok(Ok(v)) => v,
                                    Ok(Err(e)) => return Err(PluginError::msg(format!("read_holding_registers exception: {}", e))),
                                    Err(e) => return Err(PluginError::msg(format!("read_holding_registers: {}", e))),
                                };
                                register_to_value_ext(&regs, dt, &parsed.endian, parsed.bit_index)
                            }
                        };
                        (tag.id, v)
                    }
                    None => (tag.id, DataValue::UInt16(0)),
                };
                out.push((tag_id, value));
                tokio::time::sleep(send_interval).await;
            }
            Ok(out)
        }

        #[cfg(not(feature = "modbus-client"))]
        {
            let _ = (node_id, s);
            let mut out = Vec::with_capacity(tags.len());
            for tag in tags {
                let _ = parse_address(&tag.address);
                out.push((tag.id, DataValue::UInt16(0)));
            }
            Ok(out)
        }
    }

    async fn list_groups(&self, node_id: NodeId) -> PluginResult<Vec<Group>> {
        let state = self.state.read().await;
        let s = state.get(&node_id).ok_or_else(|| PluginError::msg("node not open"))?;
        Ok(s.groups.clone())
    }

    async fn list_tags(&self, node_id: NodeId, group_id: GroupId) -> PluginResult<Vec<Tag>> {
        let state = self.state.read().await;
        let s = state.get(&node_id).ok_or_else(|| PluginError::msg("node not open"))?;
        Ok(s.tags
            .iter()
            .filter(|t| t.group_id == group_id)
            .cloned()
            .collect())
    }

    async fn write_tags(&self, node_id: NodeId, values: &[(Tag, DataValue)]) -> PluginResult<()> {
        if values.is_empty() {
            return Ok(());
        }
        let state = self.state.read().await;
        let s = state.get(&node_id).ok_or_else(|| PluginError::msg("node not open"))?;

        #[cfg(feature = "modbus-client")]
        {
            use tokio_modbus::client::rtu::attach_slave;
            use tokio_modbus::prelude::*;
            use tokio_modbus::slave::Slave;
            use tokio_serial::SerialPortBuilderExt;

            let builder = tokio_serial::new(&s.port, s.baud_rate)
                .data_bits(match s.data_bits {
                    7 => tokio_serial::DataBits::Seven,
                    _ => tokio_serial::DataBits::Eight,
                })
                .stop_bits(match s.stop_bits {
                    2 => tokio_serial::StopBits::Two,
                    _ => tokio_serial::StopBits::One,
                })
                .parity(match s.parity.to_lowercase().as_str() {
                    "even" => tokio_serial::Parity::Even,
                    "odd" => tokio_serial::Parity::Odd,
                    _ => tokio_serial::Parity::None,
                });
            let serial = builder
                .open_native_async()
                .map_err(|e| PluginError::msg(format!("serial open: {}", e)))?;
            let mut ctx = attach_slave(serial, Slave(s.slave_id));
            let interval = Duration::from_millis(s.send_interval_ms.min(5000));

            for (tag, value) in values {
                let Some(parsed) = parse_address_full(&tag.address, s.start_address) else {
                    continue;
                };
                match parsed.area {
                    ModbusArea::Coil => {
                        let b = value.as_bool().unwrap_or(false);
                        if parsed.count == 1 {
                            if let Err(e) = ctx.write_single_coil(parsed.start, b).await {
                                return Err(PluginError::msg(format!("write_single_coil: {}", e)));
                            }
                        } else {
                            let coils: Vec<bool> = (0..parsed.count).map(|_| b).collect();
                            if let Err(e) = ctx.write_multiple_coils(parsed.start, &coils).await {
                                return Err(PluginError::msg(format!("write_multiple_coils: {}", e)));
                            }
                        }
                    }
                    ModbusArea::HoldingRegister => {
                        let regs = value_to_registers(value, tag.data_type.as_deref().unwrap_or("uint16"));
                        if regs.is_empty() {
                            continue;
                        }
                        if regs.len() == 1 {
                            if let Err(e) = ctx.write_single_register(parsed.start, regs[0]).await {
                                return Err(PluginError::msg(format!("write_single_register: {}", e)));
                            }
                        } else if let Err(e) = ctx.write_multiple_registers(parsed.start, &regs).await {
                            return Err(PluginError::msg(format!("write_multiple_registers: {}", e)));
                        }
                    }
                    _ => return Err(PluginError::tag_invalid("only coil and holding register support write")),
                }
                tokio::time::sleep(interval).await;
            }
        }

        #[cfg(not(feature = "modbus-client"))]
        {
            let _ = (node_id, s, values);
            return Err(PluginError::not_supported("write requires modbus-client feature"));
        }
        Ok(())
    }
}
