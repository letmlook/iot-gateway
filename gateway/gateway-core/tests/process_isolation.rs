//! 进程级插件隔离测试。
//!
//! 验证的核心主张：**插件进程崩溃不会带走网关**。
//!
//! 用 `plugin-faulty` 夹具制造三类故障：
//! | 场景 | 进程内模式 | 进程隔离模式（本测试） |
//! |---|---|---|
//! | poll_group 内 panic | 插件侧 `catch_unwind` 兜住，返回 Err | 同左（跨进程同样返回 Err） |
//! | poll_group 内 `abort` | **整个网关进程被带走** | 只有子进程死，网关存活 |
//! | abort 之后的下一次调用 | —— | 自动重启子进程并重放节点生命周期，调用恢复正常 |
//!
//! 依赖夹具动态库与宿主可执行文件，二者都由 workspace 构建产出
//! （`cargo test --workspace` 会顺带构建）。

use gateway_core::proc_plugin::ProcessPluginLoader;
use gateway_core::Manager;
use gateway_sdk::{NodeId, PluginConfig};
use std::path::{Path, PathBuf};

/// 测试可执行文件位于 `<target>/<profile>/deps/<name>-<hash>`，上两级即 target profile 目录
fn target_dir() -> PathBuf {
    std::env::current_exe()
        .expect("current_exe")
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .expect("locate target profile dir")
}

fn host_bin() -> PathBuf {
    let name = if cfg!(windows) {
        "gateway-plugin-host.exe"
    } else {
        "gateway-plugin-host"
    };
    let p = target_dir().join(name);
    assert!(
        p.exists(),
        "缺少插件宿主可执行文件 {}；请先执行 `cargo build -p gateway-plugin-host`\
         （或直接 `cargo test --workspace`）",
        p.display()
    );
    p
}

fn faulty_dylib() -> PathBuf {
    let file = format!(
        "{}{}{}",
        std::env::consts::DLL_PREFIX,
        "plugin_faulty",
        std::env::consts::DLL_SUFFIX
    );
    let p = target_dir().join(file);
    assert!(
        p.exists(),
        "缺少夹具动态库 {}；请先执行 `cargo build -p plugin-faulty`（或 `cargo test --workspace`）",
        p.display()
    );
    p
}

fn cfg_with(panic_on: Option<&str>) -> PluginConfig {
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
async fn plugin_process_crash_does_not_kill_the_host() {
    // 1) 通过独立进程加载故障夹具
    let dir = std::env::temp_dir().join(format!("gw-isolation-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).expect("create temp plugin dir");
    let src = faulty_dylib();
    std::fs::copy(&src, dir.join(src.file_name().unwrap())).expect("copy fixture");

    let mut mgr = Manager::new();
    let mut loader = ProcessPluginLoader::new(host_bin());
    loader
        .load(&dir, &mut mgr)
        .await
        .expect("scan plugin dir (isolated)");

    let plugin = mgr
        .south_plugin("faulty")
        .expect("faulty plugin should be registered from an isolated process");
    assert_eq!(plugin.meta().name, "faulty");
    assert_eq!(loader.process_count(), 1, "one plugin ⇒ one child process");
    assert!(
        loader.total_restarts() >= 1,
        "initial spawn should be counted"
    );

    let gid = gateway_sdk::GroupId::new();
    let tags = vec![gateway_sdk::Tag::new("t1", "dummy", gid)];

    // 2) 正常调用应当穿过进程边界并返回数据
    let healthy = NodeId::new();
    plugin
        .open(healthy, cfg_with(None))
        .await
        .expect("open over rpc");
    let values = plugin
        .poll_group(healthy, gid, &tags)
        .await
        .expect("healthy poll over rpc");
    assert_eq!(values.len(), 1, "value should survive the process boundary");

    // 3) 插件 panic：与进程内模式一样返回 Err，子进程仍健康
    let panicky = NodeId::new();
    plugin
        .open(panicky, cfg_with(Some("poll")))
        .await
        .expect("open node with poll fault");
    let err = plugin
        .poll_group(panicky, gid, &tags)
        .await
        .expect_err("a panicking plugin must report an error, not take down the caller");
    assert!(
        err.to_string().contains("panicked"),
        "error should carry the plugin panic, got: {}",
        err
    );
    // 子进程仍能服务其他节点
    let values = plugin
        .poll_group(healthy, gid, &tags)
        .await
        .expect("host process should still be alive after a plugin panic");
    assert_eq!(values.len(), 1);

    // 4) 插件硬崩溃（abort）：子进程死，**当前测试进程必须活着**
    let aborter = NodeId::new();
    plugin
        .open(aborter, cfg_with(Some("abort")))
        .await
        .expect("open node with abort fault");
    let err = plugin
        .poll_group(aborter, gid, &tags)
        .await
        .expect_err("a crashing plugin process must surface as an error");
    assert!(
        err.to_string().contains("exited") || err.to_string().contains("closed"),
        "error should mention the host process going away, got: {}",
        err
    );

    // 到这里测试进程还活着，本身就是隔离生效的证据：
    // 同样的夹具在进程内模式下执行到这一步会让整个测试进程 abort。

    // 5) 自我修复：下一次调用自动重启子进程，并重放此前 open 过的节点
    let values = plugin
        .poll_group(healthy, gid, &tags)
        .await
        .expect("plugin host should be restarted and the node lifecycle replayed");
    assert_eq!(
        values.len(),
        1,
        "after restart the previously opened node must work again (lifecycle replay)"
    );
    assert!(
        loader.total_restarts() >= 2,
        "a restart should be counted, got {}",
        loader.total_restarts()
    );

    let _ = std::fs::remove_dir_all(&dir);
}
