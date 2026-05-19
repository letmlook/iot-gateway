//! Aggregate operator: collects data into time-based windows and emits aggregate results.

use async_trait::async_trait;
use std::sync::RwLock;
use std::collections::VecDeque;
use std::time::Instant;
use gateway_sdk::{
    Operable, PipelineData, PluginConfig, PluginMeta, PluginResult,
    NodeId,
};
use serde::Deserialize;

/// Aggregation function type.
#[derive(Debug, Clone, Copy)]
pub enum AggFn {
    Sum,
    Avg,
    Min,
    Max,
    Count,
}

impl AggFn {
    fn apply(&self, values: &[f64]) -> f64 {
        match self {
            AggFn::Sum => values.iter().sum(),
            AggFn::Avg => values.iter().sum::<f64>() / values.len() as f64,
            AggFn::Min => values.iter().cloned().fold(f64::INFINITY, f64::min),
            AggFn::Max => values.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
            AggFn::Count => values.len() as f64,
        }
    }
}

impl<'de> Deserialize<'de> for AggFn {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        match s.to_lowercase().as_str() {
            "sum" => Ok(AggFn::Sum),
            "avg" => Ok(AggFn::Avg),
            "min" => Ok(AggFn::Min),
            "max" => Ok(AggFn::Max),
            "count" => Ok(AggFn::Count),
            _ => Err(serde::de::Error::custom(format!("Unknown aggregation function: {}", s))),
        }
    }
}

/// A single aggregation specification.
#[derive(Debug, Clone, Deserialize)]
pub struct AggSpec {
    pub output_key: String,
    pub input_key: String,
    pub function: AggFn,
}

/// Aggregate operator: groups incoming data by time windows and computes aggregates.
pub struct AggregateOperator {
    window_size_secs: u64,
    specs: Vec<AggSpec>,
    buffer: RwLock<VecDeque<(PipelineData, Instant)>>,
}

impl AggregateOperator {
    pub fn new(window_size_secs: u64, specs_json: &str) -> Self {
        let specs: Vec<AggSpec> = serde_json::from_str(specs_json).unwrap_or_default();
        Self {
            window_size_secs,
            specs,
            buffer: RwLock::new(VecDeque::new()),
        }
    }
}

#[async_trait]
impl Operable for AggregateOperator {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "aggregate",
            kind: gateway_sdk::PluginKind::Operator,
            description: Some("Aggregate data over time windows"),
            version: "0.1.0",
            name_zh: Some("聚合器"),
            name_en: Some("Aggregate"),
            description_zh: Some("按时间窗口聚合数据，支持 Sum/Avg/Min/Max/Count"),
            description_en: Some("Aggregate data over time windows with Sum/Avg/Min/Max/Count"),
        }
    }
    
    async fn open(&self, _node_id: NodeId, _config: PluginConfig) -> PluginResult<()> {
        self.buffer.write().unwrap().clear();
        Ok(())
    }
    
    async fn close(&self, _node_id: NodeId) -> PluginResult<()> {
        Ok(())
    }
    
    async fn process(&self, _node_id: NodeId, data: PipelineData) -> PluginResult<Vec<PipelineData>> {
        // For Phase 1: simple tumbling window (emit aggregate when window expires)
        // The actual timing logic would be handled by the DAG executor in later phases.
        // For now, we return the data as-is (no actual aggregation in Phase 1).
        Ok(vec![data])
    }
    
    async fn reset(&self, _node_id: NodeId) -> PluginResult<()> {
        self.buffer.write().unwrap().clear();
        Ok(())
    }
}

/// Create an aggregate operator from config.
/// config expects: { "window_size_secs": 60, "specs": [{"output_key": "avg_temp", "input_key": "temperature", "function": "avg"}] }
pub fn create_aggregate_operator(config: &PluginConfig) -> Box<dyn Operable> {
    let window_size = config.get("window_size_secs")
        .and_then(|v| v.as_u64())
        .unwrap_or(60);
    let specs = config.get("specs")
        .and_then(|v| v.as_str())
        .unwrap_or("[]");
    Box::new(AggregateOperator::new(window_size, specs))
}