//! IEC 60870-5-104 worker 线程：自持 OS 线程 + current_thread Runtime + 长驻 Client。

use crate::state::Iec104NodeState;
use crate::value::asdu_to_cache_entry;
use gateway_sdk::types::DataValue;
use iec104::client::{Client, ClientCallback};
use iec104::config::{ClientConfig, ProtocolConfig};
use iec104::types::information_elements::Dpi;
use iec104::types::information_elements::Spi;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};

/// 写值类型（供 write_tags 使用）
#[derive(Debug)]
pub enum WriteValue {
    Bool(bool),
    /// 双点命令：0=Off, 1=On, 2=Toggle（超出范围返回错误）
    DoublePoint(u8),
}

/// 命令类型（从插件主方法发往 worker 线程）
pub enum Command {
    /// 连接被控站
    Connect,
    /// 断开连接（保留缓存）
    Disconnect,
    /// 写单个点位
    Write {
        ioa: u32,
        value: WriteValue,
        tx: oneshot::Sender<Result<(), String>>,
    },
    /// 关闭 worker（close 时调用，线程退出）
    Shutdown,
}

/// Worker 回调（实现 ClientCallback）。
/// 使用 `#[async_trait]` 宏，所以方法签名与 trait 一致。
struct WorkerCallback {
    cache: Arc<tokio::sync::Mutex<std::collections::HashMap<u32, (DataValue, u64)>>>,
    healthy: Arc<std::sync::atomic::AtomicBool>,
    #[allow(dead_code)]
    generation: Arc<std::sync::atomic::AtomicU64>,
}

impl WorkerCallback {
    fn new(
        cache: Arc<tokio::sync::Mutex<std::collections::HashMap<u32, (DataValue, u64)>>>,
        healthy: Arc<std::sync::atomic::AtomicBool>,
        generation: Arc<std::sync::atomic::AtomicU64>,
    ) -> Self {
        Self {
            cache,
            healthy,
            generation,
        }
    }
}

#[async_trait::async_trait]
impl ClientCallback for WorkerCallback {
    async fn on_new_objects(&self, asdu: iec104::asdu::Asdu) {
        if let Some((ioa, value, _ms)) = asdu_to_cache_entry(&asdu) {
            let mut c = self.cache.lock().await;
            c.insert(ioa, (value, 0));
        }
    }

    async fn on_connection_started(&self) {
        tracing::info!("iec104 connection started");
    }

    async fn on_connection_stopped(&self) {
        self.healthy
            .store(false, std::sync::atomic::Ordering::SeqCst);
        tracing::info!("iec104 connection stopped");
    }

    async fn on_error(&self, error: iec104::error::Error) {
        tracing::warn!(?error, "iec104 client error");
    }

    async fn on_reconnecting(&self) {
        self.healthy
            .store(false, std::sync::atomic::Ordering::SeqCst);
        tracing::info!("iec104 reconnecting (crate-managed)");
    }
}

/// Worker 主循环（跑在自持线程的 current_thread Runtime 上）。
pub async fn worker_loop(state: Arc<Iec104NodeState>, mut cmd_rx: mpsc::Receiver<Command>) {
    tracing::info!(
        "iec104 worker loop started, host={}, ca={}",
        state.host,
        state.common_address
    );

    // Client 存活在此 Arc 中
    let client: Arc<tokio::sync::Mutex<Option<Client<WorkerCallback>>>> =
        Arc::new(tokio::sync::Mutex::new(None));

    loop {
        use Command::*;
        match cmd_rx.recv().await {
            Some(Connect) => {
                tracing::info!(
                    host = %state.host,
                    port = %state.port,
                    ca = %state.common_address,
                    "iec104 connect"
                );

                // 构建 ClientConfig
                let protocol = ProtocolConfig {
                    t3: Duration::from_millis(state.t3_ms),
                    t2: Duration::from_millis(state.t1_ms * 2 / 3), // t2 由 k/w 推导
                    t1: Duration::from_millis(state.t1_ms),
                    t0: Duration::from_millis(state.t0_ms),
                    k: state.k,
                    w: state.w,
                    max_pending_outgoing_asdu: 1024,
                    originator_address: state.originator_address,
                };

                let cfg = ClientConfig {
                    address: state.host.clone(),
                    port: state.port,
                    protocol,
                    tls: None,
                };

                let cb = WorkerCallback::new(
                    state.cache.clone(),
                    state.healthy.clone(),
                    state.generation.clone(),
                );

                let mut c = Client::new(cfg, cb);

                // 连接（内部 spawn 接收循环并自管重连）
                if let Err(e) = c.connect().await {
                    tracing::error!(?e, "iec104 connect failed");
                    continue;
                }

                state
                    .generation
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                state
                    .healthy
                    .store(true, std::sync::atomic::Ordering::SeqCst);

                let mut guard = client.lock().await;
                *guard = Some(c);
            }
            Some(Disconnect) => {
                tracing::info!("iec104 disconnect");
                let mut guard = client.lock().await;
                if let Some(c) = guard.as_mut() {
                    if let Err(e) = c.stop_receiving().await {
                        tracing::warn!(?e, "iec104 stop_receiving error");
                    }
                }
                *guard = None;
                state
                    .healthy
                    .store(false, std::sync::atomic::Ordering::SeqCst);
            }
            Some(Write { ioa, value, tx }) => {
                let guard = client.lock().await;
                if let Some(c) = guard.as_ref() {
                    let result = match value {
                        WriteValue::Bool(b) => {
                            let spi = if b { Spi::On } else { Spi::Off };
                            c.send_command_sp(state.common_address, ioa, spi, None, None, None)
                                .await
                        }
                        WriteValue::DoublePoint(dp) => {
                            let dpi = match dp {
                                0 => Dpi::Off,
                                1 => Dpi::On,
                                _ => {
                                    let _ = tx.send(Err(format!(
                                        "invalid double-point value: {dp} (must be 0 or 1)"
                                    )));
                                    continue;
                                }
                            };
                            c.send_command_dp(state.common_address, ioa, dpi, None, None, None)
                                .await
                        }
                    };
                    let result = result.map_err(|e| format!("{:?}", e));
                    let _ = tx.send(result);
                } else {
                    let _ = tx.send(Err("not connected".to_string()));
                }
            }
            Some(Shutdown) => {
                tracing::info!("iec104 shutdown");
                {
                    let mut guard = client.lock().await;
                    if let Some(c) = guard.as_mut() {
                        let _ = c.stop_receiving().await;
                    }
                    *guard = None;
                }
                break;
            }
            None => {
                tracing::info!("iec104 command channel closed");
                break;
            }
        }
    }

    tracing::info!("iec104 worker loop exit");
}
