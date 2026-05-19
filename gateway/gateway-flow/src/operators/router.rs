//! Router operator: routes data to different output branches based on condition matching.

use async_trait::async_trait;
use std::collections::HashMap;
use gateway_sdk::{
    Operable, PipelineData, PluginConfig, PluginMeta, PluginResult,
    DataValue, NodeId,
};
use rhai::Engine;

/// A single routing rule: if condition is true, output through the named port.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct RouteRule {
    pub port: String,       // output port name for this route
    pub condition: String,  // rhai condition expression
    pub passthrough: bool, // if true, also pass data to subsequent rules
}

pub struct RouterOperator {
    engine: Engine,
    rules: Vec<RouteRule>,
    default_port: String,
}

impl RouterOperator {
    pub fn new(rules_json: &str, default_port: &str) -> Self {
        let mut engine = Engine::new();
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
        
        let rules: Vec<RouteRule> = serde_json::from_str(rules_json).unwrap_or_default();
        Self {
            engine,
            rules,
            default_port: default_port.to_string(),
        }
    }
}

#[async_trait]
impl Operable for RouterOperator {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "router",
            kind: gateway_sdk::PluginKind::Operator,
            description: Some("Route data to different outputs based on conditions"),
            version: "0.1.0",
            name_zh: Some("路由器"),
            name_en: Some("Router"),
            description_zh: Some("根据条件将数据路由到不同的输出端口"),
            description_en: Some("Route data to different output ports based on condition matching"),
        }
    }
    
    async fn open(&self, _node_id: NodeId, _config: PluginConfig) -> PluginResult<()> {
        Ok(())
    }
    
    async fn close(&self, _node_id: NodeId) -> PluginResult<()> {
        Ok(())
    }
    
    async fn process(&self, _node_id: NodeId, data: PipelineData) -> PluginResult<Vec<PipelineData>> {
        let mut results = Vec::new();
        let mut routed = false;
        let payload = data.payload.clone();
        
        for rule in &self.rules {
            let mut scope = rhai::Scope::new();
            scope.push("data", rhai::Dynamic::from(payload.clone()));
            
            match self.engine.eval_with_scope::<bool>(&mut scope, &rule.condition) {
                Ok(true) => {
                    let mut out = data.clone();
                    out.metadata.insert("router.port".into(), rule.port.clone());
                    results.push(out);
                    routed = true;
                    if !rule.passthrough {
                        break;
                    }
                }
                _ => {}
            }
        }
        
        // If no rule matched, output to default port
        if !routed {
            let mut out = data;
            out.metadata.insert("router.port".into(), self.default_port.clone());
            results.push(out);
        }
        
        Ok(results)
    }
    
    async fn reset(&self, _node_id: NodeId) -> PluginResult<()> {
        Ok(())
    }
}

/// Create a router operator from config.
/// config expects: { "rules": [{"port": "high", "condition": "data.temp > 80", "passthrough": false}], "default_port": "normal" }
pub fn create_router_operator(config: &PluginConfig) -> Box<dyn Operable> {
    let rules = config.get("rules")
        .and_then(|v| v.as_str())
        .unwrap_or("[]");
    let default_port = config.get("default_port")
        .and_then(|v| v.as_str())
        .unwrap_or("default");
    Box::new(RouterOperator::new(rules, default_port))
}