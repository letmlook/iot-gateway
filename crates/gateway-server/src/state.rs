//! 应用状态：Manager + Config + 可选 PluginLoader，供 API 与持久化使用。

use crate::config::Config;
use gateway_core::{persist_save, PluginLoader, Manager};
use std::sync::Arc;
use tracing::warn;

#[derive(Clone)]
pub struct AppState {
    pub manager: Arc<Manager>,
    pub config: Config,
    _loader: Option<Arc<PluginLoader>>,
}

impl AppState {
    pub fn new(manager: Arc<Manager>, config: Config, loader: Option<PluginLoader>) -> Self {
        Self {
            manager,
            config,
            _loader: loader.map(Arc::new),
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
