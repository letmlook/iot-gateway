//! 南向 OPC UA 插件。
//! 功能：端点 URL、用户名/密码、证书/密钥、地址格式 NS!NODEID、读写。

#[cfg(feature = "ffi")]
mod ffi;

use gateway_sdk::{
    ConfigSchema, Group, GroupId, NodeId, ParamAttribute, ParamOption, ParamSchema, ParamType,
    ParamValid, PluginMeta, SouthPlugin, Tag, TagId, TagRegexEntry, TagSchema,
};
use gateway_sdk::types::{DataValue, PluginKind};
use gateway_sdk::{PluginConfig, PluginError, PluginResult};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

const DEFAULT_ENDPOINT: &str = "opc.tcp://127.0.0.1:4840/";

/// 解析后的 OPC UA 地址：NS!NODEID（NS=命名空间索引，NODEID=数字或字符串）
#[derive(Clone, Debug)]
#[allow(dead_code)]
struct ParsedOpcAddress {
    namespace: u16,
    /// 数字节点 ID 或字符串节点 ID
    node_id: OpcNodeId,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
enum OpcNodeId {
    Numeric(u32),
    String(String),
}

/// 解析地址格式：NS!NODEID。例如 0!2258、2!Device1.Module1.Tag1
fn parse_address(addr: &str) -> Option<ParsedOpcAddress> {
    let addr = addr.trim();
    let mut it = addr.split('!');
    let ns_str = it.next()?;
    let node_part = it.next()?;
    if it.next().is_some() {
        return None;
    }
    let namespace: u16 = ns_str.parse().ok()?;
    let node_id = if let Ok(n) = node_part.parse::<u32>() {
        OpcNodeId::Numeric(n)
    } else {
        OpcNodeId::String(node_part.to_string())
    };
    Some(ParsedOpcAddress { namespace, node_id })
}

/// OPC UA 南向插件
pub struct OpcuaPlugin {
    state: Arc<RwLock<HashMap<NodeId, OpcuaState>>>,
}

#[allow(dead_code)]
struct OpcuaState {
    endpoint_url: String,
    username: Option<String>,
    password: Option<String>,
    certificate: Option<String>,
    key: Option<String>,
    groups: Vec<Group>,
    tags: Vec<Tag>,
}

impl Default for OpcuaPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl OpcuaPlugin {
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

fn config_str(config: &PluginConfig, key: &str, default: &str) -> String {
    config
        .get(key)
        .and_then(|v| v.as_str())
        .map(String::from)
        .unwrap_or_else(|| default.to_string())
}

#[async_trait::async_trait]
impl SouthPlugin for OpcuaPlugin {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "opcua",
            kind: PluginKind::South,
            description: Some("OPC UA 南向驱动，连接 OPC UA 服务器采集与写点位"),
            version: "0.1.0",
            name_zh: Some("OPC UA"),
            name_en: Some("OPC UA"),
            description_zh: Some("OPC UA 南向驱动，连接 OPC UA 服务器采集与写点位"),
            description_en: Some("OPC UA south driver, connect to OPC UA server for read/write tags"),
        }
    }

    fn config_schema(&self) -> Option<ConfigSchema> {
        Some(
            ConfigSchema::new()
                .param(ParamSchema {
                    name: "endpoint_url".to_string(),
                    name_zh: Some("端点 URL".to_string()),
                    name_en: Some("Endpoint URL".to_string()),
                    description: Some("OPCUA server endpoint url".to_string()),
                    description_zh: Some("OPCUA 服务器端点 URL".to_string()),
                    description_en: Some("OPCUA server endpoint url".to_string()),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::String,
                    default: Some(serde_json::json!(DEFAULT_ENDPOINT)),
                    valid: Some(ParamValid {
                        min: None,
                        max: None,
                        regex: Some(r"^opc\.tcp:\/\/\S+:\d+(\/[\w\-._~:/?#\[\]@!$&'()*+,;=]*)?$".to_string()),
                        length: Some(256),
                    }),
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "username".to_string(),
                    name_zh: Some("用户名".to_string()),
                    name_en: Some("Username".to_string()),
                    attribute: ParamAttribute::Optional,
                    ty: ParamType::String,
                    default: Some(serde_json::json!("")),
                    valid: Some(ParamValid { min: None, max: None, regex: None, length: Some(30) }),
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "password".to_string(),
                    name_zh: Some("密码".to_string()),
                    name_en: Some("Password".to_string()),
                    attribute: ParamAttribute::Optional,
                    ty: ParamType::String,
                    default: Some(serde_json::json!("")),
                    valid: Some(ParamValid { min: None, max: None, regex: None, length: Some(30) }),
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "certificate".to_string(),
                    name_zh: Some("证书文件".to_string()),
                    name_en: Some("Certificate file".to_string()),
                    attribute: ParamAttribute::Optional,
                    ty: ParamType::File,
                    default: None,
                    valid: Some(ParamValid { min: None, max: None, regex: None, length: Some(81960) }),
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "key".to_string(),
                    name_zh: Some("密钥文件".to_string()),
                    name_en: Some("Key file".to_string()),
                    attribute: ParamAttribute::Optional,
                    ty: ParamType::File,
                    default: None,
                    valid: Some(ParamValid { min: None, max: None, regex: None, length: Some(81960) }),
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "security_mode".to_string(),
                    name_zh: Some("安全策略".to_string()),
                    name_en: Some("Security Mode".to_string()),
                    attribute: ParamAttribute::Optional,
                    ty: ParamType::Select,
                    default: Some(serde_json::json!(1)),
                    valid: None,
                    options: Some(vec![
                        ParamOption { value: serde_json::json!(1), label: Some("None".to_string()), label_zh: Some("无".to_string()), label_en: Some("None".to_string()) },
                        ParamOption { value: serde_json::json!(2), label: Some("Sign".to_string()), label_zh: Some("签名".to_string()), label_en: Some("Sign".to_string()) },
                        ParamOption { value: serde_json::json!(3), label: Some("Sign & Encrypt".to_string()), label_zh: Some("签名与加密".to_string()), label_en: Some("Sign & Encrypt".to_string()) },
                    ]),
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "update_mode".to_string(),
                    name_zh: Some("更新模式".to_string()),
                    name_en: Some("Update Mode".to_string()),
                    attribute: ParamAttribute::Optional,
                    ty: ParamType::Select,
                    default: Some(serde_json::json!(1)),
                    valid: None,
                    options: Some(vec![
                        ParamOption { value: serde_json::json!(1), label: Some("Read".to_string()), label_zh: Some("读取".to_string()), label_en: Some("Read".to_string()) },
                        ParamOption { value: serde_json::json!(2), label: Some("Subscribe".to_string()), label_zh: Some("订阅".to_string()), label_en: Some("Subscribe".to_string()) },
                        ParamOption { value: serde_json::json!(3), label: Some("Read & Subscribe".to_string()), label_zh: Some("读取与订阅".to_string()), label_en: Some("Read & Subscribe".to_string()) },
                    ]),
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "publish_interval".to_string(),
                    name_zh: Some("发布间隔 (ms)".to_string()),
                    name_en: Some("Publish Interval (ms)".to_string()),
                    attribute: ParamAttribute::Optional,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(500)),
                    valid: Some(ParamValid { min: Some(100), max: Some(65535), regex: None, length: None }),
                    ..Default::default()
                })
                .tag_regex(vec![
                    TagRegexEntry {
                        data_type: "int8".to_string(),
                        regex: r"^[0-9]+![0-9]+$|^[0-9]+![A-Za-z0-9_.]+$".to_string(),
                    },
                    TagRegexEntry {
                        data_type: "int16".to_string(),
                        regex: r"^[0-9]+![0-9]+$|^[0-9]+![A-Za-z0-9_.]+$".to_string(),
                    },
                    TagRegexEntry {
                        data_type: "int32".to_string(),
                        regex: r"^[0-9]+![0-9]+$|^[0-9]+![A-Za-z0-9_.]+$".to_string(),
                    },
                    TagRegexEntry {
                        data_type: "int64".to_string(),
                        regex: r"^[0-9]+![0-9]+$|^[0-9]+![A-Za-z0-9_.]+$".to_string(),
                    },
                    TagRegexEntry {
                        data_type: "uint8".to_string(),
                        regex: r"^[0-9]+![0-9]+$|^[0-9]+![A-Za-z0-9_.]+$".to_string(),
                    },
                    TagRegexEntry {
                        data_type: "uint16".to_string(),
                        regex: r"^[0-9]+![0-9]+$|^[0-9]+![A-Za-z0-9_.]+$".to_string(),
                    },
                    TagRegexEntry {
                        data_type: "uint32".to_string(),
                        regex: r"^[0-9]+![0-9]+$|^[0-9]+![A-Za-z0-9_.]+$".to_string(),
                    },
                    TagRegexEntry {
                        data_type: "uint64".to_string(),
                        regex: r"^[0-9]+![0-9]+$|^[0-9]+![A-Za-z0-9_.]+$".to_string(),
                    },
                    TagRegexEntry {
                        data_type: "float32".to_string(),
                        regex: r"^[0-9]+![0-9]+$|^[0-9]+![A-Za-z0-9_.]+$".to_string(),
                    },
                    TagRegexEntry {
                        data_type: "float64".to_string(),
                        regex: r"^[0-9]+![0-9]+$|^[0-9]+![A-Za-z0-9_.]+$".to_string(),
                    },
                    TagRegexEntry {
                        data_type: "bool".to_string(),
                        regex: r"^[0-9]+![0-9]+$|^[0-9]+![A-Za-z0-9_.]+$".to_string(),
                    },
                    TagRegexEntry {
                        data_type: "string".to_string(),
                        regex: r"^[0-9]+![0-9]+$|^[0-9]+![A-Za-z0-9_.]+$".to_string(),
                    },
                ]),
        )
    }

    fn tag_schema(&self) -> Option<TagSchema> {
        Some(TagSchema {
            data_types: Some(vec![
                "int8".to_string(),
                "int16".to_string(),
                "int32".to_string(),
                "int64".to_string(),
                "uint8".to_string(),
                "uint16".to_string(),
                "uint32".to_string(),
                "uint64".to_string(),
                "float32".to_string(),
                "float64".to_string(),
                "bool".to_string(),
                "string".to_string(),
            ]),
            address_format: Some("NS!NODEID，NS 为命名空间索引，NODEID 为数字或字符串，如 0!2258、2!Device1.Module1.Tag1".to_string()),
            address_format_zh: Some("NS!NODEID，NS 为命名空间索引，NODEID 为数字或字符串，如 0!2258、2!Device1.Module1.Tag1".to_string()),
            address_format_en: Some("NS!NODEID, NS=namespace index, NODEID=numeric or string, e.g. 0!2258, 2!Device1.Module1.Tag1".to_string()),
        })
    }

    async fn validate_tag(&self, _node_id: NodeId, tag: &Tag) -> PluginResult<()> {
        if tag.name.is_empty() {
            return Err(PluginError::tag_invalid("tag name required"));
        }
        parse_address(&tag.address)
            .ok_or_else(|| PluginError::tag_invalid("address format: NS!NODEID (e.g. 0!2258 or 2!Device1.Tag1)"))?;
        Ok(())
    }

    async fn open(&self, node_id: NodeId, config: PluginConfig) -> PluginResult<()> {
        let endpoint_url = config_str(&config, "endpoint_url", DEFAULT_ENDPOINT);
        let username = config.get("username").and_then(|v| v.as_str()).map(String::from);
        let password = config.get("password").and_then(|v| v.as_str()).map(String::from);
        let certificate = config.get("certificate").and_then(|v| v.as_str()).map(String::from);
        let key = config.get("key").and_then(|v| v.as_str()).map(String::from);
        let groups = Self::default_groups();
        let tags = groups
            .iter()
            .flat_map(|g| {
                vec![Tag {
                    id: TagId::new(),
                    name: "ServerTimestamp".to_string(),
                    address: "0!2258".to_string(),
                    attr: gateway_sdk::TagAttr::Read,
                    data_type: Some("uint32".to_string()),
                    description: Some("OPC UA 服务器时间戳".to_string()),
                    group_id: g.id,
                }]
            })
            .collect::<Vec<_>>();
        let mut state = self.state.write().await;
        state.insert(
            node_id,
            OpcuaState {
                endpoint_url,
                username,
                password,
                certificate,
                key,
                groups,
                tags,
            },
        );
        Ok(())
    }

    async fn close(&self, node_id: NodeId) -> PluginResult<()> {
        let mut state = self.state.write().await;
        state.remove(&node_id);
        Ok(())
    }

    async fn poll_group(
        &self,
        node_id: NodeId,
        _group_id: GroupId,
        tags: &[Tag],
    ) -> PluginResult<Vec<(TagId, DataValue)>> {
        let state = self.state.read().await;
        let s = state.get(&node_id).ok_or_else(|| PluginError::msg("node not open"))?;

        #[cfg(feature = "opcua-client")]
        {
            let endpoint_url = s.endpoint_url.clone();
            let username = s.username.clone();
            let password = s.password.clone();
            let tags_vec: Vec<(TagId, Tag)> = tags.iter().map(|t| (t.id, t.clone())).collect();
            let result = tokio::task::spawn_blocking(move || {
                opcua_read(&endpoint_url, username.as_deref(), password.as_deref(), &tags_vec)
            })
            .await
            .map_err(|e| PluginError::msg(format!("spawn_blocking: {}", e)))?;
            result
        }

        #[cfg(not(feature = "opcua-client"))]
        {
            let _ = (node_id, s);
            let mut out = Vec::with_capacity(tags.len());
            for tag in tags {
                let _ = parse_address(&tag.address);
                out.push((tag.id, DataValue::UInt32(0)));
            }
            Ok(out)
        }
    }

    async fn list_groups(&self, node_id: NodeId) -> PluginResult<Vec<Group>> {
        let state = self.state.read().await;
        let s = state.get(&node_id).ok_or_else(|| PluginError::msg("node not open"))?;
        Ok(s.groups.clone())
    }

    async fn list_tags(&self, node_id: NodeId, group_id: GroupId) -> PluginResult<Vec<Tag>> {
        let state = self.state.read().await;
        let s = state.get(&node_id).ok_or_else(|| PluginError::msg("node not open"))?;
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
        let s = state.get(&node_id).ok_or_else(|| PluginError::msg("node not open"))?;

        #[cfg(feature = "opcua-client")]
        {
            let endpoint_url = s.endpoint_url.clone();
            let username = s.username.clone();
            let password = s.password.clone();
            let values_vec: Vec<(Tag, DataValue)> = values.to_vec();
            let result = tokio::task::spawn_blocking(move || {
                opcua_write(&endpoint_url, username.as_deref(), password.as_deref(), &values_vec)
            })
            .await
            .map_err(|e| PluginError::msg(format!("spawn_blocking: {}", e)))?;
            result
        }

        #[cfg(not(feature = "opcua-client"))]
        {
            let _ = (node_id, s, values);
            Err(PluginError::not_supported("write requires opcua-client feature"))
        }
    }
}

#[cfg(feature = "opcua-client")]
fn opcua_read(
    endpoint_url: &str,
    _username: Option<&str>,
    _password: Option<&str>,
    tags: &[(TagId, Tag)],
) -> PluginResult<Vec<(TagId, DataValue)>> {
    use opcua::client::prelude::*;
    use std::collections::HashMap;

    let mut client = ClientBuilder::new()
        .application_name("iot-gateway-opcua")
        .application_uri("urn:iot-gateway:opcua")
        .create_sample_keypair(true)
        .trust_server_certs(true)
        .session_retry_policy(SessionRetryPolicy::default())
        .client()
        .map_err(|e| PluginError::msg(format!("client build: {}", e)))?;

    let endpoint = (
        endpoint_url,
        "None",
        MessageSecurityMode::None,
        UserTokenPolicy::anonymous(),
    );
    let (session, _event_loop) = client
        .connect_to_endpoint(endpoint, IdentityToken::Anonymous)
        .map_err(|e| PluginError::msg(format!("connect: {}", e)))?;

    let mut tag_order: Vec<(TagId, Tag)> = Vec::new();
    let mut nodes_to_read: Vec<ReadValueId> = Vec::new();
    for (_id, tag) in tags.iter() {
        let Some(parsed) = parse_address(&tag.address) else {
            continue;
        };
        let node_id = match &parsed.node_id {
            OpcNodeId::Numeric(n) => opcua::types::NodeId::new(parsed.namespace, *n),
            OpcNodeId::String(s) => opcua::types::NodeId::new(parsed.namespace, s.clone()),
        };
        tag_order.push((*_id, tag.clone()));
        nodes_to_read.push(ReadValueId {
            node_id,
            attribute_id: AttributeId::Value as u32,
            index_range: UAString::null(),
            data_encoding: QualifiedName::null(),
        });
    }

    if nodes_to_read.is_empty() {
        return Ok(tags
            .iter()
            .map(|(id, _)| (*id, DataValue::UInt32(0)))
            .collect());
    }

    let timestamps = TimestampsToReturn::Neither;
    let max_age = 0.0;
    let results = session
        .read(&nodes_to_read, timestamps, max_age)
        .map_err(|e| PluginError::msg(format!("read: {}", e)))?;

    let value_map: HashMap<TagId, DataValue> = tag_order
        .into_iter()
        .zip(results.into_iter())
        .map(|((tag_id, tag), dv)| {
            let v = dv
                .value
                .as_ref()
                .map(|v| opcua_value_to_data_value(v, tag.data_type.as_deref()))
                .unwrap_or(DataValue::UInt32(0));
            (tag_id, v)
        })
        .collect();

    let out: Vec<(TagId, DataValue)> = tags
        .iter()
        .map(|(id, _)| (*id, value_map.get(id).copied().unwrap_or(DataValue::UInt32(0))))
        .collect();
    Ok(out)
}

#[cfg(feature = "opcua-client")]
fn opcua_value_to_data_value(v: &opcua::types::Variant, data_type: Option<&str>) -> DataValue {
    use opcua::types::VariantType;
    match v {
        opcua::types::Variant::Boolean(b) => DataValue::Bool(*b),
        opcua::types::Variant::SByte(i) => DataValue::Int8(*i),
        opcua::types::Variant::Byte(u) => DataValue::UInt8(*u),
        opcua::types::Variant::Int16(i) => DataValue::Int16(*i),
        opcua::types::Variant::UInt16(u) => DataValue::UInt16(*u),
        opcua::types::Variant::Int32(i) => DataValue::Int32(*i),
        opcua::types::Variant::UInt32(u) => DataValue::UInt32(*u),
        opcua::types::Variant::Int64(i) => DataValue::Int64(*i),
        opcua::types::Variant::UInt64(u) => DataValue::UInt64(*u),
        opcua::types::Variant::Float(f) => DataValue::Float32(*f),
        opcua::types::Variant::Double(d) => DataValue::Float64(*d),
        opcua::types::Variant::String(s) => DataValue::String(s.value.clone().unwrap_or_default()),
        opcua::types::Variant::DateTime(dt) => {
            let t = dt.ticks_since_epoch();
            DataValue::UInt32(t as u32)
        }
        _ => DataValue::UInt32(0),
    }
}

#[cfg(feature = "opcua-client")]
fn opcua_write(
    endpoint_url: &str,
    username: Option<&str>,
    password: Option<&str>,
    values: &[(Tag, DataValue)],
) -> PluginResult<()> {
    use opcua::client::prelude::*;

    let mut client = ClientBuilder::new()
        .application_name("iot-gateway-opcua")
        .application_uri("urn:iot-gateway:opcua")
        .create_sample_keypair(true)
        .trust_server_certs(true)
        .session_retry_policy(SessionRetryPolicy::default())
        .client()
        .map_err(|e| PluginError::msg(format!("client build: {}", e)))?;

    let endpoint = (
        endpoint_url,
        "None",
        MessageSecurityMode::None,
        UserTokenPolicy::anonymous(),
    );
    let (session, _event_loop) = client
        .connect_to_endpoint(endpoint, IdentityToken::Anonymous)
        .map_err(|e| PluginError::msg(format!("connect: {}", e)))?;

    if let (Some(u), Some(p)) = (username, password) {
        let _ = session.activate_session(IdentityToken::UserName(u.to_string(), p.to_string()));
    }

    for (tag, value) in values {
        let Some(parsed) = parse_address(&tag.address) else {
            continue;
        };
        let node_id = match &parsed.node_id {
            OpcNodeId::Numeric(n) => opcua::types::NodeId::new(parsed.namespace, *n),
            OpcNodeId::String(s) => opcua::types::NodeId::new(parsed.namespace, s.clone()),
        };
        let variant = data_value_to_opcua_variant(value);
        let write_value = WriteValue {
            node_id,
            attribute_id: AttributeId::Value as u32,
            index_range: UAString::null(),
            value: DataValue::new(variant),
        };
        let _ = session.write(&[write_value]).map_err(|e| PluginError::msg(format!("write: {}", e)))?;
    }
    Ok(())
}

#[cfg(feature = "opcua-client")]
fn data_value_to_opcua_variant(v: &DataValue) -> opcua::types::Variant {
    use opcua::types::Variant;
    match v {
        DataValue::Bool(b) => Variant::Boolean(*b),
        DataValue::Int8(i) => Variant::SByte(*i),
        DataValue::UInt8(u) => Variant::Byte(*u),
        DataValue::Int16(i) => Variant::Int16(*i),
        DataValue::UInt16(u) => Variant::UInt16(*u),
        DataValue::Int32(i) => Variant::Int32(*i),
        DataValue::UInt32(u) => Variant::UInt32(*u),
        DataValue::Int64(i) => Variant::Int64(*i),
        DataValue::UInt64(u) => Variant::UInt64(*u),
        DataValue::Float32(f) => Variant::Float(*f),
        DataValue::Float64(d) => Variant::Double(*d),
        DataValue::String(s) => Variant::String(UAString::from(s.as_str())),
        DataValue::Bytes(b) => Variant::ByteString(ByteString::from(b.as_slice())),
    }
}
