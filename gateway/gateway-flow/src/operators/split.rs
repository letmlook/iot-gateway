//! Split operator: split string by delimiter or array field into individual items.

use async_trait::async_trait;
use gateway_sdk::{Operable, PipelineData, PluginConfig, PluginMeta, PluginResult, DataValue, NodeId};

pub struct SplitOperator {
    field: String,
    separator: String,
}

impl SplitOperator {
    pub fn new(config: &PluginConfig) -> Self {
        let field = config.get("field").and_then(|v| v.as_str()).unwrap_or("value").to_string();
        let separator = config.get("separator").and_then(|v| v.as_str()).unwrap_or(",").to_string();
        Self { field, separator }
    }
}

#[async_trait]
impl Operable for SplitOperator {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "split",
            kind: gateway_sdk::PluginKind::Operator,
            description: Some("Split string by delimiter or array field into individual items"),
            version: "0.1.0",
            name_zh: Some("数据拆分"),
            name_en: Some("Split"),
            description_zh: Some("按分隔符拆分字符串或数组字段为单独的数据项"),
            description_en: Some("Split a string by delimiter or array field into individual output items"),
        }
    }

    async fn open(&self, _node_id: NodeId, _config: PluginConfig) -> PluginResult<()> {
        Ok(())
    }

    async fn close(&self, _node_id: NodeId) -> PluginResult<()> {
        Ok(())
    }

    async fn process(&self, _node_id: NodeId, data: PipelineData) -> PluginResult<Vec<PipelineData>> {
        if let Some(value) = data.payload.get(&self.field) {
            if let Some(s) = value.as_string() {
                let parts: Vec<&str> = s.split(&self.separator).collect();
                let mut results = Vec::new();
                for (i, part) in parts.iter().enumerate() {
                    let mut item = PipelineData::new(_node_id);
                    item.payload.insert(format!("{}_{}", self.field, i), DataValue::String(part.trim().to_string()));
                    item.payload.insert(self.field.clone(), DataValue::String(part.trim().to_string()));
                    results.push(item);
                }
                if results.is_empty() { Ok(vec![data]) } else { Ok(results) }
            } else { Ok(vec![data]) }
        } else { Ok(vec![data]) }
    }

    async fn reset(&self, _node_id: NodeId) -> PluginResult<()> {
        Ok(())
    }
}

pub fn create_split_operator(config: &PluginConfig) -> Box<dyn Operable> {
    Box::new(SplitOperator::new(config))
}
