//! 南向示例插件：模拟设备。用于测试与演示。
//! 可静态链接或编译为 .so（cdylib，`--features ffi`）经 FFI 动态加载。

#[cfg(feature = "ffi")]
mod ffi;

mod schema;
mod state;

use gateway_sdk::log;
use gateway_sdk::types::{DataValue, PluginKind};
use gateway_sdk::{
    ConfigSchema, Group, GroupId, NodeId, PluginMeta, SouthPlugin, Tag, TagId, TagSchema,
};
use gateway_sdk::{PluginConfig, PluginError, PluginResult};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use schema::{config_schema as build_config_schema, tag_schema as build_tag_schema};
use state::{default_groups, default_tags, SimState};

/// 模拟设备插件
pub struct SimPlugin {
    /// 每个节点占用的“设备”状态（可扩展为更复杂结构）
    state: Arc<RwLock<HashMap<NodeId, SimState>>>,
}

impl Default for SimPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl SimPlugin {
    pub fn new() -> Self {
        Self {
            state: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

#[async_trait::async_trait]
impl SouthPlugin for SimPlugin {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "sim",
            kind: PluginKind::South,
            description: Some("模拟设备，用于测试与演示"),
            version: "0.1.0",
            name_zh: Some("模拟设备"),
            name_en: Some("Simulator"),
            description_zh: Some("模拟设备，用于测试与演示"),
            description_en: Some("Simulated device for testing and demo"),
        }
    }

    fn config_schema(&self) -> Option<ConfigSchema> {
        Some(build_config_schema())
    }

    fn tag_schema(&self) -> Option<TagSchema> {
        Some(build_tag_schema())
    }

    async fn validate_tag(&self, _node_id: NodeId, tag: &Tag) -> PluginResult<()> {
        if tag.name.is_empty() {
            return Err(PluginError::tag_invalid("tag name required"));
        }
        if let Some(ref dt) = tag.data_type {
            if !dt.eq_ignore_ascii_case("float64") {
                return Err(PluginError::tag_invalid("sim only supports float64"));
            }
        }
        Ok(())
    }

    async fn open(&self, node_id: NodeId, config: PluginConfig) -> PluginResult<()> {
        log::info(node_id, "open sim (default groups/tags)");
        let groups = default_groups();
        let tags = groups
            .iter()
            .flat_map(|g| default_tags(g.id))
            .collect::<Vec<_>>();
        let mut state = self.state.write().await;
        state.insert(
            node_id,
            SimState {
                _config: config,
                groups,
                tags,
            },
        );
        Ok(())
    }

    async fn close(&self, node_id: NodeId) -> PluginResult<()> {
        log::info(node_id, "close sim");
        let mut state = self.state.write().await;
        state.remove(&node_id);
        Ok(())
    }

    async fn poll_group(
        &self,
        node_id: NodeId,
        group_id: GroupId,
        tags: &[Tag],
    ) -> PluginResult<Vec<(TagId, DataValue)>> {
        let _ = (node_id, group_id);
        use std::time::{SystemTime, UNIX_EPOCH};
        let t = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs_f64();
        let mut out = Vec::with_capacity(tags.len());
        for tag in tags {
            let v = if tag.name == "temperature" {
                DataValue::Float64(20.0 + 5.0 * (t * 0.1).sin())
            } else if tag.name == "humidity" {
                DataValue::Float64(50.0 + 10.0 * (t * 0.07).cos())
            } else {
                DataValue::Float64(0.0)
            };
            out.push((tag.id, v));
        }
        Ok(out)
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
}
