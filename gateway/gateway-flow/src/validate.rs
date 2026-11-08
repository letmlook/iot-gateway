//! publish 时校验（design §3.3）：draft 保存允许不完整图，publish 才强制。
//!
//! 覆盖：非空图、恰好一个 Source、≥1 Sink、算子已知、算子配置合法、
//! 边两端存在且端口名合法、每节点恰好一个输入、无环、无孤立节点。

use std::collections::{HashMap, HashSet};

use uuid::Uuid;

use crate::error::FlowError;
use crate::flow::{Flow, NodeKind};
use crate::operators::{self, PortSpec};

/// 校验整图，返回全部问题（空 = 通过）。环检测复用 `ExecutionPlan::build`。
pub fn validate(flow: &Flow) -> Vec<FlowError> {
    let mut errs: Vec<FlowError> = Vec::new();
    let nodes = &flow.graph.nodes;
    let edges = &flow.graph.edges;

    if nodes.is_empty() {
        errs.push(FlowError::Validation("graph has no nodes".into()));
        return errs;
    }

    let by_id: HashMap<Uuid, &crate::flow::FlowNode> = nodes.iter().map(|n| (n.id, n)).collect();
    let sources = nodes.iter().filter(|n| n.kind == NodeKind::Source).count();
    if sources != 1 {
        errs.push(FlowError::Validation(format!(
            "expected exactly 1 source node, found {sources}"
        )));
    }
    let sinks = nodes.iter().filter(|n| n.kind == NodeKind::Sink).count();
    if sinks == 0 {
        errs.push(FlowError::Validation("graph has no sink node".into()));
    }

    // 端口规范
    let port_of = |n: &crate::flow::FlowNode| -> Option<PortSpec> {
        match n.kind {
            NodeKind::Source => Some(PortSpec::source()),
            NodeKind::Sink => n.operator.as_deref().map(PortSpec::sink),
            NodeKind::Operator => n.operator.as_deref().map(PortSpec::operator),
        }
    };

    // 算子名与配置
    for n in nodes {
        match n.kind {
            NodeKind::Operator => {
                let Some(op) = n.operator.as_deref() else {
                    errs.push(FlowError::Validation(format!(
                        "operator node `{}` has no operator name",
                        n.name
                    )));
                    continue;
                };
                if !operators::OPERATOR_NAMES.contains(&op) {
                    errs.push(FlowError::Validation(format!("unknown operator `{op}`")));
                    continue;
                }
                if let Err(e) = operators::validate_config(op, &n.config) {
                    errs.push(FlowError::Operator(op.to_string(), e));
                }
            }
            NodeKind::Sink => {
                if let Some(op) = n.operator.as_deref() {
                    if !operators::OPERATOR_NAMES.contains(&op) {
                        errs.push(FlowError::Validation(format!("unknown sink `{op}`")));
                    } else if let Err(e) = operators::validate_config(op, &n.config) {
                        errs.push(FlowError::Operator(op.to_string(), e));
                    }
                }
            }
            NodeKind::Source => {
                if let Err(e) = operators::validate_source_config(&n.config) {
                    errs.push(FlowError::Validation(format!(
                        "source node `{}`: {e}",
                        n.name
                    )));
                }
            }
        }
    }

    // 边：两端存在、端口名合法
    let mut incoming: HashMap<Uuid, usize> = HashMap::new();
    for e in edges {
        let (Some(src), Some(_tgt)) = (by_id.get(&e.source_node_id), by_id.get(&e.target_node_id))
        else {
            errs.push(FlowError::Validation(format!(
                "edge {} -> {} references missing node",
                e.source_node_id, e.target_node_id
            )));
            continue;
        };
        let ok_port = port_of(src)
            .map(|p| p.outputs.contains(&e.source_port.as_str()))
            .unwrap_or(false);
        if !ok_port {
            errs.push(FlowError::Validation(format!(
                "node `{}` has no output port `{}`",
                src.name, e.source_port
            )));
        }
        if src.kind == NodeKind::Sink {
            errs.push(FlowError::Validation(format!(
                "sink node `{}` cannot be an edge source",
                src.name
            )));
        }
        *incoming.entry(e.target_node_id).or_insert(0) += 1;
    }

    // MVP：每节点恰好一个输入（多输入合并算子后续再上）
    for n in nodes {
        let cnt = incoming.get(&n.id).copied().unwrap_or(0);
        if n.kind != NodeKind::Source && cnt != 1 {
            errs.push(FlowError::Validation(format!(
                "node `{}` must have exactly 1 input edge, found {cnt}",
                n.name
            )));
        }
    }

    // 孤立节点（Source/Sink 只看边连通）
    let mut connected: HashSet<Uuid> = HashSet::new();
    for e in edges {
        connected.insert(e.source_node_id);
        connected.insert(e.target_node_id);
    }
    if !edges.is_empty() {
        for n in nodes {
            if !connected.contains(&n.id) {
                errs.push(FlowError::Validation(format!(
                    "node `{}` is disconnected",
                    n.name
                )));
            }
        }
    }

    // 环
    if let Err(e) = crate::plan::ExecutionPlan::build(flow) {
        errs.push(e);
    }
    errs
}
