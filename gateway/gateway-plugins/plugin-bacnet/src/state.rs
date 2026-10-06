//! BACnet 插件运行时状态。
//!
//! UDP 无连接：每个节点持有一个绑定了本机临时端口的 std::net::UdpSocket。
//! 选 std 而非 tokio 的原因：FFI 形态下每次插件调用运行在一次性的
//! current_thread runtime 上（见 SDK ffi 契约），tokio 资源跨调用复用会因
//! 「注册于其他 runtime」而失败；std socket 与 runtime 解耦，两种形态行为一致。
//! 阻塞收发通过 spawn_blocking 执行，不占住 async worker 线程。

use gateway_sdk::{Group, Tag};
use std::net::UdpSocket;
use std::sync::atomic::AtomicU8;
use std::time::Duration;

/// 单节点请求超时的下限与上限
pub const TIMEOUT_MIN_MS: u32 = 100;
pub const TIMEOUT_MAX_MS: u32 = 60_000;

/// 每个节点的运行时状态
pub struct BacnetNodeState {
    pub host: String,
    pub port: u16,
    /// 配置指定的设备实例；None = 用首个响应 I-Am 的设备
    pub device_instance: Option<u32>,
    pub timeout_ms: u32,
    pub write_priority: Option<u8>,
    pub groups: Vec<Group>,
    pub tags: Vec<Tag>,
    /// 绑定临时端口的 socket（与 runtime 解耦；Arc 以便 spawn_blocking 使用）
    pub socket: std::sync::Arc<UdpSocket>,
    /// 设备地址（host:port）
    pub remote: std::net::SocketAddr,
    /// 滚动 invoke id（单节点同一时刻只有一个在途请求）
    pub invoke: AtomicU8,
}

impl BacnetNodeState {
    pub fn timeout(&self) -> Duration {
        Duration::from_millis(self.timeout_ms.clamp(TIMEOUT_MIN_MS, TIMEOUT_MAX_MS) as u64)
    }

    /// 生成下一个 invoke id
    pub fn next_invoke(&self) -> u8 {
        self.invoke
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            .wrapping_add(1)
    }
}
