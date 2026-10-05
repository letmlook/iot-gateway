//! 北向 HTTP/Webhook 插件：接收 GroupData，POST/PUT JSON 到用户 URL。
//!
//! 事件模型（与 plugin-mqtt 完全一致）：
//! - `on_group_data` 只做序列化 + mpsc try_send / 离线队列 push，绝不内联网络 IO
//! - 发送在独立 OS 线程 worker 中完成（std::thread::spawn + current_thread runtime）
//! - 失败进离线缓存（磁盘持久化 + 按序补发）

#[cfg(feature = "ffi")]
mod ffi;

use gateway_plugin_common::config::{
    config_bool, config_str, config_u64, config_usize, queue_path_for_node,
    DEFAULT_CACHE_MEMORY_SIZE, DEFAULT_CACHE_SYNC_INTERVAL_MS,
};
use gateway_plugin_common::format::{payload_for_format, UPLOAD_FORMAT_VALUES_FORMAT};
use gateway_plugin_common::http::{HttpClass, HttpClass as CommonHttpClass};
use gateway_plugin_common::queue::{OfflineQueue, Record};
use gateway_sdk::log;
use gateway_sdk::types::PluginKind;
use gateway_sdk::PluginResult;
use gateway_sdk::{GroupData, GroupSubscription, NodeId, NorthPlugin, PluginConfig, PluginMeta};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, RwLock};

// ---------------------------------------------------------------------------
// 常量与默认值
// ---------------------------------------------------------------------------

const DEFAULT_METHOD: &str = "POST";
const DEFAULT_CONTENT_TYPE: &str = "application/json";
const DEFAULT_AUTH_TYPE: &str = "none";
const DEFAULT_TIMEOUT_MS: u64 = 5000;
const DEFAULT_UPLOAD_FORMAT: &str = UPLOAD_FORMAT_VALUES_FORMAT;
const DEFAULT_CACHE_DIR_HTTP: &str = "data/http-queue";

// ---------------------------------------------------------------------------
// 状态
// ---------------------------------------------------------------------------

/// 连接状态（返回给 API / 前端）
#[derive(Debug, Default)]
struct HttpConnectionStatus {
    connected: bool,
    last_error: Option<String>,
    dropped_rejected: u64,
    dropped_no_client: u64,
}

/// 每节点状态
struct NodeHttpState {
    queue: Arc<tokio::sync::Mutex<OfflineQueue>>,
    connection_status: Arc<RwLock<HttpConnectionStatus>>,
    /// worker 发来的消息 channel（healthy 时）
    tx: Option<mpsc::Sender<Record>>,
    /// OS 线程 cancel 通知
    cancel_tx: Option<tokio::sync::oneshot::Sender<()>>,
    event_loop_handle: Option<std::thread::JoinHandle<()>>,
}

struct HttpState {
    open_nodes: std::collections::HashSet<NodeId>,
    subscriptions: HashMap<NodeId, Vec<GroupSubscription>>,
    nodes: HashMap<NodeId, NodeHttpState>,
}

// ---------------------------------------------------------------------------
// 插件
// ---------------------------------------------------------------------------

pub struct HttpPlugin {
    state: Arc<RwLock<HttpState>>,
}

impl Default for HttpPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpPlugin {
    pub fn new() -> Self {
        Self {
            state: Arc::new(RwLock::new(HttpState {
                open_nodes: std::collections::HashSet::new(),
                subscriptions: HashMap::new(),
                nodes: HashMap::new(),
            })),
        }
    }
}

#[async_trait::async_trait]
impl NorthPlugin for HttpPlugin {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "http",
            kind: PluginKind::North,
            description: Some("HTTP/Webhook 北向：POST/PUT JSON 到用户 URL，支持 Bearer/Basic 认证、TLS、离线缓存与补发"),
            version: "0.1.0",
            name_zh: Some("HTTP"),
            name_en: Some("HTTP"),
            description_zh: Some("HTTP/Webhook 北向：POST/PUT JSON，支持 Bearer/Basic 认证、TLS、离线缓存与补发"),
            description_en: Some("HTTP/Webhook north: POST/PUT JSON to user URL, supports Bearer/Basic auth, TLS, offline cache and replay"),
        }
    }

    fn config_schema(&self) -> Option<gateway_sdk::ConfigSchema> {
        Some(config_schema())
    }

    async fn open(&self, node_id: NodeId, config: PluginConfig) -> PluginResult<()> {
        let url = config_str(&config, "url", "");
        if url.is_empty() {
            return Err(gateway_sdk::PluginError::config_invalid(
                "url is required".to_string(),
            ));
        }
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Err(gateway_sdk::PluginError::config_invalid(
                "url must start with http:// or https://".to_string(),
            ));
        }

        let method = config_str(&config, "method", DEFAULT_METHOD);
        let content_type = config_str(&config, "content_type", DEFAULT_CONTENT_TYPE);
        let auth_type = config_str(&config, "auth_type", DEFAULT_AUTH_TYPE);
        let auth_token = config_str(&config, "auth_token", "");
        let headers_json = config_str(&config, "headers_json", "");
        let timeout_ms = config_u64(&config, "timeout_ms", DEFAULT_TIMEOUT_MS);
        let insecure_skip_verify = config_bool(&config, "insecure_skip_verify", false);
        let ca_file = config_str(&config, "ca_file", "");
        let cache_memory_size =
            config_usize(&config, "cache_memory_size", DEFAULT_CACHE_MEMORY_SIZE);
        let cache_sync_interval_ms = config_u64(
            &config,
            "cache_sync_interval_ms",
            DEFAULT_CACHE_SYNC_INTERVAL_MS,
        );
        let cache_persist = config_bool(&config, "cache_persist", true);
        let cache_dir = config_str(&config, "cache_dir", DEFAULT_CACHE_DIR_HTTP);

        // 解析 headers_json
        let extra_headers: HashMap<String, String> = if headers_json.is_empty() {
            HashMap::new()
        } else {
            serde_json::from_str(&headers_json).map_err(|e| {
                gateway_sdk::PluginError::config_invalid(format!(
                    "headers_json parse failed: {}",
                    e
                ))
            })?
        };

        log::info(node_id, format!("open http: {} {}", method, url));

        let mut state = self.state.write().await;
        state.open_nodes.insert(node_id);

        let queue_path = if cache_persist {
            queue_path_for_node(&cache_dir, node_id, "-http")
        } else {
            None
        };

        let queue = Arc::new(tokio::sync::Mutex::new(if cache_persist {
            gateway_plugin_common::queue::OfflineQueue::open(queue_path, cache_memory_size)
        } else {
            gateway_plugin_common::queue::OfflineQueue::memory_only(cache_memory_size)
        }));

        {
            let q = queue.lock().await;
            let s = q.stats();
            if s.recovered > 0 || s.dropped_corrupt > 0 {
                log::info(
                    node_id,
                    format!(
                        "offline queue restored: {} record(s), {} corrupted dropped",
                        s.recovered, s.dropped_corrupt
                    ),
                );
            }
        }

        let connection_status = Arc::new(RwLock::new(HttpConnectionStatus {
            connected: false,
            last_error: None,
            dropped_rejected: 0,
            dropped_no_client: 0,
        }));

        // 解析 auth
        let auth_header: Option<String> = match auth_type.as_str() {
            "bearer" if !auth_token.is_empty() => Some(format!("Bearer {}", auth_token)),
            "basic" if !auth_token.is_empty() => {
                let encoded = base64_encode(&auth_token);
                Some(format!("Basic {}", encoded))
            }
            _ => None,
        };

        // 构建 HTTP client
        let http_client = build_http_client(
            timeout_ms,
            insecure_skip_verify,
            if ca_file.is_empty() {
                None
            } else {
                Some(&ca_file)
            },
        )
        .map_err(gateway_sdk::PluginError::config_invalid)?;

        let (record_tx, record_rx) = mpsc::channel::<Record>(1000);
        let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();

        let queue_clone = queue.clone();
        let conn_status_clone = connection_status.clone();
        let node_id_worker = node_id;
        let url_worker = url.clone();
        let method_worker = method.clone();
        let content_type_worker = content_type.clone();
        let extra_headers_worker = extra_headers.clone();
        let auth_header_worker = auth_header.clone();
        let sync_interval = cache_sync_interval_ms;

        let handle = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("http event loop runtime");
            rt.block_on(run_http_worker(
                node_id_worker,
                url_worker,
                method_worker,
                content_type_worker,
                extra_headers_worker,
                auth_header_worker,
                record_rx,
                queue_clone,
                conn_status_clone,
                http_client,
                sync_interval,
                cancel_rx,
            ));
        });

        state.nodes.insert(
            node_id,
            NodeHttpState {
                queue,
                connection_status,
                tx: Some(record_tx),
                cancel_tx: Some(cancel_tx),
                event_loop_handle: Some(handle),
            },
        );

        Ok(())
    }

    async fn close(&self, node_id: NodeId) -> PluginResult<()> {
        log::info(node_id, "close http, disconnecting");
        let (cancel_tx, handle_opt, queue_opt) = {
            let mut state = self.state.write().await;
            state.open_nodes.remove(&node_id);
            state.subscriptions.remove(&node_id);
            state
                .nodes
                .remove(&node_id)
                .map(|ns| (ns.cancel_tx, ns.event_loop_handle, Some(ns.queue)))
                .unwrap_or((None, None, None))
        };
        if let Some(q) = queue_opt {
            q.lock().await.compact_now();
        }
        if let Some(tx) = cancel_tx {
            let _ = tx.send(());
        }
        if let Some(h) = handle_opt {
            let _ = tokio::time::timeout(
                Duration::from_secs(5),
                tokio::task::spawn_blocking(move || {
                    let _ = h.join();
                }),
            )
            .await;
        }
        Ok(())
    }

    async fn start(&self, node_id: NodeId) -> PluginResult<()> {
        log::info(node_id, "start http");
        Ok(())
    }

    async fn stop(&self, node_id: NodeId) -> PluginResult<()> {
        log::info(node_id, "stop http");
        Ok(())
    }

    async fn setting(&self, node_id: NodeId, config: PluginConfig) -> PluginResult<()> {
        log::info(node_id, "setting http, reconnecting with new config");
        self.close(node_id).await?;
        self.open(node_id, config).await
    }

    async fn set_subscriptions(
        &self,
        node_id: NodeId,
        subscriptions: &[GroupSubscription],
    ) -> PluginResult<()> {
        log::info(
            node_id,
            format!("set_subscriptions: {} group(s)", subscriptions.len()),
        );
        let mut state = self.state.write().await;
        state.subscriptions.insert(node_id, subscriptions.to_vec());
        Ok(())
    }

    async fn connection_status(&self, node_id: NodeId) -> Option<serde_json::Value> {
        let state = self.state.read().await;
        let node_state = state.nodes.get(&node_id)?;
        let st = node_state.connection_status.read().await;
        let q = node_state.queue.lock().await.stats();
        Some(serde_json::json!({
            "connected": st.connected,
            "last_error": st.last_error,
            "queue_len": q.queued,
            "queue_dropped_overflow": q.dropped_overflow,
            "queue_recovered": q.recovered,
            "queue_dropped_corrupt": q.dropped_corrupt,
            "queue_persisted": q.persisted,
            "dropped_rejected": st.dropped_rejected,
            "dropped_no_client": st.dropped_no_client,
        }))
    }

    async fn on_group_data(&self, node_id: NodeId, data: Arc<GroupData>) -> PluginResult<()> {
        let state = self.state.read().await;
        let Some(node_state) = state.nodes.get(&node_id) else {
            log::warn(
                node_id,
                "on_group_data: north node not open (no HTTP client), skip publish",
            );
            return Ok(());
        };

        let payload = payload_for_format(&data, DEFAULT_UPLOAD_FORMAT);

        #[cfg(feature = "http-client")]
        {
            let (connected, tx_opt) = {
                let cs = node_state.connection_status.read().await;
                (cs.connected, node_state.tx.clone())
            };
            if !connected {
                enqueue(node_state, payload).await;
                return Ok(());
            }
            if let Some(tx) = tx_opt {
                let rec = Record {
                    topic: String::new(),
                    payload: payload.clone(),
                };
                if tx.try_send(rec).is_err() {
                    enqueue(node_state, payload).await;
                }
            }
        }

        #[cfg(not(feature = "http-client"))]
        {
            let _ = payload;
            let mut cs = node_state.connection_status.write().await;
            cs.dropped_no_client += 1;
            tracing::info!(node_id = ?node_id, "http (no client): would send");
        }

        Ok(())
    }
}

/// 入队并记录溢出（node_id 从调用方上下文中获取，这里只记录到队列）
async fn enqueue(node_state: &NodeHttpState, payload: Vec<u8>) {
    let mut q = node_state.queue.lock().await;
    let before = q.stats().dropped_overflow;
    q.push(Record {
        topic: String::new(),
        payload,
    });
    if q.stats().dropped_overflow > before {
        tracing::warn!(
            "offline queue full ({} queued), dropped oldest record(s)",
            q.len()
        );
    }
}

// ---------------------------------------------------------------------------
// HTTP Worker
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
async fn run_http_worker(
    node_id: NodeId,
    url: String,
    method: String,
    content_type: String,
    extra_headers: HashMap<String, String>,
    auth_header: Option<String>,
    mut record_rx: mpsc::Receiver<Record>,
    queue: Arc<tokio::sync::Mutex<OfflineQueue>>,
    connection_status: Arc<RwLock<HttpConnectionStatus>>,
    client: reqwest::Client,
    cache_sync_interval_ms: u64,
    mut cancel_rx: tokio::sync::oneshot::Receiver<()>,
) {
    let interval = Duration::from_millis(cache_sync_interval_ms.max(10));
    let mut backoff = Duration::from_secs(1);
    let max_backoff = Duration::from_secs(30);
    let mut healthy = true;

    loop {
        tokio::select! {
            _ = &mut cancel_rx => {
                log::info(node_id, "http event loop exited (close)");
                break;
            }
            rec = record_rx.recv() => {
                match rec {
                    Some(record) => {
                        let class = send_one_http(
                            &client,
                            &method,
                            &url,
                            &content_type,
                            &extra_headers,
                            auth_header.as_deref(),
                            record.payload.clone(),
                        )
                        .await;
                        match class {
                            CommonHttpClass::Delivered => {
                                healthy = true;
                                backoff = Duration::from_secs(1);
                                {
                                    let mut cs = connection_status.write().await;
                                    cs.connected = true;
                                    cs.last_error = None;
                                }
                            }
                            CommonHttpClass::Retryable => {
                                let restored = replenish_queue(
                                    &queue, &client, &method, &url, &content_type,
                                    &extra_headers, auth_header.as_deref(), interval,
                                ).await;
                                if restored > 0 {
                                    log::info(node_id, format!("offline queue flushed {} record(s)", restored));
                                }
                                healthy = false;
                                backoff = Duration::from_secs(1);
                            }
                            CommonHttpClass::Rejected => {
                                let mut cs = connection_status.write().await;
                                cs.dropped_rejected += 1;
                                let body = String::from_utf8_lossy(&record.payload);
                                log::warn(node_id, format!("http rejected (4xx), dropped: {}", &body[..body.len().min(200)]));
                            }
                        }
                    }
                    None => break,
                }
            }
            _ = tokio::time::sleep(backoff), if !healthy => {
                let restored = replenish_queue(
                    &queue, &client, &method, &url, &content_type,
                    &extra_headers, auth_header.as_deref(), interval,
                ).await;
                if restored > 0 {
                    healthy = true;
                    backoff = Duration::from_secs(1);
                    log::info(node_id, format!("http offline queue flushed {} record(s)", restored));
                } else {
                    backoff = (backoff * 2).min(max_backoff);
                }
            }
        }
    }
}

/// 尝试补发离线队列，返回成功条数
#[allow(clippy::too_many_arguments)]
async fn replenish_queue(
    queue: &Arc<tokio::sync::Mutex<OfflineQueue>>,
    client: &reqwest::Client,
    method: &str,
    url: &str,
    content_type: &str,
    extra_headers: &HashMap<String, String>,
    auth_header: Option<&str>,
    interval: Duration,
) -> u64 {
    let mut restored = 0u64;
    loop {
        let record = {
            let mut q = queue.lock().await;
            q.pop_front()
        };
        let Some(record) = record else { break };
        let class = send_one_http(
            client,
            method,
            url,
            content_type,
            extra_headers,
            auth_header,
            record.payload.clone(),
        )
        .await;
        match class {
            CommonHttpClass::Delivered => {
                restored += 1;
                tokio::time::sleep(interval).await;
            }
            CommonHttpClass::Retryable => {
                let mut q = queue.lock().await;
                q.push_front(record);
                break;
            }
            CommonHttpClass::Rejected => {
                // 丢弃（已在队列外）
            }
        }
    }
    restored
}

async fn send_one_http(
    client: &reqwest::Client,
    method: &str,
    url: &str,
    content_type: &str,
    extra_headers: &HashMap<String, String>,
    auth_header: Option<&str>,
    body: Vec<u8>,
) -> HttpClass {
    let method = match method {
        "PUT" => reqwest::Method::PUT,
        _ => reqwest::Method::POST,
    };
    let mut req = client.request(method, url);
    req = req.header("Content-Type", content_type);
    if let Some(auth) = auth_header {
        req = req.header("Authorization", auth);
    }
    for (k, v) in extra_headers {
        req = req.header(k.as_str(), v.as_str());
    }
    req = req.body(body);

    match req.send().await {
        Ok(resp) => HttpClass::from_status_code(resp.status().as_u16()),
        Err(e) => {
            if e.is_timeout() || e.is_connect() {
                HttpClass::Retryable
            } else {
                HttpClass::Rejected
            }
        }
    }
}

// ---------------------------------------------------------------------------
// HTTP Client 构造
// ---------------------------------------------------------------------------

#[cfg(feature = "http-client")]
fn build_http_client(
    timeout_ms: u64,
    insecure_skip_verify: bool,
    ca_file: Option<&str>,
) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder()
        .timeout(Duration::from_millis(timeout_ms))
        .user_agent("iot-gateway-north/http");

    if insecure_skip_verify {
        builder = builder.danger_accept_invalid_certs(true);
    }

    if let Some(ca_path) = ca_file {
        let ca_data = std::fs::read(ca_path).map_err(|e| format!("read ca_file failed: {}", e))?;
        let cert = reqwest::Certificate::from_pem(&ca_data)
            .or_else(|_| reqwest::Certificate::from_der(&ca_data))
            .map_err(|e| format!("parse ca_file failed: {}", e))?;
        builder = builder.add_root_certificate(cert);
    }

    builder
        .build()
        .map_err(|e| format!("build reqwest client failed: {}", e))
}

// ---------------------------------------------------------------------------
// base64（不引入额外依赖）
// ---------------------------------------------------------------------------

fn base64_encode(input: &str) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::new();
    let bytes = input.as_bytes();
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as i32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as i32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as i32;
        result.push(ALPHABET[((b0 << 2) | (b1 >> 6)) as usize] as char);
        if chunk.len() > 1 {
            result.push(ALPHABET[(((b1 & 0x3F) << 4) | (b2 >> 4)) as usize] as char);
        } else {
            result.push('=');
        }
        if chunk.len() > 2 {
            result.push(ALPHABET[((b2 & 0x3F) << 2) as usize] as char);
        } else {
            result.push('=');
        }
    }
    result
}

// ---------------------------------------------------------------------------
// ConfigSchema
// ---------------------------------------------------------------------------

fn config_schema() -> gateway_sdk::ConfigSchema {
    use gateway_sdk::schema::{ParamAttribute, ParamOption, ParamSchema, ParamType, ParamValid};
    use gateway_sdk::ConfigSchema;

    ConfigSchema::new()
        .param(ParamSchema {
            name: "url".to_string(),
            name_zh: Some("目标 URL".to_string()),
            name_en: Some("Target URL".to_string()),
            description: Some("HTTP POST/PUT target URL, must start with http:// or https://".to_string()),
            description_zh: Some("HTTP POST/PUT 目标 URL，必须以 http:// 或 https:// 开头".to_string()),
            description_en: Some("HTTP POST/PUT target URL, must start with http:// or https://".to_string()),
            attribute: ParamAttribute::Required,
            ty: ParamType::String,
            default: None,
            valid: Some(ParamValid {
                min: None, max: None,
                regex: Some("^https?://.+".to_string()),
                length: Some(2048),
            }),
            ..Default::default()
        })
        .param(ParamSchema {
            name: "method".to_string(),
            name_zh: Some("请求方法".to_string()),
            name_en: Some("Method".to_string()),
            description: Some("HTTP method".to_string()),
            description_zh: Some("HTTP 请求方法".to_string()),
            description_en: Some("HTTP method".to_string()),
            attribute: ParamAttribute::Optional,
            ty: ParamType::Select,
            default: Some(serde_json::json!("POST")),
            valid: None,
            options: Some(vec![
                ParamOption { value: serde_json::json!("POST"), label: Some("POST".to_string()), label_zh: Some("POST".to_string()), label_en: Some("POST".to_string()) },
                ParamOption { value: serde_json::json!("PUT"), label: Some("PUT".to_string()), label_zh: Some("PUT".to_string()), label_en: Some("PUT".to_string()) },
            ]),
            ..Default::default()
        })
        .param(ParamSchema {
            name: "content_type".to_string(),
            name_zh: Some("Content-Type".to_string()),
            name_en: Some("Content-Type".to_string()),
            description: Some("Request Content-Type header".to_string()),
            description_zh: Some("请求 Content-Type".to_string()),
            description_en: Some("Request Content-Type header".to_string()),
            attribute: ParamAttribute::Optional,
            ty: ParamType::String,
            default: Some(serde_json::json!("application/json")),
            valid: Some(ParamValid { min: None, max: None, regex: None, length: Some(128) }),
            ..Default::default()
        })
        .param(ParamSchema {
            name: "auth_type".to_string(),
            name_zh: Some("认证方式".to_string()),
            name_en: Some("Auth Type".to_string()),
            description: Some("Authentication type: none, bearer, basic".to_string()),
            description_zh: Some("认证方式：无、Bearer Token、Basic Auth".to_string()),
            description_en: Some("Authentication type: none, bearer, basic".to_string()),
            attribute: ParamAttribute::Optional,
            ty: ParamType::Select,
            default: Some(serde_json::json!("none")),
            valid: None,
            options: Some(vec![
                ParamOption { value: serde_json::json!("none"), label: Some("None".to_string()), label_zh: Some("无".to_string()), label_en: Some("None".to_string()) },
                ParamOption { value: serde_json::json!("bearer"), label: Some("Bearer Token".to_string()), label_zh: Some("Bearer Token".to_string()), label_en: Some("Bearer Token".to_string()) },
                ParamOption { value: serde_json::json!("basic"), label: Some("Basic Auth".to_string()), label_zh: Some("Basic Auth".to_string()), label_en: Some("Basic Auth".to_string()) },
            ]),
            ..Default::default()
        })
        .param(ParamSchema {
            name: "auth_token".to_string(),
            name_zh: Some("认证令牌".to_string()),
            name_en: Some("Auth Token".to_string()),
            description: Some("Bearer token or Basic auth credential (user:password). Named with 'token' keyword, auto-masked.".to_string()),
            description_zh: Some("Bearer token 或 Basic 凭据（user:password）。名称含 token 关键字，自动脱敏。".to_string()),
            description_en: Some("Bearer token or Basic auth credential (user:password). Named with 'token' keyword, auto-masked.".to_string()),
            attribute: ParamAttribute::Optional,
            ty: ParamType::String,
            default: None,
            valid: Some(ParamValid { min: None, max: None, regex: None, length: Some(1024) }),
            ..Default::default()
        })
        .sensitive(&["auth_token"])
        .param(ParamSchema {
            name: "headers_json".to_string(),
            name_zh: Some("额外请求头".to_string()),
            name_en: Some("Extra Headers".to_string()),
            description: Some("Extra HTTP headers as JSON object, e.g. {\"X-Tenant\":\"mytenant\"}".to_string()),
            description_zh: Some("额外 HTTP 请求头，JSON 对象，例如 {\"X-Tenant\":\"mytenant\"}".to_string()),
            description_en: Some("Extra HTTP headers as JSON object, e.g. {\"X-Tenant\":\"mytenant\"}".to_string()),
            attribute: ParamAttribute::Optional,
            ty: ParamType::String,
            default: None,
            valid: Some(ParamValid { min: None, max: None, regex: None, length: Some(2048) }),
            ..Default::default()
        })
        .param(ParamSchema {
            name: "timeout_ms".to_string(),
            name_zh: Some("请求超时（MS）".to_string()),
            name_en: Some("Timeout (MS)".to_string()),
            description: Some("HTTP request timeout in milliseconds".to_string()),
            description_zh: Some("HTTP 请求超时，单位毫秒".to_string()),
            description_en: Some("HTTP request timeout in milliseconds".to_string()),
            attribute: ParamAttribute::Optional,
            ty: ParamType::Int,
            default: Some(serde_json::json!(5000i64)),
            valid: Some(ParamValid { min: Some(1), max: Some(600_000), regex: None, length: None }),
            ..Default::default()
        })
        .param(ParamSchema {
            name: "insecure_skip_verify".to_string(),
            name_zh: Some("跳过证书验证".to_string()),
            name_en: Some("Skip Cert Verify".to_string()),
            description: Some("Skip TLS certificate verification (for self-signed certs)".to_string()),
            description_zh: Some("跳过 TLS 证书验证（用于自签名证书场景）".to_string()),
            description_en: Some("Skip TLS certificate verification (for self-signed certs)".to_string()),
            attribute: ParamAttribute::Optional,
            ty: ParamType::Bool,
            default: Some(serde_json::json!(false)),
            valid: None,
            ..Default::default()
        })
        .param(ParamSchema {
            name: "ca_file".to_string(),
            name_zh: Some("CA 证书".to_string()),
            name_en: Some("CA File".to_string()),
            description: Some("Custom CA certificate file (PEM)".to_string()),
            description_zh: Some("自定义 CA 证书文件（PEM 格式）".to_string()),
            description_en: Some("Custom CA certificate file (PEM)".to_string()),
            attribute: ParamAttribute::Optional,
            ty: ParamType::File,
            default: None,
            valid: None,
            ..Default::default()
        })
        .param(ParamSchema {
            name: "upload_format".to_string(),
            name_zh: Some("上报数据格式".to_string()),
            name_en: Some("Upload Format".to_string()),
            description: Some("Upload format: values_format, tags_format, ecp_format, group_data, raw_data.".to_string()),
            description_zh: Some("上报 JSON 格式：values_format/tags_format/ecp_format，及 group_data、raw_data.".to_string()),
            description_en: Some("Upload format: values_format, tags_format, ecp_format, group_data, raw_data.".to_string()),
            attribute: ParamAttribute::Optional,
            ty: ParamType::Select,
            default: Some(serde_json::json!(DEFAULT_UPLOAD_FORMAT)),
            valid: None,
            options: Some(vec![
                ParamOption { value: serde_json::json!(gateway_plugin_common::UPLOAD_FORMAT_VALUES_FORMAT), label: Some("Values Format".to_string()), label_zh: Some("Values 格式".to_string()), label_en: Some("Values Format".to_string()) },
                ParamOption { value: serde_json::json!(gateway_plugin_common::UPLOAD_FORMAT_TAGS_FORMAT), label: Some("Tags Format".to_string()), label_zh: Some("Tags 格式".to_string()), label_en: Some("Tags Format".to_string()) },
                ParamOption { value: serde_json::json!(gateway_plugin_common::UPLOAD_FORMAT_ECP_FORMAT), label: Some("ECP Format".to_string()), label_zh: Some("ECP 格式".to_string()), label_en: Some("ECP Format".to_string()) },
                ParamOption { value: serde_json::json!(gateway_plugin_common::UPLOAD_FORMAT_GROUP_DATA), label: Some("GroupData".to_string()), label_zh: Some("完整 GroupData".to_string()), label_en: Some("GroupData".to_string()) },
                ParamOption { value: serde_json::json!(gateway_plugin_common::UPLOAD_FORMAT_RAW_DATA), label: Some("Raw Data".to_string()), label_zh: Some("Raw Data".to_string()), label_en: Some("Raw Data".to_string()) },
            ]),
            ..Default::default()
        })
        .param(ParamSchema {
            name: "cache_memory_size".to_string(),
            name_zh: Some("缓存内存大小".to_string()),
            name_en: Some("Cache Memory Size".to_string()),
            description: Some("Max in-memory cache size (message count) when connection fails.".to_string()),
            description_zh: Some("连接异常时暂存的最大条数；写满后丢弃最旧的并计入 queue_dropped_overflow。".to_string()),
            description_en: Some("Max in-memory cache size (message count) when connection fails.".to_string()),
            attribute: ParamAttribute::Optional,
            ty: ParamType::Int,
            default: Some(serde_json::json!(gateway_plugin_common::DEFAULT_CACHE_MEMORY_SIZE)),
            valid: None,
            ..Default::default()
        })
        .param(ParamSchema {
            name: "cache_persist".to_string(),
            name_zh: Some("离线队列落盘".to_string()),
            name_en: Some("Persist Offline Queue".to_string()),
            description: Some("Persist the offline queue to disk so buffered messages survive a restart.".to_string()),
            description_zh: Some("断开期间的待发消息是否写入磁盘；开启后网关重启不会丢失。".to_string()),
            description_en: Some("Persist the offline queue to disk so buffered messages survive a restart.".to_string()),
            attribute: ParamAttribute::Optional,
            ty: ParamType::Bool,
            default: Some(serde_json::json!(true)),
            valid: None,
            ..Default::default()
        })
        .param(ParamSchema {
            name: "cache_dir".to_string(),
            name_zh: Some("离线队列目录".to_string()),
            name_en: Some("Offline Queue Directory".to_string()),
            description: Some("Directory for per-node offline queue files.".to_string()),
            description_zh: Some(format!("每个节点一个 <节点ID>-http.queue 文件，默认 {}", DEFAULT_CACHE_DIR_HTTP).to_string()),
            description_en: Some("Directory for per-node offline queue files.".to_string()),
            attribute: ParamAttribute::Optional,
            ty: ParamType::String,
            default: Some(serde_json::json!(DEFAULT_CACHE_DIR_HTTP)),
            valid: None,
            ..Default::default()
        })
        .param(ParamSchema {
            name: "cache_sync_interval_ms".to_string(),
            name_zh: Some("缓存消息重传间隔（MS）".to_string()),
            name_en: Some("Cache Sync Interval (MS)".to_string()),
            description: Some("Interval in milliseconds for replaying cached messages after reconnect.".to_string()),
            description_zh: Some("恢复连接后补发缓存消息的时间间隔，单位毫秒".to_string()),
            description_en: Some("Interval in milliseconds for replaying cached messages after reconnect.".to_string()),
            attribute: ParamAttribute::Optional,
            ty: ParamType::Int,
            default: Some(serde_json::json!(gateway_plugin_common::DEFAULT_CACHE_SYNC_INTERVAL_MS as i64)),
            valid: Some(ParamValid { min: Some(10), max: Some(120_000), regex: None, length: None }),
            ..Default::default()
        })
}
