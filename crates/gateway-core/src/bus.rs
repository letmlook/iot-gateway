//! 消息总线。南向 GroupData -> 核心 -> 北向订阅者。对标 Neuron NNG 星型拓扑。
//!
//! ## 数据流
//! - 南向插件轮询采集后调用 `publish(GroupData)` 发布到总线；
//! - 北向插件通过 `subscribe()` 获得广播接收端，再根据本节点的 `SubscriptionTable` 过滤后调用 `on_group_data`。
//!
//! ## 背压与丢包
//! - 使用 `tokio::sync::broadcast`：慢消费者会 **lagged**，`recv()` 返回 `Err(RecvError::Lagged(n))` 表示跳过了 n 条消息；
//! - 北向消费循环应处理 `Lagged`（如打日志后继续），或根据业务决定是否重连/告警；
//! - `publish` 在无订阅者时返回 `Err(data)`，调用方可选择丢弃或缓冲。

use gateway_sdk::{GroupData, GroupSubscription};
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::instrument;

/// 默认总线通道容量（条数）。慢于生产速度的消费者会被标记为 lagged 并跳过消息。
pub const BUS_CAPACITY: usize = 4096;

/// 总线：广播 GroupData，北向根据订阅表过滤后消费。
#[derive(Clone)]
pub struct Bus {
    tx: broadcast::Sender<Arc<GroupData>>,
}

impl Bus {
    /// 使用默认容量创建总线。
    pub fn new() -> Self {
        Self::with_capacity(BUS_CAPACITY)
    }

    /// 使用指定容量创建总线。容量越大，慢消费者可积压的消息越多，内存占用越高。
    pub fn with_capacity(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx }
    }

    /// 南向/适配器发布 GroupData。
    /// - 返回 `Ok(n)` 表示已投递到 n 个订阅者；
    /// - 返回 `Err(data)` 表示当前无订阅者，消息未投递，调用方可选择丢弃或缓冲。
    #[instrument(skip(self, data))]
    pub fn publish(&self, data: Arc<GroupData>) -> Result<usize, Arc<GroupData>> {
        self.tx.send(data).map_err(|e| e.0)
    }

    /// 订阅总线，返回广播接收端。北向适配器应使用 `subscription_set(subs)` 过滤后再转发给插件。
    /// 注意：`recv()` 可能返回 `Err(broadcast::error::RecvError::Lagged(n))`，表示落后并跳过了 n 条消息。
    pub fn subscribe(&self) -> broadcast::Receiver<Arc<GroupData>> {
        self.tx.subscribe()
    }

    /// 当前订阅者数量（用于监控/调试）。
    pub fn subscriber_count(&self) -> usize {
        self.tx.receiver_count()
    }
}

impl Default for Bus {
    fn default() -> Self {
        Self::new()
    }
}

/// 北向订阅表：node_id -> [(south_node_id, group_id), ...]
pub type SubscriptionTable = std::collections::HashMap<
    gateway_sdk::NodeId,
    Vec<GroupSubscription>,
>;

/// 将订阅列表转为 (south_node_id, group_id) 集合，用于北向过滤总线消息。
#[inline]
pub fn subscription_set(subs: &[GroupSubscription]) -> HashSet<(gateway_sdk::NodeId, gateway_sdk::GroupId)> {
    subs.iter().map(|s| (s.south_node_id, s.group_id)).collect()
}
