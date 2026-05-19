//! Prometheus metrics for the gateway server.
//!
//! Provides global counters/gauges for flow runtime metrics and a Prometheus
//! text-format renderer.

use std::sync::atomic::{AtomicU64, Ordering};

/// Global counter: currently running flows.
pub static FLOWS_RUNNING: AtomicU64 = AtomicU64::new(0);

/// Global counter: total number of flows (created).
pub static FLOWS_TOTAL: AtomicU64 = AtomicU64::new(0);

/// Global counter: total nodes processed across all flows.
pub static NODES_PROCESSED_TOTAL: AtomicU64 = AtomicU64::new(0);

/// Global counter: total errors encountered across all flows.
pub static ERRORS_TOTAL: AtomicU64 = AtomicU64::new(0);

/// Increment the running flows count.
pub fn set_flows_running(count: u64) {
    FLOWS_RUNNING.store(count, Ordering::Relaxed);
}

/// Increment the total flows count.
pub fn set_flows_total(count: u64) {
    FLOWS_TOTAL.store(count, Ordering::Relaxed);
}

/// Record nodes processed (adds to counter).
pub fn record_nodes_processed(count: u64) {
    NODES_PROCESSED_TOTAL.fetch_add(count, Ordering::Relaxed);
}

/// Record a single error.
pub fn record_error() {
    ERRORS_TOTAL.fetch_add(1, Ordering::Relaxed);
}

/// Render all metrics in Prometheus exposition text format.
pub fn render_prometheus() -> String {
    let flows_running = FLOWS_RUNNING.load(Ordering::Relaxed);
    let flows_total = FLOWS_TOTAL.load(Ordering::Relaxed);
    let nodes_processed = NODES_PROCESSED_TOTAL.load(Ordering::Relaxed);
    let errors = ERRORS_TOTAL.load(Ordering::Relaxed);

    format!(
        "# HELP gateway_flows_running Number of currently running flows.\n\
         # TYPE gateway_flows_running gauge\n\
         gateway_flows_running {}\n\
         # HELP gateway_flows_total Total number of flows (created).\n\
         # TYPE gateway_flows_total gauge\n\
         gateway_flows_total {}\n\
         # HELP gateway_nodes_processed_total Total number of nodes processed across all flows.\n\
         # TYPE gateway_nodes_processed_total counter\n\
         gateway_nodes_processed_total {}\n\
         # HELP gateway_errors_total Total number of errors across all flows.\n\
         # TYPE gateway_errors_total counter\n\
         gateway_errors_total {}\n",
        flows_running, flows_total, nodes_processed, errors
    )
}
