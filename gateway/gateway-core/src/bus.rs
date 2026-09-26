//! 消息总线。南向 GroupData -> 核心 -> 北向订阅者。
//!
//! ## 数据流
//! - 南向插件轮询采集后调用 `publish(GroupData)` 发布到总线；
//! - 北向节点启动时用 `subscribe_groups()` 拿到其订阅的 (node, group) 通道接收端，
//!   消费时按订阅表过滤后调用 `on_group_data`。
//!
//! ## 按组分区（为什么不广播）
//! 早期实现使用一条全局 `broadcast` 通道：所有南向组的消息都发给每个北向节点，
//! 各节点再本地过滤。这使每条消息的投递成本为 O(北向节点数)，且慢消费者拖累全局。
//! 现在按 `(node_id, group_id)` 维护独立通道：`publish` 只投给订阅该组的消费者，
//! 投递成本 O(1)，不同组之间也不会互相影响。
//!
//! ## 背压与丢包
//! - 每个通道容量由构造参数决定；慢消费者 `recv()` 会返回 `Err(RecvError::Lagged(n))`，
//!   表示跳过了 n 条消息（由调用方统计与告警，见 `DataFlowMetrics::record_lagged`）；
//! - `publish` 在无订阅者时返回 `Err(data)`，调用方计入 `bus_no_subscribers`。

use gateway_sdk::{GroupData, GroupId, GroupSubscription, NodeId};
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::instrument;

/// 默认总线通道容量（条数）。慢于生产速度的消费者会被标记为 lagged 并跳过消息。
pub const BUS_CAPACITY: usize = 4096;

/// 分区键：一个南向组
pub type GroupKey = (NodeId, GroupId);

/// 总线：按 (南向节点, 组) 分区投递 GroupData。
#[derive(Clone)]
pub struct Bus {
    capacity: usize,
    groups: Arc<dashmap::DashMap<GroupKey, broadcast::Sender<Arc<GroupData>>>>,
}

impl Bus {
    /// 使用默认容量创建总线。
    pub fn new() -> Self {
        Self::with_capacity(BUS_CAPACITY)
    }

    /// 使用指定容量创建总线。容量越大，慢消费者可积压的消息越多，内存占用越高。
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            capacity,
            groups: Arc::new(dashmap::DashMap::new()),
        }
    }

    /// 南向/适配器发布 GroupData 到该组的分区通道。
    /// - 返回 `Ok(n)` 表示已投递到 n 个订阅者；
    /// - 返回 `Err(data)` 表示该组当前无人订阅（或通道已被清理），消息未投递。
    #[instrument(skip(self, data))]
    pub fn publish(&self, data: Arc<GroupData>) -> Result<usize, Arc<GroupData>> {
        let key = (data.node_id, data.group_id);
        match self.groups.get(&key) {
            Some(tx) => tx.send(data).map_err(|e| e.0),
            None => Err(data),
        }
    }

    /// 为订阅集合取接收端；尚不存在的分区通道会按需创建。
    /// 北向节点启动（或订阅变更后重启消费任务）时调用。
    pub fn subscribe_groups(&self, keys: &[GroupKey]) -> Vec<broadcast::Receiver<Arc<GroupData>>> {
        keys.iter()
            .map(|k| {
                self.groups
                    .entry(*k)
                    .or_insert_with(|| broadcast::channel(self.capacity).0)
                    .subscribe()
            })
            .collect()
    }

    /// 删除某个组的通道（组被删除时调用，避免通道与订阅者长期驻留）
    pub fn forget_group(&self, key: &GroupKey) {
        self.groups.remove(key);
    }

    /// 删除某个南向节点下所有组的通道（节点被删除时调用）
    pub fn forget_node(&self, node: NodeId) {
        self.groups.retain(|k, _| k.0 != node);
    }

    /// 当前存在通道的分区数（监控/调试用）
    pub fn partition_count(&self) -> usize {
        self.groups.len()
    }

    /// 当前订阅者数量（监控/调试用）
    pub fn subscriber_count(&self) -> usize {
        self.groups.iter().map(|e| e.value().receiver_count()).sum()
    }
}

impl Default for Bus {
    fn default() -> Self {
        Self::new()
    }
}

/// 北向订阅表：node_id -> [(south_node_id, group_id), ...]
pub type SubscriptionTable = std::collections::HashMap<gateway_sdk::NodeId, Vec<GroupSubscription>>;

/// 将订阅列表转为 (south_node_id, group_id) 集合，用于北向过滤总线消息。
#[inline]
pub fn subscription_set(
    subs: &[GroupSubscription],
) -> HashSet<(gateway_sdk::NodeId, gateway_sdk::GroupId)> {
    subs.iter().map(|s| (s.south_node_id, s.group_id)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use gateway_sdk::types::DataValue;
    use gateway_sdk::TagId;

    fn data(node: NodeId, group: GroupId, v: i32) -> Arc<GroupData> {
        Arc::new(GroupData {
            node_id: node,
            group_id: group,
            ts: Utc::now(),
            values: vec![(TagId::new(), DataValue::Int32(v))],
            node_name: None,
            group_name: None,
            tag_names: None,
        })
    }

    #[tokio::test]
    async fn publish_without_subscribers_is_reported() {
        let bus = Bus::new();
        let d = data(NodeId::new(), GroupId::new(), 1);
        assert!(
            bus.publish(d).is_err(),
            "no subscriber means the message is dropped"
        );
    }

    #[tokio::test]
    async fn partitioned_delivery_only_reaches_interested_subscribers() {
        let bus = Bus::new();
        let south = NodeId::new();
        let g1 = GroupId::new();
        let g2 = GroupId::new();

        // 北向 A 订阅 g1，北向 B 订阅 g2
        let mut recv_a = bus.subscribe_groups(&[(south, g1)]);
        let mut recv_b = bus.subscribe_groups(&[(south, g2)]);
        assert_eq!(bus.partition_count(), 2);

        // 发布 g1：只有 A 能收到，B 完全不受影响
        let n = bus.publish(data(south, g1, 11)).expect("delivered");
        assert_eq!(n, 1, "exactly one subscriber");

        let msg = recv_a[0].recv().await.expect("A receives");
        assert_eq!(msg.values[0].1, DataValue::Int32(11));

        // B 的通道应当是空的（用 try_recv 检查，不阻塞）
        assert!(
            recv_b[0].try_recv().is_err(),
            "subscriber of another group must not see this message"
        );
    }

    #[tokio::test]
    async fn channel_is_created_on_demand_and_removed_on_forget() {
        let bus = Bus::new();
        let south = NodeId::new();
        let g = GroupId::new();
        let key = (south, g);

        // 未订阅前发布失败
        assert!(bus.publish(data(south, g, 1)).is_err());

        let mut recv = bus.subscribe_groups(&[key]);
        assert_eq!(bus.partition_count(), 1);
        assert!(bus.publish(data(south, g, 2)).is_ok());
        let msg = recv[0].recv().await.unwrap();
        assert_eq!(msg.values[0].1, DataValue::Int32(2));

        // 组删除后通道被清理：发布回到"无订阅者"语义
        bus.forget_group(&key);
        assert_eq!(bus.partition_count(), 0);
        assert!(bus.publish(data(south, g, 3)).is_err());
    }

    #[tokio::test]
    async fn forgetting_a_node_clears_all_of_its_groups() {
        let bus = Bus::new();
        let south = NodeId::new();
        let other = NodeId::new();
        let g1 = GroupId::new();
        let g2 = GroupId::new();

        let _ = bus.subscribe_groups(&[(south, g1), (south, g2), (other, g1)]);
        assert_eq!(bus.partition_count(), 3);

        bus.forget_node(south);
        assert_eq!(
            bus.partition_count(),
            1,
            "only the other node's group remains"
        );
    }

    #[tokio::test]
    async fn lagged_subscriber_is_reported_per_channel() {
        // 容量 2：发布 5 条后慢消费者会看到 Lagged(3)
        let bus = Bus::with_capacity(2);
        let south = NodeId::new();
        let g = GroupId::new();
        let mut recv = bus.subscribe_groups(&[(south, g)]);

        for i in 0..5 {
            let _ = bus.publish(data(south, g, i));
        }
        match recv[0].recv().await {
            Err(broadcast::error::RecvError::Lagged(n)) => assert_eq!(n, 3),
            other => panic!("expected Lagged, got ok={}", other.is_ok()),
        }
    }
}
