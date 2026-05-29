//! Audit REST API handlers.

use axum::{extract::State, Json};

use crate::api::ApiError;
use crate::state::AppState;

pub async fn list_events(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let events = state.audit_store.list().await.map_err(ApiError::internal)?;
    Ok(Json(serde_json::json!({ "items": events })))
}
