//! Siemens S7 南向插件：支持 S7-300/400/1200/1500 系列 PLC，通过 Snap7 协议通信。
//!
//! 地址格式：
//!   DB{n}.DBD{o} - 数据块双字（浮点数）
//!   DB{n}.DBW{o} - 数据块字（16位整数）
//!   DB{n}.DBB{o} - 数据块字节
//!   DB{n}.DBX{o}.{b} - 数据块某位
//!   I{o} / IB{o} / IW{o} / ID{o} - 输入区
//!   Q{o} / QB{o} / QW{o} / QD{o} - 输出区
//!   M{o} / MB{o} / MW{o} / MD{o} - 标志位（Marker）
//!
//! 配置：
//!   { "host": "192.168.1.10", "rack": 0, "slot": 1 }

#[cfg(feature = "ffi")]
mod ffi;

mod state;

use gateway_sdk::{
    ConfigSchema, Group, GroupId, NodeId, ParamAttribute, ParamSchema, ParamType, ParamValid,
    PluginMeta, SouthPlugin, Tag, TagId, TagSchema,
};
use gateway_sdk::log;
use gateway_sdk::types::{DataValue, PluginKind};
use gateway_sdk::{PluginConfig, PluginError, PluginResult};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// S7 地址类型
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum S7Area {
    Db,
    Input,
    Output,
    Marker,
    Counter,
    Timer,
    Unknown,
}

/// S7 点位解析结果
#[derive(Debug, Clone)]
struct S7TagAddr {
    area: S7Area,
    db_number: u16,
    byte_offset: u16,
    bit_offset: Option<u8>,
    data_size: u8,
}

/// 解析 S7 地址字符串
fn parse_s7_address(addr: &str) -> Option<S7TagAddr> {
    let addr = addr.trim().to_uppercase();

    // DB address: DB{n}.DBX{o}.{b} or DB{n}.DBD{o} or DB{n}.DBW{o} or DB{n}.DBB{o}
    if addr.starts_with("DB") {
        let rest = &addr[2..];
        let parts: Vec<&str> = rest.split('.').collect();
        if parts.is_empty() {
            return None;
        }
        let db_num: u16 = parts[0].parse().ok()?;

        if parts.len() == 1 {
            return Some(S7TagAddr {
                area: S7Area::Db,
                db_number: db_num,
                byte_offset: 0,
                bit_offset: None,
                data_size: 1,
            });
        }

        let byte_str = parts[1];
        if byte_str.starts_with("DBX") {
            let byte_off: u16 = byte_str[2..].parse().ok()?;
            if parts.len() == 3 {
                let bit_off: u8 = parts[2].parse().ok()?;
                return Some(S7TagAddr {
                    area: S7Area::Db,
                    db_number: db_num,
                    byte_offset: byte_off,
                    bit_offset: Some(bit_off),
                    data_size: 1,
                });
            } else {
                return Some(S7TagAddr {
                    area: S7Area::Db,
                    db_number: db_num,
                    byte_offset: byte_off,
                    bit_offset: None,
                    data_size: 4,
                });
            }
        } else if byte_str.starts_with("DBD") {
            let byte_off: u16 = byte_str[2..].parse().ok()?;
            return Some(S7TagAddr {
                area: S7Area::Db,
                db_number: db_num,
                byte_offset: byte_off,
                bit_offset: None,
                data_size: 4,
            });
        } else if byte_str.starts_with("DBW") {
            let byte_off: u16 = byte_str[2..].parse().ok()?;
            return Some(S7TagAddr {
                area: S7Area::Db,
                db_number: db_num,
                byte_offset: byte_off,
                bit_offset: None,
                data_size: 2,
            });
        } else if byte_str.starts_with("DBB") {
            let byte_off: u16 = byte_str[2..].parse().ok()?;
            return Some(S7TagAddr {
                area: S7Area::Db,
                db_number: db_num,
                byte_offset: byte_off,
                bit_offset: None,
                data_size: 1,
            });
        } else {
            let byte_off: u16 = byte_str.parse().ok()?;
            return Some(S7TagAddr {
                area: S7Area::Db,
                db_number: db_num,
                byte_offset: byte_off,
                bit_offset: None,
                data_size: 1,
            });
        }
    }

    // I, IB, IW, ID addresses (inputs)
    if addr.starts_with("I") {
        let rest = &addr[1..];
        if rest.starts_with("B") {
            let off: u16 = rest[1..].parse().ok()?;
            return Some(S7TagAddr { area: S7Area::Input, db_number: 0, byte_offset: off, bit_offset: None, data_size: 1 });
        } else if rest.starts_with("W") {
            let off: u16 = rest[1..].parse().ok()?;
            return Some(S7TagAddr { area: S7Area::Input, db_number: 0, byte_offset: off, bit_offset: None, data_size: 2 });
        } else if rest.starts_with("D") {
            let off: u16 = rest[1..].parse().ok()?;
            return Some(S7TagAddr { area: S7Area::Input, db_number: 0, byte_offset: off, bit_offset: None, data_size: 4 });
        } else {
            let off: u16 = rest.parse().ok()?;
            return Some(S7TagAddr { area: S7Area::Input, db_number: 0, byte_offset: off, bit_offset: None, data_size: 1 });
        }
    }

    // Q, QB, QW, QD addresses (outputs)
    if addr.starts_with("Q") {
        let rest = &addr[1..];
        if rest.starts_with("B") {
            let off: u16 = rest[1..].parse().ok()?;
            return Some(S7TagAddr { area: S7Area::Output, db_number: 0, byte_offset: off, bit_offset: None, data_size: 1 });
        } else if rest.starts_with("W") {
            let off: u16 = rest[1..].parse().ok()?;
            return Some(S7TagAddr { area: S7Area::Output, db_number: 0, byte_offset: off, bit_offset: None, data_size: 2 });
        } else if rest.starts_with("D") {
            let off: u16 = rest[1..].parse().ok()?;
            return Some(S7TagAddr { area: S7Area::Output, db_number: 0, byte_offset: off, bit_offset: None, data_size: 4 });
        } else {
            let off: u16 = rest.parse().ok()?;
            return Some(S7TagAddr { area: S7Area::Output, db_number: 0, byte_offset: off, bit_offset: None, data_size: 1 });
        }
    }

    // M, MB, MW, MD addresses (markers)
    if addr.starts_with("M") {
        let rest = &addr[1..];
        if rest.starts_with("B") {
            let off: u16 = rest[1..].parse().ok()?;
            return Some(S7TagAddr { area: S7Area::Marker, db_number: 0, byte_offset: off, bit_offset: None, data_size: 1 });
        } else if rest.starts_with("W") {
            let off: u16 = rest[1..].parse().ok()?;
            return Some(S7TagAddr { area: S7Area::Marker, db_number: 0, byte_offset: off, bit_offset: None, data_size: 2 });
        } else if rest.starts_with("D") {
            let off: u16 = rest[1..].parse().ok()?;
            return Some(S7TagAddr { area: S7Area::Marker, db_number: 0, byte_offset: off, bit_offset: None, data_size: 4 });
        } else {
            let off: u16 = rest.parse().ok()?;
            return Some(S7TagAddr { area: S7Area::Marker, db_number: 0, byte_offset: off, bit_offset: None, data_size: 1 });
        }
    }

    None
}

/// S7 节点状态
struct S7NodeState {
    host: String,
    rack: u16,
    slot: u16,
    connected: bool,
    groups: Vec<Group>,
}

impl S7NodeState {
    fn new(host: String, rack: u16, slot: u16) -> Self {
        Self {
            host,
            rack,
            slot,
            connected: false,
            groups: vec![Group {
                id: GroupId::new(),
                name: "default".to_string(),
                interval_ms: 1000,
                description: Some("默认采集组".to_string()),
            }],
        }
    }
}

/// Siemens S7 南向插件
pub struct S7Plugin {
    state: Arc<RwLock<HashMap<NodeId, S7NodeState>>>,
}

impl Default for S7Plugin {
    fn default() -> Self {
        Self::new()
    }
}

impl S7Plugin {
    pub fn new() -> Self {
        Self {
            state: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

#[async_trait::async_trait]
impl SouthPlugin for S7Plugin {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "s7",
            kind: PluginKind::South,
            description: Some("西门子 S7 系列 PLC 通信协议"),
            version: "0.1.0",
            name_zh: Some("西门子S7"),
            name_en: Some("Siemens S7"),
            description_zh: Some("西门子 S7 系列 PLC 通信协议，支持 S7-300/400/1200/1500"),
            description_en: Some("Siemens S7 PLC protocol — S7-300/400/1200/1500"),
        }
    }

    fn config_schema(&self) -> Option<ConfigSchema> {
        Some(
            ConfigSchema::new()
                .param(ParamSchema {
                    name: "host".to_string(),
                    name_zh: Some("IP地址".to_string()),
                    name_en: Some("IP Address".to_string()),
                    description: Some("PLC IP地址".to_string()),
                    description_zh: Some("PLC IP地址".to_string()),
                    description_en: Some("PLC IP address".to_string()),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::String,
                    default: Some(serde_json::json!("192.168.1.10")),
                    valid: None,
                    options: None,
                    depends_on: None,
                    depends_value: None,
                    depends_values: None,
                })
                .param(ParamSchema {
                    name: "rack".to_string(),
                    name_zh: Some("机架号".to_string()),
                    name_en: Some("Rack".to_string()),
                    description: Some("PLC 机架号，通常为 0".to_string()),
                    description_zh: Some("PLC 机架号，通常为 0".to_string()),
                    description_en: Some("PLC rack number, usually 0".to_string()),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(0)),
                    valid: Some(ParamValid { min: Some(0), max: Some(7), regex: None, length: None }),
                    options: None,
                    depends_on: None,
                    depends_value: None,
                    depends_values: None,
                })
                .param(ParamSchema {
                    name: "slot".to_string(),
                    name_zh: Some("槽号".to_string()),
                    name_en: Some("Slot".to_string()),
                    description: Some("PLC 槽号，S7-300 通常为 1，S7-1500 可能为 1".to_string()),
                    description_zh: Some("PLC 槽号，S7-300 通常为 1，S7-1500 可能为 1".to_string()),
                    description_en: Some("PLC slot number, typically 1 for S7-300, 1 for S7-1500".to_string()),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(1)),
                    valid: Some(ParamValid { min: Some(0), max: Some(31), regex: None, length: None }),
                    options: None,
                    depends_on: None,
                    depends_value: None,
                    depends_values: None,
                }),
        )
    }

    fn tag_schema(&self) -> Option<TagSchema> {
        Some(TagSchema {
            data_types: Some(vec![
                "float".to_string(),
                "int16".to_string(),
                "uint16".to_string(),
                "int32".to_string(),
                "uint32".to_string(),
                "bool".to_string(),
                "byte".to_string(),
            ]),
            address_format: Some(
                "DB{n}.DBD{o} | DB{n}.DBW{o} | DB{n}.DBB{o} | DB{n}.DBX{o}.{b} | I{o} | Q{o} | M{o}".to_string(),
            ),
            address_format_zh: Some(
                "DB{n}.DBD{o}(浮点) | DB{n}.DBW{o}(字) | I{o}(输入) | Q{o}(输出) | M{o}(标志位)".to_string(),
            ),
            address_format_en: Some(
                "DB{n}.DBD{o} (float) | DB{n}.DBW{o} (word) | I{o} (input) | Q{o} (output) | M{o} (marker)".to_string(),
            ),
        })
    }

    async fn validate_tag(&self, _node_id: NodeId, tag: &Tag) -> PluginResult<()> {
        if tag.name.is_empty() {
            return Err(PluginError::tag_invalid("tag name required"));
        }
        if parse_s7_address(&tag.address).is_none() {
            return Err(PluginError::tag_invalid(&format!(
                "invalid S7 address format: {}",
                tag.address
            )));
        }
        Ok(())
    }

    async fn open(&self, node_id: NodeId, config: PluginConfig) -> PluginResult<()> {
        let host = config
            .get("host")
            .and_then(|v| v.as_str())
            .unwrap_or("192.168.1.10")
            .to_string();
        let rack = config
            .get("rack")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u16;
        let slot = config
            .get("slot")
            .and_then(|v| v.as_u64())
            .unwrap_or(1) as u16;

        log::info(node_id, format!("open s7: host={}, rack={}, slot={}", host, rack, slot));

        let state = S7NodeState::new(host, rack, slot);
        let mut states = self.state.write().await;
        states.insert(node_id, state);
        Ok(())
    }

    async fn close(&self, node_id: NodeId) -> PluginResult<()> {
        log::info(node_id, "close s7");
        let mut states = self.state.write().await;
        states.remove(&node_id);
        Ok(())
    }

    async fn start(&self, node_id: NodeId) -> PluginResult<()> {
        log::info(node_id, "start s7");
        let mut states = self.state.write().await;
        if let Some(s) = states.get_mut(&node_id) {
            s.connected = true;
        }
        Ok(())
    }

    async fn stop(&self, node_id: NodeId) -> PluginResult<()> {
        log::info(node_id, "stop s7");
        let mut states = self.state.write().await;
        if let Some(s) = states.get_mut(&node_id) {
            s.connected = false;
        }
        Ok(())
    }

    async fn poll_group(
        &self,
        node_id: NodeId,
        _group_id: GroupId,
        tags: &[Tag],
    ) -> PluginResult<Vec<(TagId, DataValue)>> {
        let states = self.state.read().await;
        let state = states
            .get(&node_id)
            .ok_or_else(|| PluginError::msg("node not open"))?;

        if !state.connected {
            return Err(PluginError::msg("plugin not started"));
        }

        // 简化实现：返回mock数据
        // 实际实现需要使用 Snap7 库连接 PLC 读取
        let mut results = Vec::with_capacity(tags.len());
        for tag in tags {
            if let Some(addr) = parse_s7_address(&tag.address) {
                let value = match addr.data_size {
                    4 => DataValue::Float32(0.0_f32),
                    2 => DataValue::Int16(0_i16),
                    1 if addr.bit_offset.is_some() => DataValue::Bool(false),
                    1 => DataValue::UInt8(0_u8),
                    _ => DataValue::Float32(0.0_f32),
                };
                results.push((tag.id, value));
            }
        }
        Ok(results)
    }

    async fn write_tags(
        &self,
        node_id: NodeId,
        values: &[(Tag, DataValue)],
    ) -> PluginResult<()> {
        log::warn(node_id, format!("write_tags called with {} values (not implemented)", values.len()));
        Err(PluginError::not_supported("S7 write not implemented"))
    }

    async fn list_groups(&self, node_id: NodeId) -> PluginResult<Vec<Group>> {
        let states = self.state.read().await;
        let state = states
            .get(&node_id)
            .ok_or_else(|| PluginError::msg("node not open"))?;
        Ok(state.groups.clone())
    }

    async fn list_tags(&self, node_id: NodeId, _group_id: GroupId) -> PluginResult<Vec<Tag>> {
        let states = self.state.read().await;
        let _state = states
            .get(&node_id)
            .ok_or_else(|| PluginError::msg("node not open"))?;
        Ok(vec![
            Tag {
                id: TagId::new(),
                name: "db1_dbd0".to_string(),
                address: "DB1.DBD0".to_string(),
                attr: gateway_sdk::TagAttr::Read,
                data_type: Some("float".to_string()),
                description: Some("数据块1 双字0 浮点".to_string()),
                group_id: GroupId::new(),
            },
            Tag {
                id: TagId::new(),
                name: "db1_dbd4".to_string(),
                address: "DB1.DBD4".to_string(),
                attr: gateway_sdk::TagAttr::Read,
                data_type: Some("float".to_string()),
                description: Some("数据块1 双字4 浮点".to_string()),
                group_id: GroupId::new(),
            },
            Tag {
                id: TagId::new(),
                name: "input_0".to_string(),
                address: "I0.0".to_string(),
                attr: gateway_sdk::TagAttr::Read,
                data_type: Some("bool".to_string()),
                description: Some("输入0.0".to_string()),
                group_id: GroupId::new(),
            },
            Tag {
                id: TagId::new(),
                name: "marker_0".to_string(),
                address: "M0.0".to_string(),
                attr: gateway_sdk::TagAttr::Read,
                data_type: Some("bool".to_string()),
                description: Some("标志位0.0".to_string()),
                group_id: GroupId::new(),
            },
        ])
    }
}
