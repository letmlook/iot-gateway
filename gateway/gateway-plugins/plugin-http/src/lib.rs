//! HTTP Webhook North Plugin — POST data to REST endpoints.
//!
//! Sends collected group data to external HTTP endpoints via POST/PUT requests.

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_void};
use std::sync::Mutex;

#[derive(Clone)]
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

    #[cfg(feature = "http-client")]
    {
        let client = match reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
        {
            Ok(c) => c,
            Err(e) => return result_to_json(serde_json::json!({"error": e.to_string()})),
        };

        let mut req = match state.method.to_uppercase().as_str() {
            "PUT" => client.put(state.endpoint.as_str()),
            _ => client.post(state.endpoint.as_str()),
        };

        for (k, v) in &state.headers {
            req = req.header(k.as_str(), v.as_str());
        }

        match req.json(&data).send() {
            Ok(resp) => {
                tracing::info!(
                    "http: sent {} bytes to {}, status={}",
                    data.len(),
                    state.endpoint,
                    resp.status()
                );
                result_to_json(serde_json::json!({
                    "sent": 1,
                    "status": resp.status().as_u16()
                }))
            }
            Err(e) => {
                tracing::warn!("http: failed to send to {}: {}", state.endpoint, e);
                result_to_json(serde_json::json!({"error": e.to_string()}))
            }
        }
    }

    #[cfg(not(feature = "http-client"))]
    {
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

#[derive(Default)]
pub struct HttpPlugin {
    states: std::sync::Mutex<std::collections::HashMap<gateway_sdk::NodeId, HttpState>>,
    subscriptions: std::sync::Mutex<
        std::collections::HashMap<gateway_sdk::NodeId, Vec<gateway_sdk::GroupSubscription>>,
    >,
}

impl HttpPlugin {
    pub fn new() -> Self {
        Self::default()
    }

    fn state_from_config(config: gateway_sdk::PluginConfig) -> HttpState {
        let endpoint = config
            .get("endpoint")
            .and_then(|v| v.as_str())
            .unwrap_or("http://localhost:8080/webhook")
            .to_string();
        let method = config
            .get("method")
            .and_then(|v| v.as_str())
            .unwrap_or("POST")
            .to_string();
        let mut headers = std::collections::HashMap::new();
        if let Some(headers_obj) = config.get("headers").and_then(|v| v.as_object()) {
            for (key, value) in headers_obj {
                if let Some(value) = value.as_str() {
                    headers.insert(key.clone(), value.to_string());
                }
            }
        }
        HttpState {
            endpoint,
            method,
            headers,
        }
    }
}

#[async_trait::async_trait]
impl gateway_sdk::NorthPlugin for HttpPlugin {
    fn meta(&self) -> gateway_sdk::PluginMeta {
        gateway_sdk::PluginMeta {
            name: "http",
            kind: gateway_sdk::PluginKind::North,
            description: Some("HTTP webhook — POST data to REST endpoints"),
            version: "0.1.0",
            name_zh: Some("HTTP推送"),
            name_en: Some("HTTP Webhook"),
            description_zh: Some("通过HTTP POST/PUT将数据推送到REST端点"),
            description_en: Some("Push data to REST endpoints via HTTP POST/PUT"),
        }
    }

    async fn open(
        &self,
        node_id: gateway_sdk::NodeId,
        config: gateway_sdk::PluginConfig,
    ) -> gateway_sdk::PluginResult<()> {
        self.states
            .lock()
            .unwrap()
            .insert(node_id, Self::state_from_config(config));
        Ok(())
    }

    async fn close(&self, node_id: gateway_sdk::NodeId) -> gateway_sdk::PluginResult<()> {
        self.states.lock().unwrap().remove(&node_id);
        self.subscriptions.lock().unwrap().remove(&node_id);
        Ok(())
    }

    async fn setting(
        &self,
        node_id: gateway_sdk::NodeId,
        config: gateway_sdk::PluginConfig,
    ) -> gateway_sdk::PluginResult<()> {
        self.states
            .lock()
            .unwrap()
            .insert(node_id, Self::state_from_config(config));
        Ok(())
    }

    async fn set_subscriptions(
        &self,
        node_id: gateway_sdk::NodeId,
        subscriptions: &[gateway_sdk::GroupSubscription],
    ) -> gateway_sdk::PluginResult<()> {
        self.subscriptions
            .lock()
            .unwrap()
            .insert(node_id, subscriptions.to_vec());
        Ok(())
    }

    async fn connection_status(&self, node_id: gateway_sdk::NodeId) -> Option<serde_json::Value> {
        let connected = self.states.lock().unwrap().contains_key(&node_id);
        Some(serde_json::json!({
            "status": if connected { "connected" } else { "disconnected" }
        }))
    }

    async fn on_group_data(
        &self,
        node_id: gateway_sdk::NodeId,
        data: std::sync::Arc<gateway_sdk::GroupData>,
    ) -> gateway_sdk::PluginResult<()> {
        let state = self
            .states
            .lock()
            .unwrap()
            .get(&node_id)
            .cloned()
            .ok_or_else(|| {
                gateway_sdk::PluginError::connection_failed("http plugin is not open")
            })?;

        #[cfg(feature = "http-client")]
        {
            let payload = serde_json::to_value(&*data)
                .map_err(|e| gateway_sdk::PluginError::msg(e.to_string()))?;
            tokio::task::spawn_blocking(move || {
                let client = reqwest::blocking::Client::builder()
                    .timeout(std::time::Duration::from_secs(30))
                    .build()
                    .map_err(|e| gateway_sdk::PluginError::connection_failed(e.to_string()))?;
                let mut req = match state.method.to_uppercase().as_str() {
                    "PUT" => client.put(state.endpoint.as_str()),
                    _ => client.post(state.endpoint.as_str()),
                };
                for (key, value) in &state.headers {
                    req = req.header(key.as_str(), value.as_str());
                }
                req.json(&payload)
                    .send()
                    .map_err(|e| gateway_sdk::PluginError::connection_failed(e.to_string()))?;
                Ok(())
            })
            .await
            .map_err(|e| gateway_sdk::PluginError::msg(e.to_string()))?
        }

        #[cfg(not(feature = "http-client"))]
        {
            tracing::info!(node_id = ?node_id, bytes = serde_json::to_string(&*data).unwrap_or_default().len(), "http plugin stub send");
            Ok(())
        }
    }
}

#[cfg(test)]
mod north_plugin_tests {
    use super::*;
    use gateway_sdk::NorthPlugin;

    #[tokio::test]
    async fn http_plugin_implements_north_plugin_lifecycle() {
        let plugin = HttpPlugin::new();
        let node_id = gateway_sdk::NodeId::new();
        let mut config = gateway_sdk::PluginConfig::new();
        config.insert(
            "endpoint".to_string(),
            serde_json::json!("http://localhost:8080/webhook"),
        );

        plugin.open(node_id, config).await.unwrap();
        assert_eq!(
            plugin.connection_status(node_id).await.unwrap()["status"],
            "connected"
        );
        plugin.set_subscriptions(node_id, &[]).await.unwrap();
        plugin.close(node_id).await.unwrap();
        assert_eq!(
            plugin.connection_status(node_id).await.unwrap()["status"],
            "disconnected"
        );
    }
}
