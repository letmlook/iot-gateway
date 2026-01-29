//! # Gateway Core
//!
//! 消息总线、路由核心、节点/组/标签管理，对标 Neuron 的 Manager + NNG 总线。
//! 支持从 `plugins_dir` 动态加载 .so 插件（参见 `loader`）。

mod bus;
mod loader;
mod manager;
mod node;
mod persist;
mod store;

pub use bus::{Bus, SubscriptionTable, subscription_set};
pub use loader::PluginLoader;
pub use manager::Manager;
pub use node::{Node, NodeConfig};
pub use persist::{
    apply_to_store, build_snapshot, load as persist_load, load_json as persist_load_json,
    save as persist_save, save_with_backup as persist_save_with_backup, PersistError, Snapshot,
    SNAPSHOT_VERSION,
};
pub use store::Store;
