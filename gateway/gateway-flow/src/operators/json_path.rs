//! JSONPath operator: extract values from JSON using JSONPath expressions.

use std::str::FromStr;
use async_trait::async_trait;
use gateway_sdk::{Operable, PipelineData, PluginConfig, PluginMeta, PluginResult, DataValue, NodeId};
use jsonpath_rust::{JsonPathInst, find};

pub struct JsonPathOperator {
    path: JsonPathInst,
    output_field: String,
}

impl JsonPathOperator {
    pub fn new(config: &PluginConfig) -> Self {
        let path_str = config.get("path")
            .and_then(|v| v.as_str())
            .unwrap_or("$.*");
        let output_field = config.get("output_field")
            .and_then(|v| v.as_str())
            .unwrap_or("result")
            .to_string();
        let path = JsonPathInst::from_str(path_str).unwrap_or_else(|_| JsonPathInst::from_str("$.*").unwrap());
        Self { path, output_field }
    }
}

#[async_trait]
impl Operable for JsonPathOperator {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "json-path",
            kind: gateway_sdk::PluginKind::Operator,
            description: Some("Extract values from JSON using JSONPath"),
            version: "0.1.0",
            name_zh: Some("JSON路径提取"),
            name_en: Some("JSON Path"),
            description_zh: Some("使用JSONPath从JSON数据中提取字段值"),
            description_en: Some("Extract values from JSON data using JSONPath expressions"),
        }
    }

    async fn open(&self, _node_id: NodeId, _config: PluginConfig) -> PluginResult<()> {
        Ok(())
    }

    async fn close(&self, _node_id: NodeId) -> PluginResult<()> {
        Ok(())
    }

    async fn process(&self, _node_id: NodeId, mut data: PipelineData) -> PluginResult<Vec<PipelineData>> {
        // Look for a JSON string field (commonly "_raw_json" or "json")
        let json_str = data.payload.get("_raw_json")
            .and_then(|v| v.as_string())
            .or_else(|| data.payload.get("json").and_then(|v| v.as_string()));

        if let Some(json_str) = json_str {
            if let Ok(json_val) = serde_json::from_str::<serde_json::Value>(&json_str) {
                let results = find(&self.path, &json_val);
                if let Some(arr) = results.as_array() {
                    for (i, val) in arr.iter().enumerate() {
                        if let Some(extracted) = val.as_f64() {
                            let key = if arr.len() == 1 {
                                self.output_field.clone()
                            } else {
                                format!("{}_{}", self.output_field, i)
                            };
                            data.payload.insert(key, DataValue::Float64(extracted));
                        } else if let Some(s) = val.as_str() {
                            let key = if arr.len() == 1 {
                                self.output_field.clone()
                            } else {
                                format!("{}_{}", self.output_field, i)
                            };
                            data.payload.insert(key, DataValue::String(s.to_string()));
                        }
                    }
                }
            }
        }
        Ok(vec![data])
    }

    async fn reset(&self, _node_id: NodeId) -> PluginResult<()> {
        Ok(())
    }
}

pub fn create_json_path_operator(config: &PluginConfig) -> Box<dyn Operable> {
    Box::new(JsonPathOperator::new(config))
}
