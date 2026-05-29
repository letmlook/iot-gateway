//! Flow API handlers.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use uuid::Uuid;

use crate::api::ApiError;
use crate::flow::store::FlowStoreError;
use crate::state::AppState;
use gateway_flow::{Flow, FlowBinding};
use gateway_sdk::{DataValue, NodeId, OperatorMetrics, PipelineData};

impl From<FlowStoreError> for ApiError {
    fn from(e: FlowStoreError) -> Self {
        ApiError::internal(e.to_string())
    }
}

pub async fn list_flows(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let flows = state.flow_store.list_flows().await?;
    Ok(Json(serde_json::json!({ "flows": flows })))
}

/// Flexible flow creation: accepts either a full Flow or a minimal { name, nodes?, edges? }.
/// If `id` is absent, auto-generates one (UUID) plus timestamps/status/version.
#[derive(serde::Deserialize)]
struct FlowCreate {
    id: Option<uuid::Uuid>,
    name: String,
    #[serde(default)]
    nodes: Vec<serde_json::Value>,
    #[serde(default)]
    edges: Vec<serde_json::Value>,
    #[serde(default)]
    description: Option<String>,
}

pub async fn create_flow(
    State(state): State<AppState>,
    Json(raw): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let flow = if raw.get("id").is_some() {
        // Full Flow submitted — deserialize and use as-is
        let f: Flow = serde_json::from_value(raw)
            .map_err(|e| ApiError::bad_request(format!("invalid flow: {e}")))?;
        f
    } else {
        // Minimal name-only (or name+nodes+edges) — auto-generate id/timestamps
        let create: FlowCreate = serde_json::from_value(raw)
            .map_err(|e| ApiError::bad_request(format!("invalid flow: {e}")))?;
        if create.name.trim().is_empty() {
            return Err(ApiError::bad_request("flow name cannot be empty"));
        }
        let now = chrono::Utc::now();
        Flow {
            id: Uuid::new_v4(),
            name: create.name,
            description: create.description,
            status: gateway_flow::FlowStatus::Draft,
            created_at: now,
            updated_at: now,
            nodes: Vec::new(),
            edges: Vec::new(),
            bindings: Vec::new(),
            version: 1,
        }
    };

    // Note: do NOT call flow.validate() here — a name-only Draft flow has no nodes yet.
    // Validation (DAG, port types, node counts) is done at deploy time only.
    state.flow_store.create_flow(&flow).await?;
    Ok(Json(serde_json::json!({ "flow": flow })))
}

pub async fn get_flow(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let id = Uuid::parse_str(&id).map_err(|_| ApiError::bad_request("invalid flow id"))?;
    let flow = state
        .flow_store
        .get_flow(id)
        .await?
        .ok_or_else(|| ApiError::not_found("flow not found"))?;
    Ok(Json(serde_json::json!({ "flow": flow })))
}

pub async fn update_flow(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(flow): Json<Flow>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let id = Uuid::parse_str(&id).map_err(|_| ApiError::bad_request("invalid flow id"))?;
    if flow.id != id {
        return Err(ApiError::bad_request("flow id mismatch"));
    }
    flow.validate()
        .map_err(|e| ApiError::bad_request(e.to_string()))?;
    state.flow_store.update_flow(&flow).await?;
    Ok(Json(serde_json::json!({ "flow": flow })))
}

pub async fn delete_flow(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let id = Uuid::parse_str(&id).map_err(|_| ApiError::bad_request("invalid flow id"))?;
    state.flow_store.delete_flow(id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(serde::Deserialize)]
pub struct SetFlowBindingsRequest {
    pub bindings: Vec<FlowBinding>,
}

pub async fn get_flow_bindings(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let id = Uuid::parse_str(&id).map_err(|_| ApiError::bad_request("invalid flow id"))?;
    state
        .flow_store
        .get_flow(id)
        .await?
        .ok_or_else(|| ApiError::not_found("flow not found"))?;
    let bindings = state.flow_store.list_bindings(id).await?;
    Ok(Json(serde_json::json!({ "bindings": bindings })))
}

pub async fn set_flow_bindings(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<SetFlowBindingsRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let id = Uuid::parse_str(&id).map_err(|_| ApiError::bad_request("invalid flow id"))?;
    state
        .flow_store
        .get_flow(id)
        .await?
        .ok_or_else(|| ApiError::not_found("flow not found"))?;

    let mut bindings = req.bindings;
    for binding in &mut bindings {
        binding.flow_id = id;
    }
    state.flow_store.replace_bindings(id, &bindings).await?;
    Ok(Json(serde_json::json!({ "bindings": bindings })))
}

#[derive(serde::Deserialize, Default)]
pub struct FlowPreviewInput {
    #[serde(default)]
    pub node_id: Option<Uuid>,
    #[serde(default)]
    pub payload: std::collections::HashMap<String, DataValue>,
    #[serde(default)]
    pub metadata: std::collections::HashMap<String, String>,
}

#[derive(serde::Deserialize, Default)]
pub struct FlowPreviewRequest {
    #[serde(default)]
    pub input: FlowPreviewInput,
}

pub async fn preview_flow(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<FlowPreviewRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let id = Uuid::parse_str(&id).map_err(|_| ApiError::bad_request("invalid flow id"))?;
    let flow = state
        .flow_store
        .get_flow(id)
        .await?
        .ok_or_else(|| ApiError::not_found("flow not found"))?;

    let registry = gateway_flow::OperatorRegistry::new();
    let mut runtime = gateway_flow::FlowRuntime::new(&flow, registry)
        .await
        .map_err(|e| ApiError::bad_request(format!("failed to create preview runtime: {}", e)))?;

    let source_node_id = req
        .input
        .node_id
        .or_else(|| {
            flow.nodes
                .iter()
                .find(|n| n.kind == gateway_flow::NodeKind::South)
                .map(|n| n.id)
        })
        .unwrap_or(flow.id);
    let mut data = PipelineData::new(NodeId(source_node_id)).with_payload(req.input.payload);
    data.metadata = req.input.metadata;

    let result = runtime
        .execute(vec![data])
        .await
        .map_err(|e| ApiError::bad_request(format!("flow preview failed: {}", e)))?;

    Ok(Json(serde_json::json!({
        "flow_id": id,
        "nodes": result.nodes,
        "output": result.output,
        "alarm_events": result.alarm_events,
        "errors": [],
    })))
}

// ---------- Lifecycle ----------

pub async fn deploy_flow(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let id = Uuid::parse_str(&id).map_err(|_| ApiError::bad_request("invalid flow id"))?;
    let flow = state
        .flow_store
        .get_flow(id)
        .await?
        .ok_or_else(|| ApiError::not_found("flow not found"))?;

    // Validate the flow
    flow.validate()
        .map_err(|e| ApiError::bad_request(format!("flow validation failed: {}", e)))?;

    // Create FlowRuntime and deploy
    let registry = gateway_flow::OperatorRegistry::new();
    let runtime = gateway_flow::FlowRuntime::new(&flow, registry)
        .await
        .map_err(|e| ApiError::bad_request(format!("failed to deploy flow: {}", e)))?;

    state
        .flow_store
        .update_flow_status(id, gateway_flow::FlowStatus::Deployed)
        .await?;

    // Store runtime in app state
    let mut runtimes = state.flow_runtimes.write().await;
    runtimes.insert(id, runtime);

    Ok(Json(serde_json::json!({ "status": "deployed" })))
}

pub async fn start_flow(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let id = Uuid::parse_str(&id).map_err(|_| ApiError::bad_request("invalid flow id"))?;

    let mut runtimes = state.flow_runtimes.write().await;
    let runtime = runtimes
        .get_mut(&id)
        .ok_or_else(|| ApiError::not_found("flow runtime not found, deploy first"))?;

    runtime.start();
    state
        .flow_store
        .update_flow_status(id, gateway_flow::FlowStatus::Running)
        .await?;

    Ok(Json(serde_json::json!({ "status": "running" })))
}

pub async fn pause_flow(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let id = Uuid::parse_str(&id).map_err(|_| ApiError::bad_request("invalid flow id"))?;

    let mut runtimes = state.flow_runtimes.write().await;
    let runtime = runtimes
        .get_mut(&id)
        .ok_or_else(|| ApiError::not_found("flow runtime not found"))?;

    runtime.pause();
    state
        .flow_store
        .update_flow_status(id, gateway_flow::FlowStatus::Paused)
        .await?;

    Ok(Json(serde_json::json!({ "status": "paused" })))
}

pub async fn stop_flow(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let id = Uuid::parse_str(&id).map_err(|_| ApiError::bad_request("invalid flow id"))?;

    let mut runtime = None;
    {
        let mut runtimes = state.flow_runtimes.write().await;
        if runtimes.contains_key(&id) {
            runtime = Some(runtimes.remove(&id).unwrap());
        }
    }

    if let Some(mut r) = runtime {
        r.stop()
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;
    }

    state
        .flow_store
        .update_flow_status(id, gateway_flow::FlowStatus::Stopped)
        .await?;

    Ok(Json(serde_json::json!({ "status": "stopped" })))
}

pub async fn flow_metrics(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let id = Uuid::parse_str(&id).map_err(|_| ApiError::bad_request("invalid flow id"))?;

    let runtimes = state.flow_runtimes.read().await;
    let runtime = runtimes
        .get(&id)
        .ok_or_else(|| ApiError::not_found("flow runtime not found"))?;

    let metrics: Vec<OperatorMetrics> = runtime.metrics();
    Ok(Json(serde_json::json!({ "metrics": metrics })))
}

/// POST /flows/:id/reload — hot reload Flow definition
/// Stops the running flow runtime, re-validates the flow definition, and resets status to draft
pub async fn flow_reload(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let id = Uuid::parse_str(&id).map_err(|_| ApiError::bad_request("invalid flow id"))?;

    // 1. Load flow from DB
    let flow = state
        .flow_store
        .get_flow(id)
        .await?
        .ok_or_else(|| ApiError::not_found("flow not found"))?;

    // 2. If flow is running or deployed, stop and remove the runtime
    {
        let mut runtimes = state.flow_runtimes.write().await;
        if runtimes.contains_key(&id) {
            let mut runtime = runtimes.remove(&id).unwrap();
            // Ignore errors during stop - runtime may already be stopped
            let _ = runtime.stop().await;
        }
    }

    // 3. Re-validate the flow definition
    flow.validate()
        .map_err(|e| ApiError::bad_request(format!("flow validation failed: {}", e)))?;

    // 4. Update status back to draft (user must re-deploy)
    state
        .flow_store
        .update_flow_status(id, gateway_flow::FlowStatus::Draft)
        .await?;

    tracing::info!(flow_id = %id, "flow hot-reloaded, status reset to draft");

    Ok(Json(serde_json::json!({
        "status": "draft",
        "message": "flow hot-reloaded, please re-deploy to start"
    })))
}

// ---------- Operators ----------

/// GET /flows/operators — list all registered operators with metadata
pub async fn list_operators() -> Result<Json<serde_json::Value>, ApiError> {
    let registry = gateway_flow::OperatorRegistry::new();
    let names = registry.list();
    let operators: Vec<serde_json::Value> = names
        .iter()
        .filter_map(|name| {
            let meta = registry.meta(name)?;
            Some(serde_json::json!({
                "name": meta.name,
                "name_zh": meta.name_zh,
                "name_en": meta.name_en,
                "description": meta.description,
                "description_zh": meta.description_zh,
                "description_en": meta.description_en,
                "version": meta.version,
                "kind": "operator",
            }))
        })
        .collect();
    Ok(Json(serde_json::json!({ "operators": operators })))
}

// ---------- Version History ----------

/// GET /flows/:id/versions — list version history
pub async fn flow_versions(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let id = Uuid::parse_str(&id).map_err(|_| ApiError::bad_request("invalid flow id"))?;

    let history = state
        .flow_store
        .get_flow_version_history(&id)
        .await
        .map_err(ApiError::from)?;

    Ok(Json(serde_json::json!({ "versions": history })))
}

/// GET /flows/export — export flows as JSON
/// Query param: ids=id1,id2 (optional, exports all if not provided)
pub async fn export_flows(
    axum::extract::Query(params): axum::extract::Query<ExportParams>,
    State(state): State<AppState>,
) -> Result<Json<ExportResponse>, ApiError> {
    let flows = if let Some(ids) = &params.ids {
        let id_list: Vec<Uuid> = ids
            .split(',')
            .filter_map(|s| Uuid::parse_str(s.trim()).ok())
            .collect();
        let mut result = Vec::new();
        for id in id_list {
            if let Ok(Some(flow)) = state.flow_store.get_flow(id).await {
                result.push(flow);
            }
        }
        result
    } else {
        state.flow_store.list_flows().await?
    };

    let exported_flows: Vec<ExportedFlow> = flows
        .into_iter()
        .map(|f| {
            let definition = serde_json::to_string(&f).unwrap_or_default();
            ExportedFlow {
                id: f.id.to_string(),
                name: f.name,
                description: f.description,
                definition,
                status: serde_json::to_string(&f.status).unwrap_or_default(),
                version: f.version,
            }
        })
        .collect();

    Ok(Json(ExportResponse {
        version: "1.0".to_string(),
        exported_at: chrono::Utc::now().to_rfc3339(),
        flows: exported_flows,
    }))
}

#[derive(serde::Deserialize)]
pub struct ExportParams {
    pub ids: Option<String>,
}

#[derive(serde::Serialize)]
pub struct ExportResponse {
    pub version: String,
    pub exported_at: String,
    pub flows: Vec<ExportedFlow>,
}

#[derive(serde::Serialize)]
pub struct ExportedFlow {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub definition: String,
    pub status: String,
    pub version: i64,
}

#[derive(serde::Deserialize)]
pub struct ImportRequest {
    pub flows: Vec<ImportedFlow>,
    pub force: Option<bool>,
}

#[derive(serde::Deserialize)]
pub struct ImportedFlow {
    pub name: String,
    pub description: Option<String>,
    pub definition: String,
}

/// POST /flows/import — import flows from JSON
pub async fn import_flows(
    State(state): State<AppState>,
    Json(body): Json<ImportRequest>,
) -> Result<Json<ImportResponse>, ApiError> {
    let mut imported = 0;
    let mut skipped = 0;
    let force = body.force.unwrap_or(false);

    for flow in body.flows {
        // Check if flow with same name exists
        let existing = state
            .flow_store
            .get_flow_by_name(&flow.name)
            .await
            .ok()
            .flatten();

        if existing.is_some() && !force {
            skipped += 1;
            continue;
        }

        // Parse the definition to extract nodes/edges, then create new flow
        let parsed: Flow = serde_json::from_str(&flow.definition)
            .map_err(|e| ApiError::bad_request(format!("invalid flow definition: {}", e)))?;

        // Create new flow with new UUID
        let now = chrono::Utc::now();
        let new_flow = Flow {
            id: Uuid::new_v4(),
            name: flow.name,
            description: flow.description,
            nodes: parsed.nodes,
            edges: parsed.edges,
            bindings: parsed.bindings,
            status: gateway_flow::FlowStatus::Draft,
            version: 1,
            created_at: now,
            updated_at: now,
        };

        state.flow_store.create_flow(&new_flow).await?;
        imported += 1;
    }

    Ok(Json(ImportResponse { imported, skipped }))
}

#[derive(serde::Serialize)]
pub struct ImportResponse {
    pub imported: usize,
    pub skipped: usize,
}

/// POST /flows/:id/rollback/:version — rollback to specific version
pub async fn flow_rollback(
    State(state): State<AppState>,
    Path((id, version)): Path<(String, i64)>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let id = Uuid::parse_str(&id).map_err(|_| ApiError::bad_request("invalid flow id"))?;

    let history = state
        .flow_store
        .get_flow_version_history(&id)
        .await
        .map_err(ApiError::from)?;

    let snapshot = history
        .iter()
        .find(|s| s.version == version)
        .ok_or_else(|| ApiError::not_found("version not found"))?;

    // Save current state as a new snapshot before rollback
    state.flow_store.save_flow_snapshot(&id).await.ok();

    // Restore from snapshot
    let restored = state
        .flow_store
        .restore_from_snapshot(&id, snapshot)
        .await
        .map_err(ApiError::from)?;

    Ok(Json(serde_json::json!({ "flow": restored })))
}
