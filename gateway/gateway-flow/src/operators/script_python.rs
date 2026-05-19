//! Script Python operator: execute Python-like scripting using rhai.
//! Note: This is a stub implementation using rhai with Python-like syntax.
//! Real Python execution can be added later with pyoxidizer.

use async_trait::async_trait;
use gateway_sdk::{Operable, PipelineData, PluginConfig, PluginMeta, PluginResult, DataValue, NodeId};
use rhai::{Engine, Dynamic};

pub struct ScriptPythonOperator {
    engine: Engine,
    script: String,
    input_fields: Vec<String>,
    output_fields: Vec<String>,
}

impl ScriptPythonOperator {
    pub fn new(config: &PluginConfig) -> Self {
        let mut engine = Engine::new();

        // Register common Python-like builtins
        engine.register_fn("len", |s: &str| s.len() as i64);
        engine.register_fn("abs", |x: i64| x.abs() as i64);
        engine.register_fn("abs", |x: f64| x.abs());
        engine.register_fn("round", |x: f64, d: i64| {
            let m = 10_f64.powi(d as i32);
            (x * m).round() / m
        });
        engine.register_fn("min", |a: i64, b: i64| a.min(b));
        engine.register_fn("max", |a: i64, b: i64| a.max(b));
        engine.register_fn("floor", |x: f64| x.floor() as i64);
        engine.register_fn("ceil", |x: f64| x.ceil() as i64);
        engine.register_fn("sqrt", |x: f64| x.sqrt());
        engine.register_fn("pow", |x: f64, y: f64| x.powf(y));
        engine.register_fn("log", |x: f64| x.ln());
        engine.register_fn("log10", |x: f64| x.log10());
        engine.register_fn("sin", |x: f64| x.sin());
        engine.register_fn("cos", |x: f64| x.cos());
        engine.register_fn("tan", |x: f64| x.tan());
        engine.register_fn("str", |x: i64| x.to_string());
        engine.register_fn("str", |x: f64| x.to_string());
        engine.register_fn("int", |x: f64| x as i64);
        engine.register_fn("float", |x: i64| x as f64);

        let script = config.get("script")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let input_fields: Vec<String> = config
            .get("input_fields")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter().filter_map(|item| item.as_str().map(|s| s.to_string())).collect()
            })
            .unwrap_or_default();

        let output_fields: Vec<String> = config
            .get("output_fields")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter().filter_map(|item| item.as_str().map(|s| s.to_string())).collect()
            })
            .unwrap_or_default();

        Self { engine, script, input_fields, output_fields }
    }
}

fn data_value_to_rhai(v: &DataValue) -> Dynamic {
    match v {
        DataValue::Bool(b) => Dynamic::from(*b),
        DataValue::Int64(i) => Dynamic::from(*i),
        DataValue::Float64(f) => Dynamic::from(*f),
        DataValue::String(s) => Dynamic::from(s.clone()),
        _ => Dynamic::from(0i64),
    }
}

fn rhai_to_data_value(v: Dynamic) -> DataValue {
    if v.is::<bool>() { DataValue::Bool(v.cast()) }
    else if v.is::<i64>() { DataValue::Int64(v.cast()) }
    else if v.is::<f64>() { DataValue::Float64(v.cast()) }
    else if v.is::<String>() { DataValue::String(v.cast()) }
    else { DataValue::Int64(0) }
}

#[async_trait]
impl Operable for ScriptPythonOperator {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "script-python",
            kind: gateway_sdk::PluginKind::Operator,
            description: Some("Execute Python-like scripting (rhai engine)"),
            version: "0.1.0",
            name_zh: Some("Python脚本"),
            name_en: Some("Python Script"),
            description_zh: Some("使用类Python语法执行脚本处理数据"),
            description_en: Some("Execute Python-like scripts for custom data processing using rhai"),
        }
    }

    async fn open(&self, _node_id: NodeId, _config: PluginConfig) -> PluginResult<()> {
        Ok(())
    }

    async fn close(&self, _node_id: NodeId) -> PluginResult<()> {
        Ok(())
    }

    async fn process(&self, _node_id: NodeId, mut data: PipelineData) -> PluginResult<Vec<PipelineData>> {
        if self.script.is_empty() {
            return Ok(vec![data]);
        }

        // Inject input fields as rhai variables
        let mut scope = rhai::Scope::new();
        for field in &self.input_fields {
            if let Some(value) = data.payload.get(field) {
                scope.push(field.as_str(), data_value_to_rhai(value));
            }
        }

        // Execute script - result is stored in `result` variable or last expression
        let script_with_result = format!("let result = {}; result", self.script);

        match self.engine.eval_with_scope::<Dynamic>(&mut scope, &script_with_result) {
            Ok(val) => {
                if !self.output_fields.is_empty() {
                    // Use first output field for result
                    if let Some(field) = self.output_fields.first() {
                        data.payload.insert(field.clone(), rhai_to_data_value(val));
                    }
                }
            }
            Err(_) => {
                // Script error - pass through unchanged
            }
        }

        Ok(vec![data])
    }

    async fn reset(&self, _node_id: NodeId) -> PluginResult<()> {
        Ok(())
    }
}

pub fn create_script_python_operator(config: &PluginConfig) -> Box<dyn Operable> {
    Box::new(ScriptPythonOperator::new(config))
}
