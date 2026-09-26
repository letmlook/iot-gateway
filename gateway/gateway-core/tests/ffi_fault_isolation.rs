//! FFI 故障注入（集成测试）：证明插件异常不会拖垮宿主进程。
//!
//! 背景：历史上 FFI 调用直接在 tokio worker 线程上执行，插件一旦 panic（或在其内部
//! `block_on`）会 abort 整个网关。修复后每次调用在独立线程执行、`extern "C-unwind"`
//! 让 panic 回到宿主被捕获。本测试把这两点钉死：
//!
//! | 场景 | 期望 |
//! |---|---|
//! | `poll_group` 内 panic | 该次调用返回 Err，宿主存活，插件后续仍可用 |
//! | `write_tags` 内 panic | 同上 |
//! | `open` 内 panic | 节点创建拿到失败调用（可回滚），进程不退出 |
//! | `meta` 内 panic（加载期） | 插件被拒绝注册，进程照常启动 |
//! | ABI 版本不一致 | 加载阶段即被拒绝，不做运行期冒险 |
//!
//! 夹具动态库由 workspace 构建顺带产出（夹具 crate 的 `default` 已包含 `ffi` feature），
//! 因此 `cargo test --workspace` 可直接运行；单独跑 `-p gateway-core` 时若夹具缺失会给出提示。

use gateway_core::{Manager, PluginLoader};
use gateway_sdk::types::DataValue;
use gateway_sdk::{GroupId, NodeId, PluginConfig, Tag};
use std::path::{Path, PathBuf};

/// 测试可执行文件位于 `<target>/<profile>/deps/<name>-<hash>`，上两级即 target profile 目录
fn target_dir() -> PathBuf {
    let exe = std::env::current_exe().expect("current_exe");
    exe.parent()
        .and_then(|p| p.parent())
        .map(Path::to_path_buf)
        .expect("locate target profile dir")
}

/// 定位夹具动态库；缺失时给出可执行的修复命令，而不是静默跳过
fn fixture_dylib(crate_name: &str) -> PathBuf {
    let file = format!(
        "{}{}{}",
        std::env::consts::DLL_PREFIX,
        crate_name,
        std::env::consts::DLL_SUFFIX
    );
    let path = target_dir().join(&file);
    assert!(
        path.exists(),
        "缺少夹具动态库 {}；请先执行 `cargo build -p plugin-faulty -p plugin-abi-mismatch`\
         （或直接运行 `cargo test --workspace`，会顺带构建）",
        path.display()
    );
    path
}

/// 把夹具复制进一个独立目录并加载，返回 (Manager, Loader, 临时目录)
///
/// Loader 持有动态库句柄，必须比 Manager 活得久（Manager 析构时会回调插件的 destroy），
/// 所以返回值里 Manager 在前、Loader 在后，依赖结构体字段的析构顺序保证这一点。
fn load_fixture(dylibs: &[PathBuf]) -> (Manager, PluginLoader, PathBuf) {
    let dir = std::env::temp_dir().join(format!("gw-ffi-fixture-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).expect("create fixture dir");
    for src in dylibs {
        let name = src.file_name().expect("file name");
        std::fs::copy(src, dir.join(name)).expect("copy fixture dylib");
    }
    let mut mgr = Manager::new();
    let loader = PluginLoader::load(&dir, &mut mgr).expect("scan plugins dir");
    (mgr, loader, dir)
}

fn config_with(panic_on: Option<&str>) -> PluginConfig {
    let mut c = PluginConfig::new();
    if let Some(v) = panic_on {
        c.insert(
            "panic_on".to_string(),
            serde_json::Value::String(v.to_string()),
        );
    }
    c
}

#[tokio::test(flavor = "multi_thread")]
async fn plugin_faults_are_isolated_from_the_host() {
    // 场景 1：正常加载（对照基线——先证明夹具在没有注入时是可用插件）
    let (mgr, _loader, dir) = load_fixture(&[fixture_dylib("plugin_faulty")]);
    let plugin = mgr
        .south_plugin("faulty")
        .expect("faulty plugin should be registered");
    assert_eq!(plugin.meta().name, "faulty");

    let gid = GroupId::new();
    let tags = vec![Tag::new("t1", "dummy", gid)];

    let healthy = NodeId::new();
    plugin
        .open(healthy, config_with(None))
        .await
        .expect("open healthy node");
    let values = plugin
        .poll_group(healthy, gid, &tags)
        .await
        .expect("healthy poll works");
    assert_eq!(values.len(), 1);
    assert_eq!(values[0].1, DataValue::Int32(42));

    // 场景 2：poll_group 内 panic → 只影响这一次调用，宿主线程活下来
    let bad_poll = NodeId::new();
    plugin
        .open(bad_poll, config_with(Some("poll")))
        .await
        .expect("open node with poll fault");
    let err = plugin
        .poll_group(bad_poll, gid, &tags)
        .await
        .expect_err("panic inside poll_group must surface as Err, not abort the process");
    assert!(
        err.to_string().contains("panicked"),
        "error should report the plugin panic, got: {}",
        err
    );

    // 场景 3：panic 之后插件依然可用（没有把宿主状态或插件自身搞坏到不可用）
    let values = plugin
        .poll_group(healthy, gid, &tags)
        .await
        .expect("plugin must remain usable after an isolated panic");
    assert_eq!(values.len(), 1);

    // 场景 4：write_tags 内 panic
    let bad_write = NodeId::new();
    plugin
        .open(bad_write, config_with(Some("write")))
        .await
        .expect("open node with write fault");
    let err = plugin
        .write_tags(bad_write, &[(tags[0].clone(), DataValue::Int32(7))])
        .await
        .expect_err("panic inside write_tags must surface as Err");
    assert!(err.to_string().contains("panicked"), "got: {}", err);

    // 场景 5：open 内 panic → 节点创建路径拿到失败调用，进程不退出
    let err = plugin
        .open(NodeId::new(), config_with(Some("open")))
        .await
        .expect_err("panic inside open must surface as Err");
    assert!(err.to_string().contains("panicked"), "got: {}", err);

    // 场景 6：start 内 panic
    let bad_start = NodeId::new();
    plugin
        .open(bad_start, config_with(Some("start")))
        .await
        .expect("open node with start fault");
    let err = plugin
        .start(bad_start)
        .await
        .expect_err("panic inside start must surface as Err");
    assert!(err.to_string().contains("panicked"), "got: {}", err);

    // 场景 7：加载期 meta panic → 拒绝注册（而不是以占位名 "?" 注册一个坏插件）
    std::env::set_var("FAULTY_PLUGIN_PANIC_META", "1");
    let (mgr2, _loader2, dir2) = load_fixture(&[fixture_dylib("plugin_faulty")]);
    std::env::remove_var("FAULTY_PLUGIN_PANIC_META");
    assert!(
        mgr2.south_plugin("faulty").is_none(),
        "plugin whose meta panicked must not be registered"
    );
    assert!(
        mgr2.south_plugins().is_empty(),
        "no plugin should be registered when meta is unusable"
    );

    // 场景 8：ABI 不一致 → 加载阶段被拒绝
    let (mgr3, _loader3, dir3) = load_fixture(&[fixture_dylib("plugin_abi_mismatch")]);
    assert!(
        mgr3.south_plugins().is_empty() && mgr3.north_plugins().is_empty(),
        "plugin with a mismatched ABI version must be refused at load time"
    );

    for d in [dir, dir2, dir3] {
        let _ = std::fs::remove_dir_all(d);
    }
}
