//! 功能分级管理：根据授权文件中的 features 控制免费/收费功能。
//! 插件授权：默认 mqtt、modbus-tcp、modbus-rtu、sim 为免费插件，其他插件需要授权。

use crate::license::LicensePayload;
use std::sync::{Arc, RwLock};

/// 默认免费插件列表（无需授权即可使用）
const FREE_PLUGINS: &[&str] = &["mqtt", "modbus-tcp", "modbus-rtu", "sim"];

/// 免费模式最大点位数限制
const FREE_MODE_MAX_TAGS: u64 = 50;

/// 功能管理器：持有有效授权载荷，提供 can_access(feature_name)。
/// 使用 RwLock 支持运行时动态更新授权。
#[derive(Clone)]
pub struct FeatureManager {
    /// 无授权时为 None；有有效授权时为 Some(payload)
    inner: Arc<RwLock<Option<LicensePayload>>>,
}

impl FeatureManager {
    /// 无授权（未提供或校验失败）时，仅可访问"免费"功能。
    pub fn without_license() -> Self {
        Self {
            inner: Arc::new(RwLock::new(None)),
        }
    }

    /// 已通过校验的授权载荷，用于收费功能判断。
    pub fn with_license(payload: LicensePayload) -> Self {
        Self {
            inner: Arc::new(RwLock::new(Some(payload))),
        }
    }

    /// 更新授权（运行时上传新授权文件后调用）
    pub fn update_license(&self, payload: LicensePayload) {
        if let Ok(mut guard) = self.inner.write() {
            *guard = Some(payload);
        }
    }

    /// 清除授权（授权失效时调用）
    pub fn clear_license(&self) {
        if let Ok(mut guard) = self.inner.write() {
            *guard = None;
        }
    }

    /// 授权剩余天数：无授权返回 None；已过期返回负数。
    /// 用于 `/api/metrics` 的 `gateway_license_expiry_days`，便于提前告警续期。
    pub fn expiry_days_left(&self) -> Option<i64> {
        let guard = self.inner.read().ok()?;
        let payload = guard.as_ref()?;
        let expiry =
            chrono::NaiveDate::parse_from_str(payload.expiry_date.trim(), "%Y-%m-%d").ok()?;
        Some((expiry - chrono::Utc::now().date_naive()).num_days())
    }

    /// 判断当前授权是否允许访问指定功能。
    /// 通过解析授权文件中的 features 数组：若包含 feature_name 则返回 true。
    /// 无授权或未包含该功能名时返回 false（免费版不可用）。
    pub fn can_access(&self, feature_name: &str) -> bool {
        match self.inner.read() {
            Ok(guard) => match &*guard {
                Some(p) => p.features.iter().any(|f| f == feature_name),
                None => false,
            },
            Err(_) => false,
        }
    }

    /// 是否已加载有效授权（任意功能）
    pub fn has_license(&self) -> bool {
        match self.inner.read() {
            Ok(guard) => guard.is_some(),
            Err(_) => false,
        }
    }

    /// 已授权功能列表（仅供展示，无授权时为空）
    pub fn granted_features(&self) -> Vec<String> {
        match self.inner.read() {
            Ok(guard) => match &*guard {
                Some(p) => p.features.clone(),
                None => Vec::new(),
            },
            Err(_) => Vec::new(),
        }
    }

    /// 判断插件是否可用（免费插件或已授权插件）
    /// 授权文件中的 features 支持两种格式：
    /// - "plugin:<plugin_name>" 授权单个插件，如 "plugin:opcua"
    /// - "all_plugins" 授权所有插件
    pub fn can_use_plugin(&self, plugin_name: &str) -> bool {
        // 免费插件始终可用
        if FREE_PLUGINS.contains(&plugin_name) {
            return true;
        }
        // 检查授权
        match self.inner.read() {
            Ok(guard) => match &*guard {
                Some(p) => {
                    // 检查是否有 all_plugins 授权
                    if p.features.iter().any(|f| f == "all_plugins") {
                        return true;
                    }
                    // 检查是否有该插件的单独授权
                    let plugin_feature = format!("plugin:{}", plugin_name);
                    p.features.iter().any(|f| f == &plugin_feature)
                }
                None => false,
            },
            Err(_) => false,
        }
    }

    /// 判断插件是否为免费插件
    pub fn is_free_plugin(plugin_name: &str) -> bool {
        FREE_PLUGINS.contains(&plugin_name)
    }

    /// 获取免费插件列表
    pub fn free_plugins() -> &'static [&'static str] {
        FREE_PLUGINS
    }

    /// 获取已授权的插件列表（包含免费插件）
    pub fn licensed_plugins(&self) -> Vec<String> {
        let mut plugins: Vec<String> = FREE_PLUGINS.iter().map(|s| s.to_string()).collect();
        if let Ok(guard) = self.inner.read() {
            if let Some(p) = &*guard {
                // 检查 all_plugins
                if p.features.iter().any(|f| f == "all_plugins") {
                    // 返回特殊标记，表示所有插件都已授权
                    return vec!["*".to_string()];
                }
                // 添加单独授权的插件
                for f in &p.features {
                    if let Some(name) = f.strip_prefix("plugin:") {
                        if !plugins.contains(&name.to_string()) {
                            plugins.push(name.to_string());
                        }
                    }
                }
            }
        }
        plugins
    }

    /// 获取最大点位数限制
    /// - 免费模式（无授权）：最大 50 个点位
    /// - 授权模式：按授权文件中的 max_tags 限制，若未设置则无限制
    pub fn max_tags(&self) -> Option<u64> {
        match self.inner.read() {
            Ok(guard) => match &*guard {
                Some(p) => p.max_tags.filter(|&n| n > 0),
                None => Some(FREE_MODE_MAX_TAGS), // 免费模式限制 50 个点位
            },
            Err(_) => Some(FREE_MODE_MAX_TAGS),
        }
    }

    /// 检查是否可以添加指定数量的点位
    /// 返回 Ok(()) 表示可以添加，Err(msg) 表示超限
    pub fn check_tag_limit(&self, current_count: u64, add_count: u64) -> Result<(), String> {
        if let Some(max) = self.max_tags() {
            let new_total = current_count.saturating_add(add_count);
            if new_total > max {
                return Err(format!(
                    "点位数超限：当前 {}，尝试添加 {}，最大允许 {}",
                    current_count, add_count, max
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload_with_expiry(date: &str) -> LicensePayload {
        LicensePayload {
            machine_id: "test".to_string(),
            expiry_date: date.to_string(),
            features: vec![],
            max_tags: None,
        }
    }

    #[test]
    fn expiry_days_is_absent_without_license() {
        assert_eq!(FeatureManager::without_license().expiry_days_left(), None);
    }

    #[test]
    fn expiry_days_counts_down_and_goes_negative() {
        let today = chrono::Utc::now().date_naive();
        let future = (today + chrono::Duration::days(30)).format("%Y-%m-%d").to_string();
        let past = (today - chrono::Duration::days(3)).format("%Y-%m-%d").to_string();

        let fm = FeatureManager::with_license(payload_with_expiry(&future));
        assert_eq!(fm.expiry_days_left(), Some(30));

        let expired = FeatureManager::with_license(payload_with_expiry(&past));
        assert_eq!(expired.expiry_days_left(), Some(-3));
    }

    #[test]
    fn invalid_expiry_date_is_surfaced_as_unknown() {
        let fm = FeatureManager::with_license(payload_with_expiry("not-a-date"));
        assert_eq!(fm.expiry_days_left(), None);
    }
}
