//! BACnet/IP South Plugin using tokio::net::UdpSocket for APDU encoding.
//!
//! Address format: {object_type}:{instance}:{property}
//!   Example: analogInput:0:presentValue, binaryInput:1:presentValue
//!
//! Config: { "host": "192.168.1.100", "port": 47808, "device_id": 123 }

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
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use tokio::net::UdpSocket;
use tokio::sync::RwLock;
use tokio::time::{timeout, Duration};

// BACnet BVLL function codes
const BVLL_ORIGINAL_UNICAST_NPDU: u8 = 0x04;
const BVLL_ORIGINAL_BROADCAST_NPDU: u8 = 0x0C;
const BVLL_FORWARDED_NPDU: u8 = 0x08;

// APDU constants
const APDU_MAX_APDU: u8 = 0xFA; // 1476 octets

// Confirmed service choices
const SERVICE_READ_PROPERTY: u8 = 0x0C;
const SERVICE_WHO_IS: u8 = 0x08;
const SERVICE_I_AM: u8 = 0x00;

/// BACnet object types
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

    fn to_u32(&self) -> u32 {
        match self {
            BacnetObjectType::AnalogInput => 0,
            BacnetObjectType::AnalogOutput => 1,
            BacnetObjectType::AnalogValue => 2,
            BacnetObjectType::BinaryInput => 3,
            BacnetObjectType::BinaryOutput => 4,
            BacnetObjectType::BinaryValue => 5,
            BacnetObjectType::MultiStateInput => 13,
            BacnetObjectType::MultiStateOutput => 14,
            BacnetObjectType::Unknown => 0x1F,
        }
    }
}

/// Property identifier mapping
fn property_name_to_id(property: &str) -> u32 {
    match property.to_lowercase().as_str() {
        "presentvalue" | "present_value" => 85,
        "description" => 28,
        "statusflags" | "status_flags" => 111,
        "eventstate" | "event_state" => 36,
        "outofservice" | "out_of_service" => 81,
        "units" => 117,
        "objectname" | "object_name" => 77,
        "objecttype" | "object_type" => 79,
        "objectidentifier" | "object_identifier" => 75,
        "datatype" | "data_type" => 107,
        _ => 85, // PresentValue default
    }
}

/// BACnet/IP plugin state
struct BacnetNodeState {
    host: String,
    port: u16,
    device_id: u32,
    connected: bool,
    groups: Vec<Group>,
    socket: Option<Arc<UdpSocket>>,
    discovered_addr: Option<SocketAddr>,
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
                description: Some("Default group".to_string()),
            }],
            socket: None,
            discovered_addr: None,
        }
    }
}

/// BACnet/IP South Plugin
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

    /// Perform device discovery using WhoIs broadcast
    async fn discover_device(&self, node_id: NodeId) -> PluginResult<()> {
        let (socket, host, port, device_id) = {
            let states = self.state.read().await;
            let s = states.get(&node_id).ok_or_else(|| PluginError::msg("node not open"))?;
            let socket = s.socket.as_ref().ok_or_else(|| PluginError::msg("socket not initialized"))?;
            (Arc::clone(socket), s.host.clone(), s.port, s.device_id)
        };

        // Build and send WhoIs broadcast
        let bvll_msg = build_whois_bvll(device_id, device_id);
        let broadcast_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::BROADCAST), port);
        if let Err(e) = socket.send_to(&bvll_msg, broadcast_addr).await {
            log::warn(node_id, format!("WhoIs broadcast send failed: {}", e));
        }

        // Wait for IAm responses
        let timeout_duration = Duration::from_secs(3);
        let mut buf = [0u8; 2048];
        let deadline = tokio::time::Instant::now() + timeout_duration;

        while tokio::time::Instant::now() < deadline {
            match tokio::time::timeout_at(deadline, socket.recv_from(&mut buf)).await {
                Ok(Ok((len, src))) => {
                    if let Some(response_device_id) = parse_iam_response(&buf[..len]) {
                        log::info(node_id, format!("received IAm from device {} at {}", response_device_id, src));
                        let mut states = self.state.write().await;
                        if let Some(s) = states.get_mut(&node_id) {
                            s.discovered_addr = Some(src);
                        }
                        if response_device_id == device_id {
                            return Ok(());
                        }
                    }
                }
                Ok(Err(e)) => {
                    log::warn(node_id, format!("recv error: {}", e));
                    break;
                }
                Err(_) => break,
            }
        }

        // No IAm received, use configured address
        log::warn(node_id, format!("no IAm received, using configured address {}:{}", host, port));

        let ip_parts: Vec<u8> = host.split('.').filter_map(|p| p.parse::<u8>().ok()).collect();
        if ip_parts.len() == 4 {
            let direct_addr = SocketAddr::new(
                IpAddr::V4(Ipv4Addr::new(ip_parts[0], ip_parts[1], ip_parts[2], ip_parts[3])),
                port,
            );
            let mut states = self.state.write().await;
            if let Some(s) = states.get_mut(&node_id) {
                s.discovered_addr = Some(direct_addr);
            }
        }

        Ok(())
    }
}

#[async_trait::async_trait]
impl SouthPlugin for BacnetPlugin {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "bacnet",
            kind: PluginKind::South,
            description: Some("BACnet/IP Building Automation Protocol"),
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
                    description: Some("BACnet device IP address".to_string()),
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
                    description: Some("BACnet default port 47808".to_string()),
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
                    description: Some("BACnet device instance ID".to_string()),
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
            address_format: Some("{object_type}:{instance}:{property}".to_string()),
            address_format_zh: Some("{对象类型}:{实例}:{属性}，例如 analogInput:0:presentValue".to_string()),
            address_format_en: Some("{object_type}:{instance}:{property}, e.g. analogInput:0:presentValue".to_string()),
        })
    }

    async fn validate_tag(&self, _node_id: NodeId, tag: &Tag) -> PluginResult<()> {
        if tag.name.is_empty() {
            return Err(PluginError::tag_invalid("tag name required"));
        }
        let parts: Vec<&str> = tag.address.split(':').collect();
        if parts.len() != 3 {
            return Err(PluginError::tag_invalid("BACnet address format: {object_type}:{instance}:{property}"));
        }
        let obj_type = BacnetObjectType::from_str(parts[0]);
        if obj_type == BacnetObjectType::Unknown {
            return Err(PluginError::tag_invalid(&format!("unknown BACnet object type: {}", parts[0])));
        }
        if parts[1].parse::<u32>().is_err() {
            return Err(PluginError::tag_invalid(&format!("invalid instance number: {}", parts[1])));
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

        log::info(node_id, format!("open bacnet: host={}, port={}, device_id={}", host, port, device_id));

        let state = BacnetNodeState::new(host.clone(), port, device_id);
        let mut states = self.state.write().await;
        states.insert(node_id, state);
        Ok(())
    }

    async fn close(&self, node_id: NodeId) -> PluginResult<()> {
        log::info(node_id, "close bacnet");
        let mut states = self.state.write().await;
        if let Some(s) = states.get_mut(&node_id) {
            s.socket = None;
        }
        states.remove(&node_id);
        Ok(())
    }

    async fn start(&self, node_id: NodeId) -> PluginResult<()> {
        log::info(node_id, "start bacnet");

        let port = {
            let mut states = self.state.write().await;
            let s = states.get_mut(&node_id).ok_or_else(|| PluginError::msg("node not open"))?;

            let bind_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0);
            let socket = UdpSocket::bind(bind_addr).await.map_err(|e| {
                PluginError::msg(format!("failed to bind UDP socket: {}", e))
            })?;

            socket.set_broadcast(true).map_err(|e| {
                PluginError::msg(format!("failed to set broadcast: {}", e))
            })?;

            s.socket = Some(Arc::new(socket));
            s.connected = true;

            s.port
        };

        // Perform WhoIs broadcast to discover device
        self.discover_device(node_id).await?;

        Ok(())
    }

    async fn stop(&self, node_id: NodeId) -> PluginResult<()> {
        log::info(node_id, "stop bacnet");
        let mut states = self.state.write().await;
        if let Some(s) = states.get_mut(&node_id) {
            s.connected = false;
            s.socket = None;
        }
        Ok(())
    }

    async fn poll_group(
        &self,
        node_id: NodeId,
        _group_id: GroupId,
        tags: &[Tag],
    ) -> PluginResult<Vec<(TagId, DataValue)>> {
        let (socket, discovered_addr) = {
            let states = self.state.read().await;
            let state = states.get(&node_id).ok_or_else(|| PluginError::msg("node not open"))?;

            if !state.connected {
                return Err(PluginError::msg("plugin not started"));
            }

            let socket = state.socket.as_ref().ok_or_else(|| PluginError::msg("socket not initialized"))?;
            (Arc::clone(socket), state.discovered_addr)
        };

        let addr = discovered_addr.ok_or_else(|| PluginError::msg("device not discovered"))?;

        let mut results = Vec::with_capacity(tags.len());
        let mut invoke_id: u8 = 1;

        for tag in tags {
            let parts: Vec<&str> = tag.address.split(':').collect();
            if parts.len() != 3 {
                results.push((tag.id, DataValue::Float32(0.0_f32)));
                continue;
            }

            let obj_type = BacnetObjectType::from_str(parts[0]);
            let instance: u32 = match parts[1].parse() {
                Ok(i) => i,
                Err(_) => {
                    results.push((tag.id, DataValue::Float32(0.0_f32)));
                    continue;
                }
            };

            let property_id = property_name_to_id(parts[2]);
            let object_type_u32 = obj_type.to_u32();

            // Build ReadProperty APDU
            let bvll_msg = build_read_property_bvll(invoke_id, object_type_u32, instance, property_id);

            // Send ReadProperty request
            if let Err(e) = socket.send_to(&bvll_msg, addr).await {
                log::warn(node_id, format!("send ReadProperty failed: {}", e));
                results.push((tag.id, default_for_type(&obj_type)));
                invoke_id = invoke_id.wrapping_add(1);
                continue;
            }

            // Wait for response with timeout
            let timeout_duration = Duration::from_secs(2);
            let mut buf = [0u8; 2048];

            match timeout(timeout_duration, socket.recv_from(&mut buf)).await {
                Ok(Ok((len, _src))) => {
                    let value = decode_read_property_response(&buf[..len], &obj_type);
                    results.push((tag.id, value));
                }
                Ok(Err(e)) => {
                    log::warn(node_id, format!("recv error: {}", e));
                    results.push((tag.id, default_for_type(&obj_type)));
                }
                Err(_) => {
                    log::warn(node_id, format!("ReadProperty timeout for {}", tag.address));
                    results.push((tag.id, default_for_type(&obj_type)));
                }
            }

            invoke_id = invoke_id.wrapping_add(1);
        }

        Ok(results)
    }

    async fn write_tags(&self, node_id: NodeId, values: &[(Tag, DataValue)]) -> PluginResult<()> {
        log::warn(node_id, format!("write_tags called with {} values (not implemented)", values.len()));
        Err(PluginError::not_supported("BACnet write not implemented"))
    }

    async fn list_groups(&self, node_id: NodeId) -> PluginResult<Vec<Group>> {
        let states = self.state.read().await;
        let state = states.get(&node_id).ok_or_else(|| PluginError::msg("node not open"))?;
        Ok(state.groups.clone())
    }

    async fn list_tags(&self, node_id: NodeId, _group_id: GroupId) -> PluginResult<Vec<Tag>> {
        let states = self.state.read().await;
        let _state = states.get(&node_id).ok_or_else(|| PluginError::msg("node not open"))?;
        Ok(vec![
            Tag {
                id: TagId::new(),
                name: "ai_0".to_string(),
                address: "analogInput:0:presentValue".to_string(),
                attr: gateway_sdk::TagAttr::Read,
                data_type: Some("float".to_string()),
                description: Some("Analog Input 0".to_string()),
                group_id: GroupId::new(),
            },
            Tag {
                id: TagId::new(),
                name: "bi_0".to_string(),
                address: "binaryInput:0:presentValue".to_string(),
                attr: gateway_sdk::TagAttr::Read,
                data_type: Some("bool".to_string()),
                description: Some("Binary Input 0".to_string()),
                group_id: GroupId::new(),
            },
        ])
    }
}

/// Build BVLL WhoIs broadcast message
fn build_whois_bvll(low_device_id: u32, high_device_id: u32) -> Vec<u8> {
    let mut bvll = Vec::new();

    // BVLL Header for Original-Broadcast-NPDU (0x0C)
    bvll.push(0x81);
    bvll.push(BVLL_ORIGINAL_BROADCAST_NPDU);

    // BVLL length placeholder (2 bytes) - index 2,3
    let length_idx = bvll.len();
    bvll.push(0x00);
    bvll.push(0x00);

    // NPDU Header
    bvll.push(0x01); // version
    bvll.push(0x20); // NPDU: network layer message, no routing

    // APDU - WhoIs unconfirmed service request
    bvll.push(0x10); // Unconfirmed Service Request
    bvll.push(0x00); // PDU flags
    bvll.push(SERVICE_WHO_IS); // Service choice: Who-Is

    // Device ID range (optional context tag)
    if !(low_device_id == 0 && high_device_id == 0x3FFFFF) {
        bvll.push(0x21); // Context tag 1 with length
        bvll.push(0x1C); // length = 28 bits (3 octets)
        bvll.push(((low_device_id >> 16) & 0xFF) as u8);
        bvll.push(((low_device_id >> 8) & 0xFF) as u8);
        bvll.push((low_device_id & 0xFF) as u8);
        bvll.push(((high_device_id >> 16) & 0xFF) as u8);
        bvll.push(((high_device_id >> 8) & 0xFF) as u8);
        bvll.push((high_device_id & 0xFF) as u8);
    }

    // Fill BVLL length
    let total_len = bvll.len();
    bvll[length_idx] = ((total_len >> 8) & 0xFF) as u8;
    bvll[length_idx + 1] = (total_len & 0xFF) as u8;

    bvll
}

/// Build BVLL ReadProperty unicast message
fn build_read_property_bvll(invoke_id: u8, object_type: u32, instance: u32, property_id: u32) -> Vec<u8> {
    let mut bvll = Vec::new();

    // BVLL Header for Original-Unicast-NPDU (0x04)
    bvll.push(0x81);
    bvll.push(BVLL_ORIGINAL_UNICAST_NPDU);

    // BVLL length placeholder (2 bytes)
    let length_idx = bvll.len();
    bvll.push(0x00);
    bvll.push(0x00);

    // NPDU Header
    bvll.push(0x01); // version
    bvll.push(0x00); // NPDU: no network layer message

    // APDU - Confirmed Service Request: ReadProperty
    bvll.push(0x10); // Confirmed Service Request
    bvll.push(0x00); // No segmentation
    bvll.push(APDU_MAX_APDU); // max APDU size 1476
    bvll.push(invoke_id); // invoke ID
    bvll.push(0x00); // sequence number
    bvll.push(0xFF); // window size

    // Service Choice: ReadProperty
    bvll.push(SERVICE_READ_PROPERTY);

    // Object Identifier - context tag 0
    bvll.push(0x0C); // context tag 0 + opening tag
    // BACnet Object Identifier: type in upper 10 bits, instance in lower 22 bits
    // type in bits 22-31 (10 bits), instance in bits 0-21 (22 bits)
    let object_id = (object_type << 22) | instance;
    bvll.push(((object_id >> 16) & 0xFF) as u8);
    bvll.push(((object_id >> 8) & 0xFF) as u8);
    bvll.push((object_id & 0xFF) as u8);

    // Property Identifier - context tag 1
    bvll.push(0x1C); // context tag 1 (opening) - use application tag for unsigned int
    // Actually for property ID (context-specific), use:
    // Tag 1 + length + value
    bvll.push(0xC1); // context tag 1
    // Property ID is a BACnetPropertyIdentifier (22-bit unsigned)
    // Encode as 3 bytes
    bvll.push(0x19); // length = 25 bits (3 octets)
    bvll.push(((property_id >> 16) & 0xFF) as u8);
    bvll.push(((property_id >> 8) & 0xFF) as u8);
    bvll.push((property_id & 0xFF) as u8);

    // Property Array Index - context tag 2, special value (no index)
    bvll.push(0xC2); // context tag 2
    bvll.push(0x00); // length 0 = no array index

    // Fill BVLL length
    let total_len = bvll.len();
    bvll[length_idx] = ((total_len >> 8) & 0xFF) as u8;
    bvll[length_idx + 1] = (total_len & 0xFF) as u8;

    bvll
}

/// Parse IAm response to extract device ID
fn parse_iam_response(buf: &[u8]) -> Option<u32> {
    if buf.len() < 10 {
        return None;
    }

    // Check BVLL header
    if buf[0] != 0x81 {
        return None;
    }

    let npdu_start = if buf[1] == BVLL_FORWARDED_NPDU {
        if buf.len() < 8 {
            return None;
        }
        let src_len = buf[6] as usize + 7;
        if buf.len() < src_len {
            return None;
        }
        src_len
    } else if buf[1] == BVLL_ORIGINAL_UNICAST_NPDU || buf[1] == BVLL_ORIGINAL_BROADCAST_NPDU {
        4
    } else {
        return None;
    };

    let apdu = &buf[npdu_start..];
    if apdu.len() < 4 {
        return None;
    }

    // I-Am is unconfirmed service (0x10), service choice = 0x00
    if apdu[0] & 0xF0 != 0x10 {
        return None;
    }
    if apdu[2] != SERVICE_I_AM {
        return None;
    }

    // Device ID is in the object identifier field
    let mut pos = 3;
    while pos < apdu.len() && (apdu[pos] & 0xC0) == 0xC0 {
        let tag = apdu[pos] & 0x3F;
        pos += 1;
        if tag == 0x0C || tag == 0x3C {
            continue;
        }
        if pos < apdu.len() {
            let len = apdu[pos] & 0x3F;
            pos += 1 + len as usize;
        }
    }

    if pos + 4 > apdu.len() {
        return None;
    }

    // Parse object identifier (device object type=8, instance=lower 22 bits)
    let obj_id = ((apdu[pos] as u32) << 24)
        | ((apdu[pos + 1] as u32) << 16)
        | ((apdu[pos + 2] as u32) << 8)
        | (apdu[pos + 3] as u32);

    // Device instance is in the lower 22 bits of the object identifier
    // Object type is upper 10 bits
    let device_instance = obj_id & 0x003FFFFF;
    Some(device_instance)
}

/// Decode ReadProperty response to DataValue
fn decode_read_property_response(buf: &[u8], obj_type: &BacnetObjectType) -> DataValue {
    if buf.len() < 6 {
        return default_for_type(obj_type);
    }

    let npdu_start = if buf[1] == BVLL_FORWARDED_NPDU {
        if buf.len() < 8 {
            return default_for_type(obj_type);
        }
        let src_len = buf[6] as usize + 7;
        if buf.len() < src_len {
            return default_for_type(obj_type);
        }
        src_len
    } else if buf[1] == BVLL_ORIGINAL_UNICAST_NPDU || buf[1] == BVLL_ORIGINAL_BROADCAST_NPDU {
        4
    } else {
        return default_for_type(obj_type);
    };

    let apdu = &buf[npdu_start..];
    if apdu.len() < 4 {
        return default_for_type(obj_type);
    }

    // Look for the property value (context tag 3 for ReadProperty ACK)
    let mut pos = 0;
    while pos < apdu.len() {
        let tag = apdu[pos];

        if (tag & 0xC0) == 0x40 {
            // Context tag
            let context_tag = tag & 0x0F;
            pos += 1;

            if context_tag == 3 {
                // Found property value
                if pos >= apdu.len() {
                    return default_for_type(obj_type);
                }

                let len = apdu[pos] as usize;
                pos += 1;

                if pos + len > apdu.len() {
                    return default_for_type(obj_type);
                }

                let value_bytes = &apdu[pos..pos + len];
                return decode_bacnet_data(value_bytes, obj_type);
            } else {
                // Skip other context tags
                if pos < apdu.len() {
                    let len = (apdu[pos] & 0x3F) as usize;
                    pos += 1 + len;
                }
            }
        } else if tag == 0x3C {
            // Closing tag
            pos += 1;
        } else {
            pos += 1;
        }
    }

    // Fallback: try to parse application-tagged values directly
    for i in 0..apdu.len() {
        if (apdu[i] & 0xC0) == 0x00 {
            let app_tag = apdu[i] & 0x3F;
            if i + 1 < apdu.len() {
                let len = if app_tag == 4 || app_tag == 3 || app_tag == 5 || app_tag == 6 {
                    4
                } else {
                    (apdu[i + 1] & 0x3F) as usize
                };

                if i + 1 + len <= apdu.len() && len > 0 {
                    let value = decode_application_tag(app_tag, &apdu[i + 1..i + 1 + len]);
                    if value != DataValue::Float32(0.0_f32) || app_tag == 0 {
                        return value;
                    }
                }
            }
        }
    }

    default_for_type(obj_type)
}

/// Decode BACnet application tag to DataValue
fn decode_application_tag(tag_number: u8, data: &[u8]) -> DataValue {
    if data.is_empty() {
        return DataValue::Float32(0.0_f32);
    }

    match tag_number {
        0 => DataValue::Float32(0.0_f32), // Null
        1 => DataValue::Bool(data[0] != 0), // Boolean
        2 => {
            // Unsigned integer
            let mut val: u32 = 0;
            for &b in data.iter().take(4) {
                val = (val << 8) | b as u32;
            }
            DataValue::UInt32(val)
        }
        3 => {
            // Signed integer
            let mut val: i32 = 0;
            for &b in data.iter().take(4) {
                val = (val << 8) | b as i32;
            }
            DataValue::Int32(val)
        }
        4 => {
            // Real (float32) - 4 bytes
            if data.len() >= 4 {
                let bits = ((data[0] as u32) << 24)
                    | ((data[1] as u32) << 16)
                    | ((data[2] as u32) << 8)
                    | (data[3] as u32);
                DataValue::Float32(f32::from_bits(bits))
            } else {
                DataValue::Float32(0.0_f32)
            }
        }
        5 => DataValue::Float64(0.0_f64), // Double - not commonly used
        6 => DataValue::String("octet".to_string()), // Octet string
        7 => {
            // Character string
            if data.len() > 1 {
                if let Ok(s) = std::str::from_utf8(&data[1..]) {
                    return DataValue::String(s.to_string());
                }
            }
            DataValue::String(String::new())
        }
        _ => DataValue::Float32(0.0_f32),
    }
}

/// Decode BACnet data bytes to DataValue based on object type
fn decode_bacnet_data(data: &[u8], obj_type: &BacnetObjectType) -> DataValue {
    if data.is_empty() {
        return default_for_type(obj_type);
    }

    let first_byte = data[0];

    if (first_byte & 0xC0) == 0x00 {
        // Application tag
        let tag_number = first_byte & 0x3F;
        return decode_application_tag(tag_number, data);
    }

    // Try based on object type
    match obj_type {
        BacnetObjectType::AnalogInput | BacnetObjectType::AnalogOutput | BacnetObjectType::AnalogValue => {
            // Try to decode as real
            if data.len() >= 4 {
                let bits = ((data[0] as u32) << 24)
                    | ((data[1] as u32) << 16)
                    | ((data[2] as u32) << 8)
                    | (data[3] as u32);
                DataValue::Float32(f32::from_bits(bits))
            } else {
                DataValue::Float32(0.0_f32)
            }
        }
        BacnetObjectType::BinaryInput | BacnetObjectType::BinaryOutput | BacnetObjectType::BinaryValue => {
            if !data.is_empty() {
                DataValue::Bool(data[0] != 0)
            } else {
                DataValue::Bool(false)
            }
        }
        _ => DataValue::Float32(0.0_f32),
    }
}

/// Return default value for object type
fn default_for_type(obj_type: &BacnetObjectType) -> DataValue {
    match obj_type {
        BacnetObjectType::AnalogInput | BacnetObjectType::AnalogOutput | BacnetObjectType::AnalogValue => {
            DataValue::Float32(0.0_f32)
        }
        BacnetObjectType::BinaryInput | BacnetObjectType::BinaryOutput | BacnetObjectType::BinaryValue => {
            DataValue::Bool(false)
        }
        _ => DataValue::Float32(0.0_f32),
    }
}
