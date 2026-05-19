//! Convert operator: type conversion — int↔float↔string↔bool.

use async_trait::async_trait;
use gateway_sdk::{Operable, PipelineData, PluginConfig, PluginMeta, PluginResult, DataValue, NodeId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conversion {
    pub from: String,
    pub to: String,
    #[serde(rename = "type")]
    pub conv_type: String,
}

pub struct ConvertOperator {
    conversions: Vec<Conversion>,
}

impl ConvertOperator {
    pub fn new(config: &PluginConfig) -> Self {
        let conversions: Vec<Conversion> = config.get("conversions")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter().filter_map(|v| serde_json::from_value(v.clone()).ok()).collect()
            })
            .unwrap_or_default();
        Self { conversions }
    }
}

#[async_trait]
impl Operable for ConvertOperator {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "convert",
            kind: gateway_sdk::PluginKind::Operator,
            description: Some("Type conversion — int↔float↔string↔bool"),
            version: "0.1.0",
            name_zh: Some("类型转换"),
            name_en: Some("Convert"),
            description_zh: Some("数据类型转换：整数、浮点、字符串、布尔之间的转换"),
            description_en: Some("Convert data types — int↔float↔string↔bool"),
        }
    }

    async fn open(&self, _node_id: NodeId, _config: PluginConfig) -> PluginResult<()> {
        Ok(())
    }

    async fn close(&self, _node_id: NodeId) -> PluginResult<()> {
        Ok(())
    }

    async fn process(&self, _node_id: NodeId, mut data: PipelineData) -> PluginResult<Vec<PipelineData>> {
        for conv in &self.conversions {
            if let Some(from_val) = data.payload.get(&conv.from) {
                let converted = match conv.conv_type.as_str() {
                    "int_to_float" => from_val.as_f64().map(DataValue::Float64),
                    "float_to_int" => from_val.as_f64().map(|f| DataValue::Int64(f as i64)),
                    "int_to_string" => from_val.as_i64().map(|i| DataValue::String(i.to_string())),
                    "float_to_string" => from_val.as_f64().map(|f| DataValue::String(f.to_string())),
                    "string_to_int" => from_val.as_string().and_then(|s| s.parse::<i64>().ok()).map(DataValue::Int64),
                    "string_to_float" => from_val.as_string().and_then(|s| s.parse::<f64>().ok()).map(DataValue::Float64),
                    "hex_to_int" => from_val.as_string().and_then(|s| u64::from_str_radix(s.trim_start_matches("0x"), 16).ok()).map(|i| DataValue::Int64(i as i64)),
                    "bool_to_int" => from_val.as_bool().map(|b| DataValue::Int64(if b { 1 } else { 0 })),
                    _ => None,
                };

                if let Some(new_val) = converted {
                    data.payload.remove(&conv.from);
                    data.payload.insert(conv.to.clone(), new_val);
                }
            }
        }
        Ok(vec![data])
    }

    async fn reset(&self, _node_id: NodeId) -> PluginResult<()> {
        Ok(())
    }
}

pub fn create_convert_operator(config: &PluginConfig) -> Box<dyn Operable> {
    Box::new(ConvertOperator::new(config))
}
