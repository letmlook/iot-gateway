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
    /// 采集批次超期次数：一批采集耗时超过其中最小轮询周期（设备慢或并发不足）
    pub south_poll_overrun: AtomicU64,
    /// 规则触发次数
    pub rules_fired: AtomicU64,
    /// 规则动作失败次数（写值失败 / 目标点位不存在）
    pub rules_action_err: AtomicU64,
    /// 历史数据写入行数
    pub history_rows_written: AtomicU64,
    /// 历史数据清理行数
    pub history_rows_pruned: AtomicU64,
    /// 历史数据写入失败次数
    pub history_write_err: AtomicU64,
    /// 整批过滤抑制次数（过滤后无任何输出）
    pub filters_suppressed_msgs: AtomicU64,
    /// 被过滤抑制的点位次
    pub filters_suppressed_tags: AtomicU64,
    /// 窗口聚合发射次数
    pub window_emitted_msgs: AtomicU64,
    /// 按北向节点的 Lagged 条数（用于定位「是哪个北向节点在丢数据」）
    pub lagged_by_node: Arc<dashmap::DashMap<NodeId, AtomicU64>>,
    /// 按南向节点的 poll 超时次数（定位「是哪个南向节点采集超时」）
    pub south_poll_timeout_by_node: Arc<dashmap::DashMap<NodeId, AtomicU64>>,
    /// 按南向节点的 poll 错误次数（定位「是哪个南向节点在报错」）
    pub south_poll_err_by_node: Arc<dashmap::DashMap<NodeId, AtomicU64>>,
    /// 按规则的触发次数（定位「是哪条规则在频繁触发」）
    pub rules_fired_by_rule: Arc<dashmap::DashMap<String, AtomicU64>>,
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
            south_poll_overrun: AtomicU64::new(0),
            rules_fired: AtomicU64::new(0),
            rules_action_err: AtomicU64::new(0),
            history_rows_written: AtomicU64::new(0),
            history_rows_pruned: AtomicU64::new(0),
            history_write_err: AtomicU64::new(0),
            filters_suppressed_msgs: AtomicU64::new(0),
            filters_suppressed_tags: AtomicU64::new(0),
            window_emitted_msgs: AtomicU64::new(0),
            lagged_by_node: Arc::new(dashmap::DashMap::new()),
            south_poll_timeout_by_node: Arc::new(dashmap::DashMap::new()),
            south_poll_err_by_node: Arc::new(dashmap::DashMap::new()),
            rules_fired_by_rule: Arc::new(dashmap::DashMap::new()),
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

    /// 记录某南向节点一次 poll 超时（全局计数 + 按节点维度）
    pub fn record_south_poll_timeout(&self, south_node_id: NodeId) {
        self.south_poll_timeout.fetch_add(1, Ordering::Relaxed);
        self.south_poll_timeout_by_node
            .entry(south_node_id)
            .or_default()
            .fetch_add(1, Ordering::Relaxed);
    }

    /// 记录某南向节点一次 poll 错误（全局计数 + 按节点维度）
    pub fn record_south_poll_err(&self, south_node_id: NodeId) {
        self.south_poll_err.fetch_add(1, Ordering::Relaxed);
        self.south_poll_err_by_node
            .entry(south_node_id)
            .or_default()
            .fetch_add(1, Ordering::Relaxed);
    }

    /// 记录某条规则触发一次（全局计数 + 按规则维度）
    pub fn record_rule_fired(&self, rule_id: &str) {
        self.rules_fired.fetch_add(1, Ordering::Relaxed);
        self.rules_fired_by_rule
            .entry(rule_id.to_string())
            .or_default()
            .fetch_add(1, Ordering::Relaxed);
    }

    /// 清理已删除节点/组的点位级统计，避免指标 Map 只增不减造成内存缓慢增长
    pub fn forget_south_node(&self, south_node_id: NodeId) {
        self.published_per_tag.retain(|k, _| k.0 != south_node_id);
        self.forwarded_per_tag.retain(|k, _| k.1 != south_node_id);
        self.south_poll_timeout_by_node.remove(&south_node_id);
        self.south_poll_err_by_node.remove(&south_node_id);
    }

    /// 清理某南向组相关的点位级统计
    pub fn forget_group(&self, south_node_id: NodeId, group_id: GroupId) {
        self.published_per_tag
            .retain(|k, _| !(k.0 == south_node_id && k.1 == group_id));
        self.forwarded_per_tag
            .retain(|k, _| !(k.1 == south_node_id && k.2 == group_id));
    }

    /// 清理全部过滤与窗口指标
    pub fn clear_filters(&self) {
        self.filters_suppressed_msgs.store(0, Ordering::Relaxed);
        self.filters_suppressed_tags.store(0, Ordering::Relaxed);
        self.window_emitted_msgs.store(0, Ordering::Relaxed);
    }

    /// 清理某北向节点的统计
    pub fn forget_north_node(&self, north_node_id: NodeId) {
        self.forwarded_per_tag.retain(|k, _| k.0 != north_node_id);
        self.lagged_by_node.remove(&north_node_id);
    }

    /// 清理已删除规则的触发计数（Manager.rule_remove 调用，防止 map 只增不减）
    pub fn forget_rule(&self, rule_id: &str) {
        self.rules_fired_by_rule.remove(rule_id);
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
            south_poll_overrun: self.south_poll_overrun.load(Ordering::Relaxed),
            rules_fired: self.rules_fired.load(Ordering::Relaxed),
            rules_action_err: self.rules_action_err.load(Ordering::Relaxed),
            history_rows_written: self.history_rows_written.load(Ordering::Relaxed),
            history_rows_pruned: self.history_rows_pruned.load(Ordering::Relaxed),
            history_write_err: self.history_write_err.load(Ordering::Relaxed),
            filters_suppressed_msgs: self.filters_suppressed_msgs.load(Ordering::Relaxed),
            filters_suppressed_tags: self.filters_suppressed_tags.load(Ordering::Relaxed),
            window_emitted_msgs: self.window_emitted_msgs.load(Ordering::Relaxed),
            lagged_by_node: self
                .lagged_by_node
                .iter()
                .map(|e| (*e.key(), e.value().load(Ordering::Relaxed)))
                .collect(),
            south_poll_timeout_by_node: self
                .south_poll_timeout_by_node
                .iter()
                .map(|e| (*e.key(), e.value().load(Ordering::Relaxed)))
                .collect(),
            south_poll_err_by_node: self
                .south_poll_err_by_node
                .iter()
                .map(|e| (*e.key(), e.value().load(Ordering::Relaxed)))
                .collect(),
            rules_fired_by_rule: self
                .rules_fired_by_rule
                .iter()
                .map(|e| (e.key().clone(), e.value().load(Ordering::Relaxed)))
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
    /// 采集批次超期次数
    pub south_poll_overrun: u64,
    /// 规则触发次数
    pub rules_fired: u64,
    /// 规则动作失败次数
    pub rules_action_err: u64,
    /// 历史数据写入行数
    pub history_rows_written: u64,
    /// 历史数据清理行数
    pub history_rows_pruned: u64,
    /// 历史数据写入失败次数
    pub history_write_err: u64,
    /// 整批过滤抑制次数
    pub filters_suppressed_msgs: u64,
    /// 被过滤抑制的点位次
    pub filters_suppressed_tags: u64,
    /// 窗口聚合发射次数
    pub window_emitted_msgs: u64,
    /// (北向节点, 跳过的消息条数)：定位是哪个北向节点在丢数据
    pub lagged_by_node: Vec<(NodeId, u64)>,
    /// (南向节点, poll 超时次数)：定位是哪个南向节点采集超时
    pub south_poll_timeout_by_node: Vec<(NodeId, u64)>,
    /// (南向节点, poll 错误次数)：定位是哪个南向节点在报错
    pub south_poll_err_by_node: Vec<(NodeId, u64)>,
    /// (规则 id, 触发次数)：定位是哪条规则在频繁触发
    pub rules_fired_by_rule: Vec<(String, u64)>,
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

    #[test]
    fn poll_and_rule_events_are_attributed_per_node_and_rule() {
        let m = DataFlowMetrics::new();
        let a = nid();
        let b = nid();

        m.record_south_poll_timeout(a);
        m.record_south_poll_timeout(a);
        m.record_south_poll_timeout(b);
        m.record_south_poll_err(b);

        // 全局合计与维度归因同时正确
        let snap = m.snapshot();
        assert_eq!(snap.south_poll_timeout, 3);
        assert_eq!(snap.south_poll_err, 1);
        let count_of = |v: &[(NodeId, u64)], id: &NodeId| {
            v.iter().find(|(n, _)| n == id).map(|(_, c)| *c).unwrap_or(0)
        };
        assert_eq!(count_of(&snap.south_poll_timeout_by_node, &a), 2);
        assert_eq!(count_of(&snap.south_poll_timeout_by_node, &b), 1);
        assert_eq!(count_of(&snap.south_poll_err_by_node, &b), 1);
        assert_eq!(count_of(&snap.south_poll_err_by_node, &a), 0);

        // 规则维度：触发计数 + 删除规则后清理（map 不只增不减）
        let r1 = "rule-1".to_string();
        let r2 = "rule-2".to_string();
        m.record_rule_fired(&r1);
        m.record_rule_fired(&r1);
        m.record_rule_fired(&r2);
        let snap = m.snapshot();
        assert_eq!(snap.rules_fired, 3);
        assert_eq!(
            snap
                .rules_fired_by_rule
                .iter()
                .find(|(id, _)| *id == r1)
                .map(|(_, c)| *c),
            Some(2)
        );
        m.forget_rule(&r1);
        let snap = m.snapshot();
        assert_eq!(snap.rules_fired, 3, "全局计数保留为历史总量");
        assert!(!snap.rules_fired_by_rule.iter().any(|(id, _)| *id == r1));
        assert!(snap.rules_fired_by_rule.iter().any(|(id, _)| *id == r2));

        // 删除南向节点：清掉它的超时/错误维度记录
        m.forget_south_node(a);
        let snap = m.snapshot();
        assert_eq!(count_of(&snap.south_poll_timeout_by_node, &a), 0);
        assert_eq!(count_of(&snap.south_poll_timeout_by_node, &b), 1);
    }
}
