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
    config_bool, config_str, config_u64, config_usize, queue_path_for_node,
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
        let _extra_config_json = config_str(&config, "extra_config_json", "");
        let cache_memory_size =
            config_usize(&config, "cache_memory_size", DEFAULT_CACHE_MEMORY_SIZE);
        let _cache_sync_interval_ms = config_u64(
            &config,
            "cache_sync_interval_ms",
            DEFAULT_CACHE_SYNC_INTERVAL_MS,
        );
        let cache_persist = config_bool(&config, "cache_persist", true);
        let cache_dir = config_str(&config, "cache_dir", DEFAULT_CACHE_DIR_KAFKA);

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
            let extra_config_json_worker = _extra_config_json.clone();
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
            let (connected, tx_opt) = {
                let cs = node_state.connection_status.read().await;
                (cs.connected, node_state.tx.clone())
            };
            if !connected {
                enqueue(node_state, topic, payload).await;
                return Ok(());
            }
            if let Some(tx) = tx_opt {
                let rec = Record { topic, payload };
                if tx.try_send(rec).is_err() {
                    enqueue(node_state, topic, payload).await;
                }
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
async fn run_kafka_worker(
    node_id: NodeId,
    brokers: String,
    topic_template: String,
    message_timeout_ms: u64,
    stats_interval_ms: u64,
    extra_config_json: String,
    mut record_rx: mpsc::Receiver<Record>,
    queue: Arc<tokio::sync::Mutex<OfflineQueue>>,
    connection_status: Arc<RwLock<KafkaConnectionStatus>>,
    cache_sync_interval_ms: u64,
    mut cancel_rx: tokio::sync::oneshot::Receiver<()>,
) {
    use rdkafka::config::ClientConfig;
    use rdkafka::producer::{FutureProducer, FutureRecord};
    use rdkafka::statistics::Statistics;
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

    let mut builder = ClientConfig::new()
        .set("bootstrap.servers", &brokers)
        .set("message.timeout.ms", &message_timeout_ms.to_string())
        .set("statistics.interval.ms", &stats_interval_ms.to_string())
        // compression: gzip 使用 librdkafka 内置的 libz，不需外部依赖
        .set("compression.codec", "gzip")
        .set("queue.buffering.max.messages", "100000")
        .set("queue.buffering.max.kbytes", "1048576");
    // 将 extra_config 逐条 set（ssl/sasl 等在 v1 明文部署下用户不应配；配了会得到明确的参数错误）
    for (k, v) in extra_config {
        builder = builder.set(&k, &v);
    }
    let producer: FutureProducer = builder.create().expect("Failed to create Kafka producer");

    // stats 回调：解析 broker state，connected = 任一 broker state >= 3 (connected/up)
    let conn_status_clone = connection_status.clone();
    producer.stats({
        move |stats: Statistics| {
            // librdkafka broker state: 0=init, 1=disconnected, 2=connecting, 3=connected, 4=updating
            let connected = stats.brokers.values().any(|b| b.state >= 3);
            if let Some(cs) = conn_status_clone.try_write() {
                cs.connected = connected;
            }
        }
    });

    let interval = Duration::from_millis(cache_sync_interval_ms.max(10));
    let mut backoff = Duration::from_secs(1);
    let max_backoff = Duration::from_secs(30);
    let mut healthy = true;

    loop {
        tokio::select! {
            _ = &mut cancel_rx => {
                log::info(node_id, "kafka event loop exited (close)");
                break;
            }
            rec = record_rx.recv() => {
                match rec {
                    Some(record) => {
                        let topic = record.topic.clone();
                        let payload = record.payload.clone();
                        let conn_status_clone2 = connection_status.clone();
                        let queue2 = queue.clone();
                        let interval2 = interval;
                        // rdkafka FutureProducer::send 返回 Future，等待得到 delivery 结果
                        // delivery 失败（librdkafka 内部重试耗尽）只计数不回灌队列（避免乱序/重复风暴）
                        tokio::spawn(async move {
                            match producer.send(
                                FutureRecord::to(&topic).payload(&payload).key(&topic),
                                Duration::from_millis(message_timeout_ms),
                            ).await {
                                Ok(_) => {
                                    let mut cs = conn_status_clone2.write().await;
                                    cs.connected = true;
                                    cs.last_error = None;
                                }
                                Err((e, _)) => {
                                    let mut cs = conn_status_clone2.write().await;
                                    cs.connected = false;
                                    cs.last_error = Some(e.to_string());
                                    cs.dropped_delivery += 1;
                                    tracing::warn!(node_id = ?node_id, error = %e,
                                        "kafka delivery failed after retries, counted as dropped_delivery");
                                }
                            }
                        });
                    }
                    None => break,
                }
            }
            _ = tokio::time::sleep(backoff), if !healthy => {
                let restored = replenish_queue(
                    &queue, &producer, message_timeout_ms, interval,
                ).await;
                if restored > 0 {
                    healthy = true;
                    backoff = Duration::from_secs(1);
                    log::info(node_id, format!("kafka offline queue flushed {} record(s)", restored));
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
            description_zh: Some(format!("每个节点一个文件，默认 {}", DEFAULT_CACHE_DIR_KAFKA).to_string()),
            description_en: Some("Directory for per-node offline queue files.".to_string()),
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
