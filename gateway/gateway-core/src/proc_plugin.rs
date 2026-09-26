//! 进程级插件隔离。
//!
//! # 为什么需要
//!
//! 进程内加载（`loader`）下插件与网关共享地址空间。插件侧的 `catch_unwind` 只能兜住 panic；
//! **段错误、`abort`、内存越界、死循环**这类故障无法兜住，会直接带走整个网关。
//! 本模块把插件放进独立进程（`gateway-plugin-host` 可执行文件）：
//!
//! ```text
//!   gateway ──长度前缀 JSON 帧──▶ plugin-host ──FFI──▶ plugin.so
//!           ◀────────────────────
//! ```
//!
//! 数据面协议与 FFI 完全一致（同一批 JSON 结构），因此两种加载模式对上层完全同构；
//! 插件的 stderr 直接继承给网关，日志不会丢。
//!
//! # 行为
//!
//! - 子进程崩溃 / 超时 → 该次调用返回错误，**网关不受影响**；下一次调用自动重启子进程；
//! - 每次调用有独立 `seq`，可并发（同一插件进程内仍按调用并发，与进程内模式一致）；
//! - 单次 RPC 超时后主动杀掉子进程，避免一个卡死的插件持续占用资源。
//!
//! # 代价
//!
//! 每次调用多一次进程间往返与一次 JSON 编解码；现场若对时延极敏感可继续用进程内模式
//! （`GATEWAY_PLUGIN_ISOLATION=inproc`，默认值）。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use gateway_sdk::{
    DataValue, Group, GroupData, GroupId, GroupSubscription, NodeId, PluginConfig, PluginError,
    PluginMeta, PluginResult, SouthPlugin, Tag, TagId,
};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{oneshot, Mutex};
use tracing::{error, info, warn};

/// 单帧最大长度（防止对端给出一个巨大的长度字段把内存吃光）
const MAX_FRAME_BYTES: usize = 64 * 1024 * 1024;
/// 单次 RPC 默认超时：调用方（如 poll_group 的采集超时）通常有更短的上层超时，
/// 这里只是兜底，避免请求永远挂住。
const DEFAULT_RPC_TIMEOUT: Duration = Duration::from_secs(120);

// ---------- 协议 ----------

/// 请求/响应帧约定：4 字节小端长度 + UTF-8 JSON。
pub mod protocol {
    use super::*;

    #[derive(Serialize, Deserialize, Debug)]
    pub struct Request {
        pub seq: u64,
        /// 操作名，如 "poll_group"
        pub op: String,
        /// 参数（同一个 op 的参数结构固定，见宿主进程的 dispatch）
        #[serde(default)]
        pub args: serde_json::Value,
    }

    #[derive(Serialize, Deserialize, Debug)]
    pub struct Response {
        pub seq: u64,
        pub ok: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub data: Option<serde_json::Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub err: Option<String>,
    }

    impl Response {
        pub fn ok(seq: u64, data: serde_json::Value) -> Self {
            Self {
                seq,
                ok: true,
                data: Some(data),
                err: None,
            }
        }

        pub fn err(seq: u64, msg: impl Into<String>) -> Self {
            Self {
                seq,
                ok: false,
                data: None,
                err: Some(msg.into()),
            }
        }
    }

    /// 写一帧
    pub async fn write_frame<W: AsyncWriteExt + Unpin>(
        w: &mut W,
        payload: &[u8],
    ) -> std::io::Result<()> {
        if payload.len() > MAX_FRAME_BYTES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "frame too large",
            ));
        }
        w.write_all(&(payload.len() as u32).to_le_bytes()).await?;
        w.write_all(payload).await?;
        w.flush().await
    }

    /// 读一帧；对端正常关闭（EOF）返回 `Ok(None)`
    pub async fn read_frame<R: AsyncReadExt + Unpin>(
        r: &mut R,
    ) -> std::io::Result<Option<Vec<u8>>> {
        let mut len_buf = [0u8; 4];
        match r.read_exact(&mut len_buf).await {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(e) => return Err(e),
        }
        let len = u32::from_le_bytes(len_buf) as usize;
        if len == 0 || len > MAX_FRAME_BYTES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("invalid frame length {}", len),
            ));
        }
        let mut buf = vec![0u8; len];
        r.read_exact(&mut buf).await?;
        Ok(Some(buf))
    }
}

// ---------- 子进程与 RPC 通道 ----------

struct ChildShared {
    stdin: Mutex<tokio::process::ChildStdin>,
    pending: Mutex<HashMap<u64, oneshot::Sender<protocol::Response>>>,
    next_seq: AtomicU64,
    alive: AtomicBool,
    kill_tx: Mutex<Option<oneshot::Sender<()>>>,
}

impl ChildShared {
    /// 让子进程退出（用于超时后回收卡死的插件）
    async fn kill(&self) {
        self.alive.store(false, Ordering::SeqCst);
        if let Some(tx) = self.kill_tx.lock().await.take() {
            let _ = tx.send(());
        }
    }

    async fn fail_pending(&self, reason: &str) {
        let mut p = self.pending.lock().await;
        for (_, tx) in p.drain() {
            let _ = tx.send(protocol::Response::err(0, reason.to_string()));
        }
    }
}

/// 节点的生命周期状态：子进程重启后据此重放，避免「重启即失忆」。
///
/// 插件把 per-node 状态放在自己的内存里，进程一死这些状态就没了。若不重放，
/// 重启后的子进程会对所有节点回「node not open」，隔离就变成「能把插件重启成不可用」。
#[derive(Default, Clone)]
struct NodeRuntime {
    open_args: serde_json::Value,
    setting_args: Option<serde_json::Value>,
    subscriptions: Option<serde_json::Value>,
    started: bool,
}

/// 一个插件子进程及其 RPC 通道
pub struct PluginProcess {
    bin: PathBuf,
    so_path: PathBuf,
    shared: Mutex<Option<Arc<ChildShared>>>,
    /// 串行化「启动子进程」避免并发重复拉起
    spawn_lock: Mutex<()>,
    restarts: AtomicU64,
    rpc_timeout: Duration,
    /// node_id → 生命周期状态（用于子进程重启后的重放）
    runtimes: Mutex<HashMap<String, NodeRuntime>>,
}

impl PluginProcess {
    pub fn new(bin: PathBuf, so_path: PathBuf) -> Self {
        Self {
            bin,
            so_path,
            shared: Mutex::new(None),
            spawn_lock: Mutex::new(()),
            restarts: AtomicU64::new(0),
            rpc_timeout: DEFAULT_RPC_TIMEOUT,
            runtimes: Mutex::new(HashMap::new()),
        }
    }

    /// 子进程重启次数（供运维观测）
    pub fn restarts(&self) -> u64 {
        self.restarts.load(Ordering::Relaxed)
    }

    /// 拿出可用通道；不存在或已死亡则重新拉起子进程
    async fn acquire(&self) -> Result<Arc<ChildShared>, String> {
        {
            let g = self.shared.lock().await;
            if let Some(s) = g.as_ref() {
                if s.alive.load(Ordering::SeqCst) {
                    return Ok(s.clone());
                }
            }
        }
        let _guard = self.spawn_lock.lock().await;
        // 双重检查：可能已被别的调用拉起
        {
            let g = self.shared.lock().await;
            if let Some(s) = g.as_ref() {
                if s.alive.load(Ordering::SeqCst) {
                    return Ok(s.clone());
                }
            }
        }
        let shared = self.spawn_child().await?;
        *self.shared.lock().await = Some(shared.clone());
        // 首次启动时表为空，重放是空操作；真正的重启才是它存在的意义
        self.replay(&shared).await;
        Ok(shared)
    }

    async fn spawn_child(&self) -> Result<Arc<ChildShared>, String> {
        let mut cmd = tokio::process::Command::new(&self.bin);
        cmd.arg("--plugin")
            .arg(&self.so_path)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            // 插件 stderr 直接进网关日志：既不污染协议流，也不丢日志
            .stderr(std::process::Stdio::inherit())
            .kill_on_drop(true);
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("spawn {}: {}", self.bin.display(), e))?;

        let pid = child.id();
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "child stdin unavailable".to_string())?;
        let mut stdout = child
            .stdout
            .take()
            .ok_or_else(|| "child stdout unavailable".to_string())?;

        let shared = Arc::new(ChildShared {
            stdin: Mutex::new(stdin),
            pending: Mutex::new(HashMap::new()),
            next_seq: AtomicU64::new(1),
            alive: AtomicBool::new(true),
            kill_tx: Mutex::new(None),
        });

        // 读响应：按 seq 路由回各自的等待者
        {
            let shared = shared.clone();
            tokio::spawn(async move {
                loop {
                    match protocol::read_frame(&mut stdout).await {
                        Ok(Some(buf)) => {
                            let resp: protocol::Response = match serde_json::from_slice(&buf) {
                                Ok(r) => r,
                                Err(e) => {
                                    warn!("plugin host sent an unparsable frame: {}", e);
                                    continue;
                                }
                            };
                            let tx = shared.pending.lock().await.remove(&resp.seq);
                            if let Some(tx) = tx {
                                let _ = tx.send(resp);
                            }
                        }
                        Ok(None) => break, // 子进程正常退出
                        Err(e) => {
                            warn!("read from plugin host failed: {}", e);
                            break;
                        }
                    }
                }
                shared.alive.store(false, Ordering::SeqCst);
                shared.fail_pending("plugin host exited").await;
            });
        }

        // 监视退出：正常退出或被杀都收敛到同一处
        {
            let shared = shared.clone();
            let so = self.so_path.display().to_string();
            let (kill_tx, kill_rx) = oneshot::channel::<()>();
            *shared.kill_tx.lock().await = Some(kill_tx);
            tokio::spawn(async move {
                tokio::select! {
                    _ = kill_rx => {
                        let _ = child.start_kill();
                        let _ = child.wait().await;
                        warn!(plugin = %so, "plugin host killed (rpc timeout)");
                    }
                    status = child.wait() => {
                        match status {
                            Ok(s) => error!(plugin = %so, stderr_inherited = true, "plugin host exited: {}", s),
                            Err(e) => error!(plugin = %so, "wait plugin host failed: {}", e),
                        }
                    }
                }
                shared.alive.store(false, Ordering::SeqCst);
                shared.fail_pending("plugin host exited").await;
            });
        }

        self.restarts.fetch_add(1, Ordering::Relaxed);
        info!(plugin = %self.so_path.display(), pid = ?pid, "plugin host started");
        Ok(shared)
    }

    /// 发起一次 RPC（并发安全）
    async fn call(&self, op: &str, args: serde_json::Value) -> Result<serde_json::Value, String> {
        let shared = self.acquire().await?;
        let out = self.call_on(&shared, op, args.clone()).await;
        if out.is_ok() {
            self.record_lifecycle(op, &args).await;
        }
        out
    }

    /// 在既有通道上发起 RPC（不做 acquire，也不会触发重放；供重放自身使用）
    async fn call_on(
        &self,
        shared: &Arc<ChildShared>,
        op: &str,
        args: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let shared = shared.clone();
        let seq = shared.next_seq.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        shared.pending.lock().await.insert(seq, tx);

        let req = protocol::Request {
            seq,
            op: op.to_string(),
            args,
        };
        let bytes = serde_json::to_vec(&req).map_err(|e| e.to_string())?;
        {
            let mut w = shared.stdin.lock().await;
            if let Err(e) = protocol::write_frame(&mut *w, &bytes).await {
                shared.pending.lock().await.remove(&seq);
                // 写失败通常意味着子进程已死：标记后由下一次调用重启
                shared.kill().await;
                return Err(format!("send to plugin host failed: {}", e));
            }
        }

        match tokio::time::timeout(self.rpc_timeout, rx).await {
            Ok(Ok(resp)) => {
                if resp.ok {
                    Ok(resp.data.unwrap_or(serde_json::Value::Null))
                } else {
                    Err(resp.err.unwrap_or_else(|| "plugin host error".to_string()))
                }
            }
            Ok(Err(_)) => Err("plugin host channel closed".to_string()),
            Err(_) => {
                shared.pending.lock().await.remove(&seq);
                // 卡死的插件不留在运行态：杀掉，下一次调用重启一个新的
                warn!(
                    plugin = %self.so_path.display(),
                    timeout_s = self.rpc_timeout.as_secs(),
                    "rpc timeout, killing plugin host"
                );
                shared.kill().await;
                Err("plugin host rpc timeout".to_string())
            }
        }
    }

    /// 记录会影响插件内存状态的生命周期调用（成功后才记）
    async fn record_lifecycle(&self, op: &str, args: &serde_json::Value) {
        let node_key = args
            .get("node_id")
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .or_else(|| {
                // NodeId 序列化为对象时取内部 uuid 字段
                args.get("node_id")
                    .and_then(|v| v.get("0"))
                    .and_then(|v| v.as_str().map(|s| s.to_string()))
            });
        let Some(node_key) = node_key else { return };
        let mut map = self.runtimes.lock().await;
        match op {
            "open" => {
                map.insert(
                    node_key,
                    NodeRuntime {
                        open_args: args.clone(),
                        ..Default::default()
                    },
                );
            }
            "close" => {
                map.remove(&node_key);
            }
            "setting" => {
                if let Some(rt) = map.get_mut(&node_key) {
                    rt.setting_args = Some(args.clone());
                }
            }
            "start" => {
                if let Some(rt) = map.get_mut(&node_key) {
                    rt.started = true;
                }
            }
            "stop" => {
                if let Some(rt) = map.get_mut(&node_key) {
                    rt.started = false;
                }
            }
            "set_subscriptions" => {
                if let Some(rt) = map.get_mut(&node_key) {
                    rt.subscriptions = Some(args.clone());
                }
            }
            _ => {}
        }
    }

    /// 子进程重启后重放各节点的生命周期（open → init → setting → start / 订阅）
    async fn replay(&self, shared: &Arc<ChildShared>) {
        let snapshot: Vec<(String, NodeRuntime)> = self
            .runtimes
            .lock()
            .await
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        if snapshot.is_empty() {
            return;
        }
        tracing::info!(
            plugin = %self.so_path.display(),
            nodes = snapshot.len(),
            "replaying node lifecycles after plugin host restart"
        );
        for (_, rt) in snapshot {
            if let Err(e) = self.call_on(shared, "open", rt.open_args.clone()).await {
                warn!(plugin = %self.so_path.display(), "replay open failed: {}", e);
                continue;
            }
            let node_id = rt
                .open_args
                .get("node_id")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            let _ = self
                .call_on(
                    shared,
                    "init",
                    serde_json::json!({ "node_id": node_id.clone() }),
                )
                .await;
            if let Some(s) = &rt.setting_args {
                let _ = self.call_on(shared, "setting", s.clone()).await;
            }
            if let Some(s) = &rt.subscriptions {
                let _ = self.call_on(shared, "set_subscriptions", s.clone()).await;
            }
            if rt.started {
                let _ = self
                    .call_on(shared, "start", serde_json::json!({ "node_id": node_id }))
                    .await;
            }
        }
    }

    /// 把值反序列化为目标类型
    async fn call_as<T: serde::de::DeserializeOwned>(
        &self,
        op: &str,
        args: serde_json::Value,
    ) -> Result<T, String> {
        let v = self.call(op, args).await?;
        serde_json::from_value(v).map_err(|e| format!("{}: bad response: {}", op, e))
    }
}

// ---------- 南向适配器 ----------

/// 进程隔离下的南向插件：每个方法转发为一次 RPC
pub struct ProcessSouthPlugin {
    proc: Arc<PluginProcess>,
    meta: PluginMeta,
    /// Schema 在加载时取回一次：`config_schema()` 是同步接口，不能每次渲染都走 RPC
    config_schema: Option<gateway_sdk::ConfigSchema>,
    tag_schema: Option<gateway_sdk::TagSchema>,
}

impl ProcessSouthPlugin {
    pub fn new(
        proc: Arc<PluginProcess>,
        meta: PluginMeta,
        config_schema: Option<gateway_sdk::ConfigSchema>,
        tag_schema: Option<gateway_sdk::TagSchema>,
    ) -> Self {
        Self {
            proc,
            meta,
            config_schema,
            tag_schema,
        }
    }
}

fn node_arg(node_id: NodeId) -> serde_json::Value {
    serde_json::json!({ "node_id": node_id })
}

fn node_group_args(node_id: NodeId, group_id: GroupId) -> serde_json::Value {
    serde_json::json!({ "node_id": node_id, "group_id": group_id })
}

#[async_trait::async_trait]
impl SouthPlugin for ProcessSouthPlugin {
    fn meta(&self) -> PluginMeta {
        self.meta.clone()
    }

    fn config_schema(&self) -> Option<gateway_sdk::ConfigSchema> {
        self.config_schema.clone()
    }

    fn tag_schema(&self) -> Option<gateway_sdk::TagSchema> {
        self.tag_schema.clone()
    }

    async fn open(&self, node_id: NodeId, config: PluginConfig) -> PluginResult<()> {
        let args = serde_json::json!({ "node_id": node_id, "config": config });
        self.proc
            .call("open", args)
            .await
            .map(|_| ())
            .map_err(PluginError::msg)
    }

    async fn close(&self, node_id: NodeId) -> PluginResult<()> {
        self.proc
            .call("close", node_arg(node_id))
            .await
            .map(|_| ())
            .map_err(PluginError::msg)
    }

    async fn init(&self, node_id: NodeId) -> PluginResult<()> {
        self.proc
            .call("init", node_arg(node_id))
            .await
            .map(|_| ())
            .map_err(PluginError::msg)
    }

    async fn uninit(&self, node_id: NodeId) -> PluginResult<()> {
        self.proc
            .call("uninit", node_arg(node_id))
            .await
            .map(|_| ())
            .map_err(PluginError::msg)
    }

    async fn start(&self, node_id: NodeId) -> PluginResult<()> {
        self.proc
            .call("start", node_arg(node_id))
            .await
            .map(|_| ())
            .map_err(PluginError::msg)
    }

    async fn stop(&self, node_id: NodeId) -> PluginResult<()> {
        self.proc
            .call("stop", node_arg(node_id))
            .await
            .map(|_| ())
            .map_err(PluginError::msg)
    }

    async fn setting(&self, node_id: NodeId, config: PluginConfig) -> PluginResult<()> {
        let args = serde_json::json!({ "node_id": node_id, "config": config });
        self.proc
            .call("setting", args)
            .await
            .map(|_| ())
            .map_err(PluginError::msg)
    }

    async fn validate_tag(&self, node_id: NodeId, tag: &Tag) -> PluginResult<()> {
        let args = serde_json::json!({ "node_id": node_id, "tag": tag });
        self.proc
            .call("validate_tag", args)
            .await
            .map(|_| ())
            .map_err(PluginError::msg)
    }

    async fn poll_group(
        &self,
        node_id: NodeId,
        group_id: GroupId,
        tags: &[Tag],
    ) -> PluginResult<Vec<(TagId, DataValue)>> {
        let args = serde_json::json!({ "node_id": node_id, "group_id": group_id, "tags": tags });
        self.proc
            .call_as("poll_group", args)
            .await
            .map_err(PluginError::msg)
    }

    async fn write_tags(&self, node_id: NodeId, values: &[(Tag, DataValue)]) -> PluginResult<()> {
        let args = serde_json::json!({ "node_id": node_id, "values": values });
        self.proc
            .call("write_tags", args)
            .await
            .map(|_| ())
            .map_err(PluginError::msg)
    }

    async fn list_groups(&self, node_id: NodeId) -> PluginResult<Vec<Group>> {
        self.proc
            .call_as("list_groups", node_arg(node_id))
            .await
            .map_err(PluginError::msg)
    }

    async fn list_tags(&self, node_id: NodeId, group_id: GroupId) -> PluginResult<Vec<Tag>> {
        self.proc
            .call_as("list_tags", node_group_args(node_id, group_id))
            .await
            .map_err(PluginError::msg)
    }
}

// ---------- 北向适配器 ----------

pub struct ProcessNorthPlugin {
    proc: Arc<PluginProcess>,
    meta: PluginMeta,
    config_schema: Option<gateway_sdk::ConfigSchema>,
}

impl ProcessNorthPlugin {
    pub fn new(
        proc: Arc<PluginProcess>,
        meta: PluginMeta,
        config_schema: Option<gateway_sdk::ConfigSchema>,
    ) -> Self {
        Self {
            proc,
            meta,
            config_schema,
        }
    }
}

#[async_trait::async_trait]
impl gateway_sdk::NorthPlugin for ProcessNorthPlugin {
    fn meta(&self) -> PluginMeta {
        self.meta.clone()
    }

    fn config_schema(&self) -> Option<gateway_sdk::ConfigSchema> {
        self.config_schema.clone()
    }

    async fn open(&self, node_id: NodeId, config: PluginConfig) -> PluginResult<()> {
        let args = serde_json::json!({ "node_id": node_id, "config": config });
        self.proc
            .call("open", args)
            .await
            .map(|_| ())
            .map_err(PluginError::msg)
    }

    async fn close(&self, node_id: NodeId) -> PluginResult<()> {
        self.proc
            .call("close", node_arg(node_id))
            .await
            .map(|_| ())
            .map_err(PluginError::msg)
    }

    async fn init(&self, node_id: NodeId) -> PluginResult<()> {
        self.proc
            .call("init", node_arg(node_id))
            .await
            .map(|_| ())
            .map_err(PluginError::msg)
    }

    async fn uninit(&self, node_id: NodeId) -> PluginResult<()> {
        self.proc
            .call("uninit", node_arg(node_id))
            .await
            .map(|_| ())
            .map_err(PluginError::msg)
    }

    async fn start(&self, node_id: NodeId) -> PluginResult<()> {
        self.proc
            .call("start", node_arg(node_id))
            .await
            .map(|_| ())
            .map_err(PluginError::msg)
    }

    async fn stop(&self, node_id: NodeId) -> PluginResult<()> {
        self.proc
            .call("stop", node_arg(node_id))
            .await
            .map(|_| ())
            .map_err(PluginError::msg)
    }

    async fn setting(&self, node_id: NodeId, config: PluginConfig) -> PluginResult<()> {
        let args = serde_json::json!({ "node_id": node_id, "config": config });
        self.proc
            .call("setting", args)
            .await
            .map(|_| ())
            .map_err(PluginError::msg)
    }

    async fn set_subscriptions(
        &self,
        node_id: NodeId,
        subscriptions: &[GroupSubscription],
    ) -> PluginResult<()> {
        let args = serde_json::json!({ "node_id": node_id, "subscriptions": subscriptions });
        self.proc
            .call("set_subscriptions", args)
            .await
            .map(|_| ())
            .map_err(PluginError::msg)
    }

    async fn on_group_data(&self, node_id: NodeId, data: Arc<GroupData>) -> PluginResult<()> {
        // 只需要序列化一次 GroupData，因此直接借用内部值（Arc<GroupData> 本身不实现 Serialize）
        let args = serde_json::json!({ "node_id": node_id, "data": &*data });
        self.proc
            .call("on_group_data", args)
            .await
            .map(|_| ())
            .map_err(PluginError::msg)
    }

    async fn connection_status(&self, node_id: NodeId) -> Option<serde_json::Value> {
        match self.proc.call("connection_status", node_arg(node_id)).await {
            Ok(v) if v.is_null() => None,
            Ok(v) => Some(v),
            Err(e) => {
                warn!("connection_status via plugin host failed: {}", e);
                None
            }
        }
    }
}

/// `Option<String>` → `Option<&'static str>`（`PluginMeta` 用静态字符串）
fn leak_opt(v: Option<String>) -> Option<&'static str> {
    v.map(|s| Box::leak(s.into_boxed_str()) as &'static str)
}

// ---------- 加载入口 ----------

/// 以进程隔离方式加载插件目录。
///
/// 与 `PluginLoader::load` 的差别：每个 .so 起一个独立子进程，插件崩溃不会影响网关。
pub struct ProcessPluginLoader {
    bin: PathBuf,
    processes: Vec<Arc<PluginProcess>>,
}

impl ProcessPluginLoader {
    /// `bin` 为 `gateway-plugin-host` 可执行文件路径
    pub fn new(bin: PathBuf) -> Self {
        Self {
            bin,
            processes: Vec::new(),
        }
    }

    /// 扫描目录，为每个插件动态库拉起一个宿主进程并注册适配器
    pub async fn load(
        &mut self,
        plugins_dir: &Path,
        mgr: &mut crate::manager::Manager,
    ) -> Result<(), String> {
        let ext = std::env::consts::DLL_EXTENSION;
        let entries = std::fs::read_dir(plugins_dir)
            .map_err(|e| format!("read_dir {}: {}", plugins_dir.display(), e))?;
        for e in entries {
            let e = e.map_err(|e| e.to_string())?;
            let path = e.path();
            if !path
                .extension()
                .map(|x| x.to_string_lossy() == ext)
                .unwrap_or(false)
            {
                continue;
            }
            match self.load_one(&path, mgr).await {
                Ok(()) => {}
                Err(err) => warn!(path = %path.display(), "load plugin (isolated): {}", err),
            }
        }
        Ok(())
    }

    async fn load_one(
        &mut self,
        path: &Path,
        mgr: &mut crate::manager::Manager,
    ) -> Result<(), String> {
        let proc = Arc::new(PluginProcess::new(self.bin.clone(), path.to_path_buf()));
        // 通过一次 meta 调用确认子进程起来了、插件也被宿主接受了
        let meta_json = proc
            .call("meta", serde_json::Value::Null)
            .await
            .map_err(|e| format!("plugin host handshake failed: {}", e))?;
        let ffi_meta: gateway_sdk::ffi::FfiPluginMeta =
            serde_json::from_value(meta_json).map_err(|e| format!("bad meta: {}", e))?;
        if ffi_meta.name.trim().is_empty() || ffi_meta.name == "?" {
            return Err("plugin meta has no usable name".to_string());
        }
        let (schema, tag_schema) = Self::fetch_schemas(&proc).await;
        let meta = PluginMeta {
            name: Box::leak(ffi_meta.name.clone().into_boxed_str()),
            kind: if ffi_meta.kind == "north" {
                gateway_sdk::PluginKind::North
            } else {
                gateway_sdk::PluginKind::South
            },
            description: leak_opt(ffi_meta.description.clone()),
            version: Box::leak(ffi_meta.version.clone().into_boxed_str()),
            name_zh: leak_opt(ffi_meta.name_zh.clone()),
            name_en: leak_opt(ffi_meta.name_en.clone()),
            description_zh: leak_opt(ffi_meta.description_zh.clone()),
            description_en: leak_opt(ffi_meta.description_en.clone()),
        };

        match meta.kind {
            gateway_sdk::PluginKind::South => {
                let p = ProcessSouthPlugin::new(proc.clone(), meta, schema, tag_schema);
                let name = p.meta.name.to_string();
                info!(path = %path.display(), name = %name, "loaded south plugin in isolated process");
                mgr.register_south(&name, Arc::new(p));
            }
            gateway_sdk::PluginKind::North => {
                let p = ProcessNorthPlugin::new(proc.clone(), meta, schema);
                let name = p.meta.name.to_string();
                info!(path = %path.display(), name = %name, "loaded north plugin in isolated process");
                mgr.register_north(&name, Arc::new(p));
            }
        }
        self.processes.push(proc);
        Ok(())
    }

    async fn fetch_schemas(
        proc: &Arc<PluginProcess>,
    ) -> (
        Option<gateway_sdk::ConfigSchema>,
        Option<gateway_sdk::TagSchema>,
    ) {
        let cfg = match proc.call("config_schema", serde_json::Value::Null).await {
            Ok(v) if !v.is_null() => serde_json::from_value(v).ok(),
            _ => None,
        };
        let tag = match proc.call("tag_schema", serde_json::Value::Null).await {
            Ok(v) if !v.is_null() => serde_json::from_value(v).ok(),
            _ => None,
        };
        (cfg, tag)
    }

    /// 所有插件子进程的重启次数之和
    pub fn total_restarts(&self) -> u64 {
        self.processes.iter().map(|p| p.restarts()).sum()
    }

    pub fn process_count(&self) -> usize {
        self.processes.len()
    }
}
