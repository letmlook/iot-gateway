//! 路由核心：插件管理、节点启停、消息路由。

use crate::bus::{Bus, SubscriptionTable, subscription_set};
use crate::data_flow::DataFlowMetrics;
use crate::node::Node;
use crate::store::Store;
use gateway_sdk::{
    Group, GroupData, GroupSubscription, NorthPlugin, SouthPlugin, PluginConfig,
    NodeId, NodeKind, NodeState, PluginInfo, Tag,
};
use gateway_sdk::log;
use gateway_sdk::types::{DataValue, TagId};
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{instrument, warn, error};
use chrono::Utc;

use crate::persist::{apply_to_store, Snapshot};

type SouthRegistry = HashMap<String, Arc<dyn SouthPlugin>>;
type NorthRegistry = HashMap<String, Arc<dyn NorthPlugin>>;

/// 南向节点连接状态（由最近一次采集结果推断：成功为已连接，失败为异常）
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct SouthConnectionState {
    pub connected: bool,
    pub last_error: Option<String>,
}

/// 路由核心
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
}

impl Manager {
    pub fn new() -> Self {
        Self {
            bus: Bus::new(),
            store: Store::new(),
            subscriptions: Arc::new(RwLock::new(SubscriptionTable::default())),
            south_plugins: SouthRegistry::new(),
            north_plugins: NorthRegistry::new(),
            south_cancel: Arc::new(RwLock::new(HashMap::new())), // broadcast senders
            north_cancel: Arc::new(RwLock::new(HashMap::new())),
            data_flow_metrics: Arc::new(DataFlowMetrics::new()),
            south_connection_status: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// 南向节点连接状态（由最近一次采集成功/失败推断），供 API 与前端展示
    pub async fn south_connection_status(&self, node_id: NodeId) -> Option<SouthConnectionState> {
        self.south_connection_status.read().await.get(&node_id).cloned()
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
                p.open(id, config.clone()).await.map_err(|e| e.to_string())?;
                if let Err(e) = p.init(id).await {
                    let _ = p.close(id).await;
                    let _ = self.store.node_remove(id);
                    return Err(e.to_string());
                }
            }
            gateway_sdk::NodeKind::North => {
                let p = plugin_north.as_ref().ok_or("north plugin not found")?;
                p.open(id, config.clone()).await.map_err(|e| e.to_string())?;
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
        let Some(n) = name else { return Ok(()); };
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
        }
        self.store.node_remove(id)
    }

    pub fn nodes_list(&self) -> Vec<Node> {
        self.store.nodes_list()
    }

    /// 设置北向节点的订阅。若该北向节点正在运行，会重启其总线消费任务以使新订阅生效。
    /// 会过滤掉无效的订阅项（south_node_id 或 group_id 不存在）。
    #[instrument(skip(self))]
    pub async fn set_north_subscriptions(&self, north_node_id: NodeId, subs: Vec<GroupSubscription>) {
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
        let Some(plugin) = self.north_plugin(&plugin_name) else { return };
        let mut c = self.north_cancel.write().await;
        if let Some(tx) = c.remove(&north_node_id) {
            let _ = tx.send(());
        }
        drop(c);
        plugin.set_subscriptions(north_node_id, &subs).await.ok();
        let mut recv = self.bus.subscribe();
        let (tx, mut rx) = tokio::sync::oneshot::channel();
        {
            let mut c = self.north_cancel.write().await;
            c.insert(north_node_id, tx);
        }
        let sub_set = subscription_set(&subs);
        let id = north_node_id;
        let metrics = self.data_flow_metrics.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut rx => break,
                    r = recv.recv() => match r {
                        Ok(data) => {
                            metrics.north_received.fetch_add(1, Ordering::Relaxed);
                            let key = (data.node_id, data.group_id);
                            if sub_set.contains(&key) {
                                metrics.north_forwarded.fetch_add(1, Ordering::Relaxed);
                                metrics.record_north_forwarded_tags(id, data.node_id, data.group_id, &data.values);
                                match plugin.on_group_data(id, data).await {
                                    Ok(()) => { metrics.north_on_group_data_ok.fetch_add(1, Ordering::Relaxed); }
                                    Err(e) => {
                                        metrics.north_on_group_data_err.fetch_add(1, Ordering::Relaxed);
                                        error!(node_id = ?id, "on_group_data error: {}", e);
                                    }
                                }
                            } else {
                                metrics.north_filtered.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                            metrics.north_lagged.fetch_add(n, Ordering::Relaxed);
                            warn!(node_id = ?id, lagged = n, "north bus recv lagged, skipped messages");
                        }
                        Err(_) => {}
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
                let plugin = self.south_plugin(&plugin_name).ok_or("south plugin not found")?;
                plugin.start(id).await.map_err(|e| e.to_string())?;
                log::info(id, "南向节点启动 运行中 连接中");
                {
                    let mut st = self.south_connection_status.write().await;
                    st.insert(id, SouthConnectionState { connected: false, last_error: None });
                }
                // 若 Store 中尚无该节点的 group，从插件同步默认 groups/tags
                let groups = self.store.groups_by_node(id);
                if groups.is_empty() {
                    let gs = plugin.list_groups(id).await.map_err(|e| e.to_string())?;
                    for g in &gs {
                        self.store.group_insert(id, g.clone());
                    }
                    for g in &gs {
                        let ts = plugin.list_tags(id, g.id).await.map_err(|e| e.to_string())?;
                        for t in ts {
                            self.store.tag_insert(id, t);
                        }
                    }
                }
                let groups = self.store.groups_by_node(id);
                let (cancel_tx, _) = tokio::sync::broadcast::channel(1);
                {
                    let mut c = self.south_cancel.write().await;
                    c.insert(id, cancel_tx.clone());
                }
                let metrics = self.data_flow_metrics.clone();
                let south_conn = self.south_connection_status.clone();
                for g in groups {
                    let store = self.store.clone();
                    let bus = self.bus.clone();
                    let plugin = plugin.clone();
                    let cancel_rx = cancel_tx.subscribe();
                    let metrics = metrics.clone();
                    let south_conn = south_conn.clone();
                    let gid = g.id;
                    tokio::spawn(async move {
                        let mut cancel_rx = cancel_rx;
                        loop {
                            let tags = store.tags_by_group(id, gid);
                            match plugin.poll_group(id, gid, &tags).await {
                                Ok(values) => {
                                    let _point_count = values.len();
                                    let node_name = store.node_get(id).map(|n| n.config.name);
                                    let group_name = store.group_get(id, gid).map(|g| g.name);
                                    let tag_names: std::collections::HashMap<_, _> = values
                                        .iter()
                                        .filter_map(|(tid, _)| {
                                            store.tag_get(*tid).map(|t| (*tid, t.name))
                                        })
                                        .collect();
                                    let tag_names = if tag_names.is_empty() {
                                        None
                                    } else {
                                        Some(tag_names)
                                    };
                                    let data = Arc::new(GroupData {
                                        node_id: id,
                                        group_id: gid,
                                        ts: Utc::now(),
                                        values,
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
                                    // let grp_name = store.group_get(id, gid).map(|g| g.name).unwrap_or_else(|| gid.0.to_string());
                                    // log::info(id, format!("采集成功 组[{}] 全点 {} 个 连接正常", grp_name, point_count));
                                    let mut st = south_conn.write().await;
                                    st.insert(id, SouthConnectionState { connected: true, last_error: None });
                                }
                                Err(e) => {
                                    // let grp_name = store.group_get(id, gid).map(|g| g.name).unwrap_or_else(|| gid.0.to_string());
                                    let err_msg = e.to_string();
                                    // log::warn(id, format!("采集失败 组[{}]: {} 连接异常", grp_name, err_msg));
                                    warn!(node_id = ?id, group_id = ?gid, "poll_group error: {}", e);
                                    let mut st = south_conn.write().await;
                                    st.insert(id, SouthConnectionState { connected: false, last_error: Some(err_msg) });
                                }
                            }
                            let interval_ms = match store.group_get(id, gid) {
                                Some(grp) => grp.interval_ms,
                                None => break,
                            };
                            tokio::select! {
                                _ = cancel_rx.recv() => break,
                                _ = tokio::time::sleep(tokio::time::Duration::from_millis(interval_ms)) => {}
                            }
                        }
                    });
                }
            }
            NodeKind::North => {
                let plugin = self.north_plugin(&plugin_name).ok_or("north plugin not found")?;
                plugin.start(id).await.map_err(|e| e.to_string())?;
                let subs = self.get_north_subscriptions(id).await;
                plugin.set_subscriptions(id, &subs).await.map_err(|e| e.to_string())?;
                let mut recv = self.bus.subscribe();
                let (tx, mut rx) = tokio::sync::oneshot::channel();
                {
                    let mut c = self.north_cancel.write().await;
                    c.insert(id, tx);
                }
                let sub_set = subscription_set(&subs);
                let metrics = self.data_flow_metrics.clone();
                tokio::spawn(async move {
                    loop {
                        tokio::select! {
                            _ = &mut rx => break,
                            r = recv.recv() => match r {
                                Ok(data) => {
                                    metrics.north_received.fetch_add(1, Ordering::Relaxed);
                                    let key = (data.node_id, data.group_id);
                                    if sub_set.contains(&key) {
                                        metrics.north_forwarded.fetch_add(1, Ordering::Relaxed);
                                        metrics.record_north_forwarded_tags(id, data.node_id, data.group_id, &data.values);
                                        match plugin.on_group_data(id, data).await {
                                            Ok(()) => { metrics.north_on_group_data_ok.fetch_add(1, Ordering::Relaxed); }
                                            Err(e) => {
                                                metrics.north_on_group_data_err.fetch_add(1, Ordering::Relaxed);
                                                error!(node_id = ?id, "on_group_data error: {}", e);
                                            }
                                        }
                                    } else {
                                        metrics.north_filtered.fetch_add(1, Ordering::Relaxed);
                                    }
                                }
                                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                                    metrics.north_lagged.fetch_add(n, Ordering::Relaxed);
                                    warn!(node_id = ?id, lagged = n, "north bus recv lagged, skipped messages");
                                }
                                Err(_) => {}
                            }
                        }
                    }
                });
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
                st.insert(id, SouthConnectionState { connected: false, last_error: None });
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
        crate::persist::build_snapshot(nodes, groups, tags, subscriptions)
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
                    let Some(p) = self.south_plugin(&plugin_name) else { continue };
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
                    let Some(p) = self.north_plugin(&plugin_name) else { continue };
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
        self.store.group_update(node_id, group_id, name, interval_ms, description);
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
        p.validate_tag(node_id, tag).await.map_err(|e| e.to_string())
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
        self.store.tag_update(tag_id, name, address, attr, data_type, description);
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
            let Some(t) = self.store.tag_get(*tid) else { continue };
            let Some(loc) = self.store.tag_location_get(*tid) else { continue };
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
        match node.kind() {
            gateway_sdk::NodeKind::South => {
                let p = plugin_south.ok_or("south plugin not found")?;
                if was_running {
                    let _ = self.node_stop(id).await;
                }
                p.setting(id, config.clone()).await.map_err(|e| e.to_string())?;
                self.store.node_update_config(id, config.clone());
                if was_running {
                    self.node_start(id).await?;
                }
            }
            gateway_sdk::NodeKind::North => {
                let p = plugin_north.ok_or("north plugin not found")?;
                p.setting(id, config.clone()).await.map_err(|e| e.to_string())?;
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
        let mut tag_values: Vec<(Tag, gateway_sdk::types::DataValue)> = Vec::with_capacity(values.len());
        for (tid, val) in values {
            let Some(tag) = self.store.tag_get(*tid) else {
                return Err(format!("tag {} not found", tid.0));
            };
            let (nid, _) = self.store.tag_location_get(*tid).ok_or("tag location not found")?;
            if nid != node_id {
                return Err(format!("tag {} does not belong to node", tid.0));
            }
            tag_values.push((tag, val.clone()));
        }
        let p = self
            .south_plugin(node.plugin_name())
            .ok_or("south plugin not found")?;
        p.write_tags(node_id, &tag_values).await.map_err(|e| e.to_string())
    }
}

impl Default for Manager {
    fn default() -> Self {
        Self::new()
    }
}
