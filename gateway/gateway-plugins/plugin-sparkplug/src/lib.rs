//! Sparkplug B north plugin — publishes GroupData to MQTT broker using Sparkplug B payload format.
//!
//! Sparkplug B uses a binary encoding for its payloads.

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_void};
use std::sync::Mutex;
use std::time::Duration;

use chrono::Utc;

struct SparkplugState {
    url: String,
    topic_prefix: String,
    edge_node_id: String,
    device_id: String,
    seq: u64,
    mqtt_client: Option<rumqttc::AsyncClient>,
    mqtt_eventloop: Option<rumqttc::EventLoop>,
    connected: bool,
}

static STATE: once_cell::sync::Lazy<Mutex<Option<SparkplugState>>> =
    once_cell::sync::Lazy::new(|| Mutex::new(None));

fn result_to_json(v: serde_json::Value) -> *mut c_char {
    CString::new(v.to_string()).unwrap().into_raw()
}

// ---------------------------------------------------------------------------
// Sparkplug B binary payload encoding
// ---------------------------------------------------------------------------

fn encode_metric_value(value: &serde_json::Value) -> (u8, Vec<u8>) {
    match value {
        serde_json::Value::Null => (14, vec![]),
        serde_json::Value::Bool(b) => (1, vec![if *b { 1 } else { 0 }]),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                (11, i.to_be_bytes().to_vec())
            } else if let Some(f) = n.as_f64() {
                (6, f.to_be_bytes().to_vec())
            } else {
                (9, n.to_string().into_bytes())
            }
        }
        serde_json::Value::String(s) => (9, s.as_bytes().to_vec()),
        _ => (9, value.to_string().into_bytes()),
    }
}

fn encode_timestamp(ts: i64) -> [u8; 8] {
    ts.to_be_bytes()
}

fn encode_ddata_payload(ts_ms: i64, metrics: &[(String, serde_json::Value)]) -> Vec<u8> {
    let mut buf = vec![];
    buf.extend_from_slice(&encode_timestamp(ts_ms));
    buf.push(metrics.len() as u8);

    for (name, value) in metrics {
        let name_bytes = name.as_bytes();
        buf.push(name_bytes.len() as u8);
        buf.extend_from_slice(name_bytes);
        let (dtype, mut val_bytes) = encode_metric_value(value);
        buf.push(dtype);
        buf.append(&mut val_bytes);
    }
    buf
}

// ---------------------------------------------------------------------------
// MQTT helpers
// ---------------------------------------------------------------------------

fn parse_mqtt_url(url: &str) -> (String, u16) {
    let inner = url
        .trim_start_matches("mqtt://")
        .trim_start_matches("tcp://");
    if let Some(colon) = inner.rfind(':') {
        let port: u16 = inner[colon + 1..].parse().unwrap_or(1883);
        (inner[..colon].to_string(), port)
    } else {
        (inner.to_string(), 1883)
    }
}

fn mqtt_connect(state: &mut SparkplugState) -> std::io::Result<()> {
    use rumqttc::{AsyncClient, MqttOptions};

    let (host, port) = parse_mqtt_url(&state.url);
    let client_id = format!("spbc-{}-{}", state.edge_node_id, state.device_id);
    let mut mqttoptions = MqttOptions::new(client_id, host, port);
    mqttoptions.set_keep_alive(Duration::from_secs(60));

    let (client, eventloop) = AsyncClient::new(mqttoptions, 256);
    state.mqtt_client = Some(client);
    state.mqtt_eventloop = Some(eventloop);
    state.connected = false;
    Ok(())
}

fn mqtt_poll(state: &mut SparkplugState) -> bool {
    use rumqttc::{Event, Packet};

    if let Some(ref mut el) = state.mqtt_eventloop {
        match el.poll() {
            Ok(Event::Incoming(Packet::ConnAck(ack))) => {
                if ack.code == rumqttc::mqttbytes::v4::ConnectReturnCode::Success {
                    state.connected = true;
                }
            }
            Ok(Event::Incoming(Packet::Disconnect)) => {
                state.connected = false;
            }
            Err(_) => {
                state.connected = false;
            }
            _ => {}
        }
    }
    state.connected
}

fn do_mqtt_publish_sync(
    client: &rumqttc::AsyncClient,
    topic: &str,
    payload: Vec<u8>,
    qos: rumqttc::QoS,
) -> std::io::Result<()> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;

    let publish_future = client.publish(topic, qos, false, payload);
    rt.block_on(publish_future)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))
}

fn mqtt_publish(state: &mut SparkplugState, topic: &str, payload: Vec<u8>) -> std::io::Result<()> {
    if state.mqtt_client.is_none() {
        mqtt_connect(state)?;
    }

    let connected = mqtt_poll(state);
    if !connected {
        mqtt_connect(state)?;
        let _ = mqtt_poll(state);
    }

    if let Some(ref client) = state.mqtt_client {
        do_mqtt_publish_sync(client, topic, payload, rumqttc::QoS::AtLeastOnce)
    } else {
        Err(std::io::Error::new(std::io::ErrorKind::NotConnected, "no MQTT client"))
    }
}

// ---------------------------------------------------------------------------
// Plugin FFI exports
// ---------------------------------------------------------------------------

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

    let mut state = SparkplugState {
        url: url.clone(),
        topic_prefix,
        edge_node_id,
        device_id,
        seq: 0,
        mqtt_client: None,
        mqtt_eventloop: None,
        connected: false,
    };

    let connect_result = mqtt_connect(&mut state);
    let mut guard = STATE.lock().unwrap();
    *guard = Some(state);

    match connect_result {
        Ok(()) => {
            let mut g = guard.as_mut().unwrap();
            let connected = mqtt_poll(&mut g);
            result_to_json(serde_json::json!({
                "status": if connected { "connected" } else { "connecting" },
                "url": url,
                "note": "Sparkplug B MQTT initialized"
            }))
        }
        Err(e) => result_to_json(serde_json::json!({
            "status": "initialized (connection deferred)",
            "url": url,
            "error": e.to_string()
        })),
    }
}

#[no_mangle]
pub unsafe extern "C" fn north_close(_handle: *mut c_void, _node_id: *const c_char) -> *mut c_char {
    let mut guard = STATE.lock().unwrap();
    if let Some(ref mut state) = *guard {
        if let Some(ref client) = state.mqtt_client {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap_or_else(|_| tokio::runtime::Builder::new_current_thread().build().unwrap());
            let _ = rt.block_on(client.disconnect());
        }
        state.mqtt_client = None;
        state.mqtt_eventloop = None;
        state.connected = false;
    }
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

    let timestamp = Utc::now().timestamp_millis();
    state.seq = state.seq.wrapping_add(1);

    let topic = format!(
        "{}/{}/DDATA/{}",
        state.topic_prefix, state.edge_node_id, state.device_id
    );

    let mut metrics: Vec<(String, serde_json::Value)> = Vec::new();
    if let Some(obj) = data.as_object() {
        if let Some(tags) = obj.get("tags").and_then(|t| t.as_array()) {
            for tag in tags {
                if let (Some(name), Some(value)) = (
                    tag.get("name").and_then(|v| v.as_str()),
                    tag.get("value"),
                ) {
                    metrics.push((name.to_string(), value.clone()));
                }
            }
        } else {
            for (k, v) in obj {
                if k == "timestamp" || k == "node_id" || k == "group_id" {
                    continue;
                }
                metrics.push((k.clone(), v.clone()));
            }
        }
    }

    let payload = encode_ddata_payload(timestamp, &metrics);
    let publish_result = mqtt_publish(state, &topic, payload);

    match publish_result {
        Ok(()) => result_to_json(serde_json::json!({
            "sent": metrics.len(),
            "topic": topic,
            "seq": state.seq,
            "status": "published"
        })),
        Err(e) => result_to_json(serde_json::json!({
            "sent": 0,
            "topic": topic,
            "seq": state.seq,
            "error": e.to_string(),
            "status": "publish_failed"
        })),
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
    let status = match guard.as_ref() {
        Some(s) => serde_json::json!({
            "status": if s.connected { "connected" } else { "disconnected" },
            "edge_node_id": s.edge_node_id,
            "device_id": s.device_id,
            "seq": s.seq
        }),
        None => serde_json::json!({ "status": "disconnected" }),
    };
    result_to_json(status)
}
