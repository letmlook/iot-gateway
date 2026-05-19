//! InfluxDB North Plugin — write time-series data via InfluxDB Line Protocol

use async_trait::async_trait;
use gateway_sdk::{GroupData, GroupSubscription, NodeId, NorthPlugin, PluginConfig, PluginMeta};
use gateway_sdk::types::PluginKind;
use gateway_sdk::types::DataValue;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::RwLock;

mod ffi;

struct InfluxDBState {
    url: String,
    org: String,
    bucket: String,
    token: String,
    measurement: String,
    client: reqwest::Client,
}

struct PluginState {
    open_nodes: std::collections::HashSet<NodeId>,
    connections: std::collections::HashMap<NodeId, InfluxDBState>,
}

impl Default for PluginState {
    fn default() -> Self {
        Self {
            open_nodes: std::collections::HashSet::new(),
            connections: std::collections::HashMap::new(),
        }
    }
}

pub struct InfluxDBPlugin {
    state: Arc<RwLock<PluginState>>,
}

impl Default for InfluxDBPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl InfluxDBPlugin {
    pub fn new() -> Self {
        Self {
            state: Arc::new(RwLock::new(PluginState::default())),
        }
    }

    fn data_value_to_json_value(v: &DataValue) -> Value {
        match v {
            DataValue::Bool(b) => Value::Bool(*b),
            DataValue::Int8(i) => Value::Number(serde_json::Number::from(*i)),
            DataValue::Int16(i) => Value::Number(serde_json::Number::from(*i)),
            DataValue::Int32(i) => Value::Number(serde_json::Number::from(*i)),
            DataValue::Int64(i) => serde_json::json!(*i),
            DataValue::UInt8(i) => Value::Number(serde_json::Number::from(*i)),
            DataValue::UInt16(i) => Value::Number(serde_json::Number::from(*i)),
            DataValue::UInt32(i) => Value::Number(serde_json::Number::from(*i)),
            DataValue::UInt64(i) => serde_json::json!(*i),
            DataValue::Float32(f) => serde_json::json!(*f),
            DataValue::Float64(f) => serde_json::json!(*f),
            DataValue::String(s) => Value::String(s.clone()),
            DataValue::Bytes(b) => Value::String(format!("{:?}", b)),
        }
    }

    fn build_line_protocol(
        measurement: &str,
        tags: &serde_json::Map<String, Value>,
        fields: &serde_json::Map<String, Value>,
        timestamp: i64,
    ) -> String {
        let mut line = measurement.to_string();

        // Tags (optional)
        if !tags.is_empty() {
            line.push_str(",");
            let tag_strs: Vec<String> = tags
                .iter()
                .filter_map(|(k, v)| v.as_str().map(|s| format!("{}={}", k, s)))
                .collect();
            line.push_str(&tag_strs.join(","));
        }

        // Fields
        line.push(' ');
        let field_strs: Vec<String> = fields
            .iter()
            .map(|(k, v)| match v {
                Value::Number(n) => format!("{}={}", k, n),
                Value::String(s) => format!("{}=\"{}\"", k, s),
                Value::Bool(b) => format!("{}={}", k, b),
                _ => format!("{}=\"{}\"", k, v),
            })
            .collect();
        line.push_str(&field_strs.join(","));

        // Timestamp
        line.push_str(&format!(" {}", timestamp));
        line
    }
}

#[async_trait]
impl NorthPlugin for InfluxDBPlugin {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "influxdb",
            kind: PluginKind::North,
            description: Some("InfluxDB time-series database — write data via InfluxDB Line Protocol"),
            version: "0.1.0",
            name_zh: Some("InfluxDB"),
            name_en: Some("InfluxDB"),
            description_zh: Some("将时序数据写入InfluxDB数据库"),
            description_en: Some("Write time-series data to InfluxDB using InfluxDB Line Protocol"),
        }
    }

    fn config_schema(&self) -> Option<gateway_sdk::ConfigSchema> {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "url": { "type": "string", "default": "http://localhost:8086" },
                "org": { "type": "string", "default": "myorg" },
                "bucket": { "type": "string", "default": "iot" },
                "token": { "type": "string", "default": "" },
                "measurement": { "type": "string", "default": "sensor_data" }
            }
        });
        serde_json::from_value(schema).ok()
    }

    async fn open(&self, node_id: NodeId, config: PluginConfig) -> gateway_sdk::PluginResult<()> {
        let url = config
            .get("url")
            .and_then(|v| v.as_str())
            .unwrap_or("http://localhost:8086")
            .to_string();
        let org = config
            .get("org")
            .and_then(|v| v.as_str())
            .unwrap_or("myorg")
            .to_string();
        let bucket = config
            .get("bucket")
            .and_then(|v| v.as_str())
            .unwrap_or("iot")
            .to_string();
        let token = config
            .get("token")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let measurement = config
            .get("measurement")
            .and_then(|v| v.as_str())
            .unwrap_or("sensor_data")
            .to_string();

        let state = InfluxDBState {
            url: url.clone(),
            org,
            bucket: bucket.clone(),
            token,
            measurement,
            client: reqwest::Client::new(),
        };

        let mut s = self.state.write().await;
        s.open_nodes.insert(node_id);
        s.connections.insert(node_id, state);

        tracing::info!(node_id = %node_id.0, "influxdb open: {} bucket={}", url, bucket);
        Ok(())
    }

    async fn close(&self, node_id: NodeId) -> gateway_sdk::PluginResult<()> {
        let mut s = self.state.write().await;
        s.open_nodes.remove(&node_id);
        s.connections.remove(&node_id);
        tracing::info!(node_id = %node_id.0, "influxdb closed");
        Ok(())
    }

    async fn init(&self, _node_id: NodeId) -> gateway_sdk::PluginResult<()> {
        Ok(())
    }

    async fn uninit(&self, _node_id: NodeId) -> gateway_sdk::PluginResult<()> {
        Ok(())
    }

    async fn start(&self, _node_id: NodeId) -> gateway_sdk::PluginResult<()> {
        Ok(())
    }

    async fn stop(&self, _node_id: NodeId) -> gateway_sdk::PluginResult<()> {
        Ok(())
    }

    async fn set_subscriptions(
        &self,
        _node_id: NodeId,
        _subscriptions: &[GroupSubscription],
    ) -> gateway_sdk::PluginResult<()> {
        Ok(())
    }

    async fn on_group_data(
        &self,
        node_id: NodeId,
        data: Arc<GroupData>,
    ) -> gateway_sdk::PluginResult<()> {
        let s = self.state.read().await;
        let state = match s.connections.get(&node_id) {
            Some(st) => st,
            None => return Err(gateway_sdk::PluginError::msg("not connected")),
        };

        let timestamp = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);

        // Build line protocol from group data values
        let mut tags = serde_json::Map::new();
        let mut fields = serde_json::Map::new();

        // GroupData.values is Vec<(TagId, DataValue)>
        for (tag_id, value) in &data.values {
            let key = format!("{:?}", tag_id);
            let json_val = Self::data_value_to_json_value(value);
            // Tag IDs starting with "tag_" are treated as tags
            if key.starts_with("tag_") {
                tags.insert(key[4..].to_string(), json_val);
            } else {
                fields.insert(key, json_val);
            }
        }

        let line = Self::build_line_protocol(&state.measurement, &tags, &fields, timestamp);

        // Build write URL
        let write_url = format!(
            "{}/api/v2/write?org={}&bucket={}",
            state.url, state.org, state.bucket
        );

        // Send to InfluxDB
        let client = state.client.clone();
        let token = state.token.clone();

        let resp = client
            .post(&write_url)
            .header("Authorization", format!("Token {}", token))
            .header("Content-Type", "text/plain")
            .body(line)
            .send()
            .await;

        match resp {
            Ok(r) if r.status().is_success() => {
                tracing::debug!(node_id = %node_id.0, "influxdb written 1 point");
                Ok(())
            }
            Ok(r) => {
                tracing::warn!(node_id = %node_id.0, status = r.status().as_u16(), "influxdb write failed");
                Err(gateway_sdk::PluginError::msg(format!("write failed: {}", r.status())))
            }
            Err(e) => {
                tracing::error!(node_id = %node_id.0, error = %e, "influxdb request error");
                Err(gateway_sdk::PluginError::msg(e.to_string()))
            }
        }
    }

    async fn connection_status(
        &self,
        node_id: NodeId,
    ) -> Option<serde_json::Value> {
        let s = self.state.read().await;
        let connected = s.connections.contains_key(&node_id);
        Some(serde_json::json!({
            "status": if connected { "connected" } else { "disconnected" }
        }))
    }
}
