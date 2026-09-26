//! OPC UA 插件配置与 Schema 定义。

use gateway_sdk::PluginConfig;
use gateway_sdk::{
    ConfigSchema, ParamAttribute, ParamOption, ParamSchema, ParamType, ParamValid, TagRegexEntry,
    TagSchema,
};

pub const DEFAULT_ENDPOINT: &str = "opc.tcp://127.0.0.1:4840/";

/// 支持的 tag data_type 白名单（校验时大小写不敏感）
pub const TAG_DATA_TYPES: &[&str] = &[
    "int8", "int16", "int32", "int64", "uint8", "uint16", "uint32", "uint64", "float32", "float64",
    "bool", "string", "bytes",
];

pub fn config_str(config: &PluginConfig, key: &str, default: &str) -> String {
    config
        .get(key)
        .and_then(|v| v.as_str())
        .map(String::from)
        .unwrap_or_else(|| default.to_string())
}

pub fn config_schema() -> ConfigSchema {
    ConfigSchema::new()
        .param(ParamSchema {
            name: "endpoint_url".to_string(),
            name_zh: Some("端点 URL".to_string()),
            name_en: Some("Endpoint URL".to_string()),
            description: Some("OPCUA server endpoint url".to_string()),
            description_zh: Some("OPCUA 服务器端点 URL".to_string()),
            description_en: Some("OPCUA server endpoint url".to_string()),
            attribute: ParamAttribute::Required,
            ty: ParamType::String,
            default: Some(serde_json::json!(DEFAULT_ENDPOINT)),
            valid: Some(ParamValid {
                min: None,
                max: None,
                regex: Some(
                    r"^opc\.tcp:\/\/\S+:\d+(\/[\w\-._~:/?#\[\]@!$&'()*+,;=]*)?$".to_string(),
                ),
                length: Some(256),
            }),
            ..Default::default()
        })
        .param(ParamSchema {
            name: "username".to_string(),
            name_zh: Some("用户名".to_string()),
            name_en: Some("Username".to_string()),
            attribute: ParamAttribute::Optional,
            ty: ParamType::String,
            default: Some(serde_json::json!("")),
            valid: Some(ParamValid {
                min: None,
                max: None,
                regex: None,
                length: Some(30),
            }),
            ..Default::default()
        })
        .param(ParamSchema {
            name: "password".to_string(),
            name_zh: Some("密码".to_string()),
            name_en: Some("Password".to_string()),
            attribute: ParamAttribute::Optional,
            ty: ParamType::String,
            default: Some(serde_json::json!("")),
            valid: Some(ParamValid {
                min: None,
                max: None,
                regex: None,
                length: Some(30),
            }),
            ..Default::default()
        })
        .param(ParamSchema {
            name: "certificate".to_string(),
            name_zh: Some("证书文件".to_string()),
            name_en: Some("Certificate file".to_string()),
            attribute: ParamAttribute::Optional,
            ty: ParamType::File,
            default: None,
            valid: Some(ParamValid {
                min: None,
                max: None,
                regex: None,
                length: Some(81960),
            }),
            ..Default::default()
        })
        .param(ParamSchema {
            name: "key".to_string(),
            name_zh: Some("密钥文件".to_string()),
            name_en: Some("Key file".to_string()),
            attribute: ParamAttribute::Optional,
            ty: ParamType::File,
            default: None,
            valid: Some(ParamValid {
                min: None,
                max: None,
                regex: None,
                length: Some(81960),
            }),
            ..Default::default()
        })
        .param(ParamSchema {
            name: "security_mode".to_string(),
            name_zh: Some("安全策略".to_string()),
            name_en: Some("Security Mode".to_string()),
            attribute: ParamAttribute::Optional,
            ty: ParamType::Select,
            default: Some(serde_json::json!(1)),
            valid: None,
            options: Some(vec![
                ParamOption {
                    value: serde_json::json!(1),
                    label: Some("None".to_string()),
                    label_zh: Some("无".to_string()),
                    label_en: Some("None".to_string()),
                },
                ParamOption {
                    value: serde_json::json!(2),
                    label: Some("Sign".to_string()),
                    label_zh: Some("签名".to_string()),
                    label_en: Some("Sign".to_string()),
                },
                ParamOption {
                    value: serde_json::json!(3),
                    label: Some("Sign & Encrypt".to_string()),
                    label_zh: Some("签名与加密".to_string()),
                    label_en: Some("Sign & Encrypt".to_string()),
                },
            ]),
            ..Default::default()
        })
        .param(ParamSchema {
            name: "update_mode".to_string(),
            name_zh: Some("更新模式".to_string()),
            name_en: Some("Update Mode".to_string()),
            attribute: ParamAttribute::Optional,
            ty: ParamType::Select,
            default: Some(serde_json::json!(1)),
            valid: None,
            options: Some(vec![
                ParamOption {
                    value: serde_json::json!(1),
                    label: Some("Read".to_string()),
                    label_zh: Some("读取".to_string()),
                    label_en: Some("Read".to_string()),
                },
                ParamOption {
                    value: serde_json::json!(2),
                    label: Some("Subscribe".to_string()),
                    label_zh: Some("订阅".to_string()),
                    label_en: Some("Subscribe".to_string()),
                },
                ParamOption {
                    value: serde_json::json!(3),
                    label: Some("Read & Subscribe".to_string()),
                    label_zh: Some("读取与订阅".to_string()),
                    label_en: Some("Read & Subscribe".to_string()),
                },
            ]),
            ..Default::default()
        })
        .param(ParamSchema {
            name: "publish_interval".to_string(),
            name_zh: Some("发布间隔 (ms)".to_string()),
            name_en: Some("Publish Interval (ms)".to_string()),
            attribute: ParamAttribute::Optional,
            ty: ParamType::Int,
            default: Some(serde_json::json!(500)),
            valid: Some(ParamValid {
                min: Some(100),
                max: Some(65535),
                regex: None,
                length: None,
            }),
            ..Default::default()
        })
        .tag_regex(tag_regex_entries())
}

fn tag_regex_entries() -> Vec<TagRegexEntry> {
    let regex = r"^[0-9]+![0-9]+$|^[0-9]+![A-Za-z0-9_.]+$".to_string();
    vec![
        TagRegexEntry {
            data_type: "int8".to_string(),
            regex: regex.clone(),
        },
        TagRegexEntry {
            data_type: "int16".to_string(),
            regex: regex.clone(),
        },
        TagRegexEntry {
            data_type: "int32".to_string(),
            regex: regex.clone(),
        },
        TagRegexEntry {
            data_type: "int64".to_string(),
            regex: regex.clone(),
        },
        TagRegexEntry {
            data_type: "uint8".to_string(),
            regex: regex.clone(),
        },
        TagRegexEntry {
            data_type: "uint16".to_string(),
            regex: regex.clone(),
        },
        TagRegexEntry {
            data_type: "uint32".to_string(),
            regex: regex.clone(),
        },
        TagRegexEntry {
            data_type: "uint64".to_string(),
            regex: regex.clone(),
        },
        TagRegexEntry {
            data_type: "float32".to_string(),
            regex: regex.clone(),
        },
        TagRegexEntry {
            data_type: "float64".to_string(),
            regex: regex.clone(),
        },
        TagRegexEntry {
            data_type: "bool".to_string(),
            regex: regex.clone(),
        },
        TagRegexEntry {
            data_type: "string".to_string(),
            regex: regex.clone(),
        },
        TagRegexEntry {
            data_type: "bytes".to_string(),
            regex: regex.clone(),
        },
    ]
}

pub fn tag_schema() -> TagSchema {
    TagSchema {
        data_types: Some(vec![
            "int8".to_string(),
            "int16".to_string(),
            "int32".to_string(),
            "int64".to_string(),
            "uint8".to_string(),
            "uint16".to_string(),
            "uint32".to_string(),
            "uint64".to_string(),
            "float32".to_string(),
            "float64".to_string(),
            "bool".to_string(),
            "string".to_string(),
            "bytes".to_string(),
        ]),
        address_format: Some("NS!NODEID，NS 为命名空间索引，NODEID 为数字或字符串，如 0!2258、2!Device1.Module1.Tag1".to_string()),
        address_format_zh: Some("NS!NODEID，NS 为命名空间索引，NODEID 为数字或字符串，如 0!2258、2!Device1.Module1.Tag1".to_string()),
        address_format_en: Some("NS!NODEID, NS=namespace index, NODEID=numeric or string, e.g. 0!2258, 2!Device1.Module1.Tag1".to_string()),
    }
}
