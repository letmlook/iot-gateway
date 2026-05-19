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
use gateway_flow::Flow;
use gateway_sdk::OperatorMetrics;

impl From<FlowStoreError> for ApiError {
    fn from(e: FlowStoreError) -> Self {
        ApiError::internal(e.to_string())
    }
}

pub async fn list_flows(State(state): State<AppState>) -> Result<Json<serde_json::Value>, ApiError> {
    let flows = state.flow_store.list_flows().await?;
    Ok(Json(serde_json::json!({ "flows": flows })))
}

pub async fn create_flow(
    State(state): State<AppState>,
    Json(flow): Json<Flow>,
) -> Result<Json<serde_json::Value>, ApiError> {
    // Validate flow
    flow.validate().map_err(|e| ApiError::bad_request(e.to_string()))?;

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
