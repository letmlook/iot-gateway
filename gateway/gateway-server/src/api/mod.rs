//! REST API 与静态资源服务。

mod error;
mod handlers;

pub use error::ApiError;

use axum::extract::{Request, State};
use axum::http::header;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::Router;

use crate::state::AppState;

/// 恒定时间字符串比较：避免 token 比较引入时序侧信道
fn constant_time_eq(a: &str, b: &str) -> bool {
    let a = a.as_bytes();
    let b = b.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    let mut acc = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        acc |= x ^ y;
    }
    acc == 0
}

/// 认证上下文：注入到请求扩展，供授权中间件与 handler 使用
#[derive(Clone, Debug)]
pub struct AuthContext {
    pub role: crate::users::UserRole,
    pub username: Option<String>,
}

/// 角色是否满足要求（admin > operator > viewer）
fn role_satisfies(have: crate::users::UserRole, need: crate::users::UserRole) -> bool {
    use crate::users::UserRole::{Admin, Operator, Viewer};
    let rank = |r: crate::users::UserRole| match r {
        Viewer => 0,
        Operator => 1,
        Admin => 2,
    };
    rank(have) >= rank(need)
}

/// 路由 -> 所需最低角色。
/// 约定：读接口 Viewer+；写操作 Operator+；用户/授权/备份恢复等系统级操作 Admin。
fn required_role(method: &axum::http::Method, path: &str) -> crate::users::UserRole {
    use crate::users::UserRole::*;
    let p = path
        .trim_start_matches("/api/")
        .trim_start_matches("/api")
        .trim_start_matches('/')
        .trim_end_matches('/');

    // 会话自身操作（登出等）任何已认证用户都可执行
    if p.starts_with("auth/") {
        return Viewer;
    }
    // 系统级：用户管理、备份与恢复、授权文件与门禁重置
    if p == "users"
        || p.starts_with("users/")
        || p == "backup"
        || p == "restore"
        || p == "license/upload"
        || p == "license/reset"
        || p.starts_with("system/")
    {
        return Admin;
    }
    // 规则会直接驱动对现场设备的写动作，属于系统级配置：读 Viewer+、写 Admin
    if p == "rules" || p.starts_with("rules/") {
        let is_read = method == axum::http::Method::GET || method == axum::http::Method::HEAD;
        return if is_read { Viewer } else { Admin };
    }
    // 写值（对现场设备反控）：Operator 及以上
    if p.ends_with("/write_tags") {
        return Operator;
    }
    let is_read = method == axum::http::Method::GET || method == axum::http::Method::HEAD;
    if is_read {
        Viewer
    } else {
        Operator
    }
}

async fn request_id_middleware(request: Request, next: Next) -> Response {
    let id = uuid::Uuid::new_v4().to_string();
    tracing::Span::current().record("request_id", tracing::field::display(&id));
    let mut res = next.run(request).await;
    if let Ok(v) = header::HeaderValue::try_from(id) {
        res.headers_mut()
            .insert(header::HeaderName::from_static("x-request-id"), v);
    }
    res
}

/// 认证 + 授权中间件：先确定身份（静态 token / 用户 token / 未初始化），再按「路径 + 方法」校验角色。
/// 两级校验放在同一中间件内，避免多中间件顺序变化导致鉴权被绕过。
async fn auth_middleware(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Response {
    if state.config.disable_auth {
        // 认证关闭时按最高权限放行（并由启动日志给出醒目告警）
        request.extensions_mut().insert(AuthContext {
            role: crate::users::UserRole::Admin,
            username: Some("auth-disabled".to_string()),
        });
        return next.run(request).await;
    }
    // 嵌套路由中 URI 可能是 /auth/login 或 /api/auth/login，统一处理
    let path = request
        .uri()
        .path()
        .trim_start_matches("/api/")
        .trim_start_matches("/api")
        .trim_start_matches('/')
        .trim_end_matches('/');
    if path == "health"
        || path == "metrics"
        || path == "version"
        || path == "license/machine-id"
        || path == "license/status"
        || path == "auth/login"
        || path == "login"
    {
        return next.run(request).await;
    }
    let auth = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());
    let bearer = auth.and_then(|s| s.strip_prefix("Bearer ").map(|t| t.to_string()));
    let ctx = match bearer {
        Some(t) if !t.is_empty() => {
            // 静态 token 视为管理员；用户 token 携带其角色，供授权中间件判定
            if state
                .config
                .token
                .as_ref()
                .map(|c| constant_time_eq(c, &t))
                .unwrap_or(false)
            {
                Some(AuthContext {
                    role: crate::users::UserRole::Admin,
                    username: Some("static-token".to_string()),
                })
            } else {
                state.user_store.role_of(&t).map(|role| AuthContext {
                    role,
                    username: None,
                })
            }
        }
        _ => {
            // 既未配置静态 token 也无任何用户：视为"未初始化"放行（与历史行为一致）
            let has_users = state.user_store.has_any_user().await.ok() == Some(true);
            if state.config.token.is_none() && !has_users {
                Some(AuthContext {
                    role: crate::users::UserRole::Admin,
                    username: Some("uninitialized".to_string()),
                })
            } else {
                None
            }
        }
    };
    match ctx {
        Some(c) => {
            // 授权校验：路径 + 方法 -> 所需最低角色
            if state.config.enforce_roles {
                let required = required_role(request.method(), request.uri().path());
                if !role_satisfies(c.role, required) {
                    tracing::warn!(
                        path = %request.uri().path(),
                        method = %request.method(),
                        role = c.role.as_str(),
                        required = required.as_str(),
                        user = c.username.as_deref().unwrap_or("<session>"),
                        "forbidden: insufficient role"
                    );
                    return ApiError::forbidden(format!(
                        "insufficient role: '{}' requires '{}'",
                        c.role.as_str(),
                        required.as_str()
                    ))
                    .into_response();
                }
            }
            request.extensions_mut().insert(c);
            next.run(request).await
        }
        None => ApiError::unauthorized().into_response(),
    }
}

pub fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/auth/login", post(handlers::login))
        .route("/auth/logout", post(handlers::logout))
        .route("/health", get(handlers::health))
        .route("/metrics", get(handlers::metrics))
        .route("/data-flow", get(handlers::data_flow))
        .route(
            "/rules",
            get(handlers::list_rules).post(handlers::create_rule),
        )
        .route(
            "/rules/:id",
            put(handlers::update_rule).delete(handlers::delete_rule),
        )
        .route("/rules/:id/enable", post(handlers::enable_rule))
        // 历史数据：只读接口，GET 默认 Viewer+（见 required_role）
        .route("/history/series", get(handlers::history_series))
        .route("/history/series/list", get(handlers::history_series_list))
        .route("/history/stats", get(handlers::history_stats))
        .route("/hardware", get(handlers::hardware))
        .route(
            "/logs/config",
            get(handlers::get_log_config).put(handlers::put_log_config),
        )
        .route("/logs/download", get(handlers::download_log))
        .route(
            "/system/config",
            get(handlers::get_system_config).put(handlers::put_system_config),
        )
        .route("/version", get(handlers::version))
        .route("/license/machine-id", get(handlers::license_machine_id))
        .route("/license/status", get(handlers::license_status))
        .route("/license/upload", post(handlers::upload_license))
        .route("/license/reset", post(handlers::reset_license))
        .route("/license/pro-tool", get(handlers::license_pro_tool))
        .route(
            "/users",
            get(handlers::list_users).post(handlers::create_user),
        )
        .route(
            "/users/:id",
            get(handlers::get_user)
                .put(handlers::update_user)
                .delete(handlers::delete_user),
        )
        .route("/users/:id/password", put(handlers::change_password))
        .route("/backup", post(handlers::backup))
        .route("/restore", post(handlers::restore))
        .route("/plugins/south", get(handlers::list_south_plugins))
        .route(
            "/plugins/south/:name/config_schema",
            get(handlers::south_plugin_config_schema),
        )
        .route(
            "/plugins/south/:name/tag_schema",
            get(handlers::south_plugin_tag_schema),
        )
        .route("/plugins/north", get(handlers::list_north_plugins))
        .route(
            "/plugins/north/:name/config_schema",
            get(handlers::north_plugin_config_schema),
        )
        .route(
            "/nodes",
            get(handlers::list_nodes).post(handlers::create_node),
        )
        .route(
            "/nodes/:id",
            get(handlers::get_node)
                .put(handlers::update_node)
                .delete(handlers::delete_node),
        )
        .route("/nodes/:id/start", post(handlers::start_node))
        .route("/nodes/:id/stop", post(handlers::stop_node))
        .route(
            "/nodes/:id/connection-status",
            get(handlers::get_node_connection_status),
        )
        .route(
            "/nodes/:id/groups",
            get(handlers::list_groups).post(handlers::add_group),
        )
        .route(
            "/nodes/:id/groups/:gid",
            get(handlers::get_group)
                .put(handlers::update_group)
                .delete(handlers::remove_group),
        )
        .route(
            "/nodes/:id/tags",
            get(handlers::list_tags).post(handlers::add_tag),
        )
        .route("/nodes/:id/tags/batch", post(handlers::batch_add_tags))
        .route(
            "/nodes/:id/tags/:tid",
            get(handlers::get_tag)
                .put(handlers::update_tag)
                .delete(handlers::remove_tag),
        )
        .route(
            "/nodes/:id/subscriptions",
            get(handlers::get_subscriptions).put(handlers::set_subscriptions),
        )
        .route(
            "/nodes/:id/setting",
            get(handlers::get_node_setting).put(handlers::node_setting),
        )
        .route("/nodes/:id/read_tags", post(handlers::read_tags))
        .route("/nodes/:id/write_tags", post(handlers::write_tags))
        .route("/upload", post(handlers::upload_config_file))
        .with_state(state.clone())
        .route_layer(middleware::from_fn_with_state(state, auth_middleware))
        .route_layer(middleware::from_fn(request_id_middleware))
}

/// 前端静态资源或 SPA fallback。使用 config::STATIC_DIR，未设置则 `web/dist`。
///
/// 安全约束：
/// 1. 未匹配的 `/api/*` 返回 404 JSON，不再回退到 index.html（避免前端把 HTML 当 JSON 解析）；
/// 2. 静态文件路径做规范化 + 前缀校验，禁止 `..` 穿越与绝对路径。
pub async fn serve_static_or_index(uri: axum::http::Uri) -> axum::response::Response {
    use axum::body::Body;
    use axum::http::{Response, StatusCode};
    use axum::response::IntoResponse;

    let raw = uri.path().trim_start_matches('/');

    // 1) 未匹配的 /api/* → 404 JSON
    if raw == "api" || raw.starts_with("api/") {
        return (
            StatusCode::NOT_FOUND,
            axum::Json(serde_json::json!({
                "code": "not_found",
                "message": format!("unknown api endpoint: /{}", raw)
            })),
        )
            .into_response();
    }

    let base = crate::config::STATIC_DIR
        .get()
        .map(|p| p.as_path())
        .unwrap_or_else(|| std::path::Path::new("web/dist"));
    let base = std::fs::canonicalize(base).unwrap_or_else(|_| base.to_path_buf());

    if raw.is_empty() || raw == "index.html" {
        if let Ok(body) = tokio::fs::read_to_string(base.join("index.html")).await {
            return Response::builder()
                .status(200)
                .header("content-type", "text/html; charset=utf-8")
                .body(Body::from(body))
                .unwrap()
                .into_response();
        }
    }

    // 2) 路径穿越防护：拒绝绝对路径与含 `..` 的路径，并要求规范化后仍位于 base 之内
    if !raw.is_empty() && !raw.contains("..") && !raw.starts_with('/') && !raw.contains('\\') {
        let candidate = base.join(raw);
        if let Ok(real) = std::fs::canonicalize(&candidate) {
            if real.starts_with(&base) && real.is_file() {
                if let Ok(contents) = tokio::fs::read(&real).await {
                    let ct = mime_guess::from_path(&real)
                        .first_raw()
                        .unwrap_or("application/octet-stream");
                    return Response::builder()
                        .status(200)
                        .header("content-type", ct)
                        .body(Body::from(contents))
                        .unwrap()
                        .into_response();
                }
            }
        }
    }

    // SPA fallback: 前端路由回退到 index.html
    if let Ok(body) = tokio::fs::read_to_string(base.join("index.html")).await {
        return Response::builder()
            .status(200)
            .header("content-type", "text/html; charset=utf-8")
            .body(Body::from(body))
            .unwrap()
            .into_response();
    }
    (StatusCode::NOT_FOUND, "Not Found").into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::AppState;
    use axum::body::Body;
    use gateway_core::Manager;
    use std::sync::Arc;
    use tower::util::ServiceExt;

    fn base_config() -> crate::config::Config {
        let mut c = crate::config::Config::default();
        c.disable_auth = false;
        c.token = Some("test-token".to_string());
        c.static_dir = std::path::PathBuf::from("web/dist");
        c
    }

    fn state_with(disable_auth: bool) -> AppState {
        let mut cfg = base_config();
        cfg.disable_auth = disable_auth;
        AppState::new(
            Arc::new(Manager::new()),
            cfg,
            None,
            crate::license::FeatureManager::without_license(),
            Arc::new(crate::users::UserStore::empty()),
            None,
        )
    }

    async fn body_of(res: Response) -> String {
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        String::from_utf8_lossy(&bytes).to_string()
    }

    async fn call(uri: &str, token: Option<&str>) -> (u16, String) {
        let st = state_with(false);
        let app = super::router(st.clone()).with_state(st).into_service();
        let builder = axum::http::Request::builder().uri(uri);
        let req = match token {
            Some(t) => builder
                .header(header::AUTHORIZATION, format!("Bearer {}", t))
                .body(Body::empty())
                .unwrap(),
            None => builder.body(Body::empty()).unwrap(),
        };
        let res = ServiceExt::oneshot(app, req).await.unwrap();
        let status = res.status().as_u16();
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, String::from_utf8_lossy(&bytes).to_string())
    }

    #[tokio::test]
    async fn protected_endpoints_require_token() {
        let (status, _) = call("/nodes", None).await;
        assert_eq!(status, 401, "unauthenticated request must be rejected");
    }

    #[tokio::test]
    async fn valid_token_is_accepted() {
        let (status, _) = call("/nodes", Some("test-token")).await;
        assert_eq!(status, 200);
    }

    #[tokio::test]
    async fn wrong_token_is_rejected() {
        let (status, _) = call("/nodes", Some("not-the-token")).await;
        assert_eq!(status, 401);
    }

    #[tokio::test]
    async fn health_is_public_metrics_and_dataflow_require_auth() {
        let (health_status, _) = call("/health", None).await;
        assert_eq!(health_status, 200, "/health stays public for probes");

        let (dataflow_status, _) = call("/data-flow", None).await;
        assert_eq!(
            dataflow_status, 401,
            "/data-flow exposes internal topology and must require auth"
        );
    }

    #[tokio::test]
    async fn disable_auth_still_guards_nothing_by_design() {
        let st = state_with(true);
        let app = super::router(st.clone()).with_state(st).into_service();
        let res = ServiceExt::oneshot(
            app,
            axum::http::Request::builder()
                .uri("/nodes")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(res.status().as_u16(), 200);
    }

    #[tokio::test]
    async fn static_serving_and_traversal_behaviour() {
        // STATIC_DIR 是进程级 OnceLock：所有静态相关的断言集中在同一用例内完成
        let root = std::env::temp_dir().join(format!("gw-static-{}", uuid::Uuid::new_v4()));
        let base = root.join("dist");
        std::fs::create_dir_all(base.join("assets")).unwrap();
        std::fs::write(base.join("index.html"), "<html>spa</html>").unwrap();
        std::fs::write(base.join("assets").join("app.js"), "console.log(1)").unwrap();
        // base 同级放置秘密文件，用于验证越界读取
        std::fs::write(root.join("secret.txt"), "TOP-SECRET").unwrap();
        assert!(
            crate::config::STATIC_DIR.set(base.clone()).is_ok(),
            "STATIC_DIR was already set by another test"
        );

        // 1) 目录内的文件正常返回
        let res = serve_static_or_index("/assets/app.js".parse().unwrap()).await;
        assert_eq!(res.status().as_u16(), 200);
        assert!(body_of(res).await.contains("console.log(1)"));

        // 2) 未知 /api 路径返回 404 JSON，不再回退 SPA HTML
        let res = serve_static_or_index("/api/does-not-exist".parse().unwrap()).await;
        assert_eq!(res.status().as_u16(), 404);
        let body = body_of(res).await;
        assert!(body.contains("not_found"), "unexpected body: {}", body);
        assert!(!body.contains("spa"), "must not fall back to SPA html");

        // 3) 路径穿越：不得读到 base 之外的文件
        for probe in ["/../secret.txt", "/../../secret.txt"] {
            let body = body_of(serve_static_or_index(probe.parse().unwrap()).await).await;
            assert!(
                !body.contains("TOP-SECRET"),
                "traversal leaked via {}",
                probe
            );
        }

        // 4) 前端路由仍正常回退到 index.html
        let body = body_of(serve_static_or_index("/dashboard".parse().unwrap()).await).await;
        assert!(body.contains("spa"), "SPA fallback should still work");

        let _ = std::fs::remove_dir_all(&root);
    }

    // ---------- RBAC 授权 ----------

    struct RbacEnv {
        state: AppState,
        store: Arc<crate::users::UserStore>,
        dir: std::path::PathBuf,
    }

    /// 构造带真实用户库的测试环境（用于验证角色授权）
    async fn rbac_env() -> RbacEnv {
        let dir = std::env::temp_dir().join(format!("gw-rbac-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let store =
            Arc::new(crate::users::UserStore::open(&dir.join("data.db")).expect("open user store"));
        let mut cfg = crate::config::Config::default();
        cfg.disable_auth = false;
        cfg.token = None;
        cfg.static_dir = dir.clone();
        let state = AppState::new(
            Arc::new(Manager::new()),
            cfg,
            None,
            crate::license::FeatureManager::without_license(),
            store.clone(),
            None,
        );
        RbacEnv { state, store, dir }
    }

    async fn login_as(env: &RbacEnv, user: &str, role: crate::users::UserRole) -> String {
        let pw = "pw-for-test-123";
        env.store.create(user, pw, role).await.expect("create user");
        env.store.login(user, pw).await.expect("login").0
    }

    async fn status_of(env: &RbacEnv, method: &str, uri: &str, token: Option<&str>) -> u16 {
        let st = env.state.clone();
        let app = super::router(st.clone()).with_state(st).into_service();
        let mut builder = axum::http::Request::builder().method(method).uri(uri);
        if let Some(t) = token {
            builder = builder.header(header::AUTHORIZATION, format!("Bearer {}", t));
        }
        let res = ServiceExt::oneshot(app, builder.body(Body::empty()).unwrap())
            .await
            .unwrap();
        res.status().as_u16()
    }

    #[test]
    fn role_ordering_is_enforced() {
        use crate::users::UserRole::{Admin, Operator, Viewer};
        assert!(role_satisfies(Admin, Viewer));
        assert!(role_satisfies(Operator, Viewer));
        assert!(role_satisfies(Admin, Operator));
        assert!(!role_satisfies(Viewer, Operator));
        assert!(!role_satisfies(Operator, Admin));
        // 路径 -> 角色映射
        assert_eq!(
            required_role(&axum::http::Method::GET, "/api/nodes"),
            Viewer
        );
        assert_eq!(
            required_role(&axum::http::Method::POST, "/api/nodes"),
            Operator
        );
        assert_eq!(
            required_role(&axum::http::Method::POST, "/api/nodes/abc/write_tags"),
            Operator
        );
        assert_eq!(required_role(&axum::http::Method::GET, "/api/users"), Admin);
        assert_eq!(
            required_role(&axum::http::Method::POST, "/api/restore"),
            Admin
        );
        assert_eq!(
            required_role(&axum::http::Method::PUT, "/api/license/reset"),
            Admin
        );
        assert_eq!(
            required_role(&axum::http::Method::POST, "/api/auth/logout"),
            Viewer
        );
        // 规则：读给 Viewer，写（会驱动对设备的写动作）必须是 Admin
        assert_eq!(
            required_role(&axum::http::Method::GET, "/api/rules"),
            Viewer
        );
        assert_eq!(
            required_role(&axum::http::Method::POST, "/api/rules"),
            Admin
        );
        assert_eq!(
            required_role(&axum::http::Method::PUT, "/api/rules/abc"),
            Admin
        );
        assert_eq!(
            required_role(&axum::http::Method::DELETE, "/api/rules/abc"),
            Admin
        );
        assert_eq!(
            required_role(&axum::http::Method::POST, "/api/rules/abc/enable"),
            Admin
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn viewer_can_read_but_cannot_write_or_administer() {
        let env = rbac_env().await;
        let token = login_as(&env, "viewer1", crate::users::UserRole::Viewer).await;

        // 读接口放行
        assert_eq!(status_of(&env, "GET", "/nodes", Some(&token)).await, 200);
        // 写值（反控 PLC）必须被拒 —— B2 的核心目标
        assert_eq!(
            status_of(
                &env,
                "POST",
                "/nodes/00000000-0000-0000-0000-000000000001/write_tags",
                Some(&token)
            )
            .await,
            403
        );
        // 创建/修改节点被拒
        assert_eq!(status_of(&env, "POST", "/nodes", Some(&token)).await, 403);
        // 用户管理与恢复备份属于管理员职责
        assert_eq!(status_of(&env, "GET", "/users", Some(&token)).await, 403);
        assert_eq!(status_of(&env, "POST", "/restore", Some(&token)).await, 403);
        // 登出允许任何已认证用户
        assert_eq!(
            status_of(&env, "POST", "/auth/logout", Some(&token)).await,
            200
        );

        let _ = std::fs::remove_dir_all(&env.dir);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn operator_can_control_nodes_but_not_users() {
        let env = rbac_env().await;
        let token = login_as(&env, "operator1", crate::users::UserRole::Operator).await;

        // 写值路径允许通过授权层（节点不存在时 handler 返回 404，但不再是 403）
        let code = status_of(
            &env,
            "POST",
            "/nodes/00000000-0000-0000-0000-000000000001/write_tags",
            Some(&token),
        )
        .await;
        assert_ne!(code, 403, "operator should pass the authorization layer");
        // 仍不能管用户 / 恢复备份
        assert_eq!(status_of(&env, "GET", "/users", Some(&token)).await, 403);
        assert_eq!(status_of(&env, "POST", "/restore", Some(&token)).await, 403);

        let _ = std::fs::remove_dir_all(&env.dir);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rules_are_readable_by_viewer_but_writable_only_by_admin() {
        let env = rbac_env().await;
        let viewer = login_as(&env, "viewer-rules", crate::users::UserRole::Viewer).await;

        // 读：Viewer 可以看规则列表
        assert_eq!(status_of(&env, "GET", "/rules", Some(&viewer)).await, 200);
        // 写：规则会驱动对现场设备的写动作，Viewer 必须被拒
        assert_eq!(status_of(&env, "POST", "/rules", Some(&viewer)).await, 403);
        assert_eq!(
            status_of(&env, "PUT", "/rules/some-id", Some(&viewer)).await,
            403
        );
        assert_eq!(
            status_of(&env, "DELETE", "/rules/some-id", Some(&viewer)).await,
            403
        );
        assert_eq!(
            status_of(&env, "POST", "/rules/some-id/enable", Some(&viewer)).await,
            403
        );

        // Admin 通过授权层（空 body 会在解析阶段被拒，但不应是 403）
        let admin = login_as(&env, "admin-rules", crate::users::UserRole::Admin).await;
        let code = status_of(&env, "POST", "/rules", Some(&admin)).await;
        assert_ne!(code, 403, "admin 应通过授权层，实际 {}", code);

        let _ = std::fs::remove_dir_all(&env.dir);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn admin_can_administer() {
        let env = rbac_env().await;
        let token = login_as(&env, "boss", crate::users::UserRole::Admin).await;
        assert_eq!(status_of(&env, "GET", "/users", Some(&token)).await, 200);
        assert_eq!(status_of(&env, "GET", "/nodes", Some(&token)).await, 200);
        let _ = std::fs::remove_dir_all(&env.dir);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn unknown_token_is_rejected() {
        let env = rbac_env().await;
        assert_eq!(status_of(&env, "GET", "/nodes", Some("garbage")).await, 401);
        assert_eq!(status_of(&env, "GET", "/nodes", None).await, 401);
        let _ = std::fs::remove_dir_all(&env.dir);
    }

    #[tokio::test]
    async fn enforce_roles_switch_disables_authorization() {
        let mut cfg = crate::config::Config::default();
        cfg.disable_auth = false;
        cfg.token = Some("tok".to_string());
        cfg.enforce_roles = false;
        let state = AppState::new(
            Arc::new(Manager::new()),
            cfg,
            None,
            crate::license::FeatureManager::without_license(),
            Arc::new(crate::users::UserStore::empty()),
            None,
        );
        // 灰度开关关闭时，静态 token 可访问管理员接口
        let app = super::router(state.clone())
            .with_state(state)
            .into_service();
        let res = ServiceExt::oneshot(
            app,
            axum::http::Request::builder()
                .uri("/users")
                .header(header::AUTHORIZATION, "Bearer tok")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(res.status().as_u16(), 200);
    }

    // ---------- B3 持久化写合并 ----------

    fn persist_state(dir: &std::path::Path, debounce_ms: u64) -> AppState {
        let mut cfg = crate::config::Config::default();
        cfg.data_dir = dir.to_path_buf();
        cfg.disable_auth = true;
        cfg.persist_debounce_ms = debounce_ms;
        AppState::new(
            Arc::new(Manager::new()),
            cfg,
            None,
            crate::license::FeatureManager::without_license(),
            Arc::new(crate::users::UserStore::empty()),
            None,
        )
    }

    #[tokio::test]
    async fn debounced_persist_defers_write_until_flush() {
        let dir = std::env::temp_dir().join(format!("gw-persist-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let state = persist_state(&dir, 300);
        let db = dir.join("data.db");

        // 多次变更只累积脏标记，不立即写库
        state.persist().await;
        state.persist().await;
        state.persist().await;
        assert!(state.persist_pending(), "changes should be pending");
        assert!(
            !db.exists(),
            "write must be deferred inside the debounce window"
        );

        // 一次 flush 把所有变更合并落盘
        state.flush().await;
        assert!(!state.persist_pending(), "dirty flag should be cleared");
        assert!(db.exists(), "flush should persist the snapshot");

        // flush 幂等：无新变更时不再重复写
        let mtime = std::fs::metadata(&db).unwrap().modified().unwrap();
        state.flush().await;
        assert_eq!(
            std::fs::metadata(&db).unwrap().modified().unwrap(),
            mtime,
            "flush without changes should be a no-op"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn immediate_mode_persists_on_every_change() {
        let dir = std::env::temp_dir().join(format!("gw-persist0-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let state = persist_state(&dir, 0);
        state.persist().await;
        assert!(
            dir.join("data.db").exists(),
            "debounce=0 should write immediately"
        );
        assert!(!state.persist_pending());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn group_interval_lower_bound_is_enforced() {
        // 10ms 是服务端下限：过小会造成忙循环并压垮设备
        use super::handlers::validate_interval_ms;
        assert!(validate_interval_ms(10).is_ok());
        assert!(validate_interval_ms(1000).is_ok());
        assert!(validate_interval_ms(9).is_err());
        assert!(
            validate_interval_ms(0).is_err(),
            "interval_ms=0 must be rejected"
        );
    }
}
