//! Sparkplug B north plugin — publishes GroupData to MQTT broker using Sparkplug B payload format.

#[cfg(feature = "ffi")]
mod ffi;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use gateway_sdk::schema::{ConfigSchema, ParamAttribute, ParamSchema, ParamType};
use gateway_sdk::{GroupData, GroupSubscription, NodeId, NorthPlugin, PluginConfig, PluginMeta};
use gateway_sdk::types::PluginKind;
use gateway_sdk::PluginResult;
use tokio::sync::RwLock;

// --------------------------------------------------------------------------
// Sparkplug B binary payload encoding
// --------------------------------------------------------------------------

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

// --------------------------------------------------------------------------
// Per-node MQTT state (Send + Sync)
// --------------------------------------------------------------------------

struct NodeSparkplugState {
    url: String,
    topic_prefix: String,
    edge_node_id: String,
    device_id: String,
    seq: u64,
    connected: bool,
}

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

/// Connect to MQTT and publish a Sparkplug DDATA payload synchronously.
/// Creates a short-lived connection per call (connection is made, data sent, connection closed).
fn mqtt_publish_ddata(
    url: &str,
    topic_prefix: &str,
    edge_node_id: &str,
    device_id: &str,
    payload: Vec<u8>,
) -> std::io::Result<(bool, u64)> {
    use rumqttc::{AsyncClient, Event, MqttOptions, Packet, QoS};

    let (host, port) = parse_mqtt_url(url);
    let client_id = format!("spbc-{}-{}", edge_node_id, device_id);
    let mut mqttoptions = MqttOptions::new(client_id, host, port);
    mqttoptions.set_keep_alive(Duration::from_secs(5));

    let (client, mut eventloop) = AsyncClient::new(mqttoptions, 256);

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;

    // Connect and wait for ConnAck
    let connected = rt.block_on(async {
        loop {
            match eventloop.poll().await {
                Ok(Event::Incoming(Packet::ConnAck(ack))) => {
                    return ack.code == rumqttc::mqttbytes::v4::ConnectReturnCode::Success;
                }
                Ok(Event::Incoming(Packet::Disconnect)) => return false,
                Err(_) => return false,
                _ => {}
            }
        }
    });

    if !connected {
        return Ok((false, 0));
    }

    // Publish
    let topic = format!("{}/{}/DDATA/{}", topic_prefix, edge_node_id, device_id);
    let publish_result = rt.block_on(client.publish(&topic, QoS::AtLeastOnce, false, payload));

    // Disconnect
    let _ = rt.block_on(client.disconnect());

    match publish_result {
        Ok(()) => Ok((true, 1)),
        Err(e) => Err(std::io::Error::new(std::io::ErrorKind::Other, e.to_string())),
    }
}

// --------------------------------------------------------------------------
// DataValue to serde_json::Value conversion
// --------------------------------------------------------------------------

fn data_value_to_json(v: &gateway_sdk::types::DataValue) -> serde_json::Value {
    use gateway_sdk::types::DataValue;
    match v {
        DataValue::Bool(b) => serde_json::json!(*b),
        DataValue::Int8(v) => serde_json::json!(*v),
        DataValue::Int16(v) => serde_json::json!(*v),
        DataValue::Int32(v) => serde_json::json!(*v),
        DataValue::Int64(v) => serde_json::json!(*v),
        DataValue::UInt8(v) => serde_json::json!(*v),
        DataValue::UInt16(v) => serde_json::json!(*v),
        DataValue::UInt32(v) => serde_json::json!(*v),
        DataValue::UInt64(v) => serde_json::json!(*v),
        DataValue::Float32(v) => serde_json::json!(*v),
        DataValue::Float64(v) => serde_json::json!(*v),
        DataValue::String(s) => serde_json::json!(s),
        DataValue::Bytes(b) => serde_json::json!(b),
    }
}

// --------------------------------------------------------------------------
// SparkplugPlugin
// --------------------------------------------------------------------------

pub struct SparkplugPlugin {
    state: Arc<RwLock<SparkplugState>>,
}

struct SparkplugState {
    open_nodes: std::collections::HashSet<NodeId>,
    subscriptions: HashMap<NodeId, Vec<GroupSubscription>>,
    nodes: HashMap<NodeId, NodeSparkplugState>,
}

impl Default for SparkplugPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl SparkplugPlugin {
    pub fn new() -> Self {
        Self {
            state: Arc::new(RwLock::new(SparkplugState {
                open_nodes: std::collections::HashSet::new(),
                subscriptions: HashMap::new(),
                nodes: HashMap::new(),
            })),
        }
    }
}

#[async_trait::async_trait]
impl NorthPlugin for SparkplugPlugin {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "sparkplug",
            kind: PluginKind::North,
            description: Some("Sparkplug B — MQTT payload format for industrial IoT (Cirrus Link)"),
            version: "0.1.0",
            name_zh: Some("Sparkplug B"),
            name_en: Some("Sparkplug B"),
            description_zh: Some("Sparkplug B MQTT payload格式，应用于工业物联网"),
            description_en: Some("Sparkplug B payload format over MQTT — industrial IoT standard by Cirrus Link"),
        }
    }

    fn config_schema(&self) -> Option<ConfigSchema> {
        Some(
            ConfigSchema::new()
                .param(ParamSchema {
                    name: "url".to_string(),
                    name_zh: Some("服务器地址".to_string()),
                    name_en: Some("Broker URL".to_string()),
                    description: Some("MQTT broker URL, e.g. mqtt://localhost:1883".to_string()),
                    description_zh: Some("MQTT Broker URL，例如 mqtt://localhost:1883".to_string()),
                    description_en: Some("MQTT broker URL, e.g. mqtt://localhost:1883".to_string()),
                    attribute: ParamAttribute::Optional,
                    ty: ParamType::String,
                    default: Some(serde_json::json!("mqtt://localhost:1883")),
                    valid: None,
                    options: None,
                    depends_on: None,
                    depends_value: None,
                    depends_values: None,
                })
                .param(ParamSchema {
                    name: "topic_prefix".to_string(),
                    name_zh: Some("主题前缀".to_string()),
                    name_en: Some("Topic Prefix".to_string()),
                    description: Some("Sparkplug topic prefix".to_string()),
                    description_zh: Some("Sparkplug 主题前缀".to_string()),
                    description_en: Some("Sparkplug topic prefix".to_string()),
                    attribute: ParamAttribute::Optional,
                    ty: ParamType::String,
                    default: Some(serde_json::json!("spBv1.0")),
                    valid: None,
                    options: None,
                    depends_on: None,
                    depends_value: None,
                    depends_values: None,
                })
                .param(ParamSchema {
                    name: "edge_node_id".to_string(),
                    name_zh: Some("边缘节点ID".to_string()),
                    name_en: Some("Edge Node ID".to_string()),
                    description: Some("Sparkplug edge node identifier".to_string()),
                    description_zh: Some("Sparkplug 边缘节点标识符".to_string()),
                    description_en: Some("Sparkplug edge node identifier".to_string()),
                    attribute: ParamAttribute::Optional,
                    ty: ParamType::String,
                    default: Some(serde_json::json!("gateway1")),
                    valid: None,
                    options: None,
                    depends_on: None,
                    depends_value: None,
                    depends_values: None,
                })
                .param(ParamSchema {
                    name: "device_id".to_string(),
                    name_zh: Some("设备ID".to_string()),
                    name_en: Some("Device ID".to_string()),
                    description: Some("Sparkplug device identifier".to_string()),
                    description_zh: Some("Sparkplug 设备标识符".to_string()),
                    description_en: Some("Sparkplug device identifier".to_string()),
                    attribute: ParamAttribute::Optional,
                    ty: ParamType::String,
                    default: Some(serde_json::json!("device1")),
                    valid: None,
                    options: None,
                    depends_on: None,
                    depends_value: None,
                    depends_values: None,
                }),
        )
    }

    async fn open(&self, node_id: NodeId, config: PluginConfig) -> PluginResult<()> {
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

        let node_state = NodeSparkplugState {
            url,
            topic_prefix,
            edge_node_id,
            device_id,
            seq: 0,
            connected: false,
        };

        let mut state = self.state.write().await;
        state.open_nodes.insert(node_id);
        state.nodes.insert(node_id, node_state);

        Ok(())
    }

    async fn close(&self, node_id: NodeId) -> PluginResult<()> {
        let mut state = self.state.write().await;
        state.open_nodes.remove(&node_id);
        state.subscriptions.remove(&node_id);
        state.nodes.remove(&node_id);
        Ok(())
    }

    async fn start(&self, _node_id: NodeId) -> PluginResult<()> {
        Ok(())
    }

    async fn stop(&self, _node_id: NodeId) -> PluginResult<()> {
        Ok(())
    }

    async fn setting(&self, node_id: NodeId, config: PluginConfig) -> PluginResult<()> {
        self.close(node_id).await?;
        self.open(node_id, config).await
    }

    async fn set_subscriptions(
        &self,
        node_id: NodeId,
        subscriptions: &[GroupSubscription],
    ) -> PluginResult<()> {
        let mut state = self.state.write().await;
        state.subscriptions.insert(node_id, subscriptions.to_vec());
        Ok(())
    }

    async fn connection_status(&self, node_id: NodeId) -> Option<serde_json::Value> {
        let state = self.state.read().await;
        let node_state = state.nodes.get(&node_id)?;
        Some(serde_json::json!({
            "status": if node_state.connected { "connected" } else { "disconnected" },
            "edge_node_id": node_state.edge_node_id,
            "device_id": node_state.device_id,
            "seq": node_state.seq,
        }))
    }

    async fn on_group_data(&self, node_id: NodeId, data: Arc<GroupData>) -> PluginResult<()> {
        let timestamp = Utc::now().timestamp_millis();

        let (url, topic_prefix, edge_node_id, device_id) = {
            let mut state = self.state.write().await;
            let node_state = match state.nodes.get_mut(&node_id) {
                Some(s) => s,
                None => return Ok(()),
            };
            node_state.seq = node_state.seq.wrapping_add(1);
            (
                node_state.url.clone(),
                node_state.topic_prefix.clone(),
                node_state.edge_node_id.clone(),
                node_state.device_id.clone(),
            )
        };

        // Build metrics from GroupData
        let mut metrics: Vec<(String, serde_json::Value)> = Vec::new();
        if let Some(ref tag_names) = data.tag_names {
            for (tag_id, value) in &data.values {
                if let Some(name) = tag_names.get(tag_id) {
                    metrics.push((name.clone(), data_value_to_json(value)));
                }
            }
        } else {
            for (tag_id, value) in &data.values {
                metrics.push((format!("{:?}", tag_id), data_value_to_json(value)));
            }
        }

        let payload = encode_ddata_payload(timestamp, &metrics);

        // Publish synchronously (creates short-lived connection)
        let _topic = format!("{}/{}/DDATA/{}", topic_prefix, edge_node_id, device_id);
        let (connected, _) = mqtt_publish_ddata(&url, &topic_prefix, &edge_node_id, &device_id, payload).unwrap_or((false, 0));

        // Update connection status
        let mut state = self.state.write().await;
        if let Some(node_state) = state.nodes.get_mut(&node_id) {
            node_state.connected = connected;
        }

        Ok(())
    }
}
