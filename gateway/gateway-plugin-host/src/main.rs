//! 插件宿主进程：加载**一个**插件动态库，并通过 stdin/stdout 上的长度前缀 JSON 帧对外提供调用。
//!
//! 由网关的「进程级隔离」模式拉起（见 `gateway-core/src/proc_plugin.rs`）：
//!
//! ```text
//! gateway ──帧──▶ plugin-host ──FFI──▶ plugin.so
//!         ◀──────
//! ```
//!
//! # 为什么单独一个进程
//!
//! 进程内模式下，插件里的 panic 可以被插件侧的 `catch_unwind` 兜住，但 `abort`、段错误、
//! 内存越界、死循环这些故障无法兜住，会带走整个网关。放进独立进程后，最坏情况只是这个
//! 子进程死掉，网关检测到 EOF 后重启它。
//!
//! # 约定
//!
//! - **stdout 只承载协议帧**，插件日志请走 stderr（网关会继承子进程 stderr，不会丢日志）；
//! - 请求 `{"seq":1,"op":"poll_group","args":{...}}`，响应 `{"seq":1,"ok":true,"data":...}`
//!   或 `{"seq":1,"ok":false,"err":"..."}`；
//! - 启动失败（插件加载失败 / ABI 不匹配）直接打印到 stderr 并退出，网关会在握手时报错。

use gateway_core::proc_plugin::protocol::{write_frame, Request, Response};
use gateway_sdk::{
    DataValue, Group, GroupData, GroupId, GroupSubscription, NodeId, PluginConfig, SouthPlugin,
    Tag, TagId,
};
use serde_json::Value;
use std::sync::Arc;
use tokio::io::{stdout, AsyncWriteExt};
use tokio::sync::{mpsc, Semaphore};

/// 同时在处理的请求上限：网关侧的采集并发上限远小于此，这里只是防止异常客户端打爆内存
const MAX_INFLIGHT: usize = 64;

fn main() {
    let mut args = std::env::args().skip(1);
    let mut plugin: Option<String> = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--plugin" => plugin = args.next(),
            other => {
                eprintln!("gateway-plugin-host: unknown argument '{}'", other);
                std::process::exit(2);
            }
        }
    }
    let Some(plugin_path) = plugin else {
        eprintln!("usage: gateway-plugin-host --plugin <path-to-plugin.so>");
        std::process::exit(2);
    };

    init_tracing();

    let loaded =
        match gateway_core::PluginLoader::load_plugin_file(std::path::Path::new(&plugin_path)) {
            Ok(l) => l,
            Err(e) => {
                eprintln!(
                    "gateway-plugin-host: failed to load plugin '{}': {}",
                    plugin_path, e
                );
                std::process::exit(1);
            }
        };

    // 进程生命周期内常驻：适配器内部持有库里函数指针，库一旦卸载即悬空。
    // 本进程只服务一个插件、随网关退出而结束，因此泄漏是最简单且正确的做法。
    let loaded: &'static gateway_core::LoadedPlugin = Box::leak(Box::new(loaded));
    let south = loaded.south();
    let north = loaded.north();
    let meta = serde_json::to_value(loaded.meta()).unwrap_or(Value::Null);

    tracing::info!(
        plugin = %plugin_path,
        kind = if south.is_some() { "south" } else { "north" },
        "plugin host ready"
    );

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("build runtime");
    rt.block_on(serve(south, north, meta));
}

fn init_tracing() {
    use tracing_subscriber::{fmt, EnvFilter};
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    // 强制写到 stderr：stdout 是协议通道
    let _ = fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_target(false)
        .try_init();
}

/// 主循环：读请求 → 并发分发 → 按 seq 回写响应
async fn serve(
    south: Option<Arc<dyn SouthPlugin>>,
    north: Option<Arc<dyn gateway_sdk::NorthPlugin>>,
    meta: Value,
) {
    let (tx, mut rx) = mpsc::channel::<Request>(MAX_INFLIGHT);
    let (resp_tx, mut resp_rx) = mpsc::channel::<Response>(MAX_INFLIGHT);
    let permits = Arc::new(Semaphore::new(MAX_INFLIGHT));

    // 读线程：stdin 是阻塞流，单独跑在阻塞线程池上
    std::thread::spawn(move || {
        use std::io::{stdin as block_stdin, Read};
        let mut stdin = block_stdin();
        let mut len_buf = [0u8; 4];
        // 读到 EOF（父进程关闭 stdin）即正常退出
        while stdin.read_exact(&mut len_buf).is_ok() {
            let len = u32::from_le_bytes(len_buf) as usize;
            if len == 0 || len > 64 * 1024 * 1024 {
                eprintln!("gateway-plugin-host: invalid frame length {}", len);
                break;
            }
            let mut buf = vec![0u8; len];
            if stdin.read_exact(&mut buf).is_err() {
                break;
            }
            match serde_json::from_slice::<Request>(&buf) {
                Ok(req) => {
                    if tx.blocking_send(req).is_err() {
                        break;
                    }
                }
                Err(e) => eprintln!("gateway-plugin-host: bad request frame: {}", e),
            }
        }
    });

    // 分发：每个请求一个任务，携带信号量许可
    {
        let south = south.clone();
        let north = north.clone();
        let meta = meta.clone();
        // 交给分发任务的发送端副本；外层这一份随后 drop，让写循环在全部任务结束后退出
        let resp_tx_task = resp_tx.clone();
        tokio::spawn(async move {
            while let Some(req) = rx.recv().await {
                let Ok(permit) = permits.clone().acquire_owned().await else {
                    break;
                };
                let south = south.clone();
                let north = north.clone();
                let meta = meta.clone();
                let resp_tx = resp_tx_task.clone();
                tokio::spawn(async move {
                    let _permit = permit;
                    let resp = match dispatch(&south, &north, &meta, &req).await {
                        Ok(data) => Response::ok(req.seq, data),
                        Err(e) => Response::err(req.seq, e),
                    };
                    let _ = resp_tx.send(resp).await;
                });
            }
        });
    }
    drop(resp_tx);

    // 写响应：单点写 stdout，天然串行
    let mut out = stdout();
    while let Some(resp) = resp_rx.recv().await {
        let bytes = match serde_json::to_vec(&resp) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("gateway-plugin-host: serialize response failed: {}", e);
                continue;
            }
        };
        if let Err(e) = write_frame(&mut out, &bytes).await {
            eprintln!("gateway-plugin-host: write frame failed: {}", e);
            break;
        }
    }
    let _ = out.flush().await;
}

/// 操作分发：协议 op ↔ 插件 trait 方法
async fn dispatch(
    south: &Option<Arc<dyn SouthPlugin>>,
    north: &Option<Arc<dyn gateway_sdk::NorthPlugin>>,
    meta: &Value,
    req: &Request,
) -> Result<Value, String> {
    let args = &req.args;
    match req.op.as_str() {
        "meta" => Ok(meta.clone()),

        // ---------- 南向 ----------
        "open" => {
            let p = need_south(south)?;
            let node_id: NodeId = take(args, "node_id")?;
            let config: PluginConfig = take(args, "config").unwrap_or_default();
            p.open(node_id, config)
                .await
                .map(|_| Value::Null)
                .map_err(|e| e.to_string())
        }
        "close" => {
            let p = need_south(south)?;
            let node_id: NodeId = take(args, "node_id")?;
            p.close(node_id)
                .await
                .map(|_| Value::Null)
                .map_err(|e| e.to_string())
        }
        "init" => {
            let p = need_south(south)?;
            let node_id: NodeId = take(args, "node_id")?;
            p.init(node_id)
                .await
                .map(|_| Value::Null)
                .map_err(|e| e.to_string())
        }
        "uninit" => {
            let p = need_south(south)?;
            let node_id: NodeId = take(args, "node_id")?;
            p.uninit(node_id)
                .await
                .map(|_| Value::Null)
                .map_err(|e| e.to_string())
        }
        "start" => match (south, north) {
            (Some(p), _) => p.start(take(args, "node_id")?).await,
            (None, Some(p)) => p.start(take(args, "node_id")?).await,
            _ => return Err("no plugin loaded".to_string()),
        }
        .map(|_| Value::Null)
        .map_err(|e| e.to_string()),
        "stop" => match (south, north) {
            (Some(p), _) => p.stop(take(args, "node_id")?).await,
            (None, Some(p)) => p.stop(take(args, "node_id")?).await,
            _ => return Err("no plugin loaded".to_string()),
        }
        .map(|_| Value::Null)
        .map_err(|e| e.to_string()),
        "setting" => {
            let node_id: NodeId = take(args, "node_id")?;
            let config: PluginConfig = take(args, "config").unwrap_or_default();
            match (south, north) {
                (Some(p), _) => p.setting(node_id, config).await,
                (None, Some(p)) => p.setting(node_id, config).await,
                _ => return Err("no plugin loaded".to_string()),
            }
            .map(|_| Value::Null)
            .map_err(|e| e.to_string())
        }
        "validate_tag" => {
            let p = need_south(south)?;
            let node_id: NodeId = take(args, "node_id")?;
            let tag: Tag = take(args, "tag")?;
            p.validate_tag(node_id, &tag)
                .await
                .map(|_| Value::Null)
                .map_err(|e| e.to_string())
        }
        "poll_group" => {
            let p = need_south(south)?;
            let node_id: NodeId = take(args, "node_id")?;
            let group_id: GroupId = take(args, "group_id")?;
            let tags: Vec<Tag> = take(args, "tags").unwrap_or_default();
            let out: Vec<(TagId, DataValue)> = p
                .poll_group(node_id, group_id, &tags)
                .await
                .map_err(|e| e.to_string())?;
            serde_json::to_value(out).map_err(|e| e.to_string())
        }
        "write_tags" => {
            let p = need_south(south)?;
            let node_id: NodeId = take(args, "node_id")?;
            let values: Vec<(Tag, DataValue)> = take(args, "values").unwrap_or_default();
            p.write_tags(node_id, &values)
                .await
                .map(|_| Value::Null)
                .map_err(|e| e.to_string())
        }
        "list_groups" => {
            let p = need_south(south)?;
            let node_id: NodeId = take(args, "node_id")?;
            let v: Vec<Group> = p.list_groups(node_id).await.map_err(|e| e.to_string())?;
            serde_json::to_value(v).map_err(|e| e.to_string())
        }
        "list_tags" => {
            let p = need_south(south)?;
            let node_id: NodeId = take(args, "node_id")?;
            let group_id: GroupId = take(args, "group_id")?;
            let v: Vec<Tag> = p
                .list_tags(node_id, group_id)
                .await
                .map_err(|e| e.to_string())?;
            serde_json::to_value(v).map_err(|e| e.to_string())
        }
        "config_schema" => {
            let schema = match (south, north) {
                (Some(p), _) => p.config_schema(),
                (None, Some(p)) => p.config_schema(),
                _ => None,
            };
            match schema {
                Some(s) => serde_json::to_value(s).map_err(|e| e.to_string()),
                None => Ok(Value::Null),
            }
        }
        "tag_schema" => {
            let p = need_south(south)?;
            match p.tag_schema() {
                Some(s) => serde_json::to_value(s).map_err(|e| e.to_string()),
                None => Ok(Value::Null),
            }
        }

        // ---------- 北向 ----------
        "set_subscriptions" => {
            let p = need_north(north)?;
            let node_id: NodeId = take(args, "node_id")?;
            let subs: Vec<GroupSubscription> = take(args, "subscriptions").unwrap_or_default();
            p.set_subscriptions(node_id, &subs)
                .await
                .map(|_| Value::Null)
                .map_err(|e| e.to_string())
        }
        "on_group_data" => {
            let p = need_north(north)?;
            let node_id: NodeId = take(args, "node_id")?;
            let data: GroupData = take(args, "data")?;
            p.on_group_data(node_id, Arc::new(data))
                .await
                .map(|_| Value::Null)
                .map_err(|e| e.to_string())
        }
        "connection_status" => {
            let p = need_north(north)?;
            let node_id: NodeId = take(args, "node_id")?;
            Ok(p.connection_status(node_id).await.unwrap_or(Value::Null))
        }

        other => Err(format!("unknown op '{}'", other)),
    }
}

fn need_south(south: &Option<Arc<dyn SouthPlugin>>) -> Result<&Arc<dyn SouthPlugin>, String> {
    south
        .as_ref()
        .ok_or_else(|| "this plugin is not a south plugin".to_string())
}

fn need_north(
    north: &Option<Arc<dyn gateway_sdk::NorthPlugin>>,
) -> Result<&Arc<dyn gateway_sdk::NorthPlugin>, String> {
    north
        .as_ref()
        .ok_or_else(|| "this plugin is not a north plugin".to_string())
}

fn take<T: serde::de::DeserializeOwned>(args: &Value, key: &str) -> Result<T, String> {
    let v = args
        .get(key)
        .cloned()
        .ok_or_else(|| format!("missing arg '{}'", key))?;
    serde_json::from_value(v).map_err(|e| format!("bad arg '{}': {}", key, e))
}
