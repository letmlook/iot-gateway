//! 流程编排 API 集成测试（v2.0 T1-③）。
//!
//! 锁定的契约（design docs/design/流程编排-v2.md §5）：
//! - **P1 对策**：create → get 完整往返图（nodes/edges/config 不被丢弃）；
//! - **P2 对策**：draft 保存允许任意图，publish 才强制校验（环/缺 sink/配置非法 → 400）；
//! - 角色矩阵：读 Viewer+ / 保存 Operator+ / 发布·下线·删除 Admin；
//! - 租户隔离：跨租户 get 404、list 不可见（防存在性探测）；Admin（All 域）全量可见。

use axum::body::Body;
use axum::http::{header, Request};
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

struct FlowsEnv {
    state: gateway_server::state::AppState,
    store: Arc<gateway_server::users::UserStore>,
    dir: std::path::PathBuf,
}

async fn flows_env() -> FlowsEnv {
    use gateway_core::Manager;
    use gateway_server::license::FeatureManager;
    use gateway_server::state::AppState;
    use gateway_server::users::UserStore;

    let dir = std::env::temp_dir().join(format!("gw-flows-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let db =
        gateway_core::Db::open(&dir.join("data.db"), gateway_core::IntegrityMode::Off).unwrap();
    let store = Arc::new(UserStore::open(db.clone(), 0).unwrap());
    let cfg = gateway_server::config::Config {
        disable_auth: false,
        token: None,
        static_dir: dir.clone(),
        ..Default::default()
    };
    let state = AppState::new(
        Arc::new(Manager::new()),
        cfg,
        None,
        FeatureManager::without_license(),
        store.clone(),
        None,
        Some(db),
    );
    // 用户矩阵：root=default Admin（All 域）；op1/view1=t1；op2=t2
    store
        .create(
            "root",
            "pw-admin-123",
            gateway_server::users::UserRole::Admin,
            "default",
        )
        .await
        .unwrap();
    store
        .create(
            "op1",
            "pw-op1-12345",
            gateway_server::users::UserRole::Operator,
            "t1",
        )
        .await
        .unwrap();
    store
        .create(
            "view1",
            "pw-view-12345",
            gateway_server::users::UserRole::Viewer,
            "t1",
        )
        .await
        .unwrap();
    store
        .create(
            "op2",
            "pw-op2-12345",
            gateway_server::users::UserRole::Operator,
            "t2",
        )
        .await
        .unwrap();
    FlowsEnv { state, store, dir }
}

async fn login(env: &FlowsEnv, username: &str, password: &str) -> String {
    env.store.login(username, password).await.unwrap().0
}

/// oneshot 打生产同款路由（/api 前缀嵌套），uri 与真实请求一致
async fn call(
    env: &FlowsEnv,
    method: &str,
    uri: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> (u16, Value) {
    let app = axum::Router::new()
        .nest("/api", gateway_server::api::router(env.state.clone()))
        .with_state(env.state.clone())
        .into_service();
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(t) = token {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {t}"));
    }
    let req = match body {
        Some(v) => builder
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(v.to_string()))
            .unwrap(),
        None => builder.body(Body::empty()).unwrap(),
    };
    let res = ServiceExt::oneshot(app, req).await.unwrap();
    let status = res.status().as_u16();
    let bytes = axum::body::to_bytes(res.into_body(), 1_000_000)
        .await
        .unwrap();
    let json = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, json)
}

/// 合法三节点图：source → threshold_alarm(temperature>80) → sink_alarm
fn valid_graph_body(name: &str) -> Value {
    let src = Uuid::new_v4();
    let th = Uuid::new_v4();
    let sa = Uuid::new_v4();
    json!({
        "name": name,
        "nodes": [
            {"id": src, "name": "src", "kind": "source", "config": {}, "position": {"x": 0.0, "y": 0.0}},
            {"id": th, "name": "th", "kind": "operator", "operator": "threshold_alarm",
             "config": {"tag_name": "temperature", "high": 80.0, "level": "high"},
             "position": {"x": 100.0, "y": 0.0}},
            {"id": sa, "name": "sa", "kind": "sink", "operator": "sink_alarm", "config": {},
             "position": {"x": 200.0, "y": 0.0}}
        ],
        "edges": [
            {"source_node_id": src, "source_port": "out", "target_node_id": th},
            {"source_node_id": th, "source_port": "alarm", "target_node_id": sa}
        ]
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn create_get_round_trips_full_graph() {
    let env = flows_env().await;
    let op1 = login(&env, "op1", "pw-op1-12345").await;

    let body = valid_graph_body("overtemp");
    let (status, created) = call(&env, "POST", "/api/flows", Some(&op1), Some(body.clone())).await;
    assert_eq!(status, 200, "create 应成功: {created}");
    assert_eq!(created["status"], "draft");
    assert_eq!(created["version"], 0);

    let id = created["id"].as_str().unwrap().to_string();
    let (status, got) = call(&env, "GET", &format!("/api/flows/{id}"), Some(&op1), None).await;
    assert_eq!(status, 200);
    // P1 对策：nodes/edges/config/position 一一往返，不得丢失
    assert_eq!(got["graph"]["nodes"].as_array().unwrap().len(), 3);
    assert_eq!(got["graph"]["edges"].as_array().unwrap().len(), 2);
    assert_eq!(
        got["graph"]["nodes"][1]["config"]["tag_name"],
        json!("temperature")
    );
    assert_eq!(got["graph"]["nodes"][0]["position"]["x"], json!(0.0));
    let _ = std::fs::remove_dir_all(&env.dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn publish_validates_and_bumps_version() {
    let env = flows_env().await;
    let op1 = login(&env, "op1", "pw-op1-12345").await;
    let root = login(&env, "root", "pw-admin-123").await;

    // Operator 不能发布
    let (_, created) = call(
        &env,
        "POST",
        "/api/flows",
        Some(&op1),
        Some(valid_graph_body("p1")),
    )
    .await;
    let id = created["id"].as_str().unwrap().to_string();
    let (status, _) = call(
        &env,
        "POST",
        &format!("/api/flows/{id}/publish"),
        Some(&op1),
        None,
    )
    .await;
    assert_eq!(status, 403, "Operator 发布应 403");

    // Admin 发布合法图 → version 0→1，status=published
    let (status, published) = call(
        &env,
        "POST",
        &format!("/api/flows/{id}/publish"),
        Some(&root),
        None,
    )
    .await;
    assert_eq!(status, 200, "Admin 发布应成功: {published}");
    assert_eq!(published["status"], "published");
    assert_eq!(published["version"], 1);

    // 非法图（缺 sink、缺输入）发布 → 400 且带校验信息
    let bad = json!({
        "name": "broken",
        "nodes": [
            {"id": Uuid::new_v4(), "name": "src", "kind": "source", "config": {}}
        ],
        "edges": []
    });
    let (_, created) = call(&env, "POST", "/api/flows", Some(&op1), Some(bad)).await;
    let bad_id = created["id"].as_str().unwrap().to_string();
    let (status, err) = call(
        &env,
        "POST",
        &format!("/api/flows/{bad_id}/publish"),
        Some(&root),
        None,
    )
    .await;
    assert_eq!(status, 400, "缺 sink 的图发布应 400: {err}");
    assert!(
        err["message"]
            .as_str()
            .unwrap_or("")
            .contains("validation failed"),
        "错误信息应包含校验详情: {err}"
    );

    // unpublish → 回草稿，version 不变
    let (status, unpublished) = call(
        &env,
        "POST",
        &format!("/api/flows/{id}/unpublish"),
        Some(&root),
        None,
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(unpublished["status"], "draft");
    assert_eq!(unpublished["version"], 1, "unpublish 不递增版本");
    let _ = std::fs::remove_dir_all(&env.dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn tenant_isolation_and_admin_visibility() {
    let env = flows_env().await;
    let op1 = login(&env, "op1", "pw-op1-12345").await;
    let op2 = login(&env, "op2", "pw-op2-12345").await;
    let root = login(&env, "root", "pw-admin-123").await;

    let (_, created) = call(
        &env,
        "POST",
        "/api/flows",
        Some(&op1),
        Some(valid_graph_body("t1flow")),
    )
    .await;
    let id = created["id"].as_str().unwrap().to_string();

    // 跨租户 get → 404（防存在性探测）；list → 不可见
    let (status, _) = call(&env, "GET", &format!("/api/flows/{id}"), Some(&op2), None).await;
    assert_eq!(status, 404, "跨租户 get 必须 404");
    let (_, list) = call(&env, "GET", "/api/flows", Some(&op2), None).await;
    assert_eq!(
        list["flows"].as_array().unwrap().len(),
        0,
        "t2 不可见 t1 流程"
    );

    // op2 建自己的流程后，Admin（All 域）全量可见
    call(
        &env,
        "POST",
        "/api/flows",
        Some(&op2),
        Some(valid_graph_body("t2flow")),
    )
    .await;
    let (_, admin_list) = call(&env, "GET", "/api/flows", Some(&root), None).await;
    assert_eq!(
        admin_list["flows"].as_array().unwrap().len(),
        2,
        "Admin 应见两租户流程"
    );

    // 跨租户删除：DELETE 是 Admin-only，op2 先被角色挡下（403）；跨租户的「存在性不可探测」
    // 由 GET 404 保证（上条断言）。无论如何数据不丢：
    let (status, _) = call(
        &env,
        "DELETE",
        &format!("/api/flows/{id}"),
        Some(&op2),
        None,
    )
    .await;
    assert_eq!(status, 403, "Operator（含跨租户）删除应被角色矩阵 403");
    let (status, _) = call(&env, "GET", &format!("/api/flows/{id}"), Some(&op1), None).await;
    assert_eq!(status, 200, "跨租户删除不得误删");
    let _ = std::fs::remove_dir_all(&env.dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn role_matrix_and_name_uniqueness() {
    let env = flows_env().await;
    let op1 = login(&env, "op1", "pw-op1-12345").await;
    let view1 = login(&env, "view1", "pw-view-12345").await;
    let root = login(&env, "root", "pw-admin-123").await;

    // Viewer：读 OK、写 403
    let (status, _) = call(&env, "GET", "/api/flows", Some(&view1), None).await;
    assert_eq!(status, 200);
    let (status, _) = call(
        &env,
        "POST",
        "/api/flows",
        Some(&view1),
        Some(valid_graph_body("v1")),
    )
    .await;
    assert_eq!(status, 403, "Viewer 创建应 403");

    // 算子发现任何已认证用户可读
    let (status, ops) = call(&env, "GET", "/api/flows/operators", Some(&view1), None).await;
    assert_eq!(status, 200);
    assert_eq!(
        ops["operators"].as_array().unwrap().len(),
        5,
        "MVP 算子 5 个"
    );

    // 同租户重名 → 400
    call(
        &env,
        "POST",
        "/api/flows",
        Some(&op1),
        Some(valid_graph_body("dup")),
    )
    .await;
    let (status, err) = call(
        &env,
        "POST",
        "/api/flows",
        Some(&op1),
        Some(valid_graph_body("dup")),
    )
    .await;
    assert_eq!(status, 400);
    assert!(err["message"]
        .as_str()
        .unwrap_or("")
        .contains("already exists"));

    // Operator 删除 → 403；Admin 删除 → 204 且 get 404
    let (_, created) = call(
        &env,
        "POST",
        "/api/flows",
        Some(&op1),
        Some(valid_graph_body("del")),
    )
    .await;
    let id = created["id"].as_str().unwrap().to_string();
    let (status, _) = call(
        &env,
        "DELETE",
        &format!("/api/flows/{id}"),
        Some(&op1),
        None,
    )
    .await;
    assert_eq!(status, 403, "Operator 删除应 403");
    let (status, _) = call(
        &env,
        "DELETE",
        &format!("/api/flows/{id}"),
        Some(&root),
        None,
    )
    .await;
    assert_eq!(status, 204);
    let (status, _) = call(&env, "GET", &format!("/api/flows/{id}"), Some(&op1), None).await;
    assert_eq!(status, 404);
    let _ = std::fs::remove_dir_all(&env.dir);
}
