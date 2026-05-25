//! REST API 与静态资源服务。

mod error;
mod handlers;

pub use error::ApiError;

use axum::extract::{Request, State};
use axum::http::header;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use axum::Router;
use axum::http::StatusCode;

use crate::state::AppState;
use crate::flow;

async fn request_id_middleware(request: Request, next: Next) -> Response {
    let id = uuid::Uuid::new_v4().to_string();
    tracing::Span::current().record("request_id", tracing::field::display(&id));
    let mut res = next.run(request).await;
    if let Ok(v) = header::HeaderValue::try_from(id) {
        res.headers_mut().insert(header::HeaderName::from_static("x-request-id"), v);
    }
    res
}

async fn auth_middleware(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    if state.config.disable_auth {
        return next.run(request).await;
    }
    // 嵌套路由中 URI 可能是 /auth/login 或 /api/auth/login，统一处理
    let path = request.uri().path()
        .trim_start_matches("/api/")
        .trim_start_matches("/api")
        .trim_start_matches('/')
        .trim_end_matches('/');
    if path == "health" || path == "metrics" || path == "version" || path == "data-flow"
        || path == "license/machine-id" || path == "license/status"
        || path == "auth/login" || path == "login"
    {
        return next.run(request).await;
    }
    let auth = request.headers().get(header::AUTHORIZATION).and_then(|v| v.to_str().ok());
    let bearer = auth.and_then(|s| s.strip_prefix("Bearer ").map(|t| t.to_string()));
    let ok = match bearer {
        Some(t) if !t.is_empty() => {
            state.config.token.as_ref().map(|c| c.as_str() == t).unwrap_or(false)
                || state.user_store.token_valid(&t)
        }
        _ => {
            let has_users = state.user_store.has_any_user().await.ok() == Some(true);
            state.config.token.is_none() && !has_users
        }
    };
    if ok {
        next.run(request).await
    } else {
        ApiError::unauthorized().into_response()
    }
}

pub fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/auth/login", post(handlers::login))
        .route("/health", get(handlers::health))
        .route("/metrics", get(handlers::metrics))
        .route("/data-flow", get(handlers::data_flow))
        .route("/version", get(handlers::version))
        .route("/dashboard/stats", get(handlers::dashboard_stats))
        .route("/license/machine-id", get(handlers::license_machine_id))
        .route("/license/status", get(handlers::license_status))
        .route("/license/upload", post(handlers::upload_license))
        .route("/license/reset", post(handlers::reset_license))
        .route("/license/pro-tool", get(handlers::license_pro_tool))
        .route("/users", get(handlers::list_users).post(handlers::create_user))
        .route("/users/:id", get(handlers::get_user).put(handlers::update_user).delete(handlers::delete_user))
        .route("/users/:id/password", put(handlers::change_password))
        .route("/backup", post(handlers::backup))
        .route("/restore", post(handlers::restore))
        .route("/plugins/south", get(handlers::list_south_plugins))
        .route("/plugins/south/:name/config_schema", get(handlers::south_plugin_config_schema))
        .route("/plugins/south/:name/tag_schema", get(handlers::south_plugin_tag_schema))
        .route("/plugins/north", get(handlers::list_north_plugins))
        .route("/plugins/north/:name/config_schema", get(handlers::north_plugin_config_schema))
        .route("/plugins", get(handlers::plugins_all))
        .route("/nodes", get(handlers::list_nodes).post(handlers::create_node))
        .route(
            "/nodes/:id",
            get(handlers::get_node).put(handlers::update_node).delete(handlers::delete_node),
        )
        .route("/nodes/:id/start", post(handlers::start_node))
        .route("/nodes/:id/stop", post(handlers::stop_node))
        .route("/nodes/:id/connection-status", get(handlers::get_node_connection_status))
        .route("/nodes/:id/groups", get(handlers::list_groups).post(handlers::add_group))
        .route(
            "/nodes/:id/groups/:gid",
            get(handlers::get_group).put(handlers::update_group).delete(handlers::remove_group),
        )
        .route("/nodes/:id/tags", get(handlers::list_tags).post(handlers::add_tag))
        .route("/nodes/:id/tags/batch", post(handlers::batch_add_tags))
        .route(
            "/nodes/:id/tags/:tid",
            get(handlers::get_tag).put(handlers::update_tag).delete(handlers::remove_tag),
        )
        .route(
            "/nodes/:id/subscriptions",
            get(handlers::get_subscriptions).put(handlers::set_subscriptions),
        )
        .route("/nodes/:id/setting", get(handlers::get_node_setting).put(handlers::node_setting))
        .route("/nodes/:id/read_tags", post(handlers::read_tags))
        .route("/nodes/:id/write_tags", post(handlers::write_tags))
        .route("/upload", post(handlers::upload_config_file))
        .route("/flows", get(flow::handlers::list_flows).post(flow::handlers::create_flow))
        .route("/flows/operators", get(flow::handlers::list_operators))
        .route("/flows/:id", get(flow::handlers::get_flow).put(flow::handlers::update_flow).delete(flow::handlers::delete_flow))
        .route("/flows/:id/deploy", post(flow::handlers::deploy_flow))
        .route("/flows/:id/start", post(flow::handlers::start_flow))
        .route("/flows/:id/pause", post(flow::handlers::pause_flow))
        .route("/flows/:id/stop", post(flow::handlers::stop_flow))
        .route("/flows/:id/metrics", get(flow::handlers::flow_metrics))
        .route("/flows/:id/reload", post(flow::handlers::flow_reload))
        .route("/flows/:id/versions", get(flow::handlers::flow_versions))
        .route("/flows/:id/rollback/:version", post(flow::handlers::flow_rollback))
        .route("/flows/export", get(flow::handlers::export_flows))
        .route("/flows/import", post(flow::handlers::import_flows))
        .route("/flows/:id/live/summary", get(crate::websocket::flow_live_summary))
        // WebSocket routes (outside /api to avoid auth layer)
        .route("/ws/flows/:id/live", axum::routing::get(crate::websocket::ws_flow_live))
        // Admin: SQLite file-level backup/restore
        .route("/admin/sqlite-backup", post(crate::backup::sqlite_backup))
        .route("/admin/sqlite-backup", get(crate::backup::sqlite_download_latest))
        .route("/admin/sqlite-restore", post(crate::backup::sqlite_restore))
        // Catch-all: return 404 for any unmatched API route
        .route("/:path", axum::routing::any(api_not_found))
        .with_state(state.clone())
        .route_layer(middleware::from_fn_with_state(state, auth_middleware))
        .route_layer(middleware::from_fn(request_id_middleware))
}

/// Catch-all for unmatched API routes → 404 JSON.
pub async fn api_not_found() -> impl IntoResponse {
    (StatusCode::NOT_FOUND, r#"{"code":"not_found","message":"endpoint not found"}"#)
}

/// 前端静态资源或 SPA fallback。使用 config::STATIC_DIR，未设置则 `web/dist`。
pub async fn serve_static_or_index(
    uri: axum::http::Uri,
) -> impl axum::response::IntoResponse {
    use axum::body::Body;
    use axum::http::{Response, StatusCode};
    use axum::response::IntoResponse;

    let path = uri.path().trim_start_matches('/');
    let base = crate::config::STATIC_DIR
        .get()
        .map(|p| p.as_path())
        .unwrap_or_else(|| std::path::Path::new("web/dist"));
    if path.is_empty() || path == "index.html" {
        let index = base.join("index.html");
        if index.exists() {
            let body = tokio::fs::read_to_string(index).await.unwrap_or_default();
            return Response::builder()
                .status(200)
                .header("content-type", "text/html; charset=utf-8")
                .body(Body::from(body))
                .unwrap()
                .into_response();
        }
    }
    let file = base.join(path);
    if file.exists() && file.is_file() {
        if let Ok(contents) = tokio::fs::read(&file).await {
            let ct = mime_guess::from_path(&file)
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
    // SPA fallback: 前端路由回退到 index.html
    let index = base.join("index.html");
    if index.exists() {
        if let Ok(body) = tokio::fs::read_to_string(index).await {
            return Response::builder()
                .status(200)
                .header("content-type", "text/html; charset=utf-8")
                .body(Body::from(body))
                .unwrap()
                .into_response();
        }
    }
    (StatusCode::NOT_FOUND, "Not Found").into_response()
}
