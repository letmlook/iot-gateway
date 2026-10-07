//! WebSocket 处理：协议握手、认证、心跳、订阅、分发。
//!
//! 路径：/api/v1/ws
//! 认证：建立连接后客户端立即发送首帧 `{ type: "auth", token: "Bearer xxx" }`
//! - 服务器 10s 内未收到有效认证则关闭连接
//! - 服务器每 60s 发 ping，客户端需在 25s 内回应 pong
//!
//! 数据推送：每连接独立的 bus.subscribe_all() tap，按订阅过滤器推送 values 帧；
//! 节点状态每 ws_snapshot_interval_ms 推送一次全量快照。
//!
//! 优雅停机：main.rs 的停机序列在停止节点之前调用 WsShutdown::notify_and_drain()，
//! 所有连接任务经 watch 广播被唤醒，向客户端发送 Close(4002, "server shutting down")
//! 后立即退出；排水窗口有界，避免进程退出前关闭帧来不及送达。

use axum::{
    extract::{
        ws::{Message, WebSocket},
        State, WebSocketUpgrade,
    },
    response::IntoResponse,
};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, mpsc, watch};
use tokio::time::{interval, Instant};

use crate::api::{resolve_bearer, ApiError};
use crate::state::AppState;

// ---------------------------------------------------------------------------
// WS 帧类型（与 dto.rs 保持一致，使用 camelCase）
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
    Subscribe {
        topics: Vec<String>,
        #[serde(rename = "nodeIds", skip_serializing_if = "Option::is_none")]
        node_ids: Option<Vec<String>>,
        #[serde(rename = "groupIds", skip_serializing_if = "Option::is_none")]
        group_ids: Option<Vec<String>>,
    },
    #[serde(rename = "unsubscribe")]
    Unsubscribe { topics: Vec<String> },
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
#[allow(dead_code)]
pub enum WsServerFrame {
    /// 认证响应（auth 成功后发送）
    #[serde(rename = "hello")]
    Hello {
        version: String,
        build_date: String,
        features: Vec<String>,
    },
    /// 认证失败
    #[serde(rename = "auth")]
    Auth {
        success: bool,
        message: Option<String>,
    },
    /// 服务器 ping
    #[serde(rename = "ping")]
    Ping,
    /// 客户端响应 pong
    #[serde(rename = "pong")]
    Pong,
    /// values 帧：来自总线旁路订阅
    #[serde(rename = "values")]
    Values { data: WsValuesData },
    /// nodes 快照帧：服务端周期推送
    #[serde(rename = "nodes")]
    Nodes { nodes: Vec<WsNodeSnapshot> },
    /// 错误帧
    #[serde(rename = "error")]
    Error { message: String },
}

/// values 帧数据体
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WsValuesData {
    #[serde(rename = "nodeId")]
    pub node_id: String,
    #[serde(rename = "nodeName")]
    pub node_name: Option<String>,
    #[serde(rename = "groupId")]
    pub group_id: String,
    #[serde(rename = "groupName")]
    pub group_name: Option<String>,
    pub values: Vec<WsTagValue>,
}

/// 单个 tag 的值（DataValue 编码与 REST 完全一致）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WsTagValue {
    #[serde(rename = "tagId")]
    pub tag_id: String,
    #[serde(rename = "tagName")]
    pub tag_name: Option<String>,
    pub value: serde_json::Value,
}

/// nodes 快照帧中的节点
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WsNodeSnapshot {
    pub id: String,
    pub name: String,
    pub kind: String,
    #[serde(rename = "pluginName")]
    pub plugin_name: String,
    pub state: String,
    #[serde(rename = "connectionStatus", skip_serializing_if = "Option::is_none")]
    pub connection_status: Option<serde_json::Value>,
}

impl WsServerFrame {
    fn hello() -> Self {
        WsServerFrame::Hello {
            version: env!("CARGO_PKG_VERSION").to_string(),
            build_date: env!("BUILD_DATE").to_string(),
            features: vec!["values".to_string(), "nodes".to_string()],
        }
    }
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
    fn values(data: WsValuesData) -> Self {
        WsServerFrame::Values { data }
    }
    fn nodes(nodes: Vec<WsNodeSnapshot>) -> Self {
        WsServerFrame::Nodes { nodes }
    }
}

// ---------------------------------------------------------------------------
// 订阅主题
// ---------------------------------------------------------------------------

bitflags::bitflags! {
    #[derive(Clone, Copy, Default)]
    pub struct Topics: u8 {
        const VALUES = 1 << 0;
        const NODES = 1 << 1;
    }
}

impl Topics {
    pub fn from_strs<'a>(iter: impl Iterator<Item = &'a str>) -> Self {
        let mut t = Topics::empty();
        for s in iter {
            match s {
                "values" => t |= Topics::VALUES,
                "nodes" => t |= Topics::NODES,
                _ => {}
            }
        }
        t
    }
}

// ---------------------------------------------------------------------------
// 连接计数器与停机信号
// ---------------------------------------------------------------------------

static WS_CLIENT_COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static WS_FRAMES_SENT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static WS_FRAMES_DROPPED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn ws_client_count() -> u64 {
    WS_CLIENT_COUNT.load(std::sync::atomic::Ordering::Relaxed)
}

fn ws_frames_sent() -> u64 {
    WS_FRAMES_SENT.load(std::sync::atomic::Ordering::Relaxed)
}

fn ws_frames_dropped() -> u64 {
    WS_FRAMES_DROPPED.load(std::sync::atomic::Ordering::Relaxed)
}

// ---------------------------------------------------------------------------
// 优雅停机广播：向所有 WS 连接发送 Close(4002)
// ---------------------------------------------------------------------------

/// 停机关闭帧的 code 与 reason：前端据此区分「服务停机」与普通断线。
pub const WS_SHUTDOWN_CLOSE_CODE: u16 = 4002;
pub const WS_SHUTDOWN_CLOSE_REASON: &str = "server shutting down";

/// 优雅停机时等待 WS 连接退出的排水窗口（有界，避免进程退出前关闭帧来不及送达）
pub const WS_SHUTDOWN_DRAIN_TIMEOUT: Duration = Duration::from_secs(5);

/// 排水轮询间隔
const DRAIN_POLL_INTERVAL: Duration = Duration::from_millis(20);

/// 停机广播句柄：`watch` 单槽信号（无队列，内存与连接数无关，不引入无界资源）。
///
/// `AppState` 持有 Sender 端；每个连接任务进入事件循环前 `subscribe()` 一个 Receiver。
/// `notify()` 置位后，所有连接任务——包括置位之后才订阅的——都会立即观察到标志位，
/// 随即向客户端发送 `Close(4002, "server shutting down")` 并退出。
///
/// 选择广播通道而非连接注册表：注册表需经每连接的出站通道投递关闭帧，
/// 而关闭帧必须走 `handle_frames` 直接持有的 socket sender（与 4001 关闭路径一致），
/// watch 广播让各连接任务自行发送，路径最短。
#[derive(Clone)]
pub struct WsShutdown {
    tx: Arc<watch::Sender<bool>>,
}

impl Default for WsShutdown {
    fn default() -> Self {
        Self {
            tx: Arc::new(watch::Sender::new(false)),
        }
    }
}

impl WsShutdown {
    pub fn new() -> Self {
        Self::default()
    }

    /// 置位停机标志，唤醒所有连接任务
    pub fn notify(&self) {
        self.tx.send_replace(true);
    }

    /// 连接任务订阅停机信号
    pub fn subscribe(&self) -> watch::Receiver<bool> {
        self.tx.subscribe()
    }

    /// 排水：等待全部连接任务退出（至多 `timeout`），返回超时后仍未断开的连接数。
    /// 轮询进程级连接计数，窗口有界。
    pub async fn drain(&self, timeout: Duration) -> u64 {
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = ws_client_count();
            if remaining == 0 || Instant::now() >= deadline {
                return remaining;
            }
            tokio::time::sleep(DRAIN_POLL_INTERVAL).await;
        }
    }

    /// 优雅停机入口：先广播停机（触发各连接发送 4002 关闭帧），再等待连接退出。
    pub async fn notify_and_drain(&self, timeout: Duration) -> u64 {
        self.notify();
        self.drain(timeout).await
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
    // 连接数上限检查
    let current = WS_CLIENT_COUNT.load(std::sync::atomic::Ordering::Relaxed);
    if current >= state.config.ws_max_clients as u64 {
        tracing::warn!(
            "WS rejected: max clients {} reached",
            state.config.ws_max_clients
        );
        return Err(ApiError::service_unavailable("WS server at capacity"));
    }

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

/// 认证失败（token 无效 / 未认证操作 / 认证超时）的关闭码
const WS_AUTH_FAIL_CLOSE_CODE: u16 = 4001;

async fn handle_socket(socket: WebSocket, state: Arc<AppState>) {
    // 增加连接计数
    WS_CLIENT_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

    // 每个连接独立的 bus tap
    let bus = state.manager.bus();
    let (tap_id, mut bus_rx) = bus.subscribe_all();

    // 出站通道（有界 256）：批量帧（values/nodes 快照）先入队，由事件循环统一写 socket；
    // 入队用 try_send，通道满时丢帧计数，绝不阻塞事件循环（无死锁、无无界积压）
    let (out_tx, mut out_rx) = mpsc::channel::<Message>(CHANNEL_CAPACITY);

    let (sender, receiver) = socket.split();
    let result = handle_frames(sender, receiver, state, &mut bus_rx, out_tx, &mut out_rx).await;

    // 清理：注销 tap、减少计数
    bus.unsubscribe_all(tap_id);
    WS_CLIENT_COUNT.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);

    if let Err(e) = result {
        tracing::debug!("WS connection error: {}", e);
    }
}

// ---------------------------------------------------------------------------
// 帧处理循环
// ---------------------------------------------------------------------------

/// 直接写 socket 的控制帧（认证响应/错误/关闭/ping-pong），成功即计入已发送帧数。
/// 控制帧不过出站队列：保证低延迟且不因队列满被丢弃。
async fn send_control<S>(sender: &mut S, msg: Message)
where
    S: SinkExt<Message> + Unpin,
{
    if sender.send(msg).await.is_ok() {
        WS_FRAMES_SENT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
}

/// 停机关闭帧 `Close(4002, "server shutting down")`
fn shutdown_close_frame() -> Message {
    Message::Close(Some(axum::extract::ws::CloseFrame {
        code: WS_SHUTDOWN_CLOSE_CODE,
        reason: WS_SHUTDOWN_CLOSE_REASON.into(),
    }))
}

/// 认证失败关闭帧 `Close(4001, <原因>)`
fn auth_fail_close_frame(reason: &str) -> Message {
    Message::Close(Some(axum::extract::ws::CloseFrame {
        code: WS_AUTH_FAIL_CLOSE_CODE,
        reason: reason.to_string().into(),
    }))
}

async fn handle_frames<S, R>(
    mut sender: S,
    mut receiver: R,
    state: Arc<AppState>,
    bus_rx: &mut broadcast::Receiver<Arc<gateway_sdk::GroupData>>,
    out_tx: mpsc::Sender<Message>,
    out_rx: &mut mpsc::Receiver<Message>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
where
    S: SinkExt<Message> + Unpin,
    R: StreamExt<Item = Result<Message, axum::Error>> + Unpin,
{
    let mut session = Session {
        authenticated: false,
        username: None,
        topics: Topics::empty(),
        node_filter: HashSet::new(),
        group_filter: HashSet::new(),
    };

    // 10s 认证超时
    let auth_deadline = Instant::now() + AUTH_TIMEOUT;

    // 60s 服务器 ping 定时器
    let mut ping_timer = interval(SERVER_PING_INTERVAL);
    ping_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    // 客户端 pong 超时定时器
    let mut pong_timer = interval(CLIENT_PONG_TIMEOUT);
    pong_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    // nodes 快照定时器
    let mut snapshot_timer = interval(Duration::from_millis(state.config.ws_snapshot_interval_ms));
    snapshot_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    // 停机信号：wait_for 对「订阅前标志已置位」的边界同样立即返回，不存在竞态窗口
    let mut shutdown_rx = state.ws_shutdown.subscribe();

    // interval 的首个 tick 立即完成：先消费掉，避免连接刚建立或刚认证时
    // 立即触发服务器 ping / pong 超时判定（曾导致认证后连接被瞬间以
    // Close(1000, "pong timeout") 关闭）/ 快照
    ping_timer.tick().await;
    pong_timer.tick().await;
    snapshot_timer.tick().await;

    loop {
        tokio::select! {
            // 优雅停机：立即向客户端发送 Close(4002) 并退出事件循环（不依赖认证状态）。
            // 映射为 bool：watch::Ref 非 Send，不能作为 select 分支输出绑定后跨 await 存活。
            shutdown = async { shutdown_rx.wait_for(|s| *s).await.is_ok() } => {
                if shutdown {
                    tracing::info!("WS: server shutting down, sending Close({})", WS_SHUTDOWN_CLOSE_CODE);
                    send_control(&mut sender, shutdown_close_frame()).await;
                }
                break;
            }

            // 客户端消息
            msg = receiver.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        if let Err(e) = process_client_frame(&text, &state, &mut session, &mut sender).await {
                            tracing::warn!("WS client frame error: {}", e);
                            // 错误帧 + 4001 关闭帧都直接写 socket，客户端据此立即感知认证失败
                            send_control(
                                &mut sender,
                                Message::Text(
                                    serde_json::to_string(&WsServerFrame::error(&e.to_string()))
                                        .unwrap(),
                                ),
                            )
                            .await;
                            send_control(&mut sender, auth_fail_close_frame(&e.to_string())).await;
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
                    send_control(&mut sender, Message::Text(serde_json::to_string(&WsServerFrame::ping()).unwrap())).await;
                    pong_timer.reset();
                }
            }

            // 客户端 pong 超时（25s 未响应）
            _ = pong_timer.tick() => {
                if session.authenticated {
                    tracing::warn!("WS: client pong timeout, closing connection");
                    send_control(&mut sender, Message::Close(Some(axum::extract::ws::CloseFrame{
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
                    send_control(&mut sender, Message::Text(serde_json::to_string(&WsServerFrame::auth_fail("auth timeout")).unwrap())).await;
                    send_control(&mut sender, auth_fail_close_frame("auth timeout")).await;
                    break;
                }
            }

            // nodes 快照（ws_snapshot_interval_ms=0 时关闭）
            _ = snapshot_timer.tick() => {
                if session.authenticated && session.topics.contains(Topics::NODES) && state.config.ws_snapshot_interval_ms > 0 {
                    let frame = build_nodes_frame(&state).await;
                    if let Some(f) = frame {
                        let text = serde_json::to_string(&f).unwrap_or_default();
                        enqueue_frame(&out_tx, Message::Text(text));
                    }
                }
            }

            // 总线数据（旁路订阅）
            data = bus_rx.recv() => {
                match data {
                    Ok(gd) => {
                        if session.authenticated && session.topics.contains(Topics::VALUES) {
                            // 按 node_ids / group_ids 过滤
                            if !session.node_filter.is_empty() && !session.node_filter.contains(&gd.node_id.0.to_string()) {
                                continue;
                            }
                            if !session.group_filter.is_empty() && !session.group_filter.contains(&gd.group_id.0.to_string()) {
                                continue;
                            }

                            let frame = WsServerFrame::values(WsValuesData {
                                node_id: gd.node_id.0.to_string(),
                                node_name: gd.node_name.clone(),
                                group_id: gd.group_id.0.to_string(),
                                group_name: gd.group_name.clone(),
                                values: gd.values.iter().map(|(tid, dv)| {
                                    let tag_name = gd.tag_names.as_ref().and_then(|m| m.get(tid)).cloned();
                                    WsTagValue {
                                        tag_id: tid.0.to_string(),
                                        tag_name,
                                        value: serde_json::to_value(dv).unwrap_or_default(),
                                    }
                                }).collect(),
                            });

                            let text = serde_json::to_string(&frame).unwrap_or_default();
                            enqueue_frame(&out_tx, Message::Text(text));
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        tracing::debug!("WS bus tap lagged {} frames", n);
                        WS_FRAMES_DROPPED.fetch_add(n, std::sync::atomic::Ordering::Relaxed);
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }

            // 出站队列：values/快照帧经 out_tx 排队，由事件循环统一写 socket
            msg = out_rx.recv() => {
                match msg {
                    Some(m) => {
                        WS_FRAMES_SENT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        if sender.send(m).await.is_err() {
                            break;
                        }
                    }
                    // 出站通道关闭：本循环持有唯一 sender，正常运行不可达，防御性退出
                    None => break,
                }
            }
        }
    }

    Ok(())
}

/// 批量帧入站：非阻塞 try_send，通道满时丢帧并计数（有界队列，绝不阻塞事件循环）
fn enqueue_frame(out_tx: &mpsc::Sender<Message>, msg: Message) {
    match out_tx.try_send(msg) {
        Ok(()) => {}
        Err(mpsc::error::TrySendError::Full(_)) => {
            // 通道满，丢帧
            WS_FRAMES_DROPPED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        Err(mpsc::error::TrySendError::Closed(_)) => {
            tracing::debug!("WS out channel closed, frame dropped");
        }
    }
}

// ---------------------------------------------------------------------------
// Session
// ---------------------------------------------------------------------------

struct Session {
    authenticated: bool,
    username: Option<String>,
    topics: Topics,
    node_filter: HashSet<String>,
    group_filter: HashSet<String>,
}

// ---------------------------------------------------------------------------
// 处理单个客户端帧
// ---------------------------------------------------------------------------

async fn process_client_frame<S>(
    text: &str,
    state: &Arc<AppState>,
    session: &mut Session,
    sender: &mut S,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
where
    S: SinkExt<Message> + Unpin,
{
    let frame: WsClientFrame = serde_json::from_str(text)?;

    match frame {
        WsClientFrame::Auth { token } => {
            let ctx = resolve_bearer(state, token.as_deref());
            if let Some(ctx) = ctx {
                session.authenticated = true;
                session.username = ctx.username;
                tracing::info!(username = ?session.username, "WS authenticated");
                // 发送 auth success
                send_control(
                    sender,
                    Message::Text(serde_json::to_string(&WsServerFrame::auth_ok()).unwrap()),
                )
                .await;
                // 发送 hello
                send_control(
                    sender,
                    Message::Text(serde_json::to_string(&WsServerFrame::hello()).unwrap()),
                )
                .await;
            } else {
                // 认证失败
                return Err("invalid token".into());
            }
        }

        WsClientFrame::Ping => {
            tracing::debug!("WS client ping received");
        }

        WsClientFrame::Pong => {
            tracing::debug!("WS client pong received");
        }

        WsClientFrame::Subscribe {
            topics,
            node_ids,
            group_ids,
        } => {
            if !session.authenticated {
                return Err("not authenticated".into());
            }
            session.topics |= Topics::from_strs(topics.iter().map(|s| s.as_str()));
            if let Some(ids) = node_ids {
                session.node_filter = ids.into_iter().collect();
            }
            if let Some(ids) = group_ids {
                session.group_filter = ids.into_iter().collect();
            }
            tracing::debug!(username = ?session.username, "WS subscribed");
        }

        WsClientFrame::Unsubscribe { topics } => {
            if !session.authenticated {
                return Err("not authenticated".into());
            }
            let to_remove = Topics::from_strs(topics.iter().map(|s| s.as_str()));
            session.topics &= !to_remove;
            tracing::debug!(username = ?session.username, "WS unsubscribed");
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// 构建 nodes 快照帧
// ---------------------------------------------------------------------------

async fn build_nodes_frame(state: &AppState) -> Option<WsServerFrame> {
    use gateway_sdk::NodeKind;
    let nodes = state.manager.nodes_list();

    let snapshots: Vec<WsNodeSnapshot> = nodes
        .iter()
        .map(|node| {
            let kind_str = if node.kind() == NodeKind::North {
                "north"
            } else {
                "south"
            };
            let state_str = match node.state {
                gateway_sdk::NodeState::Running => "running",
                gateway_sdk::NodeState::Stopped => "stopped",
                _ => "unknown",
            };

            WsNodeSnapshot {
                id: node.id().0.to_string(),
                name: node.config.name.clone(),
                kind: kind_str.to_string(),
                plugin_name: node.config.plugin_name.clone(),
                state: state_str.to_string(),
                connection_status: None,
            }
        })
        .collect();

    Some(WsServerFrame::nodes(snapshots))
}

// ---------------------------------------------------------------------------
// 导出指标（供 handlers.rs metrics 使用）
// ---------------------------------------------------------------------------

/// 获取当前 WS 客户端数量
pub fn metric_ws_clients() -> u64 {
    ws_client_count()
}

/// 获取 WS 已发送帧计数
pub fn metric_ws_frames_sent() -> u64 {
    ws_frames_sent()
}

/// 获取 WS 丢弃帧计数
pub fn metric_ws_frames_dropped() -> u64 {
    ws_frames_dropped()
}
