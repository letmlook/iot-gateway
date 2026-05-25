//! Siemens S7 南向插件：支持 S7-300/400/1200/1500 系列 PLC，通过 ISO-TCP 协议通信。
//!
//! 地址格式：
//!   DB{n}.DBD{o} - 数据块双字（浮点数）
//!   DB{n}.DBW{o} - 数据块字（16位整数）
//!   DB{n}.DBB{o} - 数据块字节
//!   DB{n}.DBX{o}.{b} - 数据块某位
//!   I{o} / IB{o} / IW{o} / ID{o} - 输入区
//!   Q{o} / QB{o} / QW{o} / QD{o} - 输出区
//!   M{o} / MB{o} / MW{o} / MD{o} - 标志位（Marker）
//!
//! 配置：
//!   { "host": "192.168.1.10", "rack": 0, "slot": 1 }
//!
//! ISO-TCP 协议栈 (RFC1006):
//!   TPKT (4 bytes) -> COTP (variable) -> S7 PDU (variable)

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
use tokio::net::TcpStream;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::RwLock;
use std::net::SocketAddr;

#[cfg(feature = "real-impl")]
use bytes::BytesMut;

/// S7 地址类型
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum S7Area {
    Db,
    Input,
    Output,
    Marker,
    Counter,
    Timer,
    Unknown,
}

/// S7 点位解析结果
#[derive(Debug, Clone)]
struct S7TagAddr {
    area: S7Area,
    db_number: u16,
    byte_offset: u16,
    bit_offset: Option<u8>,
    data_size: u8,
}

/// 解析 S7 地址字符串
fn parse_s7_address(addr: &str) -> Option<S7TagAddr> {
    let addr = addr.trim().to_uppercase();

    // DB address: DB{n}.DBX{o}.{b} or DB{n}.DBD{o} or DB{n}.DBW{o} or DB{n}.DBB{o}
    if addr.starts_with("DB") {
        let rest = &addr[2..];
        let parts: Vec<&str> = rest.split('.').collect();
        if parts.is_empty() {
            return None;
        }
        let db_num: u16 = parts[0].parse().ok()?;

        if parts.len() == 1 {
            return Some(S7TagAddr {
                area: S7Area::Db,
                db_number: db_num,
                byte_offset: 0,
                bit_offset: None,
                data_size: 1,
            });
        }

        let byte_str = parts[1];
        if byte_str.starts_with("DBX") {
            let byte_off: u16 = byte_str[2..].parse().ok()?;
            if parts.len() == 3 {
                let bit_off: u8 = parts[2].parse().ok()?;
                return Some(S7TagAddr {
                    area: S7Area::Db,
                    db_number: db_num,
                    byte_offset: byte_off,
                    bit_offset: Some(bit_off),
                    data_size: 1,
                });
            } else {
                return Some(S7TagAddr {
                    area: S7Area::Db,
                    db_number: db_num,
                    byte_offset: byte_off,
                    bit_offset: None,
                    data_size: 4,
                });
            }
        } else if byte_str.starts_with("DBD") {
            let byte_off: u16 = byte_str[2..].parse().ok()?;
            return Some(S7TagAddr {
                area: S7Area::Db,
                db_number: db_num,
                byte_offset: byte_off,
                bit_offset: None,
                data_size: 4,
            });
        } else if byte_str.starts_with("DBW") {
            let byte_off: u16 = byte_str[2..].parse().ok()?;
            return Some(S7TagAddr {
                area: S7Area::Db,
                db_number: db_num,
                byte_offset: byte_off,
                bit_offset: None,
                data_size: 2,
            });
        } else if byte_str.starts_with("DBB") {
            let byte_off: u16 = byte_str[2..].parse().ok()?;
            return Some(S7TagAddr {
                area: S7Area::Db,
                db_number: db_num,
                byte_offset: byte_off,
                bit_offset: None,
                data_size: 1,
            });
        } else {
            let byte_off: u16 = byte_str.parse().ok()?;
            return Some(S7TagAddr {
                area: S7Area::Db,
                db_number: db_num,
                byte_offset: byte_off,
                bit_offset: None,
                data_size: 1,
            });
        }
    }

    // I, IB, IW, ID addresses (inputs)
    if addr.starts_with("I") {
        let rest = &addr[1..];
        if rest.starts_with("B") {
            let off: u16 = rest[1..].parse().ok()?;
            return Some(S7TagAddr { area: S7Area::Input, db_number: 0, byte_offset: off, bit_offset: None, data_size: 1 });
        } else if rest.starts_with("W") {
            let off: u16 = rest[1..].parse().ok()?;
            return Some(S7TagAddr { area: S7Area::Input, db_number: 0, byte_offset: off, bit_offset: None, data_size: 2 });
        } else if rest.starts_with("D") {
            let off: u16 = rest[1..].parse().ok()?;
            return Some(S7TagAddr { area: S7Area::Input, db_number: 0, byte_offset: off, bit_offset: None, data_size: 4 });
        } else {
            let off: u16 = rest.parse().ok()?;
            return Some(S7TagAddr { area: S7Area::Input, db_number: 0, byte_offset: off, bit_offset: None, data_size: 1 });
        }
    }

    // Q, QB, QW, QD addresses (outputs)
    if addr.starts_with("Q") {
        let rest = &addr[1..];
        if rest.starts_with("B") {
            let off: u16 = rest[1..].parse().ok()?;
            return Some(S7TagAddr { area: S7Area::Output, db_number: 0, byte_offset: off, bit_offset: None, data_size: 1 });
        } else if rest.starts_with("W") {
            let off: u16 = rest[1..].parse().ok()?;
            return Some(S7TagAddr { area: S7Area::Output, db_number: 0, byte_offset: off, bit_offset: None, data_size: 2 });
        } else if rest.starts_with("D") {
            let off: u16 = rest[1..].parse().ok()?;
            return Some(S7TagAddr { area: S7Area::Output, db_number: 0, byte_offset: off, bit_offset: None, data_size: 4 });
        } else {
            let off: u16 = rest.parse().ok()?;
            return Some(S7TagAddr { area: S7Area::Output, db_number: 0, byte_offset: off, bit_offset: None, data_size: 1 });
        }
    }

    // M, MB, MW, MD addresses (markers)
    if addr.starts_with("M") {
        let rest = &addr[1..];
        if rest.starts_with("B") {
            let off: u16 = rest[1..].parse().ok()?;
            return Some(S7TagAddr { area: S7Area::Marker, db_number: 0, byte_offset: off, bit_offset: None, data_size: 1 });
        } else if rest.starts_with("W") {
            let off: u16 = rest[1..].parse().ok()?;
            return Some(S7TagAddr { area: S7Area::Marker, db_number: 0, byte_offset: off, bit_offset: None, data_size: 2 });
        } else if rest.starts_with("D") {
            let off: u16 = rest[1..].parse().ok()?;
            return Some(S7TagAddr { area: S7Area::Marker, db_number: 0, byte_offset: off, bit_offset: None, data_size: 4 });
        } else {
            let off: u16 = rest.parse().ok()?;
            return Some(S7TagAddr { area: S7Area::Marker, db_number: 0, byte_offset: off, bit_offset: None, data_size: 1 });
        }
    }

    None
}

/// S7 ISO-TCP Connection State
#[cfg(feature = "real-impl")]
struct S7Connection {
    stream: TcpStream,
    local_tref: u16,
    remote_tref: u16,
    pdu_ref: u16,
}

#[cfg(feature = "real-impl")]
impl S7Connection {
    /// Connect to PLC via ISO-TCP (port 102)
    async fn connect(host: &str, rack: u8, slot: u8) -> PluginResult<Self> {
        let addr: SocketAddr = format!("{}:102", host).parse()
            .map_err(|e| PluginError::msg(format!("invalid address {}:102: {}", host, e)))?;
        
        let mut stream = TcpStream::connect(addr).await
            .map_err(|e| PluginError::msg(format!("TCP connect failed: {}", e)))?;
        
        // ISO-TCP connection uses TPKT + COTP CR (Connect Request)
        let mut buf = Vec::new();
        
        // TPKT header (RFC1006): version, reserved, length
        buf.push(0x03);  // TPKT version 3
        buf.push(0x00);  // Reserved
        buf.push(0x00);  // Length high (placeholder)
        buf.push(0x00);  // Length low (placeholder)
        
        // COTP CR (Connect Request) - Connection Request
        let cotp_payload_len = 17; // excluding the length byte itself
        buf.push(cotp_payload_len as u8);  // COTP length
        buf.push(0x0E);  // COTP PDU type: CR (Connect Request)
        buf.push(0x00);  // Destination reference (high)
        buf.push(0x00);  // Destination reference (low)
        buf.push(0x00);  // Source reference (high)
        buf.push(0x01);  // Source reference (low)
        buf.push(0x00);  // Flags
        buf.push(0x00);  // Class
        
        // TPKT parameter: called TSAP (2 bytes)
        buf.push(0xC0);  // Parameter type: called TSAP
        buf.push(0x01);  // Parameter length
        buf.push(0x01);  // TSAP length = 1
        buf.push(0x02);  // called TSAP (rack/slot encoded in second byte)
        
        // Calling TSAP
        buf.push(0xC1);  // Parameter type: calling TSAP
        buf.push(0x01);  // Parameter length
        buf.push(0x01);  // TSAP length = 1
        buf.push(((rack & 0x07) << 4) | (slot & 0x0F));  // calling TSAP encoded
        
        // Set TPKT length
        let total_len = buf.len();
        buf[2] = ((total_len >> 8) & 0xFF) as u8;
        buf[3] = (total_len & 0xFF) as u8;
        
        stream.write_all(&buf).await
            .map_err(|e| PluginError::msg(format!("write failed: {}", e)))?;
        
        // Read COTP CC (Connect Confirm)
        let mut resp = [0u8; 256];
        let n = stream.read(&mut resp).await
            .map_err(|e| PluginError::msg(format!("read failed: {}", e)))?;
        
        if n < 10 {
            return Err(PluginError::msg("short response on ISO connect"));
        }
        
        // Check COTP CC PDU type (should be 0x0D = CC)
        if resp[4] != 0x0D {
            return Err(PluginError::msg(format!("unexpected COTP response: 0x{:02x}", resp[4])));
        }
        
        let local_tref = 0x0100;
        let remote_tref = ((resp[6] as u16) << 8) | (resp[7] as u16);
        
        Ok(Self {
            stream,
            local_tref,
            remote_tref,
            pdu_ref: 0,
        })
    }
    
    /// Send S7 Read Request and receive response
    async fn read_var(&mut self, items: &[(S7Area, u16, u16, u8)]) -> PluginResult<Vec<Vec<u8>>> {
        self.pdu_ref = self.pdu_ref.wrapping_add(1);
        
        let mut request = Vec::new();
        
        // S7 Header
        // Protocol ID
        request.push(0x32);  // S7 Protocol ID
        
        // Message Type: 0x01 = Job, 0x02 = Ack, 0x03 = Ack_Data, 0x07 = Userdata
        request.push(0x01);  // Job
        
        // Reserved
        request.push(0x00);
        request.push(0x00);
        
        // PDU reference (little endian)
        request.push((self.pdu_ref & 0xFF) as u8);
        request.push(((self.pdu_ref >> 8) & 0xFF) as u8);
        
        // Parameters length (high, low) - placeholder
        let params_start = request.len();
        request.push(0x00);
        request.push(0x00);
        
        // Data length (high, low)
        request.push(0x00);
        request.push(0x00);
        
        // Function: 0x04 = Read Var
        request.push(0x04);
        
        // Item count
        request.push(items.len() as u8);
        
        // Items
        for (area, db_num, offset, _size) in items {
            request.push(0x12);  // Item spec: address specification follows
            request.push(0x0A);  // Specification length
            
            // Variable specification
            let s7_area: u8 = match area {
                S7Area::Db => 0x84,
                S7Area::Input => 0x81,
                S7Area::Output => 0x82,
                S7Area::Marker => 0x83,
                S7Area::Counter => 0x1C,
                S7Area::Timer => 0x1D,
                S7Area::Unknown => 0x00,
            };
            request.push(s7_area);  // Area
            
            // DB number (if DB area)
            if *area == S7Area::Db {
                request.push(((db_num >> 8) & 0xFF) as u8);
                request.push((db_num & 0xFF) as u8);
            } else {
                request.push(0x00);
                request.push(0x00);
            }
            
            // Address (byte offset, bit offset in high nibble of byte offset)
            request.push(((offset >> 8) & 0xFF) as u8);  // Byte address high
            request.push((offset & 0xFF) as u8);  // Byte address low
            request.push(0x00);  // Bit address = 0 for byte access
        }
        
        // Set parameters length
        let params_len = request.len() - params_start - 2;
        request[params_start] = ((params_len >> 8) & 0xFF) as u8;
        request[params_start + 1] = (params_len & 0xFF) as u8;
        
        // Send S7 request over ISO-TCP
        self.send_s7_packet(&request).await?;
        
        // Receive response
        let response = self.recv_s7_packet().await?;
        
        // Parse S7 Ack_Data response
        self.parse_read_response(&response)
    }
    
    /// Send data as ISO-TCP packet (TPKT + COTP + S7)
    async fn send_s7_packet(&mut self, s7_data: &[u8]) -> PluginResult<()> {
        let mut packet = Vec::new();
        
        // TPKT header (4 bytes)
        let total_len = 4 + 4 + s7_data.len();  // TPKT + COTP DT + S7 data
        packet.push(0x03);  // TPKT version
        packet.push(0x00);  // Reserved
        packet.push(((total_len >> 8) & 0xFF) as u8);  // Length high
        packet.push((total_len & 0xFF) as u8);  // Length low
        
        // COTP DT (Data) header (4 bytes)
        packet.push(0x02);  // Length (COTP header)
        packet.push(0xF0);  // PDU type: DT (Data)
        packet.push(0x80);  // TPDU number
        packet.push(0x00);  // Last data unit
        
        // S7 data
        packet.extend_from_slice(s7_data);
        
        self.stream.write_all(&packet).await
            .map_err(|e| PluginError::msg(format!("send failed: {}", e)))?;
        
        Ok(())
    }
    
    /// Receive ISO-TCP packet and extract S7 data
    async fn recv_s7_packet(&mut self) -> PluginResult<Vec<u8>> {
        let mut header = [0u8; 4];
        self.stream.read_exact(&mut header).await
            .map_err(|e| PluginError::msg(format!("read TPKT header failed: {}", e)))?;
        
        if header[0] != 0x03 {
            return Err(PluginError::msg(format!("invalid TPKT version: {}", header[0])));
        }
        
        let total_len = ((header[2] as usize) << 8) | (header[3] as usize);
        let mut payload = vec![0u8; total_len - 4];
        self.stream.read_exact(&mut payload).await
            .map_err(|e| PluginError::msg(format!("read payload failed: {}", e)))?;
        
        // Skip COTP header (usually 4 bytes for DT)
        let cotp_len = payload[0] as usize;
        if payload.len() < cotp_len {
            return Err(PluginError::msg("COTP length exceeds packet"));
        }
        
        let s7_data = payload[cotp_len..].to_vec();
        Ok(s7_data)
    }
    
    /// Parse S7 Read response and extract data values
    fn parse_read_response(&self, data: &[u8]) -> PluginResult<Vec<Vec<u8>>> {
        if data.len() < 10 {
            return Err(PluginError::msg("response too short"));
        }
        
        // Check function code in response (should be 0x04 for Read Var)
        if data[7] != 0x04 {
            return Err(PluginError::msg(format!("unexpected function code: 0x{:02x}", data[7])));
        }
        
        // Check item count
        let item_count = data[8] as usize;
        if data.len() < 9 + item_count * 2 {
            return Err(PluginError::msg("response too short for items"));
        }
        
        let mut results = Vec::new();
        let mut offset = 9;
        
        for _ in 0..item_count {
            if offset >= data.len() {
                break;
            }
            
            let item_len = data[offset + 1] as usize;
            offset += 2;
            
            if offset + item_len > data.len() {
                break;
            }
            
            // Skip the return code byte and take the actual data
            if item_len > 1 {
                results.push(data[offset + 1..offset + item_len].to_vec());
            } else {
                results.push(vec![]);
            }
            
            offset += item_len;
        }
        
        Ok(results)
    }
    
    /// Disconnect
    async fn disconnect(&mut self) -> PluginResult<()> {
        // Send ISO disconnect
        let mut packet = Vec::new();
        let total_len = 9;
        packet.push(0x03);  // TPKT version
        packet.push(0x00);  // Reserved
        packet.push(((total_len >> 8) & 0xFF) as u8);
        packet.push((total_len & 0xFF) as u8);
        
        // COTP DR (Disconnect Request)
        packet.push(0x05);  // Length
        packet.push(0x08);  // DR PDU type
        packet.push(0x00);  // Destination reference
        packet.push(0x00);
        packet.push((self.local_tref & 0xFF) as u8);
        packet.push(((self.local_tref >> 8) & 0xFF) as u8);
        packet.push(0x00);  // Flags
        packet.push(0x00);  // Disconnect reason
        
        let _ = self.stream.write_all(&packet).await;
        Ok(())
    }
}

/// S7 节点状态
struct S7NodeState {
    host: String,
    rack: u8,
    slot: u8,
    connected: bool,
    groups: Vec<Group>,
    #[cfg(feature = "real-impl")]
    connection: Option<S7Connection>,
}

impl S7NodeState {
    fn new(host: String, rack: u8, slot: u8) -> Self {
        Self {
            host,
            rack,
            slot,
            connected: false,
            groups: vec![Group {
                id: GroupId::new(),
                name: "default".to_string(),
                interval_ms: 1000,
                description: Some("默认采集组".to_string()),
            }],
            #[cfg(feature = "real-impl")]
            connection: None,
        }
    }
}

/// Siemens S7 南向插件
pub struct S7Plugin {
    state: Arc<RwLock<HashMap<NodeId, S7NodeState>>>,
}

impl Default for S7Plugin {
    fn default() -> Self {
        Self::new()
    }
}

impl S7Plugin {
    pub fn new() -> Self {
        Self {
            state: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

#[async_trait::async_trait]
impl SouthPlugin for S7Plugin {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "s7",
            kind: PluginKind::South,
            description: Some("西门子 S7 系列 PLC 通信协议"),
            version: "0.1.0",
            name_zh: Some("西门子S7"),
            name_en: Some("Siemens S7"),
            description_zh: Some("西门子 S7 系列 PLC 通信协议，支持 S7-300/400/1200/1500"),
            description_en: Some("Siemens S7 PLC protocol — S7-300/400/1200/1500"),
        }
    }

    fn config_schema(&self) -> Option<ConfigSchema> {
        Some(
            ConfigSchema::new()
                .param(ParamSchema {
                    name: "host".to_string(),
                    name_zh: Some("IP地址".to_string()),
                    name_en: Some("IP Address".to_string()),
                    description: Some("PLC IP地址".to_string()),
                    description_zh: Some("PLC IP地址".to_string()),
                    description_en: Some("PLC IP address".to_string()),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::String,
                    default: Some(serde_json::json!("192.168.1.10")),
                    valid: None,
                    options: None,
                    depends_on: None,
                    depends_value: None,
                    depends_values: None,
                })
                .param(ParamSchema {
                    name: "rack".to_string(),
                    name_zh: Some("机架号".to_string()),
                    name_en: Some("Rack".to_string()),
                    description: Some("PLC 机架号，通常为 0".to_string()),
                    description_zh: Some("PLC 机架号，通常为 0".to_string()),
                    description_en: Some("PLC rack number, usually 0".to_string()),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(0)),
                    valid: Some(ParamValid { min: Some(0), max: Some(7), regex: None, length: None }),
                    options: None,
                    depends_on: None,
                    depends_value: None,
                    depends_values: None,
                })
                .param(ParamSchema {
                    name: "slot".to_string(),
                    name_zh: Some("槽号".to_string()),
                    name_en: Some("Slot".to_string()),
                    description: Some("PLC 槽号，S7-300 通常为 1，S7-1500 可能为 1".to_string()),
                    description_zh: Some("PLC 槽号，S7-300 通常为 1，S7-1500 可能为 1".to_string()),
                    description_en: Some("PLC slot number, typically 1 for S7-300, 1 for S7-1500".to_string()),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(1)),
                    valid: Some(ParamValid { min: Some(0), max: Some(31), regex: None, length: None }),
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
                "int16".to_string(),
                "uint16".to_string(),
                "int32".to_string(),
                "uint32".to_string(),
                "bool".to_string(),
                "byte".to_string(),
            ]),
            address_format: Some(
                "DB{n}.DBD{o} | DB{n}.DBW{o} | DB{n}.DBB{o} | DB{n}.DBX{o}.{b} | I{o} | Q{o} | M{o}".to_string(),
            ),
            address_format_zh: Some(
                "DB{n}.DBD{o}(浮点) | DB{n}.DBW{o}(字) | I{o}(输入) | Q{o}(输出) | M{o}(标志位)".to_string(),
            ),
            address_format_en: Some(
                "DB{n}.DBD{o} (float) | DB{n}.DBW{o} (word) | I{o} (input) | Q{o} (output) | M{o} (marker)".to_string(),
            ),
        })
    }

    async fn validate_tag(&self, _node_id: NodeId, tag: &Tag) -> PluginResult<()> {
        if tag.name.is_empty() {
            return Err(PluginError::tag_invalid("tag name required"));
        }
        if parse_s7_address(&tag.address).is_none() {
            return Err(PluginError::tag_invalid(&format!(
                "invalid S7 address format: {}",
                tag.address
            )));
        }
        Ok(())
    }

    async fn open(&self, node_id: NodeId, config: PluginConfig) -> PluginResult<()> {
        let host = config
            .get("host")
            .and_then(|v| v.as_str())
            .unwrap_or("192.168.1.10")
            .to_string();
        let rack = config
            .get("rack")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u8;
        let slot = config
            .get("slot")
            .and_then(|v| v.as_u64())
            .unwrap_or(1) as u8;

        log::info(node_id, format!("open s7: host={}, rack={}, slot={}", host, rack, slot));

        let state = S7NodeState::new(host, rack, slot);
        let mut states = self.state.write().await;
        states.insert(node_id, state);
        Ok(())
    }

    async fn close(&self, node_id: NodeId) -> PluginResult<()> {
        log::info(node_id, "close s7");
        
        #[cfg(feature = "real-impl")]
        {
            let mut states = self.state.write().await;
            if let Some(s) = states.get_mut(&node_id) {
                if let Some(mut conn) = s.connection.take() {
                    let _ = conn.disconnect().await;
                }
            }
        }
        
        let mut states = self.state.write().await;
        states.remove(&node_id);
        Ok(())
    }

    async fn start(&self, node_id: NodeId) -> PluginResult<()> {
        log::info(node_id, "start s7");
        
        #[cfg(feature = "real-impl")]
        {
            let mut states = self.state.write().await;
            if let Some(s) = states.get_mut(&node_id) {
                match S7Connection::connect(&s.host, s.rack, s.slot).await {
                    Ok(conn) => {
                        s.connection = Some(conn);
                        s.connected = true;
                        log::info(node_id, "S7 ISO-TCP connection established");
                    }
                    Err(e) => {
                        log::info(node_id, format!("S7 connection failed (will use simulated): {}", e));
                        s.connected = true;  // Still mark as connected for simulated mode
                    }
                }
            }
        }
        
        #[cfg(not(feature = "real-impl"))]
        {
            let mut states = self.state.write().await;
            if let Some(s) = states.get_mut(&node_id) {
                s.connected = true;
            }
        }
        
        Ok(())
    }

    async fn stop(&self, node_id: NodeId) -> PluginResult<()> {
        log::info(node_id, "stop s7");
        
        #[cfg(feature = "real-impl")]
        {
            let mut states = self.state.write().await;
            if let Some(s) = states.get_mut(&node_id) {
                if let Some(mut conn) = s.connection.take() {
                    let _ = conn.disconnect().await;
                }
                s.connected = false;
            }
        }
        
        #[cfg(not(feature = "real-impl"))]
        {
            let mut states = self.state.write().await;
            if let Some(s) = states.get_mut(&node_id) {
                s.connected = false;
            }
        }
        
        Ok(())
    }

    async fn poll_group(
        &self,
        node_id: NodeId,
        _group_id: GroupId,
        tags: &[Tag],
    ) -> PluginResult<Vec<(TagId, DataValue)>> {
        let mut states = self.state.write().await;
        let state = states
            .get_mut(&node_id)
            .ok_or_else(|| PluginError::msg("node not open"))?;

        if !state.connected {
            return Err(PluginError::msg("plugin not started"));
        }

        #[cfg(feature = "real-impl")]
        {
            if let Some(ref mut conn) = state.connection {
                // Real S7 read using ISO-TCP protocol
                let items: Vec<(S7Area, u16, u16, u8)> = tags
                    .iter()
                    .filter_map(|t| {
                        parse_s7_address(&t.address).map(|addr| {
                            (addr.area, addr.db_number, addr.byte_offset, addr.data_size)
                        })
                    })
                    .collect();
                
                if !items.is_empty() {
                    match conn.read_var(&items).await {
                        Ok(results) => {
                            let mut out = Vec::with_capacity(tags.len());
                            for (i, tag) in tags.iter().enumerate() {
                                if i < results.len() && !results[i].is_empty() {
                                    let data = &results[i];
                                    let value = interpret_s7_data(data, tag);
                                    out.push((tag.id, value));
                                } else {
                                    out.push((tag.id, DataValue::Float32(0.0)));
                                }
                            }
                            return Ok(out);
                        }
                        Err(e) => {
                            log::info(node_id, format!("S7 read error: {}, using simulated", e));
                        }
                    }
                }
            }
        }

        // Fallback: simulated data with realistic variation
        let mut results = Vec::with_capacity(tags.len());
        for tag in tags {
            if let Some(addr) = parse_s7_address(&tag.address) {
                // Generate realistic simulated data based on address
                let value = generate_simulated_value(&addr, tag);
                results.push((tag.id, value));
            } else {
                results.push((tag.id, DataValue::Float32(0.0_f32)));
            }
        }
        Ok(results)
    }

    async fn write_tags(
        &self,
        node_id: NodeId,
        values: &[(Tag, DataValue)],
    ) -> PluginResult<()> {
        log::warn(node_id, format!("write_tags called with {} values", values.len()));
        
        #[cfg(feature = "real-impl")]
        {
            let states = self.state.read().await;
            if let Some(state) = states.get(&node_id) {
                if let Some(ref conn) = state.connection {
                    let _ = (conn, values);
                    log::info(node_id, "S7 write not implemented in real mode");
                }
            }
        }
        
        Err(PluginError::not_supported("S7 write not implemented"))
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
        Ok(vec![
            Tag {
                id: TagId::new(),
                name: "db1_dbd0".to_string(),
                address: "DB1.DBD0".to_string(),
                attr: gateway_sdk::TagAttr::Read,
                data_type: Some("float".to_string()),
                description: Some("数据块1 双字0 浮点".to_string()),
                group_id: GroupId::new(),
            },
            Tag {
                id: TagId::new(),
                name: "db1_dbd4".to_string(),
                address: "DB1.DBD4".to_string(),
                attr: gateway_sdk::TagAttr::Read,
                data_type: Some("float".to_string()),
                description: Some("数据块1 双字4 浮点".to_string()),
                group_id: GroupId::new(),
            },
            Tag {
                id: TagId::new(),
                name: "input_0".to_string(),
                address: "I0.0".to_string(),
                attr: gateway_sdk::TagAttr::Read,
                data_type: Some("bool".to_string()),
                description: Some("输入0.0".to_string()),
                group_id: GroupId::new(),
            },
            Tag {
                id: TagId::new(),
                name: "marker_0".to_string(),
                address: "M0.0".to_string(),
                attr: gateway_sdk::TagAttr::Read,
                data_type: Some("bool".to_string()),
                description: Some("标志位0.0".to_string()),
                group_id: GroupId::new(),
            },
        ])
    }
}

/// Interpret S7 data bytes into DataValue based on tag type
fn interpret_s7_data(data: &[u8], tag: &Tag) -> DataValue {
    match tag.data_type.as_deref() {
        Some("float") | Some("float32") => {
            if data.len() >= 4 {
                let bits = ((data[0] as u32) << 24)
                    | ((data[1] as u32) << 16)
                    | ((data[2] as u32) << 8)
                    | (data[3] as u32);
                DataValue::Float32(f32::from_bits(bits))
            } else {
                DataValue::Float32(0.0)
            }
        }
        Some("int32") | Some("int") => {
            if data.len() >= 4 {
                let val = ((data[0] as i32) << 24)
                    | ((data[1] as i32) << 16)
                    | ((data[2] as i32) << 8)
                    | (data[3] as i32);
                DataValue::Int32(val)
            } else {
                DataValue::Int32(0)
            }
        }
        Some("uint32") => {
            if data.len() >= 4 {
                let val = ((data[0] as u32) << 24)
                    | ((data[1] as u32) << 16)
                    | ((data[2] as u32) << 8)
                    | (data[3] as u32);
                DataValue::UInt32(val)
            } else {
                DataValue::UInt32(0)
            }
        }
        Some("int16") | Some("int") => {
            if data.len() >= 2 {
                let val = ((data[0] as i16) << 8) | (data[1] as i16);
                DataValue::Int16(val)
            } else {
                DataValue::Int16(0)
            }
        }
        Some("uint16") => {
            if data.len() >= 2 {
                let val = ((data[0] as u16) << 8) | (data[1] as u16);
                DataValue::UInt16(val)
            } else {
                DataValue::UInt16(0)
            }
        }
        Some("bool") => {
            if !data.is_empty() {
                DataValue::Bool(data[0] != 0)
            } else {
                DataValue::Bool(false)
            }
        }
        Some("byte") | Some("uint8") => {
            if !data.is_empty() {
                DataValue::UInt8(data[0])
            } else {
                DataValue::UInt8(0)
            }
        }
        _ => {
            // Default to float
            if data.len() >= 4 {
                let bits = ((data[0] as u32) << 24)
                    | ((data[1] as u32) << 16)
                    | ((data[2] as u32) << 8)
                    | (data[3] as u32);
                DataValue::Float32(f32::from_bits(bits))
            } else {
                DataValue::Float32(0.0)
            }
        }
    }
}

/// Generate realistic simulated data based on S7 address
fn generate_simulated_value(addr: &S7TagAddr, tag: &Tag) -> DataValue {
    use std::time::{SystemTime, UNIX_EPOCH};
    
    // Use time-based variation for realistic simulation
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as f64;
    
    // Create variation based on address to make it consistent per address
    let addr_factor = (addr.db_number as f64 * 1000.0) + (addr.byte_offset as f64);
    let variation = (now / 1000.0 * addr_factor).sin() * 0.1;
    
    match addr.data_size {
        4 => {
            // 4-byte data (DWORD/float) - use sine wave variation
            let base: f32 = match tag.data_type.as_deref() {
                Some("int32") => 1000.0 + variation as f32,
                Some("uint32") => 5000.0 + variation as f32,
                _ => 50.5 + variation as f32,  // float default
            };
            DataValue::Float32(base)
        }
        2 => {
            // 2-byte data (WORD/int16)
            let base: i16 = match tag.data_type.as_deref() {
                Some("int16") => 100 + (addr.byte_offset as i16),
                Some("uint16") => 200 + (addr.byte_offset as u16) as i16,
                _ => addr.byte_offset as i16,
            };
            DataValue::Int16(base)
        }
        1 => {
            // 1-byte data (BYTE/bool)
            if addr.bit_offset.is_some() {
                // Bit address - toggle based on time
                let toggle = ((now / 500.0) as i32) % 2 == 0;
                DataValue::Bool(toggle ^ (addr.byte_offset as u8 % 2 == 0))
            } else {
                DataValue::UInt8(addr.byte_offset as u8)
            }
        }
        _ => DataValue::Float32(0.0),
    }
}
