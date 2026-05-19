//! Buffer operator: buffers data and emits batches.

use async_trait::async_trait;
use std::collections::VecDeque;
use std::sync::RwLock;
use gateway_sdk::{
    Operable, PipelineData, PluginConfig, PluginMeta, PluginResult,
    NodeId,
};

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
            FlushStrategy::Either { count: batch_size, timeout_secs }
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
        Ok(())
    }
    
    async fn close(&self, _node_id: NodeId) -> PluginResult<()> {
        Ok(())
    }
    
    async fn process(&self, _node_id: NodeId, data: PipelineData) -> PluginResult<Vec<PipelineData>> {
        // For Phase 1: just pass through immediately (timing logic in Phase 2)
        // The DAG executor will handle actual timing/flush in later phases.
        Ok(vec![data])
    }
    
    async fn reset(&self, _node_id: NodeId) -> PluginResult<()> {
        self.buffer.write().unwrap().clear();
        Ok(())
    }
}

/// Create a buffer operator from config.
/// config expects: { "batch_size": 100, "timeout_secs": 5 }
pub fn create_buffer_operator(config: &PluginConfig) -> Box<dyn Operable> {
    let batch_size = config.get("batch_size")
        .and_then(|v| v.as_u64())
        .unwrap_or(100) as usize;
    let timeout_secs = config.get("timeout_secs")
        .and_then(|v| v.as_u64())
        .unwrap_or(5);
    Box::new(BufferOperator::new(batch_size, timeout_secs))
}
