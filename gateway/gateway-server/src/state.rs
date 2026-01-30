//! 应用状态：Manager + Config + 可选 PluginLoader + 功能授权 + 用户存储，供 API 与持久化使用。

use crate::config::Config;
use crate::license::FeatureManager;
use crate::logging::NodeLogNameMap;
use crate::users::UserStore;
use gateway_core::{persist_save, PluginLoader, Manager};
use std::sync::Arc;
use tracing::warn;

#[derive(Clone)]
pub struct AppState {
    pub manager: Arc<Manager>,
    pub config: Config,
    /// 功能分级：根据 license.dat 中的 features 控制免费/收费功能
    pub feature_manager: FeatureManager,
    /// 用户存储（登录、用户 CRUD）
    pub user_store: Arc<UserStore>,
    /// 节点 ID -> 节点名称，用于节点日志文件名（按名称生成）
    pub node_log_names: Option<NodeLogNameMap>,
    _loader: Option<Arc<PluginLoader>>,
}

impl AppState {
    pub fn new(
        manager: Arc<Manager>,
        config: Config,
        loader: Option<PluginLoader>,
        feature_manager: FeatureManager,
        user_store: Arc<UserStore>,
        node_log_names: Option<NodeLogNameMap>,
    ) -> Self {
        Self {
            manager,
            config,
            feature_manager,
            user_store,
            node_log_names,
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
