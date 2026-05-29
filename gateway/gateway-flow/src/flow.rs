//! Flow definition and validation.

use chrono::{DateTime, Utc};
use petgraph::graph::{DiGraph, NodeIndex};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

use super::node::{FlowNode, NodeKind};
use crate::error::FlowError;
use gateway_sdk::{GroupId, NodeId};

/// Flow runtime status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FlowStatus {
    Draft,
    Deployed,
    Running,
    Paused,
    Stopped,
    Error,
}

/// Edge in the flow graph (source_node -> target_node).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowEdge {
    pub source_node_id: Uuid,
    pub source_port: String,
    pub target_node_id: Uuid,
    pub target_port: String,
}

/// What the Manager should do when a bound flow fails while processing south data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FlowFailurePolicy {
    FailOpen,
    FailClosed,
    PublishError,
}

impl Default for FlowFailurePolicy {
    fn default() -> Self {
        Self::FailOpen
    }
}

/// Binds a flow to one south node/group source in the main data path.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowBinding {
    pub flow_id: Uuid,
    pub south_node_id: NodeId,
    pub group_id: GroupId,
    pub enabled: bool,
    #[serde(default)]
    pub failure_policy: FlowFailurePolicy,
}

/// A flow definition: nodes + edges + metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Flow {
    // ---------- Identity ----------
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,

    // ---------- Persistence ----------
    pub status: FlowStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,

    // ---------- Graph ----------
    pub nodes: Vec<FlowNode>,
    pub edges: Vec<FlowEdge>,

    // ---------- Data-path bindings ----------
    #[serde(default)]
    pub bindings: Vec<FlowBinding>,

    // ---------- Versioning ----------
    pub version: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flow_binding_serializes_south_group_source() {
        let flow_id = Uuid::new_v4();
        let south_node_id = NodeId(Uuid::new_v4());
        let group_id = GroupId(Uuid::new_v4());
        let binding = FlowBinding {
            flow_id,
            south_node_id,
            group_id,
            enabled: true,
            failure_policy: FlowFailurePolicy::FailOpen,
        };

        let json = serde_json::to_value(&binding).unwrap();
        assert_eq!(json["flow_id"], flow_id.to_string());
        assert_eq!(json["south_node_id"], south_node_id.0.to_string());
        assert_eq!(json["group_id"], group_id.0.to_string());
        assert_eq!(json["enabled"], true);
        assert_eq!(json["failure_policy"], "fail_open");

        let decoded: FlowBinding = serde_json::from_value(json).unwrap();
        assert_eq!(decoded.flow_id, flow_id);
        assert_eq!(decoded.south_node_id, south_node_id);
        assert_eq!(decoded.group_id, group_id);
        assert_eq!(decoded.failure_policy, FlowFailurePolicy::FailOpen);
    }
}

impl Flow {
    // ---------- Builders ----------

    pub fn new(name: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            description: None,
            status: FlowStatus::Draft,
            created_at: now,
            updated_at: now,
            nodes: Vec::new(),
            edges: Vec::new(),
            bindings: Vec::new(),
            version: 1,
        }
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn add_node(&mut self, node: FlowNode) {
        self.nodes.push(node);
        self.updated_at = Utc::now();
    }

    pub fn add_edge(&mut self, edge: FlowEdge) {
        self.edges.push(edge);
        self.updated_at = Utc::now();
    }

    // ---------- Lookups ----------

    pub fn node(&self, id: Uuid) -> Option<&FlowNode> {
        self.nodes.iter().find(|n| n.id == id)
    }

    pub fn node_mut(&mut self, id: Uuid) -> Option<&mut FlowNode> {
        self.nodes.iter_mut().find(|n| n.id == id)
    }

    // ---------- Validation ----------

    /// Validate the flow: checks for cycles, orphan nodes, port type matching, and required inputs.
    pub fn validate(&self) -> Result<(), FlowError> {
        // 1. Cycle detection via topological sort (petgraph)
        self.validate_dag()?;

        // 2. Every edge must connect existing nodes
        self.validate_edge_nodes()?;

        // 3. Port type matching: source output port must exist and target input port must exist
        self.validate_port_types()?;

        // 4. Required input ports must be satisfied (or have default)
        self.validate_required_inputs()?;

        // 5. At least one South and one North node
        self.validate_node_counts()?;

        Ok(())
    }

    fn validate_dag(&self) -> Result<(), FlowError> {
        let mut graph: DiGraph<(), ()> = DiGraph::new();
        // Create node index map: Uuid -> petgraph NodeIndex
        let mut index_map: HashMap<Uuid, NodeIndex> = HashMap::new();

        for node in &self.nodes {
            let idx = graph.add_node(());
            index_map.insert(node.id, idx);
        }

        for edge in &self.edges {
            let Some(&src_idx) = index_map.get(&edge.source_node_id) else {
                continue;
            };
            let Some(&tgt_idx) = index_map.get(&edge.target_node_id) else {
                continue;
            };
            graph.add_edge(src_idx, tgt_idx, ());
        }

        // Topological sort — if the number of sorted nodes != total nodes, there's a cycle
        match petgraph::algo::toposort(&graph, None) {
            Ok(sorted) => {
                if sorted.len() != self.nodes.len() {
                    return Err(FlowError::Validation("flow contains a cycle".into()));
                }
                Ok(())
            }
            Err(cycle_idx) => Err(FlowError::Validation(format!(
                "flow contains a cycle at node index {:?}",
                cycle_idx
            ))),
        }
    }

    fn validate_edge_nodes(&self) -> Result<(), FlowError> {
        let node_ids: HashSet<Uuid> = self.nodes.iter().map(|n| n.id).collect();
        for edge in &self.edges {
            if !node_ids.contains(&edge.source_node_id) {
                return Err(FlowError::Validation(format!(
                    "edge references unknown source node {}",
                    edge.source_node_id
                )));
            }
            if !node_ids.contains(&edge.target_node_id) {
                return Err(FlowError::Validation(format!(
                    "edge references unknown target node {}",
                    edge.target_node_id
                )));
            }
        }
        Ok(())
    }

    fn validate_port_types(&self) -> Result<(), FlowError> {
        for edge in &self.edges {
            let Some(source_node) = self.node(edge.source_node_id) else {
                continue;
            };
            let Some(target_node) = self.node(edge.target_node_id) else {
                continue;
            };

            // Source port must exist as an output port on source node
            let src_port = source_node
                .output_ports
                .iter()
                .find(|p| p.id == edge.source_port || p.name == edge.source_port);
            if src_port.is_none() {
                return Err(FlowError::Validation(format!(
                    "edge from {}: port '{}' not found on source node '{}' (kind={:?})",
                    edge.source_node_id, edge.source_port, source_node.name, source_node.kind
                )));
            }

            // Target port must exist as an input port on target node
            let tgt_port = target_node
                .input_ports
                .iter()
                .find(|p| p.id == edge.target_port || p.name == edge.target_port);
            if tgt_port.is_none() {
                return Err(FlowError::Validation(format!(
                    "edge to {}: port '{}' not found on target node '{}' (kind={:?})",
                    edge.target_node_id, edge.target_port, target_node.name, target_node.kind
                )));
            }
        }
        Ok(())
    }

    fn validate_required_inputs(&self) -> Result<(), FlowError> {
        // For each node, check that all required input ports are covered by edges
        for node in &self.nodes {
            for in_port in &node.input_ports {
                if !in_port.required {
                    continue;
                }
                let has_edge = self.edges.iter().any(|e| {
                    e.target_node_id == node.id
                        && (e.target_port == in_port.id || e.target_port == in_port.name)
                });
                if !has_edge {
                    return Err(FlowError::Validation(format!(
                        "required input port '{}' on node '{}' (kind={:?}) has no incoming edge",
                        in_port.name, node.name, node.kind
                    )));
                }
            }
        }
        Ok(())
    }

    fn validate_node_counts(&self) -> Result<(), FlowError> {
        let has_south = self.nodes.iter().any(|n| n.kind == NodeKind::South);
        let has_north = self.nodes.iter().any(|n| n.kind == NodeKind::North);

        if !has_south {
            return Err(FlowError::Validation(
                "flow must have at least one South node".into(),
            ));
        }
        if !has_north {
            return Err(FlowError::Validation(
                "flow must have at least one North node".into(),
            ));
        }
        Ok(())
    }
}
