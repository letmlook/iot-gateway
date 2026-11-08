//! 流程运行时闭环集成测试（v2.0 T1-④）：
//! 总线 → gateway_flow 求值 → sink_alarm（告警表去重）+ sink_north（桩插件接收），
//! 以及 publish/unpublish 生命周期与告警 API 的角色矩阵。

use axum::body::Body;
use axum::http::{header, Request};
use gateway_flow::FlowPayload;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use tower::ServiceExt;
use uuid::Uuid;

// ---------- 北向桩插件：记录 on_group_data 调用 ----------

#[derive(Default)]
struct RecordingNorth {
    calls: Mutex<Vec<String>>,
}

impl RecordingNorth {
    fn snapshot(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl gateway_sdk::NorthPlugin for RecordingNorth {
    fn meta(&self) -> gateway_sdk::PluginMeta {
        gateway_sdk::PluginMeta {
            name: "stub-north",
            kind: gateway_sdk::PluginKind::North,
            description: Some("test stub"),
            version: "0.1.0",
            name_zh: None,
            name_en: None,
            description_zh: None,
            description_en: None,
        }
    }

    async fn open(
        &self,
        _: gateway_sdk::NodeId,
        _: gateway_sdk::PluginConfig,
    ) -> gateway_sdk::PluginResult<()> {
        Ok(())
    }

    async fn close(&self, _: gateway_sdk::NodeId) -> gateway_sdk::PluginResult<()> {
        Ok(())
    }

    async fn set_subscriptions(
        &self,
        _: gateway_sdk::NodeId,
        _: &[gateway_sdk::GroupSubscription],
    ) -> gateway_sdk::PluginResult<()> {
        Ok(())
    }

    async fn on_group_data(
        &self,
        node_id: gateway_sdk::NodeId,
        data: Arc<gateway_sdk::GroupData>,
    ) -> gateway_sdk::PluginResult<()> {
        let temps: Vec<f64> = data.values.iter().filter_map(|(_, v)| v.as_f64()).collect();
        self.calls
            .lock()
            .unwrap()
            .push(format!("node={} temps={:?}", node_id.0, temps));
        Ok(())
    }
}

// ---------- 环境 ----------

struct RtEnv {
    state: gateway_server::state::AppState,
    store: Arc<gateway_server::users::UserStore>,
    north: Arc<RecordingNorth>,
    north_node_id: Uuid,
    south_node: Uuid,
    group: Uuid,
    dir: std::path::PathBuf,
}

async fn rt_env() -> RtEnv {
    use gateway_core::Manager;
    use gateway_server::license::FeatureManager;
    use gateway_server::state::AppState;
    use gateway_server::users::UserStore;

    let dir = std::env::temp_dir().join(format!("gw-rt-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let db =
        gateway_core::Db::open(&dir.join("data.db"), gateway_core::IntegrityMode::Off).unwrap();
    let user_store = Arc::new(UserStore::open(db.clone(), 0).unwrap());

    let mut mgr = Manager::new();
    let north = Arc::new(RecordingNorth::default());
    mgr.register_north("stub-north", north.clone());
    // 北向节点：sink_north 的投递目标（须 Running）
    let node = mgr
        .node_create(
            "nb-stub".into(),
            gateway_sdk::NodeKind::North,
            "stub-north".into(),
            gateway_sdk::PluginConfig::new(),
        )
        .await
        .expect("create north node");
    mgr.node_start(node.id()).await.expect("start north node");

    let mut cfg = gateway_server::config::Config {
        disable_auth: false,
        token: None,
        static_dir: dir.clone(),
        ..Default::default()
    };
    cfg.disable_auth = false;
    let state = AppState::new(
        Arc::new(mgr),
        cfg,
        None,
        FeatureManager::without_license(),
        user_store.clone(),
        None,
        Some(db),
    );
    user_store
        .create(
            "root",
            "pw-admin-123",
            gateway_server::users::UserRole::Admin,
            "default",
        )
        .await
        .unwrap();
    user_store
        .create(
            "op1",
            "pw-op1-12345",
            gateway_server::users::UserRole::Operator,
            "t1",
        )
        .await
        .unwrap();
    user_store
        .create(
            "view1",
            "pw-view-12345",
            gateway_server::users::UserRole::Viewer,
            "t1",
        )
        .await
        .unwrap();
    RtEnv {
        state,
        store: user_store,
        north,
        north_node_id: node.id().0,
        south_node: Uuid::new_v4(),
        group: Uuid::new_v4(),
        dir,
    }
}

impl RtEnv {
    async fn login(&self, u: &str, p: &str) -> String {
        self.store.login(u, p).await.unwrap().0
    }

    fn insert_flow(&self, name: &str) -> Uuid {
        let flow_id = Uuid::new_v4();
        let row = gateway_server::flows::FlowRow {
            id: flow_id,
            tenant_id: "t1".into(),
            name: name.into(),
            description: None,
            status: "published".into(),
            version: 1,
            graph_json: json!({
                "nodes": [
                    {"id": Uuid::new_v4(), "name": "src", "kind": "source",
                     "config": {"south_node_id": self.south_node, "group_id": self.group}},
                    {"id": Uuid::new_v4(), "name": "th", "kind": "operator", "operator": "threshold_alarm",
                     "config": {"tag_name": "temperature", "high": 80.0, "level": "high"}},
                    {"id": Uuid::new_v4(), "name": "alarm-out", "kind": "sink", "operator": "sink_alarm", "config": {}},
                    {"id": Uuid::new_v4(), "name": "north-out", "kind": "sink", "operator": "sink_north",
                     "config": {"node_id": self.north_node_id}}
                ],
                "edges": [
                    {"source_node_id": Uuid::nil(), "source_port": "", "target_node_id": Uuid::nil()}
                ]
            }),
            created_at: 0,
            updated_at: 0,
        };
        // 用真实 id 重连边
        let mut graph: gateway_flow::FlowGraph =
            serde_json::from_value(row.graph_json.clone()).unwrap();
        let ids: Vec<Uuid> = graph.nodes.iter().map(|n| n.id).collect();
        graph.edges = vec![
            gateway_flow::FlowEdge {
                source_node_id: ids[0],
                source_port: "out".into(),
                target_node_id: ids[1],
            },
            gateway_flow::FlowEdge {
                source_node_id: ids[1],
                source_port: "alarm".into(),
                target_node_id: ids[2],
            },
            gateway_flow::FlowEdge {
                source_node_id: ids[1],
                source_port: "normal".into(),
                target_node_id: ids[3],
            },
        ];
        let row = gateway_server::flows::FlowRow {
            graph_json: serde_json::to_value(&graph).unwrap(),
            ..row
        };
        self.state.flow_store.insert(&row).unwrap();
        flow_id
    }

    fn publish_temperature(&self, v: f64) {
        let tag = gateway_sdk::TagId::new();
        let gd = gateway_sdk::GroupData {
            node_id: gateway_sdk::NodeId(self.south_node),
            group_id: gateway_sdk::GroupId(self.group),
            ts: chrono::Utc::now(),
            values: vec![(tag, gateway_sdk::DataValue::Float64(v))],
            node_name: Some("sim".into()),
            group_name: Some("default".into()),
            tag_names: None,
        };
        // tag_names 缺失时算子按名称找不到值——这里需要名称映射
        let mut gd = gd;
        let mut names = std::collections::HashMap::new();
        names.insert(gd.values[0].0, "temperature".to_string());
        gd.tag_names = Some(names);
        // 无订阅者时 publish 返回 Err(data)——这正是「flow 停止后不再消费」的语义，容忍即可
        let _ = self.state.manager.bus().publish(Arc::new(gd));
    }

    async fn alarm_count(&self) -> (usize, i64) {
        let rows = self
            .state
            .alarm_store
            .list(
                &gateway_server::api::scope::TenantScope::All,
                None,
                None,
                100,
            )
            .unwrap();
        let occ = rows.first().map(|r| r.occurrences).unwrap_or(0);
        (rows.len(), occ)
    }
}

// ---------- 闭环 ----------

#[tokio::test(flavor = "multi_thread")]
async fn closed_loop_bus_to_alarm_and_north() {
    let env = rt_env().await;
    let flow_id = env.insert_flow("overtemp");
    env.state
        .flow_runtime
        .start(
            &env.state,
            env.state
                .flow_store
                .get(flow_id, &gateway_server::api::scope::TenantScope::All)
                .unwrap()
                .unwrap(),
        )
        .await;
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;

    // 高温帧：告警 1 条 + 北向透传 1 帧
    env.publish_temperature(90.0);
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let (rows, occ) = env.alarm_count().await;
    assert_eq!(rows, 1, "应产生 1 条 active 告警");
    assert_eq!(occ, 1);

    // 再次高温：同键 active 去重前移（occurrences 2），不建新行
    env.publish_temperature(95.0);
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let (rows, occ) = env.alarm_count().await;
    assert_eq!(rows, 1, "同键告警必须去重");
    assert_eq!(occ, 2);

    // 北向桩收到 2 帧透传（normal 端）
    assert_eq!(env.north.snapshot().len(), 2, "normal 端透传帧数");

    // 未超阈值帧：无新告警，但北向仍收到
    env.publish_temperature(20.0);
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let (rows, _) = env.alarm_count().await;
    assert_eq!(rows, 1);
    assert_eq!(env.north.snapshot().len(), 3);

    // unpublish（stop）后：不再产生新告警与投递
    env.state.flow_runtime.stop(flow_id).await;
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    env.publish_temperature(99.0);
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let (rows, _) = env.alarm_count().await;
    assert_eq!(rows, 1, "停止后不得产生新告警");
    assert_eq!(env.north.snapshot().len(), 3, "停止后不得再投递");
    let _ = std::fs::remove_dir_all(&env.dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn alarm_store_dedup_and_lifecycle() {
    let env = rt_env().await;
    let ev = gateway_flow::AlarmEvent {
        level: gateway_flow::AlarmLevel::High,
        tag: "temperature".into(),
        value: Some(90.0),
        threshold: Some(80.0),
        message: "overtemp".into(),
        ts: chrono::Utc::now(),
    };
    let flow_id = Uuid::new_v4();
    let id1 = env
        .state
        .alarm_store
        .raise("t1", &flow_id, "alarm-out", &ev)
        .unwrap();
    let id2 = env
        .state
        .alarm_store
        .raise("t1", &flow_id, "alarm-out", &ev)
        .unwrap();
    assert_eq!(id1, id2, "同键 active 去重返回同一事件");

    // 跨租户同键：独立事件
    env.state
        .alarm_store
        .raise("t2", &flow_id, "alarm-out", &ev)
        .unwrap();

    // 生命周期：ack → acknowledged；重复 ack 失败；resolve → resolved；再 resolve 失败
    let all = gateway_server::api::scope::TenantScope::All;
    assert!(env.state.alarm_store.ack(id1, &all, "op1").unwrap());
    assert!(
        !env.state.alarm_store.ack(id1, &all, "op1").unwrap(),
        "ack 不可重复"
    );
    assert!(env.state.alarm_store.resolve(id1, &all).unwrap());
    assert!(
        !env.state.alarm_store.resolve(id1, &all).unwrap(),
        "resolved 是终态"
    );

    // 租户过滤：t1 会话看不到 t2 的事件
    let rows = env
        .state
        .alarm_store
        .list(
            &gateway_server::api::scope::TenantScope::One("t1".into()),
            None,
            None,
            100,
        )
        .unwrap();
    assert!(rows.iter().all(|r| r.tenant_id == "t1"));
    let _ = std::fs::remove_dir_all(&env.dir);
}

// ---------- 告警 API 角色矩阵 ----------

async fn call(env: &RtEnv, method: &str, uri: &str, token: Option<&str>) -> (u16, Value) {
    let app = axum::Router::new()
        .nest("/api", gateway_server::api::router(env.state.clone()))
        .with_state(env.state.clone())
        .into_service();
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(t) = token {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {t}"));
    }
    let req = builder.body(Body::empty()).unwrap();
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

#[tokio::test(flavor = "multi_thread")]
async fn alarms_api_role_matrix() {
    let env = rt_env().await;
    let view1 = env.login("view1", "pw-view-12345").await;
    let op1 = env.login("op1", "pw-op1-12345").await;
    let root = env.login("root", "pw-admin-123").await;

    let ev = gateway_flow::AlarmEvent {
        level: gateway_flow::AlarmLevel::Critical,
        tag: "temperature".into(),
        value: Some(99.0),
        threshold: Some(80.0),
        message: "critical overtemp".into(),
        ts: chrono::Utc::now(),
    };
    let alarm_id = env
        .state
        .alarm_store
        .raise("t1", &Uuid::new_v4(), "alarm-out", &ev)
        .unwrap();

    // Viewer：读 OK、ack 403、resolve 403
    let (status, list) = call(&env, "GET", "/api/alarms", Some(&view1)).await;
    assert_eq!(status, 200);
    assert_eq!(list["alarms"].as_array().unwrap().len(), 1);
    let (status, _) = call(
        &env,
        "POST",
        &format!("/api/alarms/{alarm_id}/ack"),
        Some(&view1),
    )
    .await;
    assert_eq!(status, 403);
    let (status, _) = call(
        &env,
        "POST",
        &format!("/api/alarms/{alarm_id}/resolve"),
        Some(&view1),
    )
    .await;
    assert_eq!(status, 403);

    // Operator：ack OK（记录操作人）、resolve 403
    let (status, acked) = call(
        &env,
        "POST",
        &format!("/api/alarms/{alarm_id}/ack"),
        Some(&op1),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(acked["state"], "acknowledged");
    assert_eq!(acked["acked_by"], "op1");
    let (status, _) = call(
        &env,
        "POST",
        &format!("/api/alarms/{alarm_id}/resolve"),
        Some(&op1),
    )
    .await;
    assert_eq!(status, 403);

    // Admin：resolve OK；再 resolve → 400
    let (status, resolved) = call(
        &env,
        "POST",
        &format!("/api/alarms/{alarm_id}/resolve"),
        Some(&root),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(resolved["state"], "resolved");
    let (status, _) = call(
        &env,
        "POST",
        &format!("/api/alarms/{alarm_id}/resolve"),
        Some(&root),
    )
    .await;
    assert_eq!(status, 400, "resolved 是终态");

    // state 过滤
    let (_, active) = call(&env, "GET", "/api/alarms?state=active", Some(&root)).await;
    assert_eq!(active["alarms"].as_array().unwrap().len(), 0);
    let _ = std::fs::remove_dir_all(&env.dir);
}

// 静态确认：FlowPayload 的 Data/Alarm 双载荷在本 crate 边界可用（防误删导出）
#[test]
fn payload_variants_exist() {
    let ev = gateway_flow::AlarmEvent {
        level: gateway_flow::AlarmLevel::Info,
        tag: "t".into(),
        value: None,
        threshold: None,
        message: "m".into(),
        ts: chrono::Utc::now(),
    };
    let p = FlowPayload::Alarm(ev);
    assert!(matches!(p, FlowPayload::Alarm(_)));
}
