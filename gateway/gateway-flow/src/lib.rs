//! # gateway-flow
//!
//! Flow orchestration: DAG execution, node registry, built-in operators.

pub mod error;
pub mod executor;
pub mod flow;
pub mod node;
pub mod operators;
pub mod registry;
pub mod runtime;

pub use error::FlowError;
pub use executor::DagExecutor;
pub use flow::{Flow, FlowBinding, FlowEdge, FlowFailurePolicy, FlowStatus};
pub use node::{FlowNode, NodeKind, Port, PortType};
pub use registry::OperatorRegistry;
pub use runtime::FlowRuntime;
