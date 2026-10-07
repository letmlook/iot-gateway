//! FFI 导出符号存在性（缺陷④回归，docs/联调记录-2026-10-07.md §5.4）。
//!
//! 此前 plugin-kafka 的 `src/ffi.rs` 只有一行占位注释，cdylib 无任何导出符号，
//! 宿主加载报 `no south/north symbols found`——插件无法以任何形态被网关加载。
//! 本测试把两件事钉死：
//!
//! 1. **逐符号存在性**：构建出的 `plugin_kafka` cdylib 必须导出宿主 loader
//!    （gateway-core `try_north`）要求的全部北向符号 + SDK 公共符号（free_string /
//!    abi_version），且 ABI 版本与宿主一致；
//! 2. **真实宿主加载链路**：`PluginLoader::load` 必须能加载 cdylib 并注册为北向
//!    插件 `kafka`，经 FFI 的建节点 / 查状态 / 关节点生命周期可用（夹具与加载方式
//!    参照 `gateway-core/tests/ffi_fault_isolation.rs`）。
//!
//! 运行方式：`cargo test -p plugin-kafka --features ffi`（ffi feature 让本 crate
//! 同时产出带导出符号的 cdylib；与 plugin-mqtt 等一致，builtin rlib 链接形态不携带
//! 导出符号，故默认构建下本测试不编译，`cargo test --workspace` 不受影响）。
//!
//! 平台说明：panic 注入类夹具（ffi_fault_isolation.rs）在 Windows 因夹具卸载崩溃
//! 被禁用；本测试的插件行为良好（无故障注入、stub 无后台线程），与网关在 Windows
//! inproc 加载 plugin_sim.dll 的既有行为同构，故不设平台门。

#![cfg(feature = "ffi")]

use gateway_core::{Manager, PluginLoader};
use std::path::{Path, PathBuf};

/// 测试可执行文件位于 `<target>/<profile>/deps/<name>-<hash>`，上两级即 target profile 目录
fn target_dir() -> PathBuf {
    let exe = std::env::current_exe().expect("current_exe");
    exe.parent()
        .and_then(|p| p.parent())
        .map(Path::to_path_buf)
        .expect("locate target profile dir")
}

/// 定位本 crate 的 cdylib（cargo 对 cdylib 产物文件名用 `_` 连接 crate 名，
/// 与 ffi_fault_isolation.rs 的 `plugin_faulty` 夹具同规则）
fn kafka_dylib() -> PathBuf {
    let file = format!(
        "{}plugin_kafka{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );
    let path = target_dir().join(&file);
    assert!(
        path.exists(),
        "缺少 cdylib {}；请先执行 `cargo build -p plugin-kafka --features ffi` \
         （或直接 `cargo test -p plugin-kafka --features ffi`，会顺带构建）",
        path.display()
    );
    path
}

/// 宿主 loader 必需的全部北向符号（gateway-core loader.rs `try_north` 强制要求项，
/// 加上宏生成的全量清单；`gateway_north_plugin_set_log` 为可选符号，不在此列）
const REQUIRED_NORTH_SYMBOLS: &[&[u8]] = &[
    b"gateway_north_plugin_create\0",
    b"gateway_north_plugin_destroy\0",
    b"gateway_north_plugin_meta\0",
    b"gateway_north_plugin_open\0",
    b"gateway_north_plugin_close\0",
    b"gateway_north_plugin_init\0",
    b"gateway_north_plugin_uninit\0",
    b"gateway_north_plugin_start\0",
    b"gateway_north_plugin_stop\0",
    b"gateway_north_plugin_setting\0",
    b"gateway_north_plugin_set_subscriptions\0",
    b"gateway_north_plugin_on_group_data\0",
    b"gateway_north_plugin_connection_status\0",
    b"gateway_north_plugin_config_schema\0",
    // SDK 级公共符号：宿主加载入口（ABI 校验）与跨边界字符串释放
    b"gateway_plugin_free_string\0",
    b"gateway_plugin_abi_version\0",
];

/// 逐符号存在性：每个必需符号都必须能被解析，ABI 版本必须与宿主 SDK 一致
/// （loader.rs 在加载期对 ABI 不符直接拒绝）。
#[test]
fn cdylib_exports_all_required_north_symbols() {
    let lib = unsafe { libloading::Library::new(kafka_dylib()) }
        .expect("plugin_kafka cdylib must be loadable");
    for sym in REQUIRED_NORTH_SYMBOLS {
        let found: Result<libloading::Symbol<unsafe extern "C-unwind" fn()>, _> =
            unsafe { lib.get(sym) };
        assert!(
            found.is_ok(),
            "missing export symbol: {}",
            String::from_utf8_lossy(sym)
        );
    }
    let abi: libloading::Symbol<unsafe extern "C-unwind" fn() -> u32> =
        unsafe { lib.get(b"gateway_plugin_abi_version\0") }.expect("abi symbol must be callable");
    assert_eq!(
        unsafe { abi() },
        gateway_sdk::ffi::FFI_ABI_VERSION,
        "cdylib ABI version must match the host SDK"
    );
}

/// 真实宿主加载链路：`PluginLoader::load` 必须把 cdylib 注册为北向插件 `kafka`
/// （缺陷④下同一路径报 `no south/north symbols found`），且经 FFI 的
/// 建节点 / 查 connection_status / 关节点生命周期可用。
#[tokio::test]
async fn host_loader_registers_kafka_and_node_lifecycle_works() {
    let dir = std::env::temp_dir().join(format!("gw-kafka-ffi-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).expect("create fixture dir");
    let src = kafka_dylib();
    let dst = dir.join(src.file_name().expect("file name"));
    std::fs::copy(&src, &dst).expect("copy cdylib into fixture dir");

    let mut mgr = Manager::new();
    // Loader 持有动态库句柄，必须比 Manager 活得久（Manager 析构回调插件 destroy，
    // 析构顺序见 ffi_fault_isolation.rs 的 load_fixture 注释）
    let loader = PluginLoader::load(&dir, &mut mgr).expect("load plugin_kafka cdylib");
    let plugin = mgr
        .north_plugin("kafka")
        .expect("kafka must register as a north plugin via the host loader");
    assert_eq!(plugin.meta().name, "kafka");
    assert_eq!(plugin.meta().kind, gateway_sdk::types::PluginKind::North);

    // 经 FFI 的节点生命周期：stub 构建（无 kafka-client）下 open 后状态即可查
    let node_id = gateway_sdk::NodeId::new();
    let mut config = gateway_sdk::PluginConfig::new();
    config.insert("brokers".to_string(), serde_json::json!("localhost:9092"));
    plugin
        .open(node_id, config)
        .await
        .expect("open via FFI must succeed");
    let status = plugin
        .connection_status(node_id)
        .await
        .expect("connection_status via FFI must be available after open");
    assert_eq!(status["connected"], serde_json::json!(false));
    assert!(status.get("dropped_no_client").is_some());
    plugin
        .close(node_id)
        .await
        .expect("close via FFI must succeed");

    drop(plugin);
    drop(mgr);
    drop(loader);
    let _ = std::fs::remove_dir_all(&dir);
}
