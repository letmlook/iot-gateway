//! Join operator: join multiple data items into an array/batch.

use async_trait::async_trait;
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::RwLock;
use gateway_sdk::{Operable, PipelineData, PluginConfig, PluginMeta, PluginResult, DataValue, NodeId};

pub struct JoinOperator {
    max_items: usize,
    timeout_ms: u64,
    buffer: Arc<RwLock<VecDeque<PipelineData>>>,
}

impl JoinOperator {
    pub fn new(config: &PluginConfig) -> Self {
        let max_items = config.get("max_items").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
        let timeout_ms = config.get("timeout_ms").and_then(|v| v.as_u64()).unwrap_or(1000);
        Self {
            max_items,
            timeout_ms,
            buffer: Arc::new(RwLock::new(VecDeque::new())),
        }
    }
}

#[async_trait]
impl Operable for JoinOperator {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "join",
            kind: gateway_sdk::PluginKind::Operator,
            description: Some("Join multiple data items into an array/batch"),
            version: "0.1.0",
            name_zh: Some("数据合并"),
            name_en: Some("Join"),
            description_zh: Some("将多个数据项合并为一个数组或批次"),
            description_en: Some("Join multiple incoming data items into a single array/batch output"),
        }
    }

    async fn open(&self, _node_id: NodeId, _config: PluginConfig) -> PluginResult<()> {
        Ok(())
    }

    async fn close(&self, _node_id: NodeId) -> PluginResult<()> {
        Ok(())
    }

    async fn process(&self, _node_id: NodeId, data: PipelineData) -> PluginResult<Vec<PipelineData>> {
        let mut buffer = self.buffer.write().await;
        buffer.push_back(data);

        if buffer.len() >= self.max_items {
            let output_field = "joined";
            let items: Vec<_> = buffer.drain(..).map(|d| {
                serde_json::to_value(&d.payload).unwrap_or(serde_json::Value::Null)
            }).collect();

            let mut result = PipelineData::new(_node_id);
            result.payload.insert(output_field.to_string(), DataValue::String(serde_json::to_string(&items).unwrap_or_default()));
            Ok(vec![result])
        } else {
            Ok(vec![])
        }
    }

    async fn reset(&self, _node_id: NodeId) -> PluginResult<()> {
        self.buffer.write().await.clear();
        Ok(())
    }
}

pub fn create_join_operator(config: &PluginConfig) -> Box<dyn Operable> {
    Box::new(JoinOperator::new(config))
}
