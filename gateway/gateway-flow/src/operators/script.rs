//! Script operator: execute scripting for custom data processing logic using rhai.

use async_trait::async_trait;
use gateway_sdk::{Operable, PipelineData, PluginConfig, PluginMeta, PluginResult, DataValue, NodeId};
use rhai::Engine;

pub struct ScriptOperator {
    engine: Engine,
    script: String,
}

impl ScriptOperator {
    pub fn new(config: &PluginConfig) -> Self {
        let mut engine = Engine::new();
        engine.register_fn("log", |v: &str| tracing::info!("[script] {}", v));

        let script = config.get("script")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        Self { engine, script }
    }
}

#[async_trait]
impl Operable for ScriptOperator {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "script",
            kind: gateway_sdk::PluginKind::Operator,
            description: Some("Execute JavaScript-like scripting (rhai engine)"),
            version: "0.1.0",
            name_zh: Some("脚本执行"),
            name_en: Some("Script"),
            description_zh: Some("使用脚本语言执行自定义数据处理逻辑"),
            description_en: Some("Execute custom data processing logic using a scripting language"),
        }
    }

    async fn open(&self, _node_id: NodeId, _config: PluginConfig) -> PluginResult<()> {
        Ok(())
    }

    async fn close(&self, _node_id: NodeId) -> PluginResult<()> {
        Ok(())
    }

    async fn process(&self, _node_id: NodeId, mut data: PipelineData) -> PluginResult<Vec<PipelineData>> {
        if self.script.is_empty() { return Ok(vec![data]); }

        let payload_str = serde_json::to_string(&data.payload).unwrap_or_default();

        let script_with_output = format!(
            "let data = {}; {}; data;",
            payload_str,
            self.script
        );

        if let Ok(result) = self.engine.eval::<String>(&script_with_output) {
            if let Ok(new_payload) = serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(&result) {
                data.payload.clear();
                for (k, v) in new_payload {
                    data.payload.insert(k, DataValue::from_json(v));
                }
            }
        }

        Ok(vec![data])
    }

    async fn reset(&self, _node_id: NodeId) -> PluginResult<()> {
        Ok(())
    }
}

pub fn create_script_operator(config: &PluginConfig) -> Box<dyn Operable> {
    Box::new(ScriptOperator::new(config))
}
