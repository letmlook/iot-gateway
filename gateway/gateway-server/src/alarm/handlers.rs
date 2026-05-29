//! Alarm REST API handlers.

use axum::{
    extract::{Path, State},
    Json,
};
use uuid::Uuid;

use crate::api::ApiError;
use crate::state::AppState;
use crate::websocket::AlarmLiveEvent;

pub async fn list_events(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let events = state.alarm_store.list().await.map_err(ApiError::internal)?;
    Ok(Json(serde_json::json!({
        "items": events,
        "events": events,
    })))
}

pub async fn ack_event(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let id = Uuid::parse_str(&id).map_err(|_| ApiError::bad_request("invalid alarm event id"))?;
    let event = state.alarm_store.acknowledge(id).await.map_err(|e| {
        if e.contains("not found") {
            ApiError::not_found(e)
        } else {
            ApiError::internal(e)
        }
    })?;
    let _ = state
        .audit_store
        .record(
            "system",
            "alarm.ack",
            "alarm_event",
            Some(event.id.to_string()),
            None,
            serde_json::json!({ "status": event.status }),
        )
        .await;
    state
        .ws_hub
        .broadcast_alarm(AlarmLiveEvent::Acknowledged {
            event: event.clone(),
        })
        .await;
    Ok(Json(serde_json::json!({ "event": event })))
}

pub async fn resolve_event(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let id = Uuid::parse_str(&id).map_err(|_| ApiError::bad_request("invalid alarm event id"))?;
    let event = state.alarm_store.resolve(id).await.map_err(|e| {
        if e.contains("not found") {
            ApiError::not_found(e)
        } else {
            ApiError::internal(e)
        }
    })?;
    let _ = state
        .audit_store
        .record(
            "system",
            "alarm.resolve",
            "alarm_event",
            Some(event.id.to_string()),
            None,
            serde_json::json!({ "status": event.status }),
        )
        .await;
    state
        .ws_hub
        .broadcast_alarm(AlarmLiveEvent::Resolved {
            event: event.clone(),
        })
        .await;
    Ok(Json(serde_json::json!({ "event": event })))
}
