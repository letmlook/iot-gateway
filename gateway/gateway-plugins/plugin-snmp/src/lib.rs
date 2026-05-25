//! SNMP 南向插件：支持 SNMP v1/v2c GET/SET 操作。
//!
//! 地址格式：点分 OID 字符串，如 `1.3.6.1.2.1.1.1.0`

#[cfg(feature = "ffi")]
mod ffi;

use gateway_sdk::{
    ConfigSchema, DataValue, Group, GroupId, NodeId, ParamSchema, ParamType, PluginConfig,
    PluginError, PluginMeta, PluginResult, SouthPlugin, Tag, TagId, TagSchema,
};
use gateway_sdk::log;
use gateway_sdk::types::PluginKind;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

/// SNMP 连接状态（per-node）
#[derive(Clone)]
pub struct SnmpNodeState {
    pub host: String,
    pub port: u16,
    pub community: String,
    pub timeout_ms: u64,
}

impl SnmpNodeState {
    /// Perform a single SNMP GET for one OID.
    fn get_one(&self, oid: &[u32]) -> Result<serde_json::Value, String> {
        let addr = format!("{}:{}", self.host, self.port);
        let timeout = Duration::from_millis(self.timeout_ms);

        let community_bytes = self.community.as_bytes();
        let mut session = snmp::SyncSession::new(&addr, community_bytes, Some(timeout), 0)
            .map_err(|e| format!("SNMP session: {:?}", e))?;

        let response = session.get(oid).map_err(|e| format!("SNMP get: {:?}", e))?;

        let mut vb = response.varbinds;
        if let Some((_oid, val)) = vb.next() {
            Ok(snmp_val_to_json(&val))
        } else {
            Ok(serde_json::Value::Null)
        }
    }

    /// Perform a single SNMP SET for one OID.
    fn set_one(&self, oid: &[u32], raw_data: &[u8]) -> Result<(), String> {
        let addr = format!("{}:{}", self.host, self.port);
        let timeout = Duration::from_millis(self.timeout_ms);

        let community_bytes = self.community.as_bytes();
        let mut session = snmp::SyncSession::new(&addr, community_bytes, Some(timeout), 0)
            .map_err(|e| format!("SNMP session: {:?}", e))?;

        let value = snmp::Value::OctetString(raw_data);
        session.set(&[(oid, value)]).map_err(|e| format!("SNMP set: {:?}", e))?;
        Ok(())
    }
}

/// Parse a dotted OID string like "1.3.6.1.2.1.1.1.0" into a Vec<u32>.
fn parse_oid(oid_str: &str) -> Option<Vec<u32>> {
    if oid_str.is_empty() {
        return None;
    }
    let parts: Vec<u32> = oid_str
        .split('.')
        .map(|p| p.parse::<u32>().ok())
        .collect::<Option<_>>()?;
    if parts.is_empty() {
        None
    } else {
        Some(parts)
    }
}

fn snmp_val_to_json(v: &snmp::Value) -> serde_json::Value {
    match v {
        snmp::Value::OctetString(s) => serde_json::json!(String::from_utf8_lossy(s).to_string()),
        snmp::Value::Integer(i) => serde_json::json!(*i),
        snmp::Value::Unsigned32(u) => serde_json::json!(*u),
        snmp::Value::Counter32(u) => serde_json::json!(*u),
        snmp::Value::Counter64(u) => serde_json::json!(*u),
        snmp::Value::Timeticks(u) => serde_json::json!(*u),
        snmp::Value::IpAddress(a) => {
            serde_json::json!(format!("{}.{}.{}.{}", a[0], a[1], a[2], a[3]))
        }
        snmp::Value::ObjectIdentifier(oid) => {
            serde_json::json!(format!("{}", oid))
        }
        snmp::Value::Null => serde_json::json!(null),
        _ => serde_json::json!(null),
    }
}

/// Encode a JSON value to SNMP octets for SET.
fn encode_for_snmp_set(val: &serde_json::Value) -> Result<(u8, Vec<u8>), String> {
    match val {
        serde_json::Value::Bool(b) => {
            let val: i64 = if *b { 1 } else { 0 };
            Ok((0x02, val.to_be_bytes().to_vec()))
        }
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok((0x02, i.to_be_bytes().to_vec()))
            } else if let Some(u) = n.as_u64() {
                Ok((0x02, (u as i64).to_be_bytes().to_vec()))
            } else if let Some(f) = n.as_f64() {
                Ok((0x02, (f as i64).to_be_bytes().to_vec()))
            } else {
                Err("unusable number".to_string())
            }
        }
        serde_json::Value::String(s) => {
            Ok((0x04, s.as_bytes().to_vec()))
        }
        serde_json::Value::Array(arr) => {
            let bytes: Vec<u8> = arr
                .iter()
                .filter_map(|x| x.as_u64().map(|n| n as u8))
                .collect();
            Ok((0x04, bytes))
        }
        _ => Err("unsupported JSON value type for SNMP SET".to_string()),
    }
}

/// SNMP 南向插件
pub struct SnmpPlugin {
    states: Arc<RwLock<HashMap<NodeId, SnmpNodeState>>>,
}

impl Default for SnmpPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl SnmpPlugin {
    pub fn new() -> Self {
        Self {
            states: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    fn json_to_data_value(v: serde_json::Value) -> DataValue {
        DataValue::from_json(v)
    }

    /// Get a cloned state for a node (for FFI layer)
    #[cfg(feature = "ffi")]
    pub async fn get_state(&self, node_id: &NodeId) -> Option<SnmpNodeState> {
        let states = self.states.read().await;
        states.get(node_id).cloned()
    }
}

#[async_trait::async_trait]
impl SouthPlugin for SnmpPlugin {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "snmp",
            kind: PluginKind::South,
            description: Some("SNMP (v1/v2c) network device monitoring — routers, switches, UPS, etc."),
            version: "0.1.0",
            name_zh: Some("SNMP"),
            name_en: Some("SNMP"),
            description_zh: Some("SNMP协议监控网络设备，支持路由器/交换机/UPS等"),
            description_en: Some("SNMP v1/v2c monitoring — routers, switches, UPS, environmental sensors"),
        }
    }

    fn config_schema(&self) -> Option<ConfigSchema> {
        let params = vec![
            ParamSchema {
                name: "host".to_string(),
                description: Some("SNMP agent IP address".to_string()),
                name_zh: Some("主机地址".to_string()),
                name_en: Some("Host".to_string()),
                description_zh: Some("SNMP代理IP地址".to_string()),
                description_en: Some("SNMP agent IP address".to_string()),
                attribute: gateway_sdk::schema::ParamAttribute::Required,
                ty: ParamType::String,
                default: Some(serde_json::json!("192.168.1.1")),
                valid: None,
                options: None,
                depends_on: None,
                depends_value: None,
                depends_values: None,
            },
            ParamSchema {
                name: "port".to_string(),
                description: Some("SNMP UDP port".to_string()),
                name_zh: Some("端口".to_string()),
                name_en: Some("Port".to_string()),
                description_zh: Some("SNMP UDP端口".to_string()),
                description_en: Some("SNMP UDP port".to_string()),
                attribute: gateway_sdk::schema::ParamAttribute::Optional,
                ty: ParamType::Int,
                default: Some(serde_json::json!(161)),
                valid: Some(gateway_sdk::schema::ParamValid {
                    min: Some(1),
                    max: Some(65535),
                    regex: None,
                    length: None,
                }),
                options: None,
                depends_on: None,
                depends_value: None,
                depends_values: None,
            },
            ParamSchema {
                name: "community".to_string(),
                description: Some("SNMP v2c community string".to_string()),
                name_zh: Some("Community".to_string()),
                name_en: Some("Community".to_string()),
                description_zh: Some("SNMP v2c community字符串".to_string()),
                description_en: Some("SNMP v2c community string".to_string()),
                attribute: gateway_sdk::schema::ParamAttribute::Optional,
                ty: ParamType::String,
                default: Some(serde_json::json!("public")),
                valid: None,
                options: None,
                depends_on: None,
                depends_value: None,
                depends_values: None,
            },
            ParamSchema {
                name: "timeout_ms".to_string(),
                description: Some("Request timeout in milliseconds".to_string()),
                name_zh: Some("超时时间".to_string()),
                name_en: Some("Timeout (ms)".to_string()),
                description_zh: Some("请求超时时间（毫秒）".to_string()),
                description_en: Some("Request timeout in milliseconds".to_string()),
                attribute: gateway_sdk::schema::ParamAttribute::Optional,
                ty: ParamType::Int,
                default: Some(serde_json::json!(3000)),
                valid: Some(gateway_sdk::schema::ParamValid {
                    min: Some(100),
                    max: Some(60000),
                    regex: None,
                    length: None,
                }),
                options: None,
                depends_on: None,
                depends_value: None,
                depends_values: None,
            },
        ];
        Some(ConfigSchema {
            params,
            tag_regex: None,
        })
    }

    fn tag_schema(&self) -> Option<TagSchema> {
        Some(TagSchema {
            data_types: Some(vec![
                "string".to_string(),
                "int64".to_string(),
                "uint64".to_string(),
                "float64".to_string(),
                "bool".to_string(),
            ]),
            address_format: Some("Dotted OID, e.g. 1.3.6.1.2.1.1.1.0".to_string()),
            address_format_zh: Some("点分OID，如 1.3.6.1.2.1.1.1.0".to_string()),
            address_format_en: Some("Dotted OID, e.g. 1.3.6.1.2.1.1.1.0".to_string()),
        })
    }

    async fn open(&self, node_id: NodeId, config: PluginConfig) -> PluginResult<()> {
        let host = config
            .get("host")
            .and_then(|v| v.as_str())
            .unwrap_or("192.168.1.1")
            .to_string();
        let port = config
            .get("port")
            .and_then(|v| v.as_u64())
            .unwrap_or(161) as u16;
        let community = config
            .get("community")
            .and_then(|v| v.as_str())
            .unwrap_or("public")
            .to_string();
        let timeout_ms = config
            .get("timeout_ms")
            .and_then(|v| v.as_u64())
            .unwrap_or(3000);

        let state = SnmpNodeState {
            host: host.clone(),
            port,
            community: community.clone(),
            timeout_ms,
        };

        // Ping via SNMP GET on sysDescr to verify connectivity
        let sys_descr_oid = [1, 3, 6, 1, 2, 1, 1, 1, 0];
        match state.get_one(&sys_descr_oid) {
            Ok(serde_json::Value::String(s)) => {
                log::info(node_id, format!("SNMP connected: {}:{} sysDescr={}", host, port, s));
            }
            Ok(_) => {
                log::info(node_id, format!("SNMP connected: {}:{} (ping ok)", host, port));
            }
            Err(e) => {
                log::warn(node_id, format!("SNMP connected but ping failed: {}", e));
            }
        }

        let mut states = self.states.write().await;
        states.insert(node_id, state);
        Ok(())
    }

    async fn close(&self, node_id: NodeId) -> PluginResult<()> {
        let mut states = self.states.write().await;
        states.remove(&node_id);
        log::info(node_id, "SNMP node closed");
        Ok(())
    }

    async fn validate_tag(&self, _node_id: NodeId, tag: &Tag) -> PluginResult<()> {
        if tag.address.is_empty() {
            return Err(PluginError::tag_invalid("address (OID) is required"));
        }
        if parse_oid(&tag.address).is_none() {
            return Err(PluginError::tag_invalid(format!("invalid OID: {}", tag.address)));
        }
        Ok(())
    }

    async fn poll_group(
        &self,
        node_id: NodeId,
        _group_id: GroupId,
        tags: &[Tag],
    ) -> PluginResult<Vec<(TagId, DataValue)>> {
        let state = {
            let states = self.states.read().await;
            states.get(&node_id).cloned()
        };

        let state = state.ok_or_else(|| PluginError::msg("node not open"))?;

        let mut results = Vec::with_capacity(tags.len());
        for tag in tags {
            let oid = match parse_oid(&tag.address) {
                Some(o) => o,
                None => {
                    results.push((tag.id, DataValue::String("invalid OID".to_string())));
                    continue;
                }
            };

            match state.get_one(&oid) {
                Ok(val) => {
                    results.push((tag.id, Self::json_to_data_value(val)));
                }
                Err(e) => {
                    results.push((tag.id, DataValue::String(format!("error: {}", e))));
                }
            }
        }

        Ok(results)
    }

    async fn write_tags(
        &self,
        node_id: NodeId,
        values: &[(Tag, DataValue)],
    ) -> PluginResult<()> {
        let state = {
            let states = self.states.read().await;
            states.get(&node_id).cloned()
        };

        let state = state.ok_or_else(|| PluginError::msg("node not open"))?;

        for (tag, value) in values {
            let oid = parse_oid(&tag.address).ok_or_else(|| {
                PluginError::tag_invalid(format!("invalid OID: {}", tag.address))
            })?;

            let json_val = serde_json::to_value(value)
                .map_err(|e| PluginError::msg(format!("serialize error: {}", e)))?;

            let (_type_tag, raw_data) = encode_for_snmp_set(&json_val)
                .map_err(|e| PluginError::msg(format!("encode error: {}", e)))?;

            state.set_one(&oid, &raw_data)
                .map_err(|e| PluginError::msg(format!("SNMP SET error: {}", e)))?;
        }

        Ok(())
    }

    async fn list_groups(&self, node_id: NodeId) -> PluginResult<Vec<Group>> {
        let states = self.states.read().await;
        if states.contains_key(&node_id) {
            Ok(vec![Group {
                id: GroupId::default(),
                name: "default".to_string(),
                interval_ms: 1000,
                description: Some("Default SNMP group".to_string()),
            }])
        } else {
            Ok(vec![])
        }
    }

    async fn list_tags(&self, _node_id: NodeId, _group_id: GroupId) -> PluginResult<Vec<Tag>> {
        // Tags are managed externally (in the store), not by the plugin
        Ok(vec![])
    }
}


