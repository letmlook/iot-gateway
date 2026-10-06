//! 节点配置与运行时状态。节点 = 适配器 + 插件实例。

use gateway_sdk::types::{NodeId, NodeKind, NodeState};
use gateway_sdk::PluginConfig;
use serde::{Deserialize, Serialize};

/// 默认租户 ID（用于 serde default）
fn default_tenant() -> String {
    "default".to_string()
}

/// 节点配置（对应一个插件实例）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeConfig {
    pub id: NodeId,
    pub name: String,
    pub kind: NodeKind,
    /// 插件名称，如 "sim" / "mqtt"
    pub plugin_name: String,
    pub config: PluginConfig,
    /// 所属租户 ID
    #[serde(default = "default_tenant")]
    pub tenant_id: String,
}

/// 节点：配置 + 运行时状态
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    #[serde(flatten)]
    pub config: NodeConfig,
    pub state: NodeState,
}

impl Node {
    pub fn new(
        name: impl Into<String>,
        kind: NodeKind,
        plugin_name: impl Into<String>,
        config: PluginConfig,
    ) -> Self {
        Self {
            config: NodeConfig {
                id: NodeId::new(),
                name: name.into(),
                kind,
                plugin_name: plugin_name.into(),
                config,
                tenant_id: default_tenant(),
            },
            state: NodeState::Stopped,
        }
    }

    pub fn id(&self) -> NodeId {
        self.config.id
    }

    pub fn kind(&self) -> NodeKind {
        self.config.kind
    }

    pub fn plugin_name(&self) -> &str {
        &self.config.plugin_name
    }
}
