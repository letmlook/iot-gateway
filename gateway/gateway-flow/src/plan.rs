//! 执行计划：Kahn 拓扑排序 + 分层（层内可并行）。
//!
//! 语义与 redesign/v1 `DagExecutor::execution_tiers` 一致（设计稿 §3.2），实现为手写 Kahn
//! 算法，不引入 petgraph 依赖；环检测在构计划时完成（publish 校验复用）。

use std::collections::{HashMap, HashSet};

use uuid::Uuid;

use crate::error::FlowError;
use crate::flow::{Flow, NodeKind};

/// 分层执行计划：tiers[k] 内的节点只依赖 tiers[<k] 的节点。
#[derive(Debug, Clone)]
pub struct ExecutionPlan {
    pub tiers: Vec<Vec<Uuid>>,
}

impl ExecutionPlan {
    /// 从流程图构建计划。环 → `FlowError::Cycle`。
    pub fn build(flow: &Flow) -> Result<Self, FlowError> {
        let ids: HashSet<Uuid> = flow.graph.nodes.iter().map(|n| n.id).collect();
        // 入度表（只统计两端都在图内的边；悬空边在 validate 报错，这里宽容跳过）
        let mut in_degree: HashMap<Uuid, usize> =
            flow.graph.nodes.iter().map(|n| (n.id, 0)).collect();
        let mut edges_from: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
        for e in &flow.graph.edges {
            if !ids.contains(&e.source_node_id) || !ids.contains(&e.target_node_id) {
                continue;
            }
            *in_degree.entry(e.target_node_id).or_insert(0) += 1;
            edges_from
                .entry(e.source_node_id)
                .or_default()
                .push(e.target_node_id);
        }

        let mut tiers: Vec<Vec<Uuid>> = Vec::new();
        let mut ready: Vec<Uuid> = in_degree
            .iter()
            .filter(|(_, &d)| d == 0)
            .map(|(&id, _)| id)
            .collect();
        let mut placed = 0usize;

        while !ready.is_empty() {
            ready.sort(); // 确定性输出，便于测试与 diff
            let mut next: Vec<Uuid> = Vec::new();
            for id in &ready {
                if let Some(targets) = edges_from.get(id) {
                    for t in targets {
                        let d = in_degree.get_mut(t).expect("target in ids");
                        *d -= 1;
                        if *d == 0 {
                            next.push(*t);
                        }
                    }
                }
            }
            placed += ready.len();
            tiers.push(ready);
            ready = next;
        }

        if placed != flow.graph.nodes.len() {
            // 有节点永远入度为正 → 环
            return Err(FlowError::Cycle);
        }
        Ok(ExecutionPlan { tiers })
    }

    /// Source 节点 id（约定 MVP 恰好一个；数量校验在 validate）。
    pub fn source_id(&self, flow: &Flow) -> Option<Uuid> {
        flow.graph
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Source)
            .map(|n| n.id)
    }

    pub fn node<'a>(&self, flow: &'a Flow, id: Uuid) -> Option<&'a crate::flow::FlowNode> {
        flow.graph.nodes.iter().find(|n| n.id == id)
    }
}
