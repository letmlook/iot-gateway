//! Buffer operator: buffers data and emits batches.

use async_trait::async_trait;
use gateway_sdk::{NodeId, Operable, PipelineData, PluginConfig, PluginMeta, PluginResult};
use std::collections::VecDeque;
use std::sync::RwLock;

/// Buffer flush strategy.
#[derive(Debug, Clone, Copy)]
pub enum FlushStrategy {
    /// Flush when buffer reaches this size.
    Count(usize),
    /// Flush after this many seconds (tumbling).
    TimeoutSecs(u64),
    /// Flush when either condition is met.
    Either { count: usize, timeout_secs: u64 },
}

pub struct BufferOperator {
    batch_size: usize,
    flush_strategy: FlushStrategy,
    buffer: RwLock<VecDeque<PipelineData>>,
}

impl BufferOperator {
    pub fn new(batch_size: usize, timeout_secs: u64) -> Self {
        let flush_strategy = if batch_size > 0 && timeout_secs > 0 {
            FlushStrategy::Either {
                count: batch_size,
                timeout_secs,
            }
        } else if batch_size > 0 {
            FlushStrategy::Count(batch_size)
        } else {
            FlushStrategy::TimeoutSecs(timeout_secs.max(1))
        };
        Self {
            batch_size,
            flush_strategy,
            buffer: RwLock::new(VecDeque::new()),
        }
    }
}

#[async_trait]
impl Operable for BufferOperator {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "buffer",
            kind: gateway_sdk::PluginKind::Operator,
            description: Some("Buffer data and emit in batches"),
            version: "0.1.0",
            name_zh: Some("缓冲器"),
            name_en: Some("Buffer"),
            description_zh: Some("缓冲数据并批量输出，支持按数量或时间触发"),
            description_en: Some("Buffer data and emit in batches, triggered by count or time"),
        }
    }

    async fn open(&self, _node_id: NodeId, _config: PluginConfig) -> PluginResult<()> {
        self.buffer.write().unwrap().clear();
        Ok(())
    }

    async fn close(&self, _node_id: NodeId) -> PluginResult<()> {
        Ok(())
    }

    async fn process(
        &self,
        _node_id: NodeId,
        data: PipelineData,
    ) -> PluginResult<Vec<PipelineData>> {
        let flush_count = match self.flush_strategy {
            FlushStrategy::Count(count) => count,
            FlushStrategy::Either { count, .. } => count,
            FlushStrategy::TimeoutSecs(_) => self.batch_size,
        }
        .max(1);

        let mut buffer = self.buffer.write().unwrap();
        buffer.push_back(data);
        if buffer.len() < flush_count {
            return Ok(Vec::new());
        }
        Ok(buffer.drain(..).collect())
    }

    async fn reset(&self, _node_id: NodeId) -> PluginResult<()> {
        self.buffer.write().unwrap().clear();
        Ok(())
    }
}

/// Create a buffer operator from config.
/// config expects: { "batch_size": 100, "timeout_secs": 5 }
pub fn create_buffer_operator(config: &PluginConfig) -> Box<dyn Operable> {
    let batch_size = config
        .get("batch_size")
        .or_else(|| config.get("max_size"))
        .and_then(|v| v.as_u64())
        .unwrap_or(100) as usize;
    let timeout_secs = config
        .get("timeout_secs")
        .and_then(|v| v.as_u64())
        .unwrap_or(5);
    Box::new(BufferOperator::new(batch_size, timeout_secs))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gateway_sdk::DataValue;

    fn sample(seq: i64) -> PipelineData {
        let mut data = PipelineData::new(NodeId::new());
        data.payload
            .insert("seq".to_string(), DataValue::Int64(seq));
        data
    }

    #[tokio::test]
    async fn buffer_flushes_when_max_size_is_reached() {
        let operator = BufferOperator::new(3, 0);
        let node_id = NodeId::new();

        assert!(operator
            .process(node_id, sample(1))
            .await
            .unwrap()
            .is_empty());
        assert!(operator
            .process(node_id, sample(2))
            .await
            .unwrap()
            .is_empty());

        let output = operator.process(node_id, sample(3)).await.unwrap();
        assert_eq!(output.len(), 3);
        assert_eq!(output[0].payload.get("seq"), Some(&DataValue::Int64(1)));
        assert_eq!(output[1].payload.get("seq"), Some(&DataValue::Int64(2)));
        assert_eq!(output[2].payload.get("seq"), Some(&DataValue::Int64(3)));
    }
}
