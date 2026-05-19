//! 应用状态：Manager + Config + 可选 PluginLoader + 功能授权 + 用户存储，供 API 与持久化使用。

use crate::config::Config;
use crate::flow::FlowStore;
use crate::license::FeatureManager;
use crate::logging::NodeLogNameMap;
use crate::users::UserStore;
use crate::websocket::WsHub;
use gateway_core::{persist_save, PluginLoader, Manager};
use gateway_flow::FlowRuntime;
use std::sync::Arc;
use tracing::warn;
use uuid::Uuid;

pub struct AppState {
    pub manager: Arc<Manager>,
    pub config: Config,
    /// 功能分级：根据 license.dat 中的 features 控制免费/收费功能
    pub feature_manager: FeatureManager,
    /// 用户存储（登录、用户 CRUD）
    pub user_store: Arc<UserStore>,
    /// 节点 ID -> 节点名称，用于节点日志文件名（按名称生成）
    pub node_log_names: Option<NodeLogNameMap>,
    /// Flow 持久化存储
    pub flow_store: FlowStore,
    /// 运行中的 Flow 运行时
    pub flow_runtimes: Arc<tokio::sync::RwLock<std::collections::HashMap<Uuid, FlowRuntime>>>,
    /// WebSocket hub for real-time flow monitoring
    pub ws_hub: WsHub,
    _loader: Option<Arc<PluginLoader>>,
}

impl Clone for AppState {
    fn clone(&self) -> Self {
        Self {
            manager: self.manager.clone(),
            config: self.config.clone(),
            feature_manager: self.feature_manager.clone(),
            user_store: self.user_store.clone(),
            node_log_names: self.node_log_names.clone(),
            flow_store: self.flow_store.clone(),
            flow_runtimes: self.flow_runtimes.clone(),
            ws_hub: self.ws_hub.clone(),
            _loader: self._loader.clone(),
        }
    }
}

impl AppState {
    pub fn new(
        manager: Arc<Manager>,
        config: Config,
        loader: Option<PluginLoader>,
        feature_manager: FeatureManager,
        user_store: Arc<UserStore>,
        node_log_names: Option<NodeLogNameMap>,
        flow_store: FlowStore,
        flow_runtimes: Arc<tokio::sync::RwLock<std::collections::HashMap<Uuid, FlowRuntime>>>,
        ws_hub: WsHub,
    ) -> Self {
        Self {
            manager,
            config,
            feature_manager,
            user_store,
            node_log_names,
            flow_store,
            flow_runtimes,
            ws_hub,
            _loader: loader.map(Arc::new),
        }
    }

    /// 将当前所有节点的 id -> name 同步到 node_log_names，供节点日志按名称生成文件名
    pub fn sync_node_log_names(&self) {
        if let Some(ref map) = self.node_log_names {
            let nodes = self.manager.store.nodes_list();
            let mut m = match map.write() {
                Ok(m) => m,
                Err(_) => return,
            };
            m.clear();
            for node in nodes {
                m.insert(node.id().0.to_string(), node.config.name.clone());
            }
        }
    }

    /// 持久化到 config.data_db()（SQLite）
    pub async fn persist(&self) {
        let path = self.config.data_db();
        let snap = self.manager.build_snapshot().await;
        if let Err(e) = persist_save(&path, &snap).await {
            warn!(path = %path.display(), "persist failed: {}", e);
        }
    }
}
