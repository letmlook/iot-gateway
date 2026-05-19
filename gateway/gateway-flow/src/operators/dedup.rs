//! Dedup operator: filter duplicate consecutive data — only pass when value changes.

use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use gateway_sdk::{Operable, PipelineData, PluginConfig, PluginMeta, PluginResult, NodeId};

pub struct DedupOperator {
    key: String,
    last_values: Arc<RwLock<HashMap<String, String>>>,
}

impl DedupOperator {
    pub fn new(config: &PluginConfig) -> Self {
        let key = config.get("key").and_then(|v| v.as_str()).unwrap_or("value").to_string();
        Self {
            key,
            last_values: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

#[async_trait]
impl Operable for DedupOperator {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "dedup",
            kind: gateway_sdk::PluginKind::Operator,
            description: Some("Filter duplicate consecutive data — only pass when value changes"),
            version: "0.1.0",
            name_zh: Some("去重过滤"),
            name_en: Some("Dedup"),
            description_zh: Some("过滤连续重复的数据，只在值发生变化时传递"),
            description_en: Some("Filter duplicate consecutive data — only pass when the key value changes"),
        }
    }

    async fn open(&self, _node_id: NodeId, _config: PluginConfig) -> PluginResult<()> {
        Ok(())
    }

    async fn close(&self, _node_id: NodeId) -> PluginResult<()> {
        Ok(())
    }

    async fn process(&self, _node_id: NodeId, data: PipelineData) -> PluginResult<Vec<PipelineData>> {
        let current = data.payload.get(&self.key)
            .map(|v| format!("{:?}", v))
            .unwrap_or_default();

        let mut last = self.last_values.write().await;
        let prev = last.get(&self.key).cloned();

        if prev.as_ref() != Some(&current) {
            last.insert(self.key.clone(), current);
            Ok(vec![data])
        } else {
            Ok(vec![])
        }
    }

    async fn reset(&self, _node_id: NodeId) -> PluginResult<()> {
        self.last_values.write().await.clear();
        Ok(())
    }
}

pub fn create_dedup_operator(config: &PluginConfig) -> Box<dyn Operable> {
    Box::new(DedupOperator::new(config))
}
