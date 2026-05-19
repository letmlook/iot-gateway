//! Formula operator: evaluate mathematical expressions on data fields.

use async_trait::async_trait;
use gateway_sdk::{
    Operable, PipelineData, PluginConfig, PluginMeta, PluginResult,
    DataValue, NodeId,
};
use rhai::{Engine, Dynamic};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FormulaExpr {
    pub tag: String,
    pub expr: String,
    pub target: String,
}

pub struct FormulaOperator {
    engine: Engine,
    expressions: Vec<FormulaExpr>,
}

impl FormulaOperator {
    pub fn new(config: &PluginConfig) -> Self {
        let mut engine = Engine::new();
        engine.register_fn("abs", |x: f64| x.abs());
        engine.register_fn("sin", |x: f64| x.sin());
        engine.register_fn("cos", |x: f64| x.cos());
        engine.register_fn("tan", |x: f64| x.tan());
        engine.register_fn("sqrt", |x: f64| x.sqrt());
        engine.register_fn("pow", |x: f64, y: f64| x.powf(y));
        engine.register_fn("min", |a: f64, b: f64| a.min(b));
        engine.register_fn("max", |a: f64, b: f64| a.max(b));
        engine.register_fn("if", |cond: bool, a: f64, b: f64| if cond { a } else { b });

        let expressions: Vec<FormulaExpr> = config
            .get("expressions")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| serde_json::from_value(v.clone()).ok())
                    .collect()
            })
            .unwrap_or_default();

        Self { engine, expressions }
    }
}

#[async_trait]
impl Operable for FormulaOperator {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "formula",
            kind: gateway_sdk::PluginKind::Operator,
            description: Some("Evaluate mathematical expressions on data fields"),
            version: "0.1.0",
            name_zh: Some("公式解析"),
            name_en: Some("Formula"),
            description_zh: Some("对数据字段进行数学公式求值"),
            description_en: Some("Evaluate mathematical expressions on data fields using formula syntax"),
        }
    }

    async fn open(&self, _node_id: NodeId, _config: PluginConfig) -> PluginResult<()> {
        Ok(())
    }

    async fn close(&self, _node_id: NodeId) -> PluginResult<()> {
        Ok(())
    }

    async fn process(&self, _node_id: NodeId, mut data: PipelineData) -> PluginResult<Vec<PipelineData>> {
        for expr in &self.expressions {
            if let Some(value) = data.payload.get(&expr.tag).and_then(|v| v.as_f64()) {
                let mut scope = rhai::Scope::new();
                scope.push("value", value);
                let json_str = serde_json::to_string(&data.payload).unwrap_or_default();
                scope.push("data", Dynamic::from(json_str));

                if let Ok(result) = self.engine.eval_with_scope::<f64>(&mut scope, &expr.expr) {
                    data.payload.insert(expr.target.clone(), DataValue::Float64(result));
                }
            }
        }
        Ok(vec![data])
    }

    async fn reset(&self, _node_id: NodeId) -> PluginResult<()> {
        Ok(())
    }
}

pub fn create_formula_operator(config: &PluginConfig) -> Box<dyn Operable> {
    Box::new(FormulaOperator::new(config))
}