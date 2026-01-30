//! 北向 MQTT 插件：接收 GroupData，发布到 MQTT Broker。
//!
//! 功能：QoS 0/1/2、主题模板（变量替换）、TLS/SSL、离线内存缓存与恢复补发、
//! 上传格式（group_data / tags_format）、retain、keep_alive、cache_sync_interval。

#[cfg(feature = "ffi")]
mod ffi;

use gateway_sdk::schema::{ConfigSchema, ParamAttribute, ParamSchema, ParamType, ParamValid};
use gateway_sdk::{
    GroupData, GroupId, GroupSubscription, NodeId, NorthPlugin, PluginConfig, PluginMeta,
};
use gateway_sdk::types::{PluginKind, TagId};
use gateway_sdk::PluginResult;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::warn;

const DEFAULT_HOST: &str = "localhost";
const DEFAULT_PORT: u16 = 1883;
const DEFAULT_TOPIC_TEMPLATE: &str = "gateway/data/${node_id}/${group_id}";
const DEFAULT_CACHE_MEMORY_SIZE: usize = 1000;
const DEFAULT_KEEP_ALIVE_SECS: u64 = 30;
const DEFAULT_CACHE_SYNC_INTERVAL_MS: u64 = 100;
const DEFAULT_QOS: u8 = 1;
const UPLOAD_FORMAT_GROUP_DATA: &str = "group_data";
const UPLOAD_FORMAT_TAGS_FORMAT: &str = "tags_format";

/// 北向 MQTT 插件
pub struct MqttPlugin {
    state: Arc<RwLock<MqttState>>,
}

struct MqttState {
    open_nodes: std::collections::HashSet<NodeId>,
    subscriptions: HashMap<NodeId, Vec<GroupSubscription>>,
    /// node_id -> 该节点的 MQTT 客户端与缓存
    nodes: HashMap<NodeId, NodeMqttState>,
}

/// 发布时使用的 QoS（0/1/2）
#[derive(Clone, Copy)]
struct PublishQos(u8);

#[cfg(feature = "mqtt-client")]
struct NodeMqttState {
    client: rumqttc::AsyncClient,
    cache: Arc<RwLock<VecDeque<(String, Vec<u8>)>>>,
    cache_max: usize,
    topic_template: String,
    qos: PublishQos,
    retain: bool,
    upload_format: String,
}

#[cfg(not(feature = "mqtt-client"))]
struct NodeMqttState {
    cache: Arc<RwLock<VecDeque<(String, Vec<u8>)>>>,
    cache_max: usize,
    topic_template: String,
    qos: PublishQos,
    retain: bool,
    upload_format: String,
}

impl Default for MqttPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl MqttPlugin {
    pub fn new() -> Self {
        Self {
            state: Arc::new(RwLock::new(MqttState {
                open_nodes: std::collections::HashSet::new(),
                subscriptions: HashMap::new(),
                nodes: HashMap::new(),
            })),
        }
    }
}

fn config_str(config: &PluginConfig, key: &str, default: &str) -> String {
    config
        .get(key)
        .and_then(|v| v.as_str())
        .map(String::from)
        .unwrap_or_else(|| default.to_string())
}

fn config_u16(config: &PluginConfig, key: &str, default: u16) -> u16 {
    config
        .get(key)
        .and_then(|v| v.as_u64())
        .map(|n| n as u16)
        .unwrap_or(default)
}

fn config_usize(config: &PluginConfig, key: &str, default: usize) -> usize {
    config
        .get(key)
        .and_then(|v| v.as_u64())
        .map(|n| n as usize)
        .unwrap_or(default)
}

fn config_bool(config: &PluginConfig, key: &str, default: bool) -> bool {
    config
        .get(key)
        .and_then(|v| v.as_bool())
        .unwrap_or(default)
}

/// 从主题模板生成主题。支持变量：${node_id} ${group_id} ${timestamp}
fn topic_from_template(template: &str, node_id: NodeId, group_id: GroupId, ts: chrono::DateTime<chrono::Utc>) -> String {
    let node_id_s = node_id.0.to_string();
    let group_id_s = group_id.0.to_string();
    let timestamp_s = ts.to_rfc3339();
    template
        .replace("${node_id}", &node_id_s)
        .replace("${group_id}", &group_id_s)
        .replace("${timestamp}", &timestamp_s)
}

/// 按上传格式生成 payload：group_data = 完整 GroupData JSON；tags_format = { node_id, group_id, ts, tags: [{ tag_id, value }] }
fn payload_for_format(data: &GroupData, upload_format: &str) -> Vec<u8> {
    if upload_format == UPLOAD_FORMAT_TAGS_FORMAT {
        #[derive(serde::Serialize)]
        struct TagsFormatPayload<'a> {
            node_id: NodeId,
            group_id: GroupId,
            ts: chrono::DateTime<chrono::Utc>,
            tags: Vec<(TagId, &'a gateway_sdk::types::DataValue)>,
        }
        let payload = TagsFormatPayload {
            node_id: data.node_id,
            group_id: data.group_id,
            ts: data.ts,
            tags: data.values.iter().map(|(id, v)| (*id, v)).collect(),
        };
        serde_json::to_vec(&payload).unwrap_or_default()
    } else {
        serde_json::to_vec(data).unwrap_or_default()
    }
}

impl PublishQos {
    #[cfg(feature = "mqtt-client")]
    fn to_rumqttc(self) -> rumqttc::QoS {
        use rumqttc::QoS;
        match self.0 {
            0 => QoS::AtMostOnce,
            2 => QoS::ExactlyOnce,
            _ => QoS::AtLeastOnce,
        }
    }
}

#[async_trait::async_trait]
impl NorthPlugin for MqttPlugin {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "mqtt",
            kind: PluginKind::North,
            description: Some("MQTT 北向：QoS 0/1/2、主题模板、TLS、离线缓存与恢复补发、上传格式 group_data/tags_format"),
            version: "0.2.0",
            name_zh: Some("MQTT"),
            name_en: Some("MQTT"),
            description_zh: Some("MQTT 北向：QoS 0/1/2、主题模板、TLS、离线缓存与恢复补发、上传格式 group_data/tags_format"),
            description_en: Some("MQTT north app: QoS 0/1/2, topic template, TLS, offline cache and replay, upload format group_data/tags_format"),
        }
    }

    fn config_schema(&self) -> Option<ConfigSchema> {
        Some(
            ConfigSchema::new()
                .param(ParamSchema {
                    name: "host".to_string(),
                    description: Some("Broker 地址".to_string()),
                    name_zh: Some("Broker 地址".to_string()),
                    name_en: Some("Host".to_string()),
                    description_zh: Some("MQTT Broker IP 或域名".to_string()),
                    description_en: Some("MQTT Broker IP or hostname".to_string()),
                    attribute: ParamAttribute::Optional,
                    ty: ParamType::String,
                    default: Some(serde_json::json!(DEFAULT_HOST)),
                    valid: None,
                })
                .param(ParamSchema {
                    name: "port".to_string(),
                    description: Some("Broker 端口（TLS 通常 8883）".to_string()),
                    name_zh: Some("端口".to_string()),
                    name_en: Some("Port".to_string()),
                    description_zh: Some("Broker 端口，TLS 通常为 8883".to_string()),
                    description_en: Some("Broker port, typically 8883 for TLS".to_string()),
                    attribute: ParamAttribute::Optional,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(DEFAULT_PORT)),
                    valid: None,
                })
                .param(ParamSchema {
                    name: "client_id".to_string(),
                    description: Some("MQTT 客户端 ID，不填则自动生成".to_string()),
                    name_zh: Some("客户端 ID".to_string()),
                    name_en: Some("Client ID".to_string()),
                    description_zh: Some("MQTT 客户端 ID，不填则自动生成".to_string()),
                    description_en: Some("MQTT client ID, auto-generated if empty".to_string()),
                    attribute: ParamAttribute::Optional,
                    ty: ParamType::String,
                    default: None,
                    valid: None,
                })
                .param(ParamSchema {
                    name: "topic_template".to_string(),
                    description: Some("发布主题模板，支持变量：${node_id} ${group_id} ${timestamp}".to_string()),
                    name_zh: Some("主题模板".to_string()),
                    name_en: Some("Topic template".to_string()),
                    description_zh: Some("发布主题模板，支持变量：${node_id} ${group_id} ${timestamp}".to_string()),
                    description_en: Some("Publish topic template, variables: ${node_id} ${group_id} ${timestamp}".to_string()),
                    attribute: ParamAttribute::Optional,
                    ty: ParamType::String,
                    default: Some(serde_json::json!(DEFAULT_TOPIC_TEMPLATE)),
                    valid: None,
                })
                .param(ParamSchema {
                    name: "qos".to_string(),
                    description: Some("QoS 等级 0/1/2".to_string()),
                    name_zh: Some("QoS".to_string()),
                    name_en: Some("QoS".to_string()),
                    description_zh: Some("QoS 等级：0 至多一次、1 至少一次、2 恰好一次".to_string()),
                    description_en: Some("QoS level: 0 at most once, 1 at least once, 2 exactly once".to_string()),
                    attribute: ParamAttribute::Optional,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(DEFAULT_QOS)),
                    valid: Some(ParamValid {
                        min: Some(0),
                        max: Some(2),
                        regex: None,
                        length: None,
                    }),
                })
                .param(ParamSchema {
                    name: "retain".to_string(),
                    description: Some("是否保留消息（retain）".to_string()),
                    name_zh: Some("保留消息".to_string()),
                    name_en: Some("Retain".to_string()),
                    description_zh: Some("是否将消息设为保留（retain）".to_string()),
                    description_en: Some("Whether to set message as retained".to_string()),
                    attribute: ParamAttribute::Optional,
                    ty: ParamType::Bool,
                    default: Some(serde_json::json!(false)),
                    valid: None,
                })
                .param(ParamSchema {
                    name: "upload_format".to_string(),
                    description: Some("上传格式：group_data（完整 GroupData）/ tags_format（tags 数组）".to_string()),
                    name_zh: Some("上传格式".to_string()),
                    name_en: Some("Upload format".to_string()),
                    description_zh: Some("上传格式：group_data 完整 GroupData，tags_format 为 tags 数组".to_string()),
                    description_en: Some("Upload format: group_data for full GroupData, tags_format for tags array".to_string()),
                    attribute: ParamAttribute::Optional,
                    ty: ParamType::String,
                    default: Some(serde_json::json!(UPLOAD_FORMAT_GROUP_DATA)),
                    valid: None,
                })
                .param(ParamSchema {
                    name: "keep_alive_secs".to_string(),
                    description: Some("Keep Alive 秒数".to_string()),
                    name_zh: Some("保活时间(秒)".to_string()),
                    name_en: Some("Keep alive (sec)".to_string()),
                    description_zh: Some("MQTT Keep Alive 间隔，单位秒".to_string()),
                    description_en: Some("MQTT Keep Alive interval in seconds".to_string()),
                    attribute: ParamAttribute::Optional,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(DEFAULT_KEEP_ALIVE_SECS as i64)),
                    valid: None,
                })
                .param(ParamSchema {
                    name: "username".to_string(),
                    description: Some("Broker 用户名（可选）".to_string()),
                    name_zh: Some("用户名".to_string()),
                    name_en: Some("Username".to_string()),
                    description_zh: Some("Broker 认证用户名，可选".to_string()),
                    description_en: Some("Broker username for authentication, optional".to_string()),
                    attribute: ParamAttribute::Optional,
                    ty: ParamType::String,
                    default: None,
                    valid: None,
                })
                .param(ParamSchema {
                    name: "password".to_string(),
                    description: Some("Broker 密码（可选）".to_string()),
                    name_zh: Some("密码".to_string()),
                    name_en: Some("Password".to_string()),
                    description_zh: Some("Broker 认证密码，可选".to_string()),
                    description_en: Some("Broker password for authentication, optional".to_string()),
                    attribute: ParamAttribute::Optional,
                    ty: ParamType::String,
                    default: None,
                    valid: None,
                })
                .param(ParamSchema {
                    name: "cache_memory_size".to_string(),
                    description: Some("离线缓存条数".to_string()),
                    name_zh: Some("离线缓存条数".to_string()),
                    name_en: Some("Cache size".to_string()),
                    description_zh: Some("离线时内存缓存的最大消息条数".to_string()),
                    description_en: Some("Maximum number of messages to cache when offline".to_string()),
                    attribute: ParamAttribute::Optional,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(DEFAULT_CACHE_MEMORY_SIZE)),
                    valid: None,
                })
                .param(ParamSchema {
                    name: "cache_sync_interval_ms".to_string(),
                    description: Some("恢复连接后补发缓存消息的时间间隔（毫秒）".to_string()),
                    name_zh: Some("补发间隔(ms)".to_string()),
                    name_en: Some("Cache sync interval (ms)".to_string()),
                    description_zh: Some("恢复连接后补发缓存消息的时间间隔，单位毫秒".to_string()),
                    description_en: Some("Interval for replaying cached messages after reconnect, in milliseconds".to_string()),
                    attribute: ParamAttribute::Optional,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(DEFAULT_CACHE_SYNC_INTERVAL_MS as i64)),
                    valid: None,
                })
                .param(ParamSchema {
                    name: "ssl".to_string(),
                    description: Some("是否启用 TLS/SSL".to_string()),
                    name_zh: None,
                    name_en: None,
                    description_zh: None,
                    description_en: None,
                    attribute: ParamAttribute::Optional,
                    ty: ParamType::Bool,
                    default: Some(serde_json::json!(false)),
                    valid: None,
                })
                .param(ParamSchema {
                    name: "ca_file".to_string(),
                    description: Some("CA 证书文件路径（SSL 且自签名时）".to_string()),
                    name_zh: None,
                    name_en: None,
                    description_zh: None,
                    description_en: None,
                    attribute: ParamAttribute::Optional,
                    ty: ParamType::String,
                    default: None,
                    valid: None,
                })
                .param(ParamSchema {
                    name: "client_cert_file".to_string(),
                    description: Some("客户端证书文件路径（双向认证时）".to_string()),
                    name_zh: None,
                    name_en: None,
                    description_zh: None,
                    description_en: None,
                    attribute: ParamAttribute::Optional,
                    ty: ParamType::String,
                    default: None,
                    valid: None,
                })
                .param(ParamSchema {
                    name: "client_key_file".to_string(),
                    description: Some("客户端私钥文件路径（双向认证时）".to_string()),
                    name_zh: None,
                    name_en: None,
                    description_zh: None,
                    description_en: None,
                    attribute: ParamAttribute::Optional,
                    ty: ParamType::String,
                    default: None,
                    valid: None,
                }),
        )
    }

    async fn open(&self, node_id: NodeId, config: PluginConfig) -> PluginResult<()> {
        let host = config_str(&config, "host", DEFAULT_HOST);
        let port = config_u16(&config, "port", DEFAULT_PORT);
        let client_id = config_str(
            &config,
            "client_id",
            &format!("gateway-{}", uuid::Uuid::from_u128(node_id.0.as_u128())),
        );
        let topic_template = if let Some(t) = config.get("topic_template").and_then(|v| v.as_str()) {
            t.to_string()
        } else if let Some(prefix) = config.get("topic_prefix").and_then(|v| v.as_str()) {
            format!("{}/${{node_id}}/${{group_id}}", prefix.trim_end_matches('/'))
        } else {
            DEFAULT_TOPIC_TEMPLATE.to_string()
        };
        let qos_u8 = config.get("qos").and_then(|v| v.as_u64()).map(|n| n as u8).unwrap_or(DEFAULT_QOS);
        let qos = PublishQos(qos_u8.min(2));
        let retain = config_bool(&config, "retain", false);
        let upload_format = config_str(&config, "upload_format", UPLOAD_FORMAT_GROUP_DATA);
        let keep_alive_secs = config.get("keep_alive_secs").and_then(|v| v.as_u64()).unwrap_or(DEFAULT_KEEP_ALIVE_SECS);
        let cache_memory_size = config_usize(&config, "cache_memory_size", DEFAULT_CACHE_MEMORY_SIZE);
        let cache_sync_interval_ms = config.get("cache_sync_interval_ms").and_then(|v| v.as_u64()).unwrap_or(DEFAULT_CACHE_SYNC_INTERVAL_MS);
        let ssl = config_bool(&config, "ssl", false);

        let mut state = self.state.write().await;
        state.open_nodes.insert(node_id);

        #[cfg(feature = "mqtt-client")]
        {
            use rumqttc::{AsyncClient, MqttOptions, Transport};

            let mut mqttoptions = MqttOptions::new(client_id.clone(), host.clone(), port);
            mqttoptions.set_keep_alive(Duration::from_secs(keep_alive_secs.min(65535)));
            if let Some(u) = config.get("username").and_then(|v| v.as_str()) {
                if let Some(p) = config.get("password").and_then(|v| v.as_str()) {
                    mqttoptions.set_credentials(u, p);
                }
            }
            if ssl {
                let ca = config.get("ca_file").and_then(|v| v.as_str())
                    .and_then(|p| std::fs::read(p).ok())
                    .unwrap_or_default();
                let client_auth = match (
                    config.get("client_cert_file").and_then(|v| v.as_str()),
                    config.get("client_key_file").and_then(|v| v.as_str()),
                ) {
                    (Some(cert_path), Some(key_path)) => {
                        let cert = std::fs::read(cert_path).ok();
                        let key = std::fs::read(key_path).ok();
                        match (cert, key) {
                            (Some(c), Some(k)) => Some((c, k)),
                            _ => None,
                        }
                    }
                    _ => None,
                };
                if !ca.is_empty() {
                    mqttoptions.set_transport(Transport::tls(ca, client_auth, None));
                }
            }
            let cap = (cache_memory_size + 32).min(65535);
            let (client, eventloop) = AsyncClient::new(mqttoptions, cap);
            let cache = Arc::new(RwLock::new(VecDeque::new()));
            let cache_clone = cache.clone();
            let client_clone = client.clone();
            let sync_interval = cache_sync_interval_ms;
            tokio::spawn(async move {
                run_event_loop(client_clone, eventloop, cache_clone, cache_memory_size, qos, retain, sync_interval).await;
            });
            state.nodes.insert(
                node_id,
                NodeMqttState {
                    client,
                    cache,
                    cache_max: cache_memory_size,
                    topic_template,
                    qos,
                    retain,
                    upload_format,
                },
            );
        }

        #[cfg(not(feature = "mqtt-client"))]
        {
            let _ = (host, port, client_id, ssl);
            state.nodes.insert(
                node_id,
                NodeMqttState {
                    cache: Arc::new(RwLock::new(VecDeque::new())),
                    cache_max: cache_memory_size,
                    topic_template,
                    qos,
                    retain,
                    upload_format,
                },
            );
        }

        Ok(())
    }

    async fn close(&self, node_id: NodeId) -> PluginResult<()> {
        let mut state = self.state.write().await;
        state.open_nodes.remove(&node_id);
        state.subscriptions.remove(&node_id);
        state.nodes.remove(&node_id);
        Ok(())
    }

    /// 用户修改插件配置时：关闭当前连接并用新配置重新 open，使 QoS/主题模板/TLS 等生效。
    async fn setting(&self, node_id: NodeId, config: PluginConfig) -> PluginResult<()> {
        self.close(node_id).await?;
        self.open(node_id, config).await
    }

    async fn set_subscriptions(
        &self,
        node_id: NodeId,
        subscriptions: &[GroupSubscription],
    ) -> PluginResult<()> {
        let mut state = self.state.write().await;
        state
            .subscriptions
            .insert(node_id, subscriptions.to_vec());
        Ok(())
    }

    async fn on_group_data(&self, node_id: NodeId, data: Arc<GroupData>) -> PluginResult<()> {
        let state = self.state.read().await;
        let Some(node_state) = state.nodes.get(&node_id) else {
            return Ok(());
        };
        let topic = topic_from_template(
            &node_state.topic_template,
            data.node_id,
            data.group_id,
            data.ts,
        );
        let payload = payload_for_format(&data, &node_state.upload_format);

        #[cfg(feature = "mqtt-client")]
        {
            let qos = node_state.qos.to_rumqttc();
            match node_state.client.publish(&topic, qos, node_state.retain, payload.clone()).await {
                Ok(()) => {}
                Err(e) => {
                    warn!(node_id = ?node_id, topic = %topic, "mqtt publish failed, enqueue cache: {}", e);
                    let mut cache = node_state.cache.write().await;
                    if cache.len() < node_state.cache_max {
                        cache.push_back((topic, payload));
                    }
                }
            }
        }

        #[cfg(not(feature = "mqtt-client"))]
        {
            let _ = (topic, payload);
            tracing::info!(node_id = ?node_id, "mqtt (no client): would publish");
        }

        Ok(())
    }
}

#[cfg(feature = "mqtt-client")]
async fn run_event_loop(
    client: rumqttc::AsyncClient,
    mut eventloop: rumqttc::EventLoop,
    cache: Arc<RwLock<VecDeque<(String, Vec<u8>)>>>,
    _cache_max: usize,
    qos: PublishQos,
    retain: bool,
    cache_sync_interval_ms: u64,
) {
    use rumqttc::{Event, Packet};
    let qos_r = qos.to_rumqttc();
    let interval = Duration::from_millis(cache_sync_interval_ms.max(10));
    loop {
        match eventloop.poll().await {
            Ok(Event::Incoming(Packet::ConnAck(_))) => {
                loop {
                    let (topic, payload) = {
                        let mut c = cache.write().await;
                        match c.pop_front() {
                            Some(t) => t,
                            None => break,
                        }
                    };
                    let p = payload.clone();
                    if let Err(e) = client.publish(&topic, qos_r, retain, p).await {
                        warn!(topic = %topic, "cache drain publish failed: {}", e);
                        let mut c = cache.write().await;
                        c.push_front((topic, payload));
                        break;
                    }
                    tokio::time::sleep(interval).await;
                }
            }
            Err(e) => {
                warn!("mqtt event loop error: {}", e);
            }
            _ => {}
        }
    }
}
