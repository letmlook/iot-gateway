//! REST API handlers。

use axum::body::Bytes;
use axum::extract::{Multipart, Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use gateway_sdk::{Group, GroupSubscription, NodeId, NodeKind, NodeState, PluginConfig, Tag};
use serde::Deserialize;
use uuid::Uuid;

use crate::api::ApiError;
use crate::backup;
use crate::logging;
use crate::state::AppState;

fn parse_node_id(s: &str) -> Result<NodeId, ApiError> {
    Uuid::parse_str(s)
        .map(NodeId)
        .map_err(|_| ApiError::bad_request("invalid node id"))
}

fn parse_group_id(s: &str) -> Result<gateway_sdk::GroupId, ApiError> {
    Uuid::parse_str(s)
        .map(gateway_sdk::GroupId)
        .map_err(|_| ApiError::bad_request("invalid group id"))
}

fn parse_tag_id(s: &str) -> Result<gateway_sdk::TagId, ApiError> {
    Uuid::parse_str(s)
        .map(gateway_sdk::TagId)
        .map_err(|_| ApiError::bad_request("invalid tag id"))
}

/// 北向节点无组/标签：仅南向节点允许组、标签、读 Tag、写 Tag。
fn ensure_node_south(state: &AppState, nid: NodeId) -> Result<(), ApiError> {
    let node = state
        .manager
        .node_get(nid)
        .ok_or_else(|| ApiError::not_found("node not found"))?;
    if node.kind() == NodeKind::North {
        return Err(ApiError::not_found("north node has no groups or tags"));
    }
    Ok(())
}

/// 南向节点无订阅：仅北向节点允许订阅管理。
fn ensure_node_north(state: &AppState, nid: NodeId) -> Result<(), ApiError> {
    let node = state
        .manager
        .node_get(nid)
        .ok_or_else(|| ApiError::not_found("node not found"))?;
    if node.kind() == NodeKind::South {
        return Err(ApiError::not_found("south node has no subscriptions"));
    }
    Ok(())
}

// ---------- Auth ----------
#[derive(serde::Deserialize)]
pub struct LoginRequest {
    pub username: Option<String>,
    pub password: Option<String>,
}

/// 登录：若 disable_auth 返回 token: null；若有用户表则校验用户名密码并返回 token+user；否则兼容旧逻辑（admin + config.token）。
pub async fn login(
    State(state): State<AppState>,
    Json(body): Json<LoginRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    tracing::info!("login request: username={:?}", body.username);
    if state.config.disable_auth {
        tracing::info!("login: auth disabled, returning null token");
        return Ok(Json(serde_json::json!({ "token": null, "user": null })));
    }
    let username = body.username.as_deref().unwrap_or("").trim().to_string();
    let password = body.password.as_deref().unwrap_or("").trim().to_string();

    let has_users = match state.user_store.has_any_user().await {
        Ok(v) => {
            tracing::info!("login: has_any_user() = {}", v);
            v
        }
        Err(e) => {
            tracing::error!("login: has_any_user() error: {}", e);
            return Err(ApiError::internal(e));
        }
    };

    if has_users {
        match state.user_store.login(&username, &password).await {
            Ok((token, user)) => {
                tracing::info!("login success: user={}", user.username);
                return Ok(Json(serde_json::json!({
                    "token": token,
                    "user": {
                        "id": user.id,
                        "username": user.username,
                        "role": user.role.as_str(),
                        "created_at": user.created_at,
                        "updated_at": user.updated_at,
                    }
                })));
            }
            Err(e) => {
                tracing::warn!("login failed via user_store: {}", e);
                return Err(ApiError::unauthorized());
            }
        }
    }

    tracing::info!("login: no users in db, trying legacy token auth");
    let token = state.config.token.as_ref().ok_or_else(|| {
        tracing::warn!("login failed: no users and no config.token set");
        ApiError::unauthorized()
    })?;
    if username == "admin" && password == token.as_str() {
        tracing::info!("login success via legacy token: admin");
        Ok(Json(serde_json::json!({
            "token": token,
            "user": { "id": "admin", "username": "admin", "role": "admin", "created_at": "", "updated_at": "" }
        })))
    } else {
        tracing::warn!("login failed: legacy token mismatch");
        Err(ApiError::unauthorized())
    }
}

// ---------- Users ----------
#[derive(serde::Deserialize)]
pub struct CreateUserRequest {
    pub username: Option<String>,
    pub password: Option<String>,
    pub role: Option<String>,
}

#[derive(serde::Deserialize)]
pub struct UpdateUserRequest {
    pub username: Option<String>,
    pub role: Option<String>,
}

#[derive(serde::Deserialize)]
pub struct ChangePasswordRequest {
    pub password: Option<String>,
}

fn parse_user_id(s: &str) -> Result<String, ApiError> {
    Uuid::parse_str(s)
        .map(|_| s.to_string())
        .or(Ok(s.to_string()))
}

pub async fn list_users(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let list = state
        .user_store
        .list()
        .await
        .map_err(|e| ApiError::internal(e))?;
    let arr: Vec<serde_json::Value> = list
        .into_iter()
        .map(|u| {
            serde_json::json!({
                "id": u.id,
                "username": u.username,
                "role": u.role.as_str(),
                "created_at": u.created_at,
                "updated_at": u.updated_at,
            })
        })
        .collect();
    Ok(Json(serde_json::json!({ "users": arr })))
}

pub async fn create_user(
    State(state): State<AppState>,
    Json(body): Json<CreateUserRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let username = body.username.as_deref().unwrap_or("").trim();
    let password = body.password.as_deref().unwrap_or("");
    let role = body.role.as_deref().unwrap_or("operator");
    let role_enum = match role {
        "admin" => crate::users::UserRole::Admin,
        "viewer" => crate::users::UserRole::Viewer,
        _ => crate::users::UserRole::Operator,
    };
    let user = state
        .user_store
        .create(username, password, role_enum)
        .await
        .map_err(|e| ApiError::bad_request(e))?;
    Ok(Json(serde_json::json!({
        "id": user.id,
        "username": user.username,
        "role": user.role.as_str(),
        "created_at": user.created_at,
        "updated_at": user.updated_at,
    })))
}

pub async fn get_user(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let id = parse_user_id(&id)?;
    let user = state
        .user_store
        .get(&id)
        .await
        .map_err(|e| ApiError::internal(e))?
        .ok_or_else(|| ApiError::not_found("user not found"))?;
    Ok(Json(serde_json::json!({
        "id": user.id,
        "username": user.username,
        "role": user.role.as_str(),
        "created_at": user.created_at,
        "updated_at": user.updated_at,
    })))
}

pub async fn update_user(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<UpdateUserRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let id = parse_user_id(&id)?;
    let username = body
        .username
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty());
    let role = body.role.as_deref().map(|r| match r {
        "admin" => crate::users::UserRole::Admin,
        "viewer" => crate::users::UserRole::Viewer,
        _ => crate::users::UserRole::Operator,
    });
    let user = state
        .user_store
        .update_simple(&id, username, role)
        .await
        .map_err(|e| ApiError::bad_request(e))?;
    Ok(Json(serde_json::json!({
        "id": user.id,
        "username": user.username,
        "role": user.role.as_str(),
        "created_at": user.created_at,
        "updated_at": user.updated_at,
    })))
}

pub async fn delete_user(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let id = parse_user_id(&id)?;
    state
        .user_store
        .delete(&id)
        .await
        .map_err(|e| ApiError::internal(e))?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn change_password(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<ChangePasswordRequest>,
) -> Result<StatusCode, ApiError> {
    let id = parse_user_id(&id)?;
    let password = body.password.as_deref().unwrap_or("");
    state
        .user_store
        .set_password(&id, password)
        .await
        .map_err(|e| ApiError::bad_request(e))?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------- Version ----------
/// 版本信息。
pub async fn version() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "build_date": option_env!("BUILD_DATE").unwrap_or(""),
        "revision": option_env!("GIT_REV").unwrap_or(""),
    }))
}

// ---------- License（完全离线，无网络请求）----------
/// 返回当前设备机器码，供客户发送给管理员生成授权文件。
pub async fn license_machine_id() -> Result<Json<serde_json::Value>, ApiError> {
    let mid = crate::license::machine_id().map_err(|e| ApiError::internal(e))?;
    Ok(Json(serde_json::json!({ "machineId": mid })))
}

/// 授权状态：是否已加载有效授权、已授权功能列表、已授权插件列表、点位数限制。
pub async fn license_status(State(state): State<AppState>) -> Json<serde_json::Value> {
    let has_license = state.feature_manager.has_license();
    let raw_features = state.feature_manager.granted_features();

    // 如果 features 中包含 all_plugins，则展开为所有已加载的插件名称
    let features: Vec<String> = if raw_features.iter().any(|f| f == "all_plugins") {
        // 获取所有已加载的插件名称
        let mut all_plugins: Vec<String> = state
            .manager
            .south_plugins()
            .into_iter()
            .map(|p| p.name)
            .collect();
        all_plugins.extend(state.manager.north_plugins().into_iter().map(|p| p.name));
        // 去重
        all_plugins.sort();
        all_plugins.dedup();
        all_plugins
    } else {
        // 将 plugin:xxx 格式转换为纯插件名
        raw_features
            .iter()
            .filter_map(|f| {
                if let Some(name) = f.strip_prefix("plugin:") {
                    Some(name.to_string())
                } else if f != "all_plugins" {
                    Some(f.clone())
                } else {
                    None
                }
            })
            .collect()
    };

    let licensed_plugins = state.feature_manager.licensed_plugins();
    let free_plugins: Vec<&str> = crate::license::FeatureManager::free_plugins().to_vec();
    let max_tags = state.feature_manager.max_tags();
    let used_tags = state.manager.store.tags_total_count();
    Json(serde_json::json!({
        "hasLicense": has_license,
        "features": features,
        "licensedPlugins": licensed_plugins,
        "freePlugins": free_plugins,
        "maxTags": max_tags,
        "usedTags": used_tags,
    }))
}

/// 演示：收费功能 pro_tool。仅当授权中包含 "pro_tool" 时可访问，否则 403。
pub async fn license_pro_tool(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    if state.feature_manager.can_access("pro_tool") {
        Ok(Json(serde_json::json!({
            "allowed": true,
            "message": "Pro tool access granted.",
        })))
    } else {
        Err(ApiError::forbidden(
            "此功能需要授权，请在授权文件中包含 pro_tool",
        ))
    }
}

/// 上传授权文件：接收 license.dat 文件，保存到数据目录，验证后立即生效。
pub async fn upload_license(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<Json<serde_json::Value>, ApiError> {
    let mut file_data: Option<Bytes> = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::bad_request(e.to_string()))?
    {
        let name = field.name().unwrap_or("").to_string();
        if name == "file" {
            let bytes = field
                .bytes()
                .await
                .map_err(|e| ApiError::bad_request(e.to_string()))?;
            if !bytes.is_empty() {
                file_data = Some(bytes);
            }
        }
    }
    let data = file_data.ok_or_else(|| ApiError::bad_request("missing license file"))?;

    // 保存到数据目录
    let license_path = state.config.license_path();
    if let Some(parent) = license_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| ApiError::internal(format!("create dir failed: {}", e)))?;
    }
    std::fs::write(&license_path, &data)
        .map_err(|e| ApiError::internal(format!("write license failed: {}", e)))?;

    // 验证授权文件
    match crate::license::load_and_verify_license(&license_path) {
        Ok(payload) => {
            let features = payload.features.clone();
            state.feature_manager.update_license(payload);
            tracing::info!("license uploaded and activated, features: {:?}", features);
            Ok(Json(serde_json::json!({
                "ok": true,
                "message": "License activated",
                "features": features,
            })))
        }
        Err(e) => {
            // 验证失败，删除上传的文件
            let _ = std::fs::remove_file(&license_path);
            tracing::warn!("license upload failed: {}", e);
            Err(ApiError::bad_request(format!(
                "license validation failed: {}",
                e
            )))
        }
    }
}

/// 上传节点配置用文件（如证书）：保存到 data/uploads，返回绝对路径供配置存储。
pub async fn upload_config_file(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<Json<serde_json::Value>, ApiError> {
    let mut file_data: Option<Bytes> = None;
    let mut original_name: Option<String> = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::bad_request(e.to_string()))?
    {
        let name = field.name().unwrap_or("").to_string();
        if name == "file" {
            original_name = field.file_name().map(|s| s.to_string());
            let bytes = field
                .bytes()
                .await
                .map_err(|e| ApiError::bad_request(e.to_string()))?;
            if !bytes.is_empty() {
                file_data = Some(bytes);
            }
        }
    }
    let data = file_data.ok_or_else(|| ApiError::bad_request("missing file"))?;

    let uploads_dir = state.config.data_dir.join("uploads");
    std::fs::create_dir_all(&uploads_dir)
        .map_err(|e| ApiError::internal(format!("create uploads dir failed: {}", e)))?;

    let ext = original_name
        .as_deref()
        .and_then(|n| std::path::Path::new(n).extension())
        .and_then(|e| e.to_str())
        .unwrap_or("bin");
    let save_name = format!("{}.{}", uuid::Uuid::new_v4(), ext);
    let save_path = uploads_dir.join(&save_name);
    std::fs::write(&save_path, &data)
        .map_err(|e| ApiError::internal(format!("write file failed: {}", e)))?;

    let path_str = save_path
        .canonicalize()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| save_path.to_string_lossy().into_owned());

    Ok(Json(serde_json::json!({ "path": path_str })))
}

/// 重置授权：删除授权文件并清除内存中的授权状态，此操作不可逆。
pub async fn reset_license(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let license_path = state.config.license_path();

    // 删除授权文件（如果存在）
    if license_path.exists() {
        std::fs::remove_file(&license_path)
            .map_err(|e| ApiError::internal(format!("delete license file failed: {}", e)))?;
    }

    // 清除内存中的授权状态
    state.feature_manager.clear_license();

    tracing::info!("license reset: file deleted and memory cleared");
    Ok(Json(serde_json::json!({
        "ok": true,
        "message": "License reset successfully",
    })))
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

/// Dashboard overview stats: nodes, flows, tags, system resource usage.
pub async fn dashboard_stats(State(state): State<AppState>) -> Json<serde_json::Value> {
    let nodes = state.manager.nodes_list();
    let south_nodes = nodes
        .iter()
        .filter(|n| n.kind() == gateway_sdk::NodeKind::South)
        .count();
    let north_nodes = nodes
        .iter()
        .filter(|n| n.kind() == gateway_sdk::NodeKind::North)
        .count();
    let running_nodes = nodes
        .iter()
        .filter(|n| n.state == gateway_sdk::NodeState::Running)
        .count();
    let stopped_nodes = nodes.len() - running_nodes;

    let total_flows = {
        let path = state.flow_store.db_path.clone();
        tokio::task::spawn_blocking(move || {
            let conn = rusqlite::Connection::open(&path).ok();
            conn.and_then(|c| {
                c.query_row("SELECT COUNT(*) FROM flows", [], |row| row.get::<_, i64>(0))
                    .ok()
            })
            .map(|c| c as usize)
            .unwrap_or(0)
        })
        .await
        .unwrap_or(0)
    };
    let running_flows = 0;
    let stopped_flows = 0;
    let draft_flows = total_flows;

    let (south_count, north_count) = (
        state.manager.south_plugins().len(),
        state.manager.north_plugins().len(),
    );

    let df = state.manager.data_flow_snapshot();
    let uptime_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
        .saturating_sub(state.started_at.timestamp() as u64);

    let has_license = state.feature_manager.has_license();
    let raw_features = state.feature_manager.granted_features();
    let features: Vec<String> = if raw_features.iter().any(|f| f == "all_plugins") {
        let mut all_plugins: Vec<String> = state
            .manager
            .south_plugins()
            .into_iter()
            .map(|p| p.name)
            .collect();
        all_plugins.extend(state.manager.north_plugins().into_iter().map(|p| p.name));
        all_plugins.sort();
        all_plugins.dedup();
        all_plugins
    } else {
        raw_features
            .iter()
            .filter_map(|f| {
                if let Some(name) = f.strip_prefix("plugin:") {
                    Some(name.to_string())
                } else if f != "all_plugins" {
                    Some(f.clone())
                } else {
                    None
                }
            })
            .collect()
    };

    Json(serde_json::json!({
        "nodes": {
            "total": nodes.len(),
            "south": south_nodes,
            "north": north_nodes,
            "running": running_nodes,
            "stopped": stopped_nodes,
        },
        "flows": {
            "total": total_flows,
            "running": running_flows,
            "stopped": stopped_flows,
            "draft": draft_flows,
        },
        "plugins": {
            "south": south_count,
            "north": north_count,
        },
        "data_flow": {
            "south_published": df.south_published,
            "bus_no_subscribers": df.bus_no_subscribers,
            "north_received": df.north_received,
            "north_filtered": df.north_filtered,
            "north_forwarded": df.north_forwarded,
            "north_on_group_data_ok": df.north_on_group_data_ok,
            "north_on_group_data_err": df.north_on_group_data_err,
        },
        "license": {
            "has_license": has_license,
            "features": features,
        },
        "system": {
            "uptime_secs": uptime_secs,
        }
    }))
}

/// 默认备份/恢复密码（未填写时使用，保证文件仍为加密不可直接查看）
const DEFAULT_BACKUP_SECRET: &str = "gateway-backup";

#[derive(Deserialize, Default)]
pub struct BackupReq {
    pub password: Option<String>,
}

/// 备份：返回压缩加密的二进制，不展示内容；POST body 可选 { "password": "xxx" }
pub async fn backup(
    State(state): State<AppState>,
    Json(body): Json<BackupReq>,
) -> Result<impl IntoResponse, ApiError> {
    let secret = body
        .password
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_BACKUP_SECRET.to_string());
    let snap = state.manager.build_snapshot().await;
    let data = backup::encrypt_backup(&snap, &secret).map_err(ApiError::internal)?;
    let filename = format!(
        "gateway-backup-{}.bin",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    );
    let mut res = (StatusCode::OK, data).into_response();
    res.headers_mut().insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static("application/octet-stream"),
    );
    res.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        header::HeaderValue::try_from(format!("attachment; filename=\"{}\"", filename))
            .unwrap_or(header::HeaderValue::from_static("attachment")),
    );
    Ok(res)
}

/// 恢复：仅接受加密备份文件 + 可选密码，不展示内容
pub async fn restore(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<Json<serde_json::Value>, ApiError> {
    let mut file_data: Option<Bytes> = None;
    let mut password: Option<String> = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::bad_request(e.to_string()))?
    {
        let name = field.name().unwrap_or("").to_string();
        if name == "file" {
            let bytes = field
                .bytes()
                .await
                .map_err(|e| ApiError::bad_request(e.to_string()))?;
            if !bytes.is_empty() {
                file_data = Some(bytes);
            }
        } else if name == "password" {
            let s = field
                .text()
                .await
                .map_err(|e| ApiError::bad_request(e.to_string()))?;
            password = Some(s);
        }
    }
    let data = file_data.ok_or_else(|| ApiError::bad_request("missing backup file"))?;
    let secret = password
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_BACKUP_SECRET.to_string());
    let snap = backup::decrypt_backup(&data, &secret).map_err(ApiError::bad_request)?;
    if snap.version != gateway_core::SNAPSHOT_VERSION {
        return Err(ApiError::bad_request(format!(
            "unsupported snapshot version: {}, expected {}",
            snap.version,
            gateway_core::SNAPSHOT_VERSION
        )));
    }
    state.manager.apply_snapshot(&snap).await;
    state.sync_node_log_names();
    state.persist().await;
    Ok(Json(
        serde_json::json!({ "ok": true, "message": "restored" }),
    ))
}

/// Prometheus 格式指标（节点数、运行数、插件数、数据流链路等）
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
    let df = state.manager.data_flow_snapshot();
    let alarm_events = state.alarm_store.list().await.unwrap_or_default();
    let alarm_active_total = alarm_events
        .iter()
        .filter(|event| event.status == crate::alarm::AlarmStatus::Active)
        .count() as u64;
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
         gateway_plugins_north {}\n\
         # HELP gateway_data_flow_south_published GroupData published by south to bus.\n\
         # TYPE gateway_data_flow_south_published counter\n\
         gateway_data_flow_south_published {}\n\
         # HELP gateway_data_flow_bus_no_subscribers Times publish had no north subscribers.\n\
         # TYPE gateway_data_flow_bus_no_subscribers counter\n\
         gateway_data_flow_bus_no_subscribers {}\n\
         # HELP gateway_data_flow_north_received Messages received by north from bus.\n\
         # TYPE gateway_data_flow_north_received counter\n\
         gateway_data_flow_north_received {}\n\
         # HELP gateway_data_flow_north_filtered Messages filtered out by subscription table.\n\
         # TYPE gateway_data_flow_north_filtered counter\n\
         gateway_data_flow_north_filtered {}\n\
         # HELP gateway_data_flow_north_forwarded Messages passed to on_group_data.\n\
         # TYPE gateway_data_flow_north_forwarded counter\n\
         gateway_data_flow_north_forwarded {}\n\
         # HELP gateway_data_flow_north_on_group_data_ok on_group_data returned Ok.\n\
         # TYPE gateway_data_flow_north_on_group_data_ok counter\n\
         gateway_data_flow_north_on_group_data_ok {}\n\
         # HELP gateway_data_flow_north_on_group_data_err on_group_data returned Err.\n\
         # TYPE gateway_data_flow_north_on_group_data_err counter\n\
         gateway_data_flow_north_on_group_data_err {}\n\
         # HELP gateway_data_flow_north_lagged Total messages skipped due to lag.\n\
         # TYPE gateway_data_flow_north_lagged counter\n\
         gateway_data_flow_north_lagged {}\n\
         # HELP gateway_south_poll_total South poll publish total, mapped from current data-flow counters.\n\
         # TYPE gateway_south_poll_total counter\n\
         gateway_south_poll_total {}\n\
         # HELP gateway_south_poll_errors_total South poll errors.\n\
         # TYPE gateway_south_poll_errors_total counter\n\
         gateway_south_poll_errors_total 0\n\
         # HELP gateway_south_poll_duration_ms South poll duration placeholder.\n\
         # TYPE gateway_south_poll_duration_ms gauge\n\
         gateway_south_poll_duration_ms 0\n\
         # HELP gateway_north_publish_total North publish total, mapped from on_group_data successes.\n\
         # TYPE gateway_north_publish_total counter\n\
         gateway_north_publish_total {}\n\
         # HELP gateway_north_publish_errors_total North publish errors, mapped from on_group_data failures.\n\
         # TYPE gateway_north_publish_errors_total counter\n\
         gateway_north_publish_errors_total {}\n\
         # HELP gateway_flow_exec_total Flow execution total, mapped from south publishes through the processing seam.\n\
         # TYPE gateway_flow_exec_total counter\n\
         gateway_flow_exec_total {}\n\
         # HELP gateway_flow_exec_errors_total Flow execution errors.\n\
         # TYPE gateway_flow_exec_errors_total counter\n\
         gateway_flow_exec_errors_total 0\n\
         # HELP gateway_flow_exec_duration_ms Flow execution duration placeholder.\n\
         # TYPE gateway_flow_exec_duration_ms gauge\n\
         gateway_flow_exec_duration_ms 0\n\
         # HELP gateway_bus_lag_total Bus lag total.\n\
         # TYPE gateway_bus_lag_total counter\n\
         gateway_bus_lag_total {}\n\
         # HELP gateway_alarm_active_total Active alarm count.\n\
         # TYPE gateway_alarm_active_total gauge\n\
         gateway_alarm_active_total{{severity=\"all\"}} {}\n",
        nodes_total,
        nodes_running,
        plugins_south,
        plugins_north,
        df.south_published,
        df.bus_no_subscribers,
        df.north_received,
        df.north_filtered,
        df.north_forwarded,
        df.north_on_group_data_ok,
        df.north_on_group_data_err,
        df.north_lagged,
        df.south_published,
        df.north_on_group_data_ok,
        df.north_on_group_data_err,
        df.south_published,
        df.north_lagged,
        alarm_active_total,
    );
    (StatusCode::OK, body)
}

/// 数据流链路监控：返回各环节计数与点位级统计，用于排查「数据未正常发出」问题
pub async fn data_flow(State(state): State<AppState>) -> Json<serde_json::Value> {
    let m = state.manager.data_flow_snapshot();
    let published_per_tag: Vec<serde_json::Value> = m
        .published_per_tag
        .iter()
        .map(|s| {
            let tag = state.manager.store.tag_get(s.tag_id);
            let south_node = state.manager.node_get(s.south_node_id);
            let group = state.manager.store.group_get(s.south_node_id, s.group_id);
            serde_json::json!({
                "south_node_id": s.south_node_id,
                "south_node_name": south_node.as_ref().map(|n| &n.config.name),
                "group_id": s.group_id,
                "group_name": group.as_ref().map(|g| &g.name),
                "tag_id": s.tag_id,
                "tag_name": tag.as_ref().map(|t| &t.name),
                "count": s.count,
            })
        })
        .collect();
    let forwarded_per_tag: Vec<serde_json::Value> = m
        .forwarded_per_tag
        .iter()
        .map(|s| {
            let tag = state.manager.store.tag_get(s.tag_id);
            let north_node = state.manager.node_get(s.north_node_id);
            let south_node = state.manager.node_get(s.south_node_id);
            let group = state.manager.store.group_get(s.south_node_id, s.group_id);
            serde_json::json!({
                "north_node_id": s.north_node_id,
                "north_node_name": north_node.as_ref().map(|n| &n.config.name),
                "south_node_id": s.south_node_id,
                "south_node_name": south_node.as_ref().map(|n| &n.config.name),
                "group_id": s.group_id,
                "group_name": group.as_ref().map(|g| &g.name),
                "tag_id": s.tag_id,
                "tag_name": tag.as_ref().map(|t| &t.name),
                "count": s.count,
            })
        })
        .collect();
    Json(serde_json::json!({
        "metrics": {
            "south_published": m.south_published,
            "bus_no_subscribers": m.bus_no_subscribers,
            "north_received": m.north_received,
            "north_filtered": m.north_filtered,
            "north_forwarded": m.north_forwarded,
            "north_on_group_data_ok": m.north_on_group_data_ok,
            "north_on_group_data_err": m.north_on_group_data_err,
            "north_lagged": m.north_lagged,
        },
        "per_tag": {
            "published": published_per_tag,
            "forwarded": forwarded_per_tag,
        },
        "flow": "南向 poll_group -> bus.publish -> 北向 recv -> 订阅过滤 -> on_group_data",
        "troubleshoot": {
            "bus_no_subscribers > 0": "南向有数据但无北向订阅者（北向未启动或未订阅该南向组）",
            "north_filtered 高": "北向收到数据但被订阅表过滤（检查北向订阅的 south_node_id/group_id）",
            "north_on_group_data_err > 0": "北向插件 on_group_data 执行失败（查北向节点日志）",
            "north_lagged > 0": "北向消费慢，跳过了部分消息（考虑增加采集间隔或优化北向）",
        },
    }))
}

// ---------- Plugins ----------
/// 插件列表（含 name, name_zh, name_en, description, description_zh, description_en, version, licensed, is_free）
pub async fn list_south_plugins(State(state): State<AppState>) -> Json<Vec<serde_json::Value>> {
    let plugins = state.manager.south_plugins();
    let result: Vec<serde_json::Value> = plugins
        .into_iter()
        .map(|p| {
            let licensed = state.feature_manager.can_use_plugin(&p.name);
            let is_free = crate::license::FeatureManager::is_free_plugin(&p.name);
            serde_json::json!({
                "name": p.name,
                "description": p.description,
                "version": p.version,
                "name_zh": p.name_zh,
                "name_en": p.name_en,
                "description_zh": p.description_zh,
                "description_en": p.description_en,
                "licensed": licensed,
                "is_free": is_free,
                "status": p.status,
                "protocol_stack": p.protocol_stack,
                "capabilities": p.capabilities,
                "known_limits": p.known_limits,
            })
        })
        .collect();
    Json(result)
}

pub async fn list_north_plugins(State(state): State<AppState>) -> Json<Vec<serde_json::Value>> {
    let plugins = state.manager.north_plugins();
    let result: Vec<serde_json::Value> = plugins
        .into_iter()
        .map(|p| {
            let licensed = state.feature_manager.can_use_plugin(&p.name);
            let is_free = crate::license::FeatureManager::is_free_plugin(&p.name);
            serde_json::json!({
                "name": p.name,
                "description": p.description,
                "version": p.version,
                "name_zh": p.name_zh,
                "name_en": p.name_en,
                "description_zh": p.description_zh,
                "description_en": p.description_en,
                "licensed": licensed,
                "is_free": is_free,
                "status": p.status,
                "protocol_stack": p.protocol_stack,
                "capabilities": p.capabilities,
                "known_limits": p.known_limits,
            })
        })
        .collect();
    Json(result)
}

/// GET /plugins — combined list of all south + north plugins with metadata
pub async fn plugins_all(State(state): State<AppState>) -> Json<serde_json::Value> {
    let south_plugins: Vec<serde_json::Value> = state
        .manager
        .south_plugins()
        .into_iter()
        .map(|p| {
            let licensed = state.feature_manager.can_use_plugin(&p.name);
            let is_free = crate::license::FeatureManager::is_free_plugin(&p.name);
            serde_json::json!({
                "name": p.name,
                "description": p.description,
                "version": p.version,
                "name_zh": p.name_zh,
                "name_en": p.name_en,
                "description_zh": p.description_zh,
                "description_en": p.description_en,
                "kind": "south",
                "licensed": licensed,
                "is_free": is_free,
                "status": p.status,
                "protocol_stack": p.protocol_stack,
                "capabilities": p.capabilities,
                "known_limits": p.known_limits,
            })
        })
        .collect();

    let north_plugins: Vec<serde_json::Value> = state
        .manager
        .north_plugins()
        .into_iter()
        .map(|p| {
            let licensed = state.feature_manager.can_use_plugin(&p.name);
            let is_free = crate::license::FeatureManager::is_free_plugin(&p.name);
            serde_json::json!({
                "name": p.name,
                "description": p.description,
                "version": p.version,
                "name_zh": p.name_zh,
                "name_en": p.name_en,
                "description_zh": p.description_zh,
                "description_en": p.description_en,
                "kind": "north",
                "licensed": licensed,
                "is_free": is_free,
                "status": p.status,
                "protocol_stack": p.protocol_stack,
                "capabilities": p.capabilities,
                "known_limits": p.known_limits,
            })
        })
        .collect();

    Json(serde_json::json!({
        "south": south_plugins,
        "north": north_plugins,
    }))
}

pub async fn north_plugin_config_schema(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<gateway_sdk::ConfigSchema>, ApiError> {
    let p = state
        .manager
        .north_plugin(&name)
        .ok_or_else(|| ApiError::not_found("plugin not found"))?;
    let s = p
        .config_schema()
        .ok_or_else(|| ApiError::not_found("no config_schema"))?;
    Ok(Json(s))
}

pub async fn south_plugin_config_schema(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<gateway_sdk::ConfigSchema>, ApiError> {
    let p = state
        .manager
        .south_plugin(&name)
        .ok_or_else(|| ApiError::not_found("plugin not found"))?;
    let s = p
        .config_schema()
        .ok_or_else(|| ApiError::not_found("no config_schema"))?;
    Ok(Json(s))
}

pub async fn south_plugin_tag_schema(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<gateway_sdk::TagSchema>, ApiError> {
    let p = state
        .manager
        .south_plugin(&name)
        .ok_or_else(|| ApiError::not_found("plugin not found"))?;
    let s = p
        .tag_schema()
        .ok_or_else(|| ApiError::not_found("no tag_schema"))?;
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

pub async fn list_nodes(State(state): State<AppState>) -> Json<Vec<serde_json::Value>> {
    let nodes = state.manager.nodes_list();
    let mut result = Vec::with_capacity(nodes.len());
    for node in nodes {
        let mut j = serde_json::to_value(&node).unwrap_or(serde_json::json!({}));
        if let Some(obj) = j.as_object_mut() {
            if node.kind() == NodeKind::North {
                if let Some(plugin) = state.manager.north_plugin(&node.config.plugin_name) {
                    if let Some(conn) = plugin.connection_status(node.id()).await {
                        obj.insert("connection_status".to_string(), conn);
                    }
                }
            } else if node.kind() == NodeKind::South && node.state == NodeState::Running {
                if let Some(conn) = state.manager.south_connection_status(node.id()).await {
                    obj.insert(
                        "connection_status".to_string(),
                        serde_json::to_value(&conn).unwrap_or(serde_json::json!({ "connected": conn.connected, "last_error": conn.last_error })),
                    );
                }
            }
        }
        result.push(j);
    }
    Json(result)
}

pub async fn create_node(
    State(state): State<AppState>,
    Json(req): Json<CreateNodeReq>,
) -> Result<Json<gateway_core::Node>, ApiError> {
    // 检查插件是否已授权
    if !state.feature_manager.can_use_plugin(&req.plugin_name) {
        return Err(ApiError::forbidden(format!(
            "插件 {} 需要授权才能使用，请在授权文件中包含 plugin:{} 或 all_plugins",
            req.plugin_name, req.plugin_name
        )));
    }
    let node = state
        .manager
        .node_create(req.name, req.kind, req.plugin_name, req.config)
        .await
        .map_err(ApiError::bad_request)?;
    state.sync_node_log_names();
    state.persist().await;
    Ok(Json(node))
}

pub async fn get_node(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let nid = parse_node_id(&id)?;
    let node = state
        .manager
        .node_get(nid)
        .ok_or_else(|| ApiError::not_found("node not found"))?;
    let mut j = serde_json::to_value(&node).map_err(|e| ApiError::internal(e.to_string()))?;
    if let Some(obj) = j.as_object_mut() {
        if node.kind() == NodeKind::North {
            if let Some(plugin) = state.manager.north_plugin(&node.config.plugin_name) {
                if let Some(conn) = plugin.connection_status(nid).await {
                    obj.insert("connection_status".to_string(), conn);
                }
            }
        } else if node.kind() == NodeKind::South && node.state == NodeState::Running {
            if let Some(conn) = state.manager.south_connection_status(nid).await {
                obj.insert(
                    "connection_status".to_string(),
                    serde_json::to_value(&conn).unwrap_or(serde_json::json!({ "connected": conn.connected, "last_error": conn.last_error })),
                );
            }
        }
    }
    Ok(Json(j))
}

#[derive(Deserialize)]
pub struct UpdateNodeReq {
    pub name: Option<String>,
}

pub async fn update_node(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateNodeReq>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let nid = parse_node_id(&id)?;
    state
        .manager
        .node_update(nid, req.name)
        .map_err(ApiError::bad_request)?;
    state.sync_node_log_names();
    state.persist().await;

    let node = state
        .manager
        .node_get(nid)
        .ok_or_else(|| ApiError::not_found("node not found"))?;
    let mut j = serde_json::to_value(&node).map_err(|e| ApiError::internal(e.to_string()))?;
    if let Some(obj) = j.as_object_mut() {
        if node.kind() == NodeKind::North {
            if let Some(plugin) = state.manager.north_plugin(&node.config.plugin_name) {
                if let Some(conn) = plugin.connection_status(nid).await {
                    obj.insert("connected".into(), serde_json::json!(conn));
                }
            }
        }
    }
    Ok(Json(j))
}

/// 获取节点插件配置（仅 config）。
pub async fn get_node_setting(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let nid = parse_node_id(&id)?;
    let node = state
        .manager
        .node_get(nid)
        .ok_or_else(|| ApiError::not_found("node not found"))?;
    Ok(Json(serde_json::json!({ "config": node.config.config })))
}

pub async fn delete_node(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let nid = parse_node_id(&id)?;
    state.manager.node_remove(nid).await;
    if let Some(ref dir) = state.config.log_dir_nodes {
        logging::remove_node_log_file(dir.as_ref(), &id, state.node_log_names.as_ref());
    }
    state.sync_node_log_names();
    state.persist().await;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn start_node(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let nid = parse_node_id(&id)?;
    state
        .manager
        .node_start(nid)
        .await
        .map_err(ApiError::bad_request)?;
    state.persist().await;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn stop_node(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let nid = parse_node_id(&id)?;
    state
        .manager
        .node_stop(nid)
        .await
        .map_err(ApiError::bad_request)?;
    state.persist().await;
    Ok(StatusCode::NO_CONTENT)
}

/// 获取节点连接状态。北向由插件上报（如 MQTT）；南向由最近一次采集结果推断（成功为已连接，失败为异常）。
pub async fn get_node_connection_status(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let nid = parse_node_id(&id)?;
    let node = state
        .manager
        .node_get(nid)
        .ok_or_else(|| ApiError::not_found("node not found"))?;
    if node.kind() == NodeKind::South {
        let conn = state.manager.south_connection_status(nid).await;
        let v = conn
            .map(|c| {
                serde_json::to_value(&c).unwrap_or(
                    serde_json::json!({ "connected": c.connected, "last_error": c.last_error }),
                )
            })
            .unwrap_or_else(|| serde_json::json!({ "connected": false, "last_error": null }));
        return Ok(Json(v));
    }
    let plugin = state
        .manager
        .north_plugin(&node.config.plugin_name)
        .ok_or_else(|| ApiError::not_found("plugin not found"))?;
    match plugin.connection_status(nid).await {
        Some(v) => Ok(Json(v)),
        None => Ok(Json(
            serde_json::json!({ "connected": null, "last_error": null }),
        )),
    }
}

// ---------- Groups ----------
pub async fn list_groups(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Vec<Group>>, ApiError> {
    let nid = parse_node_id(&id)?;
    ensure_node_south(&state, nid)?;
    Ok(Json(state.manager.groups_by_node(nid)))
}

pub async fn get_group(
    State(state): State<AppState>,
    Path((id, gid)): Path<(String, String)>,
) -> Result<Json<Group>, ApiError> {
    let nid = parse_node_id(&id)?;
    ensure_node_south(&state, nid)?;
    let g = parse_group_id(&gid)?;
    let group = state
        .manager
        .group_get(nid, g)
        .ok_or_else(|| ApiError::not_found("group not found"))?;
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
) -> Result<Json<Group>, ApiError> {
    let nid = parse_node_id(&id)?;
    ensure_node_south(&state, nid)?;
    let mut g = Group::new(req.name, req.interval_ms);
    g.description = req.description;
    state
        .manager
        .group_add(nid, g.clone())
        .map_err(ApiError::bad_request)?;
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
) -> Result<StatusCode, ApiError> {
    let nid = parse_node_id(&id)?;
    ensure_node_south(&state, nid)?;
    let g = parse_group_id(&gid)?;
    state
        .manager
        .group_update(nid, g, req.name, req.interval_ms, req.description)
        .map_err(ApiError::bad_request)?;
    state.persist().await;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn remove_group(
    State(state): State<AppState>,
    Path((id, gid)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let nid = parse_node_id(&id)?;
    ensure_node_south(&state, nid)?;
    let g = parse_group_id(&gid)?;
    state
        .manager
        .group_remove(nid, g)
        .await
        .ok_or_else(|| ApiError::not_found("group not found"))?;
    state.persist().await;
    Ok(StatusCode::NO_CONTENT)
}

// ---------- Tags ----------
pub async fn list_tags(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Vec<Tag>>, ApiError> {
    let nid = parse_node_id(&id)?;
    ensure_node_south(&state, nid)?;
    let tags = state
        .manager
        .groups_by_node(nid)
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
) -> Result<Json<Tag>, ApiError> {
    // 检查点位数限制
    let current_count = state.manager.store.tags_total_count();
    state
        .feature_manager
        .check_tag_limit(current_count, 1)
        .map_err(ApiError::forbidden)?;

    let nid = parse_node_id(&id)?;
    ensure_node_south(&state, nid)?;
    let mut t = Tag::new(req.name, req.address, req.group_id);
    t.attr = req.attr.unwrap_or(gateway_sdk::TagAttr::Read);
    t.data_type = req.data_type;
    t.description = req.description;
    state
        .manager
        .tag_add_validated(nid, t.clone())
        .await
        .map_err(ApiError::bad_request)?;
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
) -> Result<Json<Vec<Tag>>, ApiError> {
    // 检查点位数限制
    let current_count = state.manager.store.tags_total_count();
    let add_count = req.tags.len() as u64;
    state
        .feature_manager
        .check_tag_limit(current_count, add_count)
        .map_err(ApiError::forbidden)?;

    let nid = parse_node_id(&id)?;
    ensure_node_south(&state, nid)?;
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
            .map_err(ApiError::bad_request)?;
        created.push(t);
    }
    state.persist().await;
    Ok(Json(created))
}

pub async fn get_tag(
    State(state): State<AppState>,
    Path((id, tid)): Path<(String, String)>,
) -> Result<Json<Tag>, ApiError> {
    let nid = parse_node_id(&id)?;
    ensure_node_south(&state, nid)?;
    let tag_id = parse_tag_id(&tid)?;
    let tag = state
        .manager
        .tag_get(nid, tag_id)
        .ok_or_else(|| ApiError::not_found("tag not found"))?;
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
) -> Result<StatusCode, ApiError> {
    let nid = parse_node_id(&id)?;
    ensure_node_south(&state, nid)?;
    let tag_id = parse_tag_id(&tid)?;
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
        .map_err(ApiError::bad_request)?;
    state.persist().await;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn remove_tag(
    State(state): State<AppState>,
    Path((id, tid)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let nid = parse_node_id(&id)?;
    ensure_node_south(&state, nid)?;
    let tag_id = parse_tag_id(&tid)?;
    state
        .manager
        .tag_remove(tag_id)
        .ok_or_else(|| ApiError::not_found("tag not found"))?;
    state.persist().await;
    Ok(StatusCode::NO_CONTENT)
}

// ---------- Subscriptions (North) ----------
pub async fn get_subscriptions(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Vec<GroupSubscription>>, ApiError> {
    let nid = parse_node_id(&id)?;
    ensure_node_north(&state, nid)?;
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
) -> Result<StatusCode, ApiError> {
    let nid = parse_node_id(&id)?;
    ensure_node_north(&state, nid)?;
    state
        .manager
        .set_north_subscriptions(nid, req.subscriptions)
        .await;
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
) -> Result<StatusCode, ApiError> {
    let nid = parse_node_id(&id)?;
    state
        .manager
        .node_setting(nid, req.config)
        .await
        .map_err(ApiError::bad_request)?;
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
) -> Result<Json<Vec<(gateway_sdk::TagId, gateway_sdk::types::DataValue)>>, ApiError> {
    let nid = parse_node_id(&id)?;
    ensure_node_south(&state, nid)?;
    let values = state
        .manager
        .read_tags(nid, &req.tag_ids)
        .await
        .map_err(ApiError::bad_request)?;
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
) -> Result<StatusCode, ApiError> {
    let nid = parse_node_id(&id)?;
    ensure_node_south(&state, nid)?;
    state
        .manager
        .write_tags(nid, &req.values)
        .await
        .map_err(ApiError::bad_request)?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------- System / logs / alarm compatibility ----------
fn uptime_seconds(state: &AppState) -> u64 {
    chrono::Utc::now()
        .signed_duration_since(state.started_at)
        .num_seconds()
        .max(0) as u64
}

pub async fn hardware(State(state): State<AppState>) -> Json<serde_json::Value> {
    let hardware_id = crate::license::machine_id().unwrap_or_else(|_| "unknown".to_string());
    Json(serde_json::json!({
        "hardware_id": hardware_id,
        "hostname": std::env::var("HOSTNAME")
            .or_else(|_| std::env::var("COMPUTERNAME"))
            .unwrap_or_else(|_| "unknown".to_string()),
        "arch": std::env::consts::ARCH,
        "os": std::env::consts::OS,
        "os_version": std::env::consts::OS,
        "kernel_version": "unknown",
        "cpu_count": std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1),
        "cpu_usage": 0.0,
        "memory_used": 0,
        "memory_total": 0,
        "uptime_seconds": uptime_seconds(&state),
    }))
}

#[derive(Deserialize)]
pub struct LogDownloadQuery {
    #[serde(rename = "type")]
    pub kind: Option<String>,
}

pub async fn logs_download(
    State(state): State<AppState>,
    Query(query): Query<LogDownloadQuery>,
) -> Result<Response, ApiError> {
    let kind = query.kind.as_deref().unwrap_or("all");
    let (filename, body) = match kind {
        "system" => {
            let path = state.config.log_file.clone();
            let body = match path.as_ref() {
                Some(path) if path.exists() => tokio::fs::read(path)
                    .await
                    .map_err(|e| ApiError::internal(format!("read log file failed: {}", e)))?,
                Some(path) => {
                    format!("system log file is not available: {}\n", path.display()).into_bytes()
                }
                None => b"system log file is not configured\n".to_vec(),
            };
            ("gateway-system.log", body)
        }
        "driver" => {
            let dir = state.config.log_dir_nodes.clone();
            let body = match dir.as_ref() {
                Some(dir) if dir.exists() => {
                    let mut lines = Vec::new();
                    let mut entries = tokio::fs::read_dir(dir).await.map_err(|e| {
                        ApiError::internal(format!("read node log dir failed: {}", e))
                    })?;
                    while let Some(entry) = entries.next_entry().await.map_err(|e| {
                        ApiError::internal(format!("read node log entry failed: {}", e))
                    })? {
                        lines.push(entry.file_name().to_string_lossy().into_owned());
                    }
                    lines.sort();
                    if lines.is_empty() {
                        format!("driver log directory is empty: {}\n", dir.display()).into_bytes()
                    } else {
                        format!(
                            "driver log files in {}:\n{}\n",
                            dir.display(),
                            lines.join("\n")
                        )
                        .into_bytes()
                    }
                }
                Some(dir) => format!("driver log directory is not available: {}\n", dir.display())
                    .into_bytes(),
                None => b"driver log directory is not configured\n".to_vec(),
            };
            ("gateway-driver-logs.txt", body)
        }
        _ => {
            let body = serde_json::to_vec_pretty(&serde_json::json!({
                "system_log": state.config.log_file.as_ref().map(|p| p.display().to_string()),
                "driver_log_dir": state.config.log_dir_nodes.as_ref().map(|p| p.display().to_string()),
                "message": "Use type=system or type=driver for a specific log download.",
            }))
            .map_err(|e| ApiError::internal(e.to_string()))?;
            ("gateway-logs.json", body)
        }
    };

    let mut res = (StatusCode::OK, body).into_response();
    res.headers_mut().insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static("application/octet-stream"),
    );
    res.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        header::HeaderValue::try_from(format!("attachment; filename=\"{}\"", filename))
            .unwrap_or(header::HeaderValue::from_static("attachment")),
    );
    Ok(res)
}

#[derive(Deserialize, Default)]
pub struct LogConfigRequest {
    pub level: Option<String>,
    pub filter: Option<String>,
    pub upload_enabled: Option<bool>,
}

fn log_config_json(state: &AppState, req: Option<LogConfigRequest>) -> serde_json::Value {
    serde_json::json!({
        "level": req.as_ref().and_then(|r| r.level.as_deref()).unwrap_or(&state.config.log_level),
        "filter": req.as_ref().and_then(|r| r.filter.as_deref()).unwrap_or(&state.config.log_filter),
        "upload_enabled": req.as_ref().and_then(|r| r.upload_enabled).unwrap_or(false),
        "dynamic_reload": false,
        "restart_required": true,
    })
}

pub async fn get_log_config(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(log_config_json(&state, None))
}

pub async fn set_log_config(
    State(state): State<AppState>,
    Json(req): Json<LogConfigRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let level = req
        .level
        .as_deref()
        .unwrap_or(&state.config.log_level)
        .trim();
    if level.is_empty() {
        return Err(ApiError::bad_request("log level cannot be empty"));
    }
    Ok(Json(log_config_json(&state, Some(req))))
}

#[derive(Deserialize, Default)]
pub struct SystemConfigRequest {
    pub port: Option<u16>,
    pub http_port: Option<u16>,
    pub data_dir: Option<String>,
    pub static_dir: Option<String>,
    pub plugins_dir: Option<String>,
    pub log_level: Option<String>,
    pub log_filter: Option<String>,
    pub disable_auth: Option<bool>,
    pub auth_enabled: Option<bool>,
    pub backup_enabled: Option<bool>,
    pub backup_retention: Option<u64>,
}

fn system_config_json(state: &AppState, req: Option<SystemConfigRequest>) -> serde_json::Value {
    let port = req
        .as_ref()
        .and_then(|r| r.port.or(r.http_port))
        .unwrap_or(state.config.port);
    let disable_auth = req
        .as_ref()
        .and_then(|r| r.disable_auth)
        .unwrap_or(state.config.disable_auth);
    let auth_enabled = req
        .as_ref()
        .and_then(|r| r.auth_enabled)
        .unwrap_or(!disable_auth);
    serde_json::json!({
        "port": port,
        "http_port": port,
        "data_dir": req.as_ref().and_then(|r| r.data_dir.as_deref()).map(str::to_string)
            .unwrap_or_else(|| state.config.data_dir.display().to_string()),
        "static_dir": req.as_ref().and_then(|r| r.static_dir.as_deref()).map(str::to_string)
            .unwrap_or_else(|| state.config.static_dir.display().to_string()),
        "plugins_dir": req.as_ref().and_then(|r| r.plugins_dir.as_deref()).map(str::to_string)
            .unwrap_or_else(|| state.config.plugins_dir.display().to_string()),
        "auth_enabled": auth_enabled,
        "disable_auth": !auth_enabled,
        "backup_enabled": req.as_ref().and_then(|r| r.backup_enabled).unwrap_or(true),
        "backup_retention": req.as_ref().and_then(|r| r.backup_retention).unwrap_or(5),
        "log_level": req.as_ref().and_then(|r| r.log_level.as_deref()).unwrap_or(&state.config.log_level),
        "log_filter": req.as_ref().and_then(|r| r.log_filter.as_deref()).unwrap_or(&state.config.log_filter),
        "log_file": state.config.log_file.as_ref().map(|p| p.display().to_string()),
        "log_dir_nodes": state.config.log_dir_nodes.as_ref().map(|p| p.display().to_string()),
        "restart_required": true,
    })
}

pub async fn get_system_config(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(system_config_json(&state, None))
}

pub async fn set_system_config(
    State(state): State<AppState>,
    Json(req): Json<SystemConfigRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    if req.port == Some(0) || req.http_port == Some(0) {
        return Err(ApiError::bad_request("port must be greater than 0"));
    }
    Ok(Json(system_config_json(&state, Some(req))))
}
