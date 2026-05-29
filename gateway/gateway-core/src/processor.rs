//! GroupData processing seam for the main south-to-north data path.
//!
//! The Manager calls the installed processor (if any) before publishing
//! GroupData to the Bus. This allows gateway-server to wire in the
//! FlowRuntime without gateway-core depending on gateway-flow.

use gateway_sdk::{GroupData, GroupId, NodeId};
use std::sync::Arc;

/// Decision returned by a GroupDataProcessor after examining south poll data.
#[derive(Debug, Clone)]
pub enum ProcessDecision {
    /// Publish this (possibly transformed) data to the Bus.
    Publish(Arc<GroupData>),
    /// Drop the data — it was consumed or filtered by the processor.
    Drop,
}

/// Async trait for processing south GroupData before Bus publish.
///
/// Implemented by gateway-server's FlowRuntime wiring; gateway-core
/// remains flow-agnostic.
#[async_trait::async_trait]
pub trait GroupDataProcessor: Send + Sync {
    /// Process a GroupData produced by a south poll.
    ///
    /// Returns a decision: publish (possibly modified), or drop.
    /// On processing failure, implementations should follow their
    /// configured failure policy (fail-open / fail-closed).
    async fn process_group_data(
        &self,
        south_node_id: NodeId,
        group_id: GroupId,
        data: Arc<GroupData>,
    ) -> ProcessDecision;
}
