//! BACnet/IP 南向插件：支持 BACnet/IP 楼宇自动化协议，读取模拟量/数字量输入输出。
//!
//! 地址格式：{object_type}:{instance}:{property}
//!   例如：analogInput:0:presentValue, binaryInput:1:presentValue
//!
//! 配置：
//!   { "host": "192.168.1.100", "port": 47808, "device_id": 123 }

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

/// BACnet 对象类型
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum BacnetObjectType {
    AnalogInput,
    AnalogOutput,
    AnalogValue,
    BinaryInput,
    BinaryOutput,
    BinaryValue,
    MultiStateInput,
    MultiStateOutput,
    Unknown,
}

impl BacnetObjectType {
    fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "analoginput" | "ai" => BacnetObjectType::AnalogInput,
            "analogoutput" | "ao" => BacnetObjectType::AnalogOutput,
            "analogvalue" | "av" => BacnetObjectType::AnalogValue,
            "binaryinput" | "bi" => BacnetObjectType::BinaryInput,
            "binaryoutput" | "bo" => BacnetObjectType::BinaryOutput,
            "binaryvalue" | "bv" => BacnetObjectType::BinaryValue,
            "multistateinput" | "msi" => BacnetObjectType::MultiStateInput,
            "multistateoutput" | "mso" => BacnetObjectType::MultiStateOutput,
            _ => BacnetObjectType::Unknown,
        }
    }
}

/// BACnet点位状态
struct BacnetTagState {
    object_type: BacnetObjectType,
    instance: u32,
    property: String,
}

/// BACnet插件状态
struct BacnetNodeState {
    host: String,
    port: u16,
    device_id: u32,
    connected: bool,
    groups: Vec<Group>,
}

impl BacnetNodeState {
    fn new(host: String, port: u16, device_id: u32) -> Self {
        Self {
            host,
            port,
            device_id,
            connected: false,
            groups: vec![Group {
                id: GroupId::new(),
                name: "default".to_string(),
                interval_ms: 1000,
                description: Some("默认采集组".to_string()),
            }],
        }
    }
}

/// BACnet/IP 南向插件
pub struct BacnetPlugin {
    state: Arc<RwLock<HashMap<NodeId, BacnetNodeState>>>,
}

impl Default for BacnetPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl BacnetPlugin {
    pub fn new() -> Self {
        Self {
            state: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

#[async_trait::async_trait]
impl SouthPlugin for BacnetPlugin {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "bacnet",
            kind: PluginKind::South,
            description: Some("BACnet/IP 楼宇自动化协议"),
            version: "0.1.0",
            name_zh: Some("BACnet"),
            name_en: Some("BACnet"),
            description_zh: Some("BACnet/IP 楼宇自动化协议，支持模拟量/数字量输入输出"),
            description_en: Some("BACnet/IP protocol for building automation — analog/digital I/O"),
        }
    }

    fn config_schema(&self) -> Option<ConfigSchema> {
        Some(
            ConfigSchema::new()
                .param(ParamSchema {
                    name: "host".to_string(),
                    name_zh: Some("IP地址".to_string()),
                    name_en: Some("IP Address".to_string()),
                    description: Some("BACnet设备IP地址".to_string()),
                    description_zh: Some("BACnet设备IP地址".to_string()),
                    description_en: Some("BACnet device IP address".to_string()),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::String,
                    default: Some(serde_json::json!("192.168.1.100")),
                    valid: None,
                    options: None,
                    depends_on: None,
                    depends_value: None,
                    depends_values: None,
                })
                .param(ParamSchema {
                    name: "port".to_string(),
                    name_zh: Some("端口号".to_string()),
                    name_en: Some("Port".to_string()),
                    description: Some("BACnet默认端口47808".to_string()),
                    description_zh: Some("BACnet默认端口47808".to_string()),
                    description_en: Some("BACnet default port 47808".to_string()),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(47808)),
                    valid: Some(ParamValid { min: Some(1), max: Some(65535), regex: None, length: None }),
                    options: None,
                    depends_on: None,
                    depends_value: None,
                    depends_values: None,
                })
                .param(ParamSchema {
                    name: "device_id".to_string(),
                    name_zh: Some("设备ID".to_string()),
                    name_en: Some("Device ID".to_string()),
                    description: Some("BACnet设备ID".to_string()),
                    description_zh: Some("BACnet设备ID".to_string()),
                    description_en: Some("BACnet device instance ID".to_string()),
                    attribute: ParamAttribute::Optional,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(123)),
                    valid: Some(ParamValid { min: Some(0), max: Some(4194303), regex: None, length: None }),
                    options: None,
                    depends_on: None,
                    depends_value: None,
                    depends_values: None,
                }),
        )
    }

    fn tag_schema(&self) -> Option<TagSchema> {
        Some(TagSchema {
            data_types: Some(vec![
                "float".to_string(),
                "bool".to_string(),
                "uint".to_string(),
                "string".to_string(),
            ]),
            address_format: Some(
                "{object_type}:{instance}:{property}".to_string(),
            ),
            address_format_zh: Some(
                "{对象类型}:{实例}:{属性}，例如 analogInput:0:presentValue".to_string(),
            ),
            address_format_en: Some(
                "{object_type}:{instance}:{property}, e.g. analogInput:0:presentValue".to_string(),
            ),
        })
    }

    async fn validate_tag(&self, _node_id: NodeId, tag: &Tag) -> PluginResult<()> {
        if tag.name.is_empty() {
            return Err(PluginError::tag_invalid("tag name required"));
        }
        // BACnet地址格式: object_type:instance:property
        let parts: Vec<&str> = tag.address.split(':').collect();
        if parts.len() != 3 {
            return Err(PluginError::tag_invalid(
                "BACnet address format: {object_type}:{instance}:{property}",
            ));
        }
        let obj_type = BacnetObjectType::from_str(parts[0]);
        if obj_type == BacnetObjectType::Unknown {
            return Err(PluginError::tag_invalid(&format!(
                "unknown BACnet object type: {}",
                parts[0]
            )));
        }
        if parts[1].parse::<u32>().is_err() {
            return Err(PluginError::tag_invalid(&format!(
                "invalid instance number: {}",
                parts[1]
            )));
        }
        Ok(())
    }

    async fn open(&self, node_id: NodeId, config: PluginConfig) -> PluginResult<()> {
        let host = config
            .get("host")
            .and_then(|v| v.as_str())
            .unwrap_or("192.168.1.100")
            .to_string();
        let port = config
            .get("port")
            .and_then(|v| v.as_u64())
            .unwrap_or(47808) as u16;
        let device_id = config
            .get("device_id")
            .and_then(|v| v.as_u64())
            .unwrap_or(123) as u32;

        log::info(
            node_id,
            format!("open bacnet: host={}, port={}, device_id={}", host, port, device_id),
        );

        let state = BacnetNodeState::new(host.clone(), port, device_id);
        let mut states = self.state.write().await;
        states.insert(node_id, state);
        Ok(())
    }

    async fn close(&self, node_id: NodeId) -> PluginResult<()> {
        log::info(node_id, "close bacnet");
        let mut states = self.state.write().await;
        states.remove(&node_id);
        Ok(())
    }

    async fn start(&self, node_id: NodeId) -> PluginResult<()> {
        log::info(node_id, "start bacnet");
        let mut states = self.state.write().await;
        if let Some(s) = states.get_mut(&node_id) {
            s.connected = true;
        }
        Ok(())
    }

    async fn stop(&self, node_id: NodeId) -> PluginResult<()> {
        log::info(node_id, "stop bacnet");
        let mut states = self.state.write().await;
        if let Some(s) = states.get_mut(&node_id) {
            s.connected = false;
        }
        Ok(())
    }

    async fn poll_group(
        &self,
        node_id: NodeId,
        _group_id: GroupId,
        tags: &[Tag],
    ) -> PluginResult<Vec<(TagId, DataValue)>> {
        let states = self.state.read().await;
        let state = states
            .get(&node_id)
            .ok_or_else(|| PluginError::msg("node not open"))?;

        if !state.connected {
            return Err(PluginError::msg("plugin not started"));
        }

        // 简化实现：返回mock数据
        // 实际实现需要使用BACnet协议栈连接设备读取

        let mut results = Vec::with_capacity(tags.len());
        for tag in tags {
            let parts: Vec<&str> = tag.address.split(':').collect();
            if parts.len() != 3 {
                continue;
            }
            let obj_type = BacnetObjectType::from_str(parts[0]);
            let value = match obj_type {
                BacnetObjectType::AnalogInput | BacnetObjectType::AnalogOutput | BacnetObjectType::AnalogValue => {
                    DataValue::Float32(0.0_f32)
                }
                BacnetObjectType::BinaryInput | BacnetObjectType::BinaryOutput | BacnetObjectType::BinaryValue => {
                    DataValue::Bool(false)
                }
                    _ => DataValue::Float32(0.0_f32),
            };
            results.push((tag.id, value));
        }
        Ok(results)
    }

    async fn write_tags(
        &self,
        node_id: NodeId,
        values: &[(Tag, DataValue)],
    ) -> PluginResult<()> {
        log::warn(node_id, format!("write_tags called with {} values (not implemented)", values.len()));
        Err(PluginError::not_supported("BACnet write not implemented"))
    }

    async fn list_groups(&self, node_id: NodeId) -> PluginResult<Vec<Group>> {
        let states = self.state.read().await;
        let state = states
            .get(&node_id)
            .ok_or_else(|| PluginError::msg("node not open"))?;
        Ok(state.groups.clone())
    }

    async fn list_tags(&self, node_id: NodeId, _group_id: GroupId) -> PluginResult<Vec<Tag>> {
        let states = self.state.read().await;
        let _state = states
            .get(&node_id)
            .ok_or_else(|| PluginError::msg("node not open"))?;
        // 返回示例标签
        Ok(vec![
            Tag {
                id: TagId::new(),
                name: "ai_0".to_string(),
                address: "analogInput:0:presentValue".to_string(),
                attr: gateway_sdk::TagAttr::Read,
                data_type: Some("float".to_string()),
                description: Some("模拟输入0".to_string()),
                group_id: GroupId::new(),
            },
            Tag {
                id: TagId::new(),
                name: "bi_0".to_string(),
                address: "binaryInput:0:presentValue".to_string(),
                attr: gateway_sdk::TagAttr::Read,
                data_type: Some("bool".to_string()),
                description: Some("数字输入0".to_string()),
                group_id: GroupId::new(),
            },
        ])
    }
}
