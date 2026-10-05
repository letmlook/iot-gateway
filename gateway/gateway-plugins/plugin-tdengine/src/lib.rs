//! 北向 TDengine 插件：通过 taosAdapter REST /influxdb/v1/write 写入行协议。

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

const DEFAULT_CACHE_DIR_TDENGINE: &str = "data/tdengine-queue";
const DEFAULT_URL: &str = "http://127.0.0.1:6041";
const DEFAULT_DATABASE: &str = "gateway";
const DEFAULT_USERNAME: &str = "root";
const DEFAULT_PASSWORD: &str = "taosdata";
const DEFAULT_TABLE_NAME_KEY: &str = "gateway_table";
const DEFAULT_BATCH_MAX_LINES: usize = 500;
const DEFAULT_BATCH_INTERVAL_MS: u64 = 200;
/// TDengine 3.x taosAdapter 支持的写入端点（已通过源码核实，无 /v2/write）
const TAOS_WRITE_PATH: &str = "/influxdb/v1/write";
/// taosAdapter /rest/sql 用于建库
const TAOS_SQL_PATH: &str = "/rest/sql";
/// database 名白名单：^[A-Za-z0-9_]{1,192}$
const DB_NAME_REGEX: &str = "^[A-Za-z0-9_]{1,192}$";

#[derive(Debug, Default)]
struct TdEngineConnectionStatus {
    connected: bool,
    last_error: Option<String>,
    dropped_rejected: u64,
    dropped_no_client: u64,
    skipped_bytes_fields: u64,
}

struct NodeTdEngineState {
    queue: Arc<tokio::sync::Mutex<OfflineQueue>>,
    connection_status: Arc<RwLock<TdEngineConnectionStatus>>,
    tx: Option<mpsc::Sender<Record>>,
    cancel_tx: Option<tokio::sync::oneshot::Sender<()>>,
    event_loop_handle: Option<std::thread::JoinHandle<()>>,
}

struct TdEngineState {
    open_nodes: std::collections::HashSet<NodeId>,
    subscriptions: HashMap<NodeId, Vec<GroupSubscription>>,
    nodes: HashMap<NodeId, NodeTdEngineState>,
}

pub struct TdEnginePlugin {
    state: Arc<RwLock<TdEngineState>>,
}

impl Default for TdEnginePlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl TdEnginePlugin {
    pub fn new() -> Self {
        Self {
            state: Arc::new(RwLock::new(TdEngineState {
                open_nodes: std::collections::HashSet::new(),
                subscriptions: HashMap::new(),
                nodes: HashMap::new(),
            })),
        }
    }
}

#[async_trait::async_trait]
impl NorthPlugin for TdEnginePlugin {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "tdengine",
            kind: PluginKind::North,
            description: Some("TDengine 北向：通过 taosAdapter REST /influxdb/v1/write 行协议写入，支持 Basic 认证、数据库自动创建、离线缓存"),
            version: "0.1.0",
            name_zh: Some("TDengine"),
            name_en: Some("TDengine"),
            description_zh: Some("TDengine 北向：taosAdapter 行协议写入，支持 Basic 认证、自动建库、离线缓存"),
            description_en: Some("TDengine north: writes via taosAdapter /influxdb/v1/write REST API with line protocol, Basic auth, auto-DB-creation, offline cache"),
        }
    }

    fn config_schema(&self) -> Option<gateway_sdk::ConfigSchema> {
        Some(config_schema())
    }

    async fn open(&self, node_id: NodeId, config: PluginConfig) -> PluginResult<()> {
        let url = config_str(&config, "url", DEFAULT_URL);
        let database = config_str(&config, "database", DEFAULT_DATABASE);
        let username = config_str(&config, "username", DEFAULT_USERNAME);
        let password = config_str(&config, "password", DEFAULT_PASSWORD);
        let measurement_template = config_str(&config, "measurement_template", "${node_name}");
        let tags_json = config_str(&config, "tags_json", "");
        let table_name_key = config_str(&config, "table_name_key", DEFAULT_TABLE_NAME_KEY);
        let auto_create_db = config_bool(&config, "auto_create_db", true);
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
        let cache_dir = config_str(&config, "cache_dir", DEFAULT_CACHE_DIR_TDENGINE);
        let insecure_skip_verify = config_bool(&config, "insecure_skip_verify", false);
        let ca_file = config_str(&config, "ca_file", "");

        // database 白名单校验（正则 + 代码双重）
        if !regex::Regex::new(DB_NAME_REGEX)
            .map(|r| r.is_match(&database))
            .unwrap_or(false)
        {
            return Err(gateway_sdk::PluginError::config_invalid(format!(
                "database name '{}' does not match whitelist pattern {}",
                database, DB_NAME_REGEX
            )));
        }

        // 解析 tags_json
        let extra_tags: HashMap<String, String> = if tags_json.is_empty() {
            HashMap::new()
        } else {
            serde_json::from_str(&tags_json).map_err(|e| {
                gateway_sdk::PluginError::config_invalid(format!("tags_json parse failed: {}", e))
            })?
        };

        log::info(
            node_id,
            format!("open tdengine: {} database={}", url, database),
        );

        let mut state = self.state.write().await;
        state.open_nodes.insert(node_id);

        let queue_path = if cache_persist {
            queue_path_for_node(&cache_dir, node_id, "-tdengine")
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

        let connection_status = Arc::new(RwLock::new(TdEngineConnectionStatus {
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

        // Basic auth header
        let basic_auth = base64_encode(&format!("{}:{}", username, password));

        // 自动建库（失败只 warn 不阻塞）
        if auto_create_db {
            let db_url = format!("{}{}?db={}", url, TAOS_SQL_PATH, database);
            let create_sql = format!("CREATE DATABASE IF NOT EXISTS {} PRECISION 'ms'", database);
            let req = http_client
                .post(&db_url)
                .header("Authorization", format!("Basic {}", basic_auth))
                .header("Content-Type", "application/json")
                .body(serde_json::json!({ "sql": create_sql }).to_string());
            match req.send().await {
                Ok(resp) => {
                    let status = resp.status().as_u16();
                    if status == 200 {
                        log::info(node_id, format!("tdengine database '{}' ensured", database));
                    } else {
                        let body = resp.text().await.unwrap_or_default();
                        log::warn(
                            node_id,
                            format!(
                                "tdengine create database failed (status {}): {}",
                                status,
                                &body[..body.len().min(200)]
                            ),
                        );
                    }
                }
                Err(e) => {
                    log::warn(
                        node_id,
                        format!(
                            "tdengine create database request failed (will retry on write): {}",
                            e
                        ),
                    );
                }
            }
        }

        // 探活版本
        let version_url = format!("{}{}", url, TAOS_SQL_PATH);
        let req = http_client
            .post(&version_url)
            .header("Authorization", format!("Basic {}", basic_auth))
            .header("Content-Type", "application/json")
            .body(r#"{"sql":"SELECT server_version()"}"#.to_string());
        if let Ok(resp) = req.send().await {
            if resp.status().as_u16() == 200 {
                if let Ok(body) = resp.text().await {
                    log::info(node_id, format!("tdengine server_version: {}", body));
                }
            }
        }

        let (record_tx, record_rx) = mpsc::channel::<Record>(1000);
        let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();

        let queue_clone = queue.clone();
        let conn_status_clone = connection_status.clone();
        let node_id_worker = node_id;
        let url_worker = url.clone();
        let db_worker = database.clone();
        let basic_auth_worker = basic_auth.clone();
        let table_name_key_worker = table_name_key.clone();
        let extra_tags_worker = extra_tags.clone();
        let measurement_template_worker = measurement_template.clone();
        let sync_interval = cache_sync_interval_ms;
        let max_lines = batch_max_lines;
        let batch_interval = batch_interval_ms;

        let handle = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("tdengine event loop runtime");
            rt.block_on(run_tdengine_worker(
                node_id_worker,
                url_worker,
                db_worker,
                basic_auth_worker,
                table_name_key_worker,
                extra_tags_worker,
                measurement_template_worker,
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
            NodeTdEngineState {
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
        log::info(node_id, "close tdengine, disconnecting");
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
        log::info(node_id, "start tdengine");
        Ok(())
    }

    async fn stop(&self, node_id: NodeId) -> PluginResult<()> {
        log::info(node_id, "stop tdengine");
        Ok(())
    }

    async fn setting(&self, node_id: NodeId, config: PluginConfig) -> PluginResult<()> {
        log::info(node_id, "setting tdengine, reconnecting with new config");
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
                "on_group_data: north node not open (no TDengine client), skip",
            );
            return Ok(());
        };

        let node_name = data
            .node_name
            .clone()
            .unwrap_or_else(|| data.node_id.0.to_string());
        let group_name = data
            .group_name
            .clone()
            .unwrap_or_else(|| data.group_id.0.to_string());
        let measurement = "${node_name}".replace("${node_name}", &node_name);

        // 合成 gateway_table tag（table_name_key 指定的 tag 作为子表名）
        let gateway_table = format!(
            "{}_{}",
            sanitize_table_name(&node_name),
            sanitize_table_name(&group_name)
        );

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

        // 构建行协议行
        let mut line = String::new();
        let point = LinePoint {
            measurement,
            tags: vec![
                ("node".to_string(), node_name.clone()),
                ("group".to_string(), group_name),
                // table_name_key 指定 tag 作为子表名
                (DEFAULT_TABLE_NAME_KEY.to_string(), gateway_table),
            ],
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
            if !connected {
                enqueue(node_state, line.into_bytes()).await;
                return Ok(());
            }
            if let Some(tx) = tx_opt {
                let payload_bytes = line.into_bytes();
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
            tracing::info!(node_id = ?node_id, "tdengine (no client): would send");
        }

        Ok(())
    }
}

/// 清理字符串为 [A-Za-z0-9_]+（用于 gateway_table tag 值）
fn sanitize_table_name(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

async fn enqueue(node_state: &NodeTdEngineState, payload: Vec<u8>) {
    let mut q = node_state.queue.lock().await;
    let before = q.stats().dropped_overflow;
    q.push(Record {
        topic: String::new(),
        payload,
    });
    if q.stats().dropped_overflow > before {
        tracing::warn!("tdengine offline queue full, dropped oldest record(s)");
    }
}

// ---------------------------------------------------------------------------
// Worker
// ---------------------------------------------------------------------------

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
async fn run_tdengine_worker(
    node_id: NodeId,
    url: String,
    database: String,
    basic_auth: String,
    table_name_key: String,
    _extra_tags: HashMap<String, String>,
    _measurement_template: String,
    mut record_rx: mpsc::Receiver<Record>,
    queue: Arc<tokio::sync::Mutex<OfflineQueue>>,
    connection_status: Arc<RwLock<TdEngineConnectionStatus>>,
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
    let mut batch = Vec::with_capacity(batch_max_lines);

    loop {
        tokio::select! {
            _ = &mut cancel_rx => {
                log::info(node_id, "tdengine event loop exited (close)");
                break;
            }
            rec = record_rx.recv() => {
                match rec {
                    Some(record) => {
                        batch.push(record);
                        if batch.len() >= batch_max_lines || batch_interval == Duration::ZERO {
                            let body = join_payloads(batch.drain(..).map(|r| r.payload).collect());
                            let class = send_lines(&client, &url, &database, &table_name_key, &basic_auth, body).await;
                            handle_class(class, &connection_status, &mut healthy, &mut backoff, max_backoff).await;
                        }
                    }
                    None => break,
                }
            }
            _ = tokio::time::sleep(batch_interval), if !batch.is_empty() && batch_interval > Duration::ZERO => {
                let body = join_payloads(batch.drain(..).map(|r| r.payload).collect());
                let class = send_lines(&client, &url, &database, &table_name_key, &basic_auth, body).await;
                handle_class(class, &connection_status, &mut healthy, &mut backoff, max_backoff).await;
            }
            _ = tokio::time::sleep(backoff), if !healthy => {
                let restored = replenish_queue(&queue, &client, &url, &database, &table_name_key, &basic_auth, interval).await;
                if restored > 0 {
                    healthy = true;
                    backoff = Duration::from_secs(1);
                    log::info(node_id, format!("tdengine offline queue flushed {} record(s)", restored));
                } else {
                    backoff = (backoff * 2).min(max_backoff);
                }
            }
        }
    }
}

async fn handle_class(
    class: HttpClass,
    connection_status: &Arc<RwLock<TdEngineConnectionStatus>>,
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

/// 发送行协议到 taosAdapter /influxdb/v1/write
async fn send_lines(
    client: &reqwest::Client,
    url: &str,
    database: &str,
    table_name_key: &str,
    basic_auth: &str,
    body: Vec<u8>,
) -> HttpClass {
    let write_url = format!(
        "{}{}?db={}&precision=ms&table_name_key={}",
        url, TAOS_WRITE_PATH, database, table_name_key
    );
    let mut req = client.post(&write_url);
    req = req.header("Authorization", format!("Basic {}", basic_auth));
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
    database: &str,
    table_name_key: &str,
    basic_auth: &str,
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
            database,
            table_name_key,
            basic_auth,
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
        .user_agent("iot-gateway-north/tdengine");

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
    use gateway_sdk::schema::{ParamAttribute, ParamSchema, ParamType, ParamValid};
    use gateway_sdk::ConfigSchema;

    ConfigSchema::new()
        .param(ParamSchema {
            name: "url".to_string(), name_zh: Some("taosAdapter URL".to_string()), name_en: Some("taosAdapter URL".to_string()),
            description: Some("taosAdapter REST URL, default http://127.0.0.1:6041".to_string()),
            description_zh: Some("taosAdapter REST URL，默认为 http://127.0.0.1:6041".to_string()),
            description_en: Some("taosAdapter REST URL, default http://127.0.0.1:6041".to_string()),
            attribute: ParamAttribute::Optional, ty: ParamType::String,
            default: Some(serde_json::json!(DEFAULT_URL)),
            valid: Some(ParamValid { min: None, max: None, regex: Some("^https?://.*".to_string()), length: Some(1024) }),
            ..Default::default()
        })
        .param(ParamSchema {
            name: "database".to_string(), name_zh: Some("数据库名".to_string()), name_en: Some("Database".to_string()),
            description: Some("TDengine database name. Must match ^[A-Za-z0-9_]{1,192}$.".to_string()),
            description_zh: Some("TDengine 数据库名。必须匹配 ^[A-Za-z0-9_]{1,192}$。".to_string()),
            description_en: Some("TDengine database name. Must match ^[A-Za-z0-9_]{1,192}$.".to_string()),
            attribute: ParamAttribute::Required, ty: ParamType::String,
            default: Some(serde_json::json!(DEFAULT_DATABASE)),
            valid: Some(ParamValid { min: None, max: None, regex: Some(DB_NAME_REGEX.to_string()), length: Some(192) }),
            ..Default::default()
        })
        .param(ParamSchema {
            name: "username".to_string(), name_zh: Some("用户名".to_string()), name_en: Some("Username".to_string()),
            description: Some("TDengine username for Basic auth".to_string()),
            description_zh: Some("TDengine 用户名（Basic 认证）".to_string()),
            description_en: Some("TDengine username for Basic auth".to_string()),
            attribute: ParamAttribute::Optional, ty: ParamType::String,
            default: Some(serde_json::json!(DEFAULT_USERNAME)),
            valid: Some(ParamValid { min: None, max: None, regex: None, length: Some(128) }),
            ..Default::default()
        })
        .param(ParamSchema {
            name: "password".to_string(), name_zh: Some("密码".to_string()), name_en: Some("Password".to_string()),
            description: Some("TDengine password for Basic auth. Named with 'password' keyword, auto-masked.".to_string()),
            description_zh: Some("TDengine 密码（Basic 认证），名称含 password 关键字，自动脱敏。".to_string()),
            description_en: Some("TDengine password for Basic auth. Named with 'password' keyword, auto-masked.".to_string()),
            attribute: ParamAttribute::Optional, ty: ParamType::String,
            default: Some(serde_json::json!(DEFAULT_PASSWORD)),
            valid: Some(ParamValid { min: None, max: None, regex: None, length: Some(128) }),
            ..Default::default()
        })
        .sensitive(&["password"])
        .param(ParamSchema {
            name: "measurement_template".to_string(), name_zh: Some("Measurement 模板".to_string()), name_en: Some("Measurement Template".to_string()),
            description: Some("Measurement 模板，支持变量 ${node_id}/${group_id}/${node_name}/${group_name}".to_string()),
            description_zh: Some("Measurement 模板，支持变量 ${node_id}/${group_id}/${node_name}/${group_name}".to_string()),
            description_en: Some("Measurement template with ${node_id}/${group_id}/${node_name}/${group_name}".to_string()),
            attribute: ParamAttribute::Optional, ty: ParamType::String,
            default: Some(serde_json::json!("${node_name}")),
            valid: Some(ParamValid { min: None, max: None, regex: None, length: Some(256) }),
            ..Default::default()
        })
        .param(ParamSchema {
            name: "tags_json".to_string(), name_zh: Some("额外 Tags".to_string()), name_en: Some("Extra Tags".to_string()),
            description: Some("Extra InfluxDB-format tags as JSON object, e.g. {\"location\":\"room1\"}".to_string()),
            description_zh: Some("额外 tags，JSON 对象，例如 {\"location\":\"room1\"}".to_string()),
            description_en: Some("Extra InfluxDB-format tags as JSON object".to_string()),
            attribute: ParamAttribute::Optional, ty: ParamType::String,
            default: None, valid: Some(ParamValid { min: None, max: None, regex: None, length: Some(2048) }),
            ..Default::default()
        })
        .param(ParamSchema {
            name: "table_name_key".to_string(), name_zh: Some("子表名 Tag 键".to_string()), name_en: Some("Table Name Tag Key".to_string()),
            description: Some("Tag key whose value determines the TDengine child table name. Default 'gateway_table'. Set empty to use taosAdapter default naming.".to_string()),
            description_zh: Some("指定作为 TDengine 子表名的 tag 键名，默认为 'gateway_table'（注入值为 <node>_<group>）。置空则使用 taosAdapter 默认命名。".to_string()),
            description_en: Some("Tag key for child table name in TDengine. Default 'gateway_table'. Set empty to use taosAdapter default.".to_string()),
            attribute: ParamAttribute::Optional, ty: ParamType::String,
            default: Some(serde_json::json!(DEFAULT_TABLE_NAME_KEY)),
            valid: Some(ParamValid { min: None, max: None, regex: None, length: Some(128) }),
            ..Default::default()
        })
        .param(ParamSchema {
            name: "auto_create_db".to_string(), name_zh: Some("自动建库".to_string()), name_en: Some("Auto Create DB".to_string()),
            description: Some("Automatically create database on open (CREATE DATABASE IF NOT EXISTS)".to_string()),
            description_zh: Some("open 时自动建库（CREATE DATABASE IF NOT EXISTS）".to_string()),
            description_en: Some("Automatically create database on open".to_string()),
            attribute: ParamAttribute::Optional, ty: ParamType::Bool,
            default: Some(serde_json::json!(true)), valid: None,
            ..Default::default()
        })
        .param(ParamSchema {
            name: "batch_max_lines".to_string(), name_zh: Some("批量行数上限".to_string()), name_en: Some("Batch Max Lines".to_string()),
            description: Some("Max lines per write request (0 = send immediately)".to_string()),
            description_zh: Some("每次写入请求的最大行数（0 = 逐条发送）".to_string()),
            description_en: Some("Max lines per write request".to_string()),
            attribute: ParamAttribute::Optional, ty: ParamType::Int,
            default: Some(serde_json::json!(DEFAULT_BATCH_MAX_LINES as i64)),
            valid: Some(ParamValid { min: Some(0), max: Some(100_000), regex: None, length: None }),
            ..Default::default()
        })
        .param(ParamSchema {
            name: "batch_interval_ms".to_string(), name_zh: Some("批量间隔（MS）".to_string()), name_en: Some("Batch Interval (MS)".to_string()),
            description: Some("Flush interval for partial batches in milliseconds".to_string()),
            description_zh: Some("批量Flush间隔，单位毫秒".to_string()),
            description_en: Some("Flush interval for partial batches in milliseconds".to_string()),
            attribute: ParamAttribute::Optional, ty: ParamType::Int,
            default: Some(serde_json::json!(DEFAULT_BATCH_INTERVAL_MS as i64)),
            valid: Some(ParamValid { min: Some(0), max: Some(600_000), regex: None, length: None }),
            ..Default::default()
        })
        .param(ParamSchema {
            name: "insecure_skip_verify".to_string(), name_zh: Some("跳过证书验证".to_string()), name_en: Some("Skip Cert Verify".to_string()),
            description: Some("Skip TLS certificate verification".to_string()),
            description_zh: Some("跳过 TLS 证书验证".to_string()),
            description_en: Some("Skip TLS certificate verification".to_string()),
            attribute: ParamAttribute::Optional, ty: ParamType::Bool,
            default: Some(serde_json::json!(false)), valid: None,
            ..Default::default()
        })
        .param(ParamSchema {
            name: "ca_file".to_string(), name_zh: Some("CA 证书".to_string()), name_en: Some("CA File".to_string()),
            description: Some("Custom CA certificate file (PEM)".to_string()),
            description_zh: Some("自定义 CA 证书文件（PEM 格式）".to_string()),
            description_en: Some("Custom CA certificate file (PEM)".to_string()),
            attribute: ParamAttribute::Optional, ty: ParamType::File,
            default: None, valid: None,
            ..Default::default()
        })
        .param(ParamSchema {
            name: "cache_memory_size".to_string(), name_zh: Some("缓存内存大小".to_string()), name_en: Some("Cache Memory Size".to_string()),
            description: Some("Max in-memory cache size when connection fails.".to_string()),
            description_zh: Some("连接异常时暂存的最大条数。".to_string()),
            description_en: Some("Max in-memory cache size when connection fails.".to_string()),
            attribute: ParamAttribute::Optional, ty: ParamType::Int,
            default: Some(serde_json::json!(gateway_plugin_common::DEFAULT_CACHE_MEMORY_SIZE)), valid: None,
            ..Default::default()
        })
        .param(ParamSchema {
            name: "cache_persist".to_string(), name_zh: Some("离线队列落盘".to_string()), name_en: Some("Persist Offline Queue".to_string()),
            description: Some("Persist the offline queue to disk.".to_string()),
            description_zh: Some("离线队列是否落盘。".to_string()),
            description_en: Some("Persist offline queue to disk.".to_string()),
            attribute: ParamAttribute::Optional, ty: ParamType::Bool,
            default: Some(serde_json::json!(true)), valid: None,
            ..Default::default()
        })
        .param(ParamSchema {
            name: "cache_dir".to_string(), name_zh: Some("离线队列目录".to_string()), name_en: Some("Offline Queue Directory".to_string()),
            description: Some("Directory for per-node offline queue files.".to_string()),
            description_zh: Some(format!("每个节点一个文件，默认 {}", DEFAULT_CACHE_DIR_TDENGINE).to_string()),
            description_en: Some("Directory for per-node offline queue files.".to_string()),
            attribute: ParamAttribute::Optional, ty: ParamType::String,
            default: Some(serde_json::json!(DEFAULT_CACHE_DIR_TDENGINE)), valid: None,
            ..Default::default()
        })
        .param(ParamSchema {
            name: "cache_sync_interval_ms".to_string(), name_zh: Some("缓存消息重传间隔（MS）".to_string()), name_en: Some("Cache Sync Interval (MS)".to_string()),
            description: Some("Interval for replaying cached messages after reconnect.".to_string()),
            description_zh: Some("恢复连接后补发缓存消息的时间间隔。".to_string()),
            description_en: Some("Interval for replaying cached messages.".to_string()),
            attribute: ParamAttribute::Optional, ty: ParamType::Int,
            default: Some(serde_json::json!(gateway_plugin_common::DEFAULT_CACHE_SYNC_INTERVAL_MS as i64)),
            valid: Some(ParamValid { min: Some(10), max: Some(120_000), regex: None, length: None }),
            ..Default::default()
        })
}
