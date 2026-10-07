//! WebSocket API integration tests.
//!
//! 验证 WS 帧序列化和基本协议。

use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt as _;
use tokio_tungstenite::tungstenite::Message;

/// 逐帧读取，直到收到 Close(4002, "server shutting down")；其余帧（若有）跳过。
async fn read_until_close_4002<S>(ws: &mut S, label: &str)
where
    S: futures_util::Stream<Item = tokio_tungstenite::tungstenite::Result<Message>> + Unpin,
{
    loop {
        let msg = match tokio::time::timeout(Duration::from_secs(10), ws.next()).await {
            Ok(Some(Ok(m))) => m,
            Ok(Some(Err(e))) => panic!("{label}: ws stream error: {e}"),
            Ok(None) => panic!("{label}: ws stream ended early"),
            Err(_) => panic!("{label}: timed out waiting for Close(4002)"),
        };
        match msg {
            Message::Close(Some(cf)) => {
                assert_eq!(u16::from(cf.code), 4002, "{label}: close code");
                assert_eq!(
                    cf.reason.as_str(),
                    "server shutting down",
                    "{label}: close reason"
                );
                return;
            }
            Message::Close(None) => panic!("{label}: Close frame without code/reason"),
            _ => {}
        }
    }
}

/// 优雅停机：停机广播触发后，所有已连接的 WS 客户端都应收到 Close(4002) 且连接任务退出。
#[tokio::test]
async fn graceful_shutdown_broadcasts_close_4002_to_all_clients() {
    use gateway_core::Manager;
    use gateway_server::license::FeatureManager;
    use gateway_server::state::AppState;
    use gateway_server::users::UserStore;
    use gateway_server::ws::{metric_ws_clients, WS_SHUTDOWN_DRAIN_TIMEOUT};

    let cfg = gateway_server::config::Config {
        token: Some("test-token".to_string()),
        ..Default::default()
    };

    let state = AppState::new(
        Arc::new(Manager::new()),
        cfg,
        None,
        FeatureManager::without_license(),
        Arc::new(UserStore::empty()),
        None,
        None,
    );

    // 生产路由形态（与 main.rs 一致）：/api 前缀嵌套；/api/v1/ws 在认证中间件白名单内，
    // 握手无需 token（认证走首帧）
    let app = axum::Router::new()
        .nest("/api", gateway_server::api::router(state.clone()))
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let url = format!("ws://{addr}/api/v1/ws");
    let (mut c1, _) = tokio_tungstenite::connect_async(url.as_str())
        .await
        .unwrap();
    let (mut c2, _) = tokio_tungstenite::connect_async(url.as_str())
        .await
        .unwrap();

    // 等待两个连接都被服务端登记（handle_socket 已进入并递增计数），消除启动竞态
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while metric_ws_clients() < 2 {
        assert!(
            tokio::time::Instant::now() < deadline,
            "ws connections were not registered in time"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    // 与 main.rs 停机序列第一步一致：停止节点之前先广播停机并排水
    let remaining = state
        .ws_shutdown
        .notify_and_drain(WS_SHUTDOWN_DRAIN_TIMEOUT)
        .await;
    assert_eq!(
        remaining, 0,
        "all ws connection tasks should exit after the shutdown broadcast"
    );
    assert_eq!(
        metric_ws_clients(),
        0,
        "connection counter should return to zero after drain"
    );

    // 两个客户端都必须收到 Close(4002, "server shutting down")
    read_until_close_4002(&mut c1, "client1").await;
    read_until_close_4002(&mut c2, "client2").await;

    // 停机标志已置位后新建的连接（排水窗口内的后来者）也应立即收到 Close(4002)
    let (mut c3, _) = tokio_tungstenite::connect_async(url.as_str())
        .await
        .unwrap();
    read_until_close_4002(&mut c3, "client3-late-joiner").await;
}

#[test]
fn ws_server_frame_serializes_correctly() {
    use gateway_server::ws::WsServerFrame;

    let auth_ok = WsServerFrame::Auth {
        success: true,
        message: None,
    };
    let json = serde_json::to_string(&auth_ok).unwrap();
    assert!(json.contains(r#""type":"auth""#));
    assert!(json.contains(r#""success":true"#));

    let auth_fail = WsServerFrame::Auth {
        success: false,
        message: Some("bad token".into()),
    };
    let json = serde_json::to_string(&auth_fail).unwrap();
    assert!(json.contains(r#""success":false"#));
    assert!(json.contains("bad token"));

    let ping = WsServerFrame::Ping;
    let json = serde_json::to_string(&ping).unwrap();
    assert!(json.contains(r#""type":"ping""#));

    let hello = WsServerFrame::Hello {
        version: "1.0.0".to_string(),
        build_date: "2026-10-06".to_string(),
        features: vec!["values".to_string(), "nodes".to_string()],
    };
    let json = serde_json::to_string(&hello).unwrap();
    assert!(json.contains(r#""type":"hello""#));
    assert!(json.contains("1.0.0"));
}

#[test]
fn ws_client_frame_deserializes_correctly() {
    use gateway_server::ws::WsClientFrame;

    let auth: WsClientFrame =
        serde_json::from_str(r#"{ "type": "auth", "token": "Bearer xxx" }"#).unwrap();
    match auth {
        WsClientFrame::Auth { token } => assert_eq!(token, Some("Bearer xxx".into())),
        _ => panic!("expected Auth variant"),
    }

    let subscribe: WsClientFrame =
        serde_json::from_str(r#"{ "type": "subscribe", "topics": ["values", "nodes"] }"#).unwrap();
    match subscribe {
        WsClientFrame::Subscribe {
            topics,
            node_ids,
            group_ids,
        } => {
            assert_eq!(topics, vec!["values", "nodes"]);
            assert!(node_ids.is_none());
            assert!(group_ids.is_none());
        }
        _ => panic!("expected Subscribe variant"),
    }

    let subscribe_with_filter: WsClientFrame = serde_json::from_str(
        r#"{ "type": "subscribe", "topics": ["values"], "nodeIds": ["n1"], "groupIds": ["g1"] }"#,
    )
    .unwrap();
    match subscribe_with_filter {
        WsClientFrame::Subscribe {
            topics,
            node_ids,
            group_ids,
        } => {
            assert_eq!(topics, vec!["values"]);
            assert_eq!(node_ids, Some(vec!["n1".to_string()]));
            assert_eq!(group_ids, Some(vec!["g1".to_string()]));
        }
        _ => panic!("expected Subscribe variant"),
    }

    let ping: WsClientFrame = serde_json::from_str(r#"{ "type": "ping" }"#).unwrap();
    match ping {
        WsClientFrame::Ping => {}
        _ => panic!("expected Ping variant"),
    }
}

#[test]
fn ws_topics_bitflags() {
    use gateway_server::ws::Topics;

    let empty = Topics::empty();
    assert!(!empty.contains(Topics::VALUES));
    assert!(!empty.contains(Topics::NODES));

    let with_values = Topics::from_strs(["values"].into_iter());
    assert!(with_values.contains(Topics::VALUES));
    assert!(!with_values.contains(Topics::NODES));

    let combined = Topics::from_strs(["values", "nodes"].into_iter());
    assert!(combined.contains(Topics::VALUES));
    assert!(combined.contains(Topics::NODES));

    let unknown = Topics::from_strs(["unknown-topic"].into_iter());
    assert!(!unknown.contains(Topics::VALUES));
}

#[test]
fn ws_metrics_functions_work() {
    use gateway_server::ws::{metric_ws_clients, metric_ws_frames_dropped, metric_ws_frames_sent};

    // 初始值应为 0
    assert_eq!(metric_ws_clients(), 0);
    assert_eq!(metric_ws_frames_sent(), 0);
    assert_eq!(metric_ws_frames_dropped(), 0);
}
