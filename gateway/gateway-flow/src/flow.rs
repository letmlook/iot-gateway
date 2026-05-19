//! Flow definition.

use serde::{Deserialize, Serialize};
use uuid::Uuid;
use super::node::FlowNode;

/// Edge in the flow graph (source_node -> target_node).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowEdge {
    pub source_node_id: Uuid,
    pub source_port: String,
    pub target_node_id: Uuid,
    pub target_port: String,
}

/// A flow definition: nodes + edges + metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Flow {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub nodes: Vec<FlowNode>,
    pub edges: Vec<FlowEdge>,
    pub version: i64,
}
