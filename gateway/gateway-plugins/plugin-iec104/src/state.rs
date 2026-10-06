//! IEC 60870-5-104 插件 per-node 状态。

use gateway_sdk::types::{DataValue, Group, Tag};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

/// IOA → (数据值, 毫秒时间戳)
pub type CacheEntry = (DataValue, u64);

/// 节点状态（open 时创建，close 时销毁 worker 线程）。
pub struct Iec104NodeState {
    /// 配置参数
    pub host: String,
    pub port: u16,
    pub common_address: u16,
    pub originator_address: u8,
    pub k: u16,
    pub w: u16,
    pub t0_ms: u64,
    pub t1_ms: u64,
    pub t3_ms: u64,
    pub gi_interval_ms: u64,
    pub clock_sync_interval_ms: u64,
    /// 默认采集组
    pub groups: Vec<Group>,
    /// 全部点位（按 group 归类由 list_tags 返回）
    pub tags: Vec<Tag>,
    /// 点位缓存：IOA → (DataValue, 毫秒时间戳)
    pub cache: Arc<Mutex<HashMap<u32, CacheEntry>>>,
    /// 连接健康标志，由 on_reconnecting 维护
    pub healthy: Arc<std::sync::atomic::AtomicBool>,
    /// 连接代次计数（每次 Connect 递增，用于区分新旧连接）
    pub generation: Arc<std::sync::atomic::AtomicU64>,
    /// 命令通道发送端（→ worker 线程）
    pub cmd_tx: tokio::sync::mpsc::Sender<super::worker::Command>,
    /// Worker 线程句柄（start 时设置，close 时 join）
    /// std::thread::JoinHandle 不是 Sync，用 Mutex 包装以满足 Send+Sync 约束
    pub join_handle: Mutex<Option<std::thread::JoinHandle<()>>>,
}

impl Iec104NodeState {
    /// 创建节点状态（不含 worker 线程，线程在 start 时 spawn）。
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        host: String,
        port: u16,
        common_address: u16,
        originator_address: u8,
        k: u16,
        w: u16,
        t0_ms: u64,
        t1_ms: u64,
        t3_ms: u64,
        gi_interval_ms: u64,
        clock_sync_interval_ms: u64,
        groups: Vec<Group>,
        tags: Vec<Tag>,
        cmd_tx: tokio::sync::mpsc::Sender<super::worker::Command>,
    ) -> Self {
        Self {
            host,
            port,
            common_address,
            originator_address,
            k,
            w,
            t0_ms,
            t1_ms,
            t3_ms,
            gi_interval_ms,
            clock_sync_interval_ms,
            groups,
            tags,
            cache: Arc::new(Mutex::new(HashMap::new())),
            healthy: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            generation: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            cmd_tx,
            join_handle: Mutex::new(None),
        }
    }
}
