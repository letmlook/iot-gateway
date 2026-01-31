//! 南向 OPC UA 插件。
//! 功能：端点 URL、用户名/密码、证书/密钥、地址格式 NS!NODEID、读写。

#[cfg(feature = "ffi")]
mod ffi;

mod address;
mod config;
mod state;

#[cfg(feature = "opcua-client")]
mod client;

use gateway_sdk::{
    ConfigSchema, Group, GroupId, NodeId, PluginMeta, SouthPlugin, Tag, TagId, TagSchema,
};
use gateway_sdk::log;
use gateway_sdk::types::{DataValue, PluginKind};
use gateway_sdk::{PluginConfig, PluginError, PluginResult};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use address::parse_address;
use config::{config_str, tag_schema as config_tag_schema, TAG_DATA_TYPES, DEFAULT_ENDPOINT};
use state::OpcuaState;

/// OPC UA 南向插件
pub struct OpcuaPlugin {
    state: Arc<RwLock<HashMap<NodeId, OpcuaState>>>,
}

impl Default for OpcuaPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl OpcuaPlugin {
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
impl SouthPlugin for OpcuaPlugin {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "opcua",
            kind: PluginKind::South,
            description: Some("OPC UA 南向驱动，连接 OPC UA 服务器采集与写点位"),
            version: "0.1.0",
            name_zh: Some("OPC UA"),
            name_en: Some("OPC UA"),
            description_zh: Some("OPC UA 南向驱动，连接 OPC UA 服务器采集与写点位"),
            description_en: Some("OPC UA south driver, connect to OPC UA server for read/write tags"),
        }
    }

    fn config_schema(&self) -> Option<ConfigSchema> {
        Some(config::config_schema())
    }

    fn tag_schema(&self) -> Option<TagSchema> {
        Some(config_tag_schema())
    }

    async fn validate_tag(&self, _node_id: NodeId, tag: &Tag) -> PluginResult<()> {
        if tag.name.is_empty() {
            return Err(PluginError::tag_invalid("tag name required"));
        }
        parse_address(&tag.address)
            .ok_or_else(|| PluginError::tag_invalid("address format: NS!NODEID (e.g. 0!2258 or 2!Device1.Tag1)"))?;
        if let Some(ref dt) = tag.data_type {
            let ok = TAG_DATA_TYPES
                .iter()
                .any(|allowed| allowed.eq_ignore_ascii_case(dt.as_str()));
            if !ok {
                return Err(PluginError::tag_invalid(format!(
                    "data_type must be one of: {}",
                    TAG_DATA_TYPES.join(", ")
                )));
            }
        }
        Ok(())
    }

    async fn open(&self, node_id: NodeId, config: PluginConfig) -> PluginResult<()> {
        let endpoint_url = config_str(&config, "endpoint_url", DEFAULT_ENDPOINT);
        log::info(node_id, format!("open opcua: endpoint={}", endpoint_url));
        let username = config.get("username").and_then(|v| v.as_str()).map(String::from);
        let password = config.get("password").and_then(|v| v.as_str()).map(String::from);
        let certificate = config.get("certificate").and_then(|v| v.as_str()).map(String::from);
        let key = config.get("key").and_then(|v| v.as_str()).map(String::from);
        let groups = Self::default_groups();
        let tags = groups
            .iter()
            .flat_map(|g| {
                vec![Tag {
                    id: TagId::new(),
                    name: "ServerTimestamp".to_string(),
                    address: "0!2258".to_string(),
                    attr: gateway_sdk::TagAttr::Read,
                    data_type: Some("uint32".to_string()),
                    description: Some("OPC UA 服务器时间戳".to_string()),
                    group_id: g.id,
                }]
            })
            .collect::<Vec<_>>();
        let mut state = self.state.write().await;
        state.insert(
            node_id,
            OpcuaState {
                endpoint_url,
                username,
                password,
                certificate,
                key,
                groups,
                tags,
            },
        );
        Ok(())
    }

    async fn close(&self, node_id: NodeId) -> PluginResult<()> {
        log::info(node_id, "close opcua");
        let mut state = self.state.write().await;
        state.remove(&node_id);
        Ok(())
    }

    async fn start(&self, node_id: NodeId) -> PluginResult<()> {
        log::info(node_id, "start opcua");
        Ok(())
    }

    async fn stop(&self, node_id: NodeId) -> PluginResult<()> {
        log::info(node_id, "stop opcua");
        Ok(())
    }

    async fn setting(&self, node_id: NodeId, config: PluginConfig) -> PluginResult<()> {
        log::info(node_id, "setting opcua (config updated)");
        let endpoint_url = config_str(&config, "endpoint_url", DEFAULT_ENDPOINT);
        let mut state = self.state.write().await;
        if let Some(s) = state.get_mut(&node_id) {
            s.endpoint_url = endpoint_url;
            s.username = config.get("username").and_then(|v| v.as_str()).map(String::from);
            s.password = config.get("password").and_then(|v| v.as_str()).map(String::from);
            s.certificate = config.get("certificate").and_then(|v| v.as_str()).map(String::from);
            s.key = config.get("key").and_then(|v| v.as_str()).map(String::from);
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
        let s = state.get(&node_id).ok_or_else(|| PluginError::msg("node not open"))?;

        #[cfg(feature = "opcua-client")]
        {
            let endpoint_url = s.endpoint_url.clone();
            let username = s.username.clone();
            let password = s.password.clone();
            let tags_vec: Vec<(TagId, Tag)> = tags.iter().map(|t| (t.id, t.clone())).collect();
            let result = tokio::task::spawn_blocking(move || {
                client::opcua_read(&endpoint_url, username.as_deref(), password.as_deref(), &tags_vec)
            })
            .await
            .map_err(|e| PluginError::msg(format!("spawn_blocking: {}", e)))?;
            result
        }

        #[cfg(not(feature = "opcua-client"))]
        {
            let _ = (node_id, s);
            let mut out = Vec::with_capacity(tags.len());
            for tag in tags {
                let _ = parse_address(&tag.address);
                out.push((tag.id, DataValue::UInt32(0)));
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

        #[cfg(feature = "opcua-client")]
        {
            let endpoint_url = s.endpoint_url.clone();
            let username = s.username.clone();
            let password = s.password.clone();
            let values_vec: Vec<(Tag, DataValue)> = values.to_vec();
            let result = tokio::task::spawn_blocking(move || {
                client::opcua_write(&endpoint_url, username.as_deref(), password.as_deref(), &values_vec)
            })
            .await
            .map_err(|e| PluginError::msg(format!("spawn_blocking: {}", e)))?;
            result
        }

        #[cfg(not(feature = "opcua-client"))]
        {
            let _ = (node_id, s, values);
            Err(PluginError::not_supported("write requires opcua-client feature"))
        }
    }
}
