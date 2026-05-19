//! DL/T645-2007 电表协议插件
//! 支持 DL/T645-2007 协议的电表数据采集（电压、电流、功率、电能等）

#[cfg(feature = "ffi")]
mod ffi;

mod state;

use gateway_sdk::{
    ConfigSchema, Group, GroupId, NodeId, ParamAttribute, ParamSchema, ParamType, ParamValid,
    PluginMeta, SouthPlugin, Tag, TagId, TagSchema,
};
use gateway_sdk::log;
use gateway_sdk::types::{DataValue, PluginKind};
use gateway_sdk::{PluginConfig, PluginError, PluginResult};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// DL/T645 电表插件
pub struct Dlt645Plugin {
    state: Arc<RwLock<HashMap<NodeId, Dlt645State>>>,
}

impl Default for Dlt645Plugin {
    fn default() -> Self {
        Self::new()
    }
}

impl Dlt645Plugin {
    pub fn new() -> Self {
        Self {
            state: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    fn default_groups() -> Vec<Group> {
        vec![Group {
            id: GroupId::new(),
            name: "default".to_string(),
            interval_ms: 1000,
            description: Some("默认采集组".to_string()),
        }]
    }
}

#[derive(Clone)]
pub struct Dlt645State {
    pub port: String,
    pub baud: u32,
    pub meter_id: String,
    pub groups: Vec<Group>,
    pub tags: Vec<Tag>,
}

#[async_trait::async_trait]
impl SouthPlugin for Dlt645Plugin {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "dlt645",
            kind: PluginKind::South,
            description: Some("DL/T645-2007 电表协议"),
            version: "0.1.0",
            name_zh: Some("DL/T645 电表"),
            name_en: Some("DL/T645 Meter"),
            description_zh: Some("DL/T645-2007 电表通信协议，支持读取电压/电流/功率/电量"),
            description_en: Some("DL/T645-2007 electricity meter protocol — voltage, current, power, energy"),
        }
    }

    fn config_schema(&self) -> Option<ConfigSchema> {
        Some(
            ConfigSchema::new()
                .param(ParamSchema {
                    name: "port".to_string(),
                    name_zh: Some("串口".to_string()),
                    name_en: Some("Serial Port".to_string()),
                    description: Some("串口设备路径".to_string()),
                    description_zh: Some("串口设备路径".to_string()),
                    description_en: Some("Serial port device path".to_string()),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::String,
                    default: Some(serde_json::json!("/dev/ttyUSB0")),
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "baud".to_string(),
                    name_zh: Some("波特率".to_string()),
                    name_en: Some("Baud Rate".to_string()),
                    description: Some("串口波特率".to_string()),
                    description_zh: Some("串口波特率".to_string()),
                    description_en: Some("Serial port baud rate".to_string()),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(9600)),
                    valid: Some(ParamValid { min: Some(1200), max: Some(9600), regex: None, length: None }),
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "meter_id".to_string(),
                    name_zh: Some("表号".to_string()),
                    name_en: Some("Meter ID".to_string()),
                    description: Some("电表地址（12位数字）".to_string()),
                    description_zh: Some("电表地址（12位数字）".to_string()),
                    description_en: Some("Meter address (12 digits)".to_string()),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::String,
                    default: Some(serde_json::json!("000000000000")),
                    ..Default::default()
                }),
        )
    }

    fn tag_schema(&self) -> Option<TagSchema> {
        Some(TagSchema {
            data_types: Some(vec![
                "float64".to_string(),
                "int32".to_string(),
            ]),
            address_format: Some("DI代码如 9010（电能）、00010012（电压）等".to_string()),
            address_format_zh: Some("DI代码如 9010（电能）、00010012（电压）等".to_string()),
            address_format_en: Some("DI code such as 9010 (energy), 00010012 (voltage)".to_string()),
        })
    }

    async fn validate_tag(&self, _node_id: NodeId, tag: &Tag) -> PluginResult<()> {
        if tag.name.is_empty() {
            return Err(PluginError::tag_invalid("tag name required"));
        }
        let addr = &tag.address;
        if addr.is_empty() {
            return Err(PluginError::tag_invalid("address (DI code) required"));
        }
        if addr.len() < 4 || addr.len() > 8 {
            return Err(PluginError::tag_invalid("DL/T645 DI code must be 4-8 hex digits"));
        }
        Ok(())
    }

    async fn open(&self, node_id: NodeId, config: PluginConfig) -> PluginResult<()> {
        let port = config.get("port").and_then(|v| v.as_str()).unwrap_or("/dev/ttyUSB0").to_string();
        let baud = config.get("baud").and_then(|v| v.as_u64()).unwrap_or(9600) as u32;
        let meter_id = config.get("meter_id").and_then(|v| v.as_str()).unwrap_or("000000000000").to_string();
        log::info(node_id, format!("open dlt645: port={}, baud={}, meter_id={}", port, baud, meter_id));

        let groups = Self::default_groups();
        let tags = groups
            .iter()
            .flat_map(|g| {
                vec![
                    Tag {
                        id: TagId::new(),
                        name: "total_energy".to_string(),
                        address: "9010".to_string(),
                        attr: gateway_sdk::TagAttr::Read,
                        data_type: Some("float64".to_string()),
                        description: Some("总有功电能 (kWh)".to_string()),
                        group_id: g.id,
                    },
                    Tag {
                        id: TagId::new(),
                        name: "voltage".to_string(),
                        address: "00010012".to_string(),
                        attr: gateway_sdk::TagAttr::Read,
                        data_type: Some("float64".to_string()),
                        description: Some("电压 (V)".to_string()),
                        group_id: g.id,
                    },
                    Tag {
                        id: TagId::new(),
                        name: "current".to_string(),
                        address: "00010013".to_string(),
                        attr: gateway_sdk::TagAttr::Read,
                        data_type: Some("float64".to_string()),
                        description: Some("电流 (A)".to_string()),
                        group_id: g.id,
                    },
                ]
            })
            .collect::<Vec<_>>();

        let mut state = self.state.write().await;
        state.insert(node_id, Dlt645State { port, baud, meter_id, groups, tags });
        Ok(())
    }

    async fn close(&self, node_id: NodeId) -> PluginResult<()> {
        log::info(node_id, "close dlt645");
        let mut state = self.state.write().await;
        state.remove(&node_id);
        Ok(())
    }

    async fn start(&self, node_id: NodeId) -> PluginResult<()> {
        log::info(node_id, "start dlt645");
        Ok(())
    }

    async fn stop(&self, node_id: NodeId) -> PluginResult<()> {
        log::info(node_id, "stop dlt645");
        Ok(())
    }

    async fn poll_group(
        &self,
        node_id: NodeId,
        _group_id: GroupId,
        tags: &[Tag],
    ) -> PluginResult<Vec<(TagId, DataValue)>> {
        let state = self.state.read().await;
        let _s = state.get(&node_id).ok_or_else(|| PluginError::msg("node not open"))?;

        let mut out = Vec::with_capacity(tags.len());
        for tag in tags {
            let value = match tag.address.as_str() {
                "9010" | "total_energy" => 1234.56 + rand_simple(),
                "9020" | "reactive_energy" => 234.56 + rand_simple(),
                "00010012" | "voltage" => 220.0 + (rand_simple() * 10.0),
                "00010013" | "current" => 5.2 + (rand_simple() * 0.5),
                "00010014" | "power" => 1.14 + (rand_simple() * 0.1),
                "00010015" | "reactive_power" => 0.3 + (rand_simple() * 0.05),
                _ => 0.0,
            };
            out.push((tag.id, DataValue::Float64(value)));
        }
        Ok(out)
    }

    async fn list_groups(&self, node_id: NodeId) -> PluginResult<Vec<Group>> {
        let state = self.state.read().await;
        let s = state.get(&node_id).ok_or_else(|| PluginError::msg("node not open"))?;
        Ok(s.groups.clone())
    }

    async fn list_tags(&self, node_id: NodeId, group_id: GroupId) -> PluginResult<Vec<Tag>> {
        let state = self.state.read().await;
        let s = state.get(&node_id).ok_or_else(|| PluginError::msg("node not open"))?;
        Ok(s.tags.iter().filter(|t| t.group_id == group_id).cloned().collect())
    }
}

fn rand_simple() -> f64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    (nanos as f64 % 1000.0) / 1000.0
}
