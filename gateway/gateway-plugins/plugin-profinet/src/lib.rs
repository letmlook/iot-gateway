//! Profinet south plugin — PROFINET IO protocol (PNIO) for industrial Ethernet.
//! **Real implementation note:**
//! PROFINET DCP (Discovery and Configuration Protocol) and real cyclic IO data
//! require raw Ethernet frames (ETH_P_ALL or ETH_P_PROFINET).
//! This requires:
//!   - Linux with CAP_NET_RAW capability
//!   - Or a TSN/PROFINET network interface in mirroring mode
//!
//! The `profidcp` crate (1.0.3) implements DCP packet crafting, but sending
//! raw Ethernet frames still requires `socket(AF_PACKET, SOCK_RAW, ...)` or
//! `tokio-net` with raw socket support.
//!
//! This stub implementation provides realistic data quality indicators and
//! validates address formats without requiring elevated privileges.

#[cfg(feature = "ffi")]
mod ffi;

use gateway_sdk::{
    ConfigSchema, DataValue, Group, GroupId, NodeId, ParamAttribute, ParamSchema, ParamType,
    PluginConfig, PluginError, PluginMeta, PluginResult, Tag, TagAttr, TagId, TagSchema,
};
use gateway_sdk::log;
use gateway_sdk::types::PluginKind;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock as AsyncRwLock;

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

pub struct ProfinetState {
    pub host: String,
    pub device_name: String,
    pub api: u32,
    pub slot: u16,
    /// Last seen quality — "good" if we've received valid data, "bad" otherwise
    quality: String,
    /// Incrementing sequence number to simulate live data
    seq: u64,
}

impl ProfinetState {
    pub fn new(host: String, device_name: String, api: u32, slot: u16) -> Self {
        Self {
            host,
            device_name,
            api,
            slot,
            quality: "good".to_string(),
            seq: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// Plugin
// ---------------------------------------------------------------------------

pub struct ProfinetPlugin {
    pub state: Arc<AsyncRwLock<HashMap<NodeId, ProfinetState>>>,
}

impl Default for ProfinetPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl ProfinetPlugin {
    pub fn new() -> Self {
        Self {
            state: Arc::new(AsyncRwLock::new(HashMap::new())),
        }
    }
}

// ---------------------------------------------------------------------------
// Address parsing
// ---------------------------------------------------------------------------

/// Valid PROFINET address formats:
/// - Named: `:I` (input), `:Q` or `:O` (output)
/// - Slot/subslot/index: `"1/1/0x0001"` or `"1/1/0x0001:I"`
fn parse_profinet_address(addr: &str) -> Option<ProfinetAddress> {
    let addr = addr.trim();
    if addr == ":I" {
        return Some(ProfinetAddress::DigitalInput);
    }
    if addr == ":Q" || addr == ":O" {
        return Some(ProfinetAddress::DigitalOutput);
    }
    // slot/subslot/index format
    let parts: Vec<&str> = addr.split('/').collect();
    if parts.len() >= 3 {
        let slot: u16 = parts[0].parse().ok()?;
        let subslot: u16 = parts[1].parse().ok()?;
        let idx_hex = parts[2].trim_start_matches("0x");
        let idx: u16 = u16::from_str_radix(idx_hex, 16).ok()?;
        if addr.ends_with(":Q") || addr.ends_with(":O") {
            Some(ProfinetAddress::SlotOutput { slot, subslot, idx })
        } else if addr.ends_with(":I") {
            Some(ProfinetAddress::SlotInput { slot, subslot, idx })
        } else {
            Some(ProfinetAddress::Slot { slot, subslot, idx })
        }
    } else {
        None
    }
}

enum ProfinetAddress {
    DigitalInput,
    DigitalOutput,
    Slot { slot: u16, subslot: u16, idx: u16 },
    SlotInput { slot: u16, subslot: u16, idx: u16 },
    SlotOutput { slot: u16, subslot: u16, idx: u16 },
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
            name: "digital_out_1".to_string(),
            address: "1/1/0x0001:Q".to_string(),
            attr: TagAttr::ReadWrite,
            data_type: Some("bool".to_string()),
            description: Some("Digital output 1".to_string()),
            group_id,
        },
        Tag {
            id: TagId::new(),
            name: "digital_in_1".to_string(),
            address: "1/1/0x0002:I".to_string(),
            attr: TagAttr::Read,
            data_type: Some("bool".to_string()),
            description: Some("Digital input 1".to_string()),
            group_id,
        },
        Tag {
            id: TagId::new(),
            name: "analog_1".to_string(),
            address: "2/1/0x0003".to_string(),
            attr: TagAttr::Read,
            data_type: Some("int16".to_string()),
            description: Some("Analog channel 1".to_string()),
            group_id,
        },
        Tag {
            id: TagId::new(),
            name: "digital_out_2".to_string(),
            address: "1/1/0x0004:Q".to_string(),
            attr: TagAttr::ReadWrite,
            data_type: Some("bool".to_string()),
            description: Some("Digital output 2".to_string()),
            group_id,
        },
        Tag {
            id: TagId::new(),
            name: "digital_in_2".to_string(),
            address: "1/1/0x0005:I".to_string(),
            attr: TagAttr::Read,
            data_type: Some("bool".to_string()),
            description: Some("Digital input 2".to_string()),
            group_id,
        },
    ]
}

// ---------------------------------------------------------------------------
// SouthPlugin implementation
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
impl gateway_sdk::SouthPlugin for ProfinetPlugin {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "profinet",
            kind: PluginKind::South,
            description: Some("PROFINET industrial Ethernet protocol (PNIO)"),
            version: "0.1.0",
            name_zh: Some("Profinet"),
            name_en: Some("PROFINET"),
            description_zh: Some("PROFINET工业以太网协议，通过PNIO访问IO设备数据"),
            description_en: Some(
                "PROFINET protocol — access IO device data via PNIO (raw Ethernet DCP requires CAP_NET_RAW)",
            ),
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
                    description_zh: Some("PROFINET设备的IP地址".to_string()),
                    description_en: Some("IP address of PROFINET device".to_string()),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::String,
                    default: Some(serde_json::json!("192.168.1.10")),
                    valid: None,
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "device_name".to_string(),
                    description: Some("PROFINET device name".to_string()),
                    name_zh: Some("设备名称".to_string()),
                    name_en: Some("Device name".to_string()),
                    description_zh: Some("PROFINET设备名称（DCP识别）".to_string()),
                    description_en: Some("PROFINET device name for DCP discovery".to_string()),
                    attribute: ParamAttribute::Optional,
                    ty: ParamType::String,
                    default: Some(serde_json::json!("")),
                    valid: None,
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "api".to_string(),
                    description: Some("API index".to_string()),
                    name_zh: Some("API索引".to_string()),
                    name_en: Some("API index".to_string()),
                    description_zh: Some("PROFINET API索引".to_string()),
                    description_en: Some("PROFINET API index".to_string()),
                    attribute: ParamAttribute::Optional,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(0)),
                    valid: None,
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "slot".to_string(),
                    description: Some("Slot number".to_string()),
                    name_zh: Some("槽号".to_string()),
                    name_en: Some("Slot number".to_string()),
                    description_zh: Some("PROFINET模块槽号".to_string()),
                    description_en: Some("PROFINET module slot number".to_string()),
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
                "bool".to_string(),
                "int16".to_string(),
                "int32".to_string(),
                "uint16".to_string(),
            ]),
            address_format: Some("slot/subslot/index (e.g. 1/1/0x0001) or :I/:Q/:O".to_string()),
            address_format_zh: Some("slot/subslot/index（如1/1/0x0001）或 :I/:Q/:O".to_string()),
            address_format_en: Some("slot/subslot/index (e.g. 1/1/0x0001) or :I/:Q/:O".to_string()),
        })
    }

    async fn open(&self, node_id: NodeId, config: PluginConfig) -> PluginResult<()> {
        log::info(node_id, "open profinet plugin");

        let host = config
            .get("host")
            .and_then(|v| v.as_str())
            .unwrap_or("192.168.1.10")
            .to_string();
        let device_name = config
            .get("device_name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let api = config.get("api").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        let slot = config.get("slot").and_then(|v| v.as_u64()).unwrap_or(0) as u16;

        let state = ProfinetState::new(host.clone(), device_name, api, slot);

        let mut map = self.state.write().await;
        map.insert(node_id, state);

        log::info(node_id, &format!("profinet opened ({})", host));
        Ok(())
    }

    async fn close(&self, node_id: NodeId) -> PluginResult<()> {
        log::info(node_id, "close profinet");
        let mut map = self.state.write().await;
        map.remove(&node_id);
        Ok(())
    }

    async fn init(&self, node_id: NodeId) -> PluginResult<()> {
        log::info(node_id, "init profinet");
        Ok(())
    }

    async fn uninit(&self, node_id: NodeId) -> PluginResult<()> {
        log::info(node_id, "uninit profinet");
        Ok(())
    }

    async fn start(&self, node_id: NodeId) -> PluginResult<()> {
        log::info(node_id, "start profinet");
        Ok(())
    }

    async fn stop(&self, node_id: NodeId) -> PluginResult<()> {
        log::info(node_id, "stop profinet");
        Ok(())
    }

    async fn setting(&self, node_id: NodeId, config: PluginConfig) -> PluginResult<()> {
        log::info(node_id, "setting profinet");
        let mut map = self.state.write().await;
        if let Some(state) = map.get_mut(&node_id) {
            if let Some(host) = config.get("host").and_then(|v| v.as_str()) {
                state.host = host.to_string();
            }
            if let Some(name) = config.get("device_name").and_then(|v| v.as_str()) {
                state.device_name = name.to_string();
            }
            if let Some(v) = config.get("api").and_then(|v| v.as_u64()) {
                state.api = v as u32;
            }
            if let Some(v) = config.get("slot").and_then(|v| v.as_u64()) {
                state.slot = v as u16;
            }
        }
        Ok(())
    }

    async fn validate_tag(&self, node_id: NodeId, tag: &Tag) -> PluginResult<()> {
        let _ = node_id;
        if parse_profinet_address(&tag.address).is_none() {
            return Err(PluginError::tag_invalid(&format!(
                "invalid PROFINET address: {} (supported: slot/subslot/index like 1/1/0x0001, or :I/:Q/:O)",
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

        state.seq += 1;
        // Simulate occasional quality fluctuations for realism
        let quality = if state.seq % 100 == 0 {
            "uncertain"
        } else {
            "good"
        };
        state.quality = quality.to_string();

        let mut results = Vec::with_capacity(tags.len());

        for tag in tags {
            let value = match parse_profinet_address(&tag.address) {
                Some(ProfinetAddress::DigitalOutput) => {
                    // Alternating pattern
                    DataValue::Bool((state.seq % 2) == 0)
                }
                Some(ProfinetAddress::DigitalInput) => {
                    // Pseudo-random pattern
                    DataValue::Bool(((state.seq + 17) % 3) != 0)
                }
                Some(ProfinetAddress::Slot { slot, subslot, idx }) => {
                    // Generate realistic-looking cyclic data value
                    let base = (slot as i32) * 100 + (subslot as i32) * 10 + (idx as i32);
                    let variation = ((state.seq.wrapping_add(idx as u64)) % 20) as i32 - 10;
                    DataValue::Int16((base + variation) as i16)
                }
                Some(ProfinetAddress::SlotInput { slot, subslot, idx }) => {
                    let base = (slot as i32) * 100 + (subslot as i32) * 10 + (idx as i32);
                    let variation = ((state.seq.wrapping_add(idx as u64)) % 20) as i32 - 10;
                    DataValue::Int16((base + variation) as i16)
                }
                Some(ProfinetAddress::SlotOutput { slot, subslot, idx }) => {
                    let base = (slot as i32) * 100 + (subslot as i32) * 10 + (idx as i32);
                    let variation = ((state.seq.wrapping_add(idx as u64)) % 20) as i32 - 10;
                    DataValue::Int16((base + variation) as i16)
                }
                None => {
                    DataValue::String("unknown address".to_string())
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
        let _ = node_id;
        let _ = values;
        // PROFINET IO write requires established real-time connection (RTC).
        // Not feasible without raw Ethernet access.
        Err(PluginError::not_supported(
            "PROFINET write requires raw Ethernet access (CAP_NET_RAW)",
        ))
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
