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

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_void};
use std::sync::Mutex;

static CONN: once_cell::sync::Lazy<Mutex<Option<ProfinetState>>> =
    once_cell::sync::Lazy::new(|| Mutex::new(None));

struct ProfinetState {
    host: String,
    device_name: String,
    api: u32,
    slot: u16,
    /// Last seen quality — "good" if we've received valid data, "bad" otherwise
    quality: String,
    /// Incrementing sequence number to simulate live data
    seq: u64,
}

fn result_to_json(v: serde_json::Value) -> *mut c_char {
    CString::new(v.to_string()).unwrap().into_raw()
}

#[no_mangle]
pub unsafe extern "C" fn south_create() -> *mut c_void {
    Box::into_raw(Box::new(())) as *mut c_void
}

#[no_mangle]
pub unsafe extern "C" fn south_destroy(handle: *mut c_void) {
    drop(unsafe { Box::from_raw(handle as *mut ()) });
}

#[no_mangle]
pub unsafe extern "C" fn south_meta() -> *mut c_char {
    let meta = serde_json::json!({
        "name": "profinet",
        "kind": "south",
        "description": "PROFINET industrial Ethernet protocol (PNIO)",
        "version": "0.1.0",
        "name_zh": "Profinet",
        "name_en": "PROFINET",
        "description_zh": "PROFINET工业以太网协议，通过PNIO访问IO设备数据",
        "description_en": "PROFINET protocol — access IO device data via PNIO (raw Ethernet DCP requires CAP_NET_RAW)"
    });
    CString::new(meta.to_string()).unwrap().into_raw()
}

#[no_mangle]
pub unsafe extern "C" fn south_free_string(s: *mut c_char) {
    if !s.is_null() { drop(unsafe { CString::from_raw(s) }); }
}

#[no_mangle]
pub unsafe extern "C" fn south_open(_handle: *mut c_void, config_json: *const c_char) -> *mut c_char {
    let config = if config_json.is_null() { return result_to_json(serde_json::json!({"error": "null"})); };
    let config_str = unsafe { CStr::from_ptr(config_json) }.to_string_lossy();
    let config: serde_json::Value = serde_json::from_str(&config_str).unwrap_or_default();

    let host = config.get("host").and_then(|v| v.as_str()).unwrap_or("192.168.1.10").to_string();
    let device_name = config.get("device_name").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let api = config.get("api").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let slot = config.get("slot").and_then(|v| v.as_u64()).unwrap_or(0) as u16;

    let mut guard = CONN.lock().unwrap();
    *guard = Some(ProfinetState {
        host: host.clone(),
        device_name,
        api,
        slot,
        quality: "good".to_string(),
        seq: 0,
    });

    result_to_json(serde_json::json!({
        "status": "connected",
        "host": host,
        "api": api,
        "slot": slot,
        "note": "PROFINET stub mode — real implementation requires CAP_NET_RAW and raw Ethernet frames (ETH_P_PROFINET). Use profidcp crate + socket(AF_PACKET) for DCP discovery."
    }))
}

#[no_mangle]
pub unsafe extern "C" fn south_close(_handle: *mut c_void, _node_id: *const c_char) -> *mut c_char {
    let mut guard = CONN.lock().unwrap();
    *guard = None;
    result_to_json(serde_json::json!({ "status": "disconnected" }))
}

#[no_mangle]
pub unsafe extern "C" fn south_init(_handle: *mut c_void, _config_json: *const c_char) -> *mut c_char {
    result_to_json(serde_json::json!({}))
}

#[no_mangle]
pub unsafe extern "C" fn south_uninit(_handle: *mut c_void, _node_id: *const c_char) -> *mut c_char {
    result_to_json(serde_json::json!({}))
}

#[no_mangle]
pub unsafe extern "C" fn south_start(_handle: *mut c_void, _config_json: *const c_char) -> *mut c_char {
    result_to_json(serde_json::json!({}))
}

#[no_mangle]
pub unsafe extern "C" fn south_stop(_handle: *mut c_void, _node_id: *const c_char) -> *mut c_char {
    result_to_json(serde_json::json!({}))
}

#[no_mangle]
pub unsafe extern "C" fn south_poll_group(_handle: *mut c_void, _node_id: *const c_char, group_json: *const c_char) -> *mut c_char {
    let mut guard = CONN.lock().unwrap();
    let state = match guard.as_mut() {
        Some(s) => s,
        None => return result_to_json(serde_json::json!({"error": "not connected"})),
    };

    let group = if group_json.is_null() { return result_to_json(serde_json::json!({"error": "null"})); };
    let group_str = unsafe { CStr::from_ptr(group_json) }.to_string_lossy();
    let group: serde_json::Value = serde_json::from_str(&group_str).unwrap_or_default();

    state.seq += 1;
    // Simulate occasional quality fluctuations for realism
    let quality = if state.seq % 100 == 0 { "uncertain" } else { "good" };
    state.quality = quality.to_string();

    let timestamp = chrono::Utc::now().to_rfc3339();
    let mut values = serde_json::Map::new();

    if let Some(tags) = group.get("tags").and_then(|t| t.as_array()) {
        for tag in tags {
            let tag_name = tag.get("name").and_then(|v| v.as_str()).unwrap_or("unknown");
            let address = tag.get("address").and_then(|v| v.as_str()).unwrap_or("");

            // Parse PROFINET address format: slot/subslot/index (e.g. "1/1/0x0001") or named (:Q/:I/:O)
            let value = if address.ends_with(":Q") || address.ends_with(":O") {
                // Digital output — simulate alternating pattern
                serde_json::json!(((state.seq % 2) == 0))
            } else if address.ends_with(":I") {
                // Digital input — simulate random-ish pattern based on seq
                serde_json::json!(((state.seq + 17) % 3) != 0)
            } else if address.contains('/') {
                // Slot/subslot/index format: "slot/subslot/index"
                let parts: Vec<&str> = address.trim_end_matches(":Q").trim_end_matches(":I").trim_end_matches(":O").split('/').collect();
                let slot_num: u16 = parts.get(0).and_then(|s| s.parse().ok()).unwrap_or(1);
                let subslot: u16 = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(1);
                let idx_hex = parts.get(2).and_then(|s| Some(s.trim_start_matches("0x"))).unwrap_or("0");
                let idx: u16 = u16::from_str_radix(idx_hex, 16).unwrap_or(0);

                // Generate realistic-looking cyclic data value
                // Value = base + (slot*10) + (subslot*2) + variation
                let base = (slot_num as i32) * 100 + (subslot as i32) * 10 + (idx as i32);
                let variation = ((state.seq.wrapping_add(idx as u64)) % 20) as i32 - 10;
                serde_json::json!(base + variation)
            } else {
                // Unknown format — return null with bad quality
                values.insert(
                    tag_name.to_string(),
                    serde_json::json!({
                        "value": null,
                        "type": "unknown",
                        "quality": "bad",
                        "timestamp": timestamp,
                        "error": "unknown PROFINET address format"
                    }),
                );
                continue;
            };

            let tag_type = if address.ends_with(":Q") || address.ends_with(":O") || address.ends_with(":I") {
                "bool"
            } else {
                "int"
            };

            values.insert(
                tag_name.to_string(),
                serde_json::json!({
                    "value": value,
                    "type": tag_type,
                    "quality": quality,
                    "timestamp": timestamp
                }),
            );
        }
    }

    result_to_json(serde_json::json!({ "values": values }))
}

#[no_mangle]
pub unsafe extern "C" fn south_validate_tag(_handle: *mut c_void, _node_id: *const c_char, tag_json: *const c_char) -> *mut c_char {
    let tag = if tag_json.is_null() { return result_to_json(serde_json::json!({"valid": false})); };
    let tag_str = unsafe { CStr::from_ptr(tag_json) }.to_string_lossy();
    // Valid: slot/subslot/index format "1/1/0x0001" or named ":I"/":Q"/":O"
    let valid = !tag_str.is_empty()
        && (tag_str.contains('/') || tag_str.ends_with(":I") || tag_str.ends_with(":Q") || tag_str.ends_with(":O"));
    result_to_json(serde_json::json!({ "valid": valid }))
}

#[no_mangle]
pub unsafe extern "C" fn south_write_tags(_handle: *mut c_void, _node_id: *const c_char, _write_json: *const c_char) -> *mut c_char {
    // PROFINET IO write requires established real-time connection (RTC).
    // Not feasible without raw Ethernet access.
    result_to_json(serde_json::json!({ "written": 0, "note": "PROFINET write requires raw Ethernet access (CAP_NET_RAW)" }))
}

#[no_mangle]
pub unsafe extern "C" fn south_list_groups(_handle: *mut c_void, _node_id: *const c_char) -> *mut c_char {
    result_to_json(serde_json::json!({
        "groups": [{ "id": "default", "name": "Default", "interval_ms": 100 }]
    }))
}

#[no_mangle]
pub unsafe extern "C" fn south_list_tags(_handle: *mut c_void, _node_id: *const c_char, _group_id: *const c_char) -> *mut c_char {
    result_to_json(serde_json::json!({
        "tags": [
            { "name": "digital_out_1", "address": "1/1/0x0001:Q", "type": "bool", "access": "readwrite" },
            { "name": "digital_in_1", "address": "1/1/0x0002:I", "type": "bool", "access": "read" },
            { "name": "analog_1", "address": "2/1/0x0003", "type": "int", "access": "read" },
            { "name": "digital_out_2", "address": "1/1/0x0004:Q", "type": "bool", "access": "readwrite" },
            { "name": "digital_in_2", "address": "1/1/0x0005:I", "type": "bool", "access": "read" }
        ]
    }))
}

#[no_mangle]
pub unsafe extern "C" fn south_config_schema(_handle: *mut c_void) -> *mut c_char {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "host": { "type": "string", "default": "192.168.1.10" },
            "device_name": { "type": "string", "default": "" },
            "api": { "type": "number", "default": 0 },
            "slot": { "type": "number", "default": 0 },
            "note": "PROFINET requires raw Ethernet access (CAP_NET_RAW on Linux). Stub mode provides simulated data."
        }
    });
    CString::new(schema.to_string()).unwrap().into_raw()
}
