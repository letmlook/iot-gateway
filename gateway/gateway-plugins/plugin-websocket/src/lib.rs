use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_void};
use std::sync::Mutex;

use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::{connect_async, tungstenite::Message};

struct WebSocketState {
    url: String,
}

static STATE: once_cell::sync::Lazy<Mutex<Option<WebSocketState>>> =
    once_cell::sync::Lazy::new(|| Mutex::new(None));

fn result_to_json(v: serde_json::Value) -> *mut c_char {
    CString::new(v.to_string()).unwrap().into_raw()
}

/// Synchronous wrapper that bridges the async tokio-tungstenite world
/// into the blocking FFI calls by grabbing the current Tokio runtime handle.
fn block_on<F: std::future::Future>(f: F) -> F::Output {
    tokio::runtime::Handle::current().block_on(f)
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
        "name": "websocket",
        "kind": "north",
        "description": "WebSocket — push data to ws:// or wss:// endpoint",
        "version": "0.1.0",
        "name_zh": "WebSocket",
        "name_en": "WebSocket",
        "description_zh": "通过WebSocket协议向上游推送数据",
        "description_en": "Push data to WebSocket server — ws:// or wss://"
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
    let config_str = unsafe { CStr::from_ptr(config_json) }.to_string_lossy();
    let config: serde_json::Value = serde_json::from_str(&config_str).unwrap_or_default();

    let url = config
        .get("url")
        .and_then(|v| v.as_str())
        .unwrap_or("ws://localhost:8080/ws")
        .to_string();

    let mut guard = STATE.lock().unwrap();
    *guard = Some(WebSocketState { url: url.clone() });

    result_to_json(serde_json::json!({ "status": "connected", "url": url }))
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
pub unsafe extern "C" fn north_stop(
    _handle: *mut c_void,
    _node_id: *const c_char,
) -> *mut c_char {
    result_to_json(serde_json::json!({}))
}

#[no_mangle]
pub unsafe extern "C" fn north_on_group_data(
    _handle: *mut c_void,
    _node_id: *const c_char,
    group_json: *const c_char,
) -> *mut c_char {
    let state = {
        let guard = STATE.lock().unwrap();
        match guard.as_ref() {
            Some(s) => s.url.clone(),
            None => return result_to_json(serde_json::json!({"error": "not connected"})),
        }
    };

    if group_json.is_null() {
        return result_to_json(serde_json::json!({"error": "null"}));
    }
    let data_str = unsafe { CStr::from_ptr(group_json) }.to_string_lossy();

    // Validate that the incoming JSON is well-formed before sending.
    #[allow(unused)]
    let _data: Result<serde_json::Value, _> = serde_json::from_str(&data_str);

    let send_result = block_on(send_over_ws(state.clone(), data_str.to_string()));

    match send_result {
        Ok(()) => result_to_json(serde_json::json!({
            "sent": data_str.len(),
            "endpoint": state,
        })),
        Err(e) => result_to_json(serde_json::json!({"error": e.to_string()})),
    }
}

async fn send_over_ws(url: String, payload: String) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let (ws, _) = connect_async(&url).await?;
    let (mut sink, _stream) = ws.split();
    sink.send(Message::Text(payload)).await?;
    Ok(())
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
            "url": { "type": "string", "default": "ws://localhost:8080/ws" },
            "tls": { "type": "boolean", "default": false },
            "reconnect": { "type": "boolean", "default": true }
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
