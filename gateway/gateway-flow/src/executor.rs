//! DAG executor for running flows.

use super::error::FlowError;
use super::flow::Flow;
use petgraph::graph::{DiGraph, NodeIndex};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

pub struct DagExecutor;

impl DagExecutor {
    pub fn new() -> Self {
        Self
    }
    
    /// Compute topological sort order of flow nodes.
    /// Returns nodes in execution order (all sources first, sinks last).
    pub fn topological_order(&self, flow: &Flow) -> Result<Vec<Uuid>, FlowError> {
        let mut graph: DiGraph<(), ()> = DiGraph::new();
        let mut index_map: HashMap<Uuid, NodeIndex> = HashMap::new();
        
        for node in &flow.nodes {
            let idx = graph.add_node(());
            index_map.insert(node.id, idx);
        }
        
        for edge in &flow.edges {
            if let (Some(&src), Some(&tgt)) = (
                index_map.get(&edge.source_node_id),
                index_map.get(&edge.target_node_id),
            ) {
                graph.add_edge(src, tgt, ());
            }
        }
        
        let topo = petgraph::algo::toposort(&graph, None)
            .map_err(|_| FlowError::Validation("cycle detected in flow".into()))?;
        
        // Convert NodeIndex back to Uuid
        let id_map: HashMap<NodeIndex, Uuid> = index_map.iter()
            .map(|(k, v)| (*v, *k))
            .collect();
        
        Ok(topo.into_iter().filter_map(|idx| id_map.get(&idx).copied()).collect())
    }
    
    /// Group nodes into tiers (parallel execution tiers).
    /// Returns a vector of tiers; nodes within a tier can run in parallel.
    pub fn execution_tiers(&self, flow: &Flow) -> Result<Vec<Vec<Uuid>>, FlowError> {
        self.topological_order(flow)?;
        let mut in_degree: HashMap<Uuid, usize> = flow.nodes.iter()
            .map(|n| (n.id, 0))
            .collect();
        
        for edge in &flow.edges {
            *in_degree.entry(edge.target_node_id).or_insert(0) += 1;
        }
        
        let mut tiers: Vec<Vec<Uuid>> = Vec::new();
        let mut remaining: HashSet<Uuid> = flow.nodes.iter().map(|n| n.id).collect();
        
        while !remaining.is_empty() {
            // Find all nodes with in_degree == 0 that are still remaining
            let tier: Vec<Uuid> = remaining.iter()
                .filter(|id| in_degree.get(*id) == Some(&0))
                .copied()
                .collect();
            
            if tier.is_empty() {
                // Should not happen if validate() passed, but guard anyway
                return Err(FlowError::Validation("unable to compute execution tiers".into()));
            }
            
            tiers.push(tier.clone());
            
            // Remove these nodes and update in_degrees
            for node_id in tier {
                remaining.remove(&node_id);
                for edge in &flow.edges {
                    if edge.source_node_id == node_id {
                        if let Some(d) = in_degree.get_mut(&edge.target_node_id) {
                            *d = d.saturating_sub(1);
                        }
                    }
                }
            }
        }
        
        Ok(tiers)
    }
}

impl Default for DagExecutor {
    fn default() -> Self {
        Self::new()
    }
}
