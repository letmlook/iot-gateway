//! 北向 InfluxDB v2 插件：把 GroupData 编码为 InfluxDB 行协议，批量写入 /api/v2/write。

#[cfg(feature = "ffi")]
mod ffi;

use gateway_plugin_common::config::{
    config_bool, config_str, config_u64, config_usize, queue_path_for_node,
    DEFAULT_CACHE_MEMORY_SIZE, DEFAULT_CACHE_SYNC_INTERVAL_MS,
};
use gateway_plugin_common::http::{HttpClass, HttpClass as CommonHttpClass};
use gateway_plugin_common::line::{data_value_to_line_field, encode, LineField, LinePoint};
use gateway_plugin_common::queue::{OfflineQueue, Record};
use gateway_sdk::log;
use gateway_sdk::types::PluginKind;
use gateway_sdk::PluginResult;
use gateway_sdk::{GroupData, GroupSubscription, NodeId, NorthPlugin, PluginConfig, PluginMeta};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, RwLock};

const DEFAULT_CACHE_DIR_INFLUXDB: &str = "data/influxdb-queue";
const DEFAULT_URL: &str = "http://127.0.0.1:8086";
const DEFAULT_PRECISION: &str = "ms";
const DEFAULT_MEASUREMENT_TEMPLATE: &str = "${node_name}";
const DEFAULT_BATCH_MAX_LINES: usize = 500;
const DEFAULT_BATCH_INTERVAL_MS: u64 = 200;

#[derive(Debug, Default)]
struct InfluxDbConnectionStatus {
    connected: bool,
    last_error: Option<String>,
    dropped_rejected: u64,
    dropped_no_client: u64,
    skipped_bytes_fields: u64,
}

struct NodeInfluxDbState {
    queue: Arc<tokio::sync::Mutex<OfflineQueue>>,
    connection_status: Arc<RwLock<InfluxDbConnectionStatus>>,
    tx: Option<mpsc::Sender<Record>>,
    cancel_tx: Option<tokio::sync::oneshot::Sender<()>>,
    event_loop_handle: Option<std::thread::JoinHandle<()>>,
}

struct InfluxDbState {
    open_nodes: std::collections::HashSet<NodeId>,
    subscriptions: HashMap<NodeId, Vec<GroupSubscription>>,
    nodes: HashMap<NodeId, NodeInfluxDbState>,
}

pub struct InfluxDbPlugin {
    state: Arc<RwLock<InfluxDbState>>,
}

impl Default for InfluxDbPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl InfluxDbPlugin {
    pub fn new() -> Self {
        Self {
            state: Arc::new(RwLock::new(InfluxDbState {
                open_nodes: std::collections::HashSet::new(),
                subscriptions: HashMap::new(),
                nodes: HashMap::new(),
            })),
        }
    }
}

#[async_trait::async_trait]
impl NorthPlugin for InfluxDbPlugin {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "influxdb",
            kind: PluginKind::North,
            description: Some("InfluxDB v2 北向：行协议写入 /api/v2/write，支持 Token 认证、批量、离线缓存与补发"),
            version: "0.1.0",
            name_zh: Some("InfluxDB"),
            name_en: Some("InfluxDB"),
            description_zh: Some("InfluxDB v2 北向：行协议写入，支持 Token 认证、批量、离线缓存与补发"),
            description_en: Some("InfluxDB v2 north: writes via /api/v2/write with Token auth, batching, offline cache and replay"),
        }
    }

    fn config_schema(&self) -> Option<gateway_sdk::ConfigSchema> {
        Some(config_schema())
    }

    async fn open(&self, node_id: NodeId, config: PluginConfig) -> PluginResult<()> {
        let url = config_str(&config, "url", DEFAULT_URL);
        let org = config_str(&config, "org", "");
        let bucket = config_str(&config, "bucket", "");
        let token = config_str(&config, "token", "");
        let precision = config_str(&config, "precision", DEFAULT_PRECISION);
        let _measurement_template = config_str(
            &config,
            "measurement_template",
            DEFAULT_MEASUREMENT_TEMPLATE,
        );
        let tags_json = config_str(&config, "tags_json", "");
        let batch_max_lines = config_usize(&config, "batch_max_lines", DEFAULT_BATCH_MAX_LINES);
        let batch_interval_ms = config_u64(&config, "batch_interval_ms", DEFAULT_BATCH_INTERVAL_MS);
        let cache_memory_size =
            config_usize(&config, "cache_memory_size", DEFAULT_CACHE_MEMORY_SIZE);
        let cache_sync_interval_ms = config_u64(
            &config,
            "cache_sync_interval_ms",
            DEFAULT_CACHE_SYNC_INTERVAL_MS,
        );
        let cache_persist = config_bool(&config, "cache_persist", true);
        let cache_dir = config_str(&config, "cache_dir", DEFAULT_CACHE_DIR_INFLUXDB);
        let insecure_skip_verify = config_bool(&config, "insecure_skip_verify", false);
        let ca_file = config_str(&config, "ca_file", "");

        if org.is_empty() || bucket.is_empty() {
            return Err(gateway_sdk::PluginError::config_invalid(
                "org and bucket are required".to_string(),
            ));
        }

        // 解析 tags_json
        let _extra_tags: HashMap<String, String> = if tags_json.is_empty() {
            HashMap::new()
        } else {
            serde_json::from_str(&tags_json).map_err(|e| {
                gateway_sdk::PluginError::config_invalid(format!("tags_json parse failed: {}", e))
            })?
        };

        log::info(
            node_id,
            format!(
                "open influxdb: {}/api/v2/write?org={}&bucket={}",
                url, org, bucket
            ),
        );

        let mut state = self.state.write().await;
        state.open_nodes.insert(node_id);

        let queue_path = if cache_persist {
            queue_path_for_node(&cache_dir, node_id, "-influxdb")
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

        let connection_status = Arc::new(RwLock::new(InfluxDbConnectionStatus {
            connected: false,
            last_error: None,
            dropped_rejected: 0,
            dropped_no_client: 0,
            skipped_bytes_fields: 0,
        }));

        let http_client = build_http_client(
            30000,
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
        let org_worker = org.clone();
        let bucket_worker = bucket.clone();
        let token_worker = token.clone();
        let precision_worker = precision.clone();
        let sync_interval = cache_sync_interval_ms;
        let max_lines = batch_max_lines;
        let batch_interval = batch_interval_ms;

        let handle = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("influxdb event loop runtime");
            rt.block_on(run_influxdb_worker(
                node_id_worker,
                url_worker,
                org_worker,
                bucket_worker,
                token_worker,
                precision_worker,
                record_rx,
                queue_clone,
                conn_status_clone,
                http_client,
                sync_interval,
                max_lines,
                batch_interval,
                cancel_rx,
            ));
        });

        state.nodes.insert(
            node_id,
            NodeInfluxDbState {
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
        log::info(node_id, "close influxdb, disconnecting");
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
        log::info(node_id, "start influxdb");
        Ok(())
    }

    async fn stop(&self, node_id: NodeId) -> PluginResult<()> {
        log::info(node_id, "stop influxdb");
        Ok(())
    }

    async fn setting(&self, node_id: NodeId, config: PluginConfig) -> PluginResult<()> {
        log::info(node_id, "setting influxdb, reconnecting with new config");
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
            "skipped_bytes_fields": st.skipped_bytes_fields,
        }))
    }

    async fn on_group_data(&self, node_id: NodeId, data: Arc<GroupData>) -> PluginResult<()> {
        let state = self.state.read().await;
        let Some(node_state) = state.nodes.get(&node_id) else {
            log::warn(
                node_id,
                "on_group_data: north node not open (no InfluxDB client), skip",
            );
            return Ok(());
        };

        // 构建行协议（单行）
        let mut line = String::new();
        let node_name = data
            .node_name
            .clone()
            .unwrap_or_else(|| data.node_id.0.to_string());
        let group_name = data
            .group_name
            .clone()
            .unwrap_or_else(|| data.group_id.0.to_string());
        let measurement = "${node_name}".replace("${node_name}", &node_name);

        // tags: node=<node_name>, group=<group_name>
        let tags = vec![
            ("node".to_string(), node_name.clone()),
            ("group".to_string(), group_name),
        ];
        // 追加 extra_tags
        // (将从 node_state 的 extra_tags 获取，但这里没有存储，先留空)
        // extra_tags 在 worker 中使用，这里只需要生成 line payload
        let ts_ms = data.ts.timestamp_millis();

        let mut skipped = 0u64;
        let fields: Vec<(String, LineField)> = data
            .values
            .iter()
            .filter_map(|(tag_id, v)| {
                let key = data
                    .tag_names
                    .as_ref()
                    .and_then(|m| m.get(tag_id).cloned())
                    .unwrap_or_else(|| tag_id.0.to_string());
                match data_value_to_line_field(v) {
                    Some(f) => Some((key, f)),
                    None => {
                        skipped += 1;
                        None
                    }
                }
            })
            .collect();

        if skipped > 0 {
            let mut cs = node_state.connection_status.write().await;
            cs.skipped_bytes_fields += skipped;
        }

        let point = LinePoint {
            measurement,
            tags,
            fields,
            ts_ms,
        };
        encode(&point, &mut line);
        line.push('\n');

        #[cfg(feature = "http-client")]
        {
            let (connected, tx_opt) = {
                let cs = node_state.connection_status.read().await;
                (cs.connected, node_state.tx.clone())
            };
            let payload_bytes = line.into_bytes();
            if !connected {
                enqueue(node_state, payload_bytes).await;
                return Ok(());
            }
            if let Some(tx) = tx_opt {
                let rec = Record {
                    topic: String::new(),
                    payload: payload_bytes.clone(),
                };
                if tx.try_send(rec).is_err() {
                    enqueue(node_state, payload_bytes).await;
                }
            }
        }

        #[cfg(not(feature = "http-client"))]
        {
            let _ = line;
            let mut cs = node_state.connection_status.write().await;
            cs.dropped_no_client += 1;
            tracing::info!(node_id = ?node_id, "influxdb (no client): would send");
        }

        Ok(())
    }
}

async fn enqueue(node_state: &NodeInfluxDbState, payload: Vec<u8>) {
    let mut q = node_state.queue.lock().await;
    let before = q.stats().dropped_overflow;
    q.push(Record {
        topic: String::new(),
        payload,
    });
    if q.stats().dropped_overflow > before {
        tracing::warn!("influxdb offline queue full, dropped oldest record(s)");
    }
}

// ---------------------------------------------------------------------------
// Worker
// ---------------------------------------------------------------------------

/// 将多个行协议的 Vec<u8> payload 用 `\n` 连接成一个 body
fn join_payloads(payloads: Vec<Vec<u8>>) -> Vec<u8> {
    if payloads.is_empty() {
        return Vec::new();
    }
    let total: usize = payloads.iter().map(|p| p.len()).sum();
    let mut result = Vec::with_capacity(total + payloads.len());
    for (i, p) in payloads.into_iter().enumerate() {
        if i > 0 {
            result.push(b'\n');
        }
        result.extend(p);
    }
    result
}

#[allow(clippy::too_many_arguments)]
async fn run_influxdb_worker(
    node_id: NodeId,
    url: String,
    org: String,
    bucket: String,
    token: String,
    precision: String,
    mut record_rx: mpsc::Receiver<Record>,
    queue: Arc<tokio::sync::Mutex<OfflineQueue>>,
    connection_status: Arc<RwLock<InfluxDbConnectionStatus>>,
    client: reqwest::Client,
    cache_sync_interval_ms: u64,
    batch_max_lines: usize,
    batch_interval_ms: u64,
    mut cancel_rx: tokio::sync::oneshot::Receiver<()>,
) {
    let interval = Duration::from_millis(cache_sync_interval_ms.max(10));
    let batch_interval = Duration::from_millis(batch_interval_ms);
    let mut backoff = Duration::from_secs(1);
    let max_backoff = Duration::from_secs(30);
    let mut healthy = true;
    // 攒批 buffer
    let mut batch = Vec::with_capacity(batch_max_lines);

    loop {
        tokio::select! {
            _ = &mut cancel_rx => {
                log::info(node_id, "influxdb event loop exited (close)");
                break;
            }
            rec = record_rx.recv() => {
                match rec {
                    Some(record) => {
                        batch.push(record);
                        if batch.len() >= batch_max_lines || batch_interval == Duration::ZERO {
                            let body = join_payloads(batch.drain(..).map(|r| r.payload).collect());
                            let class = send_lines(&client, &url, &org, &bucket, &token, &precision, body).await;
                            handle_class(class, &connection_status, interval, &mut healthy, &mut backoff, max_backoff).await;
                        }
                    }
                    None => break,
                }
            }
            _ = tokio::time::sleep(batch_interval), if !batch.is_empty() && batch_interval > Duration::ZERO => {
                let body = join_payloads(batch.drain(..).map(|r| r.payload).collect());
                let class = send_lines(&client, &url, &org, &bucket, &token, &precision, body).await;
                handle_class(class, &connection_status, interval, &mut healthy, &mut backoff, max_backoff).await;
            }
            _ = tokio::time::sleep(backoff), if !healthy => {
                let restored = replenish_queue(&queue, &client, &url, &org, &bucket, &token, &precision, interval).await;
                if restored > 0 {
                    healthy = true;
                    backoff = Duration::from_secs(1);
                    log::info(node_id, format!("influxdb offline queue flushed {} record(s)", restored));
                } else {
                    backoff = (backoff * 2).min(max_backoff);
                }
            }
        }
    }
}

async fn handle_class(
    class: HttpClass,
    connection_status: &Arc<RwLock<InfluxDbConnectionStatus>>,
    _interval: Duration,
    healthy: &mut bool,
    backoff: &mut Duration,
    _max_backoff: Duration,
) {
    match class {
        CommonHttpClass::Delivered => {
            *healthy = true;
            *backoff = Duration::from_secs(1);
            let mut cs = connection_status.write().await;
            cs.connected = true;
            cs.last_error = None;
        }
        CommonHttpClass::Retryable => {
            *healthy = false;
            *backoff = Duration::from_secs(1);
            let mut cs = connection_status.write().await;
            cs.connected = false;
        }
        CommonHttpClass::Rejected => {
            let mut cs = connection_status.write().await;
            cs.dropped_rejected += 1;
        }
    }
}

async fn send_lines(
    client: &reqwest::Client,
    url: &str,
    org: &str,
    bucket: &str,
    token: &str,
    precision: &str,
    body: Vec<u8>,
) -> HttpClass {
    let write_url = format!(
        "{}/api/v2/write?org={}&bucket={}&precision={}",
        url, org, bucket, precision
    );
    let mut req = client.post(&write_url);
    req = req.header("Authorization", format!("Token {}", token));
    req = req.header("Content-Type", "text/plain; charset=utf-8");
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

#[allow(clippy::too_many_arguments)]
async fn replenish_queue(
    queue: &Arc<tokio::sync::Mutex<OfflineQueue>>,
    client: &reqwest::Client,
    url: &str,
    org: &str,
    bucket: &str,
    token: &str,
    precision: &str,
    interval: Duration,
) -> u64 {
    let mut restored = 0u64;
    loop {
        let record = {
            let mut q = queue.lock().await;
            q.pop_front()
        };
        let Some(record) = record else { break };
        let class = send_lines(
            client,
            url,
            org,
            bucket,
            token,
            precision,
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
            CommonHttpClass::Rejected => {}
        }
    }
    restored
}

fn build_http_client(
    timeout_ms: u64,
    insecure_skip_verify: bool,
    ca_file: Option<&str>,
) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder()
        .timeout(Duration::from_millis(timeout_ms))
        .user_agent("iot-gateway-north/influxdb");

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
// ConfigSchema
// ---------------------------------------------------------------------------

fn config_schema() -> gateway_sdk::ConfigSchema {
    use gateway_sdk::schema::{ParamAttribute, ParamOption, ParamSchema, ParamType, ParamValid};
    use gateway_sdk::ConfigSchema;

    ConfigSchema::new()
        .param(ParamSchema {
            name: "url".to_string(), name_zh: Some("InfluxDB URL".to_string()), name_en: Some("InfluxDB URL".to_string()),
            description: Some("InfluxDB v2 URL".to_string()), description_zh: Some("InfluxDB v2 服务地址".to_string()), description_en: Some("InfluxDB v2 URL".to_string()),
            attribute: ParamAttribute::Optional, ty: ParamType::String,
            default: Some(serde_json::json!(DEFAULT_URL)),
            valid: Some(ParamValid { min: None, max: None, regex: Some("^https?://.*".to_string()), length: Some(1024) }),
            ..Default::default()
        })
        .param(ParamSchema {
            name: "org".to_string(), name_zh: Some("组织".to_string()), name_en: Some("Org".to_string()),
            description: Some("InfluxDB v2 organization".to_string()),
            description_zh: Some("InfluxDB v2 organization".to_string()), description_en: Some("InfluxDB v2 organization".to_string()),
            attribute: ParamAttribute::Required, ty: ParamType::String,
            default: None, valid: Some(ParamValid { min: None, max: None, regex: None, length: Some(256) }),
            ..Default::default()
        })
        .param(ParamSchema {
            name: "bucket".to_string(), name_zh: Some("Bucket".to_string()), name_en: Some("Bucket".to_string()),
            description: Some("InfluxDB v2 bucket".to_string()),
            description_zh: Some("InfluxDB v2 bucket".to_string()), description_en: Some("InfluxDB v2 bucket".to_string()),
            attribute: ParamAttribute::Required, ty: ParamType::String,
            default: None, valid: Some(ParamValid { min: None, max: None, regex: None, length: Some(256) }),
            ..Default::default()
        })
        .param(ParamSchema {
            name: "token".to_string(), name_zh: Some("Token".to_string()), name_en: Some("Token".to_string()),
            description: Some("InfluxDB v2 Token. Named with 'token' keyword, auto-masked.".to_string()),
            description_zh: Some("InfluxDB v2 Token。名称含 token 关键字，自动脱敏。".to_string()),
            description_en: Some("InfluxDB v2 Token. Named with 'token' keyword, auto-masked.".to_string()),
            attribute: ParamAttribute::Required, ty: ParamType::String,
            default: None, valid: Some(ParamValid { min: None, max: None, regex: None, length: Some(1024) }),
            ..Default::default()
        })
        .sensitive(&["token"])
        .param(ParamSchema {
            name: "precision".to_string(), name_zh: Some("时间精度".to_string()), name_en: Some("Precision".to_string()),
            description: Some("Timestamp precision: ms (milliseconds) or ns (nanoseconds)".to_string()),
            description_zh: Some("时间戳精度：ms（毫秒）或 ns（纳秒）".to_string()), description_en: Some("Timestamp precision: ms or ns".to_string()),
            attribute: ParamAttribute::Optional, ty: ParamType::Select,
            default: Some(serde_json::json!(DEFAULT_PRECISION)), valid: None,
            options: Some(vec![
                ParamOption { value: serde_json::json!("ms"), label: Some("Milliseconds".to_string()), label_zh: Some("毫秒".to_string()), label_en: Some("Milliseconds".to_string()) },
                ParamOption { value: serde_json::json!("ns"), label: Some("Nanoseconds".to_string()), label_zh: Some("纳秒".to_string()), label_en: Some("Nanoseconds".to_string()) },
            ]),
            ..Default::default()
        })
        .param(ParamSchema {
            name: "measurement_template".to_string(), name_zh: Some("Measurement 模板".to_string()), name_en: Some("Measurement Template".to_string()),
            description: Some("Measurement 模板，支持变量 ${node_id}/${group_id}/${node_name}/${group_name}/${timestamp}".to_string()),
            description_zh: Some("Measurement 模板，支持变量 ${node_id}/${group_id}/${node_name}/${group_name}/${timestamp}".to_string()),
            description_en: Some("Measurement template with ${node_id}/${group_id}/${node_name}/${group_name}/${timestamp}".to_string()),
            attribute: ParamAttribute::Optional, ty: ParamType::String,
            default: Some(serde_json::json!(DEFAULT_MEASUREMENT_TEMPLATE)),
            valid: Some(ParamValid { min: None, max: None, regex: None, length: Some(256) }),
            ..Default::default()
        })
        .param(ParamSchema {
            name: "tags_json".to_string(), name_zh: Some("额外 Tags".to_string()), name_en: Some("Extra Tags".to_string()),
            description: Some("Extra InfluxDB tags as JSON object, e.g. {\"location\":\"room1\"}".to_string()),
            description_zh: Some("额外的 InfluxDB tags，JSON 对象，例如 {\"location\":\"room1\"}".to_string()),
            description_en: Some("Extra InfluxDB tags as JSON object".to_string()),
            attribute: ParamAttribute::Optional, ty: ParamType::String,
            default: None, valid: Some(ParamValid { min: None, max: None, regex: None, length: Some(2048) }),
            ..Default::default()
        })
        .param(ParamSchema {
            name: "batch_max_lines".to_string(), name_zh: Some("批量行数上限".to_string()), name_en: Some("Batch Max Lines".to_string()),
            description: Some("Max lines per write request (0 = send immediately)".to_string()),
            description_zh: Some("每次写入请求的最大行数（0 = 逐条发送）".to_string()), description_en: Some("Max lines per write request".to_string()),
            attribute: ParamAttribute::Optional, ty: ParamType::Int,
            default: Some(serde_json::json!(DEFAULT_BATCH_MAX_LINES as i64)),
            valid: Some(ParamValid { min: Some(0), max: Some(100_000), regex: None, length: None }),
            ..Default::default()
        })
        .param(ParamSchema {
            name: "batch_interval_ms".to_string(), name_zh: Some("批量间隔（MS）".to_string()), name_en: Some("Batch Interval (MS)".to_string()),
            description: Some("Interval to flush partial batch in milliseconds (0 = flush immediately when max_lines reached)".to_string()),
            description_zh: Some("批量Flush间隔，单位毫秒（0 = 达到 max_lines 时立即发送）".to_string()),
            description_en: Some("Flush interval for partial batches in milliseconds".to_string()),
            attribute: ParamAttribute::Optional, ty: ParamType::Int,
            default: Some(serde_json::json!(DEFAULT_BATCH_INTERVAL_MS as i64)),
            valid: Some(ParamValid { min: Some(0), max: Some(600_000), regex: None, length: None }),
            ..Default::default()
        })
        .param(ParamSchema {
            name: "insecure_skip_verify".to_string(), name_zh: Some("跳过证书验证".to_string()), name_en: Some("Skip Cert Verify".to_string()),
            description: Some("Skip TLS certificate verification (for self-signed certs)".to_string()),
            description_zh: Some("跳过 TLS 证书验证".to_string()), description_en: Some("Skip TLS certificate verification".to_string()),
            attribute: ParamAttribute::Optional, ty: ParamType::Bool,
            default: Some(serde_json::json!(false)), valid: None,
            ..Default::default()
        })
        .param(ParamSchema {
            name: "ca_file".to_string(), name_zh: Some("CA 证书".to_string()), name_en: Some("CA File".to_string()),
            description: Some("Custom CA certificate file (PEM)".to_string()),
            description_zh: Some("自定义 CA 证书文件（PEM 格式）".to_string()), description_en: Some("Custom CA certificate file (PEM)".to_string()),
            attribute: ParamAttribute::Optional, ty: ParamType::File,
            default: None, valid: None,
            ..Default::default()
        })
        .param(ParamSchema {
            name: "cache_memory_size".to_string(), name_zh: Some("缓存内存大小".to_string()), name_en: Some("Cache Memory Size".to_string()),
            description: Some("Max in-memory cache size (message count) when connection fails.".to_string()),
            description_zh: Some("连接异常时暂存的最大条数。".to_string()), description_en: Some("Max in-memory cache size when connection fails.".to_string()),
            attribute: ParamAttribute::Optional, ty: ParamType::Int,
            default: Some(serde_json::json!(gateway_plugin_common::DEFAULT_CACHE_MEMORY_SIZE)), valid: None,
            ..Default::default()
        })
        .param(ParamSchema {
            name: "cache_persist".to_string(), name_zh: Some("离线队列落盘".to_string()), name_en: Some("Persist Offline Queue".to_string()),
            description: Some("Persist the offline queue to disk.".to_string()),
            description_zh: Some("离线队列是否落盘。".to_string()), description_en: Some("Persist offline queue to disk.".to_string()),
            attribute: ParamAttribute::Optional, ty: ParamType::Bool,
            default: Some(serde_json::json!(true)), valid: None,
            ..Default::default()
        })
        .param(ParamSchema {
            name: "cache_dir".to_string(), name_zh: Some("离线队列目录".to_string()), name_en: Some("Offline Queue Directory".to_string()),
            description: Some("Directory for per-node offline queue files.".to_string()),
            description_zh: Some(format!("每个节点一个文件，默认 {}", DEFAULT_CACHE_DIR_INFLUXDB).to_string()),
            description_en: Some("Directory for per-node offline queue files.".to_string()),
            attribute: ParamAttribute::Optional, ty: ParamType::String,
            default: Some(serde_json::json!(DEFAULT_CACHE_DIR_INFLUXDB)), valid: None,
            ..Default::default()
        })
        .param(ParamSchema {
            name: "cache_sync_interval_ms".to_string(), name_zh: Some("缓存消息重传间隔（MS）".to_string()), name_en: Some("Cache Sync Interval (MS)".to_string()),
            description: Some("Interval for replaying cached messages after reconnect.".to_string()),
            description_zh: Some("恢复连接后补发缓存消息的时间间隔。".to_string()), description_en: Some("Interval for replaying cached messages.".to_string()),
            attribute: ParamAttribute::Optional, ty: ParamType::Int,
            default: Some(serde_json::json!(gateway_plugin_common::DEFAULT_CACHE_SYNC_INTERVAL_MS as i64)),
            valid: Some(ParamValid { min: Some(10), max: Some(120_000), regex: None, length: None }),
            ..Default::default()
        })
}
