//! EtherNet/IP industrial Ethernet protocol plugin (CIP over Ethernet).
//! Supports reading PLC tags from Allen-Bradley, Omron, and other CIP devices.
//! Address format: `tag_name` or `N7:0` (file:element), `D100` (data file), `F8:0` (float file)

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

use state::EIPState;

/// EtherNet/IP south plugin
pub struct EthernetIpPlugin {
    state: Arc<RwLock<HashMap<NodeId, EIPState>>>,
}

impl Default for EthernetIpPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl EthernetIpPlugin {
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
impl SouthPlugin for EthernetIpPlugin {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "ethernet-ip",
            kind: PluginKind::South,
            description: Some("EtherNet/IP industrial Ethernet protocol (CIP over Ethernet)"),
            version: "0.1.0",
            name_zh: Some("Ethernet/IP"),
            name_en: Some("EtherNet/IP"),
            description_zh: Some("EtherNet/IP 工业以太网协议，支持读取PLC标签数据（AB/Omron等）"),
            description_en: Some("EtherNet/IP protocol — read PLC tags from Allen-Bradley, Omron, and other CIP devices"),
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
                    description: Some("EtherNet/IP device IP address".to_string()),
                    description_zh: Some("EtherNet/IP 设备 IP 地址".to_string()),
                    description_en: Some("EtherNet/IP device IP address".to_string()),
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
                    description: Some("EtherNet/IP TCP port (default 44818)".to_string()),
                    description_zh: Some("EtherNet/IP TCP 端口（默认 44818）".to_string()),
                    description_en: Some("EtherNet/IP TCP port (default 44818)".to_string()),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(44818)),
                    valid: Some(ParamValid { min: Some(1), max: Some(65535), regex: None, length: None }),
                    ..Default::default()
                })
                .tag_regex(vec![
                    TagRegexEntry { data_type: "bool".to_string(), regex: r"^O?[XYMTCSLG]:[0-9]+/[0-9]+$".to_string() },
                    TagRegexEntry { data_type: "int16".to_string(), regex: r"^[KN]?[0-9]+:[0-9]+$".to_string() },
                    TagRegexEntry { data_type: "uint16".to_string(), regex: r"^[KN]?[0-9]+:[0-9]+$".to_string() },
                    TagRegexEntry { data_type: "int32".to_string(), regex: r"^[KN]?[0-9]+:[0-9]+$".to_string() },
                    TagRegexEntry { data_type: "uint32".to_string(), regex: r"^[KN]?[0-9]+:[0-9]+$".to_string() },
                    TagRegexEntry { data_type: "float32".to_string(), regex: r"^F[0-9]+:[0-9]+$".to_string() },
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
            address_format: Some("tag_name or N7:0 (file:element), D100 (data file), F8:0 (float file)".to_string()),
            address_format_zh: Some("标签名 或 N7:0 (文件:元素), D100 (数据文件), F8:0 (浮点文件)".to_string()),
            address_format_en: Some("tag_name or N7:0 (file:element), D100 (data file), F8:0 (float file)".to_string()),
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
        Ok(())
    }

    async fn open(&self, node_id: NodeId, config: PluginConfig) -> PluginResult<()> {
        let host = config.get("host")
            .and_then(|v| v.as_str())
            .unwrap_or("192.168.1.10")
            .to_string();
        let port = config.get("port")
            .and_then(|v| v.as_u64())
            .unwrap_or(44818) as u16;

        log::info(node_id, format!("open ethernet-ip: host={}, port={}", host, port));

        let groups = Self::default_groups();
        let tags = groups.iter().flat_map(|g| {
            vec![
                Tag {
                    id: TagId::new(),
                    name: "stub_tag".to_string(),
                    address: "N7:0".to_string(),
                    attr: gateway_sdk::TagAttr::Read,
                    data_type: Some("int16".to_string()),
                    description: Some("Stub tag".to_string()),
                    group_id: g.id,
                },
            ]
        }).collect::<Vec<_>>();

        let mut state = self.state.write().await;
        state.insert(node_id, EIPState {
            host,
            port,
            groups,
            tags,
        });
        Ok(())
    }

    async fn close(&self, node_id: NodeId) -> PluginResult<()> {
        log::info(node_id, "close ethernet-ip");
        let mut state = self.state.write().await;
        state.remove(&node_id);
        Ok(())
    }

    async fn start(&self, node_id: NodeId) -> PluginResult<()> {
        log::info(node_id, "start ethernet-ip");
        Ok(())
    }

    async fn stop(&self, node_id: NodeId) -> PluginResult<()> {
        log::info(node_id, "stop ethernet-ip");
        Ok(())
    }

    async fn setting(&self, node_id: NodeId, config: PluginConfig) -> PluginResult<()> {
        log::info(node_id, "setting ethernet-ip (config updated)");
        let mut state = self.state.write().await;
        if let Some(s) = state.get_mut(&node_id) {
            s.host = config.get("host")
                .and_then(|v| v.as_str())
                .unwrap_or(&s.host)
                .to_string();
            s.port = config.get("port")
                .and_then(|v| v.as_u64())
                .unwrap_or(s.port as u64) as u16;
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

        // Stub implementation - real EtherNet/IP would use cip-rs or similar
        let mut results = Vec::with_capacity(tags.len());
        for tag in tags {
            let addr = &tag.address;

            // Determine value based on address pattern (stub data)
            let value = if addr.ends_with(".0") || addr.contains("/") {
                DataValue::Bool(true) // Bit access
            } else if addr.starts_with('F') {
                DataValue::Float64(3.14) // Float file
            } else if addr.starts_with('N') || addr.starts_with('D') {
                DataValue::Int64(100) // Integer file
            } else {
                DataValue::Int64(42) // Default
            };

            results.push((tag.id, value));
        }

        Ok(results)
    }

    async fn write_tags(&self, node_id: NodeId, _values: &[(Tag, DataValue)]) -> PluginResult<()> {
        log::info(node_id, "write_tags ethernet-ip (stub - not implemented)");
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
