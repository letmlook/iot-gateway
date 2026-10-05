//! S7 插件 C ABI 导出。
//!
//! 全部符号由 `gateway_sdk::export_south_plugin!` 生成，不再手写。

gateway_sdk::export_south_plugin!(crate::S7Plugin);
