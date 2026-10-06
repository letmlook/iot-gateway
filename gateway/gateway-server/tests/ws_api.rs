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
