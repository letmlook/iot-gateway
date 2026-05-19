use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_void};
use std::sync::Mutex;

static CONN: once_cell::sync::Lazy<Mutex<Option<FINSState>>> =
    once_cell::sync::Lazy::new(|| Mutex::new(None));

struct FINSState {
    host: String,
    port: u16,
    local_net: u8,
    local_node: u8,
    remote_net: u8,
    remote_node: u8,
    remote_unit: u8,
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
        "name": "omron-fins",
        "kind": "south",
        "description": "Omron FINS protocol — CP/CJ/NJ series PLCs",
        "version": "0.1.0",
        "name_zh": "Omron FINS",
        "name_en": "Omron FINS",
        "description_zh": "Omron FINS协议，支持CP/CJ/NJ系列PLC",
        "description_en": "Omron FINS protocol — CP/CJ/NJ series PLCs via UDP/TCP"
    });
    CString::new(meta.to_string()).unwrap().into_raw()
}

#[no_mangle]
pub unsafe extern "C" fn south_free_string(s: *mut c_char) {
    if !s.is_null() {
        drop(unsafe { CString::from_raw(s) });
    }
}

#[no_mangle]
pub unsafe extern "C" fn south_open(
    _handle: *mut c_void,
    config_json: *const c_char,
) -> *mut c_char {
    let config = if config_json.is_null() {
        return result_to_json(serde_json::json!({"error": "null"}));
    };
    let config_str = unsafe { CStr::from_ptr(config_json) }.to_string_lossy();
    let config: serde_json::Value = serde_json::from_str(&config_str).unwrap_or_default();

    let host = config
        .get("host")
        .and_then(|v| v.as_str())
        .unwrap_or("192.168.1.10")
        .to_string();
    let port = config.get("port").and_then(|v| v.as_u64()).unwrap_or(9600) as u16;
    let local_net = config.get("local_net").and_then(|v| v.as_u64()).unwrap_or(0) as u8;
    let local_node = config.get("local_node").and_then(|v| v.as_u64()).unwrap_or(0) as u8;
    let remote_net = config.get("remote_net").and_then(|v| v.as_u64()).unwrap_or(0) as u8;
    let remote_node = config.get("remote_node").and_then(|v| v.as_u64()).unwrap_or(0) as u8;
    let remote_unit = config.get("remote_unit").and_then(|v| v.as_u64()).unwrap_or(0) as u8;

    let mut guard = CONN.lock().unwrap();
    *guard = Some(FINSState {
        host: host.clone(),
        port,
        local_net,
        local_node,
        remote_net,
        remote_node,
        remote_unit,
    });

    result_to_json(serde_json::json!({
        "status": "connected",
        "host": host,
        "port": port,
        "note": "stub mode — real implementation requires omron-fins crate"
    }))
}

#[no_mangle]
pub unsafe extern "C" fn south_close(_handle: *mut c_void, _node_id: *const c_char) -> *mut c_char {
    let mut guard = CONN.lock().unwrap();
    *guard = None;
    result_to_json(serde_json::json!({ "status": "disconnected" }))
}

#[no_mangle]
pub unsafe extern "C" fn south_init(
    _handle: *mut c_void,
    _config_json: *const c_char,
) -> *mut c_char {
    result_to_json(serde_json::json!({}))
}

#[no_mangle]
pub unsafe extern "C" fn south_uninit(
    _handle: *mut c_void,
    _node_id: *const c_char,
) -> *mut c_char {
    result_to_json(serde_json::json!({}))
}

#[no_mangle]
pub unsafe extern "C" fn south_start(
    _handle: *mut c_void,
    _config_json: *const c_char,
) -> *mut c_char {
    result_to_json(serde_json::json!({}))
}

#[no_mangle]
pub unsafe extern "C" fn south_stop(_handle: *mut c_void, _node_id: *const c_char) -> *mut c_char {
    result_to_json(serde_json::json!({}))
}

#[no_mangle]
pub unsafe extern "C" fn south_poll_group(
    _handle: *mut c_void,
    _node_id: *const c_char,
    group_json: *const c_char,
) -> *mut c_char {
    let guard = CONN.lock().unwrap();
    if guard.is_none() {
        return result_to_json(serde_json::json!({"error": "not connected"}));
    }

    let group = if group_json.is_null() {
        return result_to_json(serde_json::json!({"error": "null"}));
    };
    let group_str = unsafe { CStr::from_ptr(group_json) }.to_string_lossy();
    let group: serde_json::Value = serde_json::from_str(&group_str).unwrap_or_default();

    let mut values = serde_json::Map::new();
    if let Some(tags) = group.get("tags").and_then(|t| t.as_array()) {
        for tag in tags {
            let tag_name = tag
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let address = tag.get("address").and_then(|v| v.as_str()).unwrap_or("");

            // Omron FINS memory areas: D/DM (data memory), W/WR (work area), CIO, H/HR (holding), A/AR, etc.
            let value = if address.starts_with('D') || address.starts_with("DM") {
                serde_json::json!(100)
            } else if address.starts_with('W') || address.starts_with("WR") {
                serde_json::json!(200)
            } else if address.starts_with("CIO") {
                serde_json::json!(false)
            } else if address.starts_with('H') || address.starts_with("HR") {
                serde_json::json!(300)
            } else {
                serde_json::json!(0)
            };
            let tag_type = if address.starts_with("CIO") { "bit" } else { "word" };

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
pub unsafe extern "C" fn south_validate_tag(
    _handle: *mut c_void,
    _node_id: *const c_char,
    tag_json: *const c_char,
) -> *mut c_char {
    let tag = if tag_json.is_null() {
        return result_to_json(serde_json::json!({"valid": false}));
    };
    let tag_str = unsafe { CStr::from_ptr(tag_json) }.to_string_lossy();
    // Valid: DM100, D100, W100, WR100, CIO0.00, H100, HR100, A0.00, AR0.00
    let valid = !tag_str.is_empty()
        && (tag_str.starts_with('D')
            || tag_str.starts_with('W')
            || tag_str.starts_with("CIO")
            || tag_str.starts_with('H')
            || tag_str.starts_with('A')
            || tag_str.starts_with("LR"));
    result_to_json(serde_json::json!({ "valid": valid }))
}

#[no_mangle]
pub unsafe extern "C" fn south_write_tags(
    _handle: *mut c_void,
    _node_id: *const c_char,
    _write_json: *const c_char,
) -> *mut c_char {
    result_to_json(serde_json::json!({ "written": 0 }))
}

#[no_mangle]
pub unsafe extern "C" fn south_list_groups(
    _handle: *mut c_void,
    _node_id: *const c_char,
) -> *mut c_char {
    result_to_json(serde_json::json!({
        "groups": [{ "id": "default", "name": "Default", "interval_ms": 1000 }]
    }))
}

#[no_mangle]
pub unsafe extern "C" fn south_list_tags(
    _handle: *mut c_void,
    _node_id: *const c_char,
    _group_id: *const c_char,
) -> *mut c_char {
    result_to_json(serde_json::json!({
        "tags": [
            { "name": "dm100", "address": "D100", "type": "word", "access": "read" },
            { "name": "wr100", "address": "W100", "type": "word", "access": "read" },
            { "name": "cio_bit", "address": "CIO0.00", "type": "bit", "access": "read" },
            { "name": "hr100", "address": "H100", "type": "word", "access": "read" }
        ]
    }))
}

#[no_mangle]
pub unsafe extern "C" fn south_config_schema(_handle: *mut c_void) -> *mut c_char {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "host": { "type": "string", "default": "192.168.1.10" },
            "port": { "type": "number", "default": 9600 },
            "local_net": { "type": "number", "default": 0 },
            "local_node": { "type": "number", "default": 0 },
            "remote_net": { "type": "number", "default": 0 },
            "remote_node": { "type": "number", "default": 0 },
            "remote_unit": { "type": "number", "default": 0 }
        }
    });
    CString::new(schema.to_string()).unwrap().into_raw()
}
