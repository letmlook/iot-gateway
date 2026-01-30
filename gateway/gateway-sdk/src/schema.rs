//! 插件配置与点位 Schema，对标 Neuron modbus-tcp.json、tag_regex 等。

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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ParamType {
    Int,
    String,
    Bool,
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
}

/// 点位地址正则：按数据类型配置地址格式
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagRegexEntry {
    /// 数据类型标识（如 "int16"、"float64"、或自定义）
    pub data_type: String,
    /// 地址正则
    pub regex: String,
}

/// 插件配置 Schema（对标 Neuron 驱动 json）
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConfigSchema {
    /// 配置参数列表
    pub params: Vec<ParamSchema>,
    /// 点位地址正则（按数据类型）
    pub tag_regex: Option<Vec<TagRegexEntry>>,
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

    /// 按 Schema 校验插件配置。创建/修改节点时调用。
    pub fn validate_config(
        &self,
        config: &PluginConfig,
    ) -> Result<(), String> {
        use regex::Regex;
        for param in &self.params {
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
