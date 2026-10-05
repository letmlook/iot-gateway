//! Siemens S7 南向插件（占位桩，待实现）。

use async_trait::async_trait;
use gateway_sdk::{PluginError, PluginResult, SouthPlugin};

#[derive(Default)]
pub struct S7Plugin;

impl S7Plugin {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl SouthPlugin for S7Plugin {
    fn meta(&self) -> gateway_sdk::PluginMeta {
        gateway_sdk::PluginMeta {
            name: "s7",
            kind: gateway_sdk::types::PluginKind::South,
            description: Some("Siemens S7 南向驱动（占位桩）"),
            version: "0.1.0",
            name_zh: Some("S7"),
            name_en: Some("S7"),
            description_zh: Some("Siemens S7 南向驱动（占位桩）"),
            description_en: Some("Siemens S7 south driver (placeholder stub)"),
        }
    }

    async fn validate_tag(
        &self,
        _node_id: gateway_sdk::NodeId,
        _tag: &gateway_sdk::Tag,
    ) -> PluginResult<()> {
        Err(PluginError::not_supported("s7 plugin not implemented"))
    }

    async fn open(
        &self,
        _node_id: gateway_sdk::NodeId,
        _config: gateway_sdk::PluginConfig,
    ) -> PluginResult<()> {
        Err(PluginError::not_supported("s7 plugin not implemented"))
    }

    async fn close(&self, _node_id: gateway_sdk::NodeId) -> PluginResult<()> {
        Ok(())
    }

    async fn poll_group(
        &self,
        _node_id: gateway_sdk::NodeId,
        _group_id: gateway_sdk::GroupId,
        _tags: &[gateway_sdk::Tag],
    ) -> PluginResult<Vec<(gateway_sdk::TagId, gateway_sdk::types::DataValue)>> {
        Err(PluginError::not_supported("s7 plugin not implemented"))
    }

    async fn list_groups(
        &self,
        _node_id: gateway_sdk::NodeId,
    ) -> PluginResult<Vec<gateway_sdk::Group>> {
        Err(PluginError::not_supported("s7 plugin not implemented"))
    }

    async fn list_tags(
        &self,
        _node_id: gateway_sdk::NodeId,
        _group_id: gateway_sdk::GroupId,
    ) -> PluginResult<Vec<gateway_sdk::Tag>> {
        Err(PluginError::not_supported("s7 plugin not implemented"))
    }

    async fn write_tags(
        &self,
        _node_id: gateway_sdk::NodeId,
        _values: &[(gateway_sdk::Tag, gateway_sdk::types::DataValue)],
    ) -> PluginResult<()> {
        Err(PluginError::not_supported("s7 plugin not implemented"))
    }
}
