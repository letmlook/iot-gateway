//! 北向 InfluxDB v2 插件：把 GroupData 编码为 InfluxDB 行协议，批量写入 /api/v2/write。

#[cfg(feature = "ffi")]
mod ffi;

use gateway_plugin_common::config::{
    config_bool, config_str, config_u64, config_usize, default_cache_dir, queue_path_for_node,
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
    /// 渲染后的 measurement
    measurement_template: String,
    /// 额外的静态 tags（从 tags_json 配置解析）
    extra_tags: Vec<(String, String)>,
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
        let measurement_template = config_str(
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
        // 缺省目录：优先 <GATEWAY_DATA_DIR>/influxdb-queue，未设置该环境变量时保持原默认（CWD 相对）
        let cache_dir = config_str(
            &config,
            "cache_dir",
            &default_cache_dir("influxdb-queue", DEFAULT_CACHE_DIR_INFLUXDB),
        );
        let insecure_skip_verify = config_bool(&config, "insecure_skip_verify", false);
        let ca_file = config_str(&config, "ca_file", "");

        if org.is_empty() || bucket.is_empty() {
            return Err(gateway_sdk::PluginError::config_invalid(
                "org and bucket are required".to_string(),
            ));
        }

        // 解析 tags_json
        let extra_tags: Vec<(String, String)> = if tags_json.is_empty() {
            Vec::new()
        } else {
            let map: HashMap<String, String> = serde_json::from_str(&tags_json).map_err(|e| {
                gateway_sdk::PluginError::config_invalid(format!("tags_json parse failed: {}", e))
            })?;
            map.into_iter().collect()
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
                measurement_template,
                extra_tags,
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

        // 从配置获取模板和额外 tags
        let measurement_template = node_state.measurement_template.clone();
        let extra_tags = node_state.extra_tags.clone();

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

        // 按 design §3.4 渲染 measurement 模板
        let measurement = measurement_template
            .replace("${node_id}", &data.node_id.0.to_string())
            .replace("${group_id}", &data.group_id.0.to_string())
            .replace("${node_name}", &node_name)
            .replace("${group_name}", &group_name)
            .replace("${timestamp}", &data.ts.to_rfc3339());

        // tags: node=<node_name>, group=<group_name> + 用户追加的 extra_tags
        let mut tags = vec![
            ("node".to_string(), node_name.clone()),
            ("group".to_string(), group_name),
        ];
        tags.extend(extra_tags);

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
            // 缺陷①修复（worker 引导死锁）：无论 connected 与否都把记录交给 worker，
            // 由 worker 统一负责发送/入队/重试。旧逻辑在 !connected 时直接入队，
            // 而 worker 的记录唯一来源是 mpsc 通道、connected 又只能由「发送成功」置位
            // → 通道永远空、首条消息永远发不出。
            let payload_bytes = line.into_bytes();
            if let Some(tx) = node_state.tx.clone() {
                let rec = Record {
                    topic: String::new(),
                    payload: payload_bytes.clone(),
                };
                if tx.try_send(rec).is_err() {
                    // 通道满：进离线队列（保序）
                    enqueue(node_state, payload_bytes).await;
                }
            } else {
                enqueue(node_state, payload_bytes).await;
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
    // 缺陷①修复：初始即武装补发分支（healthy=false）——
    // worker 启动时就会尝试投递离线队列（含磁盘恢复的记录），而不是等「第一条发送成功」；
    // 之后 healthy 由「离线队列是否清空」维护：非空则保持重试，清空即解除。
    let mut healthy = false;
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
                            let records: Vec<Record> = std::mem::take(&mut batch);
                            let body = join_payloads(records.iter().map(|r| r.payload.clone()).collect());
                            let class = send_lines(&client, &url, &org, &bucket, &token, &precision, body).await;
                            handle_class(class, &connection_status, &mut backoff).await;
                            settle_batch(&queue, class, records, &mut healthy).await;
                        }
                    }
                    None => break,
                }
            }
            _ = tokio::time::sleep(batch_interval), if !batch.is_empty() && batch_interval > Duration::ZERO => {
                let records: Vec<Record> = std::mem::take(&mut batch);
                let body = join_payloads(records.iter().map(|r| r.payload.clone()).collect());
                let class = send_lines(&client, &url, &org, &bucket, &token, &precision, body).await;
                handle_class(class, &connection_status, &mut backoff).await;
                settle_batch(&queue, class, records, &mut healthy).await;
            }
            _ = tokio::time::sleep(backoff), if !healthy => {
                let restored = replenish_queue(&queue, &client, &url, &org, &bucket, &token, &precision, interval).await;
                let empty = queue.lock().await.is_empty();
                if empty {
                    if restored > 0 {
                        // 补发成功即证明服务可达：connected 语义必须真实反映这一点
                        let mut cs = connection_status.write().await;
                        cs.connected = true;
                        cs.last_error = None;
                        log::info(node_id, format!("influxdb offline queue flushed {} record(s)", restored));
                    }
                    healthy = true;
                    backoff = Duration::from_secs(1);
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
    backoff: &mut Duration,
) {
    match class {
        CommonHttpClass::Delivered => {
            *backoff = Duration::from_secs(1);
            let mut cs = connection_status.write().await;
            cs.connected = true;
            cs.last_error = None;
        }
        CommonHttpClass::Retryable => {
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

/// 批量发送结果落地（缺陷①修复：发送失败可靠入队）。
/// Retryable 时整批按序回灌离线队列，等待补发分支重试；
/// healthy 由「离线队列是否清空」决定——非空则保持补发分支武装（即使本次发送成功）。
async fn settle_batch(
    queue: &Arc<tokio::sync::Mutex<OfflineQueue>>,
    class: CommonHttpClass,
    records: Vec<Record>,
    healthy: &mut bool,
) {
    let mut q = queue.lock().await;
    if matches!(class, CommonHttpClass::Retryable) {
        for r in records {
            q.push(r);
        }
    }
    *healthy = q.is_empty();
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
            description_zh: Some(format!("每个节点一个文件，默认 {}（未显式配置时优先取 GATEWAY_DATA_DIR 环境变量，即 <GATEWAY_DATA_DIR>/influxdb-queue）", DEFAULT_CACHE_DIR_INFLUXDB).to_string()),
            description_en: Some("Directory for per-node offline queue files. Defaults to <GATEWAY_DATA_DIR>/influxdb-queue when the env var is set, otherwise the built-in default.".to_string()),
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

#[cfg(test)]
mod tests {
    use super::*;

    // ---------------------------------------------------------------------------
    // join_payloads tests (sync)
    // ---------------------------------------------------------------------------

    #[test]
    fn join_payloads_empty() {
        let payloads: Vec<Vec<u8>> = vec![];
        let result = join_payloads(payloads);
        assert!(result.is_empty());
    }

    #[test]
    fn join_payloads_single() {
        let payloads = vec![b"cpu,host=srv1 val=25.5i".to_vec()];
        let result = join_payloads(payloads);
        assert_eq!(result, b"cpu,host=srv1 val=25.5i".to_vec());
    }

    #[test]
    fn join_payloads_multiple_lines() {
        let payloads = vec![
            b"cpu,host=srv1 val=25.5i".to_vec(),
            b"cpu,host=srv2 val=30.0i".to_vec(),
        ];
        let result = join_payloads(payloads);
        assert_eq!(
            result,
            b"cpu,host=srv1 val=25.5i\ncpu,host=srv2 val=30.0i".to_vec()
        );
    }

    #[test]
    fn join_payloads_preserves_content_with_newlines() {
        // each payload may itself contain \n (line protocol allows this in field values
        // only when properly escaped, but join_payloads treats each vec as atomic)
        let payloads = vec![
            b"m,f1=1i".to_vec(),
            b"m,f2=2i".to_vec(),
            b"m,f3=3i".to_vec(),
        ];
        let result = join_payloads(payloads);
        let expected = b"m,f1=1i\nm,f2=2i\nm,f3=3i".to_vec();
        assert_eq!(result, expected);
    }

    // ---------------------------------------------------------------------------
    // HTTP status code → HttpClass classification
    // ---------------------------------------------------------------------------

    #[test]
    fn http_class_204_delivered() {
        assert_eq!(
            gateway_plugin_common::http::HttpClass::from_status_code(204),
            gateway_plugin_common::http::HttpClass::Delivered
        );
    }

    #[test]
    fn http_class_200_delivered() {
        assert_eq!(
            gateway_plugin_common::http::HttpClass::from_status_code(200),
            gateway_plugin_common::http::HttpClass::Delivered
        );
    }

    #[test]
    fn http_class_400_rejected() {
        assert_eq!(
            gateway_plugin_common::http::HttpClass::from_status_code(400),
            gateway_plugin_common::http::HttpClass::Rejected
        );
    }

    #[test]
    fn http_class_401_rejected() {
        assert_eq!(
            gateway_plugin_common::http::HttpClass::from_status_code(401),
            gateway_plugin_common::http::HttpClass::Rejected
        );
    }

    #[test]
    fn http_class_429_retryable() {
        assert_eq!(
            gateway_plugin_common::http::HttpClass::from_status_code(429),
            gateway_plugin_common::http::HttpClass::Retryable
        );
    }

    #[test]
    fn http_class_500_retryable() {
        assert_eq!(
            gateway_plugin_common::http::HttpClass::from_status_code(500),
            gateway_plugin_common::http::HttpClass::Retryable
        );
    }

    #[test]
    fn http_class_503_retryable() {
        assert_eq!(
            gateway_plugin_common::http::HttpClass::from_status_code(503),
            gateway_plugin_common::http::HttpClass::Retryable
        );
    }

    // ---------------------------------------------------------------------------
    // Line protocol encoding via common::line (shared with tdengine)
    // Verifies the integration path used in on_group_data
    // ---------------------------------------------------------------------------

    #[test]
    fn line_encoding_temperature_int() {
        use gateway_plugin_common::line::{encode, LineField, LinePoint};
        let point = LinePoint {
            measurement: "sensor".to_string(),
            tags: vec![
                ("node".to_string(), "temp-01".to_string()),
                ("group".to_string(), "env".to_string()),
            ],
            fields: vec![
                ("temperature".to_string(), LineField::Int(256)),
                ("humidity".to_string(), LineField::Int(85)),
            ],
            ts_ms: 1704067200000,
        };
        let mut out = String::new();
        encode(&point, &mut out);
        // measurement must not contain unescaped comma
        assert!(!out.starts_with("sensor,,"));
        // integer suffix
        assert!(out.contains("256i"));
        assert!(out.contains("85i"));
        // ends with timestamp
        assert!(out.ends_with("1704067200000"));
    }

    #[test]
    fn line_encoding_with_special_chars() {
        use gateway_plugin_common::line::{encode, LineField, LinePoint};
        let point = LinePoint {
            measurement: "cpu,usage".to_string(), // comma and space must be escaped
            tags: vec![("k=v".to_string(), "v b".to_string())], // = and space must be escaped
            fields: vec![(
                "msg".to_string(),
                LineField::Str(r#"say "hi"\test"#.to_string()),
            )],
            ts_ms: 1000,
        };
        let mut out = String::new();
        encode(&point, &mut out);
        // escaped comma in measurement
        assert!(out.starts_with("cpu\\,usage"));
        // escaped equals in tag key
        assert!(out.contains("k\\=v="));
        // escaped space in tag value
        assert!(out.contains("v\\ b"));
        // string field in double quotes with escaped backslash and quote
        assert!(out.contains(r#""say \"hi\"\\test""#));
    }

    #[test]
    fn line_encoding_bool_and_float() {
        use gateway_plugin_common::line::{encode, LineField, LinePoint};
        let point = LinePoint {
            measurement: "status".to_string(),
            tags: vec![],
            fields: vec![
                ("online".to_string(), LineField::Bool(true)),
                ("temp".to_string(), LineField::Float(36.6)),
            ],
            ts_ms: 2000,
        };
        let mut out = String::new();
        encode(&point, &mut out);
        // no tags → no comma after measurement; bool field has 't' suffix
        assert!(out.contains("online=t"));
        assert!(out.contains("temp="));
    }

    #[test]
    fn line_encoding_unsigned_int() {
        use gateway_plugin_common::line::{encode, LineField, LinePoint};
        let point = LinePoint {
            measurement: "counter".to_string(),
            tags: vec![],
            fields: vec![("packets".to_string(), LineField::UInt(1234567890u64))],
            ts_ms: 3000,
        };
        let mut out = String::new();
        encode(&point, &mut out);
        assert!(out.contains("1234567890u"));
    }

    // ---------------------------------------------------------------------------
    // HTTP status code → HttpClass classification (already covered by
    // common::http::tests; additional integration-level tests via send_lines)
    // ---------------------------------------------------------------------------

    // Note: reqwest-based mock server tests are inherently fragile on Windows due
    // to TCP stream timing. The core logic is covered by:
    //   - gateway_plugin_common::http::tests::status_code_classification
    //   - send_lines returns HttpClass based on status codes (tested below)
    //   - URL construction is a simple format! string (§3.4 requirement)
    //
    // The actual HTTP request construction (URL + headers) is verified by:
    //   1. Compilation — send_lines compiles with correct header calls
    //   2. Integration test: successful 204 response through the real send_lines path

    #[test]
    fn send_lines_classifies_204_as_delivered() {
        use std::io::{Read, Write};
        use std::net::{SocketAddr, TcpListener};

        let listener = TcpListener::bind("127.0.0.1:0".to_string()).unwrap();
        let addr: SocketAddr = listener.local_addr().unwrap();
        let url = format!("http://{}", addr);

        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            let n = stream.read(&mut buf).unwrap();
            // Send minimal 204 response
            stream
                .write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n")
                .unwrap();
            stream.flush().unwrap();
            String::from_utf8_lossy(&buf[..n]).to_string()
        });

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let client = reqwest::Client::new();
        let class = rt.block_on(send_lines(
            &client,
            &url,
            "my-org",
            "my-bucket",
            "my-token",
            "ms",
            b"cpu val=1i".to_vec(),
        ));
        assert_eq!(class, gateway_plugin_common::http::HttpClass::Delivered);
        handle.join().unwrap();
    }

    #[test]
    fn send_lines_classifies_400_as_rejected() {
        use std::io::{Read, Write};
        use std::net::{SocketAddr, TcpListener};

        let listener = TcpListener::bind("127.0.0.1:0".to_string()).unwrap();
        let addr: SocketAddr = listener.local_addr().unwrap();
        let url = format!("http://{}", addr);

        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            let _n = stream.read(&mut buf).unwrap();
            stream
                .write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n")
                .unwrap();
            stream.flush().unwrap();
        });

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let client = reqwest::Client::new();
        let class = rt.block_on(send_lines(
            &client,
            &url,
            "org",
            "bucket",
            "token",
            "ms",
            b"bad".to_vec(),
        ));
        assert_eq!(class, gateway_plugin_common::http::HttpClass::Rejected);
        handle.join().unwrap();
    }

    #[test]
    fn send_lines_classifies_429_as_retryable() {
        use std::io::{Read, Write};
        use std::net::{SocketAddr, TcpListener};

        let listener = TcpListener::bind("127.0.0.1:0".to_string()).unwrap();
        let addr: SocketAddr = listener.local_addr().unwrap();
        let url = format!("http://{}", addr);

        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            let _n = stream.read(&mut buf).unwrap();
            stream
                .write_all(b"HTTP/1.1 429 Too Many Requests\r\nContent-Length: 0\r\n\r\n")
                .unwrap();
            stream.flush().unwrap();
        });

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let client = reqwest::Client::new();
        let class = rt.block_on(send_lines(
            &client,
            &url,
            "org",
            "bucket",
            "token",
            "ms",
            b"cpu".to_vec(),
        ));
        assert_eq!(class, gateway_plugin_common::http::HttpClass::Retryable);
        handle.join().unwrap();
    }

    #[test]
    fn send_lines_classifies_500_as_retryable() {
        use std::io::{Read, Write};
        use std::net::{SocketAddr, TcpListener};

        let listener = TcpListener::bind("127.0.0.1:0".to_string()).unwrap();
        let addr: SocketAddr = listener.local_addr().unwrap();
        let url = format!("http://{}", addr);

        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            let _n = stream.read(&mut buf).unwrap();
            stream
                .write_all(b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\n\r\n")
                .unwrap();
            stream.flush().unwrap();
        });

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let client = reqwest::Client::new();
        let class = rt.block_on(send_lines(
            &client,
            &url,
            "org",
            "bucket",
            "token",
            "ms",
            b"cpu".to_vec(),
        ));
        assert_eq!(class, gateway_plugin_common::http::HttpClass::Retryable);
        handle.join().unwrap();
    }

    // ---------------------------------------------------------------------------
    // 回归测试（docs/联调记录-2026-10-07.md §5.1 缺陷①：北向 worker 引导死锁）
    //
    // 缺陷①下：on_group_data 在 !connected 时直接入队且从不唤醒 worker，
    // InfluxDB 查询 0 点、queue_len 持续增长。以下测试用本地 TCP 服务端验证：
    // 首条数据真实写出、发送失败可靠入队、服务恢复后补发。
    // ---------------------------------------------------------------------------

    fn make_node_id() -> NodeId {
        NodeId(uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap())
    }

    fn make_group_id() -> gateway_sdk::GroupId {
        gateway_sdk::GroupId(uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap())
    }

    fn make_group_data() -> Arc<GroupData> {
        use gateway_sdk::types::{DataValue, TagId};
        let node_id = make_node_id();
        let group_id = make_group_id();
        let ts = chrono::TimeZone::with_ymd_and_hms(&chrono::Utc, 2024, 1, 1, 0, 0, 0).unwrap();
        let mut tag_names = std::collections::HashMap::new();
        tag_names.insert(
            TagId(uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000003").unwrap()),
            "temperature".to_string(),
        );
        Arc::new(GroupData {
            node_id,
            group_id,
            ts,
            values: vec![(
                TagId(uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000003").unwrap()),
                DataValue::Float64(25.6),
            )],
            node_name: Some("sensor-01".to_string()),
            group_name: Some("env".to_string()),
            tag_names: Some(tag_names),
        })
    }

    fn influx_config(
        url: &str,
        overrides: std::collections::HashMap<&str, serde_json::Value>,
    ) -> PluginConfig {
        let mut cfg: PluginConfig = std::collections::HashMap::new();
        cfg.insert("url".to_string(), serde_json::json!(url));
        cfg.insert("org".to_string(), serde_json::json!("itest"));
        cfg.insert("bucket".to_string(), serde_json::json!("gateway"));
        cfg.insert("token".to_string(), serde_json::json!("0123456789abcdef"));
        cfg.insert(
            "measurement_template".to_string(),
            serde_json::json!("${node_name}"),
        );
        // 单条立即发送，保证测试时序确定
        cfg.insert("batch_max_lines".to_string(), serde_json::json!(1));
        cfg.insert("cache_persist".to_string(), serde_json::json!(false));
        cfg.insert("cache_sync_interval_ms".to_string(), serde_json::json!(50));
        for (k, v) in overrides {
            cfg.insert(k.to_string(), v);
        }
        cfg
    }

    fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        haystack.windows(needle.len()).position(|w| w == needle)
    }

    /// 读完一个 HTTP 请求（头 + Content-Length body），返回原文
    fn read_request(stream: &mut std::net::TcpStream) -> std::io::Result<String> {
        use std::io::Read;
        let mut buf = Vec::new();
        let mut chunk = [0u8; 1024];
        let header_end = loop {
            let n = stream.read(&mut chunk)?;
            if n == 0 {
                break buf.len();
            }
            buf.extend_from_slice(&chunk[..n]);
            if let Some(pos) = find_subsequence(&buf, b"\r\n\r\n") {
                break pos + 4;
            }
        };
        let head = String::from_utf8_lossy(&buf[..header_end.min(buf.len())]).to_string();
        let content_length = head
            .to_ascii_lowercase()
            .lines()
            .find_map(|l| {
                l.strip_prefix("content-length:")
                    .map(|v| v.trim().to_string())
            })
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(0);
        while buf.len() < header_end + content_length {
            let n = stream.read(&mut chunk)?;
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
        }
        Ok(String::from_utf8_lossy(&buf).to_string())
    }

    fn split_request(req: &str) -> (String, String) {
        let header_end = req.find("\r\n\r\n").unwrap_or(req.len());
        let head = &req[..header_end];
        let path = head
            .split_whitespace()
            .nth(1)
            .unwrap_or_default()
            .to_string();
        let body = req[header_end..].trim_start_matches("\r\n\r\n").to_string();
        (path, body)
    }

    /// 极简 HTTP 服务端：串行 accept，全部应答 204（Connection: close 防止连接复用），
    /// 每个请求以 (路径, body) 推入 channel。
    fn spawn_line_server(
        listener: std::net::TcpListener,
    ) -> std::sync::mpsc::Receiver<(String, String)> {
        use std::io::Write;
        let (tx, rx) = std::sync::mpsc::channel::<(String, String)>();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let Ok(req) = read_request(&mut stream) else {
                    continue;
                };
                let resp =
                    "HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                let _ = stream.write_all(resp.as_bytes());
                let _ = stream.flush();
                let (path, body) = split_request(&req);
                if tx.send((path, body)).is_err() {
                    break;
                }
            }
        });
        rx
    }

    async fn wait_for_status<F: Fn(&serde_json::Value) -> bool>(
        plugin: &InfluxDbPlugin,
        node_id: NodeId,
        pred: F,
        timeout: std::time::Duration,
    ) -> bool {
        let deadline = tokio::time::Instant::now() + timeout;
        while tokio::time::Instant::now() < deadline {
            if let Some(st) = plugin.connection_status(node_id).await {
                if pred(&st) {
                    return true;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        false
    }

    /// 回归测试（缺陷①核心）：节点启动后第一条数据必须真实写到服务端。
    /// 缺陷①下 InfluxDB 0 点、queue_len 持续增长，本测试会在此失败。
    #[tokio::test]
    async fn first_record_reaches_real_server_and_sets_connected() {
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let rx = spawn_line_server(listener);

        let plugin = InfluxDbPlugin::new();
        let node_id = make_node_id();
        plugin
            .open(
                node_id,
                influx_config(&format!("http://{addr}"), Default::default()),
            )
            .await
            .unwrap();

        // 无连接层反馈（reqwest 无连接回调），connected 由首条成功写入置位，初始为 false
        let st = plugin.connection_status(node_id).await.unwrap();
        assert_eq!(st["connected"], serde_json::json!(false));

        plugin
            .on_group_data(node_id, make_group_data())
            .await
            .unwrap();

        // 首条行协议必须真实到达服务端
        let (path, body) = rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("first record must be delivered to the real server (defect ① regression)");
        assert!(
            path.starts_with("/api/v2/write?"),
            "unexpected path: {}",
            path
        );
        assert!(path.contains("org=itest") && path.contains("bucket=gateway"));
        assert!(
            body.contains("sensor-01") && body.contains("temperature=25.6"),
            "line protocol should carry measurement/tags/fields: {}",
            body
        );

        // 首条成功写入后 connected 置位、队列不积压
        assert!(
            wait_for_status(
                &plugin,
                node_id,
                |st| st["connected"] == serde_json::json!(true),
                std::time::Duration::from_secs(5)
            )
            .await,
            "connected should become true after first successful delivery"
        );
        let st = plugin.connection_status(node_id).await.unwrap();
        assert_eq!(
            st["queue_len"],
            serde_json::json!(0),
            "no backlog after delivery"
        );

        plugin.close(node_id).await.unwrap();
    }

    /// 发送失败（服务不可用）必须可靠入队；服务恢复后由补发分支按序清空离线队列
    #[tokio::test]
    async fn failed_send_is_enqueued_and_replayed_after_recovery() {
        use std::net::TcpListener;

        // 先占端口再释放：写入端在恢复前连接被拒（Retryable）
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);

        let plugin = InfluxDbPlugin::new();
        let node_id = make_node_id();
        plugin
            .open(
                node_id,
                influx_config(&format!("http://{addr}"), Default::default()),
            )
            .await
            .unwrap();

        plugin
            .on_group_data(node_id, make_group_data())
            .await
            .unwrap();
        assert!(
            wait_for_status(
                &plugin,
                node_id,
                |st| st["queue_len"] == serde_json::json!(1),
                std::time::Duration::from_secs(5)
            )
            .await,
            "failed record must be reliably enqueued (queue_len == 1)"
        );

        // 服务恢复 → 补发分支应清空离线队列并保持 connected
        let rx = spawn_line_server(TcpListener::bind(addr).unwrap());
        assert!(
            wait_for_status(
                &plugin,
                node_id,
                |st| st["queue_len"] == serde_json::json!(0)
                    && st["connected"] == serde_json::json!(true),
                std::time::Duration::from_secs(10)
            )
            .await,
            "offline queue must be flushed after recovery"
        );
        let (path, body) = rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("queued record must be replayed to the server");
        assert!(path.starts_with("/api/v2/write?"));
        assert!(body.contains("temperature=25.6"));

        plugin.close(node_id).await.unwrap();
    }
}
