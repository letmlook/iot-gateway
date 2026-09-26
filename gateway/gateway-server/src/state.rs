//! 应用状态：Manager + Config + 可选 PluginLoader + 功能授权 + 用户存储，供 API 与持久化使用。

use crate::config::Config;
use crate::license::FeatureManager;
use crate::logging::NodeLogNameMap;
use crate::users::UserStore;
use gateway_core::proc_plugin::ProcessPluginLoader;
use gateway_core::{persist_save_secret, Manager, PluginLoader};
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
    /// 持久化脏标记：true 表示内存中已有尚未落盘的变更
    persist_dirty: Arc<std::sync::atomic::AtomicBool>,
    _loader: Option<Arc<PluginLoader>>,
    /// 进程隔离模式下的插件加载器（含各子进程的重启计数）；inproc 模式为 None
    pub plugin_processes: Option<Arc<ProcessPluginLoader>>,
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
            persist_dirty: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            _loader: loader.map(Arc::new),
            plugin_processes: None,
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

    /// 标记有配置变更。去抖窗口内不立即写库，由后台任务合并为一次事务；
    /// 窗口设为 0（`GATEWAY_PERSIST_DEBOUNCE_MS=0`）时退化为「每次变更立即落盘」。
    pub async fn persist(&self) {
        use std::sync::atomic::Ordering;
        self.persist_dirty.store(true, Ordering::Relaxed);
        if self.config.persist_debounce_ms == 0 {
            self.flush().await;
        }
    }

    /// 是否仍有未落盘的变更
    pub fn persist_pending(&self) -> bool {
        self.persist_dirty
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// 立即把当前快照落盘（合并此前累积的所有变更）。
    /// 落盘失败时保留脏标记，交由下一次触发重试。
    pub async fn flush(&self) {
        use std::sync::atomic::Ordering;
        if !self.persist_dirty.swap(false, Ordering::Relaxed) {
            return;
        }
        let path = self.config.data_db();
        let snap = self.manager.build_snapshot().await;
        let secret = self.config.master_secret.as_deref();
        if let Err(e) = persist_save_secret(&path, &snap, secret).await {
            self.persist_dirty.store(true, Ordering::Relaxed);
            warn!(path = %path.display(), "persist failed: {}", e);
        }
    }
}
