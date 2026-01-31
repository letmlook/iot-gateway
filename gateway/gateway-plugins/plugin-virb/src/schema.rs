//! 振动采集插件配置与标签 Schema。

use gateway_sdk::schema::{ConfigSchema, ParamAttribute, ParamOption, ParamSchema, ParamType, ParamValid};
use gateway_sdk::{TagRegexEntry, TagSchema};

pub fn config_schema() -> ConfigSchema {
    ConfigSchema::new()
        .param(ParamSchema {
            name: "model".to_string(),
            name_zh: Some("采集器型号".to_string()),
            name_en: Some("Model".to_string()),
            description: Some("Vibration collector model".to_string()),
            description_zh: Some("联能振动采集器型号".to_string()),
            attribute: ParamAttribute::Required,
            ty: ParamType::Select,
            default: Some(serde_json::json!(0)),
            valid: None,
            options: Some(vec![
                ParamOption { value: serde_json::json!(0), label: Some("YE6275D".to_string()), label_zh: Some("YE6275D".to_string()), label_en: Some("YE6275D".to_string()) },
                ParamOption { value: serde_json::json!(1), label: Some("YE6275D2".to_string()), label_zh: Some("YE6275D2".to_string()), label_en: Some("YE6275D2".to_string()) },
            ]),
            ..Default::default()
        })
        .param(ParamSchema {
            name: "host".to_string(),
            name_zh: Some("IP地址".to_string()),
            name_en: Some("IP Address".to_string()),
            description: Some("device IP".to_string()),
            description_zh: Some("设备 IP".to_string()),
            attribute: ParamAttribute::Required,
            ty: ParamType::String,
            default: Some(serde_json::json!("192.168.99.121")),
            valid: None,
            ..Default::default()
        })
        .param(ParamSchema {
            name: "port".to_string(),
            name_zh: Some("端口号".to_string()),
            name_en: Some("Port".to_string()),
            description: Some("Device port".to_string()),
            description_zh: Some("设备端口号".to_string()),
            attribute: ParamAttribute::Required,
            ty: ParamType::Int,
            default: Some(serde_json::json!(8089)),
            valid: Some(ParamValid { min: Some(1), max: Some(65535), regex: None, length: None }),
            ..Default::default()
        })
        .param(ParamSchema {
            name: "timeout".to_string(),
            name_zh: Some("超时时间(s)".to_string()),
            name_en: Some("Timeout(s)".to_string()),
            description: Some("No data within this seconds will trigger reconnect".to_string()),
            description_zh: Some("超过该秒数无数据自动重连".to_string()),
            attribute: ParamAttribute::Required,
            ty: ParamType::Int,
            default: Some(serde_json::json!(5)),
            valid: Some(ParamValid { min: Some(1), max: Some(60), regex: None, length: None }),
            ..Default::default()
        })
        .param(ParamSchema {
            name: "sample_rate".to_string(),
            name_zh: Some("采样率".to_string()),
            name_en: Some("Sampling Rate".to_string()),
            attribute: ParamAttribute::Required,
            ty: ParamType::Select,
            default: Some(serde_json::json!(3)),
            valid: None,
            options: Some(vec![
                ParamOption { value: serde_json::json!(7), label: Some("1600".to_string()), label_zh: Some("1600".to_string()), label_en: Some("1600".to_string()) },
                ParamOption { value: serde_json::json!(6), label: Some("3200".to_string()), label_zh: Some("3200".to_string()), label_en: Some("3200".to_string()) },
                ParamOption { value: serde_json::json!(5), label: Some("6400".to_string()), label_zh: Some("6400".to_string()), label_en: Some("6400".to_string()) },
                ParamOption { value: serde_json::json!(4), label: Some("12800".to_string()), label_zh: Some("12800".to_string()), label_en: Some("12800".to_string()) },
                ParamOption { value: serde_json::json!(3), label: Some("25600".to_string()), label_zh: Some("25600".to_string()), label_en: Some("25600".to_string()) },
                ParamOption { value: serde_json::json!(2), label: Some("51200".to_string()), label_zh: Some("51200".to_string()), label_en: Some("51200".to_string()) },
                ParamOption { value: serde_json::json!(1), label: Some("128000".to_string()), label_zh: Some("128000".to_string()), label_en: Some("128000".to_string()) },
                ParamOption { value: serde_json::json!(0), label: Some("256000".to_string()), label_zh: Some("256000".to_string()), label_en: Some("256000".to_string()) },
            ]),
            ..Default::default()
        })
        .param(ParamSchema {
            name: "channel_prop".to_string(),
            name_zh: Some("通道属性".to_string()),
            name_en: Some("Channel Property".to_string()),
            attribute: ParamAttribute::Required,
            ty: ParamType::Select,
            default: Some(serde_json::json!(1)),
            valid: None,
            options: Some(vec![
                ParamOption { value: serde_json::json!(1), label: Some("IEPE".to_string()), label_zh: Some("IEPE".to_string()), label_en: Some("IEPE".to_string()) },
                ParamOption { value: serde_json::json!(0), label: Some("VOLT".to_string()), label_zh: Some("VOLT".to_string()), label_en: Some("VOLT".to_string()) },
            ]),
            ..Default::default()
        })
        .param(ParamSchema {
            name: "channel_params".to_string(),
            name_zh: Some("通道参数".to_string()),
            name_en: Some("Channel Parameters".to_string()),
            description: Some("Channel parameters, comma-separated".to_string()),
            description_zh: Some("通道参数,英文半角逗号分隔,按顺序设置每个通道对应的传感器的参数值".to_string()),
            attribute: ParamAttribute::Required,
            ty: ParamType::String,
            default: Some(serde_json::json!("1,1,1,1,1,1,1,1")),
            valid: None,
            ..Default::default()
        })
        .param(ParamSchema {
            name: "endianess".to_string(),
            name_zh: Some("字节序".to_string()),
            name_en: Some("Endianess".to_string()),
            description: Some("Tag byte order, AB corresponds to 12".to_string()),
            description_zh: Some("点位字节序，ABCD 对应 1234".to_string()),
            attribute: ParamAttribute::Required,
            ty: ParamType::Select,
            default: Some(serde_json::json!(3)),
            valid: None,
            options: Some(vec![
                ParamOption { value: serde_json::json!(1), label: Some("ABCD".to_string()), label_zh: Some("ABCD".to_string()), label_en: Some("ABCD".to_string()) },
                ParamOption { value: serde_json::json!(2), label: Some("BADC".to_string()), label_zh: Some("BADC".to_string()), label_en: Some("BADC".to_string()) },
                ParamOption { value: serde_json::json!(3), label: Some("DCBA".to_string()), label_zh: Some("DCBA".to_string()), label_en: Some("DCBA".to_string()) },
                ParamOption { value: serde_json::json!(4), label: Some("CDAB".to_string()), label_zh: Some("CDAB".to_string()), label_en: Some("CDAB".to_string()) },
                ParamOption { value: serde_json::json!(5), label: Some("AB".to_string()), label_zh: Some("AB".to_string()), label_en: Some("AB".to_string()) },
                ParamOption { value: serde_json::json!(6), label: Some("BA".to_string()), label_zh: Some("BA".to_string()), label_en: Some("BA".to_string()) },
            ]),
            ..Default::default()
        })
        .param(ParamSchema {
            name: "max_kcount".to_string(),
            name_zh: Some("最大K".to_string()),
            name_en: Some("Max K".to_string()),
            description: Some("Save the maximum value of k (default is 0), save all k data.".to_string()),
            description_zh: Some("保存最大k,默认为0,保存所有k数据".to_string()),
            attribute: ParamAttribute::Required,
            ty: ParamType::Int,
            default: Some(serde_json::json!(0)),
            valid: Some(ParamValid { min: Some(0), max: Some(4096), regex: None, length: None }),
            ..Default::default()
        })
        .param(ParamSchema {
            name: "rspeed".to_string(),
            name_zh: Some("转速(rpm)".to_string()),
            name_en: Some("RPM".to_string()),
            description: Some("revolution per minute".to_string()),
            description_zh: Some("转速(rpm)".to_string()),
            attribute: ParamAttribute::Required,
            ty: ParamType::Int,
            default: Some(serde_json::json!(2000)),
            valid: Some(ParamValid { min: Some(1), max: Some(10000), regex: None, length: None }),
            ..Default::default()
        })
        .param(ParamSchema {
            name: "unit".to_string(),
            name_zh: Some("振幅单位".to_string()),
            name_en: Some("Amplitude unit".to_string()),
            description: Some("Amplitude data units".to_string()),
            description_zh: Some("振幅数据单位".to_string()),
            attribute: ParamAttribute::Required,
            ty: ParamType::Select,
            default: Some(serde_json::json!(1)),
            valid: None,
            options: Some(vec![
                ParamOption { value: serde_json::json!(1), label: Some("米每平方秒(m/s^2)".to_string()), label_zh: Some("米每平方秒(m/s^2)".to_string()), label_en: Some("m/s²".to_string()) },
                ParamOption { value: serde_json::json!(2), label: Some("毫重力加速度(mg)".to_string()), label_zh: Some("毫重力加速度(mg)".to_string()), label_en: Some("mg".to_string()) },
                ParamOption { value: serde_json::json!(3), label: Some("位移(μm)".to_string()), label_zh: Some("位移(μm)".to_string()), label_en: Some("μm".to_string()) },
            ]),
            ..Default::default()
        })
        .param(ParamSchema {
            name: "unit_factor".to_string(),
            name_zh: Some("单位转换倍率".to_string()),
            name_en: Some("Unit factor".to_string()),
            description: Some("To convert m/s^2 to the selected unit, multiply the original data by this parameter.".to_string()),
            description_zh: Some("m/s^2转换成选中单位的倍率，原数据乘以这个参数".to_string()),
            attribute: ParamAttribute::Required,
            ty: ParamType::String,
            default: Some(serde_json::json!("101.97")),
            valid: Some(ParamValid { min: None, max: None, regex: None, length: Some(10) }),
            depends_on: Some("unit".to_string()),
            depends_value: Some(serde_json::json!(2)),
            ..Default::default()
        })
        .param(ParamSchema {
            name: "save_raw".to_string(),
            name_zh: Some("保存原始数据".to_string()),
            name_en: Some("Save Raw".to_string()),
            description: Some("Save Raw Virb Data".to_string()),
            description_zh: Some("保存原始振动数据".to_string()),
            attribute: ParamAttribute::Optional,
            ty: ParamType::Bool,
            default: Some(serde_json::json!(false)),
            valid: None,
            ..Default::default()
        })
        .param(ParamSchema {
            name: "save_raw_chs".to_string(),
            name_zh: Some("保存原始数据的通道号".to_string()),
            name_en: Some("Save Raw Channels".to_string()),
            description: Some("List of channels for storing data, separated by English half-width characters.".to_string()),
            description_zh: Some("保存数据的通道列表,英文半角逗号分隔".to_string()),
            attribute: ParamAttribute::Optional,
            ty: ParamType::String,
            default: Some(serde_json::json!("0")),
            valid: Some(ParamValid { min: None, max: None, regex: None, length: Some(50) }),
            depends_on: Some("save_raw".to_string()),
            depends_value: Some(serde_json::json!(true)),
            ..Default::default()
        })
        .tag_regex(vec![
            TagRegexEntry { data_type: "float64".to_string(), regex: r"^[0-9]+$".to_string() },
        ])
}

pub fn tag_schema() -> TagSchema {
    TagSchema {
        data_types: Some(vec!["float64".to_string()]),
        address_format: Some("通道号 1..8 (YE6275D) 或 1..32 (YE6275D2)，对应时域最大值".to_string()),
        address_format_zh: Some("通道号 1..8 (YE6275D) 或 1..32 (YE6275D2)，对应时域最大值".to_string()),
        address_format_en: Some("Channel index 1..8 (YE6275D) or 1..32 (YE6275D2), time-domain max".to_string()),
    }
}
