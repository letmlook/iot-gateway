//! DAG executor for running flows.

use super::{Flow, FlowError};
use tokio::sync::mpsc;

/// DAG executor: topologically sorts nodes and drives execution.
pub struct DagExecutor;

impl DagExecutor {
    pub fn new() -> Self {
        Self
    }
    
    /// Validate that the flow is a valid DAG (no cycles).
    pub fn validate(&self, flow: &Flow) -> Result<(), FlowError> {
        // TODO: implement cycle detection using petgraph
        Ok(())
    }
}

impl Default for DagExecutor {
    fn default() -> Self {
        Self::new()
    }
}
