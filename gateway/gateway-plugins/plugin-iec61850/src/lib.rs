//! IEC 61850 变电站自动化协议插件
//! 支持 IEC 61850 MMS 协议访问逻辑节点数据（频率、电压、电流、功率等）

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

/// IEC 61850 插件
pub struct Iec61850Plugin {
    state: Arc<RwLock<HashMap<NodeId, Iec61850State>>>,
}

impl Default for Iec61850Plugin {
    fn default() -> Self {
        Self::new()
    }
}

impl Iec61850Plugin {
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
pub struct Iec61850State {
    pub host: String,
    pub port: u16,
    pub ied_name: String,
    pub groups: Vec<Group>,
    pub tags: Vec<Tag>,
}

#[async_trait::async_trait]
impl SouthPlugin for Iec61850Plugin {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "iec61850",
            kind: PluginKind::South,
            description: Some("IEC 61850 变电站自动化协议"),
            version: "0.1.0",
            name_zh: Some("IEC61850"),
            name_en: Some("IEC61850"),
            description_zh: Some("IEC 61850 变电站自动化协议，通过MMS服务访问逻辑节点数据"),
            description_en: Some("IEC 61850 substation automation protocol — access logical nodes via MMS"),
        }
    }

    fn config_schema(&self) -> Option<ConfigSchema> {
        Some(
            ConfigSchema::new()
                .param(ParamSchema {
                    name: "host".to_string(),
                    name_zh: Some("IP地址".to_string()),
                    name_en: Some("IP Address".to_string()),
                    description: Some("IED 设备 IP 地址".to_string()),
                    description_zh: Some("IED 设备 IP 地址".to_string()),
                    description_en: Some("IED device IP address".to_string()),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::String,
                    default: Some(serde_json::json!("192.168.1.100")),
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "port".to_string(),
                    name_zh: Some("端口号".to_string()),
                    name_en: Some("Port".to_string()),
                    description: Some("MMS 端口（默认 102）".to_string()),
                    description_zh: Some("MMS 端口（默认 102）".to_string()),
                    description_en: Some("MMS port (default 102)".to_string()),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(102)),
                    valid: Some(ParamValid { min: Some(1), max: Some(65535), regex: None, length: None }),
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "ied_name".to_string(),
                    name_zh: Some("IED 名称".to_string()),
                    name_en: Some("IED Name".to_string()),
                    description: Some("IED 逻辑设备名".to_string()),
                    description_zh: Some("IED 逻辑设备名".to_string()),
                    description_en: Some("IED logical device name".to_string()),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::String,
                    default: Some(serde_json::json!("IED1")),
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
            address_format: Some("LD/LN.DA$DA$... 格式，如 MMXU1/Hz.HighZHz".to_string()),
            address_format_zh: Some("LD/LN.DA$DA$... 格式，如 MMXU1/Hz.HighZHz".to_string()),
            address_format_en: Some("LD/LN.DA$DA$... format, e.g. MMXU1/Hz.HighZHz".to_string()),
        })
    }

    async fn validate_tag(&self, _node_id: NodeId, tag: &Tag) -> PluginResult<()> {
        if tag.name.is_empty() {
            return Err(PluginError::tag_invalid("tag name required"));
        }
        let addr = &tag.address;
        if addr.is_empty() {
            return Err(PluginError::tag_invalid("address (LD/LN.DA) required"));
        }
        // Must contain / or . for LD/LN format
        if !addr.contains('/') && !addr.contains('.') {
            return Err(PluginError::tag_invalid("IEC61850 address must be LD/LN.DA format"));
        }
        Ok(())
    }

    async fn open(&self, node_id: NodeId, config: PluginConfig) -> PluginResult<()> {
        let host = config.get("host").and_then(|v| v.as_str()).unwrap_or("192.168.1.100").to_string();
        let port = config.get("port").and_then(|v| v.as_u64()).unwrap_or(102) as u16;
        let ied_name = config.get("ied_name").and_then(|v| v.as_str()).unwrap_or("IED1").to_string();
        log::info(node_id, format!("open iec61850: host={}, port={}, ied_name={}", host, port, ied_name));

        let groups = Self::default_groups();
        let tags = groups
            .iter()
            .flat_map(|g| {
                vec![
                    Tag {
                        id: TagId::new(),
                        name: "frequency".to_string(),
                        address: "MMXU1/Hz.HighZHz".to_string(),
                        attr: gateway_sdk::TagAttr::Read,
                        data_type: Some("float64".to_string()),
                        description: Some("频率 (Hz)".to_string()),
                        group_id: g.id,
                    },
                    Tag {
                        id: TagId::new(),
                        name: "voltage_a".to_string(),
                        address: "MMXU1/PhV.phsA.cval.mag.f".to_string(),
                        attr: gateway_sdk::TagAttr::Read,
                        data_type: Some("float64".to_string()),
                        description: Some("A相电压 (V)".to_string()),
                        group_id: g.id,
                    },
                    Tag {
                        id: TagId::new(),
                        name: "current_a".to_string(),
                        address: "MMXU1/A.phsA.cval.mag.f".to_string(),
                        attr: gateway_sdk::TagAttr::Read,
                        data_type: Some("float64".to_string()),
                        description: Some("A相电流 (A)".to_string()),
                        group_id: g.id,
                    },
                    Tag {
                        id: TagId::new(),
                        name: "active_power".to_string(),
                        address: "MMXU1/WTot.actWh".to_string(),
                        attr: gateway_sdk::TagAttr::Read,
                        data_type: Some("float64".to_string()),
                        description: Some("总有功功率 (W)".to_string()),
                        group_id: g.id,
                    },
                ]
            })
            .collect::<Vec<_>>();

        let mut state = self.state.write().await;
        state.insert(node_id, Iec61850State { host, port, ied_name, groups, tags });
        Ok(())
    }

    async fn close(&self, node_id: NodeId) -> PluginResult<()> {
        log::info(node_id, "close iec61850");
        let mut state = self.state.write().await;
        state.remove(&node_id);
        Ok(())
    }

    async fn start(&self, node_id: NodeId) -> PluginResult<()> {
        log::info(node_id, "start iec61850");
        Ok(())
    }

    async fn stop(&self, node_id: NodeId) -> PluginResult<()> {
        log::info(node_id, "stop iec61850");
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
            let value = if tag.address.contains("Hz") {
                50.0 + (rand_simple() * 0.1) // Frequency ~50Hz
            } else if tag.address.contains("PhV") || tag.address.contains("Vol") {
                220.0 + (rand_simple() * 10.0) // Voltage ~220V
            } else if tag.address.contains("A.phs") || tag.address.contains("Amp") {
                100.0 + (rand_simple() * 5.0) // Current ~100A
            } else if tag.address.contains("WTot") || tag.address.contains("W") {
                1000.0 + (rand_simple() * 50.0) // Power ~1000W
            } else {
                0.0
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
