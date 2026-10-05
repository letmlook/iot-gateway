//! # Gateway Core
//!
//! 消息总线、路由核心、节点/组/标签管理。
//! 支持从 `plugins_dir` 动态加载 .so 插件（参见 `loader`）。

mod bus;
mod data_flow;
pub mod filters;
mod loader;
mod manager;
mod node;
mod persist;
pub mod proc_plugin;
mod rules;
mod store;

pub use bus::{subscription_set, Bus, SubscriptionTable};
pub use data_flow::{DataFlowMetrics, DataFlowMetricsSnapshot, TagForwardedStat, TagPublishedStat};
pub use filters::{
    apply as apply_filters, clear_all as filters_clear_all, forget_group as filters_forget_group,
    forget_node as filters_forget_node, validate_policy, FilterMode, FilterOutcome, GroupPolicy,
    TagDeadband, WindowAgg, WindowPolicy,
};
pub use loader::{LoadedPlugin, PluginLoader};
pub use manager::{
    LastValue, Manager, SouthConnectionState, DEFAULT_MAX_CONCURRENT_POLLS, MIN_POLL_INTERVAL_MS,
};
pub use node::{Node, NodeConfig};
pub use persist::{
    apply_to_store, build_snapshot, has_plaintext_secrets as persist_has_plaintext_secrets,
    load as persist_load, load_json as persist_load_json, load_secret as persist_load_secret,
    save as persist_save, save_secret as persist_save_secret,
    save_with_backup as persist_save_with_backup, Db, IntegrityMode, PersistError, Snapshot,
    SNAPSHOT_VERSION,
};
pub use rules::{
    engine as rule_engine, CompareOp, Firing, Rule, RuleAction, RuleCondition, RuleEngine,
    RuleRuntime, RuleSource, RuleView,
};
pub use store::Store;
