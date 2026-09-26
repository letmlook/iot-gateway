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
            published_per_tag: Arc::new(dashmap::DashMap::new()),
            forwarded_per_tag: Arc::new(dashmap::DashMap::new()),
        }
    }

    /// 南向 publish 成功后，记录每个 tag 的发布次数
    pub fn record_south_published_tags(&self, south_node_id: NodeId, group_id: GroupId, values: &[(TagId, gateway_sdk::types::DataValue)]) {
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
    pub published_per_tag: Vec<TagPublishedStat>,
    pub forwarded_per_tag: Vec<TagForwardedStat>,
}
