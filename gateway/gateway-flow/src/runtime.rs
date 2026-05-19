//! Flow runtime: manages a deployed flow's lifecycle.

use std::collections::HashMap;
use uuid::Uuid;

use crate::{Flow, NodeKind};
use crate::error::FlowError;
use crate::executor::DagExecutor;
use crate::registry::OperatorRegistry;
use gateway_sdk::{NodeId, Operable, OperatorMetrics};

/// Runtime state for a single flow node.
struct NodeRuntime {
    node_id: NodeId,
    operator: Box<dyn Operable>,
    metrics: OperatorMetrics,
}

/// Manages the runtime execution of a deployed flow.
pub struct FlowRuntime {
    flow_id: Uuid,
    flow_name: String,
    status: crate::flow::FlowStatus,
    nodes: HashMap<Uuid, NodeRuntime>,
    executor: DagExecutor,
    registry: OperatorRegistry,
}

impl FlowRuntime {
    /// Create a new runtime for a flow, instantiating all operator nodes.
    pub async fn new(flow: &Flow, registry: OperatorRegistry) -> Result<Self, FlowError> {
        // Validate flow first
        flow.validate()?;
        
        let mut nodes = HashMap::new();
        
        // Instantiate operators for Operator-kind nodes
        for node in &flow.nodes {
            if node.kind == NodeKind::Operator {
                let operator_name = node.operator_name.as_deref()
                    .ok_or_else(|| FlowError::Validation(
                        format!("operator node '{}' has no operator_name", node.name)
                    ))?;
                
                let operator = registry.create(operator_name)
                    .ok_or_else(|| FlowError::NotFound(
                        format!("operator '{}' not found in registry", operator_name)
                    ))?;
                
                let node_id = NodeId(node.id);
                
                // Open the operator instance
                operator.open(node_id, node.config.clone())
                    .await
                    .map_err(|e| FlowError::Execution(format!("open operator '{}': {}", operator_name, e)))?;
                
                let metrics = OperatorMetrics::new(
                    flow.id.to_string(),
                    node.id.to_string(),
                    node.name.clone(),
                    operator_name.to_string(),
                );
                
                nodes.insert(node.id, NodeRuntime { node_id, operator, metrics });
            }
        }
        
        Ok(Self {
            flow_id: flow.id,
            flow_name: flow.name.clone(),
            status: crate::flow::FlowStatus::Deployed,
            nodes,
            executor: DagExecutor::new(),
            registry,
        })
    }
    
    /// Start the flow — set status to Running.
    pub fn start(&mut self) {
        self.status = crate::flow::FlowStatus::Running;
    }
    
    /// Pause the flow — set status to Paused.
    pub fn pause(&mut self) {
        self.status = crate::flow::FlowStatus::Paused;
    }
    
    /// Stop the flow — close all operators and set status to Stopped.
    pub async fn stop(&mut self) -> Result<(), FlowError> {
        for (_, node) in self.nodes.iter_mut() {
            node.operator.close(node.node_id)
                .await
                .map_err(|e| FlowError::Execution(format!("close operator: {}", e)))?;
        }
        self.nodes.clear();
        self.status = crate::flow::FlowStatus::Stopped;
        Ok(())
    }
    
    /// Get current flow status.
    pub fn status(&self) -> crate::flow::FlowStatus {
        self.status
    }
    
    /// Get all node metrics.
    pub fn metrics(&self) -> Vec<OperatorMetrics> {
        self.nodes.values().map(|n| n.metrics.clone()).collect()
    }
    
    /// Get metrics for a specific node.
    pub fn node_metrics(&self, node_id: Uuid) -> Option<&OperatorMetrics> {
        self.nodes.get(&node_id).map(|n| &n.metrics)
    }
}
