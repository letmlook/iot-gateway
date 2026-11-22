//! 单拍求值器：一条 `GroupData` 按 ExecutionPlan 分层流过 DAG，产出各 sink 的投递。
//!
//! crate 保持纯函数（无 tokio/无 I/O）：任务编排、订阅、sink 实际投递（北向插件调用/
//! 告警落库）在 server 接线层完成（design §3.1 方案 A、§10-③）。

use std::collections::HashMap;

use gateway_sdk::{DataValue, GroupData};
use uuid::Uuid;

use crate::error::FlowError;
use crate::flow::{Flow, NodeKind};
use crate::payload::FlowPayload;
use crate::plan::ExecutionPlan;

/// flow 节点运行态（state_change 上一态 / rate 上一读数 / dedup 上一帧）。不落盘，重启清零。
#[derive(Debug, Default)]
pub struct FlowState {
    pub last_values: HashMap<Uuid, HashMap<String, String>>,
    /// rate_of_change 基线：节点 id → tag_name → (上一值, 上一 ts 毫秒)
    pub last_rate: HashMap<Uuid, HashMap<String, (f64, u64)>>,
    /// deduplicate 快照：节点 id → 上一帧 values
    pub last_frames: HashMap<Uuid, Vec<(gateway_sdk::TagId, DataValue)>>,
    /// expr_route AST 缓存：节点 id → 缓存条目（spec §4.5）。不落盘，重启清零。
    pub script_asts: HashMap<Uuid, crate::operators::CachedScript>,
}

/// 一次求值的 sink 投递（server 按此分发：sink_north → 北向插件，sink_alarm → 告警存储）。
#[derive(Debug)]
pub struct SinkDelivery {
    pub node_id: Uuid,
    pub payload: FlowPayload,
}

pub fn evaluate(
    plan: &ExecutionPlan,
    flow: &Flow,
    input: &GroupData,
    state: &mut FlowState,
) -> Result<Vec<SinkDelivery>, FlowError> {
    evaluate_with_inject(plan, flow, input, state, None)
}

/// 断流告警注入求值（spec §4.3）：`inject = Some(ev)` 时 alarm_to_data 算子与
/// sink_alarm sink 直接收 `Alarm(ev)`，其余节点（含 Source）无输入静默——
/// 缺测不误触发；alarm_to_data 产出的 Data 沿图正常传播（下游 sink_north 推送）。
/// 纯函数，下游传播复用分层循环；`inject = None` 时与 `evaluate` 逐行等价。
pub fn evaluate_with_inject(
    plan: &ExecutionPlan,
    flow: &Flow,
    input: &GroupData,
    state: &mut FlowState,
    inject: Option<&crate::payload::AlarmEvent>,
) -> Result<Vec<SinkDelivery>, FlowError> {
    // node_id → 输出（port → payload 列表；MVP 算子每端口至多一条）
    let mut outputs: HashMap<Uuid, Vec<(&'static str, FlowPayload)>> = HashMap::new();
    let mut deliveries: Vec<SinkDelivery> = Vec::new();

    for tier in &plan.tiers {
        for &node_id in tier {
            let node = plan
                .node(flow, node_id)
                .ok_or(FlowError::NodeNotFound(node_id))?;
            // 注入模式：告警直接送达 alarm_to_data / sink_alarm
            let injected = match (inject, node.operator.as_deref(), node.kind) {
                (Some(ev), Some("alarm_to_data"), NodeKind::Operator) => {
                    Some(FlowPayload::Alarm(ev.clone()))
                }
                (Some(ev), Some("sink_alarm"), NodeKind::Sink) => {
                    Some(FlowPayload::Alarm(ev.clone()))
                }
                _ => None,
            };
            let payload: Option<FlowPayload> = if let Some(p) = injected {
                Some(p)
            } else {
                match node.kind {
                    // 注入模式 Source 不输出（缺测不误触发），普通模式注入原始帧
                    NodeKind::Source if inject.is_some() => None,
                    NodeKind::Source => Some(FlowPayload::Data(input.clone())),
                    NodeKind::Operator | NodeKind::Sink => {
                        // MVP：每节点恰好一个输入（validate 保证）
                        let edge = flow
                            .graph
                            .edges
                            .iter()
                            .find(|e| e.target_node_id == node_id)
                            .ok_or_else(|| {
                                FlowError::Validation(format!(
                                    "node `{}` has no input edge",
                                    node.name
                                ))
                            })?;
                        // 上游该端口无输出（如 threshold 未命中 alarm 端、注入模式下
                        // 上游 Source 静默）：本节点静默跳过
                        outputs
                            .get(&edge.source_node_id)
                            .and_then(|outs| {
                                outs.iter().find(|(port, _)| *port == edge.source_port)
                            })
                            .map(|(_, p)| p.clone())
                    }
                }
            };
            let Some(payload) = payload else {
                continue;
            };

            match node.kind {
                NodeKind::Sink => {
                    deliveries.push(SinkDelivery { node_id, payload });
                    outputs.insert(node_id, Vec::new());
                }
                NodeKind::Source => {
                    outputs.insert(node_id, vec![("out", payload)]);
                }
                NodeKind::Operator => {
                    let op = node
                        .operator
                        .as_deref()
                        .ok_or_else(|| {
                            FlowError::Validation(format!("node `{}` has no operator", node.name))
                        })?
                        .to_string();
                    let outs = crate::operators::evaluate_node(
                        &op,
                        node_id,
                        &node.config,
                        payload,
                        state,
                    )?;
                    outputs.insert(node_id, outs);
                }
            }
        }
    }
    Ok(deliveries)
}
