//! Log operator: log data payload for debugging — passes data through unchanged.

use async_trait::async_trait;
use gateway_sdk::{Operable, PipelineData, PluginConfig, PluginMeta, PluginResult, NodeId};

pub struct LogOperator {
    level: String,
    format: String,
}

impl LogOperator {
    pub fn new(config: &PluginConfig) -> Self {
        let level = config.get("level").and_then(|v| v.as_str()).unwrap_or("info").to_string();
        let format = config.get("format").and_then(|v| v.as_str()).unwrap_or("json").to_string();
        Self { level, format }
    }
}

#[async_trait]
impl Operable for LogOperator {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "log",
            kind: gateway_sdk::PluginKind::Operator,
            description: Some("Log data for debugging — does not modify data"),
            version: "0.1.0",
            name_zh: Some("数据日志"),
            name_en: Some("Log"),
            description_zh: Some("记录数据日志用于调试，不修改数据本身"),
            description_en: Some("Log data payload for debugging — passes data through unchanged"),
        }
    }

    async fn open(&self, _node_id: NodeId, _config: PluginConfig) -> PluginResult<()> {
        Ok(())
    }

    async fn close(&self, _node_id: NodeId) -> PluginResult<()> {
        Ok(())
    }

    async fn process(&self, _node_id: NodeId, data: PipelineData) -> PluginResult<Vec<PipelineData>> {
        let log_line = if self.format == "json" {
            serde_json::to_string_pretty(&data.payload).unwrap_or_default()
        } else {
            format!("{:?}", data.payload)
        };

        match self.level.as_str() {
            "debug" => tracing::debug!(node_id = ?_node_id, "{}", log_line),
            "warn" => tracing::warn!(node_id = ?_node_id, "{}", log_line),
            "error" => tracing::error!(node_id = ?_node_id, "{}", log_line),
            _ => tracing::info!(node_id = ?_node_id, "{}", log_line),
        }

        Ok(vec![data])
    }

    async fn reset(&self, _node_id: NodeId) -> PluginResult<()> {
        Ok(())
    }
}

pub fn create_log_operator(config: &PluginConfig) -> Box<dyn Operable> {
    Box::new(LogOperator::new(config))
}
