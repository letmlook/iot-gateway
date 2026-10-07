//! 北向插件 C ABI 导出。
//!
//! 全部符号由 `gateway_sdk::export_north_plugin!` 生成，不再手写：
//! - **panic 隔离**：每个导出函数在本 crate 内 `catch_unwind`，把插件 panic 转成一次失败的调用。
//!   这一点不可省略——插件与宿主各自静态链接了一份 Rust 运行时，异常一旦越过 C ABI 边界，
//!   宿主侧会以 `Rust cannot catch foreign exceptions` 直接 abort 整个进程。
//! - **运行时管理**：每次调用创建单线程 runtime，宿主保证调用发生在独立线程上（见 core `run_sync`）。
//!
//! 缺陷④（docs/联调记录-2026-10-07.md §5.4）：本文件此前只有一行占位注释，cdylib 无任何
//! 导出符号，宿主加载报 `no south/north symbols found`。现按 plugin-mqtt 同款方式真实导出。

gateway_sdk::export_north_plugin!(crate::KafkaPlugin);
