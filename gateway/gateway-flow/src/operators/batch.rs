//! Batch operator: collect data into batches and emit when batch size or time window is reached.

use async_trait::async_trait;
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::RwLock;
use gateway_sdk::{Operable, PipelineData, PluginConfig, PluginMeta, PluginResult, NodeId};

pub struct BatchOperator {
    max_size: usize,
    timeout_ms: u64,
    buffer: Arc<RwLock<VecDeque<PipelineData>>>,
    last_flush: Arc<RwLock<i64>>,
}

impl BatchOperator {
    pub fn new(config: &PluginConfig) -> Self {
        let max_size = config.get("max_size").and_then(|v| v.as_u64()).unwrap_or(100) as usize;
        let timeout_ms = config.get("timeout_ms").and_then(|v| v.as_u64()).unwrap_or(5000);
        Self {
            max_size,
            timeout_ms,
            buffer: Arc::new(RwLock::new(VecDeque::new())),
            last_flush: Arc::new(RwLock::new(chrono::Utc::now().timestamp_millis())),
        }
    }
}

#[async_trait]
impl Operable for BatchOperator {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "batch",
            kind: gateway_sdk::PluginKind::Operator,
            description: Some("Collect data into batches"),
            version: "0.1.0",
            name_zh: Some("批量处理"),
            name_en: Some("Batch"),
            description_zh: Some("将数据收集到批次，达到批次大小或超时时间时输出"),
            description_en: Some("Collect data into batches — emit when size threshold or time window reached"),
        }
    }

    async fn open(&self, _node_id: NodeId, _config: PluginConfig) -> PluginResult<()> {
        Ok(())
    }

    async fn close(&self, _node_id: NodeId) -> PluginResult<()> {
        Ok(())
    }

    async fn process(&self, _node_id: NodeId, data: PipelineData) -> PluginResult<Vec<PipelineData>> {
        let now = chrono::Utc::now().timestamp_millis();

        let mut buffer = self.buffer.write().await;
        let mut last = self.last_flush.write().await;

        buffer.push_back(data);

        let should_flush = buffer.len() >= self.max_size ||
            (now - *last) >= self.timeout_ms as i64;

        if should_flush && !buffer.is_empty() {
            let batch_size = buffer.len();
            let mut batch = Vec::with_capacity(batch_size);
            while let Some(item) = buffer.pop_front() {
                batch.push(item);
            }
            *last = now;
            Ok(batch)
        } else {
            Ok(vec![])
        }
    }

    async fn reset(&self, _node_id: NodeId) -> PluginResult<()> {
        self.buffer.write().await.clear();
        *self.last_flush.write().await = chrono::Utc::now().timestamp_millis();
        Ok(())
    }
}

pub fn create_batch_operator(config: &PluginConfig) -> Box<dyn Operable> {
    Box::new(BatchOperator::new(config))
}
