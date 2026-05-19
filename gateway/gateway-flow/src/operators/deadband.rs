//! Deadband operator: only pass data when value change exceeds a threshold.

use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use gateway_sdk::{Operable, PipelineData, PluginConfig, PluginMeta, PluginResult, NodeId};

pub struct DeadbandOperator {
    tag: String,
    deadband: f64,
    /// Last value per tag (in case config has multiple tags)
    last_values: Arc<RwLock<HashMap<String, f64>>>,
}

impl DeadbandOperator {
    pub fn new(config: &PluginConfig) -> Self {
        let tag = config.get("tag")
            .and_then(|v| v.as_str())
            .unwrap_or("value")
            .to_string();
        let deadband = config.get("deadband")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        Self {
            tag,
            deadband,
            last_values: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

#[async_trait]
impl Operable for DeadbandOperator {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "deadband",
            kind: gateway_sdk::PluginKind::Operator,
            description: Some("Filter data based on deadband threshold"),
            version: "0.1.0",
            name_zh: Some("死区过滤"),
            name_en: Some("Deadband"),
            description_zh: Some("当数据变化量超过死区阈值时才会传递数据"),
            description_en: Some("Only pass data when value change exceeds deadband threshold"),
        }
    }

    async fn open(&self, _node_id: NodeId, _config: PluginConfig) -> PluginResult<()> {
        Ok(())
    }

    async fn close(&self, _node_id: NodeId) -> PluginResult<()> {
        Ok(())
    }

    async fn process(&self, _node_id: NodeId, data: PipelineData) -> PluginResult<Vec<PipelineData>> {
        let current_opt = data.payload.get(&self.tag).and_then(|v| v.as_f64());

        let Some(current) = current_opt else {
            return Ok(vec![data]); // pass through if no numeric value found
        };

        let mut last_map = self.last_values.write().await;
        let last_opt = last_map.get(&self.tag).copied();

        match last_opt {
            None => {
                // First data — always pass through
                last_map.insert(self.tag.clone(), current);
                Ok(vec![data])
            }
            Some(last) => {
                let diff = (current - last).abs();
                if diff > self.deadband {
                    last_map.insert(self.tag.clone(), current);
                    Ok(vec![data])
                } else {
                    // In deadband — don't pass
                    Ok(vec![])
                }
            }
        }
    }

    async fn reset(&self, _node_id: NodeId) -> PluginResult<()> {
        self.last_values.write().await.clear();
        Ok(())
    }
}

pub fn create_deadband_operator(config: &PluginConfig) -> Box<dyn Operable> {
    Box::new(DeadbandOperator::new(config))
}
