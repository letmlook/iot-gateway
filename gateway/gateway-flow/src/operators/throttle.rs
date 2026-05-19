//! Throttle operator: limit data flow rate — pass one item every N milliseconds.

use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{sleep, Duration, Instant};
use gateway_sdk::{Operable, PipelineData, PluginConfig, PluginMeta, PluginResult, NodeId};

pub struct ThrottleOperator {
    interval_ms: u64,
    last_pass: Arc<RwLock<Instant>>,
}

impl ThrottleOperator {
    pub fn new(config: &PluginConfig) -> Self {
        let interval_ms = config.get("interval_ms").and_then(|v| v.as_u64()).unwrap_or(1000);
        Self {
            interval_ms,
            last_pass: Arc::new(RwLock::new(Instant::now() - Duration::from_secs(100))),
        }
    }
}

#[async_trait]
impl Operable for ThrottleOperator {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "throttle",
            kind: gateway_sdk::PluginKind::Operator,
            description: Some("Limit data flow rate — pass one item every N milliseconds"),
            version: "0.1.0",
            name_zh: Some("速率限制"),
            name_en: Some("Throttle"),
            description_zh: Some("限制数据流速率，每隔指定毫秒才通过一个数据项"),
            description_en: Some("Throttle data flow — only pass one item every N milliseconds"),
        }
    }

    async fn open(&self, _node_id: NodeId, _config: PluginConfig) -> PluginResult<()> {
        Ok(())
    }

    async fn close(&self, _node_id: NodeId) -> PluginResult<()> {
        Ok(())
    }

    async fn process(&self, _node_id: NodeId, data: PipelineData) -> PluginResult<Vec<PipelineData>> {
        let min_interval = Duration::from_millis(self.interval_ms);
        let mut last = self.last_pass.write().await;
        let elapsed = last.elapsed();

        if elapsed >= min_interval {
            *last = Instant::now();
            Ok(vec![data])
        } else {
            let remaining = min_interval - elapsed;
            drop(last);
            sleep(remaining).await;
            *self.last_pass.write().await = Instant::now();
            Ok(vec![data])
        }
    }

    async fn reset(&self, _node_id: NodeId) -> PluginResult<()> {
        *self.last_pass.write().await = Instant::now() - Duration::from_secs(100);
        Ok(())
    }
}

pub fn create_throttle_operator(config: &PluginConfig) -> Box<dyn Operable> {
    Box::new(ThrottleOperator::new(config))
}
