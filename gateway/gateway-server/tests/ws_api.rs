//! WebSocket API integration tests.
//!
//! 验证 WS 帧序列化和基本协议。

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

    let node_vals = WsServerFrame::NodeValues {
        data: serde_json::json!([{"tagId":"t1","value":42.0}]),
    };
    let json = serde_json::to_string(&node_vals).unwrap();
    assert!(json.contains(r#""type":"node-values""#));
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

    let subscribe: WsClientFrame = serde_json::from_str(
        r#"{ "type": "subscribe", "topics": ["node-values", "group-values"] }"#,
    )
    .unwrap();
    match subscribe {
        WsClientFrame::Subscribe { topics } => {
            assert_eq!(topics, vec!["node-values", "group-values"])
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
fn ws_broadcast_functions_work() {
    use gateway_core::Manager;
    use gateway_server::config::Config;
    use gateway_server::license::FeatureManager;
    use gateway_server::users::UserStore;
    use std::sync::Arc;

    let cfg = Config::default();
    let state = gateway_server::state::AppState::new(
        Arc::new(Manager::with_limits(16, 4)),
        cfg,
        None,
        FeatureManager::without_license(),
        Arc::new(UserStore::empty()),
        None,
        None,
    );

    // broadcast 函数调用不 panic 即通过（无订阅者时直接丢弃）
    let data = serde_json::json!([{ "tagId": "t1", "value": 1.0 }]);
    gateway_server::ws::broadcast_node_values(&state, data.clone());
    gateway_server::ws::broadcast_group_values(&state, data.clone());
    gateway_server::ws::broadcast_system_metrics(&state, data.clone());
}

#[test]
fn ws_topics_bitflags() {
    use gateway_server::ws::Topics;

    let empty = Topics::empty();
    assert!(!empty.contains(Topics::NODE_VALUES));

    let with_node = Topics::from_strs(["node-values"].into_iter());
    assert!(with_node.contains(Topics::NODE_VALUES));
    assert!(!with_node.contains(Topics::GROUP_VALUES));

    let combined = Topics::from_strs(["node-values", "group-values", "system-metrics"].into_iter());
    assert!(combined.contains(Topics::NODE_VALUES));
    assert!(combined.contains(Topics::GROUP_VALUES));
    assert!(combined.contains(Topics::SYSTEM_METRICS));

    let unknown = Topics::from_strs(["unknown-topic"].into_iter());
    assert!(!unknown.contains(Topics::NODE_VALUES));
}
