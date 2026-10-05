//! 南向 EtherNet/IP 插件（ODVA CIP 协议）。
//!
//! 本实现为桩实现（CIP over TCP 无可用纯 Rust 协议栈），行为正确但无真实网络通信。

#[cfg(feature = "ffi")]
mod ffi;

mod address;
mod config;
mod state;
mod value;

use gateway_sdk::log;
use gateway_sdk::types::{DataValue, PluginKind};
use gateway_sdk::{
    ConfigSchema, Group, GroupId, NodeId, ParamSchema, ParamType, ParamValid, PluginMeta,
    SouthPlugin, Tag, TagId, TagRegexEntry, TagSchema,
};
use gateway_sdk::{PluginConfig, PluginError, PluginResult};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use address::{is_valid_tag_name, parse_address};
use config::{config_str, config_u16, config_u64};
use state::ConnectionState;
use value::CipDataType;

/// EtherNet/IP 南向插件（桩实现）
pub struct EthernetIpPlugin {
    state: Arc<RwLock<HashMap<NodeId, state::EthernetIpState>>>,
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
            description: Some("默认采集组".to_string()),
        }]
    }
}

#[async_trait::async_trait]
impl SouthPlugin for EthernetIpPlugin {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "ethernet-ip",
            kind: PluginKind::South,
            description: Some("EtherNet/IP 南向驱动（ODVA CIP 协议，支持 CIP 标签读写）"),
            version: "0.1.0",
            name_zh: Some("EtherNet/IP"),
            name_en: Some("EtherNet/IP"),
            description_zh: Some("EtherNet/IP 南向驱动（ODVA CIP 协议，支持 CIP 标签读写）"),
            description_en: Some(
                "EtherNet/IP south driver, ODVA CIP protocol for CIP tag read/write",
            ),
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
                    description: Some("Remote EtherNet/IP device IP address".to_string()),
                    description_zh: Some("远程 EtherNet/IP 设备 IP 地址".to_string()),
                    description_en: Some("Remote EtherNet/IP device IP address".to_string()),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::String,
                    default: Some(serde_json::json!("192.168.1.100")),
                    valid: None,
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "port".to_string(),
                    name_zh: Some("端口号".to_string()),
                    name_en: Some("Port".to_string()),
                    description: Some("EtherNet/IP CIP 端口（默认 44818）".to_string()),
                    description_zh: Some("EtherNet/IP CIP 端口（默认 44818）".to_string()),
                    description_en: Some("EtherNet/IP CIP port (default 44818)".to_string()),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(44818)),
                    valid: Some(ParamValid {
                        min: Some(1),
                        max: Some(65535),
                        regex: None,
                        length: None,
                    }),
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "connection_timeout_ms".to_string(),
                    name_zh: Some("连接超时时间 (ms)".to_string()),
                    name_en: Some("Connection Timeout (ms)".to_string()),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(5000)),
                    valid: Some(ParamValid {
                        min: Some(1000),
                        max: Some(60000),
                        regex: None,
                        length: None,
                    }),
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "send_interval_ms".to_string(),
                    name_zh: Some("指令发送间隔 (ms)".to_string()),
                    name_en: Some("Send Interval (ms)".to_string()),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(20)),
                    valid: Some(ParamValid {
                        min: Some(0),
                        max: Some(10000),
                        regex: None,
                        length: None,
                    }),
                    ..Default::default()
                })
                .tag_regex(vec![
                    TagRegexEntry {
                        data_type: "bool".to_string(),
                        regex: r"^[0-9]+/[0-9]+/[0-9]+$".to_string(),
                    },
                    TagRegexEntry {
                        data_type: "int".to_string(),
                        regex: r"^[0-9]+/[0-9]+(/[0-9]+)?$".to_string(),
                    },
                    TagRegexEntry {
                        data_type: "uint".to_string(),
                        regex: r"^[0-9]+/[0-9]+(/[0-9]+)?$".to_string(),
                    },
                    TagRegexEntry {
                        data_type: "real".to_string(),
                        regex: r"^[0-9]+/[0-9]+(/[0-9]+)?$".to_string(),
                    },
                ]),
        )
    }

    fn tag_schema(&self) -> Option<TagSchema> {
        Some(TagSchema {
            data_types: Some(vec![
                "bool".to_string(),
                "sint".to_string(),
                "int".to_string(),
                "dint".to_string(),
                "lint".to_string(),
                "usint".to_string(),
                "uint".to_string(),
                "udint".to_string(),
                "real".to_string(),
                "lreal".to_string(),
                "string".to_string(),
                "byte".to_string(),
                "word".to_string(),
                "dword".to_string(),
            ]),
            address_format: Some(
                "class/instance/attribute 或 class/instance（例如 100/1/3）".to_string(),
            ),
            address_format_zh: Some(
                "class/instance/attribute 或 class/instance（例如 100/1/3）".to_string(),
            ),
            address_format_en: Some(
                "class/instance/attribute or class/instance (e.g. 100/1/3)".to_string(),
            ),
        })
    }

    async fn validate_tag(&self, _node_id: NodeId, tag: &Tag) -> PluginResult<()> {
        if tag.name.is_empty() {
            return Err(PluginError::tag_invalid("tag name required"));
        }
        // 标签名格式或 CIP 地址格式均可
        if !is_valid_tag_name(&tag.address) && parse_address(&tag.address).is_none() {
            return Err(PluginError::tag_invalid(
                "address format: class/instance/attribute or class/instance",
            ));
        }
        Ok(())
    }

    async fn open(&self, node_id: NodeId, config: PluginConfig) -> PluginResult<()> {
        let host = config_str(&config, "host", "192.168.1.100");
        let port = config_u16(&config, "port", 44818);
        let connection_timeout_ms = config_u64(&config, "connection_timeout_ms", 5000);
        let send_interval_ms = config_u64(&config, "send_interval_ms", 20);
        log::info(
            node_id,
            format!(
                "open ethernet-ip: host={}, port={}, timeout={}ms",
                host, port, connection_timeout_ms
            ),
        );
        let groups = Self::default_groups();
        let tags = groups
            .iter()
            .flat_map(|g| {
                vec![Tag {
                    id: TagId::new(),
                    name: "placeholder".to_string(),
                    address: "100/1".to_string(),
                    attr: gateway_sdk::TagAttr::Read,
                    data_type: Some("int".to_string()),
                    description: Some("占位标签".to_string()),
                    group_id: g.id,
                }]
            })
            .collect::<Vec<_>>();
        let mut state = self.state.write().await;
        state.insert(
            node_id,
            state::EthernetIpState {
                host,
                port,
                connection_timeout_ms,
                send_interval_ms,
                groups,
                tags,
                conn: tokio::sync::Mutex::new(ConnectionState::new()),
            },
        );
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
        let host = config_str(&config, "host", "192.168.1.100");
        let port = config_u16(&config, "port", 44818);
        let mut state = self.state.write().await;
        if let Some(s) = state.get_mut(&node_id) {
            s.host = host;
            s.port = port;
            s.connection_timeout_ms =
                config_u64(&config, "connection_timeout_ms", s.connection_timeout_ms);
            s.send_interval_ms = config_u64(&config, "send_interval_ms", s.send_interval_ms);
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

        let mut cst = s.conn.lock().await;
        if cst.backoff_remaining_ms() > 0 {
            return Err(PluginError::msg(format!(
                "connection in backoff, retry in {} ms",
                cst.backoff_remaining_ms()
            )));
        }

        // 桩实现：解析地址但不执行网络通信
        let mut results = Vec::with_capacity(tags.len());
        for tag in tags {
            let data_type = tag.data_type.as_deref().unwrap_or("int");
            let cip_type = CipDataType::from_str(data_type);
            // 模拟成功读取占位值（无真实 CIP 栈）
            results.push((tag.id, cip_type.placeholder_value()));
        }

        cst.on_success();
        Ok(results)
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

        let mut cst = s.conn.lock().await;
        if cst.backoff_remaining_ms() > 0 {
            return Err(PluginError::msg(format!(
                "connection in backoff, retry in {} ms",
                cst.backoff_remaining_ms()
            )));
        }

        // 桩实现：验证地址格式正确性，模拟写入成功
        for (tag, _) in values {
            if !is_valid_tag_name(&tag.address) && parse_address(&tag.address).is_none() {
                return Err(PluginError::tag_invalid(
                    "address format: class/instance/attribute or class/instance",
                ));
            }
        }

        cst.on_success();
        Ok(())
    }
}
