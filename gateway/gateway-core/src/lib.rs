//! # Gateway Core
//!
//! 消息总线、路由核心、节点/组/标签管理。
//! 支持从 `plugins_dir` 动态加载 .so 插件（参见 `loader`）。

mod bus;
mod data_flow;
mod loader;
mod manager;
mod node;
mod persist;
mod store;

pub use bus::{Bus, SubscriptionTable, subscription_set};
pub use data_flow::{DataFlowMetrics, DataFlowMetricsSnapshot, TagForwardedStat, TagPublishedStat};
pub use loader::PluginLoader;
pub use manager::{Manager, SouthConnectionState, MIN_POLL_INTERVAL_MS};
pub use node::{Node, NodeConfig};
pub use persist::{
    apply_to_store, build_snapshot, load as persist_load, load_json as persist_load_json,
    save as persist_save, save_secret as persist_save_secret,
    load_secret as persist_load_secret, has_plaintext_secrets as persist_has_plaintext_secrets,
    save_with_backup as persist_save_with_backup, PersistError, Snapshot,
    SNAPSHOT_VERSION,
};
pub use store::Store;
