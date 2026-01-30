//! 南向示例插件：模拟设备。用于测试与演示，对标 Neuron 的 sim 驱动。
//! 可静态链接或编译为 .so（cdylib，`--features ffi`）经 FFI 动态加载。

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
use tokio::sync::RwLock;

/// 模拟设备插件
pub struct SimPlugin {
    /// 每个节点占用的“设备”状态（可扩展为更复杂结构）
    state: Arc<RwLock<HashMap<NodeId, SimState>>>,
}

struct SimState {
    _config: PluginConfig,
    /// 默认组与标签，用于 list_groups / list_tags / poll_group
    groups: Vec<Group>,
    tags: Vec<Tag>,
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

    fn default_groups() -> Vec<Group> {
        vec![Group {
            id: GroupId::new(),
            name: "default".to_string(),
            interval_ms: 1000,
            description: Some("默认采集组".to_string()),
        }]
    }

    fn default_tags(group_id: GroupId) -> Vec<Tag> {
        use gateway_sdk::TagAttr;
        let gid = group_id;
        vec![
            Tag {
                id: TagId::new(),
                name: "temperature".to_string(),
                address: "0".to_string(),
                attr: TagAttr::Read,
                data_type: Some("float64".to_string()),
                description: Some("模拟温度".to_string()),
                group_id: gid,
            },
            Tag {
                id: TagId::new(),
                name: "humidity".to_string(),
                address: "1".to_string(),
                attr: TagAttr::Read,
                data_type: Some("float64".to_string()),
                description: Some("模拟湿度".to_string()),
                group_id: gid,
            },
        ]
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
        Some(
            ConfigSchema::new()
                .param(ParamSchema {
                    name: "poll_base_ms".to_string(),
                    description: Some("Poll base interval (ms)".to_string()),
                    name_zh: Some("轮询基准间隔(ms)".to_string()),
                    name_en: Some("Poll base interval (ms)".to_string()),
                    description_zh: Some("采集轮询的基础间隔，单位毫秒".to_string()),
                    description_en: Some("Base interval for polling, in milliseconds".to_string()),
                    attribute: gateway_sdk::ParamAttribute::Optional,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(1000)),
                    valid: None,
                })
                .tag_regex(vec![
                    TagRegexEntry {
                        data_type: "float64".to_string(),
                        regex: r"^[0-9]+$".to_string(),
                    },
                ]),
        )
    }

    fn tag_schema(&self) -> Option<TagSchema> {
        Some(TagSchema {
            data_types: Some(vec!["float64".to_string()]),
            address_format: Some("0=temperature, 1=humidity".to_string()),
            address_format_zh: Some("0=温度, 1=湿度".to_string()),
            address_format_en: Some("0=temperature, 1=humidity".to_string()),
        })
    }

    async fn validate_tag(&self, _node_id: NodeId, tag: &Tag) -> PluginResult<()> {
        if tag.name.is_empty() {
            return Err(PluginError::tag_invalid("tag name required"));
        }
        if let Some(ref dt) = tag.data_type {
            if dt != "float64" {
                return Err(PluginError::tag_invalid("sim only supports float64"));
            }
        }
        Ok(())
    }

    async fn open(&self, node_id: NodeId, config: PluginConfig) -> PluginResult<()> {
        let groups = Self::default_groups();
        let tags = groups
            .iter()
            .flat_map(|g| Self::default_tags(g.id))
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
}
