//! 模拟插件配置与标签 Schema。

use gateway_sdk::ParamAttribute;
use gateway_sdk::{ConfigSchema, ParamSchema, ParamType, TagRegexEntry, TagSchema};

pub fn config_schema() -> ConfigSchema {
    ConfigSchema::new()
        .param(ParamSchema {
            name: "poll_base_ms".to_string(),
            description: Some("Poll base interval (ms)".to_string()),
            name_zh: Some("轮询基准间隔(ms)".to_string()),
            name_en: Some("Poll base interval (ms)".to_string()),
            description_zh: Some("采集轮询的基础间隔，单位毫秒".to_string()),
            description_en: Some("Base interval for polling, in milliseconds".to_string()),
            attribute: ParamAttribute::Optional,
            ty: ParamType::Int,
            default: Some(serde_json::json!(1000)),
            valid: None,
            ..Default::default()
        })
        .tag_regex(vec![TagRegexEntry {
            data_type: "float64".to_string(),
            regex: r"^[0-9]+$".to_string(),
        }])
}

pub fn tag_schema() -> TagSchema {
    TagSchema {
        data_types: Some(vec!["float64".to_string()]),
        address_format: Some("0=temperature, 1=humidity".to_string()),
        address_format_zh: Some("0=温度, 1=湿度".to_string()),
        address_format_en: Some("0=temperature, 1=humidity".to_string()),
    }
}
