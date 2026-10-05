//! MQTT 插件配置 Schema。

use gateway_sdk::schema::{
    ConfigSchema, ParamAttribute, ParamOption, ParamSchema, ParamType, ParamValid,
};

// MQTT 特有常量
pub const DEFAULT_HOST: &str = "broker.emqx.io";
pub const DEFAULT_PORT: u16 = 1883;
pub const DEFAULT_TOPIC_TEMPLATE: &str = "gateway/data/${node_id}/${group_id}";
pub const DEFAULT_KEEP_ALIVE_SECS: u64 = 30;
pub const DEFAULT_QOS: u8 = 1;
pub const UPLOAD_FORMAT_VALUES_FORMAT: &str = "values_format";
pub const UPLOAD_FORMAT_TAGS_FORMAT: &str = "tags_format";
pub const UPLOAD_FORMAT_ECP_FORMAT: &str = "ecp_format";
pub const UPLOAD_FORMAT_GROUP_DATA: &str = "group_data";
pub const UPLOAD_FORMAT_RAW_DATA: &str = "raw_data";

pub fn config_schema() -> ConfigSchema {
    ConfigSchema::new()
        .param(ParamSchema {
            name: "host".to_string(),
            name_zh: Some("服务器地址".to_string()),
            name_en: Some("Broker Host".to_string()),
            description: Some("MQTT Broker IP or hostname".to_string()),
            description_zh: Some("MQTT Broker IP 或域名".to_string()),
            description_en: Some("MQTT Broker IP or hostname".to_string()),
            attribute: ParamAttribute::Optional,
            ty: ParamType::String,
            default: Some(serde_json::json!(DEFAULT_HOST)),
            valid: Some(ParamValid { min: None, max: None, regex: None, length: Some(255) }),
            ..Default::default()
        })
        .param(ParamSchema {
            name: "port".to_string(),
            name_zh: Some("服务器端口".to_string()),
            name_en: Some("Broker Port".to_string()),
            description: Some("Broker port, typically 1883 or 8883 for TLS".to_string()),
            description_zh: Some("Broker 端口，TLS 通常为 8883".to_string()),
            description_en: Some("Broker port, typically 1883 or 8883 for TLS".to_string()),
            attribute: ParamAttribute::Optional,
            ty: ParamType::Int,
            default: Some(serde_json::json!(DEFAULT_PORT)),
            valid: Some(ParamValid { min: Some(1), max: Some(65535), regex: None, length: None }),
            ..Default::default()
        })
        .param(ParamSchema {
            name: "client_id".to_string(),
            name_zh: Some("客户端 ID".to_string()),
            name_en: Some("Client ID".to_string()),
            description: Some("MQTT client ID, auto-generated if empty".to_string()),
            description_zh: Some("MQTT 客户端 ID，不填则自动生成".to_string()),
            description_en: Some("MQTT client ID, auto-generated if empty".to_string()),
            attribute: ParamAttribute::Optional,
            ty: ParamType::String,
            default: None,
            valid: Some(ParamValid { min: None, max: None, regex: None, length: Some(255) }),
            ..Default::default()
        })
        .param(ParamSchema {
            name: "topic_template".to_string(),
            name_zh: Some("主题模板".to_string()),
            name_en: Some("Topic template".to_string()),
            description: Some("Publish topic template. Variables: ${node_id}/${group_id} (replaced with name when available), ${node_name}/${group_name}, ${timestamp}".to_string()),
            description_zh: Some("发布主题模板。变量：${node_id}/${group_id}（有名称时用名称）、${node_name}/${group_name}、${timestamp}".to_string()),
            description_en: Some("Publish topic template. Variables: ${node_id}/${group_id} (name when available), ${node_name}/${group_name}, ${timestamp}".to_string()),
            attribute: ParamAttribute::Optional,
            ty: ParamType::String,
            default: Some(serde_json::json!(DEFAULT_TOPIC_TEMPLATE)),
            valid: Some(ParamValid { min: None, max: None, regex: None, length: Some(255) }),
            ..Default::default()
        })
        .param(ParamSchema {
            name: "qos".to_string(),
            name_zh: Some("QoS 等级".to_string()),
            name_en: Some("QoS Level".to_string()),
            description: Some("MQTT QoS level for message delivery".to_string()),
            description_zh: Some("MQTT 消息传输使用的服务质量等级".to_string()),
            description_en: Some("MQTT QoS level for message delivery".to_string()),
            attribute: ParamAttribute::Optional,
            ty: ParamType::Select,
            default: Some(serde_json::json!(DEFAULT_QOS)),
            valid: None,
            options: Some(vec![
                ParamOption { value: serde_json::json!(0), label: Some("QoS 0".to_string()), label_zh: Some("QoS 0".to_string()), label_en: Some("QoS 0".to_string()) },
                ParamOption { value: serde_json::json!(1), label: Some("QoS 1".to_string()), label_zh: Some("QoS 1".to_string()), label_en: Some("QoS 1".to_string()) },
                ParamOption { value: serde_json::json!(2), label: Some("QoS 2".to_string()), label_zh: Some("QoS 2".to_string()), label_en: Some("QoS 2".to_string()) },
            ]),
            ..Default::default()
        })
        .param(ParamSchema {
            name: "retain".to_string(),
            name_zh: Some("保留消息".to_string()),
            name_en: Some("Retain".to_string()),
            description: Some("Whether to set message as retained".to_string()),
            description_zh: Some("是否将消息设为保留（retain）".to_string()),
            description_en: Some("Whether to set message as retained".to_string()),
            attribute: ParamAttribute::Optional,
            ty: ParamType::Bool,
            default: Some(serde_json::json!(false)),
            valid: None,
            ..Default::default()
        })
        .param(ParamSchema {
            name: "upload_format".to_string(),
            name_zh: Some("上报数据格式".to_string()),
            name_en: Some("Upload Format".to_string()),
            description: Some("Upload format: values_format, tags_format, ecp_format, group_data, raw_data.".to_string()),
            description_zh: Some("上报 JSON 格式：values_format/tags_format/ecp_format，及 group_data、raw_data.".to_string()),
            description_en: Some("Upload format: values_format, tags_format, ecp_format, group_data, raw_data.".to_string()),
            attribute: ParamAttribute::Optional,
            ty: ParamType::Select,
            default: Some(serde_json::json!(UPLOAD_FORMAT_VALUES_FORMAT)),
            valid: None,
            options: Some(vec![
                ParamOption { value: serde_json::json!(UPLOAD_FORMAT_VALUES_FORMAT), label: Some("Values Format".to_string()), label_zh: Some("Values 格式 (timestamp/node/group/values/errors/metas)".to_string()), label_en: Some("Values (timestamp, node, group, values, errors, metas)".to_string()) },
                ParamOption { value: serde_json::json!(UPLOAD_FORMAT_TAGS_FORMAT), label: Some("Tags Format".to_string()), label_zh: Some("Tags 格式 (timestamp/node/group/tags:[{name,value}])".to_string()), label_en: Some("Tags (timestamp, node, group, tags array)".to_string()) },
                ParamOption { value: serde_json::json!(UPLOAD_FORMAT_ECP_FORMAT), label: Some("ECP Format".to_string()), label_zh: Some("ECP 格式 (tags 含 type: 1 bool/2 int/3 float/4 string)".to_string()), label_en: Some("ECP (tags with type: 1 bool, 2 int, 3 float, 4 string)".to_string()) },
                ParamOption { value: serde_json::json!(UPLOAD_FORMAT_GROUP_DATA), label: Some("GroupData".to_string()), label_zh: Some("完整 GroupData".to_string()), label_en: Some("Full GroupData".to_string()) },
                ParamOption { value: serde_json::json!(UPLOAD_FORMAT_RAW_DATA), label: Some("Raw Data".to_string()), label_zh: Some("Raw Data (values 为 tag->[num])".to_string()), label_en: Some("Raw Data (values as tag->[num])".to_string()) },
            ]),
            ..Default::default()
        })
        .param(ParamSchema {
            name: "keep_alive_secs".to_string(),
            name_zh: Some("保活时间(秒)".to_string()),
            name_en: Some("Keep alive (sec)".to_string()),
            description: Some("MQTT Keep Alive interval in seconds".to_string()),
            description_zh: Some("MQTT Keep Alive 间隔，单位秒".to_string()),
            description_en: Some("MQTT Keep Alive interval in seconds".to_string()),
            attribute: ParamAttribute::Optional,
            ty: ParamType::Int,
            default: Some(serde_json::json!(DEFAULT_KEEP_ALIVE_SECS as i64)),
            valid: None,
            ..Default::default()
        })
        .param(ParamSchema {
            name: "username".to_string(),
            name_zh: Some("用户名".to_string()),
            name_en: Some("Username".to_string()),
            description: Some("Broker username for authentication".to_string()),
            description_zh: Some("Broker 认证用户名，可选".to_string()),
            description_en: Some("Broker username for authentication".to_string()),
            attribute: ParamAttribute::Optional,
            ty: ParamType::String,
            default: None,
            valid: Some(ParamValid { min: None, max: None, regex: None, length: Some(255) }),
            ..Default::default()
        })
        .param(ParamSchema {
            name: "password".to_string(),
            name_zh: Some("密码".to_string()),
            name_en: Some("Password".to_string()),
            description: Some("Broker password for authentication".to_string()),
            description_zh: Some("Broker 认证密码，可选".to_string()),
            description_en: Some("Broker password for authentication".to_string()),
            attribute: ParamAttribute::Optional,
            ty: ParamType::String,
            default: None,
            valid: Some(ParamValid { min: None, max: None, regex: None, length: Some(255) }),
            ..Default::default()
        })
        .param(ParamSchema {
            name: "cache_memory_size".to_string(),
            name_zh: Some("缓存内存大小".to_string()),
            name_en: Some("Cache Memory Size".to_string()),
            description: Some("Max in-memory cache size (message count) when MQTT connection exception occurs.".to_string()),
            description_zh: Some("MQTT 断开时暂存的最大条数；写满后丢弃最旧的并计入 queue_dropped_overflow。".to_string()),
            description_en: Some("Max in-memory cache size (message count) when MQTT connection exception occurs.".to_string()),
            attribute: ParamAttribute::Optional,
            ty: ParamType::Int,
            default: Some(serde_json::json!(gateway_plugin_common::DEFAULT_CACHE_MEMORY_SIZE)),
            valid: None,
            ..Default::default()
        })
        .param(ParamSchema {
            name: "cache_persist".to_string(),
            name_zh: Some("离线队列落盘".to_string()),
            name_en: Some("Persist Offline Queue".to_string()),
            description: Some("Persist the offline queue to disk so buffered messages survive a restart.".to_string()),
            description_zh: Some("断开期间的待发消息是否写入磁盘；开启后网关重启不会丢失。".to_string()),
            description_en: Some("Persist the offline queue to disk so buffered messages survive a restart.".to_string()),
            attribute: ParamAttribute::Optional,
            ty: ParamType::Bool,
            default: Some(serde_json::json!(true)),
            valid: None,
            ..Default::default()
        })
        .param(ParamSchema {
            name: "cache_dir".to_string(),
            name_zh: Some("离线队列目录".to_string()),
            name_en: Some("Offline Queue Directory".to_string()),
            description: Some("Directory for per-node offline queue files.".to_string()),
            description_zh: Some(format!("每个节点一个 <节点ID>.queue 文件，默认 {}", gateway_plugin_common::DEFAULT_CACHE_DIR).to_string()),
            description_en: Some("Directory for per-node offline queue files.".to_string()),
            attribute: ParamAttribute::Optional,
            ty: ParamType::String,
            default: Some(serde_json::json!(gateway_plugin_common::DEFAULT_CACHE_DIR)),
            valid: None,
            ..Default::default()
        })
        .param(ParamSchema {
            name: "cache_sync_interval_ms".to_string(),
            name_zh: Some("缓存消息重传间隔（MS）".to_string()),
            name_en: Some("Cache Sync Interval (MS)".to_string()),
            description: Some("Interval in milliseconds for replaying cached messages after reconnect.".to_string()),
            description_zh: Some("恢复连接后补发缓存消息的时间间隔，单位毫秒".to_string()),
            description_en: Some("Interval in milliseconds for replaying cached messages after reconnect.".to_string()),
            attribute: ParamAttribute::Optional,
            ty: ParamType::Int,
            default: Some(serde_json::json!(gateway_plugin_common::DEFAULT_CACHE_SYNC_INTERVAL_MS as i64)),
            valid: Some(ParamValid { min: Some(10), max: Some(120_000), regex: None, length: None }),
            ..Default::default()
        })
        .param(ParamSchema {
            name: "ssl".to_string(),
            name_zh: Some("SSL".to_string()),
            name_en: Some("SSL".to_string()),
            description: Some("Enable SSL connection".to_string()),
            description_zh: Some("是否启用 SSL 连接".to_string()),
            description_en: Some("Enable SSL connection".to_string()),
            attribute: ParamAttribute::Optional,
            ty: ParamType::Bool,
            default: Some(serde_json::json!(false)),
            valid: None,
            ..Default::default()
        })
        .param(ParamSchema {
            name: "ca_file".to_string(),
            name_zh: Some("CA 证书".to_string()),
            name_en: Some("CA".to_string()),
            description: Some("CA certificate which signs the server certificate".to_string()),
            description_zh: Some("签发服务器证书的 CA 证书".to_string()),
            description_en: Some("CA certificate which signs the server certificate".to_string()),
            attribute: ParamAttribute::Optional,
            ty: ParamType::File,
            default: None,
            valid: None,
            depends_on: Some("ssl".to_string()),
            depends_value: Some(serde_json::json!(true)),
            ..Default::default()
        })
        .param(ParamSchema {
            name: "client_cert_file".to_string(),
            name_zh: Some("客户端证书".to_string()),
            name_en: Some("Client Cert".to_string()),
            description: Some("Client x509 certificate when using two way authentication".to_string()),
            description_zh: Some("使用双向认证时，客户端的 x509 证书".to_string()),
            description_en: Some("Client x509 certificate when using two way authentication".to_string()),
            attribute: ParamAttribute::Optional,
            ty: ParamType::File,
            default: None,
            valid: None,
            depends_on: Some("ssl".to_string()),
            depends_value: Some(serde_json::json!(true)),
            ..Default::default()
        })
        .param(ParamSchema {
            name: "client_key_file".to_string(),
            name_zh: Some("客户端私钥".to_string()),
            name_en: Some("Client Private Key".to_string()),
            description: Some("Client private key when using two way authentication".to_string()),
            description_zh: Some("使用双向认证时，客户端的私钥".to_string()),
            description_en: Some("Client private key when using two way authentication".to_string()),
            attribute: ParamAttribute::Optional,
            ty: ParamType::File,
            default: None,
            valid: None,
            depends_on: Some("ssl".to_string()),
            depends_value: Some(serde_json::json!(true)),
            ..Default::default()
        })
}
