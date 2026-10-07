//! WebSocket 端到端集成测试。
//!
//! 实施报告挂账「axum Router + AppState 复杂构造的完整集成测试未做」——本文件补齐：
//! 真实构造 Router + AppState（内置 sim 插件 + 运行中的南向节点），tokio spawn 把
//! axum 起在 127.0.0.1:0 随机端口，走完整协议流：
//!
//! 1. 连接 → 首帧认证（错误 token）→ 断言错误帧 + Close(4001)；
//! 2. 重新连接 → 正确认证 → 断言 auth ok 与 hello 帧结构；
//! 3. subscribe values+nodes → 收到 sim 插件产生的 values 帧与 nodes 快照帧并断言帧结构。

use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, Stream, StreamExt};
use tokio_tungstenite::tungstenite::Message;

use gateway_core::Manager;
use gateway_server::license::FeatureManager;
use gateway_server::state::AppState;
use gateway_server::users::UserStore;

/// 起真实服务：注册内置 sim 插件、创建并启动一个南向节点、Router 挂 /api、监听随机端口。
/// 返回 (ws url, AppState, 节点 id 字符串)。
async fn spawn_server() -> (String, AppState, String) {
    let mut mgr = Manager::new();
    mgr.register_south("sim", Arc::new(plugin_sim::SimPlugin::new()));
    let node = mgr
        .node_create(
            "e2e-sim-node".to_string(),
            gateway_sdk::NodeKind::South,
            "sim".to_string(),
            gateway_sdk::PluginConfig::new(),
        )
        .await
        .expect("create sim node");
    mgr.node_start(node.id()).await.expect("start sim node");
    let node_id = node.id().0.to_string();

    let cfg = gateway_server::config::Config {
        token: Some("test-token".to_string()),
        // nodes 快照按此间隔推送；100ms 保证测试快速收到快照帧
        ws_snapshot_interval_ms: 100,
        ..Default::default()
    };
    let state = AppState::new(
        Arc::new(mgr),
        cfg,
        None,
        FeatureManager::without_license(),
        Arc::new(UserStore::empty()),
        None,
        None,
    );

    // 与 main.rs 一致的路由形态：/api 前缀嵌套（ws 在认证中间件白名单内，认证走首帧）
    let app = axum::Router::new()
        .nest("/api", gateway_server::api::router(state.clone()))
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    (format!("ws://{addr}/api/v1/ws"), state, node_id)
}

/// 读取下一个 Text 帧并解析为 JSON（跳过非文本帧；收到 Close/流结束/错误即失败）
async fn next_text<S>(ws: &mut S) -> serde_json::Value
where
    S: Stream<Item = tokio_tungstenite::tungstenite::Result<Message>> + Unpin,
{
    loop {
        let msg = match tokio::time::timeout(Duration::from_secs(10), ws.next()).await {
            Ok(Some(Ok(m))) => m,
            Ok(Some(Err(e))) => panic!("ws stream error while waiting text frame: {e}"),
            Ok(None) => panic!("ws stream ended while waiting text frame"),
            Err(_) => panic!("timed out waiting for text frame"),
        };
        match msg {
            Message::Text(t) => {
                return serde_json::from_str(&t).expect("server frame must be valid JSON");
            }
            Message::Close(cf) => panic!("unexpected close while waiting text frame: {cf:?}"),
            _ => {}
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ws_e2e_full_protocol() {
    let (url, _state, node_id) = spawn_server().await;

    // ---- 1) 错误 token：应先收到 error 文本帧，随后被 Close(4001) 关闭 ----
    let (mut ws_bad, _) = tokio_tungstenite::connect_async(url.as_str())
        .await
        .unwrap();
    ws_bad
        .send(Message::Text(
            r#"{"type":"auth","token":"Bearer wrong-token"}"#.into(),
        ))
        .await
        .unwrap();

    let mut saw_error_frame = false;
    let mut close_code = None;
    while close_code.is_none() {
        let msg = match tokio::time::timeout(Duration::from_secs(10), ws_bad.next()).await {
            Ok(Some(Ok(m))) => m,
            Ok(Some(Err(e))) => panic!("ws error during auth-fail flow (before close): {e}"),
            Ok(None) => panic!("ws stream ended without Close(4001)"),
            Err(_) => panic!("timed out waiting for Close(4001)"),
        };
        match msg {
            Message::Text(t) => {
                let v: serde_json::Value =
                    serde_json::from_str(&t).expect("error frame must be valid JSON");
                assert_eq!(v["type"], "error", "error frame structure");
                assert!(v["message"].is_string());
                saw_error_frame = true;
            }
            Message::Close(cf) => {
                close_code = cf.map(|c| u16::from(c.code));
            }
            _ => {}
        }
    }
    assert!(
        saw_error_frame,
        "an error frame must precede the 4001 close"
    );
    assert_eq!(
        close_code,
        Some(4001),
        "wrong token must be closed with 4001"
    );

    // ---- 2) 正确认证：先收到 auth ok，再收到 hello ----
    let (mut ws, _) = tokio_tungstenite::connect_async(url.as_str())
        .await
        .unwrap();
    ws.send(Message::Text(
        r#"{"type":"auth","token":"Bearer test-token"}"#.into(),
    ))
    .await
    .unwrap();

    let auth = next_text(&mut ws).await;
    assert_eq!(
        auth["type"], "auth",
        "first server frame must be auth result"
    );
    assert_eq!(auth["success"], true, "static token must authenticate");
    assert!(auth["message"].is_null(), "success auth carries no message");

    let hello = next_text(&mut ws).await;
    assert_eq!(hello["type"], "hello");
    assert!(hello["version"].is_string(), "hello carries version");
    // WsServerFrame::Hello 字段未做 camelCase 重命名，线上即 snake_case
    assert!(hello["build_date"].is_string(), "hello carries build_date");
    let features = hello["features"]
        .as_array()
        .expect("hello carries features");
    assert!(
        features.iter().any(|f| f == "values") && features.iter().any(|f| f == "nodes"),
        "hello features must advertise values+nodes: {features:?}"
    );

    // ---- 3) 订阅 values + nodes ----
    ws.send(Message::Text(
        r#"{"type":"subscribe","topics":["values","nodes"]}"#.into(),
    ))
    .await
    .unwrap();

    // ---- 4) 等待并断言 nodes 快照帧与 sim 插件的 values 帧结构 ----
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    let mut saw_nodes = false;
    let mut saw_values = false;
    while (!saw_nodes || !saw_values) && tokio::time::Instant::now() < deadline {
        let msg = match tokio::time::timeout(Duration::from_secs(5), ws.next()).await {
            Ok(Some(Ok(m))) => m,
            Ok(Some(Err(e))) => {
                panic!("ws error while waiting data frames (nodes={saw_nodes} values={saw_values}): {e}")
            }
            Ok(None) => panic!("ws stream ended before data frames"),
            Err(_) => {
                panic!("timed out waiting for data frames (nodes={saw_nodes} values={saw_values})")
            }
        };
        let v: serde_json::Value = match msg {
            Message::Text(t) => serde_json::from_str(&t).expect("server frame must be valid JSON"),
            Message::Close(cf) => panic!("unexpected close while waiting data frames: {cf:?}"),
            _ => continue,
        };
        match v["type"].as_str() {
            Some("auth") | Some("hello") => {} // 订阅前已消费，防御性跳过
            Some("nodes") => {
                let nodes = v["nodes"].as_array().expect("nodes frame carries array");
                let mine = nodes
                    .iter()
                    .find(|n| n["id"] == node_id.as_str())
                    .unwrap_or_else(|| {
                        panic!("sim node {node_id} missing from snapshot: {nodes:?}")
                    });
                assert_eq!(mine["name"], "e2e-sim-node");
                assert_eq!(mine["pluginName"], "sim");
                assert_eq!(mine["state"], "running", "started node reports running");
                assert_eq!(mine["kind"], "south");
                saw_nodes = true;
            }
            Some("values") => {
                let data = &v["data"];
                assert_eq!(
                    data["nodeId"].as_str(),
                    Some(node_id.as_str()),
                    "values frame is tagged with the node id"
                );
                assert_eq!(data["nodeName"], "e2e-sim-node");
                assert_eq!(data["groupName"], "default", "sim default group");
                assert!(data["groupId"].is_string());
                let values = data["values"].as_array().expect("values carries array");
                assert!(!values.is_empty(), "sim poll must produce tags");
                for tv in values {
                    assert!(tv["tagId"].is_string());
                    assert!(tv["tagName"].is_string());
                    // DataValue 编码与 REST 一致：{"type":"Float64","value":<数值>}（adjacently tagged）
                    assert_eq!(
                        tv["value"]["type"], "Float64",
                        "sim produces float64 values, got: {tv}"
                    );
                    assert!(
                        tv["value"]["value"].is_number(),
                        "sim value must carry a number, got: {tv}"
                    );
                }
                saw_values = true;
            }
            other => panic!("unexpected server frame: {other:?}"),
        }
    }
    assert!(saw_nodes, "should receive a nodes snapshot frame");
    assert!(
        saw_values,
        "should receive values frames from the sim plugin"
    );
}
