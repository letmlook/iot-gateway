//! Transform operator: transform/rename/map fields using rhai expressions.

use async_trait::async_trait;
use std::collections::HashMap;
use gateway_sdk::{
    Operable, PipelineData, PluginConfig, PluginMeta, PluginResult,
    DataValue, NodeId,
};
use rhai::{Engine, Dynamic};
use serde::Deserialize;

/// A single field transformation rule.
#[derive(Debug, Clone, Deserialize)]
pub struct TransformRule {
    pub output_key: String,
    pub expression: String,
}

/// Transform operator: applies rhai expressions to produce new/transformed fields.
pub struct TransformOperator {
    engine: Engine,
    rules: Vec<TransformRule>,
}

impl TransformOperator {
    pub fn new(rules_json: &str) -> Self {
        let mut engine = Engine::new();
        engine.register_fn("has_field", |map: &mut HashMap<String, DataValue>, key: &str| -> bool {
            map.contains_key(key)
        });
        engine.register_fn("get_float", |map: &mut HashMap<String, DataValue>, key: &str| -> Option<f64> {
            map.get(key).and_then(|v| v.as_f64())
        });
        engine.register_fn("get_int", |map: &mut HashMap<String, DataValue>, key: &str| -> Option<i64> {
            map.get(key).and_then(|v| v.as_i64())
        });
        engine.register_fn("get_bool", |map: &mut HashMap<String, DataValue>, key: &str| -> Option<bool> {
            map.get(key).and_then(|v| v.as_bool())
        });
        engine.register_fn("get_str", |map: &mut HashMap<String, DataValue>, key: &str| -> Option<String> {
            map.get(key).and_then(|v| v.as_string())
        });
        
        let rules: Vec<TransformRule> = serde_json::from_str(rules_json).unwrap_or_default();
        Self { engine, rules }
    }
}

#[async_trait]
impl Operable for TransformOperator {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "transform",
            kind: gateway_sdk::PluginKind::Operator,
            description: Some("Transform fields using rhai expressions"),
            version: "0.1.0",
            name_zh: Some("转换器"),
            name_en: Some("Transform"),
            description_zh: Some("使用 rhai 表达式对字段进行转换、计算、新增或删除"),
            description_en: Some("Transform, compute, add or remove fields using rhai expressions"),
        }
    }
    
    async fn open(&self, _node_id: NodeId, _config: PluginConfig) -> PluginResult<()> {
        Ok(())
    }
    
    async fn close(&self, _node_id: NodeId) -> PluginResult<()> {
        Ok(())
    }
    
    async fn process(&self, _node_id: NodeId, mut data: PipelineData) -> PluginResult<Vec<PipelineData>> {
        let mut payload = data.payload.clone();
        
        for rule in &self.rules {
            let mut scope = rhai::Scope::new();
            scope.push("data", Dynamic::from(payload.clone()));
            
            match self.engine.eval_with_scope::<Dynamic>(&mut scope, &rule.expression) {
                Ok(val) => {
                    let dv = dynamic_to_datavalue(val);
                    if let Some(v) = dv {
                        payload.insert(rule.output_key.clone(), v);
                    }
                }
                Err(_) => {
                    // Expression error — skip this rule
                }
            }
        }
        
        data.payload = payload;
        Ok(vec![data])
    }
    
    async fn reset(&self, _node_id: NodeId) -> PluginResult<()> {
        Ok(())
    }
}

/// Convert rhai Dynamic value to DataValue.
fn dynamic_to_datavalue(d: Dynamic) -> Option<DataValue> {
    if d.is::<i64>() {
        Some(DataValue::Int64(d.cast()))
    } else if d.is::<f64>() {
        Some(DataValue::Float64(d.cast()))
    } else if d.is::<bool>() {
        Some(DataValue::Bool(d.cast()))
    } else if d.is::<String>() {
        Some(DataValue::String(d.cast()))
    } else {
        None
    }
}

/// Create a transform operator from config.
/// config expects: { "rules": "[{\"output_key\": \"temp_c\", \"expression\": \"data.temperature * 1.8 + 32\"}]" }
pub fn create_transform_operator(config: &PluginConfig) -> Box<dyn Operable> {
    let rules = config.get("rules")
        .and_then(|v| v.as_str())
        .unwrap_or("[]");
    Box::new(TransformOperator::new(rules))
}
