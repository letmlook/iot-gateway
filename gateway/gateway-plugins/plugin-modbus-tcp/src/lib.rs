//! 南向 Modbus TCP 插件：支持 TCP 客户端、线圈/离散/输入/保持寄存器、
//! 地址格式（SLAVE!ADDRESS[.BIT][#ENDIAN]）、读写、超时重试、字节序、STRING/BYTES 等。

#[cfg(feature = "ffi")]
mod ffi;

mod address;
mod config;
mod merge;
mod state;
mod value;

use gateway_sdk::log;
use gateway_sdk::types::{DataValue, PluginKind};
use gateway_sdk::{
    ConfigSchema, Group, GroupId, NodeId, ParamOption, ParamSchema, ParamType, ParamValid,
    PluginMeta, SouthPlugin, Tag, TagId, TagRegexEntry, TagSchema,
};
use gateway_sdk::{PluginConfig, PluginError, PluginResult};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use address::{parse_address, parse_address_full, ModbusArea, ParsedAddress};
use config::{config_str, config_u16};
use state::ModbusTcpState;
use value::{register_to_value_ext, value_to_registers};

#[cfg(feature = "modbus-client")]
use std::time::Duration;

/// 合并规划诊断（不建立连接，纯计算）：返回 `(批量读请求数, 逐点读请求数)`。
///
/// 现场可用它评估一批点位地址的合并收益；性能基线压测也用它作为对照。
pub fn merge_plan_stats(addresses: &[String], start_address: u8) -> (usize, usize) {
    let reads: Vec<merge::TagRead> = addresses
        .iter()
        .enumerate()
        .filter_map(|(i, a)| {
            address::parse_address_full(a, start_address).map(|p| merge::TagRead::new(i, p))
        })
        .collect();
    let (plans, singles) = merge::plan_merges(
        &reads,
        merge::DEFAULT_MERGE_GAP,
        merge::DEFAULT_MAX_READ_REGS,
    );
    (plans.len(), singles.len())
}

/// 建立一条新的 Modbus TCP 连接（带连接超时）
#[cfg(feature = "modbus-client")]
async fn connect_ctx(s: &ModbusTcpState) -> PluginResult<tokio_modbus::client::Context> {
    use tokio::net::TcpStream;
    use tokio_modbus::client::tcp::attach_slave;
    use tokio_modbus::slave::Slave;

    let addr = (s.host.as_str(), s.port);
    let timeout_dur = Duration::from_millis(s.connection_timeout_ms.min(60000));
    let stream = tokio::time::timeout(timeout_dur, TcpStream::connect(addr))
        .await
        .map_err(|_| PluginError::msg("tcp connect timeout"))?
        .map_err(|e| PluginError::msg(format!("tcp connect: {}", e)))?;
    Ok(attach_slave(stream, Slave(s.slave_id)))
}

/// 取出（必要时建立）长连接。处于退避窗口内时直接报错，避免对不可达设备反复建连。
#[cfg(feature = "modbus-client")]
async fn ensure_connection(
    s: &ModbusTcpState,
) -> PluginResult<tokio::sync::MutexGuard<'_, state::ConnectionState>> {
    let mut cst = s.conn.lock().await;
    if cst.conn.is_none() {
        let wait = cst.backoff_remaining_ms();
        if wait > 0 {
            return Err(PluginError::msg(format!(
                "connection in backoff, retry in {} ms",
                wait
            )));
        }
        match connect_ctx(s).await {
            Ok(ctx) => {
                cst.conn = Some(ctx);
                cst.on_success();
            }
            Err(e) => {
                cst.on_failure();
                return Err(e);
            }
        }
    }
    Ok(cst)
}

/// Modbus TCP 南向插件
pub struct ModbusTcpPlugin {
    state: Arc<RwLock<HashMap<NodeId, ModbusTcpState>>>,
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
                    description: Some("Gateway acts as client (connect to device) or server (listen for device)".to_string()),
                    description_zh: Some("网关作为客户端（主动连接设备）或服务端（等待设备连接）".to_string()),
                    description_en: Some("Gateway acts as client (connect to device) or server (listen for device)".to_string()),
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
                "0x!addr/1x!addr/3x!addr/4x!addr 或 1!400001[.BIT][#ENDIAN]，.LEN 用于 STRING"
                    .to_string(),
            ),
            address_format_zh: Some(
                "0x!addr/1x!addr/3x!addr/4x!addr 或 1!400001[.BIT][#ENDIAN]，.LEN 用于 STRING"
                    .to_string(),
            ),
            address_format_en: Some(
                "0x!addr/1x!addr/3x!addr/4x!addr or 1!400001[.BIT][#ENDIAN], .LEN for STRING"
                    .to_string(),
            ),
        })
    }

    async fn validate_tag(&self, _node_id: NodeId, tag: &Tag) -> PluginResult<()> {
        if tag.name.is_empty() {
            return Err(PluginError::tag_invalid("tag name required"));
        }
        parse_address(&tag.address).ok_or_else(|| {
            PluginError::tag_invalid("address format: 0x!addr / 1x!addr / 3x!addr / 4x!addr")
        })?;
        Ok(())
    }

    async fn open(&self, node_id: NodeId, config: PluginConfig) -> PluginResult<()> {
        let host = config_str(&config, "host", "127.0.0.1");
        let port = config_u16(&config, "port", 502);
        log::info(
            node_id,
            format!("open modbus-tcp: host={}, port={}", host, port),
        );
        let slave_id = config
            .get("slave_id")
            .and_then(|v| v.as_u64())
            .map(|n| n as u8)
            .unwrap_or(1);
        let connection_timeout_ms = config
            .get("connection_timeout_ms")
            .and_then(|v| v.as_u64())
            .unwrap_or(3000);
        let send_interval_ms = config
            .get("send_interval_ms")
            .and_then(|v| v.as_u64())
            .unwrap_or(20);
        let max_retry_times = config
            .get("max_retry_times")
            .and_then(|v| v.as_u64())
            .map(|n| n as u32)
            .unwrap_or(0);
        let retry_interval_ms = config
            .get("retry_interval_ms")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let start_address = config
            .get("start_address")
            .and_then(|v| v.as_u64())
            .map(|n| n as u8)
            .unwrap_or(1)
            .min(1);
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
                #[cfg(feature = "modbus-client")]
                conn: tokio::sync::Mutex::new(state::ConnectionState::new()),
            },
        );
        Ok(())
    }

    async fn close(&self, node_id: NodeId) -> PluginResult<()> {
        log::info(node_id, "close modbus-tcp");
        let mut state = self.state.write().await;
        state.remove(&node_id);
        Ok(())
    }

    async fn start(&self, node_id: NodeId) -> PluginResult<()> {
        log::info(node_id, "start modbus-tcp");
        Ok(())
    }

    async fn stop(&self, node_id: NodeId) -> PluginResult<()> {
        log::info(node_id, "stop modbus-tcp");
        Ok(())
    }

    async fn setting(&self, node_id: NodeId, config: PluginConfig) -> PluginResult<()> {
        log::info(node_id, "setting modbus-tcp (config updated)");
        let host = config_str(&config, "host", "127.0.0.1");
        let port = config_u16(&config, "port", 502);
        let mut state = self.state.write().await;
        if let Some(s) = state.get_mut(&node_id) {
            s.host = host;
            s.port = port;
            s.slave_id = config
                .get("slave_id")
                .and_then(|v| v.as_u64())
                .map(|n| n as u8)
                .unwrap_or(s.slave_id);
            s.connection_timeout_ms = config
                .get("connection_timeout_ms")
                .and_then(|v| v.as_u64())
                .unwrap_or(s.connection_timeout_ms);
            s.send_interval_ms = config
                .get("send_interval_ms")
                .and_then(|v| v.as_u64())
                .unwrap_or(s.send_interval_ms);
            s.max_retry_times = config
                .get("max_retry_times")
                .and_then(|v| v.as_u64())
                .map(|n| n as u32)
                .unwrap_or(s.max_retry_times);
            s.retry_interval_ms = config
                .get("retry_interval_ms")
                .and_then(|v| v.as_u64())
                .unwrap_or(s.retry_interval_ms);
            s.start_address = config
                .get("start_address")
                .and_then(|v| v.as_u64())
                .map(|n| n as u8)
                .unwrap_or(s.start_address)
                .min(1);
        }
        Ok(())
    }

    async fn poll_group(
        &self,
        node_id: NodeId,
        _group_id: GroupId,
        tags: &[Tag],
    ) -> PluginResult<Vec<(TagId, DataValue)>> {
        let state = self.state.read().await;
        let s = state
            .get(&node_id)
            .ok_or_else(|| PluginError::msg("node not open"))?;

        #[cfg(feature = "modbus-client")]
        {
            use tokio_modbus::prelude::*;

            // 复用长连接：仅首次或上次出错后重建（退避见 state::ConnectionState）
            let mut cst = ensure_connection(s).await?;
            let ctx = cst
                .conn
                .as_mut()
                .ok_or_else(|| PluginError::msg("modbus connection unavailable"))?;

            let send_interval = Duration::from_millis(s.send_interval_ms.min(5000));

            // 先规划：地址相邻、形状相同的寄存器点位合并为一次批量读
            let parsed: Vec<Option<ParsedAddress>> = tags
                .iter()
                .map(|t| parse_address_full(&t.address, s.start_address))
                .collect();
            let reads: Vec<merge::TagRead> = parsed
                .iter()
                .enumerate()
                .filter_map(|(i, p)| p.clone().map(|p| merge::TagRead::new(i, p)))
                .collect();
            let (plans, singles) = merge::plan_merges(
                &reads,
                merge::DEFAULT_MERGE_GAP,
                merge::DEFAULT_MAX_READ_REGS,
            );

            // 现场可直接在 debug 日志里看到合并效果（请求数从 tags 个降到个位数）
            tracing::debug!(
                node_id = ?node_id,
                tags = tags.len(),
                merged_reads = plans.len(),
                single_reads = singles.len(),
                "modbus poll plan"
            );

            let mut slots: Vec<Option<(TagId, DataValue)>> = vec![None; tags.len()];
            let mut failure: Option<PluginError> = None;

            // 1) 批量读：一次请求覆盖一个地址区间
            'plan: for plan in &plans {
                let regs = match plan.area {
                    ModbusArea::HoldingRegister => {
                        match ctx.read_holding_registers(plan.start, plan.count).await {
                            Ok(Ok(v)) => v,
                            Ok(Err(e)) => {
                                failure = Some(PluginError::msg(format!(
                                    "read_holding_registers exception: {}",
                                    e
                                )));
                                break 'plan;
                            }
                            Err(e) => {
                                failure = Some(PluginError::msg(format!(
                                    "read_holding_registers: {}",
                                    e
                                )));
                                break 'plan;
                            }
                        }
                    }
                    ModbusArea::InputRegister => {
                        match ctx.read_input_registers(plan.start, plan.count).await {
                            Ok(Ok(v)) => v,
                            Ok(Err(e)) => {
                                failure = Some(PluginError::msg(format!(
                                    "read_input_registers exception: {}",
                                    e
                                )));
                                break 'plan;
                            }
                            Err(e) => {
                                failure =
                                    Some(PluginError::msg(format!("read_input_registers: {}", e)));
                                break 'plan;
                            }
                        }
                    }
                    // 规划阶段只会产出寄存器类计划
                    _ => continue 'plan,
                };

                for (idx, offset) in &plan.members {
                    let Some(p) = parsed[*idx].as_ref() else {
                        continue;
                    };
                    let begin = *offset as usize;
                    let end = begin + p.count as usize;
                    let window = regs.get(begin..end).unwrap_or(&[]);
                    let dt = tags[*idx].data_type.as_deref().unwrap_or("uint16");
                    slots[*idx] = Some((
                        tags[*idx].id,
                        register_to_value_ext(window, dt, &p.endian, p.bit_index),
                    ));
                }
                tokio::time::sleep(send_interval).await;
            }

            // 2) 逐点读：线圈 / 离散输入 / 带 .BIT 的寄存器（不参与合并）
            if failure.is_none() {
                'single: for idx in &singles {
                    let tag = &tags[*idx];
                    let value = match parsed[*idx].as_ref() {
                        Some(p) => {
                            let dt = tag.data_type.as_deref().unwrap_or("uint16");
                            match p.area {
                                ModbusArea::Coil => match ctx.read_coils(p.start, p.count).await {
                                    Ok(Ok(v)) => {
                                        DataValue::Bool(v.first().copied().unwrap_or(false))
                                    }
                                    Ok(Err(e)) => {
                                        failure = Some(PluginError::msg(format!(
                                            "read_coils exception: {}",
                                            e
                                        )));
                                        break 'single;
                                    }
                                    Err(e) => {
                                        failure =
                                            Some(PluginError::msg(format!("read_coils: {}", e)));
                                        break 'single;
                                    }
                                },
                                ModbusArea::DiscreteInput => {
                                    match ctx.read_discrete_inputs(p.start, p.count).await {
                                        Ok(Ok(v)) => {
                                            DataValue::Bool(v.first().copied().unwrap_or(false))
                                        }
                                        Ok(Err(e)) => {
                                            failure = Some(PluginError::msg(format!(
                                                "read_discrete_inputs exception: {}",
                                                e
                                            )));
                                            break 'single;
                                        }
                                        Err(e) => {
                                            failure = Some(PluginError::msg(format!(
                                                "read_discrete_inputs: {}",
                                                e
                                            )));
                                            break 'single;
                                        }
                                    }
                                }
                                ModbusArea::InputRegister => {
                                    match ctx.read_input_registers(p.start, p.count).await {
                                        Ok(Ok(regs)) => {
                                            register_to_value_ext(&regs, dt, &p.endian, p.bit_index)
                                        }
                                        Ok(Err(e)) => {
                                            failure = Some(PluginError::msg(format!(
                                                "read_input_registers exception: {}",
                                                e
                                            )));
                                            break 'single;
                                        }
                                        Err(e) => {
                                            failure = Some(PluginError::msg(format!(
                                                "read_input_registers: {}",
                                                e
                                            )));
                                            break 'single;
                                        }
                                    }
                                }
                                ModbusArea::HoldingRegister => {
                                    match ctx.read_holding_registers(p.start, p.count).await {
                                        Ok(Ok(regs)) => {
                                            register_to_value_ext(&regs, dt, &p.endian, p.bit_index)
                                        }
                                        Ok(Err(e)) => {
                                            failure = Some(PluginError::msg(format!(
                                                "read_holding_registers exception: {}",
                                                e
                                            )));
                                            break 'single;
                                        }
                                        Err(e) => {
                                            failure = Some(PluginError::msg(format!(
                                                "read_holding_registers: {}",
                                                e
                                            )));
                                            break 'single;
                                        }
                                    }
                                }
                            }
                        }
                        None => DataValue::UInt16(0),
                    };
                    slots[*idx] = Some((tag.id, value));
                    tokio::time::sleep(send_interval).await;
                }
            }

            match failure {
                // 读取失败通常意味着连接已不可用：丢弃并退避，下次采集重建
                Some(e) => {
                    cst.on_failure();
                    Err(e)
                }
                None => {
                    cst.on_success();
                    Ok(slots
                        .into_iter()
                        .enumerate()
                        .map(|(i, v)| v.unwrap_or((tags[i].id, DataValue::UInt16(0))))
                        .collect())
                }
            }
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
        let s = state
            .get(&node_id)
            .ok_or_else(|| PluginError::msg("node not open"))?;
        Ok(s.groups.clone())
    }

    async fn list_tags(&self, node_id: NodeId, group_id: GroupId) -> PluginResult<Vec<Tag>> {
        let state = self.state.read().await;
        let s = state
            .get(&node_id)
            .ok_or_else(|| PluginError::msg("node not open"))?;
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
        let s = state
            .get(&node_id)
            .ok_or_else(|| PluginError::msg("node not open"))?;

        #[cfg(feature = "modbus-client")]
        {
            use tokio_modbus::prelude::*;

            // 写值与采集共用同一条长连接，避免每次写都重新建连
            let mut cst = ensure_connection(s).await?;
            let ctx = cst
                .conn
                .as_mut()
                .ok_or_else(|| PluginError::msg("modbus connection unavailable"))?;
            let interval = Duration::from_millis(s.send_interval_ms.min(5000));
            let mut failure: Option<PluginError> = None;

            'write: for (tag, value) in values {
                let Some(parsed) = parse_address_full(&tag.address, s.start_address) else {
                    continue;
                };
                match parsed.area {
                    ModbusArea::Coil => {
                        let b = value.as_bool().unwrap_or(false);
                        if parsed.count == 1 {
                            if let Err(e) = ctx.write_single_coil(parsed.start, b).await {
                                failure =
                                    Some(PluginError::msg(format!("write_single_coil: {}", e)));
                                break 'write;
                            }
                        } else {
                            let coils: Vec<bool> = (0..parsed.count).map(|_| b).collect();
                            if let Err(e) = ctx.write_multiple_coils(parsed.start, &coils).await {
                                failure =
                                    Some(PluginError::msg(format!("write_multiple_coils: {}", e)));
                                break 'write;
                            }
                        }
                    }
                    ModbusArea::HoldingRegister => {
                        let regs =
                            value_to_registers(value, tag.data_type.as_deref().unwrap_or("uint16"));
                        if regs.is_empty() {
                            continue;
                        }
                        if regs.len() == 1 {
                            if let Err(e) = ctx.write_single_register(parsed.start, regs[0]).await {
                                failure =
                                    Some(PluginError::msg(format!("write_single_register: {}", e)));
                                break 'write;
                            }
                        } else if let Err(e) =
                            ctx.write_multiple_registers(parsed.start, &regs).await
                        {
                            failure =
                                Some(PluginError::msg(format!("write_multiple_registers: {}", e)));
                            break 'write;
                        }
                    }
                    _ => {
                        failure = Some(PluginError::tag_invalid(
                            "only coil and holding register support write",
                        ));
                        break 'write;
                    }
                }
                tokio::time::sleep(interval).await;
            }

            match failure {
                Some(e) => {
                    cst.on_failure();
                    Err(e)
                }
                None => {
                    cst.on_success();
                    Ok(())
                }
            }
        }

        #[cfg(not(feature = "modbus-client"))]
        {
            let _ = (node_id, s, values);
            Err(PluginError::not_supported(
                "write requires modbus-client feature",
            ))
        }
    }
}
