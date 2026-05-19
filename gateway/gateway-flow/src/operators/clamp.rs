//! Clamp operator: clamp values to a specified range.

use async_trait::async_trait;
use gateway_sdk::{Operable, PipelineData, PluginConfig, PluginMeta, PluginResult, DataValue, NodeId};

pub struct ClampOperator {
    tag: String,
    min_val: f64,
    max_val: f64,
}

impl ClampOperator {
    pub fn new(config: &PluginConfig) -> Self {
        let tag = config
            .get("tag")
            .and_then(|v| v.as_str())
            .unwrap_or("value")
            .to_string();
        let min_val = config.get("min").and_then(|v| v.as_f64()).unwrap_or(f64::MIN);
        let max_val = config.get("max").and_then(|v| v.as_f64()).unwrap_or(f64::MAX);
        Self { tag, min_val, max_val }
    }
}

#[async_trait]
impl Operable for ClampOperator {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "clamp",
            kind: gateway_sdk::PluginKind::Operator,
            description: Some("Clamp values to a specified range"),
            version: "0.1.0",
            name_zh: Some("值域限幅"),
            name_en: Some("Clamp"),
            description_zh: Some("将数值限制在指定范围内"),
            description_en: Some("Clamp numeric values to a minimum and maximum range"),
        }
    }

    async fn open(&self, _node_id: NodeId, _config: PluginConfig) -> PluginResult<()> {
        Ok(())
    }

    async fn close(&self, _node_id: NodeId) -> PluginResult<()> {
        Ok(())
    }

    async fn process(&self, _node_id: NodeId, mut data: PipelineData) -> PluginResult<Vec<PipelineData>> {
        if let Some(v) = data.payload.get_mut(&self.tag) {
            if let Some(f) = v.as_f64() {
                let clamped = f.max(self.min_val).min(self.max_val);
                *v = DataValue::Float64(clamped);
            }
        }
        Ok(vec![data])
    }

    async fn reset(&self, _node_id: NodeId) -> PluginResult<()> {
        Ok(())
    }
}

pub fn create_clamp_operator(config: &PluginConfig) -> Box<dyn Operable> {
    Box::new(ClampOperator::new(config))
}