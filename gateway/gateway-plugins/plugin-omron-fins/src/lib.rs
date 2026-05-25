//! Omron FINS over TCP south plugin.
//! Implements SouthPlugin trait for CP/CJ/NJ series PLCs.

#[cfg(feature = "ffi")]
mod ffi;

use gateway_sdk::{
    ConfigSchema, DataValue, Group, GroupId, NodeId, ParamAttribute, ParamSchema, ParamType,
    PluginConfig, PluginError, PluginMeta, PluginResult, Tag, TagAttr, TagId, TagSchema,
};
use gateway_sdk::log;
use gateway_sdk::types::PluginKind;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock as AsyncRwLock;

// ---------------------------------------------------------------------------
// Area codes
// ---------------------------------------------------------------------------
/// Memory area codes for FINS protocol.
const AREA_DM: u8 = 0x82;
const AREA_CIO: u8 = 0x30;
const AREA_W: u8 = 0x31;
const AREA_HR: u8 = 0x02;
const AREA_AR: u8 = 0x03;

// ---------------------------------------------------------------------------
// FINS TCP state per node
// ---------------------------------------------------------------------------
pub struct OmronFinsState {
    pub host: String,
    pub port: u16,
    pub local_net: u8,
    pub local_unit: u8,
    pub local_node: u8,
    pub remote_net: u8,
    pub remote_unit: u8,
    pub remote_node: u8,
    pub socket: Option<TcpStream>,
    pub connected: bool,
    /// SID increments on each request (FINS over TCP only)
    pub sid: u8,
}

impl OmronFinsState {
    /// Connect TCP socket to PLC.
    fn connect(&mut self) -> std::io::Result<()> {
        let addr = format!("{}:{}", self.host, self.port);
        let sock = TcpStream::connect_timeout(
            &addr.parse().unwrap(),
            Duration::from_secs(5),
        )?;
        sock.set_read_timeout(Some(Duration::from_secs(5)))?;
        sock.set_write_timeout(Some(Duration::from_secs(5)))?;
        // FINS TCP handshake (C frame)
        let mut handshake = vec![0u8; 24];
        // Reserved
        handshake[0] = 0x46; // 'F'
        handshake[1] = 0x49; // 'I'
        handshake[2] = 0x4E; // 'N'
        handshake[3] = 0x53; // 'S'
        handshake[4] = 0x00; // length high
        handshake[5] = 0x00; // length low
        handshake[6] = 0x00; // reserved
        handshake[7] = 0x00; // reserved
        // Client node info (2 bytes)
        handshake[12] = self.local_net;
        handshake[13] = self.local_node;
        // Server node info (2 bytes)
        handshake[14] = self.remote_net;
        handshake[15] = self.remote_node;
        // Connection type: 0 = P2P (2C)
        handshake[16] = 0x00;
        // Port numbers (unit is u8, expand to 16-bit BE)
        handshake[17] = (u16::from(self.local_unit) >> 8) as u8;
        handshake[18] = (u16::from(self.local_unit) & 0xFF) as u8;
        handshake[19] = (u16::from(self.remote_unit) >> 8) as u8;
        handshake[20] = (u16::from(self.remote_unit) & 0xFF) as u8;
        // Connection ID (0)
        handshake[21] = 0;
        handshake[22] = 0;
        handshake[23] = 0;

        let mut stream = std::net::TcpStream::connect(&addr)?;
        stream.write_all(&handshake)?;
        let mut resp = [0u8; 24];
        stream.read_exact(&mut resp)?;
        // Response should echo "FINS" + 0
        if &resp[0..4] != b"FINS" {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Invalid FINS TCP handshake response",
            ));
        }
        self.socket = Some(stream);
        self.connected = true;
        Ok(())
    }

    /// Disconnect TCP socket.
    fn disconnect(&mut self) {
        self.socket = None;
        self.connected = false;
    }

    /// Build FINS over TCP frame with 28-byte header.
    fn build_fins_frame(&mut self, mrc: u8, src: u8, params: &[u8]) -> Vec<u8> {
        self.sid = self.sid.wrapping_add(1);
        let len = 28 + params.len();
        let mut frame = vec![0u8; len];

        // FINS TCP header (28 bytes)
        frame[0] = 0x46; // 'F'
        frame[1] = 0x49; // 'I'
        frame[2] = 0x4E; // 'N'
        frame[3] = 0x53; // 'S'
        frame[4] = ((len >> 24) & 0xFF) as u8;
        frame[5] = ((len >> 16) & 0xFF) as u8;
        frame[6] = ((len >> 8) & 0xFF) as u8;
        frame[7] = (len & 0xFF) as u8;
        // ICF (response bit = 0)
        frame[8] = 0x80;
        frame[9] = 0x00; // RSV
        frame[10] = 0x02; // GCT
        frame[11] = self.remote_net; // DNA
        frame[12] = self.remote_node; // DA1
        frame[13] = self.remote_unit; // DA2
        frame[14] = self.local_net; // SNA
        frame[15] = self.local_node; // SA1
        frame[16] = 0x00; // SA2 (CPU)
        frame[17] = self.sid; // SID
        frame[18] = mrc; // MRC
        frame[19] = src; // SRC
        frame[20] = 0x00; // Reserved
        frame[21] = 0x00; // Reserved
        frame[22] = 0x00; // Reserved
        frame[23] = 0x00; // Reserved
        frame[24] = ((params.len() >> 8) & 0xFF) as u8;
        frame[25] = (params.len() & 0xFF) as u8;
        frame[26] = 0x00; // Reserved
        frame[27] = 0x00; // Reserved

        // Copy params after header
        frame[28..].copy_from_slice(params);
        frame
    }

    /// Send FINS command and read response over TCP.
    fn send_fins(&mut self, mrc: u8, src: u8, params: &[u8]) -> std::io::Result<Vec<u8>> {
        // Build frame first (only mutates sid)
        let frame = self.build_fins_frame(mrc, src, params);

        let sock = self.socket.as_mut().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotConnected, "socket not connected")
        })?;

        sock.write_all(&frame)?;

        // Read response header (28 bytes)
        let mut header = vec![0u8; 28];
        sock.read_exact(&mut header)?;

        // Extract data length from header bytes 24-25
        let data_len = (u16::from(header[24]) << 8) | u16::from(header[25]);

        // Read data payload
        let mut data = vec![0u8; data_len as usize];
        if !data.is_empty() {
            sock.read_exact(&mut data)?;
        }

        // Check response code (bytes 12-13 = response code)
        let response_code = u16::from_be_bytes([header[12], header[13]]);
        if response_code != 0x0000 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("FINS error: 0x{:04X}", response_code),
            ));
        }

        Ok(data)
    }

    /// Read memory area (command 0x0401).
    fn read_memory(&mut self, area: u8, address: u32, bit_offset: u8, word_count: u16) -> std::io::Result<Vec<u16>> {
        let mut params = vec![area];
        params.push(((address >> 16) & 0xFF) as u8);
        params.push(((address >> 8) & 0xFF) as u8);
        params.push((address & 0xFF) as u8);
        params.push(bit_offset);
        params.push(0x00); // padding
        params.push(((word_count >> 8) & 0xFF) as u8);
        params.push((word_count & 0xFF) as u8);

        let data = self.send_fins(0x04, 0x01, &params)?;

        let mut words = Vec::with_capacity(data.len() / 2);
        for chunk in data.chunks(2) {
            if chunk.len() == 2 {
                words.push(u16::from_be_bytes([chunk[0], chunk[1]]));
            }
        }
        Ok(words)
    }

    /// Write memory area (command 0x0802).
    fn write_memory(&mut self, area: u8, address: u32, bit_offset: u8, words: &[u16]) -> std::io::Result<()> {
        let mut params = vec![area];
        params.push(((address >> 16) & 0xFF) as u8);
        params.push(((address >> 8) & 0xFF) as u8);
        params.push((address & 0xFF) as u8);
        params.push(bit_offset);
        params.push(0x00); // padding

        for w in words {
            params.push(((w >> 8) & 0xFF) as u8);
            params.push((w & 0xFF) as u8);
        }

        let data = self.send_fins(0x08, 0x02, &params)?;
        if data.len() >= 2 {
            let response_code = u16::from_be_bytes([data[0], data[1]]);
            if response_code != 0x0000 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("FINS write error: 0x{:04X}", response_code),
                ));
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Plugin
// ---------------------------------------------------------------------------
pub struct OmronFinsPlugin {
    pub state: Arc<AsyncRwLock<HashMap<NodeId, OmronFinsState>>>,
}

impl Default for OmronFinsPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl OmronFinsPlugin {
    pub fn new() -> Self {
        Self {
            state: Arc::new(AsyncRwLock::new(HashMap::new())),
        }
    }
}

// ---------------------------------------------------------------------------
// Address parsing
// ---------------------------------------------------------------------------
/// Parse address string into (area_code, address, bit_offset).
/// Supports: DM0, D0, CIO10.5, W100, H100, A100, etc.
fn parse_address(addr: &str) -> Option<(u8, u32, u8)> {
    let addr = addr.trim().to_uppercase();
    if addr.starts_with("DM") {
        let num: u32 = addr.trim_start_matches("DM").parse().ok()?;
        Some((AREA_DM, num, 0))
    } else if addr.starts_with("D") {
        let num: u32 = addr.trim_start_matches('D').parse().ok()?;
        Some((AREA_DM, num, 0))
    } else if addr.starts_with("CIO") {
        let parts: Vec<&str> = addr[3..].split('.').collect();
        let base: u32 = parts.first().and_then(|s| s.parse().ok()).unwrap_or(0);
        let bit: u8 = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
        Some((AREA_CIO, base, bit))
    } else if addr.starts_with("W") && !addr.starts_with("WR") {
        let num: u32 = addr.trim_start_matches('W').parse().ok()?;
        Some((AREA_W, num, 0))
    } else if addr.starts_with("WR") {
        let num: u32 = addr.trim_start_matches("WR").parse().ok()?;
        Some((AREA_W, num, 0))
    } else if addr.starts_with("HR") {
        let num: u32 = addr.trim_start_matches("HR").parse().ok()?;
        Some((AREA_HR, num, 0))
    } else if addr.starts_with("H") {
        let num: u32 = addr.trim_start_matches('H').parse().ok()?;
        Some((AREA_HR, num, 0))
    } else if addr.starts_with("AR") {
        let num: u32 = addr.trim_start_matches("AR").parse().ok()?;
        Some((AREA_AR, num, 0))
    } else if addr.starts_with("A") {
        let num: u32 = addr.trim_start_matches('A').parse().ok()?;
        Some((AREA_AR, num, 0))
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Default groups / tags
// ---------------------------------------------------------------------------
fn default_group() -> Group {
    Group {
        id: GroupId::new(),
        name: "default".to_string(),
        interval_ms: 1000,
        description: Some("Default polling group".to_string()),
    }
}

fn default_tags(group_id: GroupId) -> Vec<Tag> {
    vec![
        Tag {
            id: TagId::new(),
            name: "DM0".to_string(),
            address: "DM0".to_string(),
            attr: TagAttr::ReadWrite,
            data_type: Some("int16".to_string()),
            description: Some("Data Memory 0".to_string()),
            group_id,
        },
        Tag {
            id: TagId::new(),
            name: "CIO0".to_string(),
            address: "CIO0".to_string(),
            attr: TagAttr::ReadWrite,
            data_type: Some("int16".to_string()),
            description: Some("CIO bit 0".to_string()),
            group_id,
        },
    ]
}

// ---------------------------------------------------------------------------
// SouthPlugin implementation
// ---------------------------------------------------------------------------
#[async_trait::async_trait]
impl gateway_sdk::SouthPlugin for OmronFinsPlugin {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "omron-fins",
            kind: PluginKind::South,
            description: Some("Omron FINS over TCP — CP/CJ/NJ series PLCs"),
            version: "0.1.0",
            name_zh: Some("Omron FINS"),
            name_en: Some("Omron FINS"),
            description_zh: Some("Omron FINS协议插件，支持CP/CJ/NJ系列PLC"),
            description_en: Some("Omron FINS over TCP — CP/CJ/NJ series PLCs"),
        }
    }

    fn config_schema(&self) -> Option<ConfigSchema> {
        Some(
            ConfigSchema::new()
                .param(ParamSchema {
                    name: "host".to_string(),
                    description: Some("PLC IP address".to_string()),
                    name_zh: Some("PLC IP地址".to_string()),
                    name_en: Some("PLC IP address".to_string()),
                    description_zh: Some("Omron PLC的IP地址".to_string()),
                    description_en: Some("IP address of Omron PLC".to_string()),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::String,
                    default: Some(serde_json::json!("192.168.1.10")),
                    valid: None,
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "port".to_string(),
                    description: Some("FINS TCP port".to_string()),
                    name_zh: Some("端口".to_string()),
                    name_en: Some("FINS TCP port".to_string()),
                    description_zh: Some("FINS TCP端口，默认9600".to_string()),
                    description_en: Some("FINS TCP port, default 9600".to_string()),
                    attribute: ParamAttribute::Optional,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(9600)),
                    valid: None,
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "local_net".to_string(),
                    description: Some("Local network number".to_string()),
                    name_zh: Some("本地网络号".to_string()),
                    name_en: Some("Local network number".to_string()),
                    description_zh: Some("本地FINS网络号".to_string()),
                    description_en: Some("Local FINS network number".to_string()),
                    attribute: ParamAttribute::Optional,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(0)),
                    valid: None,
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "local_unit".to_string(),
                    description: Some("Local unit number".to_string()),
                    name_zh: Some("本地单元号".to_string()),
                    name_en: Some("Local unit number".to_string()),
                    description_zh: Some("本地FINS单元号".to_string()),
                    description_en: Some("Local FINS unit number".to_string()),
                    attribute: ParamAttribute::Optional,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(0)),
                    valid: None,
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "local_node".to_string(),
                    description: Some("Local node number".to_string()),
                    name_zh: Some("本地节点号".to_string()),
                    name_en: Some("Local node number".to_string()),
                    description_zh: Some("本地FINS节点号".to_string()),
                    description_en: Some("Local FINS node number".to_string()),
                    attribute: ParamAttribute::Optional,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(0)),
                    valid: None,
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "remote_net".to_string(),
                    description: Some("Remote network number".to_string()),
                    name_zh: Some("远程网络号".to_string()),
                    name_en: Some("Remote network number".to_string()),
                    description_zh: Some("远程FINS网络号".to_string()),
                    description_en: Some("Remote FINS network number".to_string()),
                    attribute: ParamAttribute::Optional,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(0)),
                    valid: None,
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "remote_unit".to_string(),
                    description: Some("Remote unit number".to_string()),
                    name_zh: Some("远程单元号".to_string()),
                    name_en: Some("Remote unit number".to_string()),
                    description_zh: Some("远程FINS单元号".to_string()),
                    description_en: Some("Remote FINS unit number".to_string()),
                    attribute: ParamAttribute::Optional,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(0)),
                    valid: None,
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "remote_node".to_string(),
                    description: Some("Remote node number".to_string()),
                    name_zh: Some("远程节点号".to_string()),
                    name_en: Some("Remote node number".to_string()),
                    description_zh: Some("远程FINS节点号".to_string()),
                    description_en: Some("Remote FINS node number".to_string()),
                    attribute: ParamAttribute::Optional,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(0)),
                    valid: None,
                    ..Default::default()
                }),
        )
    }

    fn tag_schema(&self) -> Option<TagSchema> {
        Some(TagSchema {
            data_types: Some(vec![
                "int16".to_string(),
                "int32".to_string(),
                "bool".to_string(),
            ]),
            address_format: Some("DM0, CIO10.5, W100, H100, AR100 (bit: DM0.5)".to_string()),
            address_format_zh: Some("DM0, CIO10.5, W100, H100, AR100（位: DM0.5）".to_string()),
            address_format_en: Some("DM0, CIO10.5, W100, H100, AR100 (bit: DM0.5)".to_string()),
        })
    }

    async fn open(&self, node_id: NodeId, config: PluginConfig) -> PluginResult<()> {
        log::info(node_id, "open omron-fins plugin");

        let host = config
            .get("host")
            .and_then(|v| v.as_str())
            .unwrap_or("192.168.1.10")
            .to_string();
        let port = config
            .get("port")
            .and_then(|v| v.as_u64())
            .unwrap_or(9600) as u16;
        let local_net = config.get("local_net").and_then(|v| v.as_u64()).unwrap_or(0) as u8;
        let local_unit = config.get("local_unit").and_then(|v| v.as_u64()).unwrap_or(0) as u8;
        let local_node = config.get("local_node").and_then(|v| v.as_u64()).unwrap_or(0) as u8;
        let remote_net = config.get("remote_net").and_then(|v| v.as_u64()).unwrap_or(0) as u8;
        let remote_unit = config.get("remote_unit").and_then(|v| v.as_u64()).unwrap_or(0) as u8;
        let remote_node = config.get("remote_node").and_then(|v| v.as_u64()).unwrap_or(0) as u8;

        let grp = default_group();
        let tags = default_tags(grp.id);

        let mut state = OmronFinsState {
            host: host.clone(),
            port,
            local_net,
            local_unit,
            local_node,
            remote_net,
            remote_unit,
            remote_node,
            socket: None,
            connected: false,
            sid: 0,
        };

        // Try to connect on open
        if let Err(e) = state.connect() {
            log::warn(node_id, &format!("FINS TCP connect failed: {}", e));
        }

        let mut map = self.state.write().await;
        map.insert(node_id, state);

        // Store default group and tags in node metadata via config
        let mut full_config = config.clone();
        full_config.insert("__groups".to_string(), serde_json::json!(vec![grp]));
        full_config.insert("__tags".to_string(), serde_json::json!(tags));

        log::info(node_id, &format!("omron-fins opened ({}:{})", host, port));
        Ok(())
    }

    async fn close(&self, node_id: NodeId) -> PluginResult<()> {
        log::info(node_id, "close omron-fins");
        let mut map = self.state.write().await;
        if let Some(mut state) = map.remove(&node_id) {
            state.disconnect();
        }
        Ok(())
    }

    async fn init(&self, node_id: NodeId) -> PluginResult<()> {
        log::info(node_id, "init omron-fins");
        Ok(())
    }

    async fn uninit(&self, node_id: NodeId) -> PluginResult<()> {
        log::info(node_id, "uninit omron-fins");
        Ok(())
    }

    async fn start(&self, node_id: NodeId) -> PluginResult<()> {
        log::info(node_id, "start omron-fins");
        let map = self.state.read().await;
        if let Some(state) = map.get(&node_id) {
            if !state.connected {
                return Err(PluginError::msg("PLC not connected"));
            }
        }
        Ok(())
    }

    async fn stop(&self, node_id: NodeId) -> PluginResult<()> {
        log::info(node_id, "stop omron-fins");
        Ok(())
    }

    async fn setting(&self, node_id: NodeId, config: PluginConfig) -> PluginResult<()> {
        log::info(node_id, "setting omron-fins");
        let mut map = self.state.write().await;
        if let Some(state) = map.get_mut(&node_id) {
            if let Some(host) = config.get("host").and_then(|v| v.as_str()) {
                state.host = host.to_string();
            }
            if let Some(port) = config.get("port").and_then(|v| v.as_u64()) {
                state.port = port as u16;
            }
            if let Some(v) = config.get("local_net").and_then(|v| v.as_u64()) {
                state.local_net = v as u8;
            }
            if let Some(v) = config.get("local_unit").and_then(|v| v.as_u64()) {
                state.local_unit = v as u8;
            }
            if let Some(v) = config.get("local_node").and_then(|v| v.as_u64()) {
                state.local_node = v as u8;
            }
            if let Some(v) = config.get("remote_net").and_then(|v| v.as_u64()) {
                state.remote_net = v as u8;
            }
            if let Some(v) = config.get("remote_unit").and_then(|v| v.as_u64()) {
                state.remote_unit = v as u8;
            }
            if let Some(v) = config.get("remote_node").and_then(|v| v.as_u64()) {
                state.remote_node = v as u8;
            }
        }
        Ok(())
    }

    async fn validate_tag(&self, node_id: NodeId, tag: &Tag) -> PluginResult<()> {
        let _ = node_id;
        if parse_address(&tag.address).is_none() {
            return Err(PluginError::tag_invalid(&format!(
                "invalid FINS address: {} (supported: DM, CIO, W, HR, AR)",
                tag.address
            )));
        }
        Ok(())
    }

    async fn poll_group(
        &self,
        node_id: NodeId,
        _group_id: GroupId,
        tags: &[Tag],
    ) -> PluginResult<Vec<(TagId, DataValue)>> {
        let mut map = self.state.write().await;
        let state = map.get_mut(&node_id).ok_or_else(|| {
            PluginError::msg("node not open")
        })?;

        if !state.connected {
            return Err(PluginError::msg("not connected"));
        }

        let mut results = Vec::with_capacity(tags.len());

        for tag in tags {
            let (area, offset, bit) = match parse_address(&tag.address) {
                Some(a) => a,
                None => {
                    results.push((tag.id, DataValue::String("invalid address".to_string())));
                    continue;
                }
            };

            let value = if bit > 0 || area == AREA_CIO {
                // Bit read
                match state.read_memory(area, offset, bit, 1) {
                    Ok(words) if !words.is_empty() => {
                        let bit_val = (words[0] & (1u16 << bit)) != 0;
                        DataValue::Bool(bit_val)
                    }
                    Ok(_) => DataValue::String("read error".to_string()),
                    Err(e) => DataValue::String(format!("err: {}", e)),
                }
            } else {
                // Word read
                match state.read_memory(area, offset, 0, 1) {
                    Ok(words) if !words.is_empty() => DataValue::Int16(words[0] as i16),
                    Ok(_) => DataValue::String("read error".to_string()),
                    Err(e) => DataValue::String(format!("err: {}", e)),
                }
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
        let mut map = self.state.write().await;
        let state = map.get_mut(&node_id).ok_or_else(|| {
            PluginError::msg("node not open")
        })?;

        if !state.connected {
            return Err(PluginError::msg("not connected"));
        }

        for (tag, value) in values {
            let (area, offset, bit) = match parse_address(&tag.address) {
                Some(a) => a,
                None => continue,
            };

            if bit > 0 {
                // Bit write: read-modify-write
                match state.read_memory(area, offset, 0, 1) {
                    Ok(mut words) if !words.is_empty() => {
                        let bit_val = value.as_bool().unwrap_or(false);
                        if bit_val {
                            words[0] |= 1u16 << bit;
                        } else {
                            words[0] &= !(1u16 << bit);
                        }
                        state.write_memory(area, offset, 0, &words)?;
                    }
                    Ok(_) => {}
                    Err(e) => {
                        log::warn(node_id, &format!("write bit read error: {}", e));
                    }
                }
            } else {
                // Word write
                if let Some(val_i64) = value.as_i64() {
                    state.write_memory(area, offset, 0, &[val_i64 as u16])?;
                }
            }
        }

        Ok(())
    }

    async fn list_groups(&self, node_id: NodeId) -> PluginResult<Vec<Group>> {
        let map = self.state.read().await;
        if map.contains_key(&node_id) {
            Ok(vec![default_group()])
        } else {
            Err(PluginError::msg("node not open"))
        }
    }

    async fn list_tags(&self, node_id: NodeId, group_id: GroupId) -> PluginResult<Vec<Tag>> {
        let map = self.state.read().await;
        if map.contains_key(&node_id) {
            Ok(default_tags(group_id))
        } else {
            Err(PluginError::msg("node not open"))
        }
    }
}
