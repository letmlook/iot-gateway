//! 插件配置与点位 Schema：params、tag_regex 等。

use crate::types::PluginConfig;
use serde::{Deserialize, Serialize};

/// 参数属性：必填 / 可选
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ParamAttribute {
    #[default]
    Required,
    Optional,
}

/// 参数类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ParamType {
    #[default]
    Int,
    String,
    Bool,
    /// 文件：前端用上传控件，配置存文件路径（字符串）
    File,
    /// 下拉选项：options 列表，值为 options[].value
    Select,
}

/// 下拉选项单项：value 为实际存储值，label 为展示（可选多语言）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParamOption {
    pub value: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_zh: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_en: Option<String>,
}

/// 参数校验（范围、正则、长度等）
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ParamValid {
    pub min: Option<i64>,
    pub max: Option<i64>,
    pub regex: Option<String>,
    pub length: Option<usize>,
}

/// 单个配置参数 Schema
/// - `name` 为配置键，必填，校验与存储使用
/// - `name_zh` / `name_en`、`description_zh` / `description_en` 为展示用中英文，前端按当前语言选用
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ParamSchema {
    pub name: String,
    pub description: Option<String>,
    /// 中文标签（前端 zh 时显示）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name_zh: Option<String>,
    /// 英文标签（前端 en 时显示）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name_en: Option<String>,
    /// 中文描述（前端 zh 时显示）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description_zh: Option<String>,
    /// 英文描述（前端 en 时显示）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description_en: Option<String>,
    #[serde(default)]
    pub attribute: ParamAttribute,
    #[serde(rename = "type")]
    pub ty: ParamType,
    pub default: Option<serde_json::Value>,
    pub valid: Option<ParamValid>,
    /// 下拉选项（type=select 时必填）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<Vec<ParamOption>>,
    /// 依赖字段名：仅当依赖字段等于 depends_value（或在 depends_values 中）时本字段显示/生效
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depends_on: Option<String>,
    /// 依赖字段等于该值时显示（与 depends_values 二选一）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depends_value: Option<serde_json::Value>,
    /// 依赖字段在该列表中时显示
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depends_values: Option<Vec<serde_json::Value>>,
}

/// 点位地址正则：按数据类型配置地址格式
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagRegexEntry {
    /// 数据类型标识（如 "int16"、"float64"、或自定义）
    pub data_type: String,
    /// 地址正则
    pub regex: String,
}

/// 插件配置 Schema
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConfigSchema {
    /// 配置参数列表
    pub params: Vec<ParamSchema>,
    /// 点位地址正则（按数据类型）
    pub tag_regex: Option<Vec<TagRegexEntry>>,
    /// 敏感参数名列表（如口令、私钥路径）：API 返回时脱敏为 `"***"`，持久化时加密存储
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sensitive: Option<Vec<String>>,
}

impl ConfigSchema {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn param(mut self, p: ParamSchema) -> Self {
        self.params.push(p);
        self
    }

    pub fn tag_regex(mut self, entries: Vec<TagRegexEntry>) -> Self {
        self.tag_regex = Some(entries);
        self
    }

    /// 声明敏感参数名（口令、私钥、AK/SK 等）：这些值不会被 API 明文返回，也不会明文入库
    pub fn sensitive(mut self, names: &[&str]) -> Self {
        let mut v = self.sensitive.unwrap_or_default();
        for n in names {
            if !v.iter().any(|x| x == n) {
                v.push((*n).to_string());
            }
        }
        self.sensitive = Some(v);
        self
    }

    /// 是否为敏感字段：命中插件显式声明或名称兜底约定
    pub fn is_sensitive(&self, key: &str) -> bool {
        if self
            .sensitive
            .as_ref()
            .map(|v| v.iter().any(|n| n == key))
            .unwrap_or(false)
        {
            return true;
        }
        is_sensitive_key(key)
    }

    /// 返回脱敏后的配置副本：敏感字段统一替换为 `"***"`
    pub fn mask_config(&self, config: &PluginConfig) -> PluginConfig {
        let mut out = config.clone();
        for key in config.keys() {
            if self.is_sensitive(key) {
                if let Some(v) = out.get_mut(key) {
                    *v = serde_json::Value::String(MASKED.to_string());
                }
            }
        }
        out
    }

    /// 依赖是否满足：无 depends_on 则 true；有则检查 config[depends_on] 是否等于 depends_value 或在 depends_values 中。
    pub fn param_visible(&self, param: &ParamSchema, config: &PluginConfig) -> bool {
        let Some(ref key) = param.depends_on else {
            return true;
        };
        let dep_val = match config.get(key) {
            Some(v) => v.clone(),
            None => return false,
        };
        if let Some(ref expect) = param.depends_value {
            return dep_val == *expect;
        }
        if let Some(ref list) = param.depends_values {
            return list.contains(&dep_val);
        }
        false
    }

    /// 按数据类型校验地址是否匹配 tag_regex。无 regex 时返回 true。
    pub fn validate_address(&self, data_type: &str, address: &str) -> bool {
        use regex::Regex;
        let Some(ref entries) = self.tag_regex else {
            return true;
        };
        for e in entries {
            if e.data_type.eq_ignore_ascii_case(data_type) {
                return match Regex::new(&e.regex) {
                    Ok(r) => r.is_match(address),
                    Err(_) => true,
                };
            }
        }
        true
    }

    /// 按 Schema 校验插件配置。创建/修改节点时调用。依赖未满足的字段不参与校验。
    pub fn validate_config(
        &self,
        config: &PluginConfig,
    ) -> Result<(), String> {
        use regex::Regex;
        for param in &self.params {
            if !self.param_visible(param, config) {
                continue;
            }
            let val = config.get(&param.name);
            if val.is_none() {
                if param.attribute == ParamAttribute::Required {
                    return Err(format!("missing required config: {}", param.name));
                }
                continue;
            }
            let v = val.unwrap();
            match param.ty {
                ParamType::Int => {
                    let n = v.as_i64().ok_or_else(|| {
                        format!("config {} must be integer", param.name)
                    })?;
                    if let Some(ref valid) = param.valid {
                        if let Some(min) = valid.min {
                            if n < min {
                                return Err(format!("config {} below min {}", param.name, min));
                            }
                        }
                        if let Some(max) = valid.max {
                            if n > max {
                                return Err(format!("config {} above max {}", param.name, max));
                            }
                        }
                    }
                }
                ParamType::String => {
                    let s = v.as_str().ok_or_else(|| {
                        format!("config {} must be string", param.name)
                    })?;
                    if let Some(ref valid) = param.valid {
                        if let Some(ref re) = valid.regex {
                            let regex = Regex::new(re)
                                .map_err(|e| format!("config {} regex invalid: {}", param.name, e))?;
                            if !regex.is_match(s) {
                                return Err(format!("config {} does not match pattern", param.name));
                            }
                        }
                        if let Some(len) = valid.length {
                            if s.len() > len {
                                return Err(format!("config {} length exceeds {}", param.name, len));
                            }
                        }
                    }
                }
                ParamType::Bool => {
                    if !v.is_boolean() {
                        return Err(format!("config {} must be boolean", param.name));
                    }
                }
                ParamType::File => {
                    // 存文件路径字符串，校验同 String
                    let s = v.as_str().ok_or_else(|| {
                        format!("config {} must be string (file path)", param.name)
                    })?;
                    if let Some(ref valid) = param.valid {
                        if let Some(ref re) = valid.regex {
                            let regex = Regex::new(re)
                                .map_err(|e| format!("config {} regex invalid: {}", param.name, e))?;
                            if !regex.is_match(s) {
                                return Err(format!("config {} does not match pattern", param.name));
                            }
                        }
                        if let Some(len) = valid.length {
                            if s.len() > len {
                                return Err(format!("config {} length exceeds {}", param.name, len));
                            }
                        }
                    }
                }
                ParamType::Select => {
                    let opts = param.options.as_deref().unwrap_or(&[]);
                    let ok = opts.iter().any(|o| o.value == *v);
                    if !ok {
                        return Err(format!(
                            "config {} must be one of options",
                            param.name
                        ));
                    }
                }
            }
        }
        Ok(())
    }
}

/// 点位 Schema（用于校验、UI 展示）。可扩展。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TagSchema {
    /// 支持的数据类型
    pub data_types: Option<Vec<String>>,
    /// 地址格式说明（通用，无语言时使用）
    pub address_format: Option<String>,
    /// 地址格式说明（中文）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address_format_zh: Option<String>,
    /// 地址格式说明（英文）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address_format_en: Option<String>,
}

/// API 脱敏占位值
pub const MASKED: &str = "***";

/// 名称兜底判定的敏感字段关键字
const SENSITIVE_KEYWORDS: &[&str] = &[
    "password",
    "passwd",
    "secret",
    "token",
    "private_key",
    "client_key",
    "api_key",
    "access_key",
];

/// 凭据类配置项判定：命中即视为敏感（不通过 API 明文返回、落盘时加密）
pub fn is_sensitive_key(key: &str) -> bool {
    // 归一化后再匹配：忽略大小写与 `_`/`-`，使 accessKey / access_key / ACCESS-KEY 同等对待
    let norm = |s: &str| -> String {
        s.to_ascii_lowercase()
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect()
    };
    let k = norm(key);
    SENSITIVE_KEYWORDS.iter().any(|s| k.contains(&norm(s)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::PluginConfig;

    fn cfg(pairs: &[(&str, serde_json::Value)]) -> PluginConfig {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), v.clone()))
            .collect()
    }

    #[test]
    fn sensitive_keywords_are_detected() {
        assert!(is_sensitive_key("password"));
        assert!(is_sensitive_key("mqtt_password"));
        assert!(is_sensitive_key("client_key"));
        assert!(is_sensitive_key("accessKey"));
        assert!(!is_sensitive_key("host"));
        assert!(!is_sensitive_key("interval_ms"));
    }

    #[test]
    fn schema_sensitive_declaration_and_masking() {
        let schema = ConfigSchema::new().sensitive(&["auth_aes_key"]);
        assert!(schema.is_sensitive("auth_aes_key"));
        // 未在列表中但命中名称约定，同样视为敏感
        assert!(schema.is_sensitive("password"));

        let config = cfg(&[
            ("host", serde_json::json!("broker.local")),
            ("password", serde_json::json!("p@ss")),
            ("auth_aes_key", serde_json::json!("k1")),
        ]);
        let masked = schema.mask_config(&config);
        assert_eq!(masked.get("host").unwrap(), "broker.local");
        assert_eq!(masked.get("password").unwrap(), MASKED);
        assert_eq!(masked.get("auth_aes_key").unwrap(), MASKED);
    }

    #[test]
    fn validate_config_enforces_required_and_ranges() {
        let schema = ConfigSchema::new()
            .param(ParamSchema {
                name: "host".to_string(),
                ty: ParamType::String,
                attribute: ParamAttribute::Required,
                ..Default::default()
            })
            .param(ParamSchema {
                name: "port".to_string(),
                ty: ParamType::Int,
                attribute: ParamAttribute::Required,
                valid: Some(ParamValid {
                    min: Some(1),
                    max: Some(65535),
                    ..Default::default()
                }),
                ..Default::default()
            });

        // 缺失必填
        assert!(schema
            .validate_config(&cfg(&[("port", serde_json::json!(1883))]))
            .is_err());
        // 超出范围
        assert!(schema
            .validate_config(&cfg(&[
                ("host", serde_json::json!("127.0.0.1")),
                ("port", serde_json::json!(70000))
            ]))
            .is_err());
        // 合法
        assert!(schema
            .validate_config(&cfg(&[
                ("host", serde_json::json!("127.0.0.1")),
                ("port", serde_json::json!(1883))
            ]))
            .is_ok());
    }

    #[test]
    fn validate_address_matches_tag_regex() {
        let schema = ConfigSchema::new().tag_regex(vec![TagRegexEntry {
            data_type: "int16".to_string(),
            regex: r"^4\d{4}$".to_string(),
        }]);
        assert!(schema.validate_address("int16", "40001"));
        assert!(!schema.validate_address("int16", "abc"));
        // 未配置的数据类型默认通过
        assert!(schema.validate_address("float64", "anything"));
    }

    #[test]
    fn depends_on_controls_visibility_and_validation() {
        let schema = ConfigSchema::new().param(ParamSchema {
            name: "password".to_string(),
            ty: ParamType::String,
            attribute: ParamAttribute::Required,
            depends_on: Some("auth".to_string()),
            depends_value: Some(serde_json::json!(true)),
            ..Default::default()
        });
        let vis = cfg(&[("auth", serde_json::json!(true))]);
        let invis = cfg(&[("auth", serde_json::json!(false))]);
        assert!(schema.param_visible(&schema.params[0], &vis));
        assert!(!schema.param_visible(&schema.params[0], &invis));
        // 隐藏字段不参与必填校验
        assert!(schema.validate_config(&invis).is_ok());
        assert!(schema.validate_config(&vis).is_err());
    }
}
