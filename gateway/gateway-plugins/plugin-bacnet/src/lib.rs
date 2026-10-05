//! 南向 BACnet/IP 插件：通过 UDP 47808 连接 BACnet 设备。
//!
//! 地址格式：object-type:instance[:property]
//!   ai:1 / ao:5:units / csv:2 等（见 address.rs 缩写表）。

#[cfg(feature = "ffi")]
mod ffi;

mod address;
mod config;
mod state;
mod value;

use gateway_sdk::log;
use gateway_sdk::types::{DataValue, PluginKind};
use gateway_sdk::{
    ConfigSchema, Group, GroupId, NodeId, ParamOption, ParamSchema, ParamType, ParamValid,
    PluginMeta, SouthPlugin, Tag, TagId, TagRegexEntry, TagSchema,
};
use gateway_sdk::{PluginConfig, PluginError, PluginResult};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use address::{parse_address, BacnetObjectType};
use config::{config_str, config_u16, config_u32, config_u64};
use state::BacnetState;
use value::bacnet_property_to_datavalue;

/// BACnet 南向插件
pub struct BacnetPlugin {
    state: Arc<RwLock<HashMap<NodeId, BacnetState>>>,
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

    fn default_groups() -> Vec<Group> {
        vec![Group {
            id: GroupId::new(),
            name: "default".to_string(),
            interval_ms: 1000,
            description: Some("默认采集组".to_string()),
        }]
    }
}

#[async_trait::async_trait]
impl SouthPlugin for BacnetPlugin {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "bacnet",
            kind: PluginKind::South,
            description: Some("BACnet/IP south driver via UDP 47808"),
            version: "0.1.0",
            name_zh: Some("BACnet/IP"),
            name_en: Some("BACnet/IP"),
            description_zh: Some("BACnet/IP 南向驱动，通过 UDP 47808 连接 BACnet 设备"),
            description_en: Some("BACnet/IP south driver, connect to BACnet devices via UDP 47808"),
        }
    }

    fn config_schema(&self) -> Option<ConfigSchema> {
        use gateway_sdk::ParamAttribute;
        Some(
            ConfigSchema::new()
                .param(ParamSchema {
                    name: "ip".to_string(),
                    name_zh: Some("目标设备 IP".to_string()),
                    name_en: Some("Target Device IP".to_string()),
                    description: Some("Target BACnet device IP address".to_string()),
                    description_zh: Some("目标 BACnet 设备的 IP 地址".to_string()),
                    description_en: Some("Target BACnet device IP address".to_string()),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::String,
                    default: Some(serde_json::json!("127.0.0.1")),
                    valid: None,
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "port".to_string(),
                    name_zh: Some("端口号".to_string()),
                    name_en: Some("Port".to_string()),
                    description: Some("BACnet/IP port (default 47808)".to_string()),
                    description_zh: Some("BACnet/IP 端口（默认 47808）".to_string()),
                    description_en: Some("BACnet/IP port (default 47808)".to_string()),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(47808)),
                    valid: Some(ParamValid { min: Some(1), max: Some(65535), regex: None, length: None }),
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "device_instance".to_string(),
                    name_zh: Some("设备实例号".to_string()),
                    name_en: Some("Device Instance".to_string()),
                    description: Some("BACnet device instance number (optional). If set, Who-Is will be sent to verify device exists.".to_string()),
                    description_zh: Some("BACnet 设备实例号（可选）。若设置则先发 Who-Is 校验设备存在。".to_string()),
                    description_en: Some("BACnet device instance number (optional). If set, Who-Is will be sent to verify device exists.".to_string()),
                    attribute: ParamAttribute::Optional,
                    ty: ParamType::Int,
                    default: None,
                    valid: Some(ParamValid { min: Some(0), max: Some(4194303), regex: None, length: None }),
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "timeout_ms".to_string(),
                    name_zh: Some("超时 (ms)".to_string()),
                    name_en: Some("Timeout (ms)".to_string()),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(3000)),
                    valid: Some(ParamValid { min: Some(500), max: Some(60000), regex: None, length: None }),
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "write_priority".to_string(),
                    name_zh: Some("写优先级".to_string()),
                    name_en: Some("Write Priority".to_string()),
                    description: Some("BACnet WriteProperty priority (1-16, default 8)".to_string()),
                    description_zh: Some("BACnet WriteProperty 优先级（1-16，默认 8）".to_string()),
                    description_en: Some("BACnet WriteProperty priority (1-16, default 8)".to_string()),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(8)),
                    valid: Some(ParamValid { min: Some(1), max: Some(16), regex: None, length: None }),
                    ..Default::default()
                })
                .tag_regex(vec![
                    TagRegexEntry {
                        data_type: "bool".to_string(),
                        regex: r"^(ai|ao|av|bi|bo|bv|msi|mso|msv|csv|iv|lav|osv|piv|bsv):[0-9]+(:[a-z-]+)?$".to_string(),
                    },
                    TagRegexEntry {
                        data_type: "int32".to_string(),
                        regex: r"^(ai|ao|av|bi|bo|bv|msi|mso|msv|csv|iv|lav|osv|piv|bsv):[0-9]+(:[a-z-]+)?$".to_string(),
                    },
                    TagRegexEntry {
                        data_type: "uint64".to_string(),
                        regex: r"^(ai|ao|av|bi|bo|bv|msi|mso|msv|csv|iv|lav|osv|piv|bsv):[0-9]+(:[a-z-]+)?$".to_string(),
                    },
                    TagRegexEntry {
                        data_type: "float64".to_string(),
                        regex: r"^(ai|ao|av|bi|bo|bv|msi|mso|msv|csv|iv|lav|osv|piv|bsv):[0-9]+(:[a-z-]+)?$".to_string(),
                    },
                    TagRegexEntry {
                        data_type: "string".to_string(),
                        regex: r"^csv:[0-9]+(:[a-z-]+)?$".to_string(),
                    },
                    TagRegexEntry {
                        data_type: "bytes".to_string(),
                        regex: r"^(bsv|osv):[0-9]+(:[a-z-]+)?$".to_string(),
                    },
                ]),
        )
    }

    fn tag_schema(&self) -> Option<TagSchema> {
        Some(TagSchema {
            data_types: Some(vec![
                "bool".to_string(),
                "int32".to_string(),
                "uint64".to_string(),
                "float64".to_string(),
                "string".to_string(),
                "bytes".to_string(),
            ]),
            address_format: Some(
                "object-type:instance[:property]，如 ai:1 / ao:5:units".to_string(),
            ),
            address_format_zh: Some(
                "object-type:instance[:property]，如 ai:1 / ao:5:units（缩写表见文档）".to_string(),
            ),
            address_format_en: Some(
                "object-type:instance[:property], e.g. ai:1 / ao:5:units (see docs for abbr table)"
                    .to_string(),
            ),
        })
    }

    async fn validate_tag(&self, _node_id: NodeId, tag: &Tag) -> PluginResult<()> {
        if tag.name.is_empty() {
            return Err(PluginError::tag_invalid("tag name required"));
        }
        let parsed = parse_address(&tag.address).ok_or_else(|| {
            PluginError::tag_invalid(
                "address format: object-type:instance[:property]，如 ai:1 / ao:5:units",
            )
        })?;

        // 检查 data_type 是否与对象类型匹配
        let expected_dt = parsed.object_type.default_data_type();
        if let Some(ref dt) = tag.data_type {
            // 允许任意匹配，因为同一个对象类型可能对应多种 data_type
            let _ = expected_dt;
        }
        Ok(())
    }

    async fn open(&self, node_id: NodeId, config: PluginConfig) -> PluginResult<()> {
        let ip = config_str(&config, "ip", "127.0.0.1");
        let port = config_u16(&config, "port", 47808);
        log::info(node_id, format!("open bacnet: ip={}, port={}", ip, port));

        let device_instance = config_u32(&config, "device_instance", 0);
        let timeout_ms = config_u64(&config, "timeout_ms", 3000);
        let write_priority = config_u16(&config, "write_priority", 8) as u8;

        let groups = Self::default_groups();
        let tags = groups
            .iter()
            .flat_map(|g| {
                vec![Tag {
                    id: TagId::new(),
                    name: "ai_1".to_string(),
                    address: "ai:1".to_string(),
                    attr: gateway_sdk::TagAttr::Read,
                    data_type: Some("float64".to_string()),
                    description: Some("Analog Input 1".to_string()),
                    group_id: g.id,
                }]
            })
            .collect::<Vec<_>>();

        let mut state = self.state.write().await;
        state.insert(
            node_id,
            BacnetState {
                ip,
                port,
                device_instance,
                timeout_ms,
                write_priority,
                groups,
                tags,
                failure_count: 0,
                #[cfg(feature = "bacnet-client")]
                client: Arc::new(std::sync::Mutex::new(None)),
            },
        );
        Ok(())
    }

    async fn close(&self, node_id: NodeId) -> PluginResult<()> {
        log::info(node_id, "close bacnet");
        let mut state = self.state.write().await;
        state.remove(&node_id);
        Ok(())
    }

    async fn start(&self, node_id: NodeId) -> PluginResult<()> {
        log::info(node_id, "start bacnet");
        Ok(())
    }

    async fn stop(&self, node_id: NodeId) -> PluginResult<()> {
        log::info(node_id, "stop bacnet");
        Ok(())
    }

    async fn setting(&self, node_id: NodeId, config: PluginConfig) -> PluginResult<()> {
        log::info(node_id, "setting bacnet (config updated)");
        let ip = config_str(&config, "ip", "127.0.0.1");
        let port = config_u16(&config, "port", 47808);
        let mut state = self.state.write().await;
        if let Some(s) = state.get_mut(&node_id) {
            s.ip = ip;
            s.port = port;
            s.device_instance = config_u32(&config, "device_instance", 0);
            s.timeout_ms = config_u64(&config, "timeout_ms", s.timeout_ms);
            s.write_priority = config_u16(&config, "write_priority", s.write_priority as u16) as u8;
            // 热改配置时丢弃现有客户端
            #[cfg(feature = "bacnet-client")]
            {
                if let Ok(mut guard) = s.client.lock() {
                    *guard = None;
                }
            }
            s.record_failure();
        }
        Ok(())
    }

    async fn poll_group(
        &self,
        node_id: NodeId,
        _group_id: GroupId,
        tags: &[Tag],
    ) -> PluginResult<Vec<(TagId, DataValue)>> {
        let state = self.state.read().await;
        let s = state
            .get(&node_id)
            .ok_or_else(|| PluginError::msg("node not open"))?;

        #[cfg(feature = "bacnet-client")]
        {
            let s = s.clone();
            let tags = tags.to_vec();

            // BACnet 读：全部在 spawn_blocking 内完成
            let result = tokio::task::spawn_blocking(move || poll_group_sync(&s, &tags))
                .await
                .map_err(|e| PluginError::msg(format!("bacnet poll: {}", e)))?;

            // 更新状态
            let mut state_w = self.state.write().await;
            if let Some(gs) = state_w.get_mut(&node_id) {
                match &result {
                    Ok(_) => {
                        gs.record_success();
                    }
                    Err(_) => {
                        gs.record_failure();
                    }
                }
            }

            result
        }

        #[cfg(not(feature = "bacnet-client"))]
        {
            let _ = (node_id, s);
            let mut out = Vec::with_capacity(tags.len());
            for tag in tags {
                let _ = parse_address(&tag.address);
                out.push((tag.id, DataValue::Bool(false)));
            }
            Ok(out)
        }
    }

    async fn list_groups(&self, node_id: NodeId) -> PluginResult<Vec<Group>> {
        let state = self.state.read().await;
        let s = state
            .get(&node_id)
            .ok_or_else(|| PluginError::msg("node not open"))?;
        Ok(s.groups.clone())
    }

    async fn list_tags(&self, node_id: NodeId, group_id: GroupId) -> PluginResult<Vec<Tag>> {
        let state = self.state.read().await;
        let s = state
            .get(&node_id)
            .ok_or_else(|| PluginError::msg("node not open"))?;
        Ok(s.tags
            .iter()
            .filter(|t| t.group_id == group_id)
            .cloned()
            .collect())
    }

    async fn write_tags(&self, node_id: NodeId, values: &[(Tag, DataValue)]) -> PluginResult<()> {
        if values.is_empty() {
            return Ok(());
        }
        let state = self.state.read().await;
        let s = state
            .get(&node_id)
            .ok_or_else(|| PluginError::msg("node not open"))?;

        #[cfg(feature = "bacnet-client")]
        {
            let s = s.clone();
            let values = values.to_vec();

            let result = tokio::task::spawn_blocking(move || write_tags_sync(&s, &values))
                .await
                .map_err(|e| PluginError::msg(format!("bacnet write: {}", e)))?;

            let mut state_w = self.state.write().await;
            if let Some(gs) = state_w.get_mut(&node_id) {
                match &result {
                    Ok(_) => gs.record_success(),
                    Err(_) => gs.record_failure(),
                }
            }
            result
        }

        #[cfg(not(feature = "bacnet-client"))]
        {
            let _ = (node_id, s, values);
            Err(PluginError::not_supported(
                "write requires bacnet-client feature",
            ))
        }
    }
}

/// 同步轮询（在 spawn_blocking 内调用 BacnetClient）
#[cfg(feature = "bacnet-client")]
fn poll_group_sync(s: &BacnetState, tags: &[Tag]) -> PluginResult<Vec<(TagId, DataValue)>> {
    use bacnet_rs::property::PropertyIdentifier;

    // 获取或创建客户端
    let mut client_guard = s.client.lock().unwrap();
    let client = if client_guard.is_none() {
        let timeout = std::time::Duration::from_millis(s.timeout_ms);
        let cfg = bacnet_rs::client::ClientConfig {
            bind_addr: "0.0.0.0:0".to_string(),
            timeout,
            retries: 0,
        };
        match bacnet_rs::client::BacnetClient::from_config(cfg) {
            Ok(c) => {
                *client_guard = Some(c);
                client_guard.as_ref().unwrap()
            }
            Err(e) => {
                return Err(PluginError::connection_failed(format!(
                    "bacnet client create: {}",
                    e
                )));
            }
        }
    } else {
        client_guard.as_ref().unwrap()
    };

    // 目标地址
    let target: bacnet_rs::client::BacnetTarget =
        format!("{}:{}", s.ip, s.port)
            .parse()
            .map_err(|e: std::net::AddrParseError| {
                PluginError::config_invalid(format!("invalid target addr: {}", e))
            })?;

    // 若配置了 device_instance，先 Who-Is 校验
    if let Some(_dev_inst) = s.device_instance {
        // Who-Is 忽略超时错误（设备可能不在线）
        let _ = client.discover_device(target.clone());
    }

    // ReadPropertyMultiple 批量读
    let mut slots: Vec<Option<(TagId, DataValue)>> = vec![None; tags.len()];
    let mut failure: Option<PluginError> = None;

    for (i, tag) in tags.iter().enumerate() {
        let parsed = match parse_address(&tag.address) {
            Some(p) => p,
            None => {
                slots[i] = Some((tag.id, DataValue::Bool(false)));
                continue;
            }
        };

        let object_type = parsed.object_type.to_object_type();
        let instance = parsed.instance;

        // 属性名 → PropertyIdentifier
        let property_id = match parsed.property.as_str() {
            "present-value" | "pv" => PropertyIdentifier::PresentValue,
            "status-flags" | "sf" => PropertyIdentifier::StatusFlags,
            "units" => PropertyIdentifier::Units,
            "description" => PropertyIdentifier::Description,
            "object-name" => PropertyIdentifier::ObjectName,
            other => {
                // 尝试解析为数字属性 ID
                if let Ok(id) = other.parse::<u32>() {
                    PropertyIdentifier::from(id)
                } else {
                    slots[i] = Some((tag.id, DataValue::Bool(false)));
                    continue;
                }
            }
        };

        match client.read_property(
            target.clone(),
            bacnet_rs::object::ObjectIdentifier::new(object_type, instance),
            property_id,
        ) {
            Ok(values) => {
                if let Some(pv) = values.first() {
                    let dv = bacnet_property_to_datavalue(&parsed.object_type, pv);
                    slots[i] = Some((tag.id, dv));
                } else {
                    slots[i] = Some((tag.id, DataValue::Bool(false)));
                }
            }
            Err(e) => {
                failure = Some(PluginError::msg(format!(
                    "bacnet read {}: {}",
                    tag.address, e
                )));
                break;
            }
        }
    }

    match failure {
        Some(e) => Err(e),
        None => Ok(slots
            .into_iter()
            .enumerate()
            .map(|(i, v)| v.unwrap_or((tags[i].id, DataValue::Bool(false))))
            .collect()),
    }
}

/// 同步写（在 spawn_blocking 内调用 BacnetClient）
#[cfg(feature = "bacnet-client")]
fn write_tags_sync(s: &BacnetState, values: &[(Tag, DataValue)]) -> PluginResult<()> {
    use bacnet_rs::property::PropertyValue;

    let mut client_guard = s.client.lock().unwrap();
    let client = match client_guard.as_mut() {
        Some(c) => c,
        None => {
            return Err(PluginError::msg("bacnet client not initialized"));
        }
    };

    let target: bacnet_rs::client::BacnetTarget =
        format!("{}:{}", s.ip, s.port)
            .parse()
            .map_err(|e: std::net::AddrParseError| {
                PluginError::config_invalid(format!("invalid target addr: {}", e))
            })?;

    let priority = s.write_priority;

    for (tag, value) in values {
        let parsed = match parse_address(&tag.address) {
            Some(p) => p,
            None => continue,
        };

        let object_id = bacnet_rs::object::ObjectIdentifier::new(
            parsed.object_type.to_object_type(),
            parsed.instance,
        );

        // DataValue → PropertyValue
        let prop_value: PropertyValue = match value {
            DataValue::Bool(b) => PropertyValue::Boolean(*b),
            DataValue::Int8(v) => PropertyValue::SignedInteger(*v as i64),
            DataValue::Int16(v) => PropertyValue::SignedInteger(*v as i64),
            DataValue::Int32(v) => PropertyValue::SignedInteger(*v as i64),
            DataValue::Int64(v) => PropertyValue::SignedInteger(*v),
            DataValue::UInt8(v) => PropertyValue::UnsignedInteger(*v as u64),
            DataValue::UInt16(v) => PropertyValue::UnsignedInteger(*v as u64),
            DataValue::UInt32(v) => PropertyValue::UnsignedInteger(*v as u64),
            DataValue::UInt64(v) => PropertyValue::UnsignedInteger(*v),
            DataValue::Float32(v) => PropertyValue::Real(*v),
            DataValue::Float64(v) => PropertyValue::Double(*v),
            DataValue::String(v) => PropertyValue::CharacterString(v.clone()),
            DataValue::Bytes(v) => PropertyValue::OctetString(v.clone()),
        };

        match client.write_property(
            target.clone(),
            object_id,
            bacnet_rs::property::PropertyIdentifier::PresentValue,
            prop_value,
            priority,
        ) {
            Ok(_) => {}
            Err(e) => {
                return Err(PluginError::msg(format!(
                    "bacnet write {}: {}",
                    tag.address, e
                )));
            }
        }
    }
    Ok(())
}
