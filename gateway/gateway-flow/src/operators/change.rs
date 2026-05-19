//! Change operator: replace, rename, or delete fields.

use async_trait::async_trait;
use gateway_sdk::{Operable, PipelineData, PluginConfig, PluginMeta, PluginResult, DataValue, NodeId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ChangeOp {
    Replace { field: String, old: String, new: String },
    Rename { field: String, new_name: String },
    Delete { field: String },
}

pub struct ChangeOperator {
    operations: Vec<ChangeOp>,
}

impl ChangeOperator {
    pub fn new(config: &PluginConfig) -> Self {
        let operations: Vec<ChangeOp> = config
            .get("operations")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| serde_json::from_value(v.clone()).ok())
                    .collect()
            })
            .unwrap_or_default();
        Self { operations }
    }
}

#[async_trait]
impl Operable for ChangeOperator {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "change",
            kind: gateway_sdk::PluginKind::Operator,
            description: Some("Replace, rename, or delete fields"),
            version: "0.1.0",
            name_zh: Some("字段变更"),
            name_en: Some("Change"),
            description_zh: Some("对字段进行替换/重命名/删除操作"),
            description_en: Some("Replace, rename, or delete fields in data payload"),
        }
    }

    async fn open(&self, _node_id: NodeId, _config: PluginConfig) -> PluginResult<()> {
        Ok(())
    }

    async fn close(&self, _node_id: NodeId) -> PluginResult<()> {
        Ok(())
    }

    async fn process(&self, _node_id: NodeId, mut data: PipelineData) -> PluginResult<Vec<PipelineData>> {
        for op in &self.operations {
            match op {
                ChangeOp::Replace { field, old, new } => {
                    if let Some(v) = data.payload.get_mut(field) {
                        if let Some(s) = v.as_string() {
                            if s == *old {
                                *v = DataValue::String(new.clone());
                            }
                        }
                    }
                }
                ChangeOp::Rename { field, new_name } => {
                    if let Some(v) = data.payload.remove(field) {
                        data.payload.insert(new_name.clone(), v);
                    }
                }
                ChangeOp::Delete { field } => {
                    data.payload.remove(field);
                }
            }
        }
        Ok(vec![data])
    }

    async fn reset(&self, _node_id: NodeId) -> PluginResult<()> {
        Ok(())
    }
}

pub fn create_change_operator(config: &PluginConfig) -> Box<dyn Operable> {
    Box::new(ChangeOperator::new(config))
}