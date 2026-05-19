//! Range operator: linear transformation - map values from one range to another.

use async_trait::async_trait;
use gateway_sdk::{Operable, PipelineData, PluginConfig, PluginMeta, PluginResult, DataValue, NodeId};

pub struct RangeOperator {
    tag: String,
    in_min: f64,
    in_max: f64,
    out_min: f64,
    out_max: f64,
}

impl RangeOperator {
    pub fn new(config: &PluginConfig) -> Self {
        let tag = config
            .get("tag")
            .and_then(|v| v.as_str())
            .unwrap_or("value")
            .to_string();
        let in_min = config.get("in_min").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let in_max = config.get("in_max").and_then(|v| v.as_f64()).unwrap_or(1.0);
        let out_min = config.get("out_min").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let out_max = config.get("out_max").and_then(|v| v.as_f64()).unwrap_or(1.0);
        Self { tag, in_min, in_max, out_min, out_max }
    }

    fn map(&self, v: f64) -> f64 {
        if self.in_max == self.in_min {
            return self.out_min;
        }
        let normalized = (v - self.in_min) / (self.in_max - self.in_min);
        self.out_min + normalized * (self.out_max - self.out_min)
    }
}

#[async_trait]
impl Operable for RangeOperator {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "range",
            kind: gateway_sdk::PluginKind::Operator,
            description: Some("Map values from one range to another (linear transformation)"),
            version: "0.1.0",
            name_zh: Some("线性变换"),
            name_en: Some("Range"),
            description_zh: Some("将数值从一个范围线性映射到另一个范围"),
            description_en: Some("Linear transformation: map a value from an input range to an output range"),
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
                *v = DataValue::Float64(self.map(f));
            }
        }
        Ok(vec![data])
    }

    async fn reset(&self, _node_id: NodeId) -> PluginResult<()> {
        Ok(())
    }
}

pub fn create_range_operator(config: &PluginConfig) -> Box<dyn Operable> {
    Box::new(RangeOperator::new(config))
}