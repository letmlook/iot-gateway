//! 数据流链路监控：南向采集 -> 总线 -> 北向订阅 -> 插件转发
//!
//! 用于排查「数据未正常发出」的问题：可查看各环节计数、lagged、无订阅者、具体点位等情况。

use gateway_sdk::{GroupId, NodeId, TagId};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// 点位级统计：南向某节点某组下某 tag 的发布次数
#[derive(Debug, Clone, serde::Serialize)]
pub struct TagPublishedStat {
    pub south_node_id: NodeId,
    pub group_id: GroupId,
    pub tag_id: TagId,
    pub count: u64,
}

/// 点位级统计：北向某节点从南向某节点某组接收并转发的某 tag 次数
#[derive(Debug, Clone, serde::Serialize)]
pub struct TagForwardedStat {
    pub north_node_id: NodeId,
    pub south_node_id: NodeId,
    pub group_id: GroupId,
    pub tag_id: TagId,
    pub count: u64,
}

/// 数据流各环节计数器（原子操作，支持并发读写）
#[derive(Default)]
pub struct DataFlowMetrics {
    /// 南向 poll_group 成功并 publish 到总线的次数
    pub south_published: AtomicU64,
    /// 南向 publish 时无北向订阅者（消息被丢弃）的次数
    pub bus_no_subscribers: AtomicU64,
    /// 北向从总线 recv 到的消息总数
    pub north_received: AtomicU64,
    /// 北向收到的消息因不在订阅表中被过滤掉的次数
    pub north_filtered: AtomicU64,
    /// 北向通过订阅过滤、实际调用 on_group_data 的次数
    pub north_forwarded: AtomicU64,
    /// on_group_data 返回 Ok 的次数
    pub north_on_group_data_ok: AtomicU64,
    /// on_group_data 返回 Err 的次数
    pub north_on_group_data_err: AtomicU64,
    /// 北向 recv Lagged（落后并跳过消息）的总条数
    pub north_lagged: AtomicU64,
    /// 南向 poll_group 超时的次数（超过 max(3×interval, 5s)）
    pub south_poll_timeout: AtomicU64,
    /// 南向 poll_group 返回错误的次数
    pub south_poll_err: AtomicU64,
    /// 按北向节点的 Lagged 条数（用于定位「是哪个北向节点在丢数据」）
    pub lagged_by_node: Arc<dashmap::DashMap<NodeId, AtomicU64>>,
    /// 点位级：南向 (south_node_id, group_id, tag_id) -> 发布次数
    pub published_per_tag: Arc<dashmap::DashMap<(NodeId, GroupId, TagId), AtomicU64>>,
    /// 点位级：北向 (north_node_id, south_node_id, group_id, tag_id) -> 转发次数
    pub forwarded_per_tag: Arc<dashmap::DashMap<(NodeId, NodeId, GroupId, TagId), AtomicU64>>,
}

impl DataFlowMetrics {
    pub fn new() -> Self {
        Self {
            south_published: AtomicU64::new(0),
            bus_no_subscribers: AtomicU64::new(0),
            north_received: AtomicU64::new(0),
            north_filtered: AtomicU64::new(0),
            north_forwarded: AtomicU64::new(0),
            north_on_group_data_ok: AtomicU64::new(0),
            north_on_group_data_err: AtomicU64::new(0),
            north_lagged: AtomicU64::new(0),
            south_poll_timeout: AtomicU64::new(0),
            south_poll_err: AtomicU64::new(0),
            lagged_by_node: Arc::new(dashmap::DashMap::new()),
            published_per_tag: Arc::new(dashmap::DashMap::new()),
            forwarded_per_tag: Arc::new(dashmap::DashMap::new()),
        }
    }

    /// 南向 publish 成功后，记录每个 tag 的发布次数
    pub fn record_south_published_tags(
        &self,
        south_node_id: NodeId,
        group_id: GroupId,
        values: &[(TagId, gateway_sdk::types::DataValue)],
    ) {
        for (tag_id, _) in values {
            let key = (south_node_id, group_id, *tag_id);
            self.published_per_tag
                .entry(key)
                .or_default()
                .fetch_add(1, Ordering::Relaxed);
        }
    }

    /// 北向 on_group_data 前，记录每个 tag 的转发次数
    pub fn record_north_forwarded_tags(
        &self,
        north_node_id: NodeId,
        south_node_id: NodeId,
        group_id: GroupId,
        values: &[(TagId, gateway_sdk::types::DataValue)],
    ) {
        for (tag_id, _) in values {
            let key = (north_node_id, south_node_id, group_id, *tag_id);
            self.forwarded_per_tag
                .entry(key)
                .or_default()
                .fetch_add(1, Ordering::Relaxed);
        }
    }

    /// 记录某个北向节点因落后而跳过的消息条数
    pub fn record_lagged(&self, north_node_id: NodeId, skipped: u64) {
        self.north_lagged.fetch_add(skipped, Ordering::Relaxed);
        self.lagged_by_node
            .entry(north_node_id)
            .or_default()
            .fetch_add(skipped, Ordering::Relaxed);
    }

    /// 清理已删除节点/组的点位级统计，避免指标 Map 只增不减造成内存缓慢增长
    pub fn forget_south_node(&self, south_node_id: NodeId) {
        self.published_per_tag.retain(|k, _| k.0 != south_node_id);
        self.forwarded_per_tag.retain(|k, _| k.1 != south_node_id);
    }

    /// 清理某南向组相关的点位级统计
    pub fn forget_group(&self, south_node_id: NodeId, group_id: GroupId) {
        self.published_per_tag
            .retain(|k, _| !(k.0 == south_node_id && k.1 == group_id));
        self.forwarded_per_tag
            .retain(|k, _| !(k.1 == south_node_id && k.2 == group_id));
    }

    /// 清理某北向节点的统计
    pub fn forget_north_node(&self, north_node_id: NodeId) {
        self.forwarded_per_tag.retain(|k, _| k.0 != north_node_id);
        self.lagged_by_node.remove(&north_node_id);
    }

    /// 快照：返回当前各计数的副本
    pub fn snapshot(&self) -> DataFlowMetricsSnapshot {
        DataFlowMetricsSnapshot {
            south_published: self.south_published.load(Ordering::Relaxed),
            bus_no_subscribers: self.bus_no_subscribers.load(Ordering::Relaxed),
            north_received: self.north_received.load(Ordering::Relaxed),
            north_filtered: self.north_filtered.load(Ordering::Relaxed),
            north_forwarded: self.north_forwarded.load(Ordering::Relaxed),
            north_on_group_data_ok: self.north_on_group_data_ok.load(Ordering::Relaxed),
            north_on_group_data_err: self.north_on_group_data_err.load(Ordering::Relaxed),
            north_lagged: self.north_lagged.load(Ordering::Relaxed),
            south_poll_timeout: self.south_poll_timeout.load(Ordering::Relaxed),
            south_poll_err: self.south_poll_err.load(Ordering::Relaxed),
            lagged_by_node: self
                .lagged_by_node
                .iter()
                .map(|e| (*e.key(), e.value().load(Ordering::Relaxed)))
                .collect(),
            published_per_tag: self.snapshot_published_per_tag(),
            forwarded_per_tag: self.snapshot_forwarded_per_tag(),
        }
    }

    fn snapshot_published_per_tag(&self) -> Vec<TagPublishedStat> {
        self.published_per_tag
            .iter()
            .map(|e| {
                let (south_node_id, group_id, tag_id) = *e.key();
                TagPublishedStat {
                    south_node_id,
                    group_id,
                    tag_id,
                    count: e.value().load(Ordering::Relaxed),
                }
            })
            .filter(|s| s.count > 0)
            .collect()
    }

    fn snapshot_forwarded_per_tag(&self) -> Vec<TagForwardedStat> {
        self.forwarded_per_tag
            .iter()
            .map(|e| {
                let (north_node_id, south_node_id, group_id, tag_id) = *e.key();
                TagForwardedStat {
                    north_node_id,
                    south_node_id,
                    group_id,
                    tag_id,
                    count: e.value().load(Ordering::Relaxed),
                }
            })
            .filter(|s| s.count > 0)
            .collect()
    }
}

/// 数据流指标快照（用于 API 返回）
#[derive(Debug, Clone, serde::Serialize)]
pub struct DataFlowMetricsSnapshot {
    pub south_published: u64,
    pub bus_no_subscribers: u64,
    pub north_received: u64,
    pub north_filtered: u64,
    pub north_forwarded: u64,
    pub north_on_group_data_ok: u64,
    pub north_on_group_data_err: u64,
    pub north_lagged: u64,
    pub south_poll_timeout: u64,
    pub south_poll_err: u64,
    /// (北向节点, 跳过的消息条数)：定位是哪个北向节点在丢数据
    pub lagged_by_node: Vec<(NodeId, u64)>,
    pub published_per_tag: Vec<TagPublishedStat>,
    pub forwarded_per_tag: Vec<TagForwardedStat>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use gateway_sdk::types::DataValue;

    fn nid() -> NodeId {
        NodeId::new()
    }

    #[test]
    fn lagged_is_attributed_per_north_node() {
        let m = DataFlowMetrics::new();
        let a = nid();
        let b = nid();
        m.record_lagged(a, 3);
        m.record_lagged(a, 2);
        m.record_lagged(b, 7);

        // 全局合计仍然正确
        let snap = m.snapshot();
        assert_eq!(snap.north_lagged, 12, "global lagged counter");

        // 维度化后可定位到具体北向节点
        let mut by_node = snap.lagged_by_node.clone();
        by_node.sort_by_key(|(id, _)| id.0);
        assert_eq!(by_node.len(), 2);
        let a_count = snap
            .lagged_by_node
            .iter()
            .find(|(id, _)| *id == a)
            .map(|(_, n)| *n)
            .unwrap();
        let b_count = snap
            .lagged_by_node
            .iter()
            .find(|(id, _)| *id == b)
            .map(|(_, n)| *n)
            .unwrap();
        assert_eq!(a_count, 5);
        assert_eq!(b_count, 7);
    }

    #[test]
    fn removing_nodes_and_groups_cleans_per_tag_maps() {
        let m = DataFlowMetrics::new();
        let south = nid();
        let other_south = nid();
        let north = nid();
        let g1 = gateway_sdk::GroupId::new();
        let g2 = gateway_sdk::GroupId::new();
        let tag = gateway_sdk::TagId::new();

        m.record_south_published_tags(south, g1, &[(tag, DataValue::Int32(1))]);
        m.record_south_published_tags(south, g2, &[(tag, DataValue::Int32(2))]);
        m.record_south_published_tags(other_south, g1, &[(tag, DataValue::Int32(3))]);
        m.record_north_forwarded_tags(north, south, g1, &[(tag, DataValue::Int32(4))]);
        m.record_lagged(north, 1);

        // 删除单个组：只清掉该组
        m.forget_group(south, g1);
        let snap = m.snapshot();
        assert_eq!(
            snap.published_per_tag.len(),
            2,
            "other group/node entries must remain"
        );

        // 删除南向节点：清掉它名下所有点位统计
        m.forget_south_node(south);
        let snap = m.snapshot();
        assert_eq!(snap.published_per_tag.len(), 1);
        assert!(snap
            .published_per_tag
            .iter()
            .all(|s| s.south_node_id == other_south));
        assert!(
            snap.forwarded_per_tag.is_empty(),
            "south-side removal also clears forwarded stats"
        );

        // 删除北向节点：清掉其转发与 lagged 记录
        m.forget_north_node(north);
        let snap = m.snapshot();
        assert!(snap.lagged_by_node.is_empty());
        assert_eq!(
            snap.north_lagged, 1,
            "global counter stays as a historical total"
        );
    }

    #[test]
    fn bus_capacity_is_configurable() {
        assert_eq!(crate::bus::BUS_CAPACITY, 4096);
        let m = crate::bus::Bus::with_capacity(128);
        assert_eq!(m.subscriber_count(), 0);
    }
}
