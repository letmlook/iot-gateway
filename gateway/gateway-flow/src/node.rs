//! Flow node types.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Node kind in a flow: South (data source), Operator (processing), North (data sink).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NodeKind {
    South,
    Operator,
    North,
}

/// Port type for type-safe connections.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PortType {
    Data,
    Control,
}

/// A port on a node (input or output).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Port {
    pub id: String,
    pub name: String,
    pub port_type: PortType,
    pub required: bool,
}

/// A node in a flow graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowNode {
    pub id: Uuid,
    pub name: String,
    pub kind: NodeKind,
    pub operator_name: Option<String>,
    pub config: gateway_sdk::PluginConfig,
    pub input_ports: Vec<Port>,
    pub output_ports: Vec<Port>,
}
