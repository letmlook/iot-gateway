//! 北向 MQTT 插件：接收 GroupData，发布到 MQTT Broker。支持离线内存缓存，恢复后补发。对标 Neuron MQTT 北向。

#[cfg(feature = "ffi")]
mod ffi;

use gateway_sdk::{
    GroupData, GroupSubscription, NodeId, NorthPlugin, PluginConfig, PluginMeta,
};
use gateway_sdk::types::PluginKind;
use gateway_sdk::PluginResult;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::warn;

const DEFAULT_HOST: &str = "localhost";
const DEFAULT_PORT: u16 = 1883;
const DEFAULT_TOPIC_PREFIX: &str = "gateway/data";
const DEFAULT_CACHE_MEMORY_SIZE: usize = 1000;

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

#[cfg(feature = "mqtt-client")]
struct NodeMqttState {
    client: rumqttc::AsyncClient,
    cache: Arc<RwLock<VecDeque<(String, Vec<u8>)>>>,
    cache_max: usize,
    topic_prefix: String,
}

#[cfg(not(feature = "mqtt-client"))]
struct NodeMqttState {
    cache: Arc<RwLock<VecDeque<(String, Vec<u8>)>>>,
    cache_max: usize,
    topic_prefix: String,
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

#[async_trait::async_trait]
impl NorthPlugin for MqttPlugin {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "mqtt",
            kind: PluginKind::North,
            description: Some("MQTT 北向：将 GroupData 发布到 MQTT Broker，支持离线内存缓存"),
            version: "0.1.0",
        }
    }

    async fn open(&self, node_id: NodeId, config: PluginConfig) -> PluginResult<()> {
        let host = config_str(&config, "host", DEFAULT_HOST);
        let port = config_u16(&config, "port", DEFAULT_PORT);
        let client_id = config_str(
            &config,
            "client_id",
            &format!("gateway-{}", uuid::Uuid::from_u128(node_id.0.as_u128())),
        );
        let topic_prefix = config_str(&config, "topic_prefix", DEFAULT_TOPIC_PREFIX);
        let cache_memory_size = config_usize(&config, "cache_memory_size", DEFAULT_CACHE_MEMORY_SIZE);

        let mut state = self.state.write().await;
        state.open_nodes.insert(node_id);

        #[cfg(feature = "mqtt-client")]
        {
            use rumqttc::{AsyncClient, MqttOptions};
            use std::time::Duration;

            let mut mqttoptions =
                MqttOptions::new(client_id.clone(), host.clone(), port);
            mqttoptions.set_keep_alive(Duration::from_secs(30));
            if let Some(u) = config.get("username").and_then(|v| v.as_str()) {
                if let Some(p) = config.get("password").and_then(|v| v.as_str()) {
                    mqttoptions.set_credentials(u, p);
                }
            }
            let cap = (cache_memory_size + 32).min(65535);
            let (client, eventloop) = AsyncClient::new(mqttoptions, cap);
            let cache = Arc::new(RwLock::new(VecDeque::new()));
            let cache_clone = cache.clone();
            let client_clone = client.clone();
            let topic_prefix_clone = topic_prefix.clone();
            tokio::spawn(async move {
                run_event_loop(eventloop, client_clone, cache_clone, cache_memory_size, topic_prefix_clone).await;
            });
            state.nodes.insert(
                node_id,
                NodeMqttState {
                    client,
                    cache,
                    cache_max: cache_memory_size,
                    topic_prefix,
                },
            );
        }

        #[cfg(not(feature = "mqtt-client"))]
        {
            let _ = (host, port, client_id, topic_prefix, cache_memory_size);
            state.nodes.insert(
                node_id,
                NodeMqttState {
                    cache: Arc::new(RwLock::new(VecDeque::new())),
                    cache_max: cache_memory_size,
                    topic_prefix,
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
        let topic = format!(
            "{}/{}/{}",
            node_state.topic_prefix,
            data.node_id.0,
            data.group_id.0
        );
        let payload = serde_json::to_vec(&*data).unwrap_or_default();

        #[cfg(feature = "mqtt-client")]
        {
            use rumqttc::QoS;
            match node_state.client.publish(&topic, QoS::AtLeastOnce, false, payload.clone()).await {
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
            info!(node_id = ?node_id, "mqtt (no client): would publish");
        }

        Ok(())
    }
}

#[cfg(feature = "mqtt-client")]
async fn run_event_loop(
    mut eventloop: rumqttc::EventLoop,
    client: rumqttc::AsyncClient,
    cache: Arc<RwLock<VecDeque<(String, Vec<u8>)>>>,
    _cache_max: usize,
    _topic_prefix: String,
) {
    use rumqttc::{Event, Packet, QoS};
    loop {
        match eventloop.poll().await {
            Ok(Event::Incoming(Packet::ConnAck(_))) => {
                let mut cache = cache.write().await;
                while let Some((topic, payload)) = cache.pop_front() {
                    let p = payload.clone();
                    if let Err(e) = client.publish(&topic, QoS::AtLeastOnce, false, p).await {
                        warn!(topic = %topic, "cache drain publish failed: {}", e);
                        cache.push_front((topic, payload));
                        break;
                    }
                }
            }
            Err(e) => {
                warn!("mqtt event loop error: {}", e);
            }
            _ => {}
        }
    }
}
