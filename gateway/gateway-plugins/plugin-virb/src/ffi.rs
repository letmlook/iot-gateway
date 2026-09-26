//! 南向插件 C ABI 导出。
//!
//! 全部符号由 `gateway_sdk::export_south_plugin!` 生成，不再手写：
//! - **panic 隔离**：每个导出函数在本 crate 内 `catch_unwind`，把插件 panic 转成一次失败的调用。
//!   这一点不可省略——插件与宿主各自静态链接了一份 Rust 运行时，异常一旦越过 C ABI 边界，
//!   宿主侧会以 `Rust cannot catch foreign exceptions` 直接 abort 整个进程。
//! - **运行时管理**：每次调用创建单线程 runtime，宿主保证调用发生在独立线程上（见 core `run_sync`）。

gateway_sdk::export_south_plugin!(crate::VirbPlugin);
