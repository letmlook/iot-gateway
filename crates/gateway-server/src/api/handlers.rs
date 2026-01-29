//! REST API handlers。

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use gateway_sdk::{Group, GroupSubscription, NodeId, NodeKind, PluginConfig, Tag};
use serde::Deserialize;
use uuid::Uuid;

use crate::state::AppState;

fn parse_node_id(s: &str) -> Result<NodeId, (StatusCode, &'static str)> {
    Uuid::parse_str(s).map(NodeId).map_err(|_| (StatusCode::BAD_REQUEST, "invalid node id"))
}

fn parse_group_id(s: &str) -> Result<gateway_sdk::GroupId, (StatusCode, &'static str)> {
    Uuid::parse_str(s).map(gateway_sdk::GroupId).map_err(|_| (StatusCode::BAD_REQUEST, "invalid group id"))
}

fn parse_tag_id(s: &str) -> Result<gateway_sdk::TagId, (StatusCode, &'static str)> {
    Uuid::parse_str(s).map(gateway_sdk::TagId).map_err(|_| (StatusCode::BAD_REQUEST, "invalid tag id"))
}

// ---------- Version ----------
/// 版本信息。对标 Neuron /api/version。
pub async fn version() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "build_date": option_env!("BUILD_DATE").unwrap_or(""),
        "revision": option_env!("GIT_REV").unwrap_or(""),
    }))
}

// ---------- Health ----------
/// 健康检查：status、节点数、运行中节点数、南/北向插件数
pub async fn health(State(state): State<AppState>) -> Json<serde_json::Value> {
    let nodes = state.manager.nodes_list();
    let nodes_count = nodes.len();
    let nodes_running = nodes
        .iter()
        .filter(|n| n.state == gateway_sdk::NodeState::Running)
        .count();
    let (south_count, north_count) = (
        state.manager.south_plugins().len(),
        state.manager.north_plugins().len(),
    );
    Json(serde_json::json!({
        "status": "ok",
        "nodes_count": nodes_count,
        "nodes_running": nodes_running,
        "plugins_south": south_count,
        "plugins_north": north_count,
    }))
}

/// 导出当前快照为 JSON（用于备份或迁移）
pub async fn export_snapshot(State(state): State<AppState>) -> Json<gateway_core::Snapshot> {
    let snap = state.manager.build_snapshot().await;
    Json(snap)
}

/// Prometheus 格式指标（节点数、运行数、插件数等）
pub async fn metrics(State(state): State<AppState>) -> (axum::http::StatusCode, String) {
    use axum::http::StatusCode;
    let nodes = state.manager.nodes_list();
    let nodes_total = nodes.len() as u64;
    let nodes_running = nodes
        .iter()
        .filter(|n| n.state == gateway_sdk::NodeState::Running)
        .count() as u64;
    let (plugins_south, plugins_north) = (
        state.manager.south_plugins().len() as u64,
        state.manager.north_plugins().len() as u64,
    );
    let body = format!(
        "# HELP gateway_nodes_total Total number of nodes.\n\
         # TYPE gateway_nodes_total gauge\n\
         gateway_nodes_total {}\n\
         # HELP gateway_nodes_running Number of running nodes.\n\
         # TYPE gateway_nodes_running gauge\n\
         gateway_nodes_running {}\n\
         # HELP gateway_plugins_south South plugin count.\n\
         # TYPE gateway_plugins_south gauge\n\
         gateway_plugins_south {}\n\
         # HELP gateway_plugins_north North plugin count.\n\
         # TYPE gateway_plugins_north gauge\n\
         gateway_plugins_north {}\n",
        nodes_total, nodes_running, plugins_south, plugins_north
    );
    (StatusCode::OK, body)
}

// ---------- Plugins ----------
/// 插件条目：name, description, version
pub async fn list_south_plugins(State(state): State<AppState>) -> Json<Vec<(String, String, String)>> {
    Json(state.manager.south_plugins())
}

pub async fn list_north_plugins(State(state): State<AppState>) -> Json<Vec<(String, String, String)>> {
    Json(state.manager.north_plugins())
}

pub async fn north_plugin_config_schema(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<gateway_sdk::ConfigSchema>, (StatusCode, String)> {
    let p = state
        .manager
        .north_plugin(&name)
        .ok_or((StatusCode::NOT_FOUND, "plugin not found".to_string()))?;
    let s = p.config_schema().ok_or((StatusCode::NOT_FOUND, "no config_schema".to_string()))?;
    Ok(Json(s))
}

pub async fn south_plugin_config_schema(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<gateway_sdk::ConfigSchema>, (StatusCode, String)> {
    let p = state
        .manager
        .south_plugin(&name)
        .ok_or((StatusCode::NOT_FOUND, "plugin not found".to_string()))?;
    let s = p.config_schema().ok_or((StatusCode::NOT_FOUND, "no config_schema".to_string()))?;
    Ok(Json(s))
}

pub async fn south_plugin_tag_schema(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<gateway_sdk::TagSchema>, (StatusCode, String)> {
    let p = state
        .manager
        .south_plugin(&name)
        .ok_or((StatusCode::NOT_FOUND, "plugin not found".to_string()))?;
    let s = p.tag_schema().ok_or((StatusCode::NOT_FOUND, "no tag_schema".to_string()))?;
    Ok(Json(s))
}

// ---------- Nodes ----------
#[derive(Deserialize)]
pub struct CreateNodeReq {
    pub name: String,
    pub kind: NodeKind,
    pub plugin_name: String,
    #[serde(default)]
    pub config: PluginConfig,
}

pub async fn list_nodes(State(state): State<AppState>) -> Json<Vec<gateway_core::Node>> {
    Json(state.manager.nodes_list())
}

pub async fn create_node(
    State(state): State<AppState>,
    Json(req): Json<CreateNodeReq>,
) -> Result<Json<gateway_core::Node>, (StatusCode, String)> {
    let node = state
        .manager
        .node_create(req.name, req.kind, req.plugin_name, req.config)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    state.persist().await;
    Ok(Json(node))
}

pub async fn get_node(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<gateway_core::Node>, (StatusCode, String)> {
    let nid = parse_node_id(&id).map_err(|(s, m)| (s, m.to_string()))?;
    let node = state.manager.node_get(nid).ok_or((StatusCode::NOT_FOUND, "node not found".to_string()))?;
    Ok(Json(node))
}

#[derive(Deserialize)]
pub struct UpdateNodeReq {
    pub name: Option<String>,
}

pub async fn update_node(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateNodeReq>,
) -> Result<StatusCode, (StatusCode, String)> {
    let nid = parse_node_id(&id).map_err(|(s, m)| (s, m.to_string()))?;
    state.manager.node_update(nid, req.name).map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    state.persist().await;
    Ok(StatusCode::NO_CONTENT)
}

/// 获取节点插件配置（仅 config）。对标 Neuron GET setting。
pub async fn get_node_setting(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let nid = parse_node_id(&id).map_err(|(s, m)| (s, m.to_string()))?;
    let node = state.manager.node_get(nid).ok_or((StatusCode::NOT_FOUND, "node not found".to_string()))?;
    Ok(Json(serde_json::json!({ "config": node.config.config })))
}

pub async fn delete_node(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    let nid = parse_node_id(&id).map_err(|(s, m)| (s, m.to_string()))?;
    state.manager.node_remove(nid).await;
    state.persist().await;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn start_node(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    let nid = parse_node_id(&id).map_err(|(s, m)| (s, m.to_string()))?;
    state.manager.node_start(nid).await.map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    state.persist().await;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn stop_node(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    let nid = parse_node_id(&id).map_err(|(s, m)| (s, m.to_string()))?;
    state.manager.node_stop(nid).await.map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    state.persist().await;
    Ok(StatusCode::NO_CONTENT)
}

// ---------- Groups ----------
pub async fn list_groups(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Vec<Group>>, (StatusCode, String)> {
    let nid = parse_node_id(&id).map_err(|(s, m)| (s, m.to_string()))?;
    Ok(Json(state.manager.groups_by_node(nid)))
}

pub async fn get_group(
    State(state): State<AppState>,
    Path((id, gid)): Path<(String, String)>,
) -> Result<Json<Group>, (StatusCode, String)> {
    let nid = parse_node_id(&id).map_err(|(s, m)| (s, m.to_string()))?;
    let g = parse_group_id(&gid).map_err(|(s, m)| (s, m.to_string()))?;
    let group = state.manager.group_get(nid, g).ok_or((StatusCode::NOT_FOUND, "group not found".to_string()))?;
    Ok(Json(group))
}

#[derive(Deserialize)]
pub struct AddGroupReq {
    pub name: String,
    pub interval_ms: u64,
    pub description: Option<String>,
}

pub async fn add_group(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<AddGroupReq>,
) -> Result<Json<Group>, (StatusCode, String)> {
    let nid = parse_node_id(&id).map_err(|(s, m)| (s, m.to_string()))?;
    let mut g = Group::new(req.name, req.interval_ms);
    g.description = req.description;
    state.manager.group_add(nid, g.clone()).map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    state.persist().await;
    Ok(Json(g))
}

#[derive(Deserialize)]
pub struct UpdateGroupReq {
    pub name: Option<String>,
    pub interval_ms: Option<u64>,
    pub description: Option<Option<String>>,
}

pub async fn update_group(
    State(state): State<AppState>,
    Path((id, gid)): Path<(String, String)>,
    Json(req): Json<UpdateGroupReq>,
) -> Result<StatusCode, (StatusCode, String)> {
    let nid = parse_node_id(&id).map_err(|(s, m)| (s, m.to_string()))?;
    let g = parse_group_id(&gid).map_err(|(s, m)| (s, m.to_string()))?;
    state
        .manager
        .group_update(nid, g, req.name, req.interval_ms, req.description)
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    state.persist().await;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn remove_group(
    State(state): State<AppState>,
    Path((id, gid)): Path<(String, String)>,
) -> Result<StatusCode, (StatusCode, String)> {
    let nid = parse_node_id(&id).map_err(|(s, m)| (s, m.to_string()))?;
    let g = parse_group_id(&gid).map_err(|(s, m)| (s, m.to_string()))?;
    state.manager.group_remove(nid, g).ok_or((StatusCode::NOT_FOUND, "group not found".to_string()))?;
    state.persist().await;
    Ok(StatusCode::NO_CONTENT)
}

// ---------- Tags ----------
pub async fn list_tags(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Vec<Tag>>, (StatusCode, String)> {
    let nid = parse_node_id(&id).map_err(|(s, m)| (s, m.to_string()))?;
    let tags = state.manager.groups_by_node(nid)
        .into_iter()
        .flat_map(|g| state.manager.tags_by_group(nid, g.id))
        .collect::<Vec<_>>();
    Ok(Json(tags))
}

#[derive(Deserialize)]
pub struct AddTagReq {
    pub name: String,
    pub address: String,
    pub group_id: gateway_sdk::GroupId,
    pub attr: Option<gateway_sdk::TagAttr>,
    pub data_type: Option<String>,
    pub description: Option<String>,
}

pub async fn add_tag(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<AddTagReq>,
) -> Result<Json<Tag>, (StatusCode, String)> {
    let nid = parse_node_id(&id).map_err(|(s, m)| (s, m.to_string()))?;
    let mut t = Tag::new(req.name, req.address, req.group_id);
    t.attr = req.attr.unwrap_or(gateway_sdk::TagAttr::Read);
    t.data_type = req.data_type;
    t.description = req.description;
    state
        .manager
        .tag_add_validated(nid, t.clone())
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    state.persist().await;
    Ok(Json(t))
}

#[derive(Deserialize)]
pub struct BatchAddTagsReq {
    pub tags: Vec<AddTagReq>,
}

pub async fn batch_add_tags(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<BatchAddTagsReq>,
) -> Result<Json<Vec<Tag>>, (StatusCode, String)> {
    let nid = parse_node_id(&id).map_err(|(s, m)| (s, m.to_string()))?;
    let mut created = Vec::with_capacity(req.tags.len());
    for r in req.tags {
        let mut t = Tag::new(r.name, r.address, r.group_id);
        t.attr = r.attr.unwrap_or(gateway_sdk::TagAttr::Read);
        t.data_type = r.data_type;
        t.description = r.description;
        state
            .manager
            .tag_add_validated(nid, t.clone())
            .await
            .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
        created.push(t);
    }
    state.persist().await;
    Ok(Json(created))
}

pub async fn get_tag(
    State(state): State<AppState>,
    Path((id, tid)): Path<(String, String)>,
) -> Result<Json<Tag>, (StatusCode, String)> {
    let nid = parse_node_id(&id).map_err(|(s, m)| (s, m.to_string()))?;
    let tag_id = parse_tag_id(&tid).map_err(|(s, m)| (s, m.to_string()))?;
    let tag = state
        .manager
        .tag_get(nid, tag_id)
        .ok_or((StatusCode::NOT_FOUND, "tag not found".to_string()))?;
    Ok(Json(tag))
}

#[derive(Deserialize)]
pub struct UpdateTagReq {
    pub name: Option<String>,
    pub address: Option<String>,
    pub attr: Option<gateway_sdk::TagAttr>,
    pub data_type: Option<Option<String>>,
    pub description: Option<Option<String>>,
}

pub async fn update_tag(
    State(state): State<AppState>,
    Path((id, tid)): Path<(String, String)>,
    Json(req): Json<UpdateTagReq>,
) -> Result<StatusCode, (StatusCode, String)> {
    let nid = parse_node_id(&id).map_err(|(s, m)| (s, m.to_string()))?;
    let tag_id = parse_tag_id(&tid).map_err(|(s, m)| (s, m.to_string()))?;
    state
        .manager
        .tag_update_validated(
            nid,
            tag_id,
            req.name,
            req.address,
            req.attr,
            req.data_type,
            req.description,
        )
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    state.persist().await;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn remove_tag(
    State(state): State<AppState>,
    Path((_id, tid)): Path<(String, String)>,
) -> Result<StatusCode, (StatusCode, String)> {
    let tag_id = parse_tag_id(&tid).map_err(|(s, m)| (s, m.to_string()))?;
    state.manager.tag_remove(tag_id).ok_or((StatusCode::NOT_FOUND, "tag not found".to_string()))?;
    state.persist().await;
    Ok(StatusCode::NO_CONTENT)
}

// ---------- Subscriptions (North) ----------
pub async fn get_subscriptions(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Vec<GroupSubscription>>, (StatusCode, String)> {
    let nid = parse_node_id(&id).map_err(|(s, m)| (s, m.to_string()))?;
    let subs = state.manager.get_north_subscriptions(nid).await;
    Ok(Json(subs))
}

#[derive(Deserialize)]
pub struct SetSubscriptionsReq {
    pub subscriptions: Vec<GroupSubscription>,
}

pub async fn set_subscriptions(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<SetSubscriptionsReq>,
) -> Result<StatusCode, (StatusCode, String)> {
    let nid = parse_node_id(&id).map_err(|(s, m)| (s, m.to_string()))?;
    state.manager.set_north_subscriptions(nid, req.subscriptions).await;
    state.persist().await;
    Ok(StatusCode::NO_CONTENT)
}

// ---------- Node setting / write_tags ----------
#[derive(Deserialize)]
pub struct NodeSettingReq {
    pub config: PluginConfig,
}

pub async fn node_setting(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<NodeSettingReq>,
) -> Result<StatusCode, (StatusCode, String)> {
    let nid = parse_node_id(&id).map_err(|(s, m)| (s, m.to_string()))?;
    state
        .manager
        .node_setting(nid, req.config)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    state.persist().await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct ReadTagsReq {
    pub tag_ids: Vec<gateway_sdk::TagId>,
}

pub async fn read_tags(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<ReadTagsReq>,
) -> Result<Json<Vec<(gateway_sdk::TagId, gateway_sdk::types::DataValue)>>, (StatusCode, String)> {
    let nid = parse_node_id(&id).map_err(|(s, m)| (s, m.to_string()))?;
    let values = state
        .manager
        .read_tags(nid, &req.tag_ids)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    Ok(Json(values))
}

#[derive(Deserialize)]
pub struct WriteTagsReq {
    pub values: Vec<(gateway_sdk::TagId, gateway_sdk::types::DataValue)>,
}

pub async fn write_tags(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<WriteTagsReq>,
) -> Result<StatusCode, (StatusCode, String)> {
    let nid = parse_node_id(&id).map_err(|(s, m)| (s, m.to_string()))?;
    state
        .manager
        .write_tags(nid, &req.values)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    Ok(StatusCode::NO_CONTENT)
}
