//! Flow runtime: manages a deployed flow's lifecycle.

use std::collections::HashMap;
use uuid::Uuid;

use crate::error::FlowError;
use crate::executor::DagExecutor;
use crate::registry::OperatorRegistry;
use crate::{Flow, NodeKind};
use gateway_sdk::{NodeId, Operable, OperatorMetrics, PipelineData, PluginConfig};

/// Runtime state for a single flow node.
struct NodeRuntime {
    node_id: NodeId,
    operator: Box<dyn Operable>,
    metrics: OperatorMetrics,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct FlowPreviewNodeOutput {
    pub node_id: Uuid,
    pub input_count: usize,
    pub output_count: usize,
    pub sample_output: Option<PipelineData>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct FlowExecutionResult {
    pub output: Vec<PipelineData>,
    pub nodes: Vec<FlowPreviewNodeOutput>,
    pub alarm_events: Vec<crate::operators::alarm::AlarmEventDraft>,
}

fn create_operator_from_config(
    registry: &OperatorRegistry,
    operator_name: &str,
    config: &PluginConfig,
) -> Option<Box<dyn Operable>> {
    match operator_name {
        "filter" => Some(crate::operators::filter::create_filter_operator(config)),
        "transform" => Some(crate::operators::transform::create_transform_operator(
            config,
        )),
        "aggregate" => Some(crate::operators::aggregate::create_aggregate_operator(
            config,
        )),
        "router" => Some(crate::operators::router::create_router_operator(config)),
        "buffer" => Some(crate::operators::buffer::create_buffer_operator(config)),
        "alarm" => Some(crate::operators::alarm::create_alarm_operator(config)),
        "json-path" => Some(crate::operators::json_path::create_json_path_operator(
            config,
        )),
        "deadband" => Some(crate::operators::deadband::create_deadband_operator(config)),
        "formula" => Some(crate::operators::formula::create_formula_operator(config)),
        "clamp" => Some(crate::operators::clamp::create_clamp_operator(config)),
        "round" => Some(crate::operators::round::create_round_operator(config)),
        "change" => Some(crate::operators::change::create_change_operator(config)),
        "range" => Some(crate::operators::range::create_range_operator(config)),
        "batch" => Some(crate::operators::batch::create_batch_operator(config)),
        "split" => Some(crate::operators::split::create_split_operator(config)),
        "join" => Some(crate::operators::join::create_join_operator(config)),
        "dedup" => Some(crate::operators::dedup::create_dedup_operator(config)),
        "script" => Some(crate::operators::script::create_script_operator(config)),
        "throttle" => Some(crate::operators::throttle::create_throttle_operator(config)),
        "convert" => Some(crate::operators::convert::create_convert_operator(config)),
        "log" => Some(crate::operators::log::create_log_operator(config)),
        "xml-path" => Some(crate::operators::xml_path::create_xml_path_operator(config)),
        _ => registry.create(operator_name),
    }
}

/// Manages the runtime execution of a deployed flow.
pub struct FlowRuntime {
    flow_id: Uuid,
    flow_name: String,
    status: crate::flow::FlowStatus,
    nodes: HashMap<Uuid, NodeRuntime>,
    executor: DagExecutor,
    registry: OperatorRegistry,
    flow: Flow,
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
                let operator_name = node.operator_name.as_deref().ok_or_else(|| {
                    FlowError::Validation(format!(
                        "operator node '{}' has no operator_name",
                        node.name
                    ))
                })?;

                let operator = create_operator_from_config(&registry, operator_name, &node.config)
                    .ok_or_else(|| {
                        FlowError::NotFound(format!(
                            "operator '{}' not found in registry",
                            operator_name
                        ))
                    })?;

                let node_id = NodeId(node.id);

                // Open the operator instance
                operator
                    .open(node_id, node.config.clone())
                    .await
                    .map_err(|e| {
                        FlowError::Execution(format!("open operator '{}': {}", operator_name, e))
                    })?;

                let metrics = OperatorMetrics::new(
                    flow.id.to_string(),
                    node.id.to_string(),
                    node.name.clone(),
                    operator_name.to_string(),
                );

                nodes.insert(
                    node.id,
                    NodeRuntime {
                        node_id,
                        operator,
                        metrics,
                    },
                );
            }
        }

        Ok(Self {
            flow_id: flow.id,
            flow_name: flow.name.clone(),
            status: crate::flow::FlowStatus::Deployed,
            nodes,
            executor: DagExecutor::new(),
            registry,
            flow: flow.clone(),
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
            node.operator
                .close(node.node_id)
                .await
                .map_err(|e| FlowError::Execution(format!("close operator: {}", e)))?;
        }
        self.nodes.clear();
        self.status = crate::flow::FlowStatus::Stopped;
        Ok(())
    }

    /// Execute the flow with sample/input data and collect per-node outputs.
    pub async fn execute(
        &mut self,
        input: Vec<PipelineData>,
    ) -> Result<FlowExecutionResult, FlowError> {
        let order = self.executor.topological_order(&self.flow)?;
        let mut current = input;
        let mut node_outputs = Vec::new();
        let mut alarm_events = Vec::new();

        for node_id in order {
            let Some(runtime) = self.nodes.get_mut(&node_id) else {
                continue;
            };

            let input_count = current.len();
            let mut next = Vec::new();
            for item in current.into_iter() {
                runtime.metrics.record_processed();
                match runtime.operator.process(runtime.node_id, item).await {
                    Ok(outputs) => {
                        if outputs.is_empty() {
                            runtime.metrics.record_dropped();
                        } else {
                            for output in &outputs {
                                runtime.metrics.record_output();
                                if let Some(raw_event) = output.metadata.get("alarm.event") {
                                    if let Ok(event) =
                                        serde_json::from_str::<crate::operators::alarm::AlarmEvent>(
                                            raw_event,
                                        )
                                    {
                                        alarm_events.push(crate::operators::alarm::AlarmEventDraft::from_alarm_event(runtime.node_id, event));
                                    }
                                }
                            }
                        }
                        next.extend(outputs);
                    }
                    Err(e) => {
                        let message = e.to_string();
                        runtime.metrics.record_error(message.clone());
                        return Err(FlowError::Execution(message));
                    }
                }
            }

            let output_count = next.len();
            let sample_output = next.first().cloned();
            node_outputs.push(FlowPreviewNodeOutput {
                node_id,
                input_count,
                output_count,
                sample_output,
            });
            current = next;
        }

        Ok(FlowExecutionResult {
            output: current,
            nodes: node_outputs,
            alarm_events,
        })
    }

    /// Get current flow status.
    pub fn status(&self) -> crate::flow::FlowStatus {
        self.status
    }

    /// Get flow name.
    pub fn flow_name(&self) -> &str {
        &self.flow_name
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
