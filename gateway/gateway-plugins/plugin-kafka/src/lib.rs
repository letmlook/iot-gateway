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
    producer: std::sync::Mutex<Option<rdkafka::producer::FutureProducer>>,
}

type KafkaHandleState = Mutex<Option<KafkaState>>;

fn result_to_json(value: serde_json::Value) -> *mut c_char {
    CString::new(value.to_string()).unwrap().into_raw()
}

#[no_mangle]
pub unsafe extern "C" fn north_create() -> *mut c_void {
    Box::into_raw(Box::new(Mutex::new(None::<KafkaState>))) as *mut c_void
}

#[no_mangle]
pub unsafe extern "C" fn north_destroy(handle: *mut c_void) {
    if !handle.is_null() {
        drop(unsafe { Box::from_raw(handle as *mut KafkaHandleState) });
    }
}

unsafe fn handle_state<'a>(handle: *mut c_void) -> Result<&'a KafkaHandleState, *mut c_char> {
    if handle.is_null() {
        return Err(result_to_json(serde_json::json!({"error": "null handle"})));
    }
    Ok(unsafe { &*(handle as *mut KafkaHandleState) })
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
    handle: *mut c_void,
    config_json: *const c_char,
) -> *mut c_char {
    let state_slot = match unsafe { handle_state(handle) } {
        Ok(state) => state,
        Err(error) => return error,
    };
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
        let mut conf = rdkafka::config::ClientConfig::new();
        conf.set("bootstrap.servers", brokers);
        match conf.set("message.timeout.ms", "5000").create::<rdkafka::producer::FutureProducer>() {
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
    let mut guard = state_slot.lock().unwrap();
    *guard = Some(state);

    result_to_json(serde_json::json!({
        "status": "connected",
        "brokers": brokers,
        "topic": topic
    }))
}

#[no_mangle]
pub unsafe extern "C" fn north_close(handle: *mut c_void, _node_id: *const c_char) -> *mut c_char {
    let state_slot = match unsafe { handle_state(handle) } {
        Ok(state) => state,
        Err(error) => return error,
    };
    let mut guard = state_slot.lock().unwrap();
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
    handle: *mut c_void,
    _node_id: *const c_char,
    group_json: *const c_char,
) -> *mut c_char {
    let state_slot = match unsafe { handle_state(handle) } {
        Ok(state) => state,
        Err(error) => return error,
    };
    if group_json.is_null() {
        return result_to_json(serde_json::json!({"error": "null"}));
    }
    let data = unsafe { CStr::from_ptr(group_json) }.to_string_lossy();

    let guard = state_slot.lock().unwrap();
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

        let key = format!("{}-{}", state.brokers, chrono::Utc::now().timestamp_millis());
        let record = rdkafka::producer::FutureRecord::to(&state.topic)
            .payload(data.as_ref())
            .key(&key);

        match producer.send_result(record) {
            Ok(_) => {
                tracing::info!("kafka: enqueued message to {}", state.topic);
                result_to_json(serde_json::json!({ "sent": 1 }))
            }
            Err((e, _)) => {
                tracing::warn!("kafka: failed to enqueue message to {}: {}", state.topic, e);
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
    handle: *mut c_void,
    _node_id: *const c_char,
) -> *mut c_char {
    let state_slot = match unsafe { handle_state(handle) } {
        Ok(state) => state,
        Err(error) => return error,
    };
    let guard = state_slot.lock().unwrap();
    let status = if guard.is_some() {
        "connected"
    } else {
        "disconnected"
    };
    result_to_json(serde_json::json!({ "status": status }))
}

#[cfg(test)]
mod tests {
    use super::*;

    unsafe fn take_json(ptr: *mut c_char) -> serde_json::Value {
        let raw = CString::from_raw(ptr).into_string().unwrap();
        serde_json::from_str(&raw).unwrap()
    }

    #[test]
    fn kafka_state_is_isolated_per_handle() {
        unsafe {
            let handle_one = north_create();
            let handle_two = north_create();
            let config = CString::new(r#"{"brokers":"localhost:9092","topic":"one"}"#).unwrap();
            let _ = take_json(north_open(handle_one, config.as_ptr()));

            let status_one = take_json(north_connection_status(handle_one, std::ptr::null()));
            let status_two = take_json(north_connection_status(handle_two, std::ptr::null()));

            assert_eq!(status_one["status"], "connected");
            assert_eq!(status_two["status"], "disconnected");

            north_destroy(handle_one);
            north_destroy(handle_two);
        }
    }
}
