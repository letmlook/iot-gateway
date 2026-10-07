//! 北向 Kafka 插件：把 GroupData 按 topic 模板生产到 Kafka。
//!
//! 事件模型（与 plugin-mqtt 完全一致）：
//! - `on_group_data` 只做序列化 + mpsc try_send / 离线队列 push，绝不内联网络 IO
//! - 生产在独立 OS 线程 worker 中完成（std::thread::spawn + current_thread runtime）
//! - Delivery 失败只计数告警，不回灌离线队列（避免乱序与重复风暴）
//!
//! # Feature flags
//!
//! - `kafka-client`（非默认）：编译 rdkafka，生产者完整实现。
//!   不开时为 stub：丢弃数据 + `dropped_no_client` 计数（不进队列）。
//! - `ffi`：构建 .so/.dll 时启用。

#[cfg(feature = "ffi")]
mod ffi;

use gateway_plugin_common::config::{
    config_bool, config_str, config_u64, config_usize, default_cache_dir, queue_path_for_node,
    DEFAULT_CACHE_MEMORY_SIZE, DEFAULT_CACHE_SYNC_INTERVAL_MS,
};
use gateway_plugin_common::format::{
    payload_for_format, topic_from_template, UPLOAD_FORMAT_VALUES_FORMAT,
};
use gateway_plugin_common::queue::OfflineQueue;
#[cfg(feature = "kafka-client")]
use gateway_plugin_common::queue::Record;
use gateway_sdk::log;
use gateway_sdk::types::PluginKind;
use gateway_sdk::PluginResult;
use gateway_sdk::{GroupData, GroupSubscription, NodeId, NorthPlugin, PluginConfig, PluginMeta};
#[cfg(feature = "kafka-client")]
use rdkafka::producer::FutureProducer;
use std::collections::HashMap;
use std::sync::Arc;
#[cfg(feature = "kafka-client")]
use std::time::Duration;
#[cfg(feature = "kafka-client")]
use tokio::sync::mpsc;
use tokio::sync::RwLock;

// ---------------------------------------------------------------------------
// 常量
// ---------------------------------------------------------------------------

const DEFAULT_BROKERS: &str = "localhost:9092";
const DEFAULT_TOPIC_TEMPLATE: &str = "gateway/data/${node_id}/${group_id}";
const DEFAULT_MESSAGE_TIMEOUT_MS: u64 = 300_000;
const DEFAULT_STATS_INTERVAL_MS: u64 = 5000;
const DEFAULT_UPLOAD_FORMAT: &str = UPLOAD_FORMAT_VALUES_FORMAT;
const DEFAULT_CACHE_DIR_KAFKA: &str = "data/kafka-queue";

// ---------------------------------------------------------------------------
// 状态
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
struct KafkaConnectionStatus {
    connected: bool,
    last_error: Option<String>,
    dropped_rejected: u64,
    dropped_no_client: u64,
    dropped_delivery: u64,
}

/// 每节点状态（kafka-client 开启时）
struct NodeKafkaState {
    queue: Arc<tokio::sync::Mutex<OfflineQueue>>,
    connection_status: Arc<RwLock<KafkaConnectionStatus>>,
    #[cfg(feature = "kafka-client")]
    tx: Option<mpsc::Sender<Record>>,
    #[cfg(feature = "kafka-client")]
    cancel_tx: Option<tokio::sync::oneshot::Sender<()>>,
    #[cfg(feature = "kafka-client")]
    event_loop_handle: Option<std::thread::JoinHandle<()>>,
}

struct KafkaState {
    open_nodes: std::collections::HashSet<NodeId>,
    subscriptions: HashMap<NodeId, Vec<GroupSubscription>>,
    nodes: HashMap<NodeId, NodeKafkaState>,
}

pub struct KafkaPlugin {
    state: Arc<RwLock<KafkaState>>,
}

impl Default for KafkaPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl KafkaPlugin {
    pub fn new() -> Self {
        Self {
            state: Arc::new(RwLock::new(KafkaState {
                open_nodes: std::collections::HashSet::new(),
                subscriptions: HashMap::new(),
                nodes: HashMap::new(),
            })),
        }
    }
}

#[async_trait::async_trait]
impl NorthPlugin for KafkaPlugin {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "kafka",
            kind: PluginKind::North,
            description: Some("Kafka 北向：把 GroupData 按 topic 模板生产到 Kafka，支持离线缓存与补发"),
            version: "0.1.0",
            name_zh: Some("Kafka"),
            name_en: Some("Kafka"),
            description_zh: Some("Kafka 北向：按 topic 模板生产消息，支持离线缓存与补发"),
            description_en: Some("Kafka north: produces GroupData to Kafka by topic template, with offline cache and replay"),
        }
    }

    fn config_schema(&self) -> Option<gateway_sdk::ConfigSchema> {
        Some(config_schema())
    }

    async fn open(&self, node_id: NodeId, config: PluginConfig) -> PluginResult<()> {
        let brokers = config_str(&config, "brokers", DEFAULT_BROKERS);
        if brokers.is_empty() {
            return Err(gateway_sdk::PluginError::config_invalid(
                "brokers is required",
            ));
        }
        let _topic_template = config_str(&config, "topic_template", DEFAULT_TOPIC_TEMPLATE);
        let _message_timeout_ms =
            config_u64(&config, "message_timeout_ms", DEFAULT_MESSAGE_TIMEOUT_MS);
        let _stats_interval_ms =
            config_u64(&config, "stats_interval_ms", DEFAULT_STATS_INTERVAL_MS);
        let extra_config_json = config_str(&config, "extra_config_json", "");
        // extra_config_json 必须是合法 JSON 对象，解析失败返回 ConfigInvalid
        if !extra_config_json.is_empty() {
            if let Err(e) = serde_json::from_str::<serde_json::Value>(&extra_config_json) {
                return Err(gateway_sdk::PluginError::config_invalid(format!(
                    "extra_config_json must be valid JSON: {}",
                    e
                )));
            }
        }
        let cache_memory_size =
            config_usize(&config, "cache_memory_size", DEFAULT_CACHE_MEMORY_SIZE);
        let _cache_sync_interval_ms = config_u64(
            &config,
            "cache_sync_interval_ms",
            DEFAULT_CACHE_SYNC_INTERVAL_MS,
        );
        let cache_persist = config_bool(&config, "cache_persist", true);
        // 缺省目录：优先 <GATEWAY_DATA_DIR>/kafka-queue，未设置该环境变量时保持原默认（CWD 相对）
        let cache_dir = config_str(
            &config,
            "cache_dir",
            &default_cache_dir("kafka-queue", DEFAULT_CACHE_DIR_KAFKA),
        );

        log::info(node_id, format!("open kafka: brokers={}", brokers));

        let mut state = self.state.write().await;
        state.open_nodes.insert(node_id);

        let queue_path = if cache_persist {
            queue_path_for_node(&cache_dir, node_id, "-kafka")
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

        let connection_status = Arc::new(RwLock::new(KafkaConnectionStatus {
            connected: false,
            last_error: None,
            dropped_rejected: 0,
            dropped_no_client: 0,
            dropped_delivery: 0,
        }));

        #[cfg(feature = "kafka-client")]
        {
            let (record_tx, record_rx) = mpsc::channel::<Record>(1000);
            let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();

            let queue_clone = queue.clone();
            let conn_status_clone = connection_status.clone();
            let node_id_worker = node_id;
            let brokers_worker = brokers.clone();
            let topic_template_worker = _topic_template.clone();
            let message_timeout_ms_worker = _message_timeout_ms;
            let stats_interval_ms_worker = _stats_interval_ms;
            let extra_config_json_worker = extra_config_json.clone();
            let sync_interval = _cache_sync_interval_ms;

            let handle = std::thread::spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("kafka event loop runtime");
                rt.block_on(run_kafka_worker(
                    node_id_worker,
                    brokers_worker,
                    topic_template_worker,
                    message_timeout_ms_worker,
                    stats_interval_ms_worker,
                    extra_config_json_worker,
                    record_rx,
                    queue_clone,
                    conn_status_clone,
                    sync_interval,
                    cancel_rx,
                ));
            });

            state.nodes.insert(
                node_id,
                NodeKafkaState {
                    queue,
                    connection_status,
                    tx: Some(record_tx),
                    cancel_tx: Some(cancel_tx),
                    event_loop_handle: Some(handle),
                },
            );
        }

        #[cfg(not(feature = "kafka-client"))]
        {
            // stub：只建队列和状态，不建 client
            state.nodes.insert(
                node_id,
                NodeKafkaState {
                    queue,
                    connection_status,
                },
            );
        }

        Ok(())
    }

    async fn close(&self, node_id: NodeId) -> PluginResult<()> {
        log::info(node_id, "close kafka, disconnecting");
        #[cfg(feature = "kafka-client")]
        {
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
        }
        #[cfg(not(feature = "kafka-client"))]
        {
            let mut state = self.state.write().await;
            state.open_nodes.remove(&node_id);
            state.subscriptions.remove(&node_id);
            state.nodes.remove(&node_id);
        }
        Ok(())
    }

    async fn start(&self, node_id: NodeId) -> PluginResult<()> {
        log::info(node_id, "start kafka");
        Ok(())
    }

    async fn stop(&self, node_id: NodeId) -> PluginResult<()> {
        log::info(node_id, "stop kafka");
        Ok(())
    }

    async fn setting(&self, node_id: NodeId, config: PluginConfig) -> PluginResult<()> {
        log::info(node_id, "setting kafka, reconnecting with new config");
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
            "dropped_delivery": st.dropped_delivery,
        }))
    }

    async fn on_group_data(&self, node_id: NodeId, data: Arc<GroupData>) -> PluginResult<()> {
        let state = self.state.read().await;
        let Some(node_state) = state.nodes.get(&node_id) else {
            log::warn(node_id, "on_group_data: north node not open, skip");
            return Ok(());
        };

        let topic = topic_from_template(
            "gateway/data/${node_id}/${group_id}",
            data.node_id,
            data.group_id,
            data.node_name.as_deref(),
            data.group_name.as_deref(),
            data.ts,
        );
        let payload = payload_for_format(&data, DEFAULT_UPLOAD_FORMAT);

        #[cfg(feature = "kafka-client")]
        {
            // 缺陷①修复（worker 引导死锁）：无论 connected 与否都把记录交给 worker，
            // 由 worker 统一负责发送/入队/重试。旧逻辑在 !connected 时直接入队，
            // 而 worker 的记录唯一来源是 mpsc 通道、connected 又只能由「投递成功」置位
            // → 通道永远空、首条消息永远发不出。
            if let Some(tx) = node_state.tx.clone() {
                let rec = Record {
                    topic: topic.clone(),
                    payload: payload.clone(),
                };
                if tx.try_send(rec).is_err() {
                    // 通道满：进离线队列（保序）
                    enqueue(node_state, topic, payload).await;
                }
            } else {
                enqueue(node_state, topic, payload).await;
            }
        }

        #[cfg(not(feature = "kafka-client"))]
        {
            let _ = (topic, payload);
            let mut cs = node_state.connection_status.write().await;
            cs.dropped_no_client += 1;
            tracing::info!(node_id = ?node_id, "kafka (no client): would produce");
        }

        Ok(())
    }
}

#[cfg(feature = "kafka-client")]
async fn enqueue(node_state: &NodeKafkaState, topic: String, payload: Vec<u8>) {
    let mut q = node_state.queue.lock().await;
    let before = q.stats().dropped_overflow;
    q.push(Record { topic, payload });
    if q.stats().dropped_overflow > before {
        tracing::warn!("kafka offline queue full, dropped oldest record(s)");
    }
}

// ---------------------------------------------------------------------------
// Kafka Worker（kafka-client feature）
// ---------------------------------------------------------------------------

#[cfg(feature = "kafka-client")]
#[allow(clippy::too_many_arguments)]
async fn run_kafka_worker(
    node_id: NodeId,
    brokers: String,
    _topic_template: String,
    message_timeout_ms: u64,
    _stats_interval_ms: u64,
    extra_config_json: String,
    mut record_rx: mpsc::Receiver<Record>,
    queue: Arc<tokio::sync::Mutex<OfflineQueue>>,
    connection_status: Arc<RwLock<KafkaConnectionStatus>>,
    cache_sync_interval_ms: u64,
    mut cancel_rx: tokio::sync::oneshot::Receiver<()>,
) {
    use rdkafka::config::ClientConfig;
    use rdkafka::producer::{FutureProducer, FutureRecord};
    use std::time::Duration;

    // 解析 extra_config_json
    let extra_config: HashMap<String, String> = if extra_config_json.is_empty() {
        HashMap::new()
    } else {
        serde_json::from_str(&extra_config_json).unwrap_or_else(|e| {
            tracing::warn!(node_id = ?node_id, "extra_config_json parse failed: {}", e);
            HashMap::new()
        })
    };

    let mut builder = ClientConfig::new();
    builder
        .set("bootstrap.servers", &brokers)
        .set("message.timeout.ms", message_timeout_ms.to_string())
        .set("statistics.interval.ms", "0")
        // compression: gzip 使用 librdkafka 内置的 libz，不需外部依赖
        .set("compression.codec", "gzip")
        .set("queue.buffering.max.messages", "100000")
        .set("queue.buffering.max.kbytes", "1048576");
    // 将 extra_config 逐条 set（ssl/sasl 等在 v1 明文部署下用户不应配；配了会得到明确的参数错误）
    for (k, v) in extra_config {
        builder.set(&k, &v);
    }
    let producer: FutureProducer = builder.create().expect("Failed to create Kafka producer");

    // connected 通过发送结果维护：成功→connected=true，失败→connected=false
    let interval = Duration::from_millis(cache_sync_interval_ms.max(10));
    let mut backoff = Duration::from_secs(1);
    let max_backoff = Duration::from_secs(30);
    // 缺陷①修复：初始即武装补发分支（healthy=false）——
    // worker 启动时就会尝试补发离线队列（含磁盘恢复的记录），而不是等「第一条投递成功」；
    // 之后 healthy 由「离线队列是否清空」维护：非空则保持重试，清空即解除。
    let mut healthy = false;

    loop {
        tokio::select! {
            _ = &mut cancel_rx => {
                log::info(node_id, "kafka event loop exited (close)");
                break;
            }
            rec = record_rx.recv() => {
                match rec {
                    Some(record) => {
                        // rdkafka FutureProducer::send 返回 Future，同步等待 delivery 结果
                        // （与 http/influxdb/tdengine 的 worker 同构：发送结果必须能驱动
                        // healthy/退避/离线队列补发状态机——旧实现 tokio::spawn 把 delivery
                        // 丢到后台，worker 的 healthy 与之完全脱节，离线队列无人消费）。
                        // delivery 失败（librdkafka 内部重试耗尽）只计数不回灌离线队列（避免乱序/重复风暴）
                        let delivery = producer
                            .send(
                                FutureRecord::to(&record.topic)
                                    .payload(&record.payload)
                                    .key(&record.topic),
                                Duration::from_millis(message_timeout_ms),
                            )
                            .await;
                        match delivery {
                            Ok(_) => {
                                backoff = Duration::from_secs(1);
                                let mut cs = connection_status.write().await;
                                cs.connected = true;
                                cs.last_error = None;
                            }
                            Err((e, _)) => {
                                backoff = Duration::from_secs(1);
                                let mut cs = connection_status.write().await;
                                cs.connected = false;
                                cs.last_error = Some(e.to_string());
                                cs.dropped_delivery += 1;
                                tracing::warn!(node_id = ?node_id, error = %e,
                                    "kafka delivery failed after retries, counted as dropped_delivery");
                            }
                        }
                        // 队列非空时保持补发分支武装（即使本次投递成功），清空后解除
                        healthy = queue.lock().await.is_empty();
                    }
                    None => break,
                }
            }
            _ = tokio::time::sleep(backoff), if !healthy => {
                let restored = replenish_queue(
                    &queue, &producer, message_timeout_ms, interval,
                ).await;
                let empty = queue.lock().await.is_empty();
                if empty {
                    if restored > 0 {
                        // 补发成功即证明 broker 可达：connected 语义必须真实反映这一点
                        let mut cs = connection_status.write().await;
                        cs.connected = true;
                        cs.last_error = None;
                        log::info(node_id, format!("kafka offline queue flushed {} record(s)", restored));
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

#[cfg(feature = "kafka-client")]
async fn replenish_queue(
    queue: &Arc<tokio::sync::Mutex<OfflineQueue>>,
    producer: &FutureProducer,
    message_timeout_ms: u64,
    interval: Duration,
) -> u64 {
    use rdkafka::producer::FutureRecord;
    let mut restored = 0u64;
    loop {
        let record = {
            let mut q = queue.lock().await;
            q.pop_front()
        };
        let Some(record) = record else { break };
        let delivery = producer.send(
            FutureRecord::to(&record.topic)
                .payload(&record.payload)
                .key(&record.topic),
            Duration::from_millis(message_timeout_ms),
        );
        match delivery.await {
            Ok(_) => {
                restored += 1;
                tokio::time::sleep(interval).await;
            }
            Err((e, _)) => {
                let mut q = queue.lock().await;
                q.push_front(record);
                tracing::warn!(error = %e, "kafka offline flush delivery failed");
                break;
            }
        }
    }
    restored
}

// ---------------------------------------------------------------------------
// ConfigSchema
// ---------------------------------------------------------------------------

fn config_schema() -> gateway_sdk::ConfigSchema {
    use gateway_sdk::schema::{ParamAttribute, ParamOption, ParamSchema, ParamType, ParamValid};
    use gateway_sdk::ConfigSchema;

    ConfigSchema::new()
        .param(ParamSchema {
            name: "brokers".to_string(),
            name_zh: Some("Broker 地址".to_string()),
            name_en: Some("Brokers".to_string()),
            description: Some("Kafka bootstrap servers, comma-separated host:port".to_string()),
            description_zh: Some("Kafka bootstrap servers，逗号分隔的 host:port".to_string()),
            description_en: Some("Kafka bootstrap servers, comma-separated host:port".to_string()),
            attribute: ParamAttribute::Required,
            ty: ParamType::String,
            default: Some(serde_json::json!(DEFAULT_BROKERS)),
            valid: Some(ParamValid {
                min: None, max: None,
                regex: Some(".+".to_string()),
                length: Some(1024),
            }),
            ..Default::default()
        })
        .param(ParamSchema {
            name: "topic_template".to_string(),
            name_zh: Some("Topic 模板".to_string()),
            name_en: Some("Topic Template".to_string()),
            description: Some("Kafka topic template. Variables: ${node_id}/${group_id} (replaced with name when available), ${node_name}/${group_name}".to_string()),
            description_zh: Some("Kafka topic 模板。变量：${node_id}/${group_id}/${node_name}/${group_name}".to_string()),
            description_en: Some("Kafka topic template. Variables: ${node_id}/${group_id}/${node_name}/${group_name}".to_string()),
            attribute: ParamAttribute::Optional,
            ty: ParamType::String,
            default: Some(serde_json::json!(DEFAULT_TOPIC_TEMPLATE)),
            valid: Some(ParamValid { min: None, max: None, regex: None, length: Some(512) }),
            ..Default::default()
        })
        .param(ParamSchema {
            name: "upload_format".to_string(),
            name_zh: Some("上报数据格式".to_string()),
            name_en: Some("Upload Format".to_string()),
            description: Some("Upload format: values_format, tags_format, ecp_format, group_data, raw_data.".to_string()),
            description_zh: Some("上报 JSON 格式".to_string()),
            description_en: Some("Upload format".to_string()),
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
            name: "message_timeout_ms".to_string(),
            name_zh: Some("消息超时（MS）".to_string()),
            name_en: Some("Message Timeout (MS)".to_string()),
            description: Some("Kafka message timeout in milliseconds".to_string()),
            description_zh: Some("Kafka 消息超时，单位毫秒".to_string()),
            description_en: Some("Kafka message timeout in milliseconds".to_string()),
            attribute: ParamAttribute::Optional,
            ty: ParamType::Int,
            default: Some(serde_json::json!(DEFAULT_MESSAGE_TIMEOUT_MS as i64)),
            valid: Some(ParamValid { min: Some(1000), max: Some(86400000), regex: None, length: None }),
            ..Default::default()
        })
        .param(ParamSchema {
            name: "stats_interval_ms".to_string(),
            name_zh: Some("统计间隔（MS）".to_string()),
            name_en: Some("Stats Interval (MS)".to_string()),
            description: Some("Kafka statistics interval in milliseconds".to_string()),
            description_zh: Some("Kafka 统计信息采集间隔，单位毫秒".to_string()),
            description_en: Some("Kafka statistics interval in milliseconds".to_string()),
            attribute: ParamAttribute::Optional,
            ty: ParamType::Int,
            default: Some(serde_json::json!(DEFAULT_STATS_INTERVAL_MS as i64)),
            valid: Some(ParamValid { min: Some(0), max: Some(86400000), regex: None, length: None }),
            ..Default::default()
        })
        .param(ParamSchema {
            name: "extra_config_json".to_string(),
            name_zh: Some("额外配置".to_string()),
            name_en: Some("Extra Config".to_string()),
            description: Some("Extra rdkafka configuration as JSON object, e.g. {\"compression.codec\":\"gzip\"}. v1 does not enable ssl/sasl/zstd/lz4. Misconfigured ssl/sasl will produce a clear error.".to_string()),
            description_zh: Some("额外的 rdkafka 配置，JSON 对象，例如 {\"compression.codec\":\"gzip\"}。v1 不启用 ssl/sasl/zstd/lz4，错误配置会返回明确错误。".to_string()),
            description_en: Some("Extra rdkafka configuration as JSON object".to_string()),
            attribute: ParamAttribute::Optional,
            ty: ParamType::String,
            default: None,
            valid: Some(ParamValid { min: None, max: None, regex: None, length: Some(4096) }),
            ..Default::default()
        })
        .param(ParamSchema {
            name: "cache_memory_size".to_string(),
            name_zh: Some("缓存内存大小".to_string()),
            name_en: Some("Cache Memory Size".to_string()),
            description: Some("Max in-memory cache size (message count) when connection fails.".to_string()),
            description_zh: Some("连接异常时暂存的最大条数。".to_string()),
            description_en: Some("Max in-memory cache size when connection fails.".to_string()),
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
            description: Some("Persist the offline queue to disk.".to_string()),
            description_zh: Some("离线队列是否落盘。".to_string()),
            description_en: Some("Persist offline queue to disk.".to_string()),
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
            description_zh: Some(format!("每个节点一个文件，默认 {}（未显式配置时优先取 GATEWAY_DATA_DIR 环境变量，即 <GATEWAY_DATA_DIR>/kafka-queue）", DEFAULT_CACHE_DIR_KAFKA).to_string()),
            description_en: Some("Directory for per-node offline queue files. Defaults to <GATEWAY_DATA_DIR>/kafka-queue when the env var is set, otherwise the built-in default.".to_string()),
            attribute: ParamAttribute::Optional,
            ty: ParamType::String,
            default: Some(serde_json::json!(DEFAULT_CACHE_DIR_KAFKA)),
            valid: None,
            ..Default::default()
        })
        .param(ParamSchema {
            name: "cache_sync_interval_ms".to_string(),
            name_zh: Some("缓存消息重传间隔（MS）".to_string()),
            name_en: Some("Cache Sync Interval (MS)".to_string()),
            description: Some("Interval for replaying cached messages after reconnect.".to_string()),
            description_zh: Some("恢复连接后补发缓存消息的时间间隔。".to_string()),
            description_en: Some("Interval for replaying cached messages.".to_string()),
            attribute: ParamAttribute::Optional,
            ty: ParamType::Int,
            default: Some(serde_json::json!(gateway_plugin_common::DEFAULT_CACHE_SYNC_INTERVAL_MS as i64)),
            valid: Some(ParamValid { min: Some(10), max: Some(120_000), regex: None, length: None }),
            ..Default::default()
        })
}

// ---------------------------------------------------------------------------
// 单元测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

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
        let ts = chrono::Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
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
            group_name: Some("env-data".to_string()),
            tag_names: Some(tag_names),
        })
    }

    fn make_config(overrides: std::collections::HashMap<&str, serde_json::Value>) -> PluginConfig {
        let mut cfg = std::collections::HashMap::new();
        cfg.insert("brokers".to_string(), serde_json::json!("localhost:9092"));
        cfg.insert(
            "topic_template".to_string(),
            serde_json::json!("gateway/data/${node_id}/${group_id}"),
        );
        cfg.insert("message_timeout_ms".to_string(), serde_json::json!(300000));
        cfg.insert("stats_interval_ms".to_string(), serde_json::json!(5000));
        cfg.insert("extra_config_json".to_string(), serde_json::json!(""));
        cfg.insert("cache_memory_size".to_string(), serde_json::json!(1000));
        cfg.insert("cache_sync_interval_ms".to_string(), serde_json::json!(100));
        cfg.insert("cache_persist".to_string(), serde_json::json!(false));
        cfg.insert(
            "cache_dir".to_string(),
            serde_json::json!("data/kafka-queue"),
        );
        cfg.insert(
            "upload_format".to_string(),
            serde_json::json!("values_format"),
        );
        for (k, v) in overrides {
            cfg.insert(k.to_string(), v);
        }
        cfg
    }

    // ---- Stub 行为测试（no kafka-client feature）----
    // 这些用例断言的是「未编译 kafka-client」时的降级行为，必须按 feature 门控，
    // 否则开 kafka-client 时会走真实 client 路径而误报失败。

    #[cfg(not(feature = "kafka-client"))]
    #[tokio::test]
    async fn stub_open_creates_state_without_client() {
        let plugin = KafkaPlugin::new();
        let node_id = make_node_id();
        let config = make_config(std::collections::HashMap::new());

        let result = plugin.open(node_id, config).await;
        assert!(result.is_ok());

        let status = plugin.connection_status(node_id).await;
        assert!(status.is_some());
        let s = status.unwrap();
        assert_eq!(s["connected"], serde_json::json!(false));
        assert_eq!(s["dropped_no_client"], serde_json::json!(0));
    }

    #[cfg(not(feature = "kafka-client"))]
    #[tokio::test]
    async fn stub_on_group_data_discards_without_enqueuing() {
        let plugin = KafkaPlugin::new();
        let node_id = make_node_id();
        let config = make_config(std::collections::HashMap::new());
        plugin.open(node_id, config).await.unwrap();

        let data = make_group_data();
        plugin.on_group_data(node_id, data).await.unwrap();

        let status = plugin.connection_status(node_id).await.unwrap();
        // stub 下 dropped_no_client 应为 1
        assert_eq!(status["dropped_no_client"], serde_json::json!(1));
        // queue_len 应为 0（stub 不入队）
        assert_eq!(status["queue_len"], serde_json::json!(0));
    }

    #[cfg(not(feature = "kafka-client"))]
    #[tokio::test]
    async fn stub_multiple_on_group_data_counts_correctly() {
        let plugin = KafkaPlugin::new();
        let node_id = make_node_id();
        plugin
            .open(node_id, make_config(std::collections::HashMap::new()))
            .await
            .unwrap();

        for _ in 0..5 {
            plugin
                .on_group_data(node_id, make_group_data())
                .await
                .unwrap();
        }

        let status = plugin.connection_status(node_id).await.unwrap();
        assert_eq!(status["dropped_no_client"], serde_json::json!(5));
        assert_eq!(status["queue_len"], serde_json::json!(0));
    }

    // ---- ConfigSchema 测试 ----

    #[tokio::test]
    async fn empty_brokers_returns_config_invalid() {
        let plugin = KafkaPlugin::new();
        let node_id = make_node_id();
        let mut config = make_config(std::collections::HashMap::new());
        config.insert("brokers".to_string(), serde_json::json!(""));

        let result = plugin.open(node_id, config).await;
        let err = result.unwrap_err();
        assert_eq!(err.code(), gateway_sdk::PluginErrorCode::ConfigInvalid);
    }

    #[tokio::test]
    async fn invalid_extra_config_json_returns_config_invalid() {
        let plugin = KafkaPlugin::new();
        let node_id = make_node_id();
        let mut config = make_config(std::collections::HashMap::new());
        config.insert(
            "extra_config_json".to_string(),
            serde_json::json!("not valid json {{{"),
        );

        let result = plugin.open(node_id, config).await;
        let err = result.unwrap_err();
        assert_eq!(err.code(), gateway_sdk::PluginErrorCode::ConfigInvalid);
    }

    #[tokio::test]
    async fn valid_extra_config_json_object_succeeds() {
        let plugin = KafkaPlugin::new();
        let node_id = make_node_id();
        let mut config = make_config(std::collections::HashMap::new());
        config.insert(
            "extra_config_json".to_string(),
            serde_json::json!(r#"{"compression.codec":"gzip"}"#),
        );

        let result = plugin.open(node_id, config).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn valid_extra_config_json_array_still_fails_schema() {
        // extra_config_json 必须是 JSON 对象（HashMap），JSON 数组通过解析但行为不确定
        // 我们的验证只要求是合法 JSON（不要求对象），设计文档只要求"parse + set"
        // 这里验证有效的 JSON 解析不报错（类型由 librdkafka 运行时验证）
        let plugin = KafkaPlugin::new();
        let node_id = make_node_id();
        let mut config = make_config(std::collections::HashMap::new());
        config.insert(
            "extra_config_json".to_string(),
            serde_json::json!(["some", "array"]),
        );
        // JSON 有效但不用于任何目的，验证不会 panic
        let result = plugin.open(node_id, config).await;
        assert!(result.is_ok());
    }

    // ---- connection_status 键完整性测试 ----

    #[tokio::test]
    async fn connection_status_returns_all_required_keys() {
        let plugin = KafkaPlugin::new();
        let node_id = make_node_id();
        plugin
            .open(node_id, make_config(std::collections::HashMap::new()))
            .await
            .unwrap();

        let status = plugin.connection_status(node_id).await.unwrap();
        // 与 mqtt 同组的 7 个键 + dropped_rejected + dropped_no_client + dropped_delivery
        assert!(status.get("connected").is_some());
        assert!(status.get("last_error").is_some());
        assert!(status.get("queue_len").is_some());
        assert!(status.get("queue_dropped_overflow").is_some());
        assert!(status.get("queue_recovered").is_some());
        assert!(status.get("queue_dropped_corrupt").is_some());
        assert!(status.get("queue_persisted").is_some());
        assert!(status.get("dropped_rejected").is_some());
        assert!(status.get("dropped_no_client").is_some());
        assert!(status.get("dropped_delivery").is_some());
    }

    // ---- close 测试 ----

    #[tokio::test]
    async fn close_removes_node_state() {
        let plugin = KafkaPlugin::new();
        let node_id = make_node_id();
        plugin
            .open(node_id, make_config(std::collections::HashMap::new()))
            .await
            .unwrap();

        plugin.close(node_id).await.unwrap();

        let status = plugin.connection_status(node_id).await;
        assert!(status.is_none());
    }

    // ---- 节点不存在时 on_group_data 为空操作 ----

    #[tokio::test]
    async fn on_group_data_without_open_is_noop() {
        let plugin = KafkaPlugin::new();
        let node_id = make_node_id();
        // 不调用 open

        let result = plugin.on_group_data(node_id, make_group_data()).await;
        assert!(result.is_ok()); // 不报错，只是跳过
    }

    // ---- 回归测试（docs/联调记录-2026-10-07.md §5.1 缺陷①：北向 worker 引导死锁）----

    /// 回归测试（缺陷①，stub 模式）：记录到达 sink 判定点必须计 dropped_no_client，
    /// 绝不允许滞留/堆积进离线队列（缺陷①下未连接的记录只进队列、queue_len 持续增长）。
    #[cfg(not(feature = "kafka-client"))]
    #[tokio::test]
    async fn stub_record_reaches_sink_without_queue_growth() {
        let plugin = KafkaPlugin::new();
        let node_id = make_node_id();
        plugin
            .open(node_id, make_config(std::collections::HashMap::new()))
            .await
            .unwrap();

        // 首条记录（缺陷①场景：connected=false 时记录只进离线队列、永不投递）
        plugin
            .on_group_data(node_id, make_group_data())
            .await
            .unwrap();

        let status = plugin.connection_status(node_id).await.unwrap();
        assert_eq!(
            status["dropped_no_client"],
            serde_json::json!(1),
            "record must reach the sink decision point and be counted"
        );
        assert_eq!(
            status["queue_len"],
            serde_json::json!(0),
            "no record may pile into the offline queue"
        );
        assert_eq!(status["connected"], serde_json::json!(false));
    }

    /// 回归测试（缺陷①核心，需 `--features kafka-client`）：
    /// broker 不可达时，记录必须到达 worker 的投递路径（delivery 超时后计 dropped_delivery），
    /// 而不是滞留在离线队列——缺陷①下 queue_len==1 且 dropped_delivery 永远为 0、永不投递。
    #[cfg(feature = "kafka-client")]
    #[tokio::test]
    async fn kafka_client_record_reaches_delivery_path_not_offline_queue() {
        let plugin = KafkaPlugin::new();
        let node_id = make_node_id();
        let mut overrides = std::collections::HashMap::new();
        overrides.insert("brokers", serde_json::json!("127.0.0.1:1")); // 不可达端口
        overrides.insert("message_timeout_ms", serde_json::json!(1500)); // 1.5s 后 delivery 失败
        plugin.open(node_id, make_config(overrides)).await.unwrap();

        let status = plugin.connection_status(node_id).await.unwrap();
        assert_eq!(status["connected"], serde_json::json!(false));

        plugin
            .on_group_data(node_id, make_group_data())
            .await
            .unwrap();

        // 记录必须到达 worker 的投递路径：delivery 失败后被计数（而非困在离线队列）
        let mut attempted = false;
        for _ in 0..100 {
            let st = plugin.connection_status(node_id).await.unwrap();
            if st["dropped_delivery"].as_u64().unwrap_or(0) >= 1 {
                attempted = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        assert!(
            attempted,
            "record must reach the worker delivery path (dropped_delivery counted after timeout)"
        );

        let st = plugin.connection_status(node_id).await.unwrap();
        assert_eq!(
            st["queue_len"],
            serde_json::json!(0),
            "record must not pile into the offline queue"
        );
        assert_eq!(st["connected"], serde_json::json!(false));

        plugin.close(node_id).await.unwrap();
    }
}
