//! Mitsubishi MC (MELSEC) PLC communication protocol plugin.
//! Supports Q/iQ-R/iQ-F series via MC-3E/4E protocol.
//! Address format: `D100` (data register), `X0` (input), `Y0` (output), `M100` (marker/relay), `W100` (link register)

#[cfg(feature = "ffi")]
mod ffi;

mod state;

use gateway_sdk::{
    ConfigSchema, Group, GroupId, NodeId, ParamSchema, ParamType, ParamValid,
    PluginMeta, SouthPlugin, Tag, TagId, TagRegexEntry, TagSchema,
};
use gateway_sdk::log;
use gateway_sdk::types::{DataValue, PluginKind};
use gateway_sdk::{PluginConfig, PluginError, PluginResult};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use state::MCState;

/// Mitsubishi MC south plugin
pub struct MitsubishiMcPlugin {
    state: Arc<RwLock<HashMap<NodeId, MCState>>>,
}

impl Default for MitsubishiMcPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl MitsubishiMcPlugin {
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
            description: Some("Default polling group".to_string()),
        }]
    }
}

#[async_trait::async_trait]
impl SouthPlugin for MitsubishiMcPlugin {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "mitsubishi-mc",
            kind: PluginKind::South,
            description: Some("Mitsubishi MELSEC PLC communication protocol (MC-3E/4E)"),
            version: "0.1.0",
            name_zh: Some("三菱MC"),
            name_en: Some("Mitsubishi MC"),
            description_zh: Some("三菱MELSEC PLC通信协议，支持Q/iQ-R/iQ-F系列"),
            description_en: Some("Mitsubishi MELSEC PLC protocol — Q/iQ-R/iQ-F series via MC-3E/4E"),
        }
    }

    fn config_schema(&self) -> Option<ConfigSchema> {
        use gateway_sdk::ParamAttribute;
        Some(
            ConfigSchema::new()
                .param(ParamSchema {
                    name: "host".to_string(),
                    name_zh: Some("IP地址".to_string()),
                    name_en: Some("IP Address".to_string()),
                    description: Some("Mitsubishi PLC IP address".to_string()),
                    description_zh: Some("三菱 PLC IP 地址".to_string()),
                    description_en: Some("Mitsubishi PLC IP address".to_string()),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::String,
                    default: Some(serde_json::json!("192.168.1.10")),
                    valid: None,
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "port".to_string(),
                    name_zh: Some("端口号".to_string()),
                    name_en: Some("Port".to_string()),
                    description: Some("MC protocol TCP port (default 5002)".to_string()),
                    description_zh: Some("MC 协议 TCP 端口（默认 5002）".to_string()),
                    description_en: Some("MC protocol TCP port (default 5002)".to_string()),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(5002)),
                    valid: Some(ParamValid { min: Some(1), max: Some(65535), regex: None, length: None }),
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "network".to_string(),
                    name_zh: Some("网络号".to_string()),
                    name_en: Some("Network Number".to_string()),
                    description: Some("Mitsubishi network number (0-255)".to_string()),
                    description_zh: Some("三菱网络号（0-255）".to_string()),
                    description_en: Some("Mitsubishi network number (0-255)".to_string()),
                    attribute: ParamAttribute::Optional,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(0)),
                    valid: Some(ParamValid { min: Some(0), max: Some(255), regex: None, length: None }),
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "station".to_string(),
                    name_zh: Some("站号".to_string()),
                    name_en: Some("Station Number".to_string()),
                    description: Some("Mitsubishi station number (0-31)".to_string()),
                    description_zh: Some("三菱站号（0-31）".to_string()),
                    description_en: Some("Mitsubishi station number (0-31)".to_string()),
                    attribute: ParamAttribute::Optional,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(0)),
                    valid: Some(ParamValid { min: Some(0), max: Some(31), regex: None, length: None }),
                    ..Default::default()
                })
                .tag_regex(vec![
                    TagRegexEntry { data_type: "bool".to_string(), regex: r"^[XYMMLB][0-9]+$".to_string() },
                    TagRegexEntry { data_type: "int16".to_string(), regex: r"^[DW][0-9]+$".to_string() },
                    TagRegexEntry { data_type: "uint16".to_string(), regex: r"^[DW][0-9]+$".to_string() },
                    TagRegexEntry { data_type: "int32".to_string(), regex: r"^[DW][0-9]+$".to_string() },
                    TagRegexEntry { data_type: "uint32".to_string(), regex: r"^[DW][0-9]+$".to_string() },
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
                "float32".to_string(),
                "float64".to_string(),
                "string".to_string(),
            ]),
            address_format: Some("D100 (data register), X0 (input), Y0 (output), M100 (marker), W100 (link)".to_string()),
            address_format_zh: Some("D100 (数据寄存器), X0 (输入), Y0 (输出), M100 (继电器), W100 (链接寄存器)".to_string()),
            address_format_en: Some("D100 (data register), X0 (input), Y0 (output), M100 (marker), W100 (link)".to_string()),
        })
    }

    async fn validate_tag(&self, _node_id: NodeId, tag: &Tag) -> PluginResult<()> {
        if tag.name.is_empty() {
            return Err(PluginError::tag_invalid("tag name required"));
        }
        let addr = &tag.address;
        if addr.is_empty() {
            return Err(PluginError::tag_invalid("address required"));
        }
        // Validate Mitsubishi address prefixes
        let valid_prefixes = ["D", "W", "M", "X", "Y", "L", "B", "SM", "SD", "CN", "TN", "TS", "TC"];
        let starts_with_valid = valid_prefixes.iter().any(|p| addr.starts_with(p));
        if !starts_with_valid {
            return Err(PluginError::tag_invalid("invalid Mitsubishi address prefix"));
        }
        Ok(())
    }

    async fn open(&self, node_id: NodeId, config: PluginConfig) -> PluginResult<()> {
        let host = config.get("host")
            .and_then(|v| v.as_str())
            .unwrap_or("192.168.1.10")
            .to_string();
        let port = config.get("port")
            .and_then(|v| v.as_u64())
            .unwrap_or(5002) as u16;
        let network = config.get("network")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u8;
        let station = config.get("station")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u8;

        log::info(node_id, format!("open mitsubishi-mc: host={}, port={}, network={}, station={}", host, port, network, station));

        let groups = Self::default_groups();
        let tags = groups.iter().flat_map(|g| {
            vec![
                Tag {
                    id: TagId::new(),
                    name: "d100".to_string(),
                    address: "D100".to_string(),
                    attr: gateway_sdk::TagAttr::Read,
                    data_type: Some("int16".to_string()),
                    description: Some("Data register 100".to_string()),
                    group_id: g.id,
                },
            ]
        }).collect::<Vec<_>>();

        let mut state = self.state.write().await;
        state.insert(node_id, MCState {
            host,
            port,
            network,
            station,
            groups,
            tags,
        });
        Ok(())
    }

    async fn close(&self, node_id: NodeId) -> PluginResult<()> {
        log::info(node_id, "close mitsubishi-mc");
        let mut state = self.state.write().await;
        state.remove(&node_id);
        Ok(())
    }

    async fn start(&self, node_id: NodeId) -> PluginResult<()> {
        log::info(node_id, "start mitsubishi-mc");
        Ok(())
    }

    async fn stop(&self, node_id: NodeId) -> PluginResult<()> {
        log::info(node_id, "stop mitsubishi-mc");
        Ok(())
    }

    async fn setting(&self, node_id: NodeId, config: PluginConfig) -> PluginResult<()> {
        log::info(node_id, "setting mitsubishi-mc (config updated)");
        let mut state = self.state.write().await;
        if let Some(s) = state.get_mut(&node_id) {
            s.host = config.get("host")
                .and_then(|v| v.as_str())
                .unwrap_or(&s.host)
                .to_string();
            s.port = config.get("port")
                .and_then(|v| v.as_u64())
                .unwrap_or(s.port as u64) as u16;
            s.network = config.get("network")
                .and_then(|v| v.as_u64())
                .unwrap_or(s.network as u64) as u8;
            s.station = config.get("station")
                .and_then(|v| v.as_u64())
                .unwrap_or(s.station as u64) as u8;
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
        let _s = state.get(&node_id).ok_or_else(|| PluginError::msg("node not open"))?;

        // Stub implementation - real Mitsubishi MC would use melsec crate
        let mut results = Vec::with_capacity(tags.len());
        for tag in tags {
            let addr = &tag.address;

            // Determine value based on address prefix (stub data)
            let value = match addr.chars().next() {
                Some('D') | Some('W') => DataValue::Int64(1234), // Word devices
                Some('M') | Some('L') | Some('B') => DataValue::Bool(true), // Bit devices
                Some('X') | Some('Y') => DataValue::Bool(false), // Input/output
                _ => DataValue::Int64(0),
            };

            results.push((tag.id, value));
        }

        Ok(results)
    }

    async fn write_tags(&self, node_id: NodeId, _values: &[(Tag, DataValue)]) -> PluginResult<()> {
        log::info(node_id, "write_tags mitsubishi-mc (stub - not implemented)");
        Err(PluginError::msg("write not implemented in stub"))
    }

    async fn list_groups(&self, node_id: NodeId) -> PluginResult<Vec<Group>> {
        let state = self.state.read().await;
        let s = state.get(&node_id).ok_or_else(|| PluginError::msg("node not open"))?;
        Ok(s.groups.clone())
    }

    async fn list_tags(&self, node_id: NodeId, _group_id: GroupId) -> PluginResult<Vec<Tag>> {
        let state = self.state.read().await;
        let s = state.get(&node_id).ok_or_else(|| PluginError::msg("node not open"))?;
        Ok(s.tags.clone())
    }
}
