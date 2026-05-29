//! Flow-backed GroupData processor for the Manager main data path.

use crate::alarm::AlarmStore;
use crate::flow::FlowStore;
use crate::websocket::{AlarmLiveEvent, FlowLiveEvent, WsHub};
use gateway_core::{GroupDataProcessor, ProcessDecision};
use gateway_flow::{FlowFailurePolicy, FlowRuntime, OperatorRegistry};
use gateway_sdk::{DataValue, GroupData, GroupId, NodeId, PipelineData, TagId};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

/// Wires persisted Flow bindings and runtimes into gateway-core's flow-agnostic
/// pre-bus processing seam.
#[derive(Clone)]
pub struct FlowGroupDataProcessor {
    store: FlowStore,
    alarm_store: AlarmStore,
    runtimes: Arc<RwLock<HashMap<Uuid, FlowRuntime>>>,
    ws_hub: WsHub,
}

impl FlowGroupDataProcessor {
    pub fn new(
        store: FlowStore,
        alarm_store: AlarmStore,
        runtimes: Arc<RwLock<HashMap<Uuid, FlowRuntime>>>,
        ws_hub: WsHub,
    ) -> Self {
        Self {
            store,
            alarm_store,
            runtimes,
            ws_hub,
        }
    }

    fn group_data_to_pipeline(data: &GroupData) -> PipelineData {
        let mut payload = HashMap::new();
        for (tag_id, value) in &data.values {
            let key = data
                .tag_names
                .as_ref()
                .and_then(|names| names.get(tag_id))
                .cloned()
                .unwrap_or_else(|| tag_id.0.to_string());
            payload.insert(key, value.clone());
        }

        let mut pipeline = PipelineData::new(data.node_id).with_payload(payload);
        pipeline.ts = data.ts;
        pipeline
            .metadata
            .insert("south_node_id".to_string(), data.node_id.0.to_string());
        pipeline
            .metadata
            .insert("group_id".to_string(), data.group_id.0.to_string());
        if let Some(name) = &data.node_name {
            pipeline
                .metadata
                .insert("node_name".to_string(), name.clone());
        }
        if let Some(name) = &data.group_name {
            pipeline
                .metadata
                .insert("group_name".to_string(), name.clone());
        }
        pipeline
    }

    fn pipeline_to_group_data(original: &GroupData, output: PipelineData) -> GroupData {
        let mut key_to_tag_id = HashMap::<String, TagId>::new();
        for (tag_id, _) in &original.values {
            key_to_tag_id.insert(tag_id.0.to_string(), *tag_id);
        }
        if let Some(tag_names) = &original.tag_names {
            for (tag_id, name) in tag_names {
                key_to_tag_id.insert(name.clone(), *tag_id);
            }
        }

        let mut values = Vec::<(TagId, DataValue)>::new();
        for (key, value) in output.payload {
            if let Some(tag_id) = key_to_tag_id.get(&key) {
                values.push((*tag_id, value));
            }
        }

        GroupData {
            node_id: original.node_id,
            group_id: original.group_id,
            ts: output.ts,
            values,
            node_name: original.node_name.clone(),
            group_name: original.group_name.clone(),
            tag_names: original.tag_names.clone(),
        }
    }

    fn group_data_values_json(data: &GroupData) -> serde_json::Value {
        let mut values = serde_json::Map::new();
        for (tag_id, value) in &data.values {
            let key = data
                .tag_names
                .as_ref()
                .and_then(|names| names.get(tag_id))
                .cloned()
                .unwrap_or_else(|| tag_id.0.to_string());
            values.insert(key, serde_json::to_value(value).unwrap_or(serde_json::Value::Null));
        }
        serde_json::Value::Object(values)
    }

    async fn broadcast_processed_data(&self, flow_id: Uuid, data: &GroupData) {
        self.ws_hub
            .broadcast(
                flow_id,
                FlowLiveEvent::DataProcessed {
                    flow_id: flow_id.to_string(),
                    south_node_id: data.node_id.0.to_string(),
                    group_id: data.group_id.0.to_string(),
                    node_name: data.node_name.clone(),
                    group_name: data.group_name.clone(),
                    values: Self::group_data_values_json(data),
                },
            )
            .await;
    }

    async fn ensure_runtime(&self, flow_id: Uuid) -> Result<(), String> {
        if self.runtimes.read().await.contains_key(&flow_id) {
            return Ok(());
        }

        let flow = self
            .store
            .get_flow(flow_id)
            .await
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "bound flow not found".to_string())?;
        let runtime = FlowRuntime::new(&flow, OperatorRegistry::new())
            .await
            .map_err(|e| e.to_string())?;
        self.runtimes.write().await.insert(flow_id, runtime);
        Ok(())
    }

    async fn persist_alarm_events(
        &self,
        drafts: Vec<gateway_flow::operators::alarm::AlarmEventDraft>,
    ) {
        for draft in drafts {
            match self.alarm_store.create_from_draft(draft).await {
                Ok(event) => {
                    self.ws_hub
                        .broadcast_alarm(AlarmLiveEvent::Created { event })
                        .await;
                }
                Err(e) => {
                    tracing::warn!("persist alarm event failed: {}", e);
                }
            }
        }
    }

    fn failure_decision(
        policy: FlowFailurePolicy,
        data: Arc<GroupData>,
        message: String,
    ) -> ProcessDecision {
        match policy {
            FlowFailurePolicy::FailOpen => {
                tracing::warn!(
                    "bound flow execution failed; publishing raw data: {}",
                    message
                );
                ProcessDecision::Publish(data)
            }
            FlowFailurePolicy::FailClosed => {
                tracing::warn!("bound flow execution failed; dropping data: {}", message);
                ProcessDecision::Drop
            }
            FlowFailurePolicy::PublishError => {
                tracing::warn!("bound flow execution failed; publish_error currently falls back to raw data: {}", message);
                ProcessDecision::Publish(data)
            }
        }
    }
}

#[async_trait::async_trait]
impl GroupDataProcessor for FlowGroupDataProcessor {
    async fn process_group_data(
        &self,
        south_node_id: NodeId,
        group_id: GroupId,
        data: Arc<GroupData>,
    ) -> ProcessDecision {
        let binding = match self
            .store
            .enabled_binding_for_source(south_node_id, group_id)
            .await
        {
            Ok(Some(binding)) if binding.enabled => binding,
            Ok(_) => return ProcessDecision::Publish(data),
            Err(e) => {
                return Self::failure_decision(FlowFailurePolicy::FailOpen, data, e.to_string())
            }
        };

        if let Err(e) = self.ensure_runtime(binding.flow_id).await {
            return Self::failure_decision(binding.failure_policy, data, e);
        }

        let input = Self::group_data_to_pipeline(&data);
        let execution = {
            let mut runtimes = self.runtimes.write().await;
            let Some(runtime) = runtimes.get_mut(&binding.flow_id) else {
                return Self::failure_decision(
                    binding.failure_policy,
                    data,
                    "flow runtime unavailable after initialization".to_string(),
                );
            };
            runtime.execute(vec![input]).await
        };

        match execution {
            Ok(result) if result.output.is_empty() => {
                self.persist_alarm_events(result.alarm_events).await;
                ProcessDecision::Drop
            }
            Ok(result) => {
                self.persist_alarm_events(result.alarm_events).await;
                let Some(first) = result.output.into_iter().next() else {
                    return ProcessDecision::Drop;
                };
                let processed = Self::pipeline_to_group_data(&data, first);
                self.broadcast_processed_data(binding.flow_id, &processed).await;
                ProcessDecision::Publish(Arc::new(processed))
            }
            Err(e) => Self::failure_decision(binding.failure_policy, data, e.to_string()),
        }
    }
}
