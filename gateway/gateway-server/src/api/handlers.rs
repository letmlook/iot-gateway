//! REST API handlers。

use axum::body::Bytes;
use axum::extract::{Extension, Multipart, Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use gateway_sdk::{Group, GroupSubscription, NodeId, NodeKind, NodeState, PluginConfig, Tag};
use serde::Deserialize;
use uuid::Uuid;

use crate::api::ApiError;
use crate::backup;
use crate::logging;
use crate::state::AppState;

pub(crate) fn parse_node_id(s: &str) -> Result<NodeId, ApiError> {
    Uuid::parse_str(s)
        .map(NodeId)
        .map_err(|_| ApiError::bad_request("invalid node id"))
}

pub(crate) fn parse_group_id(s: &str) -> Result<gateway_sdk::GroupId, ApiError> {
    Uuid::parse_str(s)
        .map(gateway_sdk::GroupId)
        .map_err(|_| ApiError::bad_request("invalid group id"))
}

pub(crate) fn parse_tag_id(s: &str) -> Result<gateway_sdk::TagId, ApiError> {
    Uuid::parse_str(s)
        .map(gateway_sdk::TagId)
        .map_err(|_| ApiError::bad_request("invalid tag id"))
}

/// 北向节点无组/标签：仅南向节点允许组、标签、读 Tag、写 Tag。
pub(crate) fn ensure_node_south(state: &AppState, nid: NodeId) -> Result<(), ApiError> {
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
pub(crate) fn ensure_node_north(state: &AppState, nid: NodeId) -> Result<(), ApiError> {
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
        return Ok(Json(
            serde_json::json!({ "token": null, "expires_at": null, "user": null }),
        ));
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
            Ok((token, expires_at, user)) => {
                tracing::info!("login success: user={}", user.username);
                return Ok(Json(serde_json::json!({
                    "token": token,
                    "expires_at": expires_at,
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
        // 遗留登录签发的是静态 token，永不过期（expires_at = null）
        Ok(Json(serde_json::json!({
            "token": token,
            "expires_at": null,
            "user": { "id": "admin", "username": "admin", "role": "admin", "created_at": "", "updated_at": "" }
        })))
    } else {
        tracing::warn!("login failed: legacy token mismatch");
        Err(ApiError::unauthorized())
    }
}

/// 登出：使当前 Bearer token 立即失效（清空 DB token 并移出内存缓存）
pub async fn logout(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(|s| s.to_string());
    if let Some(t) = token {
        if !t.is_empty() {
            let _ = state.user_store.logout(&t).await;
        }
    }
    Ok(Json(serde_json::json!({ "ok": true })))
}

// ---------- Users ----------
#[derive(serde::Deserialize)]
pub struct CreateUserRequest {
    pub username: Option<String>,
    pub password: Option<String>,
    pub role: Option<String>,
    /// 所属租户 ID；默认 "default"
    #[serde(default = "default_tenant")]
    pub tenant: String,
}

#[derive(serde::Deserialize)]
pub struct UpdateUserRequest {
    pub username: Option<String>,
    pub role: Option<String>,
    /// 变更租户 ID（仅 Admin 可指定）
    pub tenant: Option<String>,
}

fn default_tenant() -> String {
    "default".to_string()
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
    let list = state.user_store.list().await.map_err(ApiError::internal)?;
    let arr: Vec<serde_json::Value> = list
        .into_iter()
        .map(|u| {
            serde_json::json!({
                "id": u.id,
                "username": u.username,
                "role": u.role.as_str(),
                "tenant_id": u.tenant_id,
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
        .create(username, password, role_enum, &body.tenant)
        .await
        .map_err(ApiError::bad_request)?;
    Ok(Json(serde_json::json!({
        "id": user.id,
        "username": user.username,
        "role": user.role.as_str(),
        "tenant_id": user.tenant_id,
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
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::not_found("user not found"))?;
    Ok(Json(serde_json::json!({
        "id": user.id,
        "username": user.username,
        "role": user.role.as_str(),
        "tenant_id": user.tenant_id,
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
        .update_simple(&id, username, role, body.tenant.as_deref())
        .await
        .map_err(ApiError::bad_request)?;
    Ok(Json(serde_json::json!({
        "id": user.id,
        "username": user.username,
        "role": user.role.as_str(),
        "tenant_id": user.tenant_id,
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
        .map_err(ApiError::internal)?;
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
        .map_err(ApiError::bad_request)?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------- Tenants ----------
/// 租户列表：Admin 返回所有租户；Operator/Viewer 仅返回自身租户（按 id/name 形式）。
pub async fn list_tenants(
    State(state): State<AppState>,
    Extension(ctx): Extension<crate::api::AuthContext>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let tenants = match &ctx.tenant {
        crate::api::TenantScope::All => {
            let rows = state
                .user_store
                .list_tenants()
                .await
                .map_err(ApiError::internal)?;
            rows.into_iter()
                .map(|t| serde_json::json!({ "id": t.id, "name": t.name }))
                .collect::<Vec<_>>()
        }
        crate::api::TenantScope::One(tid) => {
            // 非 Admin 只能看到自己的租户
            vec![serde_json::json!({ "id": tid, "name": tid })]
        }
    };
    Ok(Json(serde_json::json!({ "tenants": tenants })))
}

#[derive(serde::Deserialize)]
pub struct CreateTenantReq {
    pub id: String,
    pub name: String,
}

/// 创建租户（Admin）。
pub async fn create_tenant(
    State(state): State<AppState>,
    Json(req): Json<CreateTenantReq>,
) -> Result<Json<serde_json::Value>, ApiError> {
    if req.id.trim().is_empty() {
        return Err(ApiError::bad_request("tenant id cannot be empty"));
    }
    if req.id == "default" {
        return Err(ApiError::bad_request(
            "cannot create built-in tenant 'default'",
        ));
    }
    state
        .user_store
        .create_tenant(&req.id, &req.name)
        .await
        .map_err(ApiError::bad_request)?;
    Ok(Json(serde_json::json!({ "id": req.id, "name": req.name })))
}

/// 删除租户（Admin）。租户下有用户时拒绝删除。
pub async fn delete_tenant(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    if id == "default" {
        return Err(ApiError::bad_request(
            "cannot delete built-in tenant 'default'",
        ));
    }
    state
        .user_store
        .delete_tenant(&id)
        .await
        .map_err(ApiError::bad_request)?;
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
    let mid = crate::license::machine_id().map_err(ApiError::internal)?;
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

/// 备份/恢复的默认密钥来源：优先使用服务配置（GATEWAY_BACKUP_SECRET / 随机生成），
/// 不再使用写死在代码中的固定口令。
fn backup_secret_of(state: &AppState) -> String {
    state.config.backup_secret.clone()
}

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
        .unwrap_or_else(|| backup_secret_of(&state));
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
        .unwrap_or_else(|| backup_secret_of(&state));
    let snap = backup::decrypt_backup(&data, &secret).map_err(ApiError::bad_request)?;
    if snap.version != gateway_core::SNAPSHOT_VERSION {
        return Err(ApiError::bad_request(format!(
            "unsupported snapshot version: {}, expected {}",
            snap.version,
            gateway_core::SNAPSHOT_VERSION
        )));
    }
    // License 门禁前移到「最终状态」校验：恢复前先整体检查插件授权与点位上限，
    // 避免通过备份绕过 create_node / add_tag 的增量校验。
    let mut violations: Vec<String> = Vec::new();
    for n in &snap.nodes {
        if !state.feature_manager.can_use_plugin(&n.config.plugin_name) {
            violations.push(format!(
                "plugin '{}' is not licensed (node '{}')",
                n.config.plugin_name, n.config.name
            ));
        }
    }
    if let Err(e) = state
        .feature_manager
        .check_tag_limit(0, snap.tags.len() as u64)
    {
        violations.push(e);
    }
    if !violations.is_empty() {
        return Err(ApiError::forbidden(format!(
            "restore rejected by license policy: {}",
            violations.join("; ")
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
         # HELP gateway_south_poll_timeout Total south poll_group timeouts.\n\
         # TYPE gateway_south_poll_timeout counter\n\
         gateway_south_poll_timeout {}\n\
         # HELP gateway_south_poll_err Total south poll_group errors.\n\
         # TYPE gateway_south_poll_err counter\n\
         gateway_south_poll_err {}\n\
         # HELP gateway_south_poll_overrun Poll batches that took longer than the smallest interval.\n\
         # TYPE gateway_south_poll_overrun counter\n\
         gateway_south_poll_overrun {}\n\
         # HELP gateway_rules_fired Total rule firings.\n\
         # TYPE gateway_rules_fired counter\n\
         gateway_rules_fired {}\n\
         # HELP gateway_rules_action_err Total rule actions that failed.\n\
         # TYPE gateway_rules_action_err counter\n\
         gateway_rules_action_err {}\n\
         # HELP gateway_history_rows_written Total history samples written.\n\
         # TYPE gateway_history_rows_written counter\n\
         gateway_history_rows_written {}\n\
         # HELP gateway_history_rows_pruned Total history samples deleted by retention.\n\
         # TYPE gateway_history_rows_pruned counter\n\
         gateway_history_rows_pruned {}\n\
         # HELP gateway_history_write_err Total history write failures.\n\
         # TYPE gateway_history_write_err counter\n\
         gateway_history_write_err {}\n\
         # HELP gateway_ws_clients Current WebSocket client count.
         # TYPE gateway_ws_clients gauge
         gateway_ws_clients {}\n\
         # HELP gateway_ws_frames_sent_total Total WS frames sent to clients.
         # TYPE gateway_ws_frames_sent_total counter
         gateway_ws_frames_sent_total {}\n\
         # HELP gateway_ws_frames_dropped_total Total WS frames dropped (full channel or lagged).
         # TYPE gateway_ws_frames_dropped_total counter
         gateway_ws_frames_dropped_total {}\n",
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
        df.south_poll_timeout,
        df.south_poll_err,
        df.south_poll_overrun,
        df.rules_fired,
        df.rules_action_err,
        df.history_rows_written,
        df.history_rows_pruned,
        df.history_write_err,
        crate::ws::metric_ws_clients(),
        crate::ws::metric_ws_frames_sent(),
        crate::ws::metric_ws_frames_dropped(),
    );
    // 维度化指标：定位「是哪个北向节点在丢数据」
    let mut body = body;
    // 授权到期天数：未授权时不输出该指标（避免 0 造成误判）
    if let Some(days) = state.feature_manager.expiry_days_left() {
        body.push_str(
            "# HELP gateway_license_expiry_days Days until license expiry (negative when expired).\n\
             # TYPE gateway_license_expiry_days gauge\n",
        );
        body.push_str(&format!("gateway_license_expiry_days {}\n", days));
    }
    if !df.lagged_by_node.is_empty() {
        body.push_str(
            "# HELP gateway_north_lagged_by_node Messages skipped due to lag, per north node.\n\
             # TYPE gateway_north_lagged_by_node counter\n",
        );
        for (nid, n) in &df.lagged_by_node {
            let name = state
                .manager
                .node_get(*nid)
                .map(|node| node.config.name)
                .unwrap_or_else(|| nid.0.to_string());
            body.push_str(&format!(
                "gateway_north_lagged_by_node{{north_node=\"{}\",north_node_id=\"{}\"}} {}\n",
                name, nid.0, n
            ));
        }
    }
    // 进程隔离模式：暴露插件子进程数量与重启次数（插件崩溃自愈的观测点）
    if let Some(loader) = &state.plugin_processes {
        body.push_str(
            "# HELP gateway_plugin_processes Number of plugin host child processes.\n\
             # TYPE gateway_plugin_processes gauge\n",
        );
        body.push_str(&format!(
            "gateway_plugin_processes {}\n",
            loader.process_count().await
        ));
        body.push_str(
            "# HELP gateway_plugin_process_restarts Total plugin host process restarts.\n\
             # TYPE gateway_plugin_process_restarts counter\n",
        );
        body.push_str(&format!(
            "gateway_plugin_process_restarts {}\n",
            loader.total_restarts().await
        ));
    }
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
            "south_poll_timeout": m.south_poll_timeout,
            "south_poll_err": m.south_poll_err,
            "south_poll_overrun": m.south_poll_overrun,
            "rules_fired": m.rules_fired,
            "rules_action_err": m.rules_action_err,
            "history_rows_written": m.history_rows_written,
            "history_rows_pruned": m.history_rows_pruned,
            "history_write_err": m.history_write_err,
        },
        "lagged_by_node": m.lagged_by_node.iter().map(|(nid, n)| {
            serde_json::json!({
                "north_node_id": nid,
                "north_node_name": state.manager.node_get(*nid).map(|node| node.config.name),
                "skipped": n,
            })
        }).collect::<Vec<_>>(),
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
            })
        })
        .collect();
    Json(result)
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

/// 按插件 Schema 脱敏节点配置：敏感字段（口令、私钥、Token 等）统一返回 `"***"`
pub(crate) fn mask_plugin_config(
    state: &AppState,
    plugin_name: &str,
    north: bool,
    cfg: &serde_json::Value,
) -> serde_json::Value {
    let map: gateway_sdk::PluginConfig = serde_json::from_value(cfg.clone()).unwrap_or_default();
    let schema = if north {
        state
            .manager
            .north_plugin(plugin_name)
            .and_then(|p| p.config_schema())
    } else {
        state
            .manager
            .south_plugin(plugin_name)
            .and_then(|p| p.config_schema())
    };
    let masked = match schema {
        Some(s) => s.mask_config(&map),
        None => map
            .into_iter()
            .map(|(k, v)| {
                let v = if gateway_sdk::schema::is_sensitive_key(&k) {
                    serde_json::Value::String(gateway_sdk::schema::MASKED.to_string())
                } else {
                    v
                };
                (k, v)
            })
            .collect(),
    };
    serde_json::to_value(masked).unwrap_or_else(|_| serde_json::json!({}))
}

pub async fn list_nodes(
    State(state): State<AppState>,
    axum::extract::Extension(ctx): axum::extract::Extension<crate::api::AuthContext>,
) -> Json<Vec<serde_json::Value>> {
    let nodes = crate::api::scope::scoped_nodes(&state, &ctx.tenant);
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
            // 脱敏：口令 / 私钥 / Token 等敏感配置不以明文返回
            if let Some(cfg) = obj.get_mut("config") {
                let masked = mask_plugin_config(
                    &state,
                    &node.config.plugin_name,
                    node.kind() == NodeKind::North,
                    cfg,
                );
                *cfg = masked;
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
    Extension(ctx): Extension<crate::api::AuthContext>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let (nid, node) = crate::api::scope::resolve_node(&state, &ctx, &id)?;
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
        if let Some(cfg) = obj.get_mut("config") {
            let masked = mask_plugin_config(
                &state,
                &node.config.plugin_name,
                node.kind() == NodeKind::North,
                cfg,
            );
            *cfg = masked;
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
) -> Result<StatusCode, ApiError> {
    let nid = parse_node_id(&id)?;
    state
        .manager
        .node_update(nid, req.name)
        .map_err(ApiError::bad_request)?;
    state.sync_node_log_names();
    state.persist().await;
    Ok(StatusCode::NO_CONTENT)
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
    // 脱敏后返回：前端表单仍可展示 "***"，未修改不会覆盖真实值
    let masked = mask_plugin_config(
        &state,
        &node.config.plugin_name,
        node.kind() == NodeKind::North,
        &serde_json::to_value(&node.config.config).unwrap_or_else(|_| serde_json::json!({})),
    );
    Ok(Json(serde_json::json!({ "config": masked })))
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

/// 轮询周期下限校验：过小的周期会造成忙循环并压垮设备，属于配置事故
pub(super) fn validate_interval_ms(interval_ms: u64) -> Result<u64, ApiError> {
    if interval_ms < gateway_core::MIN_POLL_INTERVAL_MS {
        return Err(ApiError::bad_request(format!(
            "interval_ms must be >= {} (got {})",
            gateway_core::MIN_POLL_INTERVAL_MS,
            interval_ms
        )));
    }
    Ok(interval_ms)
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
    let interval_ms = validate_interval_ms(req.interval_ms)?;
    let mut g = Group::new(req.name, interval_ms);
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
        .group_update(
            nid,
            g,
            req.name,
            req.interval_ms.map(validate_interval_ms).transpose()?,
            req.description,
        )
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
    // 前端表单对敏感字段只回显 "***"：若提交值仍为该占位符，则保留原有真实值，避免被覆盖
    let mut config = req.config;
    if let Some(node) = state.manager.node_get(nid) {
        for (k, v) in config.iter_mut() {
            if gateway_sdk::schema::is_sensitive_key(k)
                && v.as_str() == Some(gateway_sdk::schema::MASKED)
            {
                if let Some(old) = node.config.config.get(k) {
                    *v = old.clone();
                }
            }
        }
    }
    state
        .manager
        .node_setting(nid, config)
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

// ---------- 硬件信息 ----------
/// 硬件与运行环境信息，供系统信息页展示。
pub async fn hardware(State(state): State<AppState>) -> Json<serde_json::Value> {
    let hardware_id = crate::license::machine_id().ok();
    let info = tokio::task::spawn_blocking(move || {
        use sysinfo::{CpuExt, DiskExt, SystemExt};
        let mut sys = sysinfo::System::new_all();
        // CPU 使用率为两次采样的差值，需短暂间隔后再采样
        std::thread::sleep(std::time::Duration::from_millis(120));
        sys.refresh_cpu();
        let cpu_usage = sys.global_cpu_info().cpu_usage() as f64 / 100.0;
        let memory_total = sys.total_memory().saturating_mul(1024); // sysinfo 以 KB 为单位
        let memory_used = sys.used_memory().saturating_mul(1024);
        let (disk_total, disk_available) = {
            let mut total = 0u64;
            let mut avail = 0u64;
            for d in sys.disks() {
                total += d.total_space();
                avail += d.available_space();
            }
            (total, avail)
        };
        serde_json::json!({
            "cpu_count": sys.cpus().len(),
            "cpu_usage": cpu_usage,
            "memory_total": memory_total,
            "memory_used": memory_used,
            "memory_available": sys.available_memory().saturating_mul(1024),
            "disk_total": disk_total,
            "disk_available": disk_available,
            "hostname": sys.host_name(),
            "os_version": sys.long_os_version().unwrap_or_else(|| sys.os_version().unwrap_or_default()),
            "kernel_version": sys.kernel_version().unwrap_or_default(),
        })
    })
    .await
    .unwrap_or_else(|_| serde_json::json!({}));

    let mut out = info;
    if let Some(obj) = out.as_object_mut() {
        obj.insert("hardware_id".into(), serde_json::json!(hardware_id));
        obj.insert("arch".into(), serde_json::json!(std::env::consts::ARCH));
        obj.insert("os".into(), serde_json::json!(std::env::consts::OS));
        obj.insert(
            "data_dir".into(),
            serde_json::json!(state.config.data_dir.display().to_string()),
        );
    }
    Json(out)
}

// ---------- 日志管理 ----------
#[derive(Deserialize)]
pub struct LogConfigReq {
    pub level: Option<String>,
    pub upload_enabled: Option<bool>,
}

/// 进程内日志上传开关（功能占位：当前版本不主动上传日志）
static LOG_UPLOAD_ENABLED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

fn valid_log_level(s: &str) -> bool {
    matches!(s, "trace" | "debug" | "info" | "warn" | "error")
}

/// 当前生效的配置文件路径（GATEWAY_CONFIG 或 config/gateway.json）
fn config_file_path() -> std::path::PathBuf {
    std::env::var("GATEWAY_CONFIG")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("config").join("gateway.json"))
}

pub async fn get_log_config(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "level": state.config.log_level,
        "filter": state.config.log_filter,
        "file": state.config.log_file.as_ref().map(|p| p.display().to_string()),
        "dir_nodes": state.config.log_dir_nodes.as_ref().map(|p| p.display().to_string()),
        "upload_enabled": LOG_UPLOAD_ENABLED.load(std::sync::atomic::Ordering::Relaxed),
        "upload_supported": false,
    }))
}

/// 修改日志级别；写入配置文件以便重启后继续生效（下次启动生效）。
pub async fn put_log_config(
    State(state): State<AppState>,
    Json(req): Json<LogConfigReq>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let mut level = state.config.log_level.clone();
    let mut persisted = false;
    if let Some(l) = req.level.as_deref() {
        let l = l.trim().to_lowercase();
        if !valid_log_level(&l) {
            return Err(ApiError::bad_request(
                "invalid log level, expect one of trace|debug|info|warn|error",
            ));
        }
        level = l;
        let path = config_file_path();
        match tokio::fs::read_to_string(&path).await {
            Ok(text) => match serde_json::from_str::<serde_json::Value>(&text) {
                Ok(mut v) => {
                    if let Some(obj) = v.as_object_mut() {
                        obj.insert("log_level".into(), serde_json::json!(level));
                    }
                    if let Ok(s) = serde_json::to_string_pretty(&v) {
                        persisted = tokio::fs::write(&path, s).await.is_ok();
                    }
                }
                Err(e) => tracing::warn!("parse {} failed: {}", path.display(), e),
            },
            Err(e) => tracing::warn!("read {} failed: {}", path.display(), e),
        }
    }
    if let Some(v) = req.upload_enabled {
        LOG_UPLOAD_ENABLED.store(v, std::sync::atomic::Ordering::Relaxed);
    }
    Ok(Json(serde_json::json!({
        "level": level,
        "upload_enabled": LOG_UPLOAD_ENABLED.load(std::sync::atomic::Ordering::Relaxed),
        "persisted": persisted,
        "restart_required": true,
    })))
}

/// 单个日志文件最大返回字节（超出取尾部）
const MAX_LOG_BYTES: u64 = 20 * 1024 * 1024;

async fn read_log_tail(path: &std::path::Path) -> Option<String> {
    let meta = tokio::fs::metadata(path).await.ok()?;
    if !meta.is_file() {
        return None;
    }
    let bytes = if meta.len() > MAX_LOG_BYTES {
        use std::io::{Read, Seek, SeekFrom};
        let mut f = std::fs::File::open(path).ok()?;
        f.seek(SeekFrom::End(-(MAX_LOG_BYTES as i64))).ok()?;
        let mut buf = Vec::with_capacity(MAX_LOG_BYTES as usize);
        f.read_to_end(&mut buf).ok()?;
        String::from_utf8_lossy(&buf).to_string()
    } else {
        tokio::fs::read_to_string(path).await.ok()?
    };
    Some(bytes)
}

/// 主日志文件基础名（tracing_appender 按日滚动，实际文件名形如 gateway.log.2026-09-26）
const GATEWAY_LOG_STEM: &str = "gateway.log";

/// 主日志文件列表（含按日滚动产生的历史文件，按名称排序）
fn gateway_log_files(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut list = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return list;
    };
    for e in entries.flatten() {
        let p = e.path();
        let Some(name) = p.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if name.starts_with(GATEWAY_LOG_STEM) && p.is_file() {
            list.push(p);
        }
    }
    list.sort();
    list
}

fn node_log_files(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut list = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return list;
    };
    for e in entries.flatten() {
        let p = e.path();
        let Some(ext) = p.extension().and_then(|x| x.to_str()) else {
            continue;
        };
        if ext == "log" && p.is_file() {
            list.push(p);
        }
    }
    list.sort();
    list
}

/// 日志下载：type=system|driver|node|all（node 需配合 node_id）
pub async fn download_log(
    State(state): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Result<axum::response::Response, ApiError> {
    use axum::http::{header, Response, StatusCode};

    let kind = q.get("type").map(|s| s.as_str()).unwrap_or("all");
    let mut buf = String::new();
    let filename = match kind {
        "system" | "gateway" => {
            let dir = state
                .config
                .log_file
                .as_ref()
                .and_then(|p| p.parent())
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| std::path::PathBuf::from("logs"));
            let files = gateway_log_files(&dir);
            if files.is_empty() {
                return Err(ApiError::not_found("system log file not found"));
            }
            for f in files {
                if let Some(content) = read_log_tail(&f).await {
                    buf.push_str(&format!(
                        "\n===== {} =====\n",
                        f.file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("gateway.log")
                    ));
                    buf.push_str(&content);
                }
            }
            "gateway-system.log".to_string()
        }
        "driver" | "node" => {
            let dir = state
                .config
                .log_dir_nodes
                .clone()
                .unwrap_or_else(|| std::path::PathBuf::from("logs/nodes"));
            let files: Vec<std::path::PathBuf> = match (kind, q.get("node_id")) {
                ("node", Some(nid)) => {
                    let p = crate::logging::node_log_file_path(
                        &dir,
                        nid,
                        state.node_log_names.as_ref(),
                    );
                    if !p.exists() {
                        return Err(ApiError::not_found("node log file not found"));
                    }
                    vec![p]
                }
                _ => node_log_files(&dir),
            };
            if files.is_empty() {
                return Err(ApiError::not_found("node log files not found"));
            }
            for f in files {
                if let Some(content) = read_log_tail(&f).await {
                    buf.push_str(&format!(
                        "\n===== {} =====\n",
                        f.file_name().and_then(|n| n.to_str()).unwrap_or("node.log")
                    ));
                    buf.push_str(&content);
                }
            }
            format!("gateway-{}.log", kind)
        }
        "all" => {
            let gdir = state
                .config
                .log_file
                .as_ref()
                .and_then(|p| p.parent())
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| std::path::PathBuf::from("logs"));
            for f in gateway_log_files(&gdir) {
                if let Some(content) = read_log_tail(&f).await {
                    buf.push_str(&format!(
                        "\n===== system/{} =====\n",
                        f.file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("gateway.log")
                    ));
                    buf.push_str(&content);
                }
            }
            let ndir = state
                .config
                .log_dir_nodes
                .clone()
                .unwrap_or_else(|| std::path::PathBuf::from("logs/nodes"));
            for f in node_log_files(&ndir) {
                if let Some(content) = read_log_tail(&f).await {
                    buf.push_str(&format!(
                        "\n===== nodes/{} =====\n",
                        f.file_name().and_then(|n| n.to_str()).unwrap_or("node.log")
                    ));
                    buf.push_str(&content);
                }
            }
            if buf.is_empty() {
                return Err(ApiError::not_found("no log files found"));
            }
            "gateway-all.log".to_string()
        }
        other => {
            return Err(ApiError::bad_request(format!(
                "invalid type '{}', expect one of system|driver|node|all",
                other
            )))
        }
    };

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{}\"", filename),
        )
        .header(header::CACHE_CONTROL, "no-store")
        .body(axum::body::Body::from(buf))
        .unwrap_or_else(|_| {
            (StatusCode::INTERNAL_SERVER_ERROR, "build response failed").into_response()
        })
        .into_response())
}

// ---------- 系统配置 ----------
/// 返回当前生效的系统配置（**脱敏**：token、备份密钥等敏感项不返回明文）
pub async fn get_system_config(State(state): State<AppState>) -> Json<serde_json::Value> {
    let c = &state.config;
    Json(serde_json::json!({
        "port": c.port,
        "bind": c.bind,
        "data_dir": c.data_dir.display().to_string(),
        "static_dir": c.static_dir.display().to_string(),
        "plugins_dir": c.plugins_dir.display().to_string(),
        "log_level": c.log_level,
        "log_filter": c.log_filter,
        "log_file": c.log_file.as_ref().map(|p| p.display().to_string()),
        "log_dir_nodes": c.log_dir_nodes.as_ref().map(|p| p.display().to_string()),
        "auth_enabled": !c.disable_auth,
        "token_configured": c.token.is_some(),
        "allowed_origins": c.allowed_origins,
        "backup_secret_configured": !c.backup_secret_ephemeral,
    }))
}

#[derive(Deserialize)]
pub struct SystemConfigReq {
    pub log_level: Option<String>,
    pub log_filter: Option<String>,
}

/// 热改安全子集（日志级别/filter）；其余配置需通过配置文件或环境变量修改后重启。
pub async fn put_system_config(
    State(state): State<AppState>,
    Json(req): Json<SystemConfigReq>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let mut v: serde_json::Value =
        serde_json::to_value(get_system_config(State(state.clone())).await.0)
            .unwrap_or_else(|_| serde_json::json!({}));
    let path = config_file_path();
    let mut persisted = false;
    if tokio::fs::metadata(&path).await.is_ok() {
        if let Ok(text) = tokio::fs::read_to_string(&path).await {
            if let Ok(mut cfg) = serde_json::from_str::<serde_json::Value>(&text) {
                let mut changed = false;
                if let Some(l) = req.log_level.as_deref() {
                    let l = l.trim().to_lowercase();
                    if !valid_log_level(&l) {
                        return Err(ApiError::bad_request(
                            "invalid log_level, expect one of trace|debug|info|warn|error",
                        ));
                    }
                    if let Some(obj) = cfg.as_object_mut() {
                        obj.insert("log_level".into(), serde_json::json!(l));
                        changed = true;
                    }
                    if let Some(obj) = v.as_object_mut() {
                        obj.insert("log_level".into(), serde_json::json!(l));
                    }
                }
                if let Some(f) = req.log_filter.as_deref() {
                    if let Some(obj) = cfg.as_object_mut() {
                        obj.insert("log_filter".into(), serde_json::json!(f));
                        changed = true;
                    }
                    if let Some(obj) = v.as_object_mut() {
                        obj.insert("log_filter".into(), serde_json::json!(f));
                    }
                }
                if changed {
                    if let Ok(s) = serde_json::to_string_pretty(&cfg) {
                        persisted = tokio::fs::write(&path, s).await.is_ok();
                    }
                }
            }
        }
    }
    if let Some(obj) = v.as_object_mut() {
        obj.insert("persisted".into(), serde_json::json!(persisted));
        obj.insert("restart_required".into(), serde_json::json!(true));
    }
    Ok(Json(v))
}

// ---------- Rules ----------

#[derive(Deserialize)]
pub struct CreateRuleReq {
    /// 为空时由服务端生成
    #[serde(default)]
    pub id: String,
    pub name: String,
    #[serde(default = "default_rule_enabled")]
    pub enabled: bool,
    pub source: gateway_core::RuleSource,
    pub condition: gateway_core::RuleCondition,
    #[serde(default)]
    pub for_ms: u64,
    #[serde(default)]
    pub clear_ms: u64,
    pub action: gateway_core::RuleAction,
}

fn default_rule_enabled() -> bool {
    true
}

#[derive(Deserialize)]
pub struct EnableRuleReq {
    pub enabled: bool,
}

impl CreateRuleReq {
    fn into_rule(self, id: String, tenant_id: String) -> gateway_core::Rule {
        gateway_core::Rule {
            id,
            name: self.name,
            enabled: self.enabled,
            source: self.source,
            condition: self.condition,
            for_ms: self.for_ms,
            clear_ms: self.clear_ms,
            action: self.action,
            tenant_id,
        }
    }
}

/// 规则列表：配置 + 运行期状态（触发次数、最近触发时间、最近值）
pub async fn list_rules(
    State(state): State<AppState>,
    axum::extract::Extension(ctx): axum::extract::Extension<crate::api::AuthContext>,
) -> Json<Vec<gateway_core::RuleView>> {
    let engine = gateway_core::rule_engine();
    let out = crate::api::scope::scoped_rules(&state, &ctx.tenant)
        .into_iter()
        .map(|rule| gateway_core::RuleView {
            runtime: engine.runtime(&rule.id),
            rule,
        })
        .collect();
    Json(out)
}

/// 新建规则
pub async fn create_rule(
    State(state): State<AppState>,
    Json(req): Json<CreateRuleReq>,
) -> Result<impl IntoResponse, ApiError> {
    let id = if req.id.trim().is_empty() {
        uuid::Uuid::new_v4().to_string()
    } else {
        req.id.trim().to_string()
    };
    if state.manager.rule_get(&id).is_some() {
        return Err(ApiError::bad_request(format!(
            "rule id already exists: {}",
            id
        )));
    }
    // 规则域以 source 节点域为准盖章
    let source_node = state
        .manager
        .node_get(req.source.south_node_id)
        .ok_or_else(|| ApiError::bad_request("source node not found"))?;
    let tenant_id = source_node.config.tenant_id;
    let rule = req.into_rule(id, tenant_id);
    rule.validate().map_err(ApiError::bad_request)?;
    validate_rule_refs(&state, &rule)?;
    state.manager.rule_insert(rule.clone());
    state.persist().await;
    Ok((StatusCode::CREATED, Json(rule)))
}

/// 更新规则（id 取自路径，body 里的 id 会被忽略）
pub async fn update_rule(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<CreateRuleReq>,
) -> Result<impl IntoResponse, ApiError> {
    let existing = state
        .manager
        .rule_get(&id)
        .ok_or_else(|| ApiError::not_found("rule not found"))?;
    // tenant_id 不可更改（由 source 节点域决定），更新时保留原值
    let mut rule = req.into_rule(id.clone(), existing.tenant_id);
    // 更新不改变启用状态，除非请求显式给出（这里保持与 body 一致，由前端决定）
    rule.id = existing.id;
    rule.validate().map_err(ApiError::bad_request)?;
    validate_rule_refs(&state, &rule)?;
    state.manager.rule_insert(rule.clone());
    state.persist().await;
    Ok(Json(rule))
}

/// 删除规则
pub async fn delete_rule(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    if state.manager.rule_remove(&id).is_none() {
        return Err(ApiError::not_found("rule not found"));
    }
    state.persist().await;
    Ok(StatusCode::NO_CONTENT)
}

/// 启用 / 停用规则
pub async fn enable_rule(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<EnableRuleReq>,
) -> Result<impl IntoResponse, ApiError> {
    let mut rule = state
        .manager
        .rule_get(&id)
        .ok_or_else(|| ApiError::not_found("rule not found"))?;
    rule.enabled = req.enabled;
    state.manager.rule_insert(rule.clone());
    state.persist().await;
    Ok(Json(rule))
}

/// 规则引用的点位必须真实存在，否则规则永远不会触发（典型的静默失效）
fn validate_rule_refs(state: &AppState, rule: &gateway_core::Rule) -> Result<(), ApiError> {
    let node = state
        .manager
        .node_get(rule.source.south_node_id)
        .ok_or_else(|| ApiError::bad_request("source node not found"))?;
    if node.kind() != gateway_sdk::NodeKind::South {
        return Err(ApiError::bad_request("source node must be a south node"));
    }
    let group = state
        .manager
        .group_get(rule.source.south_node_id, rule.source.group_id)
        .ok_or_else(|| ApiError::bad_request("source group not found"))?;
    state
        .manager
        .tag_get_by_name(rule.source.south_node_id, group.id, &rule.source.tag_name)
        .ok_or_else(|| {
            ApiError::bad_request(format!(
                "source tag not found in group: {}",
                rule.source.tag_name
            ))
        })?;
    if let gateway_core::RuleAction::WriteTag { tag_name, .. } = &rule.action {
        state
            .manager
            .tag_get_by_name(rule.source.south_node_id, rule.source.group_id, tag_name)
            .ok_or_else(|| {
                ApiError::bad_request(format!("action target tag not found: {}", tag_name))
            })?;
    }
    Ok(())
}

// ---------- History ----------

#[derive(Deserialize)]
pub struct SeriesQueryParams {
    pub node_id: String,
    pub group_id: String,
    pub tag: String,
    /// 起始时间（毫秒，可选；默认 1 小时前）
    pub from: Option<i64>,
    /// 结束时间（毫秒，可选；默认当前）
    pub to: Option<i64>,
    /// 期望最大点数（默认 500，上限 5000）
    pub max_points: Option<u32>,
}

#[derive(Deserialize)]
pub struct SeriesListParams {
    pub limit: Option<u32>,
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// 历史序列查询：SQL 侧分桶降采样，点数不超过 `max_points`
pub async fn history_series(
    State(state): State<AppState>,
    Query(p): Query<SeriesQueryParams>,
) -> Result<Json<serde_json::Value>, ApiError> {
    if !state.config.history_enabled {
        return Err(ApiError::bad_request(
            "history storage is disabled; set GATEWAY_HISTORY_ENABLED=1 to enable it",
        ));
    }
    let to = p.to.unwrap_or_else(now_ms);
    let from = p.from.unwrap_or(to - 3_600_000);
    if from >= to {
        return Err(ApiError::bad_request("from must be earlier than to"));
    }
    let max_points = p.max_points.unwrap_or(500).clamp(1, 5000);
    let q = crate::history::SeriesQuery {
        node_id: p.node_id,
        group_id: p.group_id,
        tag: p.tag,
        from_ms: from,
        to_ms: to,
        max_points,
    };
    let db = state.config.history_db();
    // SQLite 是阻塞 API：放到阻塞线程池，别占住 async worker
    let q2 = q.clone();
    let points = tokio::task::spawn_blocking(move || crate::history::query_series(&db, &q2))
        .await
        .map_err(|e| ApiError::bad_request(e.to_string()))?
        .map_err(ApiError::bad_request)?;
    Ok(Json(serde_json::json!({
        "node_id": q.node_id,
        "group_id": q.group_id,
        "tag": q.tag,
        "from": from,
        "to": to,
        "bucket_ms": crate::history::bucket_ms(from, to, max_points),
        "points": points,
    })))
}

/// 库中有哪些序列（供前端选择）
pub async fn history_series_list(
    State(state): State<AppState>,
    Query(p): Query<SeriesListParams>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let db = state.config.history_db();
    let limit = p.limit.unwrap_or(200).clamp(1, 2000);
    let list = tokio::task::spawn_blocking(move || crate::history::list_series(&db, limit))
        .await
        .map_err(|e| ApiError::bad_request(e.to_string()))?
        .map_err(ApiError::bad_request)?;
    Ok(Json(serde_json::json!({ "series": list })))
}

/// 历史存储统计：行数、磁盘占用、时间范围与配置
pub async fn history_stats(State(state): State<AppState>) -> Json<serde_json::Value> {
    let db = state.config.history_db();
    let cfg = state.config.history_cfg();
    let mut v = tokio::task::spawn_blocking(move || crate::history::db_stats(&db, &cfg))
        .await
        .unwrap_or_else(|e| Ok(serde_json::json!({ "error": e.to_string() })))
        .unwrap_or_else(|e| serde_json::json!({ "error": e }));
    if let Some(obj) = v.as_object_mut() {
        obj.insert(
            "runtime".to_string(),
            serde_json::json!({
                "rows_written": state.manager.data_flow_snapshot().history_rows_written,
                "rows_pruned": state.manager.data_flow_snapshot().history_rows_pruned,
                "write_errors": state.manager.data_flow_snapshot().history_write_err,
            }),
        );
    }
    Json(v)
}

/// 点位实时值：**来自采集缓存，不访问设备**。
///
/// 管理台的刷新按钮/定时刷新走这里；需要立即读取设备时用 `/values/read`（会真实下发）。
pub async fn node_values(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let nid = parse_node_id(&id)?;
    let node = state
        .manager
        .node_get(nid)
        .ok_or_else(|| ApiError::not_found("node not found"))?;
    let cached = state.manager.last_values(nid);
    let tags = state.manager.tags_by_node(nid);
    let values: Vec<serde_json::Value> = tags
        .iter()
        .map(|t| {
            let lv = cached.get(&t.id);
            serde_json::json!({
                "tag_id": t.id,
                "name": t.name,
                "group_id": t.group_id,
                "value": lv.map(|l| &l.value),
                "ts": lv.map(|l| l.ts_ms),
                "available": lv.is_some(),
            })
        })
        .collect();
    Ok(Json(serde_json::json!({
        "node_id": nid,
        "name": node.config.name,
        "source": "cache",
        "values": values,
    })))
}

// ---------------------------------------------------------------------------
// 组数据策略：死区/变化上报/滑动窗口聚合
// ---------------------------------------------------------------------------

/// GET /nodes/:id/groups/:gid/policy
pub async fn get_policy(
    State(state): State<AppState>,
    Path((id, gid)): Path<(String, String)>,
) -> Result<Json<Option<gateway_core::GroupPolicy>>, ApiError> {
    let nid = parse_node_id(&id)?;
    let gid = parse_group_id(&gid)?;
    ensure_node_south(&state, nid)?;
    let policy = state.manager.store.policy_get(nid, gid);
    Ok(Json(policy))
}

/// 校验策略中的 tag_name 是否都在该组内
fn validate_policy_tags(
    store: &gateway_core::Store,
    nid: gateway_sdk::NodeId,
    gid: gateway_sdk::GroupId,
    policy: &gateway_core::GroupPolicy,
) -> Result<(), String> {
    let group_tags = store.tags_by_group(nid, gid);
    let tag_names: std::collections::HashSet<_> =
        group_tags.iter().map(|t| t.name.as_str()).collect();
    for td in &policy.tags {
        if !tag_names.contains(td.tag_name.as_str()) {
            return Err(format!(
                "tag '{}' not found in group '{}'",
                td.tag_name, gid.0
            ));
        }
    }
    Ok(())
}

/// PUT /nodes/:id/groups/:gid/policy
pub async fn put_policy(
    State(state): State<AppState>,
    Path((id, gid)): Path<(String, String)>,
    Json(policy): Json<gateway_core::GroupPolicy>,
) -> Result<Json<gateway_core::GroupPolicy>, ApiError> {
    let nid = parse_node_id(&id)?;
    let gid = parse_group_id(&gid)?;
    ensure_node_south(&state, nid)?;

    // 路径参数覆盖 body 中的 id
    let mut policy = policy;
    policy.south_node_id = nid;
    policy.group_id = gid;

    // 校验
    gateway_core::validate_policy(&policy).map_err(ApiError::bad_request)?;

    // 校验 tag_name 存在于该组
    validate_policy_tags(&state.manager.store, nid, gid, &policy).map_err(ApiError::bad_request)?;

    // emit_ms < 组 interval_ms 时 warn（合法但有效输出周期受采集节拍限制）
    if let Some(ref win) = policy.window {
        if let Some(grp) = state.manager.store.group_get(nid, gid) {
            if win.emit_ms < grp.interval_ms {
                tracing::warn!(
                    "policy emit_ms ({}) < group interval_ms ({}); \
                     effective output period will be limited by interval_ms",
                    win.emit_ms,
                    grp.interval_ms
                );
            }
        }
    }

    // 写入存储 + 重建过滤基线（首采必报规则）
    state.manager.store.policy_insert(policy.clone());
    gateway_core::filters_forget_group(nid, gid);
    state.persist().await;
    Ok(Json(policy))
}

/// DELETE /nodes/:id/groups/:gid/policy
pub async fn delete_policy(
    State(state): State<AppState>,
    Path((id, gid)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let nid = parse_node_id(&id)?;
    let gid = parse_group_id(&gid)?;
    ensure_node_south(&state, nid)?;
    if state.manager.store.policy_remove(nid, gid).is_none() {
        return Err(ApiError::not_found("policy not found"));
    }
    // 清除过滤运行态
    gateway_core::filters_forget_group(nid, gid);
    state.persist().await;
    Ok(StatusCode::NO_CONTENT)
}
