//! 路由核心：插件管理、节点启停、消息路由。

use crate::bus::{subscription_set, Bus, SubscriptionTable};
use crate::data_flow::DataFlowMetrics;
use crate::node::Node;
use crate::store::Store;
use chrono::Utc;
use gateway_sdk::log;
use gateway_sdk::types::{DataValue, TagId};
use gateway_sdk::{
    Group, GroupData, GroupSubscription, NodeId, NodeKind, NodeState, NorthPlugin, PluginConfig,
    PluginInfo, SouthPlugin, Tag,
};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{error, instrument, warn};

use crate::persist::{apply_to_store, Snapshot};

type SouthRegistry = HashMap<String, Arc<dyn SouthPlugin>>;
type NorthRegistry = HashMap<String, Arc<dyn NorthPlugin>>;

/// 南向节点连接状态（由最近一次采集结果推断：成功为已连接，失败为异常）
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct SouthConnectionState {
    pub connected: bool,
    pub last_error: Option<String>,
}

/// 点位最近一次采集到的值（来自采集链路，不额外访问设备）
#[derive(Clone, Debug, serde::Serialize)]
pub struct LastValue {
    pub value: gateway_sdk::types::DataValue,
    /// 采集时刻（毫秒时间戳）
    pub ts_ms: i64,
}

/// 路由核心
/// 南向轮询周期下限（毫秒）：低于该值的配置会被钳制，避免忙循环打满 CPU
pub const MIN_POLL_INTERVAL_MS: u64 = 10;

/// 全局采集并发上限默认值：同时进行的 poll_group 次数上限
pub const DEFAULT_MAX_CONCURRENT_POLLS: usize = 32;

/// 总线默认容量（与 `gateway_core::bus::BUS_CAPACITY` 保持一致）
fn gateway_core_bus_default() -> usize {
    crate::bus::BUS_CAPACITY
}

pub struct Manager {
    pub bus: Bus,
    pub store: Store,
    /// 北向 node_id -> [(south_node_id, group_id), ...]
    subscriptions: Arc<RwLock<SubscriptionTable>>,
    south_plugins: SouthRegistry,
    north_plugins: NorthRegistry,
    /// 南向轮询 task 的 cancel 发令（broadcast，每个 group 任务 subscribe）
    south_cancel: Arc<RwLock<HashMap<NodeId, tokio::sync::broadcast::Sender<()>>>>,
    /// 北向消费 task 的 cancel 发令
    north_cancel: Arc<RwLock<HashMap<NodeId, tokio::sync::oneshot::Sender<()>>>>,
    /// 数据流链路监控
    pub data_flow_metrics: Arc<DataFlowMetrics>,
    /// 南向节点连接状态（按最近一次 poll 结果更新，供 API 与前端展示）
    south_connection_status: Arc<RwLock<HashMap<NodeId, SouthConnectionState>>>,
    /// 全局采集并发上限：所有南向组的 poll_group 共用，避免同一时刻同时打向设备
    poll_permits: Arc<tokio::sync::Semaphore>,
    /// 点位最近值缓存 `(node_id, tag_id) -> 最近一次采集值`。
    /// 供管理台看实时值：**只读缓存，不触碰设备**（否则监控页刷新会变成对 PLC 的真实轮询）。
    last_values: Arc<dashmap::DashMap<(NodeId, TagId), LastValue>>,
}

impl Manager {
    pub fn new() -> Self {
        Self::with_bus_capacity(gateway_core_bus_default())
    }

    /// 以指定总线容量创建（容量决定慢消费者可积压的消息数，用于规模调优）
    pub fn with_bus_capacity(capacity: usize) -> Self {
        Self::with_limits(capacity, DEFAULT_MAX_CONCURRENT_POLLS)
    }

    /// 以指定总线容量与采集并发上限创建。
    ///
    /// `max_concurrent_polls` 是**全局**同时进行的 poll_group 次数上限：现场设备多为
    /// 串行应答，无上限的并发既打不满设备、又会把网关 CPU 与连接数顶上去。
    pub fn with_limits(capacity: usize, max_concurrent_polls: usize) -> Self {
        Self {
            bus: Bus::with_capacity(capacity.max(16)),
            store: Store::new(),
            subscriptions: Arc::new(RwLock::new(SubscriptionTable::default())),
            south_plugins: SouthRegistry::new(),
            north_plugins: NorthRegistry::new(),
            south_cancel: Arc::new(RwLock::new(HashMap::new())), // broadcast senders
            north_cancel: Arc::new(RwLock::new(HashMap::new())),
            data_flow_metrics: Arc::new(DataFlowMetrics::new()),
            south_connection_status: Arc::new(RwLock::new(HashMap::new())),
            poll_permits: Arc::new(tokio::sync::Semaphore::new(max_concurrent_polls.max(1))),
            last_values: Arc::new(dashmap::DashMap::new()),
        }
    }

    /// 南向节点连接状态（由最近一次采集成功/失败推断），供 API 与前端展示
    pub async fn south_connection_status(&self, node_id: NodeId) -> Option<SouthConnectionState> {
        self.south_connection_status
            .read()
            .await
            .get(&node_id)
            .cloned()
    }

    /// 数据流链路监控快照
    pub fn data_flow_snapshot(&self) -> crate::data_flow::DataFlowMetricsSnapshot {
        self.data_flow_metrics.snapshot()
    }

    /// 注册南向插件
    pub fn register_south(&mut self, name: &str, plugin: Arc<dyn SouthPlugin>) {
        self.south_plugins.insert(name.to_string(), plugin);
    }

    /// 注册北向插件
    pub fn register_north(&mut self, name: &str, plugin: Arc<dyn NorthPlugin>) {
        self.north_plugins.insert(name.to_string(), plugin);
    }

    pub fn south_plugin(&self, name: &str) -> Option<Arc<dyn SouthPlugin>> {
        self.south_plugins.get(name).cloned()
    }

    pub fn north_plugin(&self, name: &str) -> Option<Arc<dyn NorthPlugin>> {
        self.north_plugins.get(name).cloned()
    }

    /// 南向插件列表（含中英文名称与描述）
    pub fn south_plugins(&self) -> Vec<PluginInfo> {
        self.south_plugins
            .iter()
            .map(|(k, v)| PluginInfo::from_meta(k, &v.meta()))
            .collect()
    }

    /// 北向插件列表（含中英文名称与描述）
    pub fn north_plugins(&self) -> Vec<PluginInfo> {
        self.north_plugins
            .iter()
            .map(|(k, v)| PluginInfo::from_meta(k, &v.meta()))
            .collect()
    }

    /// 按名称取点位（同一节点同一组内名称唯一）；规则 API 用它校验引用
    pub fn tag_get_by_name(
        &self,
        node_id: NodeId,
        group_id: gateway_sdk::GroupId,
        name: &str,
    ) -> Option<Tag> {
        self.store.tag_get_by_name(node_id, group_id, name)
    }

    /// 某节点的点位最近值（只读缓存：不发起任何设备访问）
    pub fn last_values(&self, node_id: NodeId) -> HashMap<TagId, LastValue> {
        self.last_values
            .iter()
            .filter(|e| e.key().0 == node_id)
            .map(|e| (e.key().1, e.value().clone()))
            .collect()
    }

    /// 节点的全部点位（供「实时值」接口按点位顺序输出）
    pub fn tags_by_node(&self, node_id: NodeId) -> Vec<Tag> {
        self.store.tags_by_node(node_id)
    }

    /// 取总线句柄（旁路订阅用：历史落库、旁路审计等）
    pub fn bus(&self) -> Bus {
        self.bus.clone()
    }

    // ---------- Rules ----------
    /// 规则列表（配置，不含运行期状态）
    pub fn rules_list(&self) -> Vec<crate::rules::Rule> {
        self.store.rules_list()
    }

    pub fn rule_get(&self, id: &str) -> Option<crate::rules::Rule> {
        self.store.rule_get(id)
    }

    pub fn rule_insert(&self, rule: crate::rules::Rule) {
        self.store.rule_insert(rule);
    }

    /// 删除规则，同时清掉它的运行期状态
    pub fn rule_remove(&self, id: &str) -> Option<crate::rules::Rule> {
        let removed = self.store.rule_remove(id);
        if removed.is_some() {
            crate::rules::engine().forget(id);
        }
        removed
    }

    // ---------- Nodes ----------
    /// 创建节点并调用插件 open、init。失败则回滚。按 config_schema 校验 config。
    #[instrument(skip(self))]
    pub async fn node_create(
        &self,
        name: String,
        kind: NodeKind,
        plugin_name: String,
        config: PluginConfig,
    ) -> Result<Node, String> {
        let plugin_south = self.south_plugin(&plugin_name);
        let plugin_north = self.north_plugin(&plugin_name);
        match kind {
            gateway_sdk::NodeKind::South => {
                let p = plugin_south.as_ref().ok_or("south plugin not found")?;
                if let Some(ref schema) = p.config_schema() {
                    schema.validate_config(&config).map_err(|e| e.to_string())?;
                }
            }
            gateway_sdk::NodeKind::North => {
                let p = plugin_north.as_ref().ok_or("north plugin not found")?;
                if let Some(ref schema) = p.config_schema() {
                    schema.validate_config(&config).map_err(|e| e.to_string())?;
                }
            }
        }
        let node = Node::new(name, kind, plugin_name.clone(), config.clone());
        let id = node.id();
        self.store.node_insert(node.clone());

        let plugin_south = self.south_plugin(&plugin_name);
        let plugin_north = self.north_plugin(&plugin_name);
        match kind {
            gateway_sdk::NodeKind::South => {
                let p = plugin_south.as_ref().ok_or("south plugin not found")?;
                p.open(id, config.clone())
                    .await
                    .map_err(|e| e.to_string())?;
                if let Err(e) = p.init(id).await {
                    let _ = p.close(id).await;
                    let _ = self.store.node_remove(id);
                    return Err(e.to_string());
                }
            }
            gateway_sdk::NodeKind::North => {
                let p = plugin_north.as_ref().ok_or("north plugin not found")?;
                p.open(id, config.clone())
                    .await
                    .map_err(|e| e.to_string())?;
                if let Err(e) = p.init(id).await {
                    let _ = p.close(id).await;
                    let _ = self.store.node_remove(id);
                    return Err(e.to_string());
                }
            }
        }
        Ok(node)
    }

    pub fn node_get(&self, id: NodeId) -> Option<Node> {
        self.store.node_get(id)
    }

    /// 更新节点名称。
    pub fn node_update(&self, id: NodeId, name: Option<String>) -> Result<(), String> {
        let Some(n) = name else {
            return Ok(());
        };
        self.store.node_update_name(id, n);
        Ok(())
    }

    /// 删除节点：先 stop，再 uninit、close，清理订阅引用，最后从 store 移除。
    pub async fn node_remove(&self, id: NodeId) -> Option<Node> {
        let _ = self.node_stop(id).await;
        let node = self.store.node_get(id)?;
        let plugin_name = node.config.plugin_name.clone();
        let is_south = node.kind() == NodeKind::South;
        let plugin_south = self.south_plugin(&plugin_name);
        let plugin_north = self.north_plugin(&plugin_name);
        if let Some(p) = plugin_south {
            let _ = p.uninit(id).await;
            let _ = p.close(id).await;
        }
        if let Some(p) = plugin_north {
            let _ = p.uninit(id).await;
            let _ = p.close(id).await;
        }
        if is_south {
            self.remove_subscriptions_ref_south(id).await;
            self.data_flow_metrics.forget_south_node(id);
            // 分区通道随节点回收，避免通道与订阅者长期驻留
            self.bus.forget_node(id);
            // 最近值缓存同样按节点回收
            self.last_values.retain(|k, _| k.0 != id);
            // 过滤状态与窗口状态按节点清零
            crate::filters::forget_node(id);
        } else {
            self.data_flow_metrics.forget_north_node(id);
        }
        self.store.node_remove(id)
    }

    pub fn nodes_list(&self) -> Vec<Node> {
        self.store.nodes_list()
    }

    /// 设置北向节点的订阅。若该北向节点正在运行，会重启其总线消费任务以使新订阅生效。
    /// 会过滤掉无效的订阅项（south_node_id 或 group_id 不存在）。
    #[instrument(skip(self))]
    pub async fn set_north_subscriptions(
        &self,
        north_node_id: NodeId,
        subs: Vec<GroupSubscription>,
    ) {
        let subs: Vec<GroupSubscription> = subs
            .into_iter()
            .filter(|s| {
                let node = self.store.node_get(s.south_node_id);
                let Some(n) = node else { return false };
                if n.kind() != NodeKind::South {
                    return false;
                }
                self.store.group_get(s.south_node_id, s.group_id).is_some()
            })
            .collect();
        {
            let mut t = self.subscriptions.write().await;
            t.insert(north_node_id, subs.clone());
        }
        let node = self.store.node_get(north_node_id);
        let Some(node) = node else { return };
        if node.kind() != NodeKind::North || node.state != NodeState::Running {
            return;
        }
        let plugin_name = node.config.plugin_name.clone();
        let Some(plugin) = self.north_plugin(&plugin_name) else {
            return;
        };
        let mut c = self.north_cancel.write().await;
        if let Some(tx) = c.remove(&north_node_id) {
            let _ = tx.send(());
        }
        drop(c);
        plugin.set_subscriptions(north_node_id, &subs).await.ok();
        self.spawn_north_consumer(north_node_id, plugin, &subs)
            .await;
    }

    /// 启动（或重启）北向节点的总线消费任务。
    ///
    /// 订阅集合对应的分区通道各自起一个轻量转发任务，汇总到同一条 mpsc 上统一消费：
    /// 消费循环因此只需等待一条通道，且订阅为空时也能安全等待取消信号。
    async fn spawn_north_consumer(
        &self,
        id: NodeId,
        plugin: Arc<dyn NorthPlugin>,
        subs: &[GroupSubscription],
    ) {
        let keys: Vec<(NodeId, gateway_sdk::GroupId)> =
            subs.iter().map(|s| (s.south_node_id, s.group_id)).collect();
        let recvs = self.bus.subscribe_groups(&keys);

        let (agg_tx, mut agg_rx) = tokio::sync::mpsc::channel::<
            Result<Arc<gateway_sdk::GroupData>, tokio::sync::broadcast::error::RecvError>,
        >((recvs.len() * 64).clamp(64, 4096));
        for mut r in recvs {
            let tx = agg_tx.clone();
            tokio::spawn(async move {
                loop {
                    match r.recv().await {
                        Ok(d) => {
                            if tx.send(Ok(d)).await.is_err() {
                                break;
                            }
                        }
                        // Lagged 也要上报，调用方据此累加「谁在丢数据」的指标
                        Err(e @ tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                            if tx.send(Err(e)).await.is_err() {
                                break;
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
            });
        }
        drop(agg_tx);

        let (tx, mut rx) = tokio::sync::oneshot::channel();
        {
            let mut c = self.north_cancel.write().await;
            c.insert(id, tx);
        }
        let sub_set = subscription_set(subs);
        let metrics = self.data_flow_metrics.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut rx => break,
                    maybe = agg_rx.recv() => match maybe {
                        // 所有分区通道都已关闭（订阅被清空）
                        None => break,
                        Some(Ok(data)) => {
                            metrics.north_received.fetch_add(1, Ordering::Relaxed);
                            let key = (data.node_id, data.group_id);
                            if sub_set.contains(&key) {
                                metrics.north_forwarded.fetch_add(1, Ordering::Relaxed);
                                metrics.record_north_forwarded_tags(
                                    id,
                                    data.node_id,
                                    data.group_id,
                                    &data.values,
                                );
                                match plugin.on_group_data(id, data).await {
                                    Ok(()) => {
                                        metrics.north_on_group_data_ok.fetch_add(1, Ordering::Relaxed);
                                    }
                                    Err(e) => {
                                        metrics.north_on_group_data_err.fetch_add(1, Ordering::Relaxed);
                                        error!(node_id = ?id, "on_group_data error: {}", e);
                                    }
                                }
                            } else {
                                metrics.north_filtered.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                        Some(Err(tokio::sync::broadcast::error::RecvError::Lagged(n))) => {
                            metrics.record_lagged(id, n);
                            warn!(node_id = ?id, lagged = n, "north bus recv lagged, skipped messages");
                        }
                        Some(Err(_)) => {}
                    }
                }
            }
        });
    }

    /// 获取北向节点的订阅
    pub async fn get_north_subscriptions(&self, north_node_id: NodeId) -> Vec<GroupSubscription> {
        let t = self.subscriptions.read().await;
        t.get(&north_node_id).cloned().unwrap_or_default()
    }

    /// 从所有北向订阅中移除对指定南向节点的引用（删除南向节点时调用）
    async fn remove_subscriptions_ref_south(&self, south_node_id: NodeId) {
        let mut t = self.subscriptions.write().await;
        for subs in t.values_mut() {
            subs.retain(|s| s.south_node_id != south_node_id);
        }
    }

    /// 从所有北向订阅中移除对指定 (南向节点, 组) 的引用（删除南向组时调用）
    async fn remove_subscriptions_ref_group(
        &self,
        south_node_id: NodeId,
        group_id: gateway_sdk::GroupId,
    ) {
        let mut t = self.subscriptions.write().await;
        for subs in t.values_mut() {
            subs.retain(|s| !(s.south_node_id == south_node_id && s.group_id == group_id));
        }
    }

    /// 启动节点（南向：轮询；北向：订阅总线并转发）
    #[instrument(skip(self))]
    pub async fn node_start(&self, id: NodeId) -> Result<(), String> {
        let node = self.store.node_get(id).ok_or("node not found")?;
        let plugin_name = node.config.plugin_name.clone();

        match node.kind() {
            NodeKind::South => {
                let plugin = self
                    .south_plugin(&plugin_name)
                    .ok_or("south plugin not found")?;
                plugin.start(id).await.map_err(|e| e.to_string())?;
                log::info(id, "南向节点启动 运行中 连接中");
                {
                    let mut st = self.south_connection_status.write().await;
                    st.insert(
                        id,
                        SouthConnectionState {
                            connected: false,
                            last_error: None,
                        },
                    );
                }
                // 若 Store 中尚无该节点的 group，从插件同步默认 groups/tags
                let groups = self.store.groups_by_node(id);
                if groups.is_empty() {
                    let gs = plugin.list_groups(id).await.map_err(|e| e.to_string())?;
                    for g in &gs {
                        self.store.group_insert(id, g.clone());
                    }
                    for g in &gs {
                        let ts = plugin
                            .list_tags(id, g.id)
                            .await
                            .map_err(|e| e.to_string())?;
                        for t in ts {
                            self.store.tag_insert(id, t);
                        }
                    }
                }
                let (cancel_tx, _) = tokio::sync::broadcast::channel(1);
                {
                    let mut c = self.south_cancel.write().await;
                    c.insert(id, cancel_tx.clone());
                }
                // 采集调度：**每个运行中的南向节点一个任务**（而不是每个组一个）。
                // 任务按各组自己的周期固定节拍触发、受全局并发上限约束，并在每个节拍
                // 重新同步组集合 —— 因此运行中新增/修改/删除组会立刻生效，无需重启节点。
                self.spawn_south_scheduler(id, plugin.clone(), cancel_tx.subscribe());
            }
            NodeKind::North => {
                let plugin = self
                    .north_plugin(&plugin_name)
                    .ok_or("north plugin not found")?;
                plugin.start(id).await.map_err(|e| e.to_string())?;
                let subs = self.get_north_subscriptions(id).await;
                plugin
                    .set_subscriptions(id, &subs)
                    .await
                    .map_err(|e| e.to_string())?;
                self.spawn_north_consumer(id, plugin, &subs).await;
            }
        }
        self.store.node_update_state(id, NodeState::Running);
        log::info(id, "node started");
        Ok(())
    }

    /// 停止节点
    #[instrument(skip(self))]
    pub async fn node_stop(&self, id: NodeId) -> Result<(), String> {
        let node = self.store.node_get(id).ok_or("node not found")?;
        let plugin_name = node.config.plugin_name.clone();
        let plugin_south = self.south_plugin(&plugin_name);
        let plugin_north = self.north_plugin(&plugin_name);

        if let Some(p) = plugin_south {
            log::info(id, "南向节点停止 连接断开");
            {
                let mut st = self.south_connection_status.write().await;
                st.insert(
                    id,
                    SouthConnectionState {
                        connected: false,
                        last_error: None,
                    },
                );
            }
            let mut c = self.south_cancel.write().await;
            if let Some(tx) = c.remove(&id) {
                let _ = tx.send(());
            }
            drop(c);
            let _ = p.stop(id).await;
        }
        if let Some(p) = plugin_north {
            let mut c = self.north_cancel.write().await;
            if let Some(tx) = c.remove(&id) {
                let _ = tx.send(());
            }
            drop(c);
            let _ = p.stop(id).await;
        }
        self.store.node_update_state(id, NodeState::Stopped);
        log::info(id, "node stopped");
        Ok(())
    }

    /// 构建持久化快照
    pub async fn build_snapshot(&self) -> Snapshot {
        let nodes = self.store.nodes_list();
        let mut groups = Vec::new();
        let mut tags = Vec::new();
        for n in &nodes {
            let nid = n.id();
            for g in self.store.groups_by_node(nid) {
                groups.push((nid, g.clone()));
                for t in self.store.tags_by_group(nid, g.id) {
                    tags.push((nid, t));
                }
            }
        }
        let mut subscriptions = Vec::new();
        {
            let t = self.subscriptions.read().await;
            for (nid, subs) in t.iter() {
                subscriptions.push((*nid, subs.clone()));
            }
        }
        let rules = self.store.rules_list();
        let policies = self.store.policies_list();
        crate::persist::build_snapshot(nodes, groups, tags, subscriptions, rules, policies)
    }

    /// 应用持久化快照（清空后填入），并对每个节点调用插件 open、init。
    /// 恢复保存的运行状态：快照中为 Running 的节点会自动启动。
    pub async fn apply_snapshot(&self, s: &Snapshot) {
        self.store.clear();
        apply_to_store(&self.store, s);
        let mut t = self.subscriptions.write().await;
        t.clear();
        for (nid, subs) in &s.subscriptions {
            t.insert(*nid, subs.clone());
        }
        drop(t);
        for n in &s.nodes {
            let nid = n.id();
            let plugin_name = n.config.plugin_name.clone();
            let config = n.config.config.clone();
            match n.kind() {
                gateway_sdk::NodeKind::South => {
                    let Some(p) = self.south_plugin(&plugin_name) else {
                        continue;
                    };
                    if let Err(e) = p.open(nid, config.clone()).await {
                        tracing::warn!(node_id = ?nid, "apply_snapshot open failed: {}", e);
                        self.store.node_update_state(nid, NodeState::Stopped);
                        continue;
                    }
                    if let Err(e) = p.init(nid).await {
                        tracing::warn!(node_id = ?nid, "apply_snapshot init failed: {}", e);
                        let _ = p.close(nid).await;
                        self.store.node_update_state(nid, NodeState::Stopped);
                    }
                }
                gateway_sdk::NodeKind::North => {
                    let Some(p) = self.north_plugin(&plugin_name) else {
                        continue;
                    };
                    if let Err(e) = p.open(nid, config.clone()).await {
                        tracing::warn!(node_id = ?nid, "apply_snapshot open failed: {}", e);
                        self.store.node_update_state(nid, NodeState::Stopped);
                        continue;
                    }
                    if let Err(e) = p.init(nid).await {
                        tracing::warn!(node_id = ?nid, "apply_snapshot init failed: {}", e);
                        let _ = p.close(nid).await;
                        self.store.node_update_state(nid, NodeState::Stopped);
                    }
                }
            }
        }
        // 恢复运行状态：store 中为 Running 的节点（open/init 成功）自动启动
        let to_start: Vec<NodeId> = self
            .store
            .nodes_list()
            .into_iter()
            .filter(|n| n.state == NodeState::Running)
            .map(|n| n.id())
            .collect();
        for nid in to_start {
            if let Err(e) = self.node_start(nid).await {
                tracing::warn!(node_id = ?nid, "apply_snapshot node_start failed: {}", e);
                self.store.node_update_state(nid, NodeState::Stopped);
            }
        }
    }

    // ---------- Groups / Tags ----------
    /// 添加组。同节点下组名唯一，重复返回 Err。
    pub fn group_add(&self, node_id: NodeId, g: Group) -> Result<(), String> {
        if self.store.group_get_by_name(node_id, &g.name).is_some() {
            return Err(format!("group name already exists: {}", g.name));
        }
        self.store.group_insert(node_id, g);
        Ok(())
    }

    pub fn group_get(&self, node_id: NodeId, group_id: gateway_sdk::GroupId) -> Option<Group> {
        self.store.group_get(node_id, group_id)
    }

    /// 更新组字段。同节点下组名唯一，若改 name 则与现有一致或未被占用。
    pub fn group_update(
        &self,
        node_id: NodeId,
        group_id: gateway_sdk::GroupId,
        name: Option<String>,
        interval_ms: Option<u64>,
        description: Option<Option<String>>,
    ) -> Result<(), String> {
        if let Some(ref n) = name {
            if let Some(existing) = self.store.group_get_by_name(node_id, n) {
                if existing.id != group_id {
                    return Err(format!("group name already exists: {}", n));
                }
            }
        }
        self.store
            .group_update(node_id, group_id, name, interval_ms, description);
        Ok(())
    }

    /// 删除南向组。会同步清理所有北向订阅中对该组的引用。
    pub async fn group_remove(
        &self,
        node_id: NodeId,
        group_id: gateway_sdk::GroupId,
    ) -> Option<Group> {
        let g = self.store.group_remove(node_id, group_id);
        if g.is_some() {
            self.remove_subscriptions_ref_group(node_id, group_id).await;
            self.data_flow_metrics.forget_group(node_id, group_id);
            self.bus.forget_group(&(node_id, group_id));
            // 过滤状态与窗口状态按组清零
            crate::filters::forget_group(node_id, group_id);
        }
        g
    }

    pub fn groups_by_node(&self, node_id: NodeId) -> Vec<Group> {
        self.store.groups_by_node(node_id)
    }

    /// 校验点位（南向节点委托插件 validate_tag）。通过后再插入。
    pub async fn validate_tag(&self, node_id: NodeId, tag: &Tag) -> Result<(), String> {
        let node = self.store.node_get(node_id).ok_or("node not found")?;
        if node.kind() != gateway_sdk::NodeKind::South {
            return Ok(());
        }
        let p = self
            .south_plugin(node.plugin_name())
            .ok_or("south plugin not found")?;
        p.validate_tag(node_id, tag)
            .await
            .map_err(|e| e.to_string())
    }

    pub fn tag_add(&self, node_id: NodeId, t: Tag) {
        self.store.tag_insert(node_id, t);
    }

    /// 获取单个 Tag（需属于该节点）。用于 GET /nodes/:id/tags/:tid。
    pub fn tag_get(&self, node_id: NodeId, tag_id: gateway_sdk::TagId) -> Option<Tag> {
        let (nid, _) = self.store.tag_location_get(tag_id)?;
        if nid != node_id {
            return None;
        }
        self.store.tag_get(tag_id)
    }

    /// 更新标签字段。同组内标签名唯一；南向节点可先校验点位再更新。
    #[allow(clippy::too_many_arguments)]
    pub async fn tag_update_validated(
        &self,
        node_id: NodeId,
        tag_id: gateway_sdk::TagId,
        name: Option<String>,
        address: Option<String>,
        attr: Option<gateway_sdk::TagAttr>,
        data_type: Option<Option<String>>,
        description: Option<Option<String>>,
    ) -> Result<(), String> {
        let t = self.store.tag_get(tag_id).ok_or("tag not found")?;
        let (nid, gid) = self.store.tag_location_get(tag_id).ok_or("tag not found")?;
        if nid != node_id {
            return Err("tag does not belong to this node".to_string());
        }
        if let Some(ref n) = name {
            if let Some(existing) = self.store.tag_get_by_name(node_id, gid, n) {
                if existing.id != tag_id {
                    return Err(format!("tag name already exists in group: {}", n));
                }
            }
        }
        let node = self.store.node_get(node_id).ok_or("node not found")?;
        if node.kind() == gateway_sdk::NodeKind::South {
            let mut updated = t.clone();
            if let Some(n) = name.clone() {
                updated.name = n;
            }
            if let Some(a) = address.clone() {
                updated.address = a;
            }
            if let Some(a) = attr {
                updated.attr = a;
            }
            if let Some(dt) = data_type.clone() {
                updated.data_type = dt;
            }
            if let Some(d) = description.clone() {
                updated.description = d;
            }
            self.validate_tag(node_id, &updated).await?;
        }
        self.store
            .tag_update(tag_id, name, address, attr, data_type, description);
        Ok(())
    }

    /// 校验通过后再添加 tag。同组内标签名唯一，重复返回 Err。
    pub async fn tag_add_validated(&self, node_id: NodeId, t: Tag) -> Result<(), String> {
        if self
            .store
            .tag_get_by_name(node_id, t.group_id, &t.name)
            .is_some()
        {
            return Err(format!("tag name already exists in group: {}", t.name));
        }
        self.validate_tag(node_id, &t).await?;
        self.store.tag_insert(node_id, t);
        Ok(())
    }

    /// 南向按需读 Tag（不依赖轮询周期）。复用 poll_group 按组采集后过滤。
    pub async fn read_tags(
        &self,
        node_id: NodeId,
        tag_ids: &[TagId],
    ) -> Result<Vec<(TagId, DataValue)>, String> {
        let node = self.store.node_get(node_id).ok_or("node not found")?;
        if node.kind() != gateway_sdk::NodeKind::South {
            return Err("read_tags only for south nodes".to_string());
        }
        let p = self
            .south_plugin(node.plugin_name())
            .ok_or("south plugin not found")?;
        let tag_set: std::collections::HashSet<TagId> = tag_ids.iter().copied().collect();
        let mut tags_to_read: Vec<Tag> = Vec::new();
        for tid in tag_ids {
            let Some(t) = self.store.tag_get(*tid) else {
                continue;
            };
            let Some(loc) = self.store.tag_location_get(*tid) else {
                continue;
            };
            if loc.0 != node_id {
                continue;
            }
            tags_to_read.push(t);
        }
        let by_group: std::collections::HashMap<gateway_sdk::GroupId, Vec<Tag>> = {
            let mut m: std::collections::HashMap<gateway_sdk::GroupId, Vec<Tag>> =
                std::collections::HashMap::new();
            for t in tags_to_read {
                m.entry(t.group_id).or_default().push(t);
            }
            m
        };
        let mut out: Vec<(TagId, DataValue)> = Vec::new();
        for (gid, tags) in by_group.into_iter() {
            let values = p
                .poll_group(node_id, gid, &tags)
                .await
                .map_err(|e| e.to_string())?;
            for (tid, val) in values {
                if tag_set.contains(&tid) {
                    out.push((tid, val));
                }
            }
        }
        Ok(out)
    }

    pub fn tag_remove(&self, tag_id: gateway_sdk::TagId) -> Option<Tag> {
        self.store.tag_remove(tag_id)
    }

    pub fn tags_by_group(&self, node_id: NodeId, group_id: gateway_sdk::GroupId) -> Vec<Tag> {
        self.store.tags_by_group(node_id, group_id)
    }

    /// 修改节点插件配置（不删节点）。
    /// 南向：若节点在运行，修改后自动重启轮询任务以使新配置生效。
    /// 北向：插件 setting 内部处理重连（如 MQTT 的 close+open）。
    pub async fn node_setting(&self, id: NodeId, config: PluginConfig) -> Result<(), String> {
        let node = self.store.node_get(id).ok_or("node not found")?;
        let plugin_name = node.config.plugin_name.clone();
        let was_running = node.state == NodeState::Running;
        let plugin_south = self.south_plugin(&plugin_name);
        let plugin_north = self.north_plugin(&plugin_name);

        // 热改配置与创建走同一套 Schema 校验：否则「运行期修改」就成了绕过校验的后门。
        // 必须在任何状态变更（停节点 / 写配置）之前完成，失败时节点保持原状。
        match node.kind() {
            gateway_sdk::NodeKind::South => {
                if let Some(schema) = plugin_south.as_ref().and_then(|p| p.config_schema()) {
                    schema.validate_config(&config).map_err(|e| e.to_string())?;
                }
            }
            gateway_sdk::NodeKind::North => {
                if let Some(schema) = plugin_north.as_ref().and_then(|p| p.config_schema()) {
                    schema.validate_config(&config).map_err(|e| e.to_string())?;
                }
            }
        }

        match node.kind() {
            gateway_sdk::NodeKind::South => {
                let p = plugin_south.ok_or("south plugin not found")?;
                if was_running {
                    let _ = self.node_stop(id).await;
                }
                if let Err(e) = p.setting(id, config.clone()).await {
                    // setting 失败不能把一个原本在运行的节点留在停止态
                    if was_running {
                        if let Err(start_err) = self.node_start(id).await {
                            tracing::warn!(
                                node_id = ?id,
                                "restore node after failed setting also failed: {}",
                                start_err
                            );
                        }
                    }
                    return Err(e.to_string());
                }
                self.store.node_update_config(id, config.clone());
                if was_running {
                    self.node_start(id).await?;
                }
            }
            gateway_sdk::NodeKind::North => {
                let p = plugin_north.ok_or("north plugin not found")?;
                p.setting(id, config.clone())
                    .await
                    .map_err(|e| e.to_string())?;
                self.store.node_update_config(id, config);
            }
        }
        Ok(())
    }

    /// 南向写 Tag。根据 TagId 查 Tag 后以 (Tag, DataValue) 交给插件以便解析地址。
    pub async fn write_tags(
        &self,
        node_id: NodeId,
        values: &[(gateway_sdk::TagId, gateway_sdk::types::DataValue)],
    ) -> Result<(), String> {
        let node = self.store.node_get(node_id).ok_or("node not found")?;
        if node.kind() != gateway_sdk::NodeKind::South {
            return Err("write_tags only for south nodes".to_string());
        }
        let mut tag_values: Vec<(Tag, gateway_sdk::types::DataValue)> =
            Vec::with_capacity(values.len());
        for (tid, val) in values {
            let Some(tag) = self.store.tag_get(*tid) else {
                return Err(format!("tag {} not found", tid.0));
            };
            let (nid, _) = self
                .store
                .tag_location_get(*tid)
                .ok_or("tag location not found")?;
            if nid != node_id {
                return Err(format!("tag {} does not belong to node", tid.0));
            }
            tag_values.push((tag, val.clone()));
        }
        let p = self
            .south_plugin(node.plugin_name())
            .ok_or("south plugin not found")?;
        p.write_tags(node_id, &tag_values)
            .await
            .map_err(|e| e.to_string())
    }
}

// ---------- 采集调度 ----------

impl Manager {
    /// 启动（或重启）某南向节点的采集调度任务。
    ///
    /// 取消信号走 `south_cancel` 广播（`node_stop` 发送），因此每个节点只需一个
    /// 调度任务，而不是每个组一个：千级组不再对应千个长驻 task。
    fn spawn_south_scheduler(
        &self,
        id: NodeId,
        plugin: Arc<dyn SouthPlugin>,
        cancel_rx: tokio::sync::broadcast::Receiver<()>,
    ) {
        let store = self.store.clone();
        let bus = self.bus.clone();
        let metrics = self.data_flow_metrics.clone();
        let south_conn = self.south_connection_status.clone();
        let permits = self.poll_permits.clone();
        let last_values = self.last_values.clone();
        tokio::spawn(async move {
            south_scheduler_loop(
                id,
                plugin,
                store,
                bus,
                metrics,
                south_conn,
                last_values,
                permits,
                cancel_rx,
            )
            .await;
        });
    }
}

/// 单个南向节点的采集调度循环。
///
/// 设计要点：
/// 1. **固定节拍**：下一轮时间按 `上次到期时刻 + 周期` 推进，而不是「等采集完再 sleep」，
///    否则 `周期 = 采集耗时 + interval`，周期会被设备延迟不断放大；
/// 2. **可动态变组**：每个节拍都从 Store 重新读取组集合，运行中新增/删除组立即生效
///    （旧实现只在 node_start 时为当时的组创建任务，新增组要重启节点才会被采集）；
/// 3. **限并发**：本批到期的组并发采集，但全局同时进行的 poll 受信号量约束；
/// 4. **超期可观测**：一批采集耗时超过其中最小周期时计入 `south_poll_overrun`。
#[allow(clippy::too_many_arguments)]
async fn south_scheduler_loop(
    id: NodeId,
    plugin: Arc<dyn SouthPlugin>,
    store: Store,
    bus: Bus,
    metrics: Arc<DataFlowMetrics>,
    south_conn: Arc<RwLock<HashMap<NodeId, SouthConnectionState>>>,
    last_values: Arc<dashmap::DashMap<(NodeId, TagId), LastValue>>,
    permits: Arc<tokio::sync::Semaphore>,
    mut cancel_rx: tokio::sync::broadcast::Receiver<()>,
) {
    // group_id -> 下次到期时刻
    let mut next_due: HashMap<gateway_sdk::GroupId, Instant> = HashMap::new();
    // 空闲等待上限：保证新增组 / 取消信号能被及时感知
    let idle_max = Duration::from_millis(200);

    loop {
        // 1) 同步组集合
        let groups = store.groups_by_node(id);
        let live: HashSet<gateway_sdk::GroupId> = groups.iter().map(|g| g.id).collect();
        next_due.retain(|gid, _| live.contains(gid));
        let now = Instant::now();
        for g in &groups {
            next_due.entry(g.id).or_insert(now); // 新组：立即采集一次
        }

        // 2) 立即处理本轮到期的组
        let due: Vec<(gateway_sdk::GroupId, u64)> = next_due
            .iter()
            .filter(|(_, t)| **t <= now)
            .map(|(gid, _)| {
                let interval = groups
                    .iter()
                    .find(|g| g.id == *gid)
                    .map(|g| g.interval_ms.max(MIN_POLL_INTERVAL_MS))
                    .unwrap_or(MIN_POLL_INTERVAL_MS);
                (*gid, interval)
            })
            .collect();

        if due.is_empty() {
            let earliest = next_due
                .values()
                .min()
                .copied()
                .unwrap_or(now + idle_max)
                .saturating_duration_since(now)
                .min(idle_max);
            tokio::select! {
                _ = cancel_rx.recv() => break,
                _ = tokio::time::sleep(earliest.max(Duration::from_millis(1))) => {}
            }
            continue;
        }

        // 3) 固定节拍推进下一轮时间（不因本次耗时后移）
        for (gid, interval) in &due {
            next_due.insert(*gid, now + Duration::from_millis(*interval));
        }
        let min_interval = due.iter().map(|(_, i)| *i).min().unwrap_or(u64::MAX);

        // 4) 并发采集（全局并发上限）
        let tick_start = Instant::now();
        let mut set = tokio::task::JoinSet::new();
        for (gid, _) in due {
            let permit = tokio::select! {
                _ = cancel_rx.recv() => return,
                p = permits.clone().acquire_owned() => p,
            };
            let Ok(permit) = permit else { return };
            let store = store.clone();
            let bus = bus.clone();
            let metrics = metrics.clone();
            let south_conn = south_conn.clone();
            let last_values = last_values.clone();
            let plugin = plugin.clone();
            set.spawn(async move {
                let _permit = permit; // 持有到本次采集结束
                poll_group_once(
                    &store,
                    &bus,
                    &metrics,
                    &south_conn,
                    &last_values,
                    &plugin,
                    id,
                    gid,
                )
                .await;
            });
        }

        // 5) 等待本轮结束（可被取消打断）
        loop {
            tokio::select! {
                _ = cancel_rx.recv() => {
                    set.abort_all();
                    return;
                }
                joined = set.join_next() => {
                    if joined.is_none() {
                        break;
                    }
                }
            }
        }

        // 6) 超期观测：一批耗时超过最小周期，说明设备或并发是瓶颈
        if min_interval != u64::MAX && tick_start.elapsed() > Duration::from_millis(min_interval) {
            metrics.south_poll_overrun.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// 采集一个组并发布到总线（原「每组一个任务」循环体的等价实现）。
#[allow(clippy::too_many_arguments)]
async fn poll_group_once(
    store: &Store,
    bus: &Bus,
    metrics: &Arc<DataFlowMetrics>,
    south_conn: &Arc<RwLock<HashMap<NodeId, SouthConnectionState>>>,
    last_values: &Arc<dashmap::DashMap<(NodeId, TagId), LastValue>>,
    plugin: &Arc<dyn SouthPlugin>,
    id: NodeId,
    gid: gateway_sdk::GroupId,
) {
    let tags = store.tags_by_group(id, gid);
    let Some(grp) = store.group_get(id, gid) else {
        return; // 组已被删除
    };
    // 周期下限做服务端钳制：防止 interval_ms=0 造成忙循环打满 CPU
    let interval_ms = grp.interval_ms.max(MIN_POLL_INTERVAL_MS);
    // 采集超时：慢设备不应挂住整个轮询任务（多数插件无内部超时）
    let poll_timeout_ms = interval_ms.saturating_mul(3).max(5_000);
    let poll_result = tokio::time::timeout(
        Duration::from_millis(poll_timeout_ms),
        plugin.poll_group(id, gid, &tags),
    )
    .await;

    match poll_result {
        Err(_) => {
            let err_msg = format!("poll_group timeout after {} ms", poll_timeout_ms);
            metrics.south_poll_timeout.fetch_add(1, Ordering::Relaxed);
            warn!(node_id = ?id, group_id = ?gid, "{}", err_msg);
            let mut st = south_conn.write().await;
            st.insert(
                id,
                SouthConnectionState {
                    connected: false,
                    last_error: Some(err_msg),
                },
            );
        }
        Ok(Ok(values)) => {
            // 规则求值：无启用规则时零开销（store 查询后立即返回）
            let (fired, failed) =
                crate::rules::evaluate_and_fire(store, plugin, id, gid, &values).await;
            if fired > 0 {
                metrics.rules_fired.fetch_add(fired, Ordering::Relaxed);
            }
            if failed > 0 {
                metrics
                    .rules_action_err
                    .fetch_add(failed, Ordering::Relaxed);
            }

            // 记录最近值供管理台展示（只写内存缓存，不产生额外设备访问）
            let ts_ms = Utc::now().timestamp_millis();
            for (tid, v) in &values {
                last_values.insert(
                    (id, *tid),
                    LastValue {
                        value: v.clone(),
                        ts_ms,
                    },
                );
            }

            // 数据面过滤：死区/变化上报/滑动窗口聚合（插入点固定：规则之后、publish 之前）
            let outcome = crate::filters::apply(store, id, gid, &values, ts_ms as u64);
            if outcome.suppressed_msgs > 0 {
                metrics
                    .filters_suppressed_msgs
                    .fetch_add(1, Ordering::Relaxed);
            }
            if outcome.suppressed_tags > 0 {
                metrics
                    .filters_suppressed_tags
                    .fetch_add(outcome.suppressed_tags, Ordering::Relaxed);
            }
            if outcome.window_emitted > 0 {
                metrics.window_emitted_msgs.fetch_add(1, Ordering::Relaxed);
            }

            // 过滤后值 = 真实点位(published) + 虚拟点位(synthetic)
            let filtered_values: Vec<_> = outcome
                .published
                .iter()
                .cloned()
                .chain(outcome.synthetic.iter().cloned())
                .collect();

            let node_name = store.node_get(id).map(|n| n.config.name);
            let group_name = store.group_get(id, gid).map(|g| g.name);

            // 合并真实点位名称与虚拟点位名称
            let mut tag_names: HashMap<_, _> = filtered_values
                .iter()
                .filter_map(|(tid, _)| store.tag_get(*tid).map(|t| (*tid, t.name)))
                .collect();
            tag_names.extend(outcome.synthetic_names);
            let tag_names = if tag_names.is_empty() {
                None
            } else {
                Some(tag_names)
            };

            // 整批抑制时跳过 publish（但仍更新 last_values）
            if filtered_values.is_empty() {
                // south_published 不增加（整批被过滤），bus_no_subscribers 也不增加（未尝试投递）
                let mut st = south_conn.write().await;
                st.insert(
                    id,
                    SouthConnectionState {
                        connected: true,
                        last_error: None,
                    },
                );
                return;
            }

            let data = Arc::new(GroupData {
                node_id: id,
                group_id: gid,
                ts: Utc::now(),
                values: filtered_values,
                node_name,
                group_name,
                tag_names,
            });
            metrics.record_south_published_tags(id, gid, &data.values);
            match bus.publish(data) {
                Ok(_) => {
                    metrics.south_published.fetch_add(1, Ordering::Relaxed);
                }
                Err(_) => {
                    metrics.bus_no_subscribers.fetch_add(1, Ordering::Relaxed);
                }
            }
            let mut st = south_conn.write().await;
            st.insert(
                id,
                SouthConnectionState {
                    connected: true,
                    last_error: None,
                },
            );
        }
        Ok(Err(e)) => {
            metrics.south_poll_err.fetch_add(1, Ordering::Relaxed);
            let err_msg = e.to_string();
            warn!(node_id = ?id, group_id = ?gid, "poll_group error: {}", e);
            let mut st = south_conn.write().await;
            st.insert(
                id,
                SouthConnectionState {
                    connected: false,
                    last_error: Some(err_msg),
                },
            );
        }
    }
}

impl Default for Manager {
    fn default() -> Self {
        Self::new()
    }
}
