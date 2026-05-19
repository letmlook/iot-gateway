//! # gateway-flow
//!
//! Flow orchestration: DAG execution, node registry, built-in operators.

pub mod flow;
pub mod node;
pub mod executor;
pub mod registry;
pub mod error;

pub use flow::Flow;
pub use node::{FlowNode, NodeKind, Port, PortType};
pub use executor::DagExecutor;
pub use registry::OperatorRegistry;
pub use error::FlowError;
