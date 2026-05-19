use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_void};
use std::sync::Mutex;
use std::time::Duration;

static CONN: once_cell::sync::Lazy<Mutex<Option<SNMPState>>> =
    once_cell::sync::Lazy::new(|| Mutex::new(None));

struct SNMPState {
    host: String,
    port: u16,
    community: String,
    timeout_ms: u64,
}

/// Parse a dotted OID string like "1.3.6.1.2.1.1.1.0" into a Vec<u32>.
fn parse_oid(oid_str: &str) -> Option<Vec<u32>> {
    if oid_str.is_empty() {
        return None;
    }
    let parts: Vec<u32> = oid_str
        .split('.')
        .map(|p| p.parse::<u32>().ok())
        .collect::<Option<_>>()?;
    if parts.is_empty() {
        None
    } else {
        Some(parts)
    }
}

impl SNMPState {
    /// Perform a single SNMP GET for one OID. Returns the JSON value.
    fn get_one(&self, oid: &[u32]) -> Result<serde_json::Value, String> {
        let addr = format!("{}:{}", self.host, self.port);
        let timeout = Duration::from_millis(self.timeout_ms);

        let community_bytes = self.community.as_bytes();
        let mut session = snmp::SyncSession::new(&addr, community_bytes, Some(timeout), 0)
            .map_err(|e| format!("SNMP session: {:?}", e))?;

        let response = session.get(oid).map_err(|e| format!("SNMP get: {:?}", e))?;

        // Varbinds is an iterator
        let mut vb = response.varbinds;
        if let Some((_oid, val)) = vb.next() {
            Ok(snmp_val_to_json(&val))
        } else {
            Ok(serde_json::Value::Null)
        }
    }
}

fn snmp_val_to_json(v: &snmp::Value) -> serde_json::Value {
    match v {
        snmp::Value::OctetString(s) => serde_json::json!(String::from_utf8_lossy(s).to_string()),
        snmp::Value::Integer(i) => serde_json::json!(*i),
        snmp::Value::Unsigned32(u) => serde_json::json!(*u),
        snmp::Value::Counter32(u) => serde_json::json!(*u),
        snmp::Value::Counter64(u) => serde_json::json!(*u),
        snmp::Value::Timeticks(u) => serde_json::json!(*u),
        snmp::Value::IpAddress(a) => {
            serde_json::json!(format!("{}.{}.{}.{}", a[0], a[1], a[2], a[3]))
        }
        snmp::Value::ObjectIdentifier(oid) => {
            // Use Display trait to get dotted string
            serde_json::json!(format!("{}", oid))
        }
        snmp::Value::Null => serde_json::json!(null),
        _ => serde_json::json!(null),
    }
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
    let timeout_ms = config
        .get("timeout_ms")
        .and_then(|v| v.as_u64())
        .unwrap_or(3000);

    let state = SNMPState {
        host: host.clone(),
        port,
        community: community.clone(),
        timeout_ms,
    };

    // Real SNMP ping - try to GET sysDescr
    let sys_descr_oid = [1, 3, 6, 1, 2, 1, 1, 1, 0];
    let ping_result = state.get_one(&sys_descr_oid);

    let mut guard = CONN.lock().unwrap();
    *guard = Some(state);

    match ping_result {
        Ok(serde_json::Value::String(s)) => result_to_json(serde_json::json!({
            "status": "connected",
            "host": host,
            "port": port,
            "community": community,
            "sysDescr": s,
            "note": "SNMP connected"
        })),
        Ok(_) => result_to_json(serde_json::json!({
            "status": "connected",
            "host": host,
            "port": port,
            "community": community,
            "note": "SNMP connected"
        })),
        Err(e) => result_to_json(serde_json::json!({
            "status": "connected (ping failed)",
            "host": host,
            "port": port,
            "community": community,
            "error": e,
            "note": "SNMP connected but ping failed"
        })),
    }
}

#[no_mangle]
pub unsafe extern "C" fn south_close(_handle: *mut c_void, _node_id: *const c_char) -> *mut c_char {
    let mut guard = CONN.lock().unwrap();
    *guard = None;
    result_to_json(serde_json::json!({ "status": "closed" }))
}

#[no_mangle]
pub unsafe extern "C" fn south_init(_handle: *mut c_void, _node_id: *const c_char) -> *mut c_char {
    result_to_json(serde_json::json!({ "status": "init ok" }))
}

#[no_mangle]
pub unsafe extern "C" fn south_uninit(
    _handle: *mut c_void,
    _node_id: *const c_char,
) -> *mut c_char {
    result_to_json(serde_json::json!({ "status": "uninit ok" }))
}

#[no_mangle]
pub unsafe extern "C" fn south_start(_handle: *mut c_void, _node_id: *const c_char) -> *mut c_char {
    result_to_json(serde_json::json!({ "status": "started" }))
}

#[no_mangle]
pub unsafe extern "C" fn south_stop(_handle: *mut c_void, _node_id: *const c_char) -> *mut c_char {
    result_to_json(serde_json::json!({ "status": "stopped" }))
}

#[no_mangle]
pub unsafe extern "C" fn south_poll_group(
    _handle: *mut c_void,
    _node_id: *const c_char,
    group_id: *const c_char,
    tags_json: *const c_char,
) -> *mut c_char {
    let guard = match CONN.lock() {
        Ok(g) => g,
        Err(e) => return result_to_json(serde_json::json!({"error": e.to_string()})),
    };
    let state = match guard.as_ref() {
        Some(s) => s,
        None => return result_to_json(serde_json::json!({"error": "not connected"})),
    };

    let group_id_str = if group_id.is_null() {
        return result_to_json(serde_json::json!({"error": "null group_id"}));
    } else {
        unsafe { CStr::from_ptr(group_id) }
            .to_string_lossy()
            .to_string()
    };

    let tags_str = if tags_json.is_null() {
        return result_to_json(serde_json::json!({"error": "null tags"}));
    } else {
        unsafe { CStr::from_ptr(tags_json) }
            .to_string_lossy()
            .to_string()
    };

    let tags: Vec<serde_json::Value> = serde_json::from_str(&tags_str).unwrap_or_default();

    let mut values: Vec<serde_json::Value> = Vec::new();
    for tag in &tags {
        let address = tag.get("address").and_then(|a| a.as_str()).unwrap_or("");
        let oid = match parse_oid(address) {
            Some(o) => o,
            None => {
                values.push(serde_json::json!({
                    "tag": tag.get("tag").or(tag.get("id")),
                    "address": address,
                    "value": serde_json::Value::Null,
                    "error": "invalid OID"
                }));
                continue;
            }
        };

        match state.get_one(&oid) {
            Ok(val) => {
                values.push(serde_json::json!({
                    "tag": tag.get("tag").or(tag.get("id")),
                    "address": address,
                    "value": val
                }));
            }
            Err(e) => {
                values.push(serde_json::json!({
                    "tag": tag.get("tag").or(tag.get("id")),
                    "address": address,
                    "value": serde_json::Value::Null,
                    "error": e
                }));
            }
        }
    }

    result_to_json(serde_json::json!({
        "group_id": group_id_str,
        "values": values
    }))
}

#[no_mangle]
pub unsafe extern "C" fn south_validate_tag(
    _handle: *mut c_void,
    _node_id: *const c_char,
    tag_json: *const c_char,
) -> *mut c_char {
    let tag_str = if tag_json.is_null() {
        return result_to_json(serde_json::json!({"error": "null"}));
    };
    let tag_str = unsafe { CStr::from_ptr(tag_json) }.to_string_lossy();
    let tag: serde_json::Value = serde_json::from_str(&tag_str).unwrap_or_default();

    let address = tag.get("address").and_then(|v| v.as_str()).unwrap_or("");
    let valid = parse_oid(address).is_some();

    result_to_json(serde_json::json!({
        "valid": valid,
        "address": address
    }))
}

#[no_mangle]
pub unsafe extern "C" fn south_write_tags(
    _handle: *mut c_void,
    _node_id: *const c_char,
    _writes_json: *const c_char,
) -> *mut c_char {
    result_to_json(serde_json::json!({
        "written": 0,
        "error": "SNMP write not implemented"
    }))
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
        "tags": []
    }))
}

#[no_mangle]
pub unsafe extern "C" fn south_config_schema(_handle: *mut c_void) -> *mut c_char {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "host": {
                "type": "string",
                "title": "Host",
                "default": "192.168.1.1",
                "description": "SNMP agent IP address"
            },
            "port": {
                "type": "integer",
                "title": "Port",
                "default": 161,
                "description": "SNMP UDP port"
            },
            "community": {
                "type": "string",
                "title": "Community",
                "default": "public",
                "description": "SNMP v2c community string"
            },
            "timeout_ms": {
                "type": "integer",
                "title": "Timeout (ms)",
                "default": 3000,
                "description": "Request timeout in milliseconds"
            }
        }
    });
    CString::new(schema.to_string()).unwrap().into_raw()
}
