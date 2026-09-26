//! MQTT 插件状态：MqttState、NodeMqttState、连接状态、PublishQos。

use gateway_sdk::{GroupSubscription, NodeId};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct MqttState {
    pub open_nodes: std::collections::HashSet<NodeId>,
    pub subscriptions: HashMap<NodeId, Vec<GroupSubscription>>,
    /// node_id -> 该节点的 MQTT 客户端与缓存
    pub nodes: HashMap<NodeId, NodeMqttState>,
}

/// 发布时使用的 QoS（0/1/2）
#[derive(Clone, Copy)]
pub struct PublishQos(pub u8);

/// MQTT 实际连接状态，由 event loop 更新
#[derive(Clone, Default)]
pub struct MqttConnectionStatus {
    pub connected: bool,
    pub last_error: Option<String>,
}

#[cfg(feature = "mqtt-client")]
pub struct NodeMqttState {
    pub client: rumqttc::AsyncClient,
    /// 离线队列（可持久化到磁盘），断网期间暂存待发消息
    pub queue: Arc<tokio::sync::Mutex<crate::queue::OfflineQueue>>,
    pub connection_status: Arc<RwLock<MqttConnectionStatus>>,
    pub topic_template: String,
    pub qos: PublishQos,
    pub retain: bool,
    pub upload_format: String,
    /// 用于 close 时通知 event loop 退出
    pub cancel_tx: Option<tokio::sync::oneshot::Sender<()>>,
    /// event loop 所在线程句柄，close 时等待退出
    pub event_loop_handle: Option<std::thread::JoinHandle<()>>,
}

#[cfg(not(feature = "mqtt-client"))]
pub struct NodeMqttState {
    pub queue: Arc<tokio::sync::Mutex<crate::queue::OfflineQueue>>,
    pub connection_status: Arc<RwLock<MqttConnectionStatus>>,
    pub topic_template: String,
    pub qos: PublishQos,
    pub retain: bool,
    pub upload_format: String,
}

impl PublishQos {
    #[cfg(feature = "mqtt-client")]
    pub fn to_rumqttc(self) -> rumqttc::QoS {
        use rumqttc::QoS;
        match self.0 {
            0 => QoS::AtMostOnce,
            2 => QoS::ExactlyOnce,
            _ => QoS::AtLeastOnce,
        }
    }
}
