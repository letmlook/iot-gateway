//! Operator runtime metrics.

use serde::{Deserialize, Serialize};

/// Per-node operator metrics exposed via API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperatorMetrics {
    /// Flow ID this node belongs to.
    pub flow_id: String,
    /// Node ID.
    pub node_id: String,
    /// Node name.
    pub node_name: String,
    /// Operator name.
    pub operator_name: String,
    /// Total number of items this operator has processed since startup.
    pub processed_count: u64,
    /// Total number of items this operator has output (may be less than processed_count if filter drops data).
    pub output_count: u64,
    /// Total number of items that were dropped (e.g., filter condition not met).
    pub dropped_count: u64,
    /// Number of errors encountered during processing.
    pub error_count: u64,
    /// Timestamp of last successful processing.
    pub last_processed_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Timestamp of last error.
    pub last_error_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Last error message (if any).
    pub last_error: Option<String>,
    /// Current processing rate (items/sec) — calculated over a sliding window.
    #[serde(default)]
    pub current_rate: f64,
}

impl OperatorMetrics {
    pub fn new(flow_id: impl Into<String>, node_id: impl Into<String>, node_name: impl Into<String>, operator_name: impl Into<String>) -> Self {
        Self {
            flow_id: flow_id.into(),
            node_id: node_id.into(),
            node_name: node_name.into(),
            operator_name: operator_name.into(),
            processed_count: 0,
            output_count: 0,
            dropped_count: 0,
            error_count: 0,
            last_processed_at: None,
            last_error_at: None,
            last_error: None,
            current_rate: 0.0,
        }
    }
    
    pub fn record_processed(&mut self) {
        self.processed_count += 1;
        self.last_processed_at = Some(chrono::Utc::now());
    }
    
    pub fn record_output(&mut self) {
        self.output_count += 1;
    }
    
    pub fn record_dropped(&mut self) {
        self.dropped_count += 1;
    }
    
    pub fn record_error(&mut self, message: impl Into<String>) {
        self.error_count += 1;
        self.last_error_at = Some(chrono::Utc::now());
        self.last_error = Some(message.into());
    }
}