use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_void};
use std::sync::Mutex;

static CONN: once_cell::sync::Lazy<Mutex<Option<SNMPState>>> =
    once_cell::sync::Lazy::new(|| Mutex::new(None));

struct SNMPState {
    host: String,
    port: u16,
    community: String,
    version: String, // "2c" or "1"
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
        "name": "snmp",
        "kind": "south",
        "description": "SNMP (v1/v2c) network device monitoring — routers, switches, UPS, etc.",
        "version": "0.1.0",
        "name_zh": "SNMP",
        "name_en": "SNMP",
        "description_zh": "SNMP协议监控网络设备，支持路由器/交换机/UPS等",
        "description_en": "SNMP v1/v2c monitoring — routers, switches, UPS, environmental sensors"
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
        .unwrap_or("192.168.1.1")
        .to_string();
    let port = config.get("port").and_then(|v| v.as_u64()).unwrap_or(161) as u16;
    let community = config
        .get("community")
        .and_then(|v| v.as_str())
        .unwrap_or("public")
        .to_string();
    let version = config
        .get("version")
        .and_then(|v| v.as_str())
        .unwrap_or("2c")
        .to_string();

    let mut guard = CONN.lock().unwrap();
    *guard = Some(SNMPState {
        host: host.clone(),
        port,
        community: community.clone(),
        version: version.clone(),
    });

    result_to_json(serde_json::json!({
        "status": "connected",
        "host": host,
        "port": port,
        "community": community,
        "version": version,
        "note": "stub mode — real implementation requires snmp crate or snmp人一体的async library"
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

            // SNMP OID mapping — common OIDs:
            // 1.3.6.1.2.1.1.1.0 = sysDescr
            // 1.3.6.1.2.1.2.2.1.10.N = ifInOctets (interface N)
            // 1.3.6.1.2.1.2.2.1.16.N = ifOutOctets
            // 1.3.6.1.2.1.25.1.1.0 = hrStorageSize
            let value = if address.contains("1.3.6.1.2.1.1.1") {
                serde_json::json!("Linux router")
            } else if address.contains("ifInOctets") || address.contains("ifOutOctets") {
                serde_json::json!(1000000u64)
            } else if address.contains("1.3.6.1.2.1.2.1") || address.contains("ifNumber") {
                serde_json::json!(4u64)
            } else if address.contains("1.3.6.1.2.1.25.2") {
                serde_json::json!(80u64) // hrStorageUsedPercent
            } else {
                serde_json::json!(0u64)
            };
            let tag_type = if address.contains("1.3.6.1.2.1.1.1") {
                "string"
            } else if address.contains("ifInOctets") || address.contains("ifOutOctets") {
                "counter"
            } else {
                "gauge"
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
pub unsafe extern "C" fn south_validate_tag(
    _handle: *mut c_void,
    _node_id: *const c_char,
    tag_json: *const c_char,
) -> *mut c_char {
    let tag = if tag_json.is_null() {
        return result_to_json(serde_json::json!({"valid": false}));
    };
    let tag_str = unsafe { CStr::from_ptr(tag_json) }.to_string_lossy();
    // Valid OID format: dotted numbers e.g., 1.3.6.1.2.1.1.1.0
    let valid = !tag_str.is_empty() && tag_str.split('.').all(|p| p.parse::<u64>().is_ok());
    result_to_json(serde_json::json!({ "valid": valid }))
}

#[no_mangle]
pub unsafe extern "C" fn south_write_tags(
    _handle: *mut c_void,
    _node_id: *const c_char,
    _write_json: *const c_char,
) -> *mut c_char {
    // SNMP SET is supported in v1/v2c for writable OIDs
    result_to_json(serde_json::json!({ "written": 0, "note": "SNMP write not implemented" }))
}

#[no_mangle]
pub unsafe extern "C" fn south_list_groups(
    _handle: *mut c_void,
    _node_id: *const c_char,
) -> *mut c_char {
    result_to_json(serde_json::json!({
        "groups": [
            { "id": "interface", "name": "Network Interfaces", "interval_ms": 1000 },
            { "id": "system", "name": "System Info", "interval_ms": 5000 }
        ]
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
            { "name": "sysDescr", "address": "1.3.6.1.2.1.1.1.0", "type": "string", "access": "read" },
            { "name": "ifNumber", "address": "1.3.6.1.2.1.2.1.0", "type": "gauge", "access": "read" },
            { "name": "ifInOctets_1", "address": "1.3.6.1.2.1.2.2.1.10.1", "type": "counter", "access": "read" },
            { "name": "ifOutOctets_1", "address": "1.3.6.1.2.1.2.2.1.16.1", "type": "counter", "access": "read" }
        ]
    }))
}

#[no_mangle]
pub unsafe extern "C" fn south_config_schema(_handle: *mut c_void) -> *mut c_char {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "host": { "type": "string", "default": "192.168.1.1" },
            "port": { "type": "number", "default": 161 },
            "community": { "type": "string", "default": "public" },
            "version": { "type": "string", "enum": ["1", "2c"], "default": "2c" },
            "timeout_ms": { "type": "number", "default": 3000 }
        }
    });
    CString::new(schema.to_string()).unwrap().into_raw()
}
