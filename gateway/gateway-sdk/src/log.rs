//! 节点日志：主程序与插件中按节点打印日志，由网关统一收集并按节点分文件输出。
//!
//! - **主程序 / 内置插件**：直接调用 `node_log::info(node_id, "msg")` 等，使用 tracing 发出事件，
//!   网关的 NodeFileLayer 会按**节点名称**写入对应 `{节点名称}.log`（未配置名称时用 unnamed-xxx）。
//! - **.so 插件**：在 `open` 后通过可选符号 `gateway_south_plugin_set_log` / `gateway_north_plugin_set_log`
//!   接收宿主传入的日志回调，在插件内调用该回调打印节点日志。

use crate::NodeId;
use std::fmt::Display;
use tracing::Level;

/// 日志级别（与 tracing 对应，FFI 用 u8：0=Error, 1=Warn, 2=Info, 3=Debug, 4=Trace）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum NodeLogLevel {
    Error = 0,
    Warn = 1,
    Info = 2,
    Debug = 3,
    Trace = 4,
}

impl From<NodeLogLevel> for Level {
    fn from(l: NodeLogLevel) -> Self {
        match l {
            NodeLogLevel::Error => Level::ERROR,
            NodeLogLevel::Warn => Level::WARN,
            NodeLogLevel::Info => Level::INFO,
            NodeLogLevel::Debug => Level::DEBUG,
            NodeLogLevel::Trace => Level::TRACE,
        }
    }
}

/// 按节点记录一条日志（主程序或内置插件使用）。
/// 事件带 `node_id` 和 `message` 字段，网关的 NodeFileLayer 会按节点名称写入 `{log_dir_nodes}/{节点名称}.log`。
#[inline]
pub fn node_log(level: NodeLogLevel, node_id: NodeId, message: impl Display) {
    let node_id_str = node_id.0.to_string();
    let msg = message.to_string();
    match level {
        NodeLogLevel::Error => {
            tracing::event!(Level::ERROR, node_id = %node_id_str, message = %msg)
        }
        NodeLogLevel::Warn => tracing::event!(Level::WARN, node_id = %node_id_str, message = %msg),
        NodeLogLevel::Info => tracing::event!(Level::INFO, node_id = %node_id_str, message = %msg),
        NodeLogLevel::Debug => {
            tracing::event!(Level::DEBUG, node_id = %node_id_str, message = %msg)
        }
        NodeLogLevel::Trace => {
            tracing::event!(Level::TRACE, node_id = %node_id_str, message = %msg)
        }
    }
}

/// 节点级 ERROR 日志
#[inline]
pub fn error(node_id: NodeId, message: impl Display) {
    node_log(NodeLogLevel::Error, node_id, message);
}

/// 节点级 WARN 日志
#[inline]
pub fn warn(node_id: NodeId, message: impl Display) {
    node_log(NodeLogLevel::Warn, node_id, message);
}

/// 节点级 INFO 日志
#[inline]
pub fn info(node_id: NodeId, message: impl Display) {
    node_log(NodeLogLevel::Info, node_id, message);
}

/// 节点级 DEBUG 日志
#[inline]
pub fn debug(node_id: NodeId, message: impl Display) {
    node_log(NodeLogLevel::Debug, node_id, message);
}

/// 节点级 TRACE 日志
#[inline]
pub fn trace(node_id: NodeId, message: impl Display) {
    node_log(NodeLogLevel::Trace, node_id, message);
}
