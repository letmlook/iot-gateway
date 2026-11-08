//! 告警事件 API（v2.0 T1-④；设计 docs/design/流程编排-v2.md §4.2）。
//!
//! 角色：Viewer 读 / Operator ack / Admin resolve（`required_role`）。
//! WS alarm 帧为机会式增强，随前端告警中心批接上；REST 为可靠来源（redesign 规格结论）。

use axum::extract::{Path as AxumPath, Query, State};
use axum::Json;
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use super::scope::AuthContext;
use super::ApiError;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct ListAlarmsQuery {
    /// active | acknowledged | resolved；缺省不过滤
    #[serde(default)]
    pub state: Option<String>,
    /// critical | high | medium | low | info；缺省不过滤
    #[serde(default)]
    pub level: Option<String>,
    #[serde(default)]
    pub limit: Option<i64>,
}

/// GET /alarms
pub async fn list_alarms(
    State(state): State<AppState>,
    axum::extract::Extension(ctx): axum::extract::Extension<AuthContext>,
    Query(q): Query<ListAlarmsQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let limit = q.limit.unwrap_or(200);
    let rows = tokio::task::spawn_blocking({
        let st = state.clone();
        let tenant = ctx.tenant.clone();
        move || {
            st.alarm_store
                .list(&tenant, q.state.as_deref(), q.level.as_deref(), limit)
        }
    })
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
    .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Json(json!({ "alarms": rows })))
}

/// POST /alarms/:id/ack —— active → acknowledged（记录操作人）
pub async fn ack_alarm(
    State(state): State<AppState>,
    axum::extract::Extension(ctx): axum::extract::Extension<AuthContext>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let uuid = Uuid::parse_str(&id).map_err(|_| ApiError::not_found("alarm not found"))?;
    let by = ctx.username.clone().unwrap_or_else(|| "unknown".into());
    let acked = tokio::task::spawn_blocking({
        let st = state.clone();
        let tenant = ctx.tenant.clone();
        move || st.alarm_store.ack(uuid, &tenant, &by)
    })
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
    .map_err(|e| ApiError::internal(e.to_string()))?;
    if !acked {
        // 不存在、跨租户、或已非 active —— 统一 400 语义（ack 不可重复）
        return Err(ApiError::bad_request(
            "alarm not found or not in active state",
        ));
    }
    alarm_json(&state, uuid, &ctx.tenant).await
}

/// POST /alarms/:id/resolve —— active|acknowledged → resolved（终态，Admin）
pub async fn resolve_alarm(
    State(state): State<AppState>,
    axum::extract::Extension(ctx): axum::extract::Extension<AuthContext>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let uuid = Uuid::parse_str(&id).map_err(|_| ApiError::not_found("alarm not found"))?;
    let resolved = tokio::task::spawn_blocking({
        let st = state.clone();
        let tenant = ctx.tenant.clone();
        move || st.alarm_store.resolve(uuid, &tenant)
    })
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
    .map_err(|e| ApiError::internal(e.to_string()))?;
    if !resolved {
        return Err(ApiError::bad_request("alarm not found or already resolved"));
    }
    alarm_json(&state, uuid, &ctx.tenant).await
}

async fn alarm_json(
    state: &AppState,
    id: Uuid,
    tenant: &super::scope::TenantScope,
) -> Result<Json<serde_json::Value>, ApiError> {
    let rows = tokio::task::spawn_blocking({
        let st = state.clone();
        let tenant = tenant.clone();
        move || st.alarm_store.list(&tenant, None, None, 1000)
    })
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
    .map_err(|e| ApiError::internal(e.to_string()))?;
    rows.into_iter()
        .find(|r| r.id == id)
        .map(|r| Json(serde_json::to_value(r).unwrap_or(json!({}))))
        .ok_or_else(|| ApiError::not_found("alarm not found"))
}
