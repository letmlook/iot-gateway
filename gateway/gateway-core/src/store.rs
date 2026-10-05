//! 节点/组/标签存储。内存存储，后续可接持久化。

use dashmap::DashMap;
use gateway_sdk::{Group, GroupId, NodeId, Tag, TagId};
use std::sync::Arc;
use tracing::instrument;

#[derive(Clone, Default)]
pub struct Store {
    nodes: Arc<DashMap<NodeId, crate::node::Node>>,
    groups: Arc<DashMap<(NodeId, GroupId), Group>>,
    tags: Arc<DashMap<TagId, Tag>>,
    /// 某节点下某 Group 的 Tag 列表（便于轮询时按组取 Tag）
    group_tags: Arc<DashMap<(NodeId, GroupId), Vec<TagId>>>,
    /// tag_id -> (node_id, group_id)，便于 tag_remove 时清理 group_tags
    tag_location: Arc<DashMap<TagId, (NodeId, GroupId)>>,
    /// 规则（key = rule id）。规则随配置持久化，运行期状态由 RuleEngine 维护
    rules: Arc<DashMap<String, crate::rules::Rule>>,
    /// 组数据策略（key = (node_id, group_id)）。运行期状态由 filters::FilterState 维护
    policies: Arc<DashMap<(NodeId, GroupId), crate::filters::GroupPolicy>>,
}

impl Store {
    pub fn new() -> Self {
        Self::default()
    }

    // ---------- Nodes ----------
    #[instrument(skip(self))]
    pub fn node_insert(&self, node: crate::node::Node) {
        self.nodes.insert(node.id(), node);
    }

    pub fn node_remove(&self, id: NodeId) -> Option<crate::node::Node> {
        self.nodes.remove(&id).map(|(_, v)| v)
    }

    pub fn node_get(&self, id: NodeId) -> Option<crate::node::Node> {
        self.nodes.get(&id).map(|r| r.clone())
    }

    pub fn nodes_list(&self) -> Vec<crate::node::Node> {
        self.nodes.iter().map(|r| r.value().clone()).collect()
    }

    /// 更新节点状态（启停时）
    pub fn node_update_state(&self, id: NodeId, state: gateway_sdk::NodeState) {
        if let Some(mut n) = self.nodes.get_mut(&id) {
            n.state = state;
        }
    }

    /// 更新节点插件配置（setting 时）
    pub fn node_update_config(&self, id: NodeId, config: gateway_sdk::PluginConfig) {
        if let Some(mut n) = self.nodes.get_mut(&id) {
            n.config.config = config;
        }
    }

    /// 更新节点名称
    pub fn node_update_name(&self, id: NodeId, name: String) {
        if let Some(mut n) = self.nodes.get_mut(&id) {
            n.config.name = name;
        }
    }

    /// 清空存储（加载快照前用）
    pub fn clear(&self) {
        self.nodes.clear();
        self.groups.clear();
        self.tags.clear();
        self.group_tags.clear();
        self.tag_location.clear();
        // 规则与策略都来自快照（apply_snapshot 会重新灌入），因此一并清空
        self.rules.clear();
        self.policies.clear();
    }

    // ---------- Groups ----------
    #[instrument(skip(self))]
    pub fn group_insert(&self, node_id: NodeId, g: Group) {
        self.groups.insert((node_id, g.id), g);
    }

    pub fn group_remove(&self, node_id: NodeId, group_id: GroupId) -> Option<Group> {
        self.group_tags.remove(&(node_id, group_id));
        self.groups.remove(&(node_id, group_id)).map(|(_, v)| v)
    }

    pub fn group_get(&self, node_id: NodeId, group_id: GroupId) -> Option<Group> {
        self.groups.get(&(node_id, group_id)).map(|r| r.clone())
    }

    pub fn groups_by_node(&self, node_id: NodeId) -> Vec<Group> {
        self.groups
            .iter()
            .filter(|r| r.key().0 == node_id)
            .map(|r| r.value().clone())
            .collect()
    }

    /// 同节点下组名唯一：按名称查找组
    pub fn group_get_by_name(&self, node_id: NodeId, name: &str) -> Option<Group> {
        self.groups
            .iter()
            .find(|r| r.key().0 == node_id && r.value().name == name)
            .map(|r| r.value().clone())
    }

    /// 更新组字段（name、interval_ms、description）。
    pub fn group_update(
        &self,
        node_id: NodeId,
        group_id: GroupId,
        name: Option<String>,
        interval_ms: Option<u64>,
        description: Option<Option<String>>,
    ) {
        if let Some(mut g) = self.groups.get_mut(&(node_id, group_id)) {
            if let Some(n) = name {
                g.name = n;
            }
            if let Some(i) = interval_ms {
                g.interval_ms = i;
            }
            if let Some(d) = description {
                g.description = d;
            }
        }
    }

    // ---------- Tags ----------
    #[instrument(skip(self))]
    pub fn tag_insert(&self, node_id: NodeId, t: Tag) {
        let gid = t.group_id;
        let id = t.id;
        self.tag_location.insert(id, (node_id, gid));
        self.tags.insert(id, t);
        self.group_tags.entry((node_id, gid)).or_default().push(id);
    }

    pub fn link_tag_to_group(&self, node_id: NodeId, group_id: GroupId, tag_id: TagId) {
        self.group_tags
            .entry((node_id, group_id))
            .or_default()
            .push(tag_id);
    }

    pub fn tag_remove(&self, id: TagId) -> Option<Tag> {
        let loc = self.tag_location.remove(&id).map(|(_, v)| v);
        if let Some(loc) = loc {
            let empty = self
                .group_tags
                .get_mut(&loc)
                .map(|mut v| {
                    v.retain(|x| *x != id);
                    v.is_empty()
                })
                .unwrap_or(false);
            if empty {
                self.group_tags.remove(&loc);
            }
        }
        self.tags.remove(&id).map(|(_, t)| t)
    }

    pub fn tag_get(&self, id: TagId) -> Option<Tag> {
        self.tags.get(&id).map(|r| r.clone())
    }

    /// tag_id -> (node_id, group_id)，用于 read_tags 等
    pub fn tag_location_get(&self, id: TagId) -> Option<(NodeId, GroupId)> {
        self.tag_location.get(&id).map(|r| *r.value())
    }

    pub fn tags_by_group(&self, node_id: NodeId, group_id: GroupId) -> Vec<Tag> {
        let ids = self
            .group_tags
            .get(&(node_id, group_id))
            .map(|v| v.clone())
            .unwrap_or_default();
        ids.into_iter()
            .filter_map(|id| self.tags.get(&id).map(|r| r.clone()))
            .collect()
    }

    pub fn tags_by_node(&self, node_id: NodeId) -> Vec<Tag> {
        let gs = self.groups_by_node(node_id);
        let mut out = Vec::new();
        for g in gs {
            out.extend(self.tags_by_group(node_id, g.id));
        }
        out
    }

    /// 同组内标签名唯一：按名称查找标签
    pub fn tag_get_by_name(&self, node_id: NodeId, group_id: GroupId, name: &str) -> Option<Tag> {
        self.tags_by_group(node_id, group_id)
            .into_iter()
            .find(|t| t.name == name)
    }

    /// 更新标签字段。
    pub fn tag_update(
        &self,
        tag_id: TagId,
        name: Option<String>,
        address: Option<String>,
        attr: Option<gateway_sdk::TagAttr>,
        data_type: Option<Option<String>>,
        description: Option<Option<String>>,
    ) {
        if let Some(mut t) = self.tags.get_mut(&tag_id) {
            if let Some(n) = name {
                t.name = n;
            }
            if let Some(a) = address {
                t.address = a;
            }
            if let Some(a) = attr {
                t.attr = a;
            }
            if let Some(dt) = data_type {
                t.data_type = dt;
            }
            if let Some(d) = description {
                t.description = d;
            }
        }
    }

    /// 获取所有点位总数
    // ---------- Rules ----------
    pub fn rule_insert(&self, rule: crate::rules::Rule) {
        self.rules.insert(rule.id.clone(), rule);
    }

    pub fn rule_get(&self, id: &str) -> Option<crate::rules::Rule> {
        self.rules.get(id).map(|r| r.clone())
    }

    pub fn rule_remove(&self, id: &str) -> Option<crate::rules::Rule> {
        self.rules.remove(id).map(|(_, v)| v)
    }

    pub fn rules_list(&self) -> Vec<crate::rules::Rule> {
        let mut v: Vec<_> = self.rules.iter().map(|r| r.value().clone()).collect();
        v.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
        v
    }

    /// 命中某个南向组的启用规则（采集热路径用：无规则时返回空并提前返回）
    pub fn rules_by_group(&self, node_id: NodeId, group_id: GroupId) -> Vec<crate::rules::Rule> {
        self.rules
            .iter()
            .filter(|r| {
                r.enabled && r.source.south_node_id == node_id && r.source.group_id == group_id
            })
            .map(|r| r.value().clone())
            .collect()
    }

    // ---------- Policies ----------
    pub fn policy_insert(&self, policy: crate::filters::GroupPolicy) {
        self.policies
            .insert((policy.south_node_id, policy.group_id), policy);
    }

    pub fn policy_remove(
        &self,
        node_id: NodeId,
        group_id: GroupId,
    ) -> Option<crate::filters::GroupPolicy> {
        self.policies.remove(&(node_id, group_id)).map(|(_, v)| v)
    }

    pub fn policy_get(
        &self,
        node_id: NodeId,
        group_id: GroupId,
    ) -> Option<crate::filters::GroupPolicy> {
        self.policies.get(&(node_id, group_id)).map(|r| r.clone())
    }

    pub fn policies_list(&self) -> Vec<crate::filters::GroupPolicy> {
        self.policies.iter().map(|r| r.value().clone()).collect()
    }

    pub fn tags_total_count(&self) -> u64 {
        self.tags.len() as u64
    }
}
