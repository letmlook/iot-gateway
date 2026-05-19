//! Kafka Producer North Plugin — stream data to Kafka topics.
//!
//! Sends collected group data to Apache Kafka topics.

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_void};
use std::sync::Mutex;

struct KafkaState {
    #[allow(dead_code)]
    brokers: String,
    #[allow(dead_code)]
    topic: String,
    #[cfg(feature = "kafka-client")]
    producer: std::sync::Mutex<Option<rdkafka::producer::Producer>>,
}

static KAFKA_STATE: once_cell::sync::Lazy<Mutex<Option<KafkaState>>> =
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
        "name": "kafka",
        "kind": "north",
        "description": "Apache Kafka producer — stream data to Kafka topics",
        "version": "0.1.0",
        "name_zh": "Kafka推送",
        "name_en": "Kafka Producer",
        "description_zh": "将数据流式发送到Kafka主题",
        "description_en": "Stream data to Apache Kafka topics"
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

    let brokers = config
        .get("brokers")
        .and_then(|v| v.as_str())
        .unwrap_or("localhost:9092");
    let topic = config
        .get("topic")
        .and_then(|v| v.as_str())
        .unwrap_or("iot-data");

    #[cfg(feature = "kafka-client")]
    let producer = {
        let conf = rdkafka::config::ClientConfig::new();
        let conf = conf.set("bootstrap.servers", brokers);
        match conf.set("message.timeout.ms", "5000").create::<rdkafka::producer::Producer>() {
            Ok(p) => std::sync::Mutex::new(Some(p)),
            Err(e) => return result_to_json(serde_json::json!({"error": e.to_string()})),
        }
    };

    #[cfg(not(feature = "kafka-client"))]
    let producer = ();

    let state = KafkaState {
        brokers: brokers.to_string(),
        topic: topic.to_string(),
        producer,
    };
    let mut guard = KAFKA_STATE.lock().unwrap();
    *guard = Some(state);

    result_to_json(serde_json::json!({
        "status": "connected",
        "brokers": brokers,
        "topic": topic
    }))
}

#[no_mangle]
pub unsafe extern "C" fn north_close(_handle: *mut c_void, _node_id: *const c_char) -> *mut c_char {
    let mut guard = KAFKA_STATE.lock().unwrap();
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

    let guard = KAFKA_STATE.lock().unwrap();
    let state = match guard.as_ref() {
        Some(s) => s,
        None => return result_to_json(serde_json::json!({"error": "not connected"})),
    };

    #[cfg(feature = "kafka-client")]
    {
        let producer_guard = state.producer.lock().unwrap();
        let producer = match producer_guard.as_ref() {
            Some(p) => p,
            None => return result_to_json(serde_json::json!({"error": "no producer"})),
        };

        let payload = rdkafka::producer::ProducerRecordBuilder::new(
            state.topic.clone(),
            rdkafka::message::OwnedMessage::new(
                data.into_bytes(),
                None,
                format!("{}-{}", state.brokers, chrono::Utc::now().timestamp_millis()).into(),
            ),
        )
        .key(&state.brokers)
        .build();

        match producer.send(payload) {
            Ok((partition, offset)) => {
                tracing::info!(
                    "kafka: sent to {} [{}] offset={}",
                    state.topic,
                    partition,
                    offset
                );
                result_to_json(serde_json::json!({
                    "sent": 1,
                    "partition": partition,
                    "offset": offset
                }))
            }
            Err((e, _)) => {
                tracing::warn!("kafka: failed to send to {}: {}", state.topic, e);
                result_to_json(serde_json::json!({"error": e.to_string()}))
            }
        }
    }

    #[cfg(not(feature = "kafka-client"))]
    {
        tracing::info!(
            "kafka: would send {} to {} ({})",
            data,
            state.topic,
            state.brokers
        );
        result_to_json(serde_json::json!({
            "sent": 1,
            "note": "stub mode - kafka client not available"
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
            "brokers": { "type": "string", "default": "localhost:9092" },
            "topic": { "type": "string", "default": "iot-data" }
        }
    });
    CString::new(schema.to_string()).unwrap().into_raw()
}

#[no_mangle]
pub unsafe extern "C" fn north_connection_status(
    _handle: *mut c_void,
    _node_id: *const c_char,
) -> *mut c_char {
    let guard = KAFKA_STATE.lock().unwrap();
    let status = if guard.is_some() {
        "connected"
    } else {
        "disconnected"
    };
    result_to_json(serde_json::json!({ "status": status }))
}
