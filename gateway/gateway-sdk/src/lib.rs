//! # Gateway SDK
//!
//! 南/北向插件共享的类型与 trait 定义，本系统插件体系。
//! 插件可静态链接或编译为 .so（cdylib），经 `gateway-sdk/ffi` ABI 动态加载。

pub mod address;
pub mod error;
pub mod ffi;
pub mod log;
pub mod messages;
pub mod metrics;
pub mod plugin;
pub mod schema;
pub mod types;

pub use address::{SouthAddress, S7Area};
pub use error::{PluginError, PluginErrorCode, PluginResult};
pub use ffi::{parse_result, ptr_to_string, FfiPluginMeta, FfiResult};
pub use log::{debug, error, info, node_log, trace, warn, NodeLogLevel};
pub use messages::{GroupData, GroupSubscription, TagRead, TagWrite};
pub use plugin::{NorthPlugin, Operable, PluginInfo, PluginMeta, SouthPlugin};
pub use schema::{ConfigSchema, ParamAttribute, ParamOption, ParamSchema, ParamType, ParamValid, TagRegexEntry, TagSchema};
pub use metrics::OperatorMetrics;
pub use types::{
    DataType, DataValue, Group, GroupId, NodeId, NodeKind, NodeState, PipelineData, PluginConfig, PluginKind,
    Tag, TagAttr, TagId,
};
