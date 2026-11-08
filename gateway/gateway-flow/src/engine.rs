//! 单拍求值器：一条 `GroupData` 按 ExecutionPlan 分层流过 DAG，产出各 sink 的投递。
//!
//! crate 保持纯函数（无 tokio/无 I/O）：任务编排、订阅、sink 实际投递（北向插件调用/
//! 告警落库）在 server 接线层完成（design §3.1 方案 A、§10-③）。

use std::collections::HashMap;

use gateway_sdk::GroupData;
use uuid::Uuid;

use crate::error::FlowError;
use crate::flow::{Flow, NodeKind};
use crate::payload::FlowPayload;
use crate::plan::ExecutionPlan;

/// flow 节点运行态（state_change 上一态）。不落盘，重启清零。
#[derive(Debug, Default)]
pub struct FlowState {
    pub last_values: HashMap<Uuid, HashMap<String, String>>,
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
    // node_id → 输出（port → payload 列表；MVP 算子每端口至多一条）
    let mut outputs: HashMap<Uuid, Vec<(&'static str, FlowPayload)>> = HashMap::new();
    let mut deliveries: Vec<SinkDelivery> = Vec::new();

    for tier in &plan.tiers {
        for &node_id in tier {
            let node = plan
                .node(flow, node_id)
                .ok_or(FlowError::NodeNotFound(node_id))?;
            let payload: FlowPayload = match node.kind {
                NodeKind::Source => FlowPayload::Data(input.clone()),
                NodeKind::Operator | NodeKind::Sink => {
                    // MVP：每节点恰好一个输入（validate 保证）
                    let edge = flow
                        .graph
                        .edges
                        .iter()
                        .find(|e| e.target_node_id == node_id)
                        .ok_or_else(|| {
                            FlowError::Validation(format!("node `{}` has no input edge", node.name))
                        })?;
                    let upstream = outputs
                        .get(&edge.source_node_id)
                        .and_then(|outs| outs.iter().find(|(port, _)| *port == edge.source_port))
                        .map(|(_, p)| p);
                    let Some(payload) = upstream else {
                        // 上游该端口无输出（如 threshold 未命中 alarm 端）：本节点静默跳过
                        continue;
                    };
                    payload.clone()
                }
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
                        &mut state.last_values,
                    )?;
                    outputs.insert(node_id, outs);
                }
            }
        }
    }
    Ok(deliveries)
}
