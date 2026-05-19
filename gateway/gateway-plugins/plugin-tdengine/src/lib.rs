//! TDengine North Plugin — write time-series data via REST API

use async_trait::async_trait;
use gateway_sdk::{GroupData, GroupSubscription, NodeId, NorthPlugin, PluginConfig, PluginMeta};
use gateway_sdk::types::PluginKind;
use gateway_sdk::types::DataValue;
use std::sync::Arc;
use tokio::sync::RwLock;

mod ffi;
mod state;

struct TDengineState {
    host: String,
    port: u16,
    username: String,
    password: String,
    database: String,
    client: reqwest::Client,
}

struct PluginState {
    open_nodes: std::collections::HashSet<NodeId>,
    connections: std::collections::HashMap<NodeId, TDengineState>,
}

impl Default for PluginState {
    fn default() -> Self {
        Self {
            open_nodes: std::collections::HashSet::new(),
            connections: std::collections::HashMap::new(),
        }
    }
}

pub struct TDenginePlugin {
    state: Arc<RwLock<PluginState>>,
}

impl Default for TDenginePlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl TDenginePlugin {
    pub fn new() -> Self {
        Self {
            state: Arc::new(RwLock::new(PluginState::default())),
        }
    }

    fn data_value_to_string(v: &DataValue) -> String {
        match v {
            DataValue::Bool(b) => if *b { "1" } else { "0" }.to_string(),
            DataValue::Int8(i) => i.to_string(),
            DataValue::Int16(i) => i.to_string(),
            DataValue::Int32(i) => i.to_string(),
            DataValue::Int64(i) => i.to_string(),
            DataValue::UInt8(i) => i.to_string(),
            DataValue::UInt16(i) => i.to_string(),
            DataValue::UInt32(i) => i.to_string(),
            DataValue::UInt64(i) => i.to_string(),
            DataValue::Float32(f) => f.to_string(),
            DataValue::Float64(f) => f.to_string(),
            DataValue::String(s) => format!("'{}'", s),
            DataValue::Bytes(b) => format!("'{:?}'", b),
        }
    }
}

#[async_trait]
impl NorthPlugin for TDenginePlugin {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "tdengine",
            kind: PluginKind::North,
            description: Some("TDengine high-performance time-series database — write via REST API"),
            version: "0.1.0",
            name_zh: Some("TDengine"),
            name_en: Some("TDengine"),
            description_zh: Some("将时序数据写入TDengine高性能时序数据库"),
            description_en: Some("Write time-series data to TDengine via REST API"),
        }
    }

    fn config_schema(&self) -> Option<gateway_sdk::ConfigSchema> {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "host": { "type": "string", "default": "localhost" },
                "port": { "type": "number", "default": 6041 },
                "username": { "type": "string", "default": "root" },
                "password": { "type": "string", "default": "taosdata" },
                "database": { "type": "string", "default": "iot" }
            }
        });
        serde_json::from_value(schema).ok()
    }

    async fn open(&self, node_id: NodeId, config: PluginConfig) -> gateway_sdk::PluginResult<()> {
        let host = config
            .get("host")
            .and_then(|v| v.as_str())
            .unwrap_or("localhost")
            .to_string();
        let port = config
            .get("port")
            .and_then(|v| v.as_u64())
            .unwrap_or(6041) as u16;
        let username = config
            .get("username")
            .and_then(|v| v.as_str())
            .unwrap_or("root")
            .to_string();
        let password = config
            .get("password")
            .and_then(|v| v.as_str())
            .unwrap_or("taosdata")
            .to_string();
        let database = config
            .get("database")
            .and_then(|v| v.as_str())
            .unwrap_or("iot")
            .to_string();

        let state = TDengineState {
            host: host.clone(),
            port,
            username: username.clone(),
            password,
            database: database.clone(),
            client: reqwest::Client::new(),
        };

        let mut s = self.state.write().await;
        s.open_nodes.insert(node_id);
        s.connections.insert(node_id, state);

        tracing::info!(node_id = %node_id.0, "tdengine open: {}:{}/{}", host, port, database);
        Ok(())
    }

    async fn close(&self, node_id: NodeId) -> gateway_sdk::PluginResult<()> {
        let mut s = self.state.write().await;
        s.open_nodes.remove(&node_id);
        s.connections.remove(&node_id);
        tracing::info!(node_id = %node_id.0, "tdengine closed");
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

        let table_name = "sensors";
        let timestamp = chrono::Utc::now().timestamp_millis();

        // GroupData.values is Vec<(TagId, DataValue)>
        let mut cols = Vec::new();
        let mut vals = Vec::new();

        for (tag_id, value) in &data.values {
            cols.push(format!("{:?}", tag_id));
            vals.push(Self::data_value_to_string(value));
        }

        if cols.is_empty() {
            return Ok(());
        }

        let sql = format!(
            "INSERT INTO {}.{} (ts, {}) VALUES ({}, {})",
            state.database,
            table_name,
            cols.join(", "),
            timestamp,
            vals.join(", ")
        );

        let url = format!("http://{}:{}/rest/sql", state.host, state.port);
        let auth = format!("{}:{}", state.username, state.password);
        let auth_b64 = base64::encode(&auth);

        let client = state.client.clone();
        let resp = client
            .post(&url)
            .header("Authorization", format!("Basic {}", auth_b64))
            .header("Content-Type", "application/json")
            .body(sql)
            .send()
            .await;

        match resp {
            Ok(r) if r.status().is_success() => {
                tracing::debug!(node_id = %node_id.0, "tdengine written 1 point");
                Ok(())
            }
            Ok(r) => {
                tracing::warn!(node_id = %node_id.0, status = r.status().as_u16(), "tdengine write failed");
                Err(gateway_sdk::PluginError::msg(format!("write failed: {}", r.status())))
            }
            Err(e) => {
                tracing::error!(node_id = %node_id.0, error = %e, "tdengine request error");
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
