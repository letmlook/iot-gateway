//! Filter operator: passes through data only when condition expression evaluates to true.

use async_trait::async_trait;
use std::collections::HashMap;
use gateway_sdk::{
    Operable, PipelineData, PluginConfig, PluginMeta, PluginResult,
    DataValue, NodeId,
};
use rhai::{Engine, Dynamic};

pub struct FilterOperator {
    engine: Engine,
    condition: String,
}

impl FilterOperator {
    pub fn new(condition: &str) -> Self {
        let mut engine = Engine::new();
        // Register common functions for conditions
        engine.register_fn("has_field", |map: &mut HashMap<String, DataValue>, key: &str| -> bool {
            map.contains_key(key)
        });
        engine.register_fn("get_float", |map: &mut HashMap<String, DataValue>, key: &str| -> Option<f64> {
            map.get(key).and_then(|v| v.as_f64())
        });
        engine.register_fn("get_bool", |map: &mut HashMap<String, DataValue>, key: &str| -> Option<bool> {
            map.get(key).and_then(|v| v.as_bool())
        });
        engine.register_fn("get_str", |map: &mut HashMap<String, DataValue>, key: &str| -> Option<String> {
            map.get(key).and_then(|v| v.as_string())
        });
        
        Self {
            engine,
            condition: condition.to_string(),
        }
    }
    
    fn evaluate(&self, data: &PipelineData) -> bool {
        let map = data.payload.clone();
        let result = self.engine.eval_with_scope::<Dynamic>(
            &mut rhai::Scope::new().push("data", Dynamic::from(map.clone())),
            &self.condition,
        );
        match result {
            Ok(val) => val.as_bool().unwrap_or(false),
            Err(_) => false,
        }
    }
}

#[async_trait]
impl Operable for FilterOperator {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "filter",
            kind: gateway_sdk::PluginKind::Operator,
            description: Some("Pass through data only when condition is true"),
            version: "0.1.0",
            name_zh: Some("过滤器"),
            name_en: Some("Filter"),
            description_zh: Some("根据条件表达式过滤数据，只保留满足条件的数据"),
            description_en: Some("Pass through data only when condition expression evaluates to true"),
        }
    }
    
    async fn open(&self, _node_id: NodeId, _config: PluginConfig) -> PluginResult<()> {
        Ok(())
    }
    
    async fn close(&self, _node_id: NodeId) -> PluginResult<()> {
        Ok(())
    }
    
    async fn process(&self, _node_id: NodeId, data: PipelineData) -> PluginResult<Vec<PipelineData>> {
        if self.evaluate(&data) {
            Ok(vec![data])
        } else {
            Ok(vec![])
        }
    }
    
    async fn reset(&self, _node_id: NodeId) -> PluginResult<()> {
        Ok(())
    }
}

/// Create a filter operator from config.
/// config expects: { "condition": "data.temperature > 30" }
pub fn create_filter_operator(config: &PluginConfig) -> Box<dyn Operable> {
    let condition = config.get("condition")
        .and_then(|v| v.as_str())
        .unwrap_or("true");
    Box::new(FilterOperator::new(condition))
}
