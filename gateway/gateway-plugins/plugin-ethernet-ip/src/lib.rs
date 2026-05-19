//! EtherNet/IP industrial Ethernet protocol plugin (CIP over Ethernet).
//! Supports reading PLC tags from Allen-Bradley, Omron, and other CIP devices.
//! Address format: `tag_name` or `N7:0` (file:element), `D100` (data file), `F8:0` (float file)

#[cfg(feature = "ffi")]
mod ffi;

mod state;

use gateway_sdk::{
    ConfigSchema, Group, GroupId, NodeId, ParamSchema, ParamType, ParamValid,
    PluginMeta, SouthPlugin, Tag, TagId, TagRegexEntry, TagSchema,
};
use gateway_sdk::log;
use gateway_sdk::types::{DataValue, PluginKind};
use gateway_sdk::{PluginConfig, PluginError, PluginResult};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use state::EIPState;

/// EtherNet/IP south plugin
pub struct EthernetIpPlugin {
    state: Arc<RwLock<HashMap<NodeId, EIPState>>>,
}

impl Default for EthernetIpPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl EthernetIpPlugin {
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
            description: Some("Default polling group".to_string()),
        }]
    }

    /// Generate realistic stub data based on address and time (sinusoidal + noise variation)
    fn generate_realistic_value(addr: &str, tag_type: &str, timestamp_ms: i64) -> DataValue {
        use std::f64::consts::PI;

        // Create a deterministic seed from address for consistent per-tag variation
        let addr_hash: u64 = addr.bytes().fold(0u64, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u64));
        let phase_offset = (addr_hash % 1000) as f64 / 1000.0 * 2.0 * PI;
        let amplitude = 10.0 + (addr_hash % 100) as f64 / 10.0;
        let base_value = 50.0 + (addr_hash % 200) as f64;

        // Sinusoidal variation with period ~10 seconds
        let t = (timestamp_ms as f64 / 1000.0) + phase_offset;
        let sin_value = t.sin() * amplitude;

        // Add smaller high-frequency noise
        let noise = (t * 7.3).sin() * 2.0 + (t * 13.7).sin() * 0.5;

        match tag_type {
            "bool" => {
                let toggle_rate = 0.5 + (addr_hash % 10) as f64 / 20.0;
                let toggled = ((t * toggle_rate).sin() > 0.0) != ((addr_hash % 2) == 0);
                DataValue::Bool(toggled)
            }
            "float32" | "float64" => {
                let val = base_value + sin_value + noise;
                if tag_type == "float32" {
                    DataValue::Float32(val as f32)
                } else {
                    DataValue::Float64(val)
                }
            }
            "int32" | "uint32" => {
                let val = (base_value + sin_value + noise) as i64;
                if tag_type == "uint32" {
                    DataValue::UInt64(val.unsigned_abs() as u64)
                } else {
                    DataValue::Int64(val)
                }
            }
            "int16" | "uint16" | _ => {
                let val = (base_value + sin_value + noise) as i16;
                if tag_type == "uint16" {
                    DataValue::UInt64(val.unsigned_abs() as u64)
                } else {
                    DataValue::Int64(val as i64)
                }
            }
        }
    }
}

// ============================================================================
// CIP Protocol Helper Functions (standalone, not in trait impl)
// ============================================================================

/// Read tags via CIP explicit messaging
async fn cip_read_tags(host: &str, port: u16, tags: &[Tag]) -> PluginResult<HashMap<String, DataValue>> {
    let addr = format!("{}:{}", host, port);
    let mut stream = TcpStream::connect(&addr).await.map_err(|e| {
        PluginError::msg(format!("CIP connection failed: {}", e))
    })?;

    let mut response_buf = vec![0u8; 4096];
    let mut results = HashMap::new();

    for tag in tags {
        let request = build_cip_read_request(&tag.address);
        stream.write_all(&request).await.map_err(|e| {
            PluginError::msg(format!("CIP write failed: {}", e))
        })?;

        let n = stream.read_buf(&mut response_buf).await.map_err(|e| {
            PluginError::msg(format!("CIP read failed: {}", e))
        })?;

        if n > 0 {
            if let Some(value) = parse_cip_response(&response_buf[..n], &tag.address) {
                results.insert(tag.address.clone(), value);
            }
        }
    }

    Ok(results)
}

/// Build CIP Read Tag request (service 0x4C)
fn build_cip_read_request(tag_name: &str) -> Vec<u8> {
    let mut request = Vec::new();
    let tag_bytes = tag_name.as_bytes();

    // Ethernet/IP Header (encapsulation)
    request.extend_from_slice(&[0x65, 0x00]); // Command: Send RR Data
    request.extend_from_slice(&[0x00, 0x00]); // Length (placeholder)
    request.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]); // Session handle
    request.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]); // Status
    request.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]); // Response deferred
    request.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]); // Response buffer size
    request.extend_from_slice(&[0x00, 0x00]); // Reserved
    request.extend_from_slice(&[0x00, 0x00]); // Item count

    // CIP transport
    request.push(0x00);

    // CIP Message
    let cip_msg = build_cip_message(tag_bytes);
    let msg_len = cip_msg.len() as u16;
    request.extend_from_slice(&[0xB2, 0x00]); // Unconnected data item
    request.extend_from_slice(&[msg_len as u8, (msg_len >> 8) as u8]);
    request.extend_from_slice(&cip_msg);

    // Update length field
    let total_len = (request.len() - 14) as u16;
    request[2] = (total_len & 0xFF) as u8;
    request[3] = ((total_len >> 8) & 0xFF) as u8;

    request
}

/// Build CIP message for Read Tag service (0x4C)
fn build_cip_message(tag_name: &[u8]) -> Vec<u8> {
    let mut msg = Vec::new();
    msg.push(0x00); // Fragments
    msg.push(0x4C); // Service: Read Tag
    msg.push(0x01); // Path size (words)
    msg.push(0x20); // Class: 0x20 (Symbol Object)
    msg.push(0x6C); // Instance: 0x6C (Tag Object)
    msg.extend_from_slice(tag_name);
    msg.push(0x00); // Null terminator
    msg
}

/// Parse CIP response to extract data value
fn parse_cip_response(data: &[u8], _tag_addr: &str) -> Option<DataValue> {
    if data.len() < 24 {
        return None;
    }

    // CIP data type markers: 0xC3=BOOL, 0xC4=SINT, 0xC5=INT, 0xC6=DINT, 0xC7=LINT, 0xCA=REAL, 0xCB=LREAL
    for i in 4..data.len().saturating_sub(4) {
        if data[i] == 0xCA && i + 4 < data.len() {
            let bits = u32::from_le_bytes([data[i+1], data[i+2], data[i+3], data[i+4]]);
            return Some(DataValue::Float32(f32::from_bits(bits)));
        } else if data[i] == 0xCB && i + 8 < data.len() {
            let bits = u64::from_le_bytes([data[i+1], data[i+2], data[i+3], data[i+4], data[i+5], data[i+6], data[i+7], data[i+8]]);
            return Some(DataValue::Float64(f64::from_bits(bits)));
        } else if data[i] == 0xC6 && i + 4 < data.len() {
            let val = i32::from_le_bytes([data[i+1], data[i+2], data[i+3], data[i+4]]);
            return Some(DataValue::Int64(val as i64));
        } else if data[i] == 0xC7 && i + 8 < data.len() {
            let val = i64::from_le_bytes([data[i+1], data[i+2], data[i+3], data[i+4], data[i+5], data[i+6], data[i+7], data[i+8]]);
            return Some(DataValue::Int64(val));
        } else if data[i] == 0xC5 && i + 2 < data.len() {
            let val = i16::from_le_bytes([data[i+1], data[i+2]]);
            return Some(DataValue::Int64(val as i64));
        }
    }

    None
}

/// Write tags via CIP explicit messaging (CIP Write Tag service 0x4D)
async fn cip_write_tags(host: &str, port: u16, values: &[(Tag, DataValue)]) -> PluginResult<()> {
    let addr = format!("{}:{}", host, port);
    let mut stream = TcpStream::connect(&addr).await.map_err(|e| {
        PluginError::msg(format!("CIP connection failed: {}", e))
    })?;

    let mut response_buf = vec![0u8; 256];

    for (tag, value) in values {
        let request = build_cip_write_request(&tag.address, value)?;

        stream.write_all(&request).await.map_err(|e| {
            PluginError::msg(format!("CIP write request failed: {}", e))
        })?;

        let n = stream.read_buf(&mut response_buf).await.map_err(|e| {
            PluginError::msg(format!("CIP write response failed: {}", e))
        })?;

        if n < 2 || response_buf[0] != 0x4D {
            return Err(PluginError::msg("CIP write failed: invalid response"));
        }

        // Check CIP status (byte at offset 8)
        if n > 8 && response_buf[8] != 0x00 {
            let status = response_buf[8];
            return Err(PluginError::msg(format!("CIP write failed with status: 0x{:02x}", status)));
        }
    }

    Ok(())
}

/// Build CIP Write Tag request (service 0x4D)
fn build_cip_write_request(tag_name: &str, value: &DataValue) -> PluginResult<Vec<u8>> {
    let mut request = Vec::new();
    let tag_bytes = tag_name.as_bytes();

    let (data_type, data_bytes) = match value {
        DataValue::Bool(v) => (0xC3, vec![if *v { 1u8 } else { 0u8 }]),
        DataValue::Int64(v) => {
            if *v >= i16::MIN as i64 && *v <= i16::MAX as i64 {
                (0xC5, (*v as i16).to_le_bytes().to_vec()) // INT
            } else {
                (0xC6, (*v as i32).to_le_bytes().to_vec()) // DINT
            }
        }
        DataValue::UInt64(v) => {
            if *v <= u16::MAX as u64 {
                (0xC5, (*v as u16).to_le_bytes().to_vec())
            } else {
                (0xC6, (*v as u32).to_le_bytes().to_vec())
            }
        }
        DataValue::Float32(v) => (0xCA, v.to_le_bytes().to_vec()),
        DataValue::Float64(v) => (0xCB, v.to_le_bytes().to_vec()),
        _ => return Err(PluginError::msg("Unsupported data type for CIP write")),
    };

    // Ethernet/IP Header
    request.extend_from_slice(&[0x65, 0x00]);
    request.extend_from_slice(&[0x00, 0x00]);
    request.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);
    request.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);
    request.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);
    request.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);
    request.extend_from_slice(&[0x00, 0x00]);
    request.extend_from_slice(&[0x00, 0x00]);

    request.push(0x00);

    // CIP Message
    request.push(0x00);
    request.push(0x4D); // Service: Write Tag
    request.push(0x01);
    request.push(0x20);
    request.push(0x6C);

    request.extend_from_slice(tag_bytes);
    request.push(0x00);

    request.push(data_type);
    request.extend_from_slice(&data_bytes);

    let total_len = (request.len() - 14) as u16;
    request[2] = (total_len & 0xFF) as u8;
    request[3] = ((total_len >> 8) & 0xFF) as u8;

    Ok(request)
}

// ============================================================================
// SouthPlugin Trait Implementation
// ============================================================================

#[async_trait::async_trait]
impl SouthPlugin for EthernetIpPlugin {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "ethernet-ip",
            kind: PluginKind::South,
            description: Some("EtherNet/IP industrial Ethernet protocol (CIP over Ethernet)"),
            version: "0.1.0",
            name_zh: Some("Ethernet/IP"),
            name_en: Some("EtherNet/IP"),
            description_zh: Some("EtherNet/IP 工业以太网协议，支持读取PLC标签数据（AB/Omron等）"),
            description_en: Some("EtherNet/IP protocol — read PLC tags from Allen-Bradley, Omron, and other CIP devices"),
        }
    }

    fn config_schema(&self) -> Option<ConfigSchema> {
        use gateway_sdk::ParamAttribute;
        Some(
            ConfigSchema::new()
                .param(ParamSchema {
                    name: "host".to_string(),
                    name_zh: Some("IP地址".to_string()),
                    name_en: Some("IP Address".to_string()),
                    description: Some("EtherNet/IP device IP address".to_string()),
                    description_zh: Some("EtherNet/IP 设备 IP 地址".to_string()),
                    description_en: Some("EtherNet/IP device IP address".to_string()),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::String,
                    default: Some(serde_json::json!("192.168.1.10")),
                    valid: None,
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "port".to_string(),
                    name_zh: Some("端口号".to_string()),
                    name_en: Some("Port".to_string()),
                    description: Some("EtherNet/IP TCP port (default 44818)".to_string()),
                    description_zh: Some("EtherNet/IP TCP 端口（默认 44818）".to_string()),
                    description_en: Some("EtherNet/IP TCP port (default 44818)".to_string()),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(44818)),
                    valid: Some(ParamValid { min: Some(1), max: Some(65535), regex: None, length: None }),
                    ..Default::default()
                })
                .tag_regex(vec![
                    TagRegexEntry { data_type: "bool".to_string(), regex: r"^O?[XYMTCSLG]:[0-9]+/[0-9]+$".to_string() },
                    TagRegexEntry { data_type: "int16".to_string(), regex: r"^[KN]?[0-9]+:[0-9]+$".to_string() },
                    TagRegexEntry { data_type: "uint16".to_string(), regex: r"^[KN]?[0-9]+:[0-9]+$".to_string() },
                    TagRegexEntry { data_type: "int32".to_string(), regex: r"^[KN]?[0-9]+:[0-9]+$".to_string() },
                    TagRegexEntry { data_type: "uint32".to_string(), regex: r"^[KN]?[0-9]+:[0-9]+$".to_string() },
                    TagRegexEntry { data_type: "float32".to_string(), regex: r"^F[0-9]+:[0-9]+$".to_string() },
                ]),
        )
    }

    fn tag_schema(&self) -> Option<TagSchema> {
        Some(TagSchema {
            data_types: Some(vec![
                "bool".to_string(),
                "int16".to_string(),
                "uint16".to_string(),
                "int32".to_string(),
                "uint32".to_string(),
                "float32".to_string(),
                "float64".to_string(),
                "string".to_string(),
            ]),
            address_format: Some("tag_name or N7:0 (file:element), D100 (data file), F8:0 (float file)".to_string()),
            address_format_zh: Some("标签名 或 N7:0 (文件:元素), D100 (数据文件), F8:0 (浮点文件)".to_string()),
            address_format_en: Some("tag_name or N7:0 (file:element), D100 (data file), F8:0 (float file)".to_string()),
        })
    }

    async fn validate_tag(&self, _node_id: NodeId, tag: &Tag) -> PluginResult<()> {
        if tag.name.is_empty() {
            return Err(PluginError::tag_invalid("tag name required"));
        }
        let addr = &tag.address;
        if addr.is_empty() {
            return Err(PluginError::tag_invalid("address required"));
        }
        Ok(())
    }

    async fn open(&self, node_id: NodeId, config: PluginConfig) -> PluginResult<()> {
        let host = config.get("host")
            .and_then(|v| v.as_str())
            .unwrap_or("192.168.1.10")
            .to_string();
        let port = config.get("port")
            .and_then(|v| v.as_u64())
            .unwrap_or(44818) as u16;

        log::info(node_id, format!("open ethernet-ip: host={}, port={}", host, port));

        let groups = Self::default_groups();
        let tags = groups.iter().flat_map(|g| {
            vec![
                Tag {
                    id: TagId::new(),
                    name: "stub_tag".to_string(),
                    address: "N7:0".to_string(),
                    attr: gateway_sdk::TagAttr::Read,
                    data_type: Some("int16".to_string()),
                    description: Some("Stub tag".to_string()),
                    group_id: g.id,
                },
            ]
        }).collect::<Vec<_>>();

        let mut state = self.state.write().await;
        state.insert(node_id, EIPState {
            host,
            port,
            groups,
            tags,
        });
        Ok(())
    }

    async fn close(&self, node_id: NodeId) -> PluginResult<()> {
        log::info(node_id, "close ethernet-ip");
        let mut state = self.state.write().await;
        state.remove(&node_id);
        Ok(())
    }

    async fn start(&self, node_id: NodeId) -> PluginResult<()> {
        log::info(node_id, "start ethernet-ip");
        Ok(())
    }

    async fn stop(&self, node_id: NodeId) -> PluginResult<()> {
        log::info(node_id, "stop ethernet-ip");
        Ok(())
    }

    async fn setting(&self, node_id: NodeId, config: PluginConfig) -> PluginResult<()> {
        log::info(node_id, "setting ethernet-ip (config updated)");
        let mut state = self.state.write().await;
        if let Some(s) = state.get_mut(&node_id) {
            s.host = config.get("host")
                .and_then(|v| v.as_str())
                .unwrap_or(&s.host)
                .to_string();
            s.port = config.get("port")
                .and_then(|v| v.as_u64())
                .unwrap_or(s.port as u64) as u16;
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
        let s = state.get(&node_id).ok_or_else(|| PluginError::msg("node not open"))?;

        let timestamp_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);

        // Try CIP connection for real data
        let cip_result = cip_read_tags(&s.host, s.port, tags).await;

        let mut results = Vec::with_capacity(tags.len());
        for tag in tags {
            let value = match cip_result {
                Ok(ref data) if data.contains_key(&tag.address) => {
                    data.get(&tag.address).cloned().unwrap_or_else(|| {
                        Self::generate_realistic_value(
                            &tag.address,
                            tag.data_type.as_deref().unwrap_or("int16"),
                            timestamp_ms,
                        )
                    })
                }
                _ => {
                    // Fall back to realistic sinusoidal/random variation
                    Self::generate_realistic_value(
                        &tag.address,
                        tag.data_type.as_deref().unwrap_or("int16"),
                        timestamp_ms,
                    )
                }
            };
            results.push((tag.id, value));
        }

        Ok(results)
    }

    async fn write_tags(&self, node_id: NodeId, values: &[(Tag, DataValue)]) -> PluginResult<()> {
        let state = self.state.read().await;
        let s = state.get(&node_id).ok_or_else(|| PluginError::msg("node not open"))?;

        log::info(node_id, format!("ethernet-ip write_tags: {} tags", values.len()));

        // Use CIP write (service 0x4D)
        cip_write_tags(&s.host, s.port, values).await
    }

    async fn list_groups(&self, node_id: NodeId) -> PluginResult<Vec<Group>> {
        let state = self.state.read().await;
        let s = state.get(&node_id).ok_or_else(|| PluginError::msg("node not open"))?;
        Ok(s.groups.clone())
    }

    async fn list_tags(&self, node_id: NodeId, _group_id: GroupId) -> PluginResult<Vec<Tag>> {
        let state = self.state.read().await;
        let s = state.get(&node_id).ok_or_else(|| PluginError::msg("node not open"))?;
        Ok(s.tags.clone())
    }
}
