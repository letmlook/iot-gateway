//! 北向 MQTT 插件：接收 GroupData，发布到 MQTT Broker。
//!
//! 功能：QoS 0/1/2、主题模板（变量替换）、TLS/SSL、离线内存缓存与恢复补发、
//! 上传格式（group_data / tags_format）、retain、keep_alive、cache_sync_interval。
//! 事件循环在独立 OS 线程中运行，保证以 .so/.dll 加载时 open() 返回后仍能建连；以 .so 部署时修改代码后需重新构建插件并重启网关。

#[cfg(feature = "ffi")]
mod ffi;

mod config;
mod format;
mod state;

use config::{
    config_bool, config_schema, config_str, config_u16, config_usize,
    DEFAULT_CACHE_MEMORY_SIZE, DEFAULT_CACHE_SYNC_INTERVAL_MS, DEFAULT_HOST, DEFAULT_KEEP_ALIVE_SECS,
    DEFAULT_PORT, DEFAULT_TOPIC_TEMPLATE, DEFAULT_QOS, UPLOAD_FORMAT_VALUES_FORMAT,
};
use format::{payload_for_format, topic_from_template};
use gateway_sdk::log;
use gateway_sdk::{GroupData, GroupSubscription, NodeId, NorthPlugin, PluginConfig, PluginMeta};
use gateway_sdk::types::PluginKind;
use gateway_sdk::PluginResult;
use state::{MqttConnectionStatus, MqttState, NodeMqttState, PublishQos};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

/// 北向 MQTT 插件
pub struct MqttPlugin {
    state: Arc<RwLock<MqttState>>,
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

#[async_trait::async_trait]
impl NorthPlugin for MqttPlugin {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "mqtt",
            kind: PluginKind::North,
            description: Some("MQTT 北向：QoS 0/1/2、主题模板、TLS、离线缓存；上报格式兼容 NeuronEX values/tags/ecp 及 group_data/raw_data"),
            version: "0.2.0",
            name_zh: Some("MQTT"),
            name_en: Some("MQTT"),
            description_zh: Some("MQTT 北向：QoS 0/1/2、主题模板、TLS、离线缓存；上报格式兼容 NeuronEX values/tags/ecp 及 group_data/raw_data"),
            description_en: Some("MQTT north: QoS 0/1/2, topic template, TLS, offline cache; upload formats aligned with NeuronEX values/tags/ecp, group_data, raw_data"),
        }
    }

    fn config_schema(&self) -> Option<gateway_sdk::ConfigSchema> {
        Some(config_schema())
    }

    async fn open(&self, node_id: NodeId, config: PluginConfig) -> PluginResult<()> {
        let host = config_str(&config, "host", DEFAULT_HOST);
        let port = config_u16(&config, "port", DEFAULT_PORT);
        log::info(node_id, format!("open mqtt: {}:{}, connecting...", host, port));
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
        let upload_format = config_str(&config, "upload_format", UPLOAD_FORMAT_VALUES_FORMAT);
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
            let cache = Arc::new(RwLock::new(std::collections::VecDeque::new()));
            let connection_status = Arc::new(RwLock::new(MqttConnectionStatus::default()));
            let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
            let cache_clone = cache.clone();
            let conn_status_clone = connection_status.clone();
            let client_clone = client.clone();
            let sync_interval = cache_sync_interval_ms;
            let node_id_loop = node_id;
            let host_loop = host.clone();
            let port_loop = port;
            let handle = std::thread::spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("mqtt event loop runtime");
                rt.block_on(run_event_loop(
                    node_id_loop,
                    host_loop,
                    port_loop,
                    client_clone,
                    eventloop,
                    cache_clone,
                    conn_status_clone,
                    cache_memory_size,
                    qos,
                    retain,
                    sync_interval,
                    cancel_rx,
                ));
            });
            state.nodes.insert(
                node_id,
                NodeMqttState {
                    client,
                    cache,
                    connection_status,
                    cache_max: cache_memory_size,
                    topic_template,
                    qos,
                    retain,
                    upload_format,
                    cancel_tx: Some(cancel_tx),
                    event_loop_handle: Some(handle),
                },
            );
        }

        #[cfg(not(feature = "mqtt-client"))]
        {
            let _ = (host, port, client_id, ssl);
            state.nodes.insert(
                node_id,
                NodeMqttState {
                    cache: Arc::new(RwLock::new(std::collections::VecDeque::new())),
                    connection_status: Arc::new(RwLock::new(MqttConnectionStatus::default())),
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
        log::info(node_id, "close mqtt, disconnecting");
        #[cfg(feature = "mqtt-client")]
        {
            let (client_opt, cancel_tx, handle_opt) = {
                let mut state = self.state.write().await;
                state.open_nodes.remove(&node_id);
                state.subscriptions.remove(&node_id);
                let node_state = state.nodes.remove(&node_id);
                node_state.map(|ns| (Some(ns.client), ns.cancel_tx, ns.event_loop_handle)).unwrap_or((None, None, None))
            };
            if let Some(client) = client_opt {
                let _ = client.disconnect().await;
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
        #[cfg(not(feature = "mqtt-client"))]
        {
            let mut state = self.state.write().await;
            state.open_nodes.remove(&node_id);
            state.subscriptions.remove(&node_id);
            state.nodes.remove(&node_id);
        }
        Ok(())
    }

    async fn start(&self, node_id: NodeId) -> PluginResult<()> {
        log::info(node_id, "start mqtt");
        Ok(())
    }

    async fn stop(&self, node_id: NodeId) -> PluginResult<()> {
        log::info(node_id, "stop mqtt");
        Ok(())
    }

    async fn setting(&self, node_id: NodeId, config: PluginConfig) -> PluginResult<()> {
        log::info(node_id, "setting mqtt, reconnecting with new config");
        self.close(node_id).await?;
        self.open(node_id, config).await
    }

    async fn set_subscriptions(
        &self,
        node_id: NodeId,
        subscriptions: &[GroupSubscription],
    ) -> PluginResult<()> {
        log::info(node_id, format!("set_subscriptions: {} group(s)", subscriptions.len()));
        let mut state = self.state.write().await;
        state
            .subscriptions
            .insert(node_id, subscriptions.to_vec());
        Ok(())
    }

    async fn connection_status(&self, node_id: NodeId) -> Option<serde_json::Value> {
        let state = self.state.read().await;
        let node_state = state.nodes.get(&node_id)?;
        let st = node_state.connection_status.read().await;
        Some(serde_json::json!({
            "connected": st.connected,
            "last_error": st.last_error,
        }))
    }

    async fn on_group_data(&self, node_id: NodeId, data: Arc<GroupData>) -> PluginResult<()> {
        let state = self.state.read().await;
        let Some(node_state) = state.nodes.get(&node_id) else {
            log::warn(node_id, "on_group_data: north node not open (no MQTT client), skip publish");
            return Ok(());
        };
        let topic = topic_from_template(
            &node_state.topic_template,
            data.node_id,
            data.group_id,
            data.node_name.as_deref(),
            data.group_name.as_deref(),
            data.ts,
        );
        let payload = payload_for_format(&data, &node_state.upload_format);

        #[cfg(feature = "mqtt-client")]
        {
            let qos = node_state.qos.to_rumqttc();
            match node_state.client.publish(&topic, qos, node_state.retain, payload.clone()).await {
                Ok(()) => {}
                Err(e) => {
                    log::warn(node_id, format!("mqtt publish failed, enqueue cache: {} (topic={})", e, topic));
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
    node_id: NodeId,
    host: String,
    port: u16,
    client: rumqttc::AsyncClient,
    mut eventloop: rumqttc::EventLoop,
    cache: Arc<RwLock<std::collections::VecDeque<(String, Vec<u8>)>>>,
    connection_status: Arc<RwLock<MqttConnectionStatus>>,
    _cache_max: usize,
    qos: PublishQos,
    retain: bool,
    cache_sync_interval_ms: u64,
    mut cancel_rx: tokio::sync::oneshot::Receiver<()>,
) {
    use rumqttc::mqttbytes::v4::ConnectReturnCode;
    use rumqttc::{Event, Packet};
    let addr = format!("{}:{}", host, port);
    log::info(node_id, format!("mqtt event loop 已启动, 正在连接 broker={}", addr));
    let qos_r = qos.to_rumqttc();
    let interval = Duration::from_millis(cache_sync_interval_ms.max(10));
    loop {
        tokio::select! {
            _ = &mut cancel_rx => {
                log::info(node_id, format!("mqtt event loop 已退出 (close), broker={}", addr));
                break;
            }
            ev = eventloop.poll() => {
                match ev {
                    Ok(Event::Incoming(Packet::ConnAck(ack))) => {
                        if ack.code == ConnectReturnCode::Success {
                            log::info(node_id, format!("mqtt 连接成功: broker={}", addr));
                            let mut st = connection_status.write().await;
                            st.connected = true;
                            st.last_error = None;
                        } else {
                            let err_msg = format!("mqtt 连接失败: broker={}, 拒绝原因: {:?}", addr, ack.code);
                            log::warn(node_id, err_msg.as_str());
                            let mut st = connection_status.write().await;
                            st.connected = false;
                            st.last_error = Some(err_msg.clone());
                        }
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
                                log::warn(node_id, format!("cache drain publish failed: {} (topic={})", e, topic));
                                let mut c = cache.write().await;
                                c.push_front((topic, payload));
                                break;
                            }
                            tokio::time::sleep(interval).await;
                        }
                    }
                    Err(e) => {
                        let err_msg = e.to_string();
                        log::warn(node_id, format!("mqtt 连接断开: broker={}, 错误: {}, 5s 后重连", addr, err_msg));
                        let mut st = connection_status.write().await;
                        st.connected = false;
                        st.last_error = Some(err_msg);
                        tokio::time::sleep(Duration::from_secs(5)).await;
                    }
                    _ => {}
                }
            }
        }
    }
}
