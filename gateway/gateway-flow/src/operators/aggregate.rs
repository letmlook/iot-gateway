//! Aggregate operator: collects data into time-based windows and emits aggregate results.

use async_trait::async_trait;
use gateway_sdk::{
    DataValue, NodeId, Operable, PipelineData, PluginConfig, PluginMeta, PluginResult,
};
use serde::Deserialize;
use std::collections::VecDeque;
use std::sync::RwLock;
use std::time::Instant;

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
            _ => Err(serde::de::Error::custom(format!(
                "Unknown aggregation function: {}",
                s
            ))),
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
    count_window_size: usize,
    specs: Vec<AggSpec>,
    buffer: RwLock<VecDeque<(PipelineData, Instant)>>,
}

impl AggregateOperator {
    pub fn new(window_size_secs: u64, specs_json: &str) -> Self {
        Self::new_count_window(
            window_size_secs,
            specs_json,
            window_size_secs.max(1) as usize,
        )
    }

    pub fn new_count_window(
        window_size_secs: u64,
        specs_json: &str,
        count_window_size: usize,
    ) -> Self {
        let specs: Vec<AggSpec> = serde_json::from_str(specs_json).unwrap_or_default();
        Self {
            window_size_secs,
            count_window_size: count_window_size.max(1),
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

    async fn process(
        &self,
        node_id: NodeId,
        data: PipelineData,
    ) -> PluginResult<Vec<PipelineData>> {
        let mut buffer = self.buffer.write().unwrap();
        buffer.push_back((data, Instant::now()));

        if buffer.len() < self.count_window_size {
            return Ok(Vec::new());
        }

        let items: Vec<PipelineData> = buffer.drain(..).map(|(item, _)| item).collect();
        let mut output = PipelineData::new(node_id);
        output.ts = items.last().map(|item| item.ts).unwrap_or(output.ts);
        output
            .metadata
            .insert("aggregate.window_type".into(), "count".into());
        output.metadata.insert(
            "aggregate.window_size".into(),
            self.count_window_size.to_string(),
        );
        output.metadata.insert(
            "aggregate.window_size_secs".into(),
            self.window_size_secs.to_string(),
        );

        for spec in &self.specs {
            let values: Vec<f64> = items
                .iter()
                .filter_map(|item| item.payload.get(&spec.input_key).and_then(|v| v.as_f64()))
                .collect();
            if values.is_empty() && !matches!(spec.function, AggFn::Count) {
                continue;
            }
            let value = spec.function.apply(&values);
            output
                .payload
                .insert(spec.output_key.clone(), DataValue::Float64(value));
        }

        Ok(vec![output])
    }

    async fn reset(&self, _node_id: NodeId) -> PluginResult<()> {
        self.buffer.write().unwrap().clear();
        Ok(())
    }
}

/// Create an aggregate operator from config.
/// config expects: { "window_size_secs": 60, "specs": [{"output_key": "avg_temp", "input_key": "temperature", "function": "avg"}] }
pub fn create_aggregate_operator(config: &PluginConfig) -> Box<dyn Operable> {
    let window_size = config
        .get("window_size_secs")
        .and_then(|v| v.as_u64())
        .unwrap_or(60);
    let count_window_size = config
        .get("count_window_size")
        .or_else(|| config.get("window_size"))
        .or_else(|| config.get("max_size"))
        .and_then(|v| v.as_u64())
        .or_else(|| {
            config
                .get("window")
                .and_then(|v| v.get("size"))
                .and_then(|v| v.as_u64())
        })
        .unwrap_or(window_size.max(1)) as usize;
    let specs_owned;
    let specs = match config.get("specs") {
        Some(v) if v.is_array() => {
            specs_owned = serde_json::to_string(v).unwrap_or_else(|_| "[]".to_string());
            specs_owned.as_str()
        }
        Some(v) => v.as_str().unwrap_or("[]"),
        None => "[]",
    };
    Box::new(AggregateOperator::new_count_window(
        window_size,
        specs,
        count_window_size,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(value: f64) -> PipelineData {
        let mut data = PipelineData::new(NodeId::new());
        data.payload
            .insert("temperature".to_string(), DataValue::Float64(value));
        data
    }

    #[tokio::test]
    async fn aggregate_count_window_outputs_average_min_max_sum_count() {
        let specs = r#"[
            {"input_key":"temperature","output_key":"temperature_avg","function":"avg"},
            {"input_key":"temperature","output_key":"temperature_min","function":"min"},
            {"input_key":"temperature","output_key":"temperature_max","function":"max"},
            {"input_key":"temperature","output_key":"temperature_sum","function":"sum"},
            {"input_key":"temperature","output_key":"temperature_count","function":"count"}
        ]"#;
        let operator = AggregateOperator::new_count_window(60, specs, 3);
        let node_id = NodeId::new();

        assert!(operator
            .process(node_id, sample(10.0))
            .await
            .unwrap()
            .is_empty());
        assert!(operator
            .process(node_id, sample(20.0))
            .await
            .unwrap()
            .is_empty());

        let output = operator.process(node_id, sample(30.0)).await.unwrap();
        assert_eq!(output.len(), 1);
        let payload = &output[0].payload;
        assert_eq!(
            payload.get("temperature_avg"),
            Some(&DataValue::Float64(20.0))
        );
        assert_eq!(
            payload.get("temperature_min"),
            Some(&DataValue::Float64(10.0))
        );
        assert_eq!(
            payload.get("temperature_max"),
            Some(&DataValue::Float64(30.0))
        );
        assert_eq!(
            payload.get("temperature_sum"),
            Some(&DataValue::Float64(60.0))
        );
        assert_eq!(
            payload.get("temperature_count"),
            Some(&DataValue::Float64(3.0))
        );
    }
}
