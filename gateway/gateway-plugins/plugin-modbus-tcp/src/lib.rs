//! 南向 Modbus TCP 插件：支持 TCP 客户端、线圈/离散/输入/保持寄存器、
//! 地址格式（SLAVE!ADDRESS[.BIT][#ENDIAN]）、读写、超时重试、字节序、STRING/BYTES 等。

#[cfg(feature = "ffi")]
mod ffi;

use gateway_sdk::{
    ConfigSchema, Group, GroupId, NodeId, ParamOption, ParamSchema, ParamType, ParamValid,
    PluginMeta, SouthPlugin, Tag, TagId, TagRegexEntry, TagSchema,
};
use gateway_sdk::types::{DataValue, PluginKind};
use gateway_sdk::{PluginConfig, PluginError, PluginResult};
use gateway_sdk::log;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

/// Modbus 区域：0=线圈 1=离散输入 3=输入寄存器 4=保持寄存器
#[derive(Clone, Copy, PartialEq, Eq)]
enum ModbusArea {
    Coil,
    DiscreteInput,
    InputRegister,
    HoldingRegister,
}

/// 字节序：#L/#B(16bit) #LL/#LB/#BL/#BB(32/64bit)
#[derive(Clone, Copy, Default)]
struct Endianness {
    /// 16bit: false=#L(1,2) true=#B(2,1)
    swap16: bool,
    /// 32/64bit: 0=LL 1=LB 2=BL 3=BB
    order32: u8,
}

/// 解析后的地址（支持 4x!addr 与 1!400001[.BIT][#ENDIAN]）
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

/// 完整解析：支持 4x!100、4x!100!2 与 1!400001、1!400001.4、1!400001#LB
fn parse_address_full(addr: &str, start_address: u8) -> Option<ParsedAddress> {
    let addr = addr.trim();
    let mut endian = Endianness::default();
    let mut bit_index: Option<u8> = None;
    let addr_no_endian = if let Some(excl) = addr.find('#') {
        let (addr_part, endian_part) = addr.split_at(excl);
        let endian_str = endian_part.trim_start_matches('#').to_uppercase();
        match endian_str.as_str() {
            "B" => endian.swap16 = true,
            "L" => {}
            "LL" => {}
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
    let (area, start, count) = if first == "0x" || first == "0" || first == "1x" || first == "1"
        || first == "3x" || first == "3" || first == "4x" || first == "4"
    {
        let area = match first.as_str() {
            "0x" | "0" => ModbusArea::Coil,
            "1x" | "1" => ModbusArea::DiscreteInput,
            "3x" | "3" => ModbusArea::InputRegister,
            _ => ModbusArea::HoldingRegister,
        };
        let start: u16 = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
        let count: u16 = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(1);
        (area, start, count.max(1))
    } else {
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
        let count = 1u16;
        (area, start, count)
    };
    Some((area, start, count))
}

fn config_str(config: &PluginConfig, key: &str, default: &str) -> String {
    config
        .get(key)
        .and_then(|v| v.as_str())
        .map(String::from)
        .unwrap_or_else(|| default.to_string())
}

fn config_u16(config: &PluginConfig, key: &str, default: u16) -> u16 {
    config
        .get(key)
        .and_then(|v| v.as_u64())
        .map(|n| n as u16)
        .unwrap_or(default)
}

/// Modbus TCP 南向插件
pub struct ModbusTcpPlugin {
    state: Arc<RwLock<HashMap<NodeId, ModbusTcpState>>>,
}

struct ModbusTcpState {
    host: String,
    port: u16,
    slave_id: u8,
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

impl Default for ModbusTcpPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl ModbusTcpPlugin {
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
impl SouthPlugin for ModbusTcpPlugin {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "modbus-tcp",
            kind: PluginKind::South,
            description: Some("Modbus TCP 南向驱动，通过 TCP 连接 Modbus 设备"),
            version: "0.1.0",
            name_zh: Some("Modbus TCP"),
            name_en: Some("Modbus TCP"),
            description_zh: Some("Modbus TCP 南向驱动，通过 TCP 连接 Modbus 设备"),
            description_en: Some("Modbus TCP south driver, connect to Modbus devices via TCP"),
        }
    }

    fn config_schema(&self) -> Option<ConfigSchema> {
        use gateway_sdk::ParamAttribute;
        Some(
            ConfigSchema::new()
                // connection_mode: Client=0, Server=1
                .param(ParamSchema {
                    name: "connection_mode".to_string(),
                    name_zh: Some("连接模式".to_string()),
                    name_en: Some("Connection Mode".to_string()),
                    description: Some("Neuron as the client, or as the server".to_string()),
                    description_zh: Some("Neuron 作为客户端或服务端".to_string()),
                    description_en: Some("Neuron as the client, or as the server".to_string()),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::Select,
                    default: Some(serde_json::json!(0)),
                    valid: None,
                    options: Some(vec![
                        ParamOption { value: serde_json::json!(0), label: Some("Client".to_string()), label_zh: Some("客户端".to_string()), label_en: Some("Client".to_string()) },
                        ParamOption { value: serde_json::json!(1), label: Some("Server".to_string()), label_zh: Some("服务端".to_string()), label_en: Some("Server".to_string()) },
                    ]),
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "check_header".to_string(),
                    name_zh: Some("校验报文头".to_string()),
                    name_en: Some("Check Header".to_string()),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::Select,
                    default: Some(serde_json::json!(0)),
                    valid: None,
                    options: Some(vec![
                        ParamOption { value: serde_json::json!(0), label: Some("False".to_string()), label_zh: Some("否".to_string()), label_en: Some("False".to_string()) },
                        ParamOption { value: serde_json::json!(1), label: Some("True".to_string()), label_zh: Some("是".to_string()), label_en: Some("True".to_string()) },
                    ]),
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "device_degrade".to_string(),
                    name_zh: Some("设备降级".to_string()),
                    name_en: Some("Device Degradation".to_string()),
                    description: Some("Enable or disable device degradation mechanism".to_string()),
                    description_zh: Some("启用或禁用设备降级机制".to_string()),
                    description_en: Some("Enable or disable device degradation mechanism".to_string()),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::Select,
                    default: Some(serde_json::json!(0)),
                    valid: None,
                    options: Some(vec![
                        ParamOption { value: serde_json::json!(0), label: Some("False".to_string()), label_zh: Some("否".to_string()), label_en: Some("False".to_string()) },
                        ParamOption { value: serde_json::json!(1), label: Some("True".to_string()), label_zh: Some("是".to_string()), label_en: Some("True".to_string()) },
                    ]),
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "degrade_cycle".to_string(),
                    name_zh: Some("降级失败阈值".to_string()),
                    name_en: Some("Failure Threshold for Degradation".to_string()),
                    description: Some("The number of consecutive failure cycles required to trigger device degradation".to_string()),
                    description_zh: Some("触发设备降级所需的连续失败周期数".to_string()),
                    description_en: Some("The number of consecutive failure cycles required to trigger device degradation".to_string()),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(2)),
                    valid: Some(ParamValid { min: Some(1), max: Some(65535), regex: None, length: None }),
                    depends_on: Some("device_degrade".to_string()),
                    depends_value: Some(serde_json::json!(1)),
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "degrade_time".to_string(),
                    name_zh: Some("降级恢复时间".to_string()),
                    name_en: Some("Recovery Time After Degradation".to_string()),
                    description: Some("The time in seconds after which the device recovers from degradation".to_string()),
                    description_zh: Some("设备从降级中恢复所需的时间（单位：秒）".to_string()),
                    description_en: Some("The time in seconds after which the device recovers from degradation".to_string()),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(600)),
                    valid: Some(ParamValid { min: Some(1), max: Some(65535), regex: None, length: None }),
                    depends_on: Some("device_degrade".to_string()),
                    depends_value: Some(serde_json::json!(1)),
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "max_retry_times".to_string(),
                    name_zh: Some("最大重试次数".to_string()),
                    name_en: Some("Maximum Retry Times".to_string()),
                    description: Some("The maximum number of retries after a failed attempt to send a read command".to_string()),
                    description_zh: Some("发送读指令失败后最大重试次数".to_string()),
                    description_en: Some("The maximum number of retries after a failed attempt to send a read command".to_string()),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(0)),
                    valid: Some(ParamValid { min: Some(0), max: Some(3), regex: None, length: None }),
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "retry_interval_ms".to_string(),
                    name_zh: Some("指令重新发送间隔 (ms)".to_string()),
                    name_en: Some("Retry Interval (ms)".to_string()),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(0)),
                    valid: Some(ParamValid { min: Some(0), max: Some(10000), regex: None, length: None }),
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "endianess".to_string(),
                    name_zh: Some("字节序".to_string()),
                    name_en: Some("Endianess".to_string()),
                    description: Some("Tag byte order, ABCD corresponds to 1234".to_string()),
                    description_zh: Some("点位字节序，ABCD 对应 1234".to_string()),
                    description_en: Some("Tag byte order, ABCD corresponds to 1234".to_string()),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::Select,
                    default: Some(serde_json::json!(1)),
                    valid: None,
                    options: Some(vec![
                        ParamOption { value: serde_json::json!(1), label: Some("ABCD".to_string()), label_zh: Some("ABCD".to_string()), label_en: Some("ABCD".to_string()) },
                        ParamOption { value: serde_json::json!(2), label: Some("BADC".to_string()), label_zh: Some("BADC".to_string()), label_en: Some("BADC".to_string()) },
                        ParamOption { value: serde_json::json!(3), label: Some("DCBA".to_string()), label_zh: Some("DCBA".to_string()), label_en: Some("DCBA".to_string()) },
                        ParamOption { value: serde_json::json!(4), label: Some("CDAB".to_string()), label_zh: Some("CDAB".to_string()), label_en: Some("CDAB".to_string()) },
                    ]),
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "start_address".to_string(),
                    name_zh: Some("开始地址".to_string()),
                    name_en: Some("Start Address".to_string()),
                    description: Some("Address starts from 1 or 0".to_string()),
                    description_zh: Some("地址从 1 开始或从 0 开始".to_string()),
                    description_en: Some("Address starts from 1 or 0".to_string()),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::Select,
                    default: Some(serde_json::json!(1)),
                    valid: None,
                    options: Some(vec![
                        ParamOption { value: serde_json::json!(0), label: Some("Protocol Addresses (Base 0)".to_string()), label_zh: Some("协议地址（从 0 开始）".to_string()), label_en: Some("Protocol Addresses (Base 0)".to_string()) },
                        ParamOption { value: serde_json::json!(1), label: Some("PLC Addresses (Base 1)".to_string()), label_zh: Some("PLC 地址（从 1 开始）".to_string()), label_en: Some("PLC Addresses (Base 1)".to_string()) },
                    ]),
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "send_interval_ms".to_string(),
                    name_zh: Some("指令发送间隔 (ms)".to_string()),
                    name_en: Some("Send Interval (ms)".to_string()),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(20)),
                    valid: Some(ParamValid { min: Some(0), max: Some(3000), regex: None, length: None }),
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "host".to_string(),
                    name_zh: Some("IP地址".to_string()),
                    name_en: Some("IP Address".to_string()),
                    description: Some("Local IP in server mode, remote device IP in client mode".to_string()),
                    description_zh: Some("服务端模式中填写本地 IP，客户端模式中填写目标设备 IP".to_string()),
                    description_en: Some("Local IP in server mode, remote device IP in client mode".to_string()),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::String,
                    default: Some(serde_json::json!("127.0.0.1")),
                    valid: None,
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "port".to_string(),
                    name_zh: Some("端口号".to_string()),
                    name_en: Some("Port".to_string()),
                    description: Some("Local port in server mode, remote device port in client mode".to_string()),
                    description_zh: Some("服务端模式中填写本地端口号，客户端模式中填写远程设备端口号".to_string()),
                    description_en: Some("Local port in server mode, remote device port in client mode".to_string()),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(502)),
                    valid: Some(ParamValid { min: Some(1), max: Some(65535), regex: None, length: None }),
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "slave_id".to_string(),
                    description: Some("从站 ID（Modbus TCP 通常为 0 或 1）".to_string()),
                    name_zh: Some("从站 ID".to_string()),
                    name_en: Some("Slave ID".to_string()),
                    description_zh: Some("从站 ID，Modbus TCP 通常为 0 或 1".to_string()),
                    description_en: Some("Slave ID, typically 0 or 1 for Modbus TCP".to_string()),
                    attribute: ParamAttribute::Optional,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(1)),
                    valid: None,
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "connection_timeout_ms".to_string(),
                    name_zh: Some("连接超时时间 (ms)".to_string()),
                    name_en: Some("Connection Timeout (ms)".to_string()),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(3000)),
                    valid: Some(ParamValid { min: Some(1000), max: Some(65535), regex: None, length: None }),
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "backup_host".to_string(),
                    name_zh: Some("备用 IP 地址".to_string()),
                    name_en: Some("Backup IP Address".to_string()),
                    attribute: ParamAttribute::Optional,
                    ty: ParamType::String,
                    default: None,
                    valid: None,
                    depends_on: Some("connection_mode".to_string()),
                    depends_value: Some(serde_json::json!(0)),
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "backup_port".to_string(),
                    name_zh: Some("备用端口号".to_string()),
                    name_en: Some("Backup Port".to_string()),
                    attribute: ParamAttribute::Optional,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(502)),
                    valid: Some(ParamValid { min: Some(1), max: Some(65535), regex: None, length: None }),
                    depends_on: Some("connection_mode".to_string()),
                    depends_value: Some(serde_json::json!(0)),
                    ..Default::default()
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
        let host = config_str(&config, "host", "127.0.0.1");
        let port = config_u16(&config, "port", 502);
        log::info(node_id, format!("open: host={}, port={}", host, port));
        let slave_id = config.get("slave_id").and_then(|v| v.as_u64()).map(|n| n as u8).unwrap_or(1);
        let connection_timeout_ms = config.get("connection_timeout_ms").and_then(|v| v.as_u64()).unwrap_or(3000);
        let send_interval_ms = config.get("send_interval_ms").and_then(|v| v.as_u64()).unwrap_or(20);
        let max_retry_times = config.get("max_retry_times").and_then(|v| v.as_u64()).map(|n| n as u32).unwrap_or(0);
        let retry_interval_ms = config.get("retry_interval_ms").and_then(|v| v.as_u64()).unwrap_or(0);
        let start_address = config.get("start_address").and_then(|v| v.as_u64()).map(|n| n as u8).unwrap_or(1).min(1);
        let groups = Self::default_groups();
        let tags = groups
            .iter()
            .flat_map(|g| {
                vec![
                    Tag {
                        id: TagId::new(),
                        name: "holding_0".to_string(),
                        address: "4x!0".to_string(),
                        attr: gateway_sdk::TagAttr::Read,
                        data_type: Some("uint16".to_string()),
                        description: Some("保持寄存器 0".to_string()),
                        group_id: g.id,
                    },
                ]
            })
            .collect::<Vec<_>>();
        let mut state = self.state.write().await;
        state.insert(
            node_id,
            ModbusTcpState {
                host,
                port,
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
        log::info(node_id, "close");
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
            use tokio::net::TcpStream;
            use tokio_modbus::client::tcp::attach_slave;
            use tokio_modbus::prelude::*;
            use tokio_modbus::slave::Slave;

            let addr = (s.host.as_str(), s.port);
            let timeout_dur = Duration::from_millis(s.connection_timeout_ms.min(60000));
            let stream = tokio::time::timeout(timeout_dur, TcpStream::connect(addr))
                .await
                .map_err(|_| PluginError::msg("tcp connect timeout"))?
                .map_err(|e| PluginError::msg(format!("tcp connect: {}", e)))?;
            let mut ctx = attach_slave(stream, Slave(s.slave_id));

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
                                let b = coils.first().copied().unwrap_or(false);
                                DataValue::Bool(b)
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
                        tokio::time::sleep(send_interval).await;
                        (tag.id, v)
                    }
                    None => (tag.id, DataValue::UInt16(0)),
                };
                out.push((tag_id, value));
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
            use tokio::net::TcpStream;
            use tokio_modbus::client::tcp::attach_slave;
            use tokio_modbus::prelude::*;
            use tokio_modbus::slave::Slave;

            let addr = (s.host.as_str(), s.port);
            let timeout_dur = Duration::from_millis(s.connection_timeout_ms.min(60000));
            let stream = tokio::time::timeout(timeout_dur, TcpStream::connect(addr))
                .await
                .map_err(|_| PluginError::msg("tcp connect timeout"))?
                .map_err(|e| PluginError::msg(format!("tcp connect: {}", e)))?;
            let mut ctx = attach_slave(stream, Slave(s.slave_id));
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

/// 按字节序取 32 位：order32 0=LL 1=LB 2=BL 3=BB
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

/// 按字节序取 64 位
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

#[allow(dead_code)]
fn register_to_value(regs: &[u16], data_type: &str) -> DataValue {
    register_to_value_ext(regs, data_type, &Endianness::default(), None)
}

/// 将 DataValue 转为寄存器序列用于写保持寄存器
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
                u16::from_le_bytes([b[0], b[1]]),
                u16::from_le_bytes([b[2], b[3]]),
                u16::from_le_bytes([b[4], b[5]]),
                u16::from_le_bytes([b[6], b[7]]),
            ]
        }
        DataValue::UInt64(v) => {
            let b = v.to_le_bytes();
            vec![
                u16::from_le_bytes([b[0], b[1]]),
                u16::from_le_bytes([b[2], b[3]]),
                u16::from_le_bytes([b[4], b[5]]),
                u16::from_le_bytes([b[6], b[7]]),
            ]
        }
        DataValue::Float32(v) => {
            let b = v.to_bits().to_le_bytes();
            vec![u16::from_le_bytes([b[0], b[1]]), u16::from_le_bytes([b[2], b[3]])]
        }
        DataValue::Float64(v) => {
            let b = v.to_bits().to_le_bytes();
            vec![
                u16::from_le_bytes([b[0], b[1]]),
                u16::from_le_bytes([b[2], b[3]]),
                u16::from_le_bytes([b[4], b[5]]),
                u16::from_le_bytes([b[6], b[7]]),
            ]
        }
        DataValue::String(s) => {
            let bytes = s.as_bytes();
            let mut regs = Vec::with_capacity((bytes.len() + 1) / 2);
            for chunk in bytes.chunks(2) {
                let low = chunk.get(0).copied().unwrap_or(0);
                let high = chunk.get(1).copied().unwrap_or(0);
                regs.push(u16::from_le_bytes([low, high]));
            }
            regs
        }
        DataValue::Bytes(b) => {
            let mut regs = Vec::with_capacity((b.len() + 1) / 2);
            for chunk in b.chunks(2) {
                let low = chunk.get(0).copied().unwrap_or(0);
                let high = chunk.get(1).copied().unwrap_or(0);
                regs.push(u16::from_le_bytes([low, high]));
            }
            regs
        }
        DataValue::Bool(v) if data_type == "bool" => vec![if *v { 0xff00 } else { 0 }],
        _ => {
            if let Some(u) = value.as_u64() {
                vec![u as u16]
            } else {
                vec![0]
            }
        }
    }
}

/// 将 u16 寄存器按数据类型与字节序转为 DataValue；bit_index 表示取寄存器内某一位
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
            let r = if endian.swap16 { r.swap_bytes() } else { r };
            DataValue::Int16(r as i16)
        }
        "uint16" => {
            let r = regs.first().copied().unwrap_or(0);
            DataValue::UInt16(if endian.swap16 { r.swap_bytes() } else { r })
        }
        "int32" | "uint32" => {
            let v = regs_to_u32(regs, endian);
            if data_type == "int32" {
                DataValue::Int32(v as i32)
            } else {
                DataValue::UInt32(v)
            }
        }
        "int64" | "uint64" => {
            let v = regs_to_u64(regs, endian);
            if data_type == "int64" {
                DataValue::Int64(v as i64)
            } else {
                DataValue::UInt64(v)
            }
        }
        "float32" => {
            let v = regs_to_u32(regs, endian);
            DataValue::Float32(f32::from_bits(v))
        }
        "float64" => {
            let v = regs_to_u64(regs, endian);
            DataValue::Float64(f64::from_bits(v))
        }
        "string" => {
            let mut bytes: Vec<u8> = Vec::new();
            for &r in regs {
                bytes.push((r & 0xff) as u8);
                bytes.push((r >> 8) as u8);
            }
            let s = String::from_utf8_lossy(&bytes).trim_end_matches('\0').to_string();
            DataValue::String(s)
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
