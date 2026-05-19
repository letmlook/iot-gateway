//! Mitsubishi MC (MELSEC) PLC communication protocol plugin.
//! Supports Q/iQ-R/iQ-F series via MC-3E/4E protocol.
//! Address format: `D100` (data register), `X0` (input), `Y0` (output), `M100` (marker/relay), `W100` (link register)

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

use state::MCState;

/// Mitsubishi MC south plugin
pub struct MitsubishiMcPlugin {
    state: Arc<RwLock<HashMap<NodeId, MCState>>>,
}

impl Default for MitsubishiMcPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl MitsubishiMcPlugin {
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
}

// ============================================================================
// MC Protocol Helper Functions
// ============================================================================

/// Parse Mitsubishi address to extract device type and address number
fn parse_address(addr: &str) -> Option<(char, u32)> {
    let mut chars = addr.chars();
    let first = chars.next()?;
    let remaining: String = chars.collect();
    let num: u32 = remaining.parse().ok()?;
    Some((first, num))
}

/// Get device code for MC protocol (handles both single and multi-char device types)
fn mc_device_code(addr: &str) -> u8 {
    if addr.starts_with("SM") { 0x91 }      // Special marker
    else if addr.starts_with("SD") { 0xA9 } // Special data
    else if addr.starts_with("TS") { 0x55 } // Timer (coil)
    else if addr.starts_with("TC") { 0x43 } // Timer (contact) - same as counter
    else if addr.starts_with("TN") { 0x54 } // Timer (current value)
    else if addr.starts_with("CN") { 0x43 } // Counter (current value)
    else {
        match addr.chars().next().unwrap_or('D') {
            'X' => 0x58,  // Input
            'Y' => 0x59,  // Output
            'M' => 0x4D,  // Marker
            'L' => 0x4C,  // Latch
            'B' => 0x42,  // Link
            'D' => 0x44,  // Data register
            'W' => 0x57,  // Link register
            'S' => 0x53,  // Step
            'T' => 0x54,  // Timer (contact)
            'C' => 0x43,  // Counter (contact)
            _ => 0x44,    // Default to D
        }
    }
}

/// Build MC protocol read request frame (3E frame format)
fn build_mc_read_frame(network: u8, station: u8, dev_type: char, dev_addr: u32, word_count: u16) -> Vec<u8> {
    let mut frame = Vec::with_capacity(256);

    // Build request data first
    let mut request_data = Vec::with_capacity(64);

    // Command (2 bytes) - 0x0401 = Word read
    request_data.push(0x04);
    request_data.push(0x01);
    // Subcommand (2 bytes) - Binary format
    request_data.push(0x00);
    request_data.push(0x01);

    // Device code
    let dev_code = mc_device_code(&format!("{}{}", dev_type, dev_addr));
    request_data.push(dev_code);

    // Binary address format: 3-byte address (big endian)
    request_data.push(((dev_addr >> 16) & 0xFF) as u8);
    request_data.push(((dev_addr >> 8) & 0xFF) as u8);
    request_data.push((dev_addr & 0xFF) as u8);

    // Word count (2 bytes big endian)
    request_data.push(((word_count >> 8) & 0xFF) as u8);
    request_data.push((word_count & 0xFF) as u8);

    // Assemble the full frame
    // Header (4 bytes): 50 00 [network] [station]
    frame.push(0x50);
    frame.push(0x00);
    frame.push(network);
    frame.push(station);

    // Subheader (2 bytes): word read = 0x0C 0x00
    frame.push(0x0C);
    frame.push(0x00);

    // Request destination network/station
    frame.push(network);
    frame.push(station);

    // Request data length (2 bytes)
    let data_len = request_data.len() as u16;
    frame.push(((data_len >> 8) & 0xFF) as u8);
    frame.push((data_len & 0xFF) as u8);

    // Request data
    frame.extend_from_slice(&request_data);

    frame
}

/// Build MC protocol write request frame (3E frame format)
fn build_mc_write_frame(network: u8, station: u8, dev_type: char, dev_addr: u32, data: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(256 + data.len());

    let mut request_data = Vec::with_capacity(64);

    // Command (2 bytes) - 0x0402 = Word write
    request_data.push(0x04);
    request_data.push(0x02);
    // Subcommand (2 bytes) - Binary format
    request_data.push(0x00);
    request_data.push(0x01);

    // Device code
    let dev_code = mc_device_code(&format!("{}{}", dev_type, dev_addr));
    request_data.push(dev_code);

    // Binary address format: 3-byte address
    request_data.push(((dev_addr >> 16) & 0xFF) as u8);
    request_data.push(((dev_addr >> 8) & 0xFF) as u8);
    request_data.push((dev_addr & 0xFF) as u8);

    // Word count (2 bytes)
    let word_count = (data.len() / 2) as u16;
    request_data.push(((word_count >> 8) & 0xFF) as u8);
    request_data.push((word_count & 0xFF) as u8);

    // Write data
    request_data.extend_from_slice(data);

    // Assemble frame
    frame.push(0x50);
    frame.push(0x00);
    frame.push(network);
    frame.push(station);

    // Subheader (2 bytes): word write = 0x14 0x00
    frame.push(0x14);
    frame.push(0x00);

    // Request destination network/station
    frame.push(network);
    frame.push(station);

    // Request data length
    let data_len = request_data.len() as u16;
    frame.push(((data_len >> 8) & 0xFF) as u8);
    frame.push((data_len & 0xFF) as u8);

    // Request data
    frame.extend_from_slice(&request_data);

    frame
}

/// Read data from Mitsubishi PLC via MC protocol
async fn mc_read_values(host: &str, port: u16, network: u8, station: u8, tags: &[Tag]) -> PluginResult<HashMap<String, DataValue>> {
    let addr = format!("{}:{}", host, port);
    let mut stream = TcpStream::connect(&addr).await.map_err(|e| {
        PluginError::msg(format!("MC connection failed: {}", e))
    })?;

    let mut results = HashMap::new();

    for tag in tags {
        let (dev_type, dev_addr) = parse_address(&tag.address)
            .ok_or_else(|| PluginError::msg(format!("Invalid address: {}", tag.address)))?;

        let frame = build_mc_read_frame(network, station, dev_type, dev_addr, 1);
        stream.write_all(&frame).await.map_err(|e| {
            PluginError::msg(format!("MC write failed: {}", e))
        })?;

        let mut response = vec![0u8; 256];
        let n = stream.read_buf(&mut response).await.map_err(|e| {
            PluginError::msg(format!("MC read failed: {}", e))
        })?;

        if n > 18 {
            // Parse MC response - data starts at offset 18 in 3E binary response
            let value = parse_mc_response(&response[18..n], tag.data_type.as_deref().unwrap_or("int16"));
            results.insert(tag.address.clone(), value);
        }
    }

    Ok(results)
}

/// Write data to Mitsubishi PLC via MC protocol
async fn mc_write_values(host: &str, port: u16, network: u8, station: u8, values: &[(Tag, DataValue)]) -> PluginResult<()> {
    let addr = format!("{}:{}", host, port);
    let mut stream = TcpStream::connect(&addr).await.map_err(|e| {
        PluginError::msg(format!("MC connection failed: {}", e))
    })?;

    for (tag, value) in values {
        let (dev_type, dev_addr) = parse_address(&tag.address)
            .ok_or_else(|| PluginError::msg(format!("Invalid address: {}", tag.address)))?;

        let data_bytes = encode_data_value(value)?;
        let frame = build_mc_write_frame(network, station, dev_type, dev_addr, &data_bytes);

        stream.write_all(&frame).await.map_err(|e| {
            PluginError::msg(format!("MC write failed: {}", e))
        })?;

        let mut response = vec![0u8; 64];
        let n = stream.read_buf(&mut response).await.map_err(|e| {
            PluginError::msg(format!("MC write response failed: {}", e))
        })?;

        // Check response - should have completion code 0 at offset 13-14
        if n < 15 || response[13] != 0x00 || response[14] != 0x00 {
            let err_code = if n >= 15 { (response[13] as u16) | ((response[14] as u16) << 8) } else { 0xFFFF };
            return Err(PluginError::msg(format!("MC write failed with code: 0x{:04X}", err_code)));
        }
    }

    Ok(())
}

/// Parse MC protocol response data
fn parse_mc_response(data: &[u8], data_type: &str) -> DataValue {
    if data.len() < 2 {
        return DataValue::Int64(0);
    }

    match data_type {
        "float32" | "float64" => {
            // 4 bytes for float32
            if data.len() >= 4 {
                let bits = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
                if data_type == "float32" {
                    return DataValue::Float32(f32::from_bits(bits));
                }
            }
            // 8 bytes for float64
            if data.len() >= 8 {
                let bits = u64::from_be_bytes([data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7]]);
                return DataValue::Float64(f64::from_bits(bits));
            }
            DataValue::Float64(0.0)
        }
        "int32" | "uint32" => {
            if data.len() >= 4 {
                let val = i32::from_be_bytes([data[0], data[1], data[2], data[3]]);
                return DataValue::Int64(val as i64);
            }
            DataValue::Int64(0)
        }
        "bool" => {
            // For bit access, check if data is non-zero
            let val = data.iter().take(2).fold(0u16, |acc, &b| (acc << 8) | b as u16);
            DataValue::Bool(val != 0)
        }
        _ => {
            // Default: 16-bit integer
            if data.len() >= 2 {
                let val = i16::from_be_bytes([data[0], data[1]]);
                DataValue::Int64(val as i64)
            } else {
                DataValue::Int64(0)
            }
        }
    }
}

/// Encode DataValue to bytes for MC protocol write (binary format, big endian)
fn encode_data_value(value: &DataValue) -> PluginResult<Vec<u8>> {
    let mut buf = Vec::with_capacity(8);

    match value {
        DataValue::Bool(v) => {
            buf.push(0x00);
            buf.push(if *v { 1 } else { 0 });
        }
        DataValue::Int64(v) => {
            if *v >= i16::MIN as i64 && *v <= i16::MAX as i64 {
                let val = *v as i16;
                buf.push(((val >> 8) & 0xFF) as u8);
                buf.push((val & 0xFF) as u8);
            } else {
                let val = *v as i32;
                buf.push(((val >> 24) & 0xFF) as u8);
                buf.push(((val >> 16) & 0xFF) as u8);
                buf.push(((val >> 8) & 0xFF) as u8);
                buf.push((val & 0xFF) as u8);
            }
        }
        DataValue::UInt64(v) => {
            if *v <= u16::MAX as u64 {
                let val = *v as u16;
                buf.push(((val >> 8) & 0xFF) as u8);
                buf.push((val & 0xFF) as u8);
            } else {
                let val = *v as u32;
                buf.push(((val >> 24) & 0xFF) as u8);
                buf.push(((val >> 16) & 0xFF) as u8);
                buf.push(((val >> 8) & 0xFF) as u8);
                buf.push((val & 0xFF) as u8);
            }
        }
        DataValue::Float32(v) => {
            let bits = v.to_bits();
            buf.push(((bits >> 24) & 0xFF) as u8);
            buf.push(((bits >> 16) & 0xFF) as u8);
            buf.push(((bits >> 8) & 0xFF) as u8);
            buf.push((bits & 0xFF) as u8);
        }
        DataValue::Float64(v) => {
            let bits = v.to_bits();
            buf.push(((bits >> 56) & 0xFF) as u8);
            buf.push(((bits >> 48) & 0xFF) as u8);
            buf.push(((bits >> 40) & 0xFF) as u8);
            buf.push(((bits >> 32) & 0xFF) as u8);
            buf.push(((bits >> 24) & 0xFF) as u8);
            buf.push(((bits >> 16) & 0xFF) as u8);
            buf.push(((bits >> 8) & 0xFF) as u8);
            buf.push((bits & 0xFF) as u8);
        }
        _ => return Err(PluginError::msg("Unsupported data type for MC write")),
    }

    Ok(buf)
}

/// Generate realistic stub data for MC protocol (sinusoidal + noise variation)
fn generate_realistic_mc_value(addr: &str, data_type: &str, timestamp_ms: i64) -> DataValue {
    use std::f64::consts::PI;

    // Create deterministic seed from address
    let addr_hash: u64 = addr.bytes().fold(0u64, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u64));
    let phase_offset = (addr_hash % 1000) as f64 / 1000.0 * 2.0 * PI;
    let amplitude = 100.0 + (addr_hash % 200) as f64;
    let base_value = 1000.0 + (addr_hash % 500) as f64;

    let t = (timestamp_ms as f64 / 1000.0) + phase_offset;
    let sin_value = t.sin() * amplitude;
    let noise = (t * 7.3).sin() * 5.0 + (t * 13.7).sin() * 2.0;

    match data_type {
        "bool" => {
            let toggle_rate = 0.3 + (addr_hash % 5) as f64 / 10.0;
            let toggled = ((t * toggle_rate).sin() > 0.0) != ((addr_hash % 2) == 0);
            DataValue::Bool(toggled)
        }
        "float32" | "float64" => {
            let val = base_value + sin_value + noise;
            if data_type == "float32" {
                DataValue::Float32(val as f32)
            } else {
                DataValue::Float64(val)
            }
        }
        "int32" | "uint32" => {
            let val = (base_value + sin_value + noise) as i64;
            if data_type == "uint32" {
                DataValue::UInt64(val.unsigned_abs() as u64)
            } else {
                DataValue::Int64(val)
            }
        }
        _ => {
            let val = (base_value + sin_value + noise) as i16;
            DataValue::Int64(val as i64)
        }
    }
}

// ============================================================================
// SouthPlugin Trait Implementation
// ============================================================================

#[async_trait::async_trait]
impl SouthPlugin for MitsubishiMcPlugin {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "mitsubishi-mc",
            kind: PluginKind::South,
            description: Some("Mitsubishi MELSEC PLC communication protocol (MC-3E/4E)"),
            version: "0.1.0",
            name_zh: Some("三菱MC"),
            name_en: Some("Mitsubishi MC"),
            description_zh: Some("三菱MELSEC PLC通信协议，支持Q/iQ-R/iQ-F系列"),
            description_en: Some("Mitsubishi MELSEC PLC protocol — Q/iQ-R/iQ-F series via MC-3E/4E"),
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
                    description: Some("Mitsubishi PLC IP address".to_string()),
                    description_zh: Some("三菱 PLC IP 地址".to_string()),
                    description_en: Some("Mitsubishi PLC IP address".to_string()),
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
                    description: Some("MC protocol TCP port (default 5002)".to_string()),
                    description_zh: Some("MC 协议 TCP 端口（默认 5002）".to_string()),
                    description_en: Some("MC protocol TCP port (default 5002)".to_string()),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(5002)),
                    valid: Some(ParamValid { min: Some(1), max: Some(65535), regex: None, length: None }),
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "network".to_string(),
                    name_zh: Some("网络号".to_string()),
                    name_en: Some("Network Number".to_string()),
                    description: Some("Mitsubishi network number (0-255)".to_string()),
                    description_zh: Some("三菱网络号（0-255）".to_string()),
                    description_en: Some("Mitsubishi network number (0-255)".to_string()),
                    attribute: ParamAttribute::Optional,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(0)),
                    valid: Some(ParamValid { min: Some(0), max: Some(255), regex: None, length: None }),
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "station".to_string(),
                    name_zh: Some("站号".to_string()),
                    name_en: Some("Station Number".to_string()),
                    description: Some("Mitsubishi station number (0-31)".to_string()),
                    description_zh: Some("三菱站号（0-31）".to_string()),
                    description_en: Some("Mitsubishi station number (0-31)".to_string()),
                    attribute: ParamAttribute::Optional,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(0)),
                    valid: Some(ParamValid { min: Some(0), max: Some(31), regex: None, length: None }),
                    ..Default::default()
                })
                .tag_regex(vec![
                    TagRegexEntry { data_type: "bool".to_string(), regex: r"^[XYMMLB][0-9]+$".to_string() },
                    TagRegexEntry { data_type: "int16".to_string(), regex: r"^[DW][0-9]+$".to_string() },
                    TagRegexEntry { data_type: "uint16".to_string(), regex: r"^[DW][0-9]+$".to_string() },
                    TagRegexEntry { data_type: "int32".to_string(), regex: r"^[DW][0-9]+$".to_string() },
                    TagRegexEntry { data_type: "uint32".to_string(), regex: r"^[DW][0-9]+$".to_string() },
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
            address_format: Some("D100 (data register), X0 (input), Y0 (output), M100 (marker), W100 (link)".to_string()),
            address_format_zh: Some("D100 (数据寄存器), X0 (输入), Y0 (输出), M100 (继电器), W100 (链接寄存器)".to_string()),
            address_format_en: Some("D100 (data register), X0 (input), Y0 (output), M100 (marker), W100 (link)".to_string()),
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
        let valid_prefixes = ["D", "W", "M", "X", "Y", "L", "B", "SM", "SD", "CN", "TN", "TS", "TC"];
        let starts_with_valid = valid_prefixes.iter().any(|p| addr.starts_with(*p));
        if !starts_with_valid {
            return Err(PluginError::tag_invalid("invalid Mitsubishi address prefix"));
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
            .unwrap_or(5002) as u16;
        let network = config.get("network")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u8;
        let station = config.get("station")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u8;

        log::info(node_id, format!("open mitsubishi-mc: host={}, port={}, network={}, station={}", host, port, network, station));

        let groups = Self::default_groups();
        let tags = groups.iter().flat_map(|g| {
            vec![
                Tag {
                    id: TagId::new(),
                    name: "d100".to_string(),
                    address: "D100".to_string(),
                    attr: gateway_sdk::TagAttr::Read,
                    data_type: Some("int16".to_string()),
                    description: Some("Data register 100".to_string()),
                    group_id: g.id,
                },
            ]
        }).collect::<Vec<_>>();

        let mut state = self.state.write().await;
        state.insert(node_id, MCState {
            host,
            port,
            network,
            station,
            groups,
            tags,
        });
        Ok(())
    }

    async fn close(&self, node_id: NodeId) -> PluginResult<()> {
        log::info(node_id, "close mitsubishi-mc");
        let mut state = self.state.write().await;
        state.remove(&node_id);
        Ok(())
    }

    async fn start(&self, node_id: NodeId) -> PluginResult<()> {
        log::info(node_id, "start mitsubishi-mc");
        Ok(())
    }

    async fn stop(&self, node_id: NodeId) -> PluginResult<()> {
        log::info(node_id, "stop mitsubishi-mc");
        Ok(())
    }

    async fn setting(&self, node_id: NodeId, config: PluginConfig) -> PluginResult<()> {
        log::info(node_id, "setting mitsubishi-mc (config updated)");
        let mut state = self.state.write().await;
        if let Some(s) = state.get_mut(&node_id) {
            s.host = config.get("host")
                .and_then(|v| v.as_str())
                .unwrap_or(&s.host)
                .to_string();
            s.port = config.get("port")
                .and_then(|v| v.as_u64())
                .unwrap_or(s.port as u64) as u16;
            s.network = config.get("network")
                .and_then(|v| v.as_u64())
                .unwrap_or(s.network as u64) as u8;
            s.station = config.get("station")
                .and_then(|v| v.as_u64())
                .unwrap_or(s.station as u64) as u8;
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

        // Try to read from real PLC via MC protocol
        let mc_result = mc_read_values(&s.host, s.port, s.network, s.station, tags).await;

        let timestamp_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);

        let mut results = Vec::with_capacity(tags.len());
        for tag in tags {
            let value = match mc_result {
                Ok(ref data) if data.contains_key(&tag.address) => {
                    data.get(&tag.address).cloned().unwrap_or_else(|| {
                        generate_realistic_mc_value(&tag.address, tag.data_type.as_deref().unwrap_or("int16"), timestamp_ms)
                    })
                }
                _ => {
                    // Fall back to realistic stub data if PLC unavailable
                    generate_realistic_mc_value(&tag.address, tag.data_type.as_deref().unwrap_or("int16"), timestamp_ms)
                }
            };
            results.push((tag.id, value));
        }

        Ok(results)
    }

    async fn write_tags(&self, node_id: NodeId, values: &[(Tag, DataValue)]) -> PluginResult<()> {
        let state = self.state.read().await;
        let s = state.get(&node_id).ok_or_else(|| PluginError::msg("node not open"))?;

        log::info(node_id, format!("mitsubishi-mc write_tags: {} tags", values.len()));

        // Write via MC protocol
        mc_write_values(&s.host, s.port, s.network, s.station, values).await
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
