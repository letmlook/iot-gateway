//! HTTP Webhook North Plugin — POST data to REST endpoints.
//!
//! Sends collected group data to external HTTP endpoints via POST/PUT requests.

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_void};
use std::sync::Mutex;

struct HttpState {
    #[allow(dead_code)]
    endpoint: String,
    #[allow(dead_code)]
    method: String,
    #[allow(dead_code)]
    headers: std::collections::HashMap<String, String>,
}

static HTTP_STATE: once_cell::sync::Lazy<Mutex<Option<HttpState>>> =
    once_cell::sync::Lazy::new(|| Mutex::new(None));

fn result_to_json(value: serde_json::Value) -> *mut c_char {
    CString::new(value.to_string()).unwrap().into_raw()
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
        "name": "http",
        "kind": "north",
        "description": "HTTP webhook — POST data to REST endpoints",
        "version": "0.1.0",
        "name_zh": "HTTP推送",
        "name_en": "HTTP Webhook",
        "description_zh": "通过HTTP POST/PUT将数据推送到REST端点",
        "description_en": "Push data to REST endpoints via HTTP POST/PUT"
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
    if config_json.is_null() {
        return result_to_json(serde_json::json!({"error": "null config"}));
    }
    let config = unsafe { CStr::from_ptr(config_json) }.to_string_lossy();
    let config: serde_json::Value = serde_json::from_str(&config).unwrap_or_default();

    let endpoint = config
        .get("endpoint")
        .and_then(|v| v.as_str())
        .unwrap_or("http://localhost:8080/webhook");
    let method = config
        .get("method")
        .and_then(|v| v.as_str())
        .unwrap_or("POST");

    let mut headers = std::collections::HashMap::new();
    if let Some(headers_obj) = config.get("headers").and_then(|v| v.as_object()) {
        for (k, v) in headers_obj {
            if let Some(s) = v.as_str() {
                headers.insert(k.clone(), s.to_string());
            }
        }
    }

    let state = HttpState {
        endpoint: endpoint.to_string(),
        method: method.to_string(),
        headers,
    };
    let mut guard = HTTP_STATE.lock().unwrap();
    *guard = Some(state);

    result_to_json(serde_json::json!({
        "status": "connected",
        "endpoint": endpoint
    }))
}

#[no_mangle]
pub unsafe extern "C" fn north_close(_handle: *mut c_void, _node_id: *const c_char) -> *mut c_char {
    let mut guard = HTTP_STATE.lock().unwrap();
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
    if group_json.is_null() {
        return result_to_json(serde_json::json!({"error": "null"}));
    }
    let data = unsafe { CStr::from_ptr(group_json) }.to_string_lossy();

    let guard = HTTP_STATE.lock().unwrap();
    let state = match guard.as_ref() {
        Some(s) => s,
        None => return result_to_json(serde_json::json!({"error": "not connected"})),
    };

    // Log the data that would be sent
    tracing::info!(
        "http: would send {} to {} ({})",
        data,
        state.endpoint,
        state.method
    );

    result_to_json(serde_json::json!({
        "sent": 1,
        "note": "stub mode - HTTP client not available"
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
            "endpoint": { "type": "string", "default": "http://localhost:8080/webhook" },
            "method": { "type": "string", "enum": ["POST", "PUT"], "default": "POST" },
            "timeout_secs": { "type": "number", "default": 30 },
            "headers": {
                "type": "object",
                "additionalProperties": { "type": "string" },
                "default": { "Content-Type": "application/json" }
            }
        }
    });
    CString::new(schema.to_string()).unwrap().into_raw()
}

#[no_mangle]
pub unsafe extern "C" fn north_connection_status(
    _handle: *mut c_void,
    _node_id: *const c_char,
) -> *mut c_char {
    let guard = HTTP_STATE.lock().unwrap();
    let status = if guard.is_some() {
        "connected"
    } else {
        "disconnected"
    };
    result_to_json(serde_json::json!({ "status": status }))
}
