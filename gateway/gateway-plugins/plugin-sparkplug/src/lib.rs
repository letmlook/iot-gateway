use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_void};
use std::sync::Mutex;

struct SparkplugState {
    url: String,
    topic_prefix: String,
    edge_node_id: String,
    device_id: String,
    seq: u64,
}

static STATE: once_cell::sync::Lazy<Mutex<Option<SparkplugState>>> =
    once_cell::sync::Lazy::new(|| Mutex::new(None));

fn result_to_json(v: serde_json::Value) -> *mut c_char {
    CString::new(v.to_string()).unwrap().into_raw()
}

#[no_mangle]
pub unsafe extern "C" fn north_create() -> *mut c_void {
    Box::into_raw(Box::new(())) as *mut c_void
}

#[no_mangle]
pub unsafe extern "C" fn north_destroy(handle: *mut c_void) {
    drop(unsafe { Box::from_raw(handle as *mut ()) });
}

#[no_mangle]
pub unsafe extern "C" fn north_meta() -> *mut c_char {
    let meta = serde_json::json!({
        "name": "sparkplug",
        "kind": "north",
        "description": "Sparkplug B — MQTT payload format for industrial IoT (Cirrus Link)",
        "version": "0.1.0",
        "name_zh": "Sparkplug B",
        "name_en": "Sparkplug B",
        "description_zh": "Sparkplug B MQTT payload格式，应用于工业物联网",
        "description_en": "Sparkplug B payload format over MQTT — industrial IoT standard by Cirrus Link"
    });
    CString::new(meta.to_string()).unwrap().into_raw()
}

#[no_mangle]
pub unsafe extern "C" fn north_free_string(s: *mut c_char) {
    if !s.is_null() {
        drop(unsafe { CString::from_raw(s) });
    }
}

#[no_mangle]
pub unsafe extern "C" fn north_open(
    _handle: *mut c_void,
    config_json: *const c_char,
) -> *mut c_char {
    let config = if config_json.is_null() {
        return result_to_json(serde_json::json!({"error": "null"}));
    };
    let config_str = unsafe { CStr::from_ptr(config_json) }.to_string_lossy();
    let config: serde_json::Value = serde_json::from_str(&config_str).unwrap_or_default();

    let url = config
        .get("url")
        .and_then(|v| v.as_str())
        .unwrap_or("mqtt://localhost:1883")
        .to_string();
    let topic_prefix = config
        .get("topic_prefix")
        .and_then(|v| v.as_str())
        .unwrap_or("spBv1.0")
        .to_string();
    let edge_node_id = config
        .get("edge_node_id")
        .and_then(|v| v.as_str())
        .unwrap_or("gateway1")
        .to_string();
    let device_id = config
        .get("device_id")
        .and_then(|v| v.as_str())
        .unwrap_or("device1")
        .to_string();

    let mut guard = STATE.lock().unwrap();
    *guard = Some(SparkplugState {
        url: url.clone(),
        topic_prefix,
        edge_node_id,
        device_id,
        seq: 0,
    });

    result_to_json(serde_json::json!({
        "status": "connected",
        "url": url,
        "note": "stub mode — real implementation uses mqtt crate with Sparkplug B payload encoding"
    }))
}

#[no_mangle]
pub unsafe extern "C" fn north_close(_handle: *mut c_void, _node_id: *const c_char) -> *mut c_char {
    let mut guard = STATE.lock().unwrap();
    *guard = None;
    result_to_json(serde_json::json!({ "status": "disconnected" }))
}

#[no_mangle]
pub unsafe extern "C" fn north_init(
    _handle: *mut c_void,
    _config_json: *const c_char,
) -> *mut c_char {
    result_to_json(serde_json::json!({}))
}

#[no_mangle]
pub unsafe extern "C" fn north_uninit(
    _handle: *mut c_void,
    _node_id: *const c_char,
) -> *mut c_char {
    result_to_json(serde_json::json!({}))
}

#[no_mangle]
pub unsafe extern "C" fn north_start(
    _handle: *mut c_void,
    _config_json: *const c_char,
) -> *mut c_char {
    result_to_json(serde_json::json!({}))
}

#[no_mangle]
pub unsafe extern "C" fn north_stop(_handle: *mut c_void, _node_id: *const c_char) -> *mut c_char {
    result_to_json(serde_json::json!({}))
}

#[no_mangle]
pub unsafe extern "C" fn north_on_group_data(
    _handle: *mut c_void,
    _node_id: *const c_char,
    group_json: *const c_char,
) -> *mut c_char {
    let mut guard = STATE.lock().unwrap();
    let state = match guard.as_mut() {
        Some(s) => s,
        None => return result_to_json(serde_json::json!({"error": "not connected"})),
    };

    let data = if group_json.is_null() {
        return result_to_json(serde_json::json!({"error": "null"}));
    };
    let data_str = unsafe { CStr::from_ptr(group_json) }.to_string_lossy();
    let data: serde_json::Value = serde_json::from_str(&data_str).unwrap_or_default();

    // Convert to Sparkplug B payload
    let timestamp = chrono::Utc::now().timestamp_millis();
    state.seq = state.seq.wrapping_add(1);

    let mut metrics = Vec::new();
    if let Some(obj) = data.as_object() {
        for (k, v) in obj {
            let (data_type, value) = match v {
                serde_json::Value::Number(n) => {
                    if n.is_i64() {
                        ("Int64", serde_json::json!(n.as_i64()))
                    } else {
                        ("Float", serde_json::json!(n.as_f64().unwrap_or(0.0)))
                    }
                }
                serde_json::Value::Bool(b) => ("Boolean", serde_json::json!(b)),
                serde_json::Value::String(s) => ("String", serde_json::json!(s)),
                _ => ("String", serde_json::json!(v.to_string())),
            };

            metrics.push(serde_json::json!({
                "name": k,
                "timestamp": timestamp,
                "dataType": data_type,
                "value": value
            }));
        }
    }

    let sparkplug_payload = serde_json::json!({
        "timestamp": timestamp,
        "metrics": metrics,
        "seq": state.seq
    });

    // Topic would be: spBv1.0/edge_node_id/DDATA/device_id
    let topic = format!(
        "{}/{}/DDATA/{}",
        state.topic_prefix, state.edge_node_id, state.device_id
    );

    result_to_json(serde_json::json!({
        "sent": metrics.len(),
        "topic": topic,
        "payload": sparkplug_payload,
        "note": "stub mode — real impl publishes to MQTT broker with Sparkplug B payload"
    }))
}

#[no_mangle]
pub unsafe extern "C" fn north_set_subscriptions(
    _handle: *mut c_void,
    _node_id: *const c_char,
    _sub_json: *const c_char,
) -> *mut c_char {
    result_to_json(serde_json::json!({}))
}

#[no_mangle]
pub unsafe extern "C" fn north_config_schema(_handle: *mut c_void) -> *mut c_char {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "url": { "type": "string", "default": "mqtt://localhost:1883" },
            "topic_prefix": { "type": "string", "default": "spBv1.0" },
            "edge_node_id": { "type": "string", "default": "gateway1" },
            "device_id": { "type": "string", "default": "device1" }
        }
    });
    CString::new(schema.to_string()).unwrap().into_raw()
}

#[no_mangle]
pub unsafe extern "C" fn north_connection_status(
    _handle: *mut c_void,
    _node_id: *const c_char,
) -> *mut c_char {
    let guard = STATE.lock().unwrap();
    result_to_json(serde_json::json!({ "status": if guard.is_some() { "connected" } else { "disconnected" } }))
}
