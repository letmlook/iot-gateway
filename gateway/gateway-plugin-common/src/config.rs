//! 北向插件共享配置常量与辅助函数。

use gateway_sdk::types::PluginConfig;

// ---------------------------------------------------------------------------
// 默认常量
// ---------------------------------------------------------------------------

/// 离线队列内存容量（条数）
pub const DEFAULT_CACHE_MEMORY_SIZE: usize = 1000;
/// 离线队列落盘目录
pub const DEFAULT_CACHE_DIR: &str = "data/mqtt-queue";
/// 离线队列补发间隔（毫秒）
pub const DEFAULT_CACHE_SYNC_INTERVAL_MS: u64 = 100;

// ---------------------------------------------------------------------------
// 配置辅助函数
// ---------------------------------------------------------------------------

pub fn config_str(config: &PluginConfig, key: &str, default: &str) -> String {
    config
        .get(key)
        .and_then(|v| v.as_str())
        .map(String::from)
        .unwrap_or_else(|| default.to_string())
}

pub fn config_u16(config: &PluginConfig, key: &str, default: u16) -> u16 {
    config
        .get(key)
        .and_then(|v| v.as_u64())
        .map(|n| n as u16)
        .unwrap_or(default)
}

pub fn config_usize(config: &PluginConfig, key: &str, default: usize) -> usize {
    config
        .get(key)
        .and_then(|v| v.as_u64())
        .map(|n| n as usize)
        .unwrap_or(default)
}

pub fn config_u64(config: &PluginConfig, key: &str, default: u64) -> u64 {
    config.get(key).and_then(|v| v.as_u64()).unwrap_or(default)
}

pub fn config_bool(config: &PluginConfig, key: &str, default: bool) -> bool {
    config.get(key).and_then(|v| v.as_bool()).unwrap_or(default)
}

/// 构建离线队列路径
pub fn queue_path_for_node(
    cache_dir: &str,
    node_id: gateway_sdk::NodeId,
    suffix: &str,
) -> Option<std::path::PathBuf> {
    let dir = std::path::PathBuf::from(cache_dir);
    Some(dir.join(format!("{}{}.queue", node_id.0, suffix)))
}
