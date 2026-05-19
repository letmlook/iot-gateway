use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_void};
use std::sync::Mutex;

struct GrpcState {
    url: String,
    connected: bool,
}

static STATE: once_cell::sync::Lazy<Mutex<Option<GrpcState>>> =
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
        "name": "grpc",
        "kind": "north",
        "description": "gRPC — push data to gRPC server via Protocol Buffers",
        "version": "0.1.0",
        "name_zh": "gRPC",
        "name_en": "gRPC",
        "description_zh": "通过gRPC协议推送数据（Protocol Buffers）",
        "description_en": "Push data to gRPC server using Protocol Buffers over HTTP/2"
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
        .unwrap_or("http://localhost:50051")
        .to_string();
    let service = config
        .get("service")
        .and_then(|v| v.as_str())
        .unwrap_or("iot.DataSink");
    let method = config
        .get("method")
        .and_then(|v| v.as_str())
        .unwrap_or("PushData");

    let mut guard = STATE.lock().unwrap();
    *guard = Some(GrpcState {
        url: url.clone(),
        connected: true,
    });

    result_to_json(serde_json::json!({
        "status": "connected",
        "url": url,
        "service": service,
        "method": method,
        "note": "stub mode — real implementation uses tonic for gRPC calls"
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
    let guard = STATE.lock().unwrap();
    let state = match guard.as_ref() {
        Some(s) => s,
        None => return result_to_json(serde_json::json!({"error": "not connected"})),
    };

    let data = if group_json.is_null() {
        return result_to_json(serde_json::json!({"error": "null"}));
    };
    let data_str = unsafe { CStr::from_ptr(group_json) }.to_string_lossy();
    let data: serde_json::Value = serde_json::from_str(&data_str).unwrap_or_default();

    // Real impl: serialize to Protobuf, call gRPC method via tonic
    // Stub: return success with data preview
    let count = data.as_object().map(|m| m.len()).unwrap_or(0);

    result_to_json(serde_json::json!({
        "sent": count,
        "service_url": state.url,
        "note": "stub mode — requires .proto schema and tonic for real gRPC"
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
            "url": { "type": "string", "default": "http://localhost:50051" },
            "service": { "type": "string", "default": "iot.DataSink" },
            "method": { "type": "string", "default": "PushData" },
            "tls": { "type": "boolean", "default": false }
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
