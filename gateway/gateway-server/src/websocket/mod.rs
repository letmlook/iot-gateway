//! WebSocket support for real-time flow monitoring.
//!
//! Provides:
//! - `WsHub`: a global hub that manages WebSocket sessions per flow ID
//! - WebSocket handler for `/ws/flows/:id/live`
//! - Real-time metrics broadcasting from FlowRuntime

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path, State,
    },
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use futures_util::{SinkExt, StreamExt};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};
use uuid::Uuid;

use crate::alarm::AlarmEvent;

/// Global WebSocket hub — manages all live flow WebSocket sessions.
#[derive(Clone)]
pub struct WsHub {
    /// flow_id -> broadcast sender for that flow's live channel
    channels: Arc<RwLock<HashMap<Uuid, broadcast::Sender<FlowLiveEvent>>>>,
    /// global alarm lifecycle broadcast sender
    alarm_tx: broadcast::Sender<AlarmLiveEvent>,
}

impl WsHub {
    pub fn new() -> Self {
        let (alarm_tx, _alarm_rx) = broadcast::channel(256);
        Self {
            channels: Arc::new(RwLock::new(HashMap::new())),
            alarm_tx,
        }
    }

    /// Subscribe to a flow's live events. Returns a receiver.
    pub async fn subscribe(&self, flow_id: Uuid) -> broadcast::Receiver<FlowLiveEvent> {
        let mut channels = self.channels.write().await;
        let sender = channels.entry(flow_id).or_insert_with(|| {
            let (tx, _rx) = broadcast::channel(256);
            tx
        });
        sender.subscribe()
    }

    /// Broadcast an event to all subscribers of a flow.
    pub async fn broadcast(&self, flow_id: Uuid, event: FlowLiveEvent) {
        let channels = self.channels.read().await;
        if let Some(tx) = channels.get(&flow_id) {
            let _ = tx.send(event);
        }
    }

    pub fn subscribe_alarms(&self) -> broadcast::Receiver<AlarmLiveEvent> {
        self.alarm_tx.subscribe()
    }

    pub async fn broadcast_alarm(&self, event: AlarmLiveEvent) {
        let _ = self.alarm_tx.send(event);
    }

    /// Get a snapshot of the number of active subscribers for a flow.
    pub async fn subscriber_count(&self, flow_id: Uuid) -> usize {
        let channels = self.channels.read().await;
        channels.get(&flow_id).map(|tx| tx.len()).unwrap_or(0)
    }
}

impl Default for WsHub {
    fn default() -> Self {
        Self::new()
    }
}

/// Events that can be sent over the WebSocket live channel.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FlowLiveEvent {
    /// Periodic metrics snapshot (every ~1s when connected)
    Metrics {
        flow_id: String,
        flow_name: String,
        status: String,
        metrics: Vec<gateway_sdk::OperatorMetrics>,
    },
    /// Flow status changed (deployed, running, paused, stopped, error)
    StatusChange { flow_id: String, status: String },
    /// Node-level event (e.g., error, warning)
    NodeEvent {
        flow_id: String,
        node_id: String,
        node_name: String,
        event: String,
        detail: Option<String>,
    },
    /// A bound flow processed south group data in the main gateway data path.
    DataProcessed {
        flow_id: String,
        south_node_id: String,
        group_id: String,
        node_name: Option<String>,
        group_name: Option<String>,
        values: serde_json::Value,
    },
    /// Heartbeat / keepalive
    Ping { ts: String },
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "type")]
pub enum AlarmLiveEvent {
    #[serde(rename = "alarm.created")]
    Created { event: AlarmEvent },
    #[serde(rename = "alarm.acknowledged")]
    Acknowledged { event: AlarmEvent },
    #[serde(rename = "alarm.resolved")]
    Resolved { event: AlarmEvent },
}

/// GET /ws/flows/:id/live — WebSocket upgrade for real-time flow monitoring
pub async fn ws_flow_live(
    Path(id): Path<String>,
    ws: WebSocketUpgrade,
    State(state): State<super::state::AppState>,
) -> impl IntoResponse {
    let id = match Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "invalid flow id"})),
            )
                .into_response();
        }
    };

    // Check if flow runtime exists
    let runtimes = state.flow_runtimes.read().await;
    if !runtimes.contains_key(&id) {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "flow runtime not found, deploy first"})),
        )
            .into_response();
    }
    drop(runtimes);

    ws.on_upgrade(move |socket| handle_socket(socket, id, state.ws_hub.clone()))
}

/// Handle an established WebSocket connection.
async fn handle_socket(socket: WebSocket, flow_id: Uuid, hub: WsHub) {
    let (mut sender, mut receiver) = socket.split();

    // Subscribe to the flow's broadcast channel
    let mut rx = hub.subscribe(flow_id).await;

    // Send initial snapshot
    let snapshot = serde_json::json!({
        "type": "connected",
        "flow_id": flow_id.to_string(),
        "ts": chrono::Utc::now().to_rfc3339(),
    });
    if sender
        .send(Message::Text(snapshot.to_string().into()))
        .await
        .is_err()
    {
        return;
    }

    loop {
        tokio::select! {
            // Incoming message from client (e.g., ping)
            msg = receiver.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        // Handle client messages (ping/pong, subscribe to specific nodes, etc.)
                        if text.trim() == r#"{"type":"ping"}"# || text.trim() == r#""ping""# {
                            let pong = serde_json::json!({
                                "type": "pong",
                                "ts": chrono::Utc::now().to_rfc3339(),
                            });
                            let _ = sender.send(Message::Text(pong.to_string().into())).await;
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        break;
                    }
                    Some(Err(_)) | Some(Ok(Message::Ping(_))) | Some(Ok(Message::Binary(_))) | Some(Ok(Message::Pong(_))) => {
                        // Ping/Pong/Binary — ignore, but keep connection open
                    }
                }
            }
            // Incoming broadcast event from the hub
            event = rx.recv() => {
                match event {
                    Ok(evt) => {
                        let json = serde_json::to_string(&evt).unwrap_or_default();
                        if sender.send(Message::Text(json.into())).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(flow_id = %flow_id, n, "websocket lagged behind, skipping events");
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        break;
                    }
                }
            }
        }
    }
}

pub async fn ws_alarms(
    ws: WebSocketUpgrade,
    State(state): State<super::state::AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_alarm_socket(socket, state.ws_hub.clone()))
}

async fn handle_alarm_socket(socket: WebSocket, hub: WsHub) {
    let (mut sender, mut receiver) = socket.split();
    let mut rx = hub.subscribe_alarms();

    let connected = serde_json::json!({
        "type": "connected",
        "channel": "alarms",
        "ts": chrono::Utc::now().to_rfc3339(),
    });
    if sender
        .send(Message::Text(connected.to_string().into()))
        .await
        .is_err()
    {
        return;
    }

    loop {
        tokio::select! {
            msg = receiver.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        if text.trim() == r#"{"type":"ping"}"# || text.trim() == r#""ping""# {
                            let pong = serde_json::json!({
                                "type": "pong",
                                "ts": chrono::Utc::now().to_rfc3339(),
                            });
                            let _ = sender.send(Message::Text(pong.to_string().into())).await;
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Err(_)) | Some(Ok(Message::Ping(_))) | Some(Ok(Message::Binary(_))) | Some(Ok(Message::Pong(_))) => {}
                }
            }
            event = rx.recv() => {
                match event {
                    Ok(evt) => {
                        let json = serde_json::to_string(&evt).unwrap_or_default();
                        if sender.send(Message::Text(json.into())).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(n, "alarm websocket lagged behind, skipping events");
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }
}

/// GET /flows/:id/live/summary — HTTP endpoint returning current live summary for a flow
pub async fn flow_live_summary(
    State(state): State<super::state::AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, super::api::ApiError> {
    let id =
        Uuid::parse_str(&id).map_err(|_| super::api::ApiError::bad_request("invalid flow id"))?;

    let runtimes = state.flow_runtimes.read().await;
    let runtime = runtimes
        .get(&id)
        .ok_or_else(|| super::api::ApiError::not_found("flow runtime not found"))?;

    let metrics: Vec<gateway_sdk::OperatorMetrics> = runtime.metrics();
    let status = format!("{:?}", runtime.status()).to_lowercase();
    let flow_name = runtime.flow_name().to_string();
    let ws_subscribers = state.ws_hub.subscriber_count(id).await;

    Ok(Json(serde_json::json!({
        "flow_id": id.to_string(),
        "flow_name": flow_name,
        "status": status,
        "metrics": metrics,
        "ws_subscribers": ws_subscribers,
        "ts": chrono::Utc::now().to_rfc3339(),
    })))
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_processed_event_serializes_with_source_identifiers() {
        let event = FlowLiveEvent::DataProcessed {
            flow_id: "flow-1".to_string(),
            south_node_id: "south-1".to_string(),
            group_id: "group-1".to_string(),
            node_name: Some("e2e_sim".to_string()),
            group_name: Some("e2e_group".to_string()),
            values: serde_json::json!({
                "temperature": 123.4,
                "humidity": 55.0,
            }),
        };

        let json = serde_json::to_value(event).unwrap();
        assert_eq!(json["type"], "data_processed");
        assert_eq!(json["flow_id"], "flow-1");
        assert_eq!(json["south_node_id"], "south-1");
        assert_eq!(json["group_id"], "group-1");
        assert_eq!(json["node_name"], "e2e_sim");
        assert_eq!(json["group_name"], "e2e_group");
        assert_eq!(json["values"]["temperature"], 123.4);
    }
}
