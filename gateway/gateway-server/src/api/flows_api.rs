//! 流程编排 API（v2.0 T1-③；设计 docs/design/流程编排-v2.md §5）。
//!
//! 契约要点：
//! - 请求图（nodes/edges）经 `gateway_flow::FlowNode/FlowEdge` **强类型**解析——图结构
//!   不可能被静默丢弃（redesign 踩坑 P1 的对策），畸形图直接 400；
//! - publish 才做整图校验（P2 对策），draft 保存允许不完整图；
//! - 租户：创建归属发起用户的域（All 域 → default）；读取/修改按域过滤，
//!   跨域一律 404（防存在性探测，与 resolve_node 同法）。

use axum::extract::{Path as AxumPath, State};
use axum::Json;
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use super::scope::{AuthContext, TenantScope};
use super::ApiError;
use crate::flows::FlowRow;
use crate::state::AppState;

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 创建/更新请求体。图字段强类型解析（P1 对策）。
#[derive(Debug, Deserialize)]
pub struct FlowUpsertRequest {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub nodes: Vec<gateway_flow::FlowNode>,
    #[serde(default)]
    pub edges: Vec<gateway_flow::FlowEdge>,
}

fn tenant_of(ctx: &AuthContext) -> String {
    match &ctx.tenant {
        TenantScope::All => "default".to_string(),
        TenantScope::One(t) => t.clone(),
    }
}

fn row_to_json(row: &FlowRow) -> serde_json::Value {
    json!({
        "id": row.id,
        "name": row.name,
        "description": row.description,
        "status": row.status,
        "version": row.version,
        "graph": row.graph_json,
        "created_at": row.created_at,
        "updated_at": row.updated_at,
    })
}

/// GET /flows
pub async fn list_flows(
    State(state): State<AppState>,
    axum::extract::Extension(ctx): axum::extract::Extension<AuthContext>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let rows = tokio::task::spawn_blocking({
        let st = state.clone();
        move || st.flow_store.list(&ctx.tenant)
    })
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
    .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Json(
        json!({ "flows": rows.iter().map(row_to_json).collect::<Vec<_>>() }),
    ))
}

/// POST /flows —— 创建草稿
pub async fn create_flow(
    State(state): State<AppState>,
    axum::extract::Extension(ctx): axum::extract::Extension<AuthContext>,
    Json(body): Json<FlowUpsertRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    if body.name.trim().is_empty() {
        return Err(ApiError::bad_request("flow name is required"));
    }
    let tenant = tenant_of(&ctx);
    // 同租户内名称唯一
    let existing = tokio::task::spawn_blocking({
        let st = state.clone();
        let tenant = tenant.clone();
        move || st.flow_store.list(&TenantScope::One(tenant))
    })
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
    .map_err(|e| ApiError::internal(e.to_string()))?;
    if existing.iter().any(|r| r.name == body.name) {
        return Err(ApiError::bad_request(format!(
            "flow name `{}` already exists",
            body.name
        )));
    }
    let ts = now_ms();
    let row = FlowRow {
        id: Uuid::new_v4(),
        tenant_id: tenant,
        name: body.name,
        description: body.description,
        status: "draft".into(),
        version: 0,
        graph_json: json!({ "nodes": body.nodes, "edges": body.edges }),
        created_at: ts,
        updated_at: ts,
    };
    let saved = row.clone();
    tokio::task::spawn_blocking({
        let st = state.clone();
        move || st.flow_store.insert(&saved)
    })
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
    .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Json(row_to_json(&row)))
}

/// GET /flows/:id
pub async fn get_flow(
    State(state): State<AppState>,
    axum::extract::Extension(ctx): axum::extract::Extension<AuthContext>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let uuid = Uuid::parse_str(&id).map_err(|_| ApiError::not_found("flow not found"))?;
    let row = tokio::task::spawn_blocking({
        let st = state.clone();
        move || st.flow_store.get(uuid, &ctx.tenant)
    })
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
    .map_err(|e| ApiError::internal(e.to_string()))?;
    row.map(|r| Json(row_to_json(&r)))
        .ok_or_else(|| ApiError::not_found("flow not found"))
}

/// PUT /flows/:id —— 更新草稿/已发布流程的名称、描述与图（运行时重建随接线批接上）
pub async fn update_flow(
    State(state): State<AppState>,
    axum::extract::Extension(ctx): axum::extract::Extension<AuthContext>,
    AxumPath(id): AxumPath<String>,
    Json(body): Json<FlowUpsertRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    if body.name.trim().is_empty() {
        return Err(ApiError::bad_request("flow name is required"));
    }
    let uuid = Uuid::parse_str(&id).map_err(|_| ApiError::not_found("flow not found"))?;
    let existing = tokio::task::spawn_blocking({
        let st = state.clone();
        move || st.flow_store.get(uuid, &ctx.tenant)
    })
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
    .map_err(|e| ApiError::internal(e.to_string()))?
    .ok_or_else(|| ApiError::not_found("flow not found"))?;
    // 同租户内重名检查（排除自身）
    let all = tokio::task::spawn_blocking({
        let st = state.clone();
        let tenant = existing.tenant_id.clone();
        move || st.flow_store.list(&TenantScope::One(tenant))
    })
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
    .map_err(|e| ApiError::internal(e.to_string()))?;
    if all.iter().any(|r| r.name == body.name && r.id != uuid) {
        return Err(ApiError::bad_request(format!(
            "flow name `{}` already exists",
            body.name
        )));
    }
    let mut row = existing;
    row.name = body.name;
    row.description = body.description;
    row.graph_json = json!({ "nodes": body.nodes, "edges": body.edges });
    let saved = row.clone();
    let updated = tokio::task::spawn_blocking({
        let st = state.clone();
        move || st.flow_store.update(&saved)
    })
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
    .map_err(|e| ApiError::internal(e.to_string()))?;
    if !updated {
        return Err(ApiError::not_found("flow not found"));
    }
    Ok(Json(row_to_json(&row)))
}

/// DELETE /flows/:id（Admin；运行时任务停止随接线批接上）
pub async fn delete_flow(
    State(state): State<AppState>,
    axum::extract::Extension(ctx): axum::extract::Extension<AuthContext>,
    AxumPath(id): AxumPath<String>,
) -> Result<axum::http::StatusCode, ApiError> {
    let uuid = Uuid::parse_str(&id).map_err(|_| ApiError::not_found("flow not found"))?;
    let deleted = tokio::task::spawn_blocking({
        let st = state.clone();
        move || st.flow_store.delete(uuid, &ctx.tenant)
    })
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
    .map_err(|e| ApiError::internal(e.to_string()))?;
    if !deleted {
        return Err(ApiError::not_found("flow not found"));
    }
    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// POST /flows/:id/publish —— 强制整图校验（gateway_flow::validate），通过则置 published 且 version+1
pub async fn publish_flow(
    State(state): State<AppState>,
    axum::extract::Extension(ctx): axum::extract::Extension<AuthContext>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let uuid = Uuid::parse_str(&id).map_err(|_| ApiError::not_found("flow not found"))?;
    let tenant = ctx.tenant.clone();
    let row = tokio::task::spawn_blocking({
        let st = state.clone();
        let tenant = tenant.clone();
        move || st.flow_store.get(uuid, &tenant)
    })
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
    .map_err(|e| ApiError::internal(e.to_string()))?
    .ok_or_else(|| ApiError::not_found("flow not found"))?;
    let graph: gateway_flow::FlowGraph = serde_json::from_value(row.graph_json.clone())
        .map_err(|e| ApiError::bad_request(format!("stored graph is malformed: {e}")))?;
    let flow = gateway_flow::Flow {
        id: row.id,
        tenant_id: row.tenant_id.clone(),
        name: row.name.clone(),
        description: row.description.clone(),
        status: gateway_flow::FlowStatus::Draft,
        version: row.version,
        graph,
        created_at: None,
        updated_at: None,
    };
    let errs = gateway_flow::validate(&flow);
    if !errs.is_empty() {
        let msg = errs
            .iter()
            .map(|e| e.to_string())
            .collect::<Vec<_>>()
            .join("; ");
        return Err(ApiError::bad_request(format!(
            "flow validation failed: {msg}"
        )));
    }
    let updated = tokio::task::spawn_blocking({
        let st = state.clone();
        let tenant = tenant.clone();
        move || st.flow_store.set_status(uuid, &tenant, "published", true)
    })
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
    .map_err(|e| ApiError::internal(e.to_string()))?;
    updated
        .map(|r| Json(row_to_json(&r)))
        .ok_or_else(|| ApiError::not_found("flow not found"))
}

/// POST /flows/:id/unpublish —— 回草稿（告警事件与运行态随接线批定义）
pub async fn unpublish_flow(
    State(state): State<AppState>,
    axum::extract::Extension(ctx): axum::extract::Extension<AuthContext>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let uuid = Uuid::parse_str(&id).map_err(|_| ApiError::not_found("flow not found"))?;
    let updated = tokio::task::spawn_blocking({
        let st = state.clone();
        move || st.flow_store.set_status(uuid, &ctx.tenant, "draft", false)
    })
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
    .map_err(|e| ApiError::internal(e.to_string()))?;
    updated
        .map(|r| Json(row_to_json(&r)))
        .ok_or_else(|| ApiError::not_found("flow not found"))
}

/// GET /flows/operators —— 算子发现（前端编辑器不硬编码算子知识，P6 对策）
pub async fn list_flow_operators() -> Json<serde_json::Value> {
    let operators: Vec<serde_json::Value> = gateway_flow::OPERATOR_NAMES
        .iter()
        .map(|name| {
            let spec = match *name {
                "sink_north" | "sink_alarm" => gateway_flow::operators::PortSpec::sink(name),
                _ => gateway_flow::operators::PortSpec::operator(name),
            };
            json!({
                "name": name,
                "inputs": spec.inputs,
                "outputs": spec.outputs,
            })
        })
        .collect();
    Json(json!({ "operators": operators }))
}
