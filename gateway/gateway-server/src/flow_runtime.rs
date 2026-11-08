//! 流程运行时（设计 docs/design/流程编排-v2.md §3.1 方案 A）：
//! 每个 published 流程一个总线消费任务——订阅 source 声明的 (south_node, group) 分区通道，
//! 经 `gateway_flow::engine::evaluate` 单拍求值，sink 投递：
//! - `sink_north` → 目标北向节点插件 `on_group_data`（节点须 Running）；
//! - `sink_alarm` → `AlarmStore::raise`（同键 active 去重前移）。
//!
//! 取消语义：任务句柄存于 `tasks`，stop 用 `abort()`（任务体只持总线接收器与预解析数据，
//! 无跨 await 的锁）；改图重新部署 = stop + start。进程重启后由 main 启动序列
//! `start_published` 恢复全部 published 流程。

use std::collections::HashMap;
use std::sync::Arc;

use gateway_flow::{FlowGraph, NodeKind};
use serde::Deserialize;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::flows::FlowRow;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
struct SourceCfg {
    south_node_id: Uuid,
    group_id: Uuid,
}

#[derive(Debug, Deserialize)]
struct SinkNorthCfg {
    node_id: Uuid,
}

/// sink 节点的预解析投递目标。
enum SinkTarget {
    North { north_node_id: Uuid },
    Alarm,
}

#[derive(Default)]
pub struct FlowRuntime {
    tasks: Mutex<HashMap<Uuid, JoinHandle<()>>>,
}

impl FlowRuntime {
    pub fn new() -> Self {
        Self::default()
    }

    /// 启动（或以新图重启）一个流程的消费任务。
    pub async fn start(&self, state: &AppState, row: FlowRow) {
        self.stop(row.id).await;
        let Ok(graph) = serde_json::from_value::<FlowGraph>(row.graph_json.clone()) else {
            warn!(flow_id = ?row.id, "stored graph is malformed, flow not started");
            return;
        };
        let Some(src) = graph.nodes.iter().find(|n| n.kind == NodeKind::Source) else {
            warn!(flow_id = ?row.id, "no source node, flow not started");
            return;
        };
        let Ok(sc) = serde_json::from_value::<SourceCfg>(src.config.clone()) else {
            warn!(flow_id = ?row.id, "source config invalid, flow not started");
            return;
        };
        // 预解析 sink 目标（名称 + 投递方式）
        let mut sinks: HashMap<Uuid, (String, SinkTarget)> = HashMap::new();
        for n in &graph.nodes {
            if n.kind != NodeKind::Sink {
                continue;
            }
            match n.operator.as_deref() {
                Some("sink_north") => {
                    match serde_json::from_value::<SinkNorthCfg>(n.config.clone()) {
                        Ok(c) => {
                            sinks.insert(
                                n.id,
                                (
                                    n.name.clone(),
                                    SinkTarget::North {
                                        north_node_id: c.node_id,
                                    },
                                ),
                            );
                        }
                        Err(e) => {
                            warn!(flow_id = ?row.id, node = %n.name, "sink_north config invalid: {e}")
                        }
                    }
                }
                Some("sink_alarm") => {
                    sinks.insert(n.id, (n.name.clone(), SinkTarget::Alarm));
                }
                _ => {}
            }
        }
        // flow→flow 禁环（设计 §3.4）：输入只来自南向组分区通道
        let mut rxs = state.manager.bus().subscribe_groups(&[(
            gateway_sdk::NodeId(sc.south_node_id),
            gateway_sdk::GroupId(sc.group_id),
        )]);
        let Some(mut rx) = rxs.pop() else {
            return;
        };
        let engine_flow = gateway_flow::Flow {
            id: row.id,
            tenant_id: row.tenant_id.clone(),
            name: row.name.clone(),
            description: row.description.clone(),
            status: gateway_flow::FlowStatus::Published,
            version: row.version,
            graph,
            created_at: None,
            updated_at: None,
        };
        let manager = state.manager.clone();
        let alarm_store = state.alarm_store.clone();
        let tenant = row.tenant_id.clone();
        let flow_id = row.id;
        let flow_name = row.name.clone();
        let handle = tokio::spawn(async move {
            let mut flow_state = gateway_flow::FlowState::default();
            let Ok(plan) = gateway_flow::ExecutionPlan::build(&engine_flow) else {
                error!(flow_id = ?flow_id, "execution plan failed, flow task exits");
                return;
            };
            info!(flow_id = ?flow_id, name = %flow_name, "flow task started");
            loop {
                match rx.recv().await {
                    Ok(data) => {
                        let deliveries =
                            gateway_flow::evaluate(&plan, &engine_flow, &data, &mut flow_state);
                        let deliveries = match deliveries {
                            Ok(d) => d,
                            Err(e) => {
                                error!(flow_id = ?flow_id, "evaluate error: {e}");
                                continue;
                            }
                        };
                        for d in deliveries {
                            let Some((node_name, target)) = sinks.get(&d.node_id) else {
                                continue;
                            };
                            match (target, &d.payload) {
                                (SinkTarget::Alarm, gateway_flow::FlowPayload::Alarm(ev)) => {
                                    if let Err(e) =
                                        alarm_store.raise(&tenant, &flow_id, node_name, ev)
                                    {
                                        error!(flow_id = ?flow_id, "alarm raise error: {e}");
                                    }
                                }
                                (
                                    SinkTarget::North { north_node_id },
                                    gateway_flow::FlowPayload::Data(gd),
                                ) => {
                                    deliver_north(&manager, *north_node_id, Arc::new(gd.clone()))
                                        .await;
                                }
                                _ => {}
                            }
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        warn!(flow_id = ?flow_id, lagged = n, "flow bus recv lagged, skipped messages");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
            info!(flow_id = ?flow_id, name = %flow_name, "flow task stopped");
        });
        self.tasks.lock().await.insert(row.id, handle);
    }

    /// 停止流程任务（不存在则静默）。
    pub async fn stop(&self, flow_id: Uuid) {
        if let Some(handle) = self.tasks.lock().await.remove(&flow_id) {
            handle.abort();
        }
    }

    /// 开机恢复：启动全部 published 流程。
    pub async fn start_published(&self, state: &AppState) {
        let Ok(rows) = state.flow_store.list(&crate::api::scope::TenantScope::All) else {
            warn!("flows list failed at startup, no flow started");
            return;
        };
        let n = rows.iter().filter(|r| r.status == "published").count();
        for row in rows.into_iter().filter(|r| r.status == "published") {
            self.start(state, row).await;
        }
        if n > 0 {
            info!("recovered {n} published flow(s)");
        }
    }
}

/// sink_north 投递：目标北向节点须存在且 Running；失败仅记录（不影响采集主链路）。
async fn deliver_north(
    manager: &Arc<gateway_core::Manager>,
    north_node_id: Uuid,
    gd: Arc<gateway_sdk::GroupData>,
) {
    use gateway_sdk::NodeState;
    let Some(node) = manager.node_get(gateway_sdk::NodeId(north_node_id)) else {
        warn!(north_node_id = ?north_node_id, "sink_north target node not found");
        return;
    };
    if node.state != NodeState::Running {
        return; // 目标未运行：静默跳过（与 north 消费任务「未 Running 不转发」同语义）
    }
    let Some(plugin) = manager.north_plugin(&node.config.plugin_name) else {
        return;
    };
    if let Err(e) = plugin
        .on_group_data(gateway_sdk::NodeId(north_node_id), gd)
        .await
    {
        warn!(north_node_id = ?north_node_id, "sink_north on_group_data error: {e}");
    }
}
