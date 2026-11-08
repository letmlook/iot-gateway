//! 流程定义与图模型。
//!
//! 对应 `docs/design/流程编排-v2.md` §2：单 JSON 文档存储（graph_json 含 position，无独立列，
//! 踩坑 P4 对策）；入库仅 draft/published 两态，运行态由运行时派生。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// 流程入库状态（仅两态；running/error 由运行时派生，不入库）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FlowStatus {
    Draft,
    Published,
}

/// 节点类别：南向源 / 处理算子 / 输出端
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NodeKind {
    Source,
    Operator,
    Sink,
}

/// 图中节点。position 属于 graph 文档本身（编辑器整图保存），不设独立持久化列。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowNode {
    pub id: Uuid,
    pub name: String,
    pub kind: NodeKind,
    /// Operator 节点的算子名（见 `crate::operators::OPERATOR_NAMES`）；Source/Sink 可为 None
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operator: Option<String>,
    /// 算子/端点配置（JSON，publish 时按算子 schema 校验）
    #[serde(default)]
    pub config: serde_json::Value,
    /// 编辑器画布位置（px）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<Position>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Position {
    pub x: f64,
    pub y: f64,
}

/// 有向边：source 节点的 source_port → target 节点（MVP：每节点恰好一个输入，见 validate）
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FlowEdge {
    pub source_node_id: Uuid,
    pub source_port: String,
    pub target_node_id: Uuid,
}

/// 图：节点 + 边。编辑器整图保存的唯一载荷。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FlowGraph {
    #[serde(default)]
    pub nodes: Vec<FlowNode>,
    #[serde(default)]
    pub edges: Vec<FlowEdge>,
}

/// 流程定义。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Flow {
    pub id: Uuid,
    #[serde(default)]
    pub tenant_id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub status: FlowStatus,
    /// 每次 publish 递增
    #[serde(default)]
    pub version: i64,
    #[serde(flatten)]
    pub graph: FlowGraph,
    #[serde(default)]
    pub created_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub updated_at: Option<DateTime<Utc>>,
}
