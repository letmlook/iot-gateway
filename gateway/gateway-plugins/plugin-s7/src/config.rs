//! S7 插件配置辅助（从 PluginConfig HashMap 取值）。

use gateway_sdk::PluginConfig;

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

pub fn config_u64(config: &PluginConfig, key: &str, default: u64) -> u64 {
    config.get(key).and_then(|v| v.as_u64()).unwrap_or(default)
}

#[allow(dead_code)]
pub fn config_bool(config: &PluginConfig, key: &str, default: bool) -> bool {
    config.get(key).and_then(|v| v.as_bool()).unwrap_or(default)
}
