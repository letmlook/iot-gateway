//! Round operator: round numeric values to specified precision.

use async_trait::async_trait;
use gateway_sdk::{Operable, PipelineData, PluginConfig, PluginMeta, PluginResult, DataValue, NodeId};

pub struct RoundOperator {
    tag: String,
    precision: i32,
}

impl RoundOperator {
    pub fn new(config: &PluginConfig) -> Self {
        let tag = config
            .get("tag")
            .and_then(|v| v.as_str())
            .unwrap_or("value")
            .to_string();
        let precision = config.get("precision").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
        Self { tag, precision }
    }
}

fn round_to_precision(v: f64, p: i32) -> f64 {
    let mult = 10_f64.powi(p);
    (v * mult).round() / mult
}

#[async_trait]
impl Operable for RoundOperator {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "round",
            kind: gateway_sdk::PluginKind::Operator,
            description: Some("Round numeric values to specified precision"),
            version: "0.1.0",
            name_zh: Some("数值取整"),
            name_en: Some("Round"),
            description_zh: Some("将数值四舍五入到指定精度"),
            description_en: Some("Round numeric values to specified decimal precision"),
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
                *v = DataValue::Float64(round_to_precision(f, self.precision));
            }
        }
        Ok(vec![data])
    }

    async fn reset(&self, _node_id: NodeId) -> PluginResult<()> {
        Ok(())
    }
}

pub fn create_round_operator(config: &PluginConfig) -> Box<dyn Operable> {
    Box::new(RoundOperator::new(config))
}