//! Modbus RTU 插件配置与 Schema。

use gateway_sdk::schema::{ParamAttribute, ParamOption, ParamValid};
use gateway_sdk::{
    ConfigSchema, ParamSchema, ParamType, TagRegexEntry, TagSchema,
};
use gateway_sdk::PluginConfig;

pub fn config_str(config: &PluginConfig, key: &str, default: &str) -> String {
    config.get(key).and_then(|v| v.as_str()).map(String::from).unwrap_or_else(|| default.to_string())
}

pub fn config_schema() -> ConfigSchema {
    ConfigSchema::new()
        .param(ParamSchema {
            name: "port".to_string(),
            name_zh: Some("串口设备".to_string()),
            name_en: Some("Serial Device".to_string()),
            description: Some("Serial device path".to_string()),
            description_zh: Some("串口设备路径".to_string()),
            description_en: Some("Serial device path".to_string()),
            attribute: ParamAttribute::Required,
            ty: ParamType::String,
            default: Some(serde_json::json!("COM1")),
            valid: Some(ParamValid { min: None, max: None, regex: None, length: Some(30) }),
            ..Default::default()
        })
        .param(ParamSchema {
            name: "connection_timeout_ms".to_string(),
            name_zh: Some("连接超时时间 (ms)".to_string()),
            name_en: Some("Connection Timeout (ms)".to_string()),
            description: Some("Connection and response timeout in milliseconds".to_string()),
            description_zh: Some("连接与响应超时时间，单位毫秒".to_string()),
            description_en: Some("Connection and response timeout in milliseconds".to_string()),
            attribute: ParamAttribute::Optional,
            ty: ParamType::Int,
            default: Some(serde_json::json!(3000)),
            valid: Some(ParamValid { min: Some(1000), max: Some(30000), regex: None, length: None }),
            ..Default::default()
        })
        .param(ParamSchema {
            name: "max_retry_times".to_string(),
            name_zh: Some("最大重试次数".to_string()),
            name_en: Some("Maximum Retry Times".to_string()),
            description: Some("The maximum number of retries after a failed attempt to send a read command".to_string()),
            description_zh: Some("发送读指令失败后最大重试次数".to_string()),
            description_en: Some("The maximum number of retries after a failed attempt to send a read command".to_string()),
            attribute: ParamAttribute::Optional,
            ty: ParamType::Int,
            default: Some(serde_json::json!(0)),
            valid: Some(ParamValid { min: Some(0), max: Some(3), regex: None, length: None }),
            ..Default::default()
        })
        .param(ParamSchema {
            name: "retry_interval_ms".to_string(),
            name_zh: Some("指令重新发送间隔 (ms)".to_string()),
            name_en: Some("Retry Interval (ms)".to_string()),
            description: Some("Interval between retries in milliseconds".to_string()),
            description_zh: Some("重试之间的间隔时间，单位毫秒".to_string()),
            description_en: Some("Interval between retries in milliseconds".to_string()),
            attribute: ParamAttribute::Optional,
            ty: ParamType::Int,
            default: Some(serde_json::json!(0)),
            valid: Some(ParamValid { min: Some(0), max: Some(10000), regex: None, length: None }),
            ..Default::default()
        })
        .param(ParamSchema {
            name: "start_address".to_string(),
            name_zh: Some("开始地址".to_string()),
            name_en: Some("Start Address".to_string()),
            description: Some("Address starts from 1 or 0".to_string()),
            description_zh: Some("地址从 1 开始或从 0 开始".to_string()),
            description_en: Some("Address starts from 1 or 0".to_string()),
            attribute: ParamAttribute::Optional,
            ty: ParamType::Select,
            default: Some(serde_json::json!(1)),
            valid: None,
            options: Some(vec![
                ParamOption {
                    value: serde_json::json!(0),
                    label: Some("Protocol Addresses (Base 0)".to_string()),
                    label_zh: Some("协议地址（从 0 开始）".to_string()),
                    label_en: Some("Protocol Addresses (Base 0)".to_string()),
                },
                ParamOption {
                    value: serde_json::json!(1),
                    label: Some("PLC Addresses (Base 1)".to_string()),
                    label_zh: Some("PLC 地址（从 1 开始）".to_string()),
                    label_en: Some("PLC Addresses (Base 1)".to_string()),
                },
            ]),
            ..Default::default()
        })
        .param(ParamSchema {
            name: "send_interval_ms".to_string(),
            name_zh: Some("指令发送间隔 (ms)".to_string()),
            name_en: Some("Send Interval (ms)".to_string()),
            description: Some("Interval between read/write commands in milliseconds".to_string()),
            description_zh: Some("相邻读/写命令之间的间隔，单位毫秒".to_string()),
            description_en: Some("Interval between read/write commands in milliseconds".to_string()),
            attribute: ParamAttribute::Optional,
            ty: ParamType::Int,
            default: Some(serde_json::json!(20)),
            valid: Some(ParamValid { min: Some(0), max: Some(3000), regex: None, length: None }),
            ..Default::default()
        })
        .param(ParamSchema {
            name: "slave_id".to_string(),
            name_zh: Some("从站 ID".to_string()),
            name_en: Some("Slave ID".to_string()),
            description: Some("Modbus slave ID".to_string()),
            description_zh: Some("Modbus 从站 ID".to_string()),
            description_en: Some("Modbus slave ID".to_string()),
            attribute: ParamAttribute::Optional,
            ty: ParamType::Int,
            default: Some(serde_json::json!(1)),
            valid: None,
            ..Default::default()
        })
        .param(ParamSchema {
            name: "stop_bits".to_string(),
            name_zh: Some("停止位".to_string()),
            name_en: Some("Stop Bits".to_string()),
            description: Some("Stop bits 1 or 2".to_string()),
            description_zh: Some("停止位 1 或 2".to_string()),
            description_en: Some("Stop bits 1 or 2".to_string()),
            attribute: ParamAttribute::Optional,
            ty: ParamType::Select,
            default: Some(serde_json::json!(1)),
            valid: None,
            options: Some(vec![
                ParamOption { value: serde_json::json!(1), label: Some("1".to_string()), label_zh: Some("1".to_string()), label_en: Some("1".to_string()) },
                ParamOption { value: serde_json::json!(2), label: Some("2".to_string()), label_zh: Some("2".to_string()), label_en: Some("2".to_string()) },
            ]),
            ..Default::default()
        })
        .param(ParamSchema {
            name: "parity".to_string(),
            name_zh: Some("校验位".to_string()),
            name_en: Some("Parity".to_string()),
            description: Some("Parity: none, odd, even, mark, or space".to_string()),
            description_zh: Some("校验位：none / odd / even / mark / space".to_string()),
            description_en: Some("Parity: none, odd, even, mark, or space".to_string()),
            attribute: ParamAttribute::Optional,
            ty: ParamType::Select,
            default: Some(serde_json::json!("none")),
            valid: None,
            options: Some(vec![
                ParamOption { value: serde_json::json!("none"), label: Some("none".to_string()), label_zh: Some("none".to_string()), label_en: Some("none".to_string()) },
                ParamOption { value: serde_json::json!("odd"), label: Some("odd".to_string()), label_zh: Some("odd".to_string()), label_en: Some("odd".to_string()) },
                ParamOption { value: serde_json::json!("even"), label: Some("even".to_string()), label_zh: Some("even".to_string()), label_en: Some("even".to_string()) },
                ParamOption { value: serde_json::json!("mark"), label: Some("mark".to_string()), label_zh: Some("mark".to_string()), label_en: Some("mark".to_string()) },
                ParamOption { value: serde_json::json!("space"), label: Some("space".to_string()), label_zh: Some("space".to_string()), label_en: Some("space".to_string()) },
            ]),
            ..Default::default()
        })
        .param(ParamSchema {
            name: "baud_rate".to_string(),
            name_zh: Some("波特率".to_string()),
            name_en: Some("Baud Rate".to_string()),
            description: Some("Serial baud rate".to_string()),
            description_zh: Some("串口波特率".to_string()),
            description_en: Some("Serial baud rate".to_string()),
            attribute: ParamAttribute::Optional,
            ty: ParamType::Select,
            default: Some(serde_json::json!(9600)),
            valid: None,
            options: Some(vec![
                ParamOption { value: serde_json::json!(150), label: Some("150".to_string()), label_zh: Some("150".to_string()), label_en: Some("150".to_string()) },
                ParamOption { value: serde_json::json!(200), label: Some("200".to_string()), label_zh: Some("200".to_string()), label_en: Some("200".to_string()) },
                ParamOption { value: serde_json::json!(300), label: Some("300".to_string()), label_zh: Some("300".to_string()), label_en: Some("300".to_string()) },
                ParamOption { value: serde_json::json!(600), label: Some("600".to_string()), label_zh: Some("600".to_string()), label_en: Some("600".to_string()) },
                ParamOption { value: serde_json::json!(1200), label: Some("1200".to_string()), label_zh: Some("1200".to_string()), label_en: Some("1200".to_string()) },
                ParamOption { value: serde_json::json!(1800), label: Some("1800".to_string()), label_zh: Some("1800".to_string()), label_en: Some("1800".to_string()) },
                ParamOption { value: serde_json::json!(2400), label: Some("2400".to_string()), label_zh: Some("2400".to_string()), label_en: Some("2400".to_string()) },
                ParamOption { value: serde_json::json!(4800), label: Some("4800".to_string()), label_zh: Some("4800".to_string()), label_en: Some("4800".to_string()) },
                ParamOption { value: serde_json::json!(9600), label: Some("9600".to_string()), label_zh: Some("9600".to_string()), label_en: Some("9600".to_string()) },
                ParamOption { value: serde_json::json!(19200), label: Some("19200".to_string()), label_zh: Some("19200".to_string()), label_en: Some("19200".to_string()) },
                ParamOption { value: serde_json::json!(38400), label: Some("38400".to_string()), label_zh: Some("38400".to_string()), label_en: Some("38400".to_string()) },
                ParamOption { value: serde_json::json!(57600), label: Some("57600".to_string()), label_zh: Some("57600".to_string()), label_en: Some("57600".to_string()) },
                ParamOption { value: serde_json::json!(115200), label: Some("115200".to_string()), label_zh: Some("115200".to_string()), label_en: Some("115200".to_string()) },
            ]),
            ..Default::default()
        })
        .param(ParamSchema {
            name: "data_bits".to_string(),
            name_zh: Some("数据位".to_string()),
            name_en: Some("Data Bits".to_string()),
            description: Some("Data bits 5, 6, 7 or 8".to_string()),
            description_zh: Some("数据位 5、6、7 或 8".to_string()),
            description_en: Some("Data bits 5, 6, 7 or 8".to_string()),
            attribute: ParamAttribute::Optional,
            ty: ParamType::Select,
            default: Some(serde_json::json!(8)),
            valid: None,
            options: Some(vec![
                ParamOption { value: serde_json::json!(5), label: Some("5".to_string()), label_zh: Some("5".to_string()), label_en: Some("5".to_string()) },
                ParamOption { value: serde_json::json!(6), label: Some("6".to_string()), label_zh: Some("6".to_string()), label_en: Some("6".to_string()) },
                ParamOption { value: serde_json::json!(7), label: Some("7".to_string()), label_zh: Some("7".to_string()), label_en: Some("7".to_string()) },
                ParamOption { value: serde_json::json!(8), label: Some("8".to_string()), label_zh: Some("8".to_string()), label_en: Some("8".to_string()) },
            ]),
            ..Default::default()
        })
        .tag_regex(vec![
            TagRegexEntry { data_type: "bool".to_string(), regex: r"^[01]x![0-9]+(![0-9]+)?$".to_string() },
            TagRegexEntry { data_type: "int16".to_string(), regex: r"^[34]x![0-9]+(![0-9]+)?$".to_string() },
            TagRegexEntry { data_type: "uint16".to_string(), regex: r"^[34]x![0-9]+(![0-9]+)?$".to_string() },
            TagRegexEntry { data_type: "float32".to_string(), regex: r"^[34]x![0-9]+(!2)?$".to_string() },
            TagRegexEntry { data_type: "float64".to_string(), regex: r"^[34]x![0-9]+(!4)?$".to_string() },
        ])
}

pub fn tag_schema() -> TagSchema {
    TagSchema {
        data_types: Some(vec![
            "bool".to_string(),
            "int16".to_string(),
            "uint16".to_string(),
            "int32".to_string(),
            "uint32".to_string(),
            "int64".to_string(),
            "uint64".to_string(),
            "float32".to_string(),
            "float64".to_string(),
            "string".to_string(),
            "bytes".to_string(),
        ]),
        address_format: Some("0x!addr/1x!addr/3x!addr/4x!addr 或 1!400001[.BIT][#ENDIAN]，.LEN 用于 STRING".to_string()),
        address_format_zh: Some("0x!addr/1x!addr/3x!addr/4x!addr 或 1!400001[.BIT][#ENDIAN]，.LEN 用于 STRING".to_string()),
        address_format_en: Some("0x!addr/1x!addr/3x!addr/4x!addr or 1!400001[.BIT][#ENDIAN], .LEN for STRING".to_string()),
    }
}
