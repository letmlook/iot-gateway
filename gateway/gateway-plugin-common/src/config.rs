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

/// 未显式配置 `cache_dir` 时解析默认目录。
///
/// 优先取 `GATEWAY_DATA_DIR` 环境变量，返回 `<GATEWAY_DATA_DIR>/<name>`；
/// 未设置（或为空）时回退到 `fallback`（CWD 相对路径，向后兼容）。
pub fn default_cache_dir(name: &str, fallback: &str) -> String {
    match std::env::var("GATEWAY_DATA_DIR") {
        Ok(dir) if !dir.trim().is_empty() => cache_dir_under(dir.trim(), name),
        _ => fallback.to_string(),
    }
}

/// `<data_dir>/<name>`（纯字符串拼接，统一用 `/` 分隔，便于单测与跨平台一致；
/// Windows 同样接受 `/` 分隔符，最终仍经 PathBuf 打开）
fn cache_dir_under(data_dir: &str, name: &str) -> String {
    let normalized = data_dir.trim_end_matches(['/', '\\']).replace('\\', "/");
    format!("{normalized}/{name}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 两个用例都要改 GATEWAY_DATA_DIR，用互斥锁串行化（cargo test 线程并行）
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn cache_dir_under_joins_data_dir_and_name() {
        assert_eq!(
            cache_dir_under("/var/lib/iot-gateway", "http-queue"),
            "/var/lib/iot-gateway/http-queue"
        );
        assert_eq!(
            cache_dir_under("C:\\gateway-data", "kafka-queue"),
            "C:/gateway-data/kafka-queue",
            "固定使用 / 分隔（PathBuf 两种分隔符都能处理）"
        );
        assert_eq!(
            cache_dir_under("C:\\gateway-data\\", "kafka-queue"),
            "C:/gateway-data/kafka-queue",
            "尾部分隔符应被归一化"
        );
    }

    #[test]
    fn default_cache_dir_prefers_gateway_data_dir() {
        let _guard = ENV_LOCK.lock().unwrap();
        let saved = std::env::var("GATEWAY_DATA_DIR").ok();
        std::env::set_var("GATEWAY_DATA_DIR", "/tmp/gw-test-data");
        assert_eq!(
            default_cache_dir("http-queue", "data/http-queue"),
            "/tmp/gw-test-data/http-queue"
        );
        // 恢复现场
        match saved {
            Some(v) => std::env::set_var("GATEWAY_DATA_DIR", v),
            None => std::env::remove_var("GATEWAY_DATA_DIR"),
        }
    }

    #[test]
    fn default_cache_dir_falls_back_when_unset() {
        let _guard = ENV_LOCK.lock().unwrap();
        // 未设置（或空值）时保持原默认值（CWD 相对，向后兼容）
        let saved = std::env::var("GATEWAY_DATA_DIR").ok();
        std::env::remove_var("GATEWAY_DATA_DIR");
        assert_eq!(
            default_cache_dir("http-queue", "data/http-queue"),
            "data/http-queue"
        );
        std::env::set_var("GATEWAY_DATA_DIR", "   ");
        assert_eq!(
            default_cache_dir("http-queue", "data/http-queue"),
            "data/http-queue",
            "空白值应视同未设置"
        );
        match saved {
            Some(v) => std::env::set_var("GATEWAY_DATA_DIR", v),
            None => std::env::remove_var("GATEWAY_DATA_DIR"),
        }
    }
}
