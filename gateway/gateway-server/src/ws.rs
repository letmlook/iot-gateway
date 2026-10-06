//! WebSocket 处理：协议握手、认证、心跳、订阅、分发。
//!
//! 路径：/api/v1/ws
//! 认证：建立连接后客户端立即发送首帧 `{ type: "auth", token: "Bearer xxx" }`
//! - 服务器 10s 内未收到有效认证则关闭连接
//! - 服务器每 60s 发 ping，客户端需在 25s 内回应 pong
//!
//! 订阅 topics：node-values、group-values、system-metrics
//! 帧通道：发送侧 bounded(256)，超限时丢弃最旧帧

use axum::{
    extract::{
        ws::{Message, WebSocket},
        State, WebSocketUpgrade,
    },
    response::IntoResponse,
};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, mpsc};
use tokio::time::{interval, Instant};

use crate::api::{resolve_bearer, ApiError};
use crate::state::AppState;

// ---------------------------------------------------------------------------
// WS 帧类型（与 dto.rs WsClientFrame/WsServerFrame 保持一致）
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum WsClientFrame {
    #[serde(rename = "auth")]
    Auth { token: Option<String> },
    #[serde(rename = "ping")]
    Ping,
    #[serde(rename = "pong")]
    Pong,
    #[serde(rename = "subscribe")]
    Subscribe { topics: Vec<String> },
    #[serde(rename = "unsubscribe")]
    Unsubscribe { topics: Vec<String> },
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
#[allow(dead_code)]
pub enum WsServerFrame {
    #[serde(rename = "auth")]
    Auth {
        success: bool,
        message: Option<String>,
    },
    #[serde(rename = "ping")]
    Ping,
    #[serde(rename = "pong")]
    Pong,
    #[serde(rename = "node-values")]
    NodeValues { data: serde_json::Value },
    #[serde(rename = "group-values")]
    GroupValues { data: serde_json::Value },
    #[serde(rename = "system-metrics")]
    SystemMetrics { data: serde_json::Value },
    #[serde(rename = "error")]
    Error { message: String },
}

impl WsServerFrame {
    fn auth_ok() -> Self {
        WsServerFrame::Auth {
            success: true,
            message: None,
        }
    }
    fn auth_fail(msg: &str) -> Self {
        WsServerFrame::Auth {
            success: false,
            message: Some(msg.to_string()),
        }
    }
    fn ping() -> Self {
        WsServerFrame::Ping
    }
    fn error(msg: &str) -> Self {
        WsServerFrame::Error {
            message: msg.to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// 订阅主题
// ---------------------------------------------------------------------------

bitflags::bitflags! {
    #[derive(Clone, Copy, Default)]
    pub struct Topics: u8 {
        const NODE_VALUES = 1 << 0;
        const GROUP_VALUES = 1 << 1;
        const SYSTEM_METRICS = 1 << 2;
    }
}

impl Topics {
    pub fn from_strs<'a>(iter: impl Iterator<Item = &'a str>) -> Self {
        let mut t = Topics::empty();
        for s in iter {
            match s {
                "node-values" => t |= Topics::NODE_VALUES,
                "group-values" => t |= Topics::GROUP_VALUES,
                "system-metrics" => t |= Topics::SYSTEM_METRICS,
                _ => {}
            }
        }
        t
    }
}

// ---------------------------------------------------------------------------
// HTTP handler：WebSocket upgrade
// ---------------------------------------------------------------------------

/// GET /api/v1/ws
pub async fn ws_handler(
    State(state): State<AppState>,
    ws: WebSocketUpgrade,
) -> Result<impl IntoResponse, ApiError> {
    tracing::debug!("WS connection attempt");
    let state = Arc::new(state);
    Ok(ws.on_upgrade(|socket| handle_socket(socket, state)))
}

// ---------------------------------------------------------------------------
// WebSocket 事件循环
// ---------------------------------------------------------------------------

const CHANNEL_CAPACITY: usize = 256;
const AUTH_TIMEOUT: Duration = Duration::from_secs(10);
const SERVER_PING_INTERVAL: Duration = Duration::from_secs(60);
const CLIENT_PONG_TIMEOUT: Duration = Duration::from_secs(25);

async fn handle_socket(socket: WebSocket, state: Arc<AppState>) {
    let (write_tx, write_rx) = mpsc::channel::<Message>(CHANNEL_CAPACITY);
    let broadcast_tx = state.ws_broadcast_tx.clone();
    let mut rx = broadcast_tx.subscribe();

    // 后台任务：从 broadcast channel 消费帧并写入 WebSocket
    let write_task = tokio::spawn(async move {
        let mut write_rx = write_rx;
        while let Some(msg) = write_rx.recv().await {
            if write_tx.send(msg).await.is_err() {
                break;
            }
        }
    });

    let (sender, receiver) = socket.split();
    let result = handle_frames(sender, receiver, state, broadcast_tx, &mut rx).await;
    let _ = write_task.await;
    if let Err(e) = result {
        tracing::debug!("WS connection error: {}", e);
    }
}

// ---------------------------------------------------------------------------
// 帧处理循环
// ---------------------------------------------------------------------------

async fn handle_frames<S, R>(
    mut sender: S,
    mut receiver: R,
    state: Arc<AppState>,
    broadcast_tx: broadcast::Sender<serde_json::Value>,
    rx: &mut broadcast::Receiver<serde_json::Value>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
where
    S: SinkExt<Message> + Unpin,
    R: StreamExt<Item = Result<Message, axum::Error>> + Unpin,
{
    let mut session = Session {
        authenticated: false,
        subscriptions: Topics::empty(),
        username: None,
    };

    // 10s 认证超时
    let auth_deadline = Instant::now() + AUTH_TIMEOUT;

    // 60s 服务器 ping 定时器
    let mut ping_timer = interval(SERVER_PING_INTERVAL);
    ping_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    // 客户端 pong 超时定时器
    let mut pong_timer = interval(CLIENT_PONG_TIMEOUT);
    pong_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            // 客户端消息
            msg = receiver.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        if let Err(e) = process_client_frame(&text, &state, &mut session, &broadcast_tx).await {
                            tracing::warn!("WS client frame error: {}", e);
                            let _ = sender.send(Message::Text(serde_json::to_string(&WsServerFrame::error(&e.to_string())).unwrap())).await;
                            // 认证失败时关闭连接
                            break;
                        }
                    }
                    Some(Ok(Message::Close(_))) => {
                        tracing::debug!("WS client closed connection");
                        break;
                    }
                    Some(Ok(Message::Ping(data))) => {
                        let _ = sender.send(Message::Pong(data.clone())).await;
                        let _ = sender.send(Message::Text(serde_json::to_string(&WsServerFrame::Pong).unwrap())).await;
                        pong_timer.reset();
                    }
                    Some(Ok(Message::Pong(_))) => {
                        pong_timer.reset();
                    }
                    Some(Err(e)) => {
                        tracing::debug!("WS receive error: {}", e);
                        break;
                    }
                    None => break,
                    _ => {}
                }
            }

            // 服务器定期 ping（认证后）
            _ = ping_timer.tick() => {
                if session.authenticated {
                    let _ = sender.send(Message::Text(serde_json::to_string(&WsServerFrame::ping()).unwrap())).await;
                    pong_timer.reset();
                }
            }

            // 客户端 pong 超时（25s 未响应）
            _ = pong_timer.tick() => {
                if session.authenticated {
                    tracing::warn!("WS: client pong timeout, closing connection");
                    let _ = sender.send(Message::Close(Some(axum::extract::ws::CloseFrame{
                        code: 1000u16,
                        reason: "pong timeout".into(),
                    }))).await;
                    break;
                }
            }

            // 认证超时检查（轮询）
            _ = tokio::time::sleep(Duration::from_millis(500)) => {
                if !session.authenticated && Instant::now() >= auth_deadline {
                    tracing::warn!("WS: auth timeout");
                    let _ = sender.send(Message::Text(serde_json::to_string(&WsServerFrame::auth_fail("auth timeout")).unwrap())).await;
                    let _ = sender.send(Message::Close(Some(axum::extract::ws::CloseFrame{
                        code: 1000u16,
                        reason: "auth timeout".into(),
                    }))).await;
                    break;
                }
            }

            // 广播消息（来自总线）
            data = rx.recv() => {
                match data {
                    Ok(frame_json) => {
                        let typ = frame_json.get("type").and_then(|v| v.as_str()).unwrap_or("");
                        let should_send = match typ {
                            "node-values" => session.subscriptions.contains(Topics::NODE_VALUES),
                            "group-values" => session.subscriptions.contains(Topics::GROUP_VALUES),
                            "system-metrics" => session.subscriptions.contains(Topics::SYSTEM_METRICS),
                            _ => true,
                        };
                        if should_send {
                            let text = serde_json::to_string(&frame_json).unwrap_or_default();
                            let _ = sender.send(Message::Text(text)).await;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        tracing::debug!("WS broadcast lagged {} frames", n);
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Session
// ---------------------------------------------------------------------------

struct Session {
    authenticated: bool,
    subscriptions: Topics,
    username: Option<String>,
}

// ---------------------------------------------------------------------------
// 处理单个客户端帧
// ---------------------------------------------------------------------------

async fn process_client_frame(
    text: &str,
    state: &Arc<AppState>,
    session: &mut Session,
    broadcast_tx: &broadcast::Sender<serde_json::Value>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let frame: WsClientFrame = serde_json::from_str(text)?;

    match frame {
        WsClientFrame::Auth { token } => {
            let ctx = resolve_bearer(state, token.as_deref());
            if let Some(ctx) = ctx {
                session.authenticated = true;
                session.username = ctx.username;
                tracing::info!(username = ?session.username, "WS authenticated");
                let _ = broadcast_tx.send(
                    serde_json::to_string(&WsServerFrame::auth_ok())
                        .unwrap()
                        .into(),
                );
            } else {
                // 认证失败：返回错误让调用方关闭连接
                return Err("invalid token".into());
            }
        }

        WsClientFrame::Ping => {
            tracing::debug!("WS client ping received");
        }

        WsClientFrame::Pong => {
            tracing::debug!("WS client pong received");
        }

        WsClientFrame::Subscribe { topics } => {
            if !session.authenticated {
                return Err("not authenticated".into());
            }
            let new_topics = Topics::from_strs(topics.iter().map(|s| s.as_str()));
            session.subscriptions |= new_topics;
            tracing::debug!(username = ?session.username, "WS subscribed");
        }

        WsClientFrame::Unsubscribe { topics } => {
            if !session.authenticated {
                return Err("not authenticated".into());
            }
            let to_remove = Topics::from_strs(topics.iter().map(|s| s.as_str()));
            session.subscriptions &= !to_remove;
            tracing::debug!(username = ?session.username, "WS unsubscribed");
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// 总线数据注入（由 state 或 manager 调用，发布到所有 WS 会话）
// ---------------------------------------------------------------------------

/// 向所有 WS 会话广播 node-values 数据
#[allow(dead_code)]
pub fn broadcast_node_values(state: &AppState, data: serde_json::Value) {
    let frame = serde_json::json!({ "type": "node-values", "data": data });
    if let Err(e) = state.ws_broadcast_tx.send(frame) {
        tracing::debug!("WS broadcast (node-values) skipped, no receivers: {}", e);
    }
}

/// 向所有 WS 会话广播 group-values 数据
#[allow(dead_code)]
pub fn broadcast_group_values(state: &AppState, data: serde_json::Value) {
    let frame = serde_json::json!({ "type": "group-values", "data": data });
    if let Err(e) = state.ws_broadcast_tx.send(frame) {
        tracing::debug!("WS broadcast (group-values) skipped, no receivers: {}", e);
    }
}

/// 向所有 WS 会话广播 system-metrics 数据
#[allow(dead_code)]
pub fn broadcast_system_metrics(state: &AppState, data: serde_json::Value) {
    let frame = serde_json::json!({ "type": "system-metrics", "data": data });
    if let Err(e) = state.ws_broadcast_tx.send(frame) {
        tracing::debug!("WS broadcast (system-metrics) skipped, no receivers: {}", e);
    }
}
