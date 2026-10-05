//! gateway-plugin-common：北向插件共享基础设施 crate。
//!
//! 所有北向插件共享的队列、格式化、配置辅助、HTTP 客户端与行协议编码器。
//! 四个新插件（http / influxdb / tdengine / kafka）与 plugin-mqtt 共用此 crate。
//!
//! # Feature flags
//!
//! - `http`：启用 `reqwest` HTTP 客户端与 `http.rs` 模块。

pub mod config;
pub mod format;
pub mod queue;

#[cfg(feature = "http")]
pub mod http;

pub mod line;

pub use config::{
    config_bool, config_str, config_u16, config_u64, config_usize, queue_path_for_node,
    DEFAULT_CACHE_DIR, DEFAULT_CACHE_MEMORY_SIZE, DEFAULT_CACHE_SYNC_INTERVAL_MS,
};
pub use format::{
    data_value_ecp_type, data_value_to_json_scalar, data_value_to_number_array, payload_for_format,
    tag_key, topic_from_template, UPLOAD_FORMAT_ECP_FORMAT, UPLOAD_FORMAT_GROUP_DATA,
    UPLOAD_FORMAT_RAW_DATA, UPLOAD_FORMAT_TAGS_FORMAT, UPLOAD_FORMAT_VALUES_FORMAT,
};
pub use line::{data_value_to_line_field, encode, LineField, LinePoint};
pub use queue::{OfflineQueue, QueueStats, Record};
