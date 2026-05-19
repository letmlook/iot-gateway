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
        "description_en": "PROFINET protocol — access IO device data via PNIO"
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
    *guard = Some(ProfinetState { host: host.clone(), device_name, api, slot });
    
    result_to_json(serde_json::json!({
        "status": "connected",
        "host": host,
        "api": api,
        "slot": slot,
        "note": "stub mode — real implementation requires pnIO-stack crate"
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
    let guard = CONN.lock().unwrap();
    if guard.is_none() { return result_to_json(serde_json::json!({"error": "not connected"})); }
    
    let group = if group_json.is_null() { return result_to_json(serde_json::json!({"error": "null"})); };
    let group_str = unsafe { CStr::from_ptr(group_json) }.to_string_lossy();
    let group: serde_json::Value = serde_json::from_str(&group_str).unwrap_or_default();
    
    let mut values = serde_json::Map::new();
    if let Some(tags) = group.get("tags").and_then(|t| t.as_array()) {
        for tag in tags {
            let tag_name = tag.get("name").and_then(|v| v.as_str()).unwrap_or("unknown");
            let address = tag.get("address").and_then(|v| v.as_str()).unwrap_or("");
            
            // PROFINET IO data: slot/subslot/index format e.g., "1/1/0x0001"
            // Or named IO data: ":Q" for output, ":I" for input
            let value = if address.ends_with(":Q") || address.ends_with(":O") {
                serde_json::json!(true) // Output
            } else if address.ends_with(":I") {
                serde_json::json!(false) // Input
            } else if address.contains('/') {
                // Slot/subslot format: "1/1/0x0001"
                let parts: Vec<&str> = address.split('/').collect();
                let slot: u16 = parts.get(0).and_then(|s| s.parse().ok()).unwrap_or(1);
                let idx: u16 = parts
                    .last()
                    .and_then(|s| u16::from_str_radix(s.trim_start_matches("0x"), 16).ok())
                    .unwrap_or(0);
                serde_json::json!((slot as i32) * 100 + (idx as i32))
            } else {
                serde_json::json!(100)
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
                    "quality": "good",
                    "timestamp": chrono::Utc::now().to_rfc3339()
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
    // Valid: slot/subslot format "1/1/0x0001" or named ":I"/":Q"
    let valid = !tag_str.is_empty() && (tag_str.contains('/') || tag_str.ends_with(":I") || tag_str.ends_with(":Q") || tag_str.ends_with(":O"));
    result_to_json(serde_json::json!({ "valid": valid }))
}

#[no_mangle]
pub unsafe extern "C" fn south_write_tags(_handle: *mut c_void, _node_id: *const c_char, _write_json: *const c_char) -> *mut c_char {
    result_to_json(serde_json::json!({ "written": 0 }))
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
            { "name": "analog_1", "address": "2/1/0x0003", "type": "int", "access": "read" }
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
            "slot": { "type": "number", "default": 0 }
        }
    });
    CString::new(schema.to_string()).unwrap().into_raw()
}
