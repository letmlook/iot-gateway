//! gateway-flow：流程编排 v2 的纯模型/校验/计划/求值层。
//!
//! 设计：`docs/design/流程编排-v2.md`。crate 无 tokio、无 I/O——订阅、任务编排与
//! sink 投递（北向插件 / 告警存储）在 gateway-server 接线层完成；本 crate 可独立单测。

pub mod engine;
pub mod error;
pub mod flow;
pub mod operators;
pub mod payload;
pub mod plan;
pub mod validate;

pub use engine::{evaluate, evaluate_with_inject, FlowState, SinkDelivery};
pub use error::FlowError;
pub use flow::{Flow, FlowEdge, FlowGraph, FlowNode, FlowStatus, NodeKind, Position};
pub use operators::{validate_config, CachedScript, ExprRouteConfig, OPERATOR_NAMES};
pub use payload::{AlarmEvent, AlarmLevel, FlowPayload};
pub use plan::ExecutionPlan;
pub use validate::validate;

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use gateway_sdk::{DataValue, GroupData, GroupId, NodeId, TagId};
    use serde_json::json;
    use std::collections::HashMap;
    use uuid::Uuid;

    // ---------- 构造助手 ----------

    fn node(
        name: &str,
        kind: NodeKind,
        operator: Option<&str>,
        config: serde_json::Value,
    ) -> FlowNode {
        FlowNode {
            id: Uuid::new_v4(),
            name: name.into(),
            kind,
            operator: operator.map(String::from),
            config,
            position: Some(Position { x: 0.0, y: 0.0 }),
        }
    }

    fn edge(src: &FlowNode, port: &str, tgt: &FlowNode) -> FlowEdge {
        FlowEdge {
            source_node_id: src.id,
            source_port: port.into(),
            target_node_id: tgt.id,
        }
    }

    fn group_data(temp: f64, switch: Option<&str>) -> GroupData {
        let mut tag_names = HashMap::new();
        let mut values = Vec::new();
        let t1 = TagId::new();
        tag_names.insert(t1, "temperature".to_string());
        values.push((t1, DataValue::Float64(temp)));
        if let Some(s) = switch {
            let t2 = TagId::new();
            tag_names.insert(t2, "switch".to_string());
            values.push((t2, DataValue::String(s.to_string())));
        }
        GroupData {
            node_id: NodeId::new(),
            group_id: GroupId::new(),
            ts: Utc::now(),
            values,
            node_name: Some("sim".into()),
            group_name: Some("default".into()),
            tag_names: Some(tag_names),
        }
    }

    fn source_config() -> serde_json::Value {
        json!({"south_node_id": Uuid::new_v4(), "group_id": Uuid::new_v4()})
    }

    fn alarm_graph() -> (Flow, FlowNode, FlowNode) {
        // source → threshold_alarm(temperature > 80) → { sink_alarm(alarm), sink_north(normal) }
        let src = node("src", NodeKind::Source, None, source_config());
        let th = node(
            "th",
            NodeKind::Operator,
            Some("threshold_alarm"),
            json!({"tag_name":"temperature","high":80.0,"level":"high"}),
        );
        let sa = node("alarm-sink", NodeKind::Sink, Some("sink_alarm"), json!({}));
        let sn = node(
            "north-sink",
            NodeKind::Sink,
            Some("sink_north"),
            json!({"node_id": Uuid::nil()}),
        );
        // sink_north 校验要求非 nil node_id，测试里用随机 id
        let sn = FlowNode {
            config: json!({"node_id": Uuid::new_v4()}),
            ..sn
        };
        let mut flow = Flow {
            id: Uuid::new_v4(),
            tenant_id: "default".into(),
            name: "t".into(),
            description: None,
            status: FlowStatus::Draft,
            version: 0,
            graph: FlowGraph {
                nodes: vec![src.clone(), th.clone(), sa.clone(), sn.clone()],
                edges: vec![
                    edge(&src, "out", &th),
                    edge(&th, "alarm", &sa),
                    edge(&th, "normal", &sn),
                ],
            },
            created_at: None,
            updated_at: None,
        };
        flow.status = FlowStatus::Published;
        (flow, sa, sn)
    }

    // ---------- P1/P4 对策：graph_json 往返（含 position、单文档） ----------

    #[test]
    fn flow_json_round_trips_with_position() {
        let (flow, _, _) = alarm_graph();
        let s = serde_json::to_string(&flow).expect("serialize");
        let back: Flow = serde_json::from_str(&s).expect("deserialize");
        assert_eq!(back.name, flow.name);
        assert_eq!(back.graph.nodes.len(), 4);
        assert_eq!(back.graph.edges.len(), 3);
        for (a, b) in back.graph.nodes.iter().zip(&flow.graph.nodes) {
            assert_eq!(a.id, b.id);
            assert_eq!(
                a.position, b.position,
                "position 必须住在 graph_json 里往返"
            );
            assert_eq!(a.operator, b.operator);
            assert_eq!(a.config, b.config);
        }
    }

    // ---------- 分层与环检测 ----------

    #[test]
    fn plan_builds_parallel_tiers() {
        // source → route → {sink_t, sink_f}：3 层
        let src = node("src", NodeKind::Source, None, json!({}));
        let rt = node(
            "rt",
            NodeKind::Operator,
            Some("route"),
            json!({"tag_name":"temperature","op":"gt","value":50.0}),
        );
        let s1 = node(
            "s1",
            NodeKind::Sink,
            Some("sink_north"),
            json!({"node_id": Uuid::new_v4()}),
        );
        let s2 = node(
            "s2",
            NodeKind::Sink,
            Some("sink_north"),
            json!({"node_id": Uuid::new_v4()}),
        );
        let flow = Flow {
            graph: FlowGraph {
                nodes: vec![src.clone(), rt.clone(), s1.clone(), s2.clone()],
                edges: vec![
                    edge(&src, "out", &rt),
                    edge(&rt, "true", &s1),
                    edge(&rt, "false", &s2),
                ],
            },
            ..alarm_graph().0
        };
        let plan = ExecutionPlan::build(&flow).expect("plan");
        assert_eq!(plan.tiers.len(), 3, "source / route / sinks 三层");
        assert_eq!(plan.tiers[0].len(), 1);
        assert_eq!(plan.tiers[1].len(), 1);
        assert_eq!(plan.tiers[2].len(), 2, "两个 sink 同层可并行");
    }

    #[test]
    fn plan_detects_cycle() {
        let a = node(
            "a",
            NodeKind::Operator,
            Some("route"),
            json!({"tag_name":"x","op":"gt","value":0.0}),
        );
        let b = node(
            "b",
            NodeKind::Operator,
            Some("route"),
            json!({"tag_name":"x","op":"gt","value":0.0}),
        );
        let src = node("src", NodeKind::Source, None, json!({}));
        let sink = node("s", NodeKind::Sink, Some("sink_alarm"), json!({}));
        let mut flow = Flow {
            graph: FlowGraph {
                nodes: vec![src.clone(), a.clone(), b.clone(), sink.clone()],
                edges: vec![
                    edge(&src, "out", &a),
                    edge(&a, "true", &b),
                    edge(&b, "true", &a), // 环
                    edge(&a, "false", &sink),
                ],
            },
            ..alarm_graph().0
        };
        flow.graph.edges[0].source_port = "out".into();
        let errs = validate(&flow);
        assert!(
            errs.iter().any(|e| matches!(e, FlowError::Cycle)),
            "环必须被报出: {errs:?}"
        );
        assert!(ExecutionPlan::build(&flow).is_err());
    }

    // ---------- publish 门禁（P2 对策：校验只在 publish 强制） ----------

    #[test]
    fn validate_catches_common_broken_graphs() {
        // 空图
        let mut flow = alarm_graph().0;
        flow.graph = FlowGraph::default();
        assert!(!validate(&flow).is_empty());

        // 缺 sink + 未知算子 + 缺输入
        let src = node("src", NodeKind::Source, None, json!({}));
        let bad = node("bad", NodeKind::Operator, Some("no_such_op"), json!({}));
        let th = node(
            "th",
            NodeKind::Operator,
            Some("threshold_alarm"),
            json!({"tag_name":"temperature","level":"high"}), // 缺 high/low
        );
        let mut flow = alarm_graph().0;
        flow.graph = FlowGraph {
            nodes: vec![src, bad, th],
            edges: vec![],
        };
        let errs = validate(&flow);
        assert!(
            errs.len() >= 4,
            "缺 sink/未知算子/缺配置字段/缺输入都应报出: {errs:?}"
        );

        // 修好配置与连线后应通过
        let sa = node("sa", NodeKind::Sink, Some("sink_alarm"), json!({}));
        let th_ok = node(
            "th",
            NodeKind::Operator,
            Some("threshold_alarm"),
            json!({"tag_name":"temperature","high":80.0,"level":"high"}),
        );
        flow.graph = FlowGraph {
            nodes: vec![
                node("src", NodeKind::Source, None, source_config()),
                th_ok,
                sa,
            ],
            edges: vec![FlowEdge {
                source_node_id: flow.graph.nodes[0].id,
                source_port: "out".into(),
                target_node_id: uuid::Uuid::new_v4(),
            }],
        };
        // 上面边指向随机 id 应报缺失；重建正确边
        let (src2, th2, sa2) = (
            flow.graph.nodes[0].clone(),
            flow.graph.nodes[1].clone(),
            flow.graph.nodes[2].clone(),
        );
        flow.graph.edges = vec![edge(&src2, "out", &th2), edge(&th2, "alarm", &sa2)];
        assert!(
            validate(&flow).is_empty(),
            "合法图应零错误: {:?}",
            validate(&flow)
        );
    }

    // ---------- 引擎端到端 ----------

    #[test]
    fn threshold_hit_routes_alarm_and_passthrough() {
        let (flow, sa, sn) = alarm_graph();
        let plan = ExecutionPlan::build(&flow).expect("plan");
        let mut state = FlowState::default();

        // 命中：temperature=85 > 80
        let outs = evaluate(&plan, &flow, &group_data(85.0, None), &mut state).expect("eval");
        assert_eq!(outs.len(), 2, "alarm + north 两个 sink 都有投递");
        let to_alarm = outs
            .iter()
            .find(|d| d.node_id == sa.id)
            .expect("alarm delivery");
        let to_north = outs
            .iter()
            .find(|d| d.node_id == sn.id)
            .expect("north delivery");
        match &to_alarm.payload {
            FlowPayload::Alarm(e) => {
                assert_eq!(e.tag, "temperature");
                assert_eq!(e.value, Some(85.0));
                assert_eq!(e.threshold, Some(80.0));
                assert_eq!(e.level, AlarmLevel::High);
            }
            other => panic!("alarm sink 应收到 Alarm 载荷，实际 {other:?}"),
        }
        match &to_north.payload {
            FlowPayload::Data(gd) => assert_eq!(gd.values.len(), 1, "normal 端透传原始数据"),
            other => panic!("north sink 应收到 Data 载荷，实际 {other:?}"),
        }

        // 未命中：只有 north sink 收到透传
        let outs = evaluate(&plan, &flow, &group_data(25.0, None), &mut state).expect("eval");
        assert_eq!(outs.len(), 1);
        assert!(matches!(outs[0].payload, FlowPayload::Data(_)));
    }

    #[test]
    fn state_change_triggers_on_transition_only() {
        let src = node("src", NodeKind::Source, None, json!({}));
        let sc = node(
            "sc",
            NodeKind::Operator,
            Some("state_change_alarm"),
            json!({"tag_name":"switch","from":"0","to":"1","level":"medium"}),
        );
        let sa = node("sa", NodeKind::Sink, Some("sink_alarm"), json!({}));
        let sn = node(
            "sn",
            NodeKind::Sink,
            Some("sink_north"),
            json!({"node_id": Uuid::new_v4()}),
        );
        let flow = Flow {
            graph: FlowGraph {
                nodes: vec![src.clone(), sc.clone(), sa.clone(), sn.clone()],
                edges: vec![
                    edge(&src, "out", &sc),
                    edge(&sc, "alarm", &sa),
                    edge(&sc, "normal", &sn),
                ],
            },
            ..alarm_graph().0
        };
        let plan = ExecutionPlan::build(&flow).expect("plan");
        let mut state = FlowState::default();

        // 首帧 = 基线：无上一态，不触发（仅记录 last）
        let outs = evaluate(&plan, &flow, &group_data(0.0, Some("0")), &mut state).unwrap();
        assert!(
            !outs.iter().any(|d| d.node_id == sa.id),
            "首帧仅建立基线不应触发"
        );
        // 0→1：触发
        let outs = evaluate(&plan, &flow, &group_data(0.0, Some("1")), &mut state).unwrap();
        assert!(outs.iter().any(|d| d.node_id == sa.id), "0→1 应触发告警");
        // 1→1：不触发
        let outs = evaluate(&plan, &flow, &group_data(0.0, Some("1")), &mut state).unwrap();
        assert!(!outs.iter().any(|d| d.node_id == sa.id), "同态重复不应触发");
        // 1→0：from=0 不匹配 → 不触发
        let outs = evaluate(&plan, &flow, &group_data(0.0, Some("0")), &mut state).unwrap();
        assert!(
            !outs.iter().any(|d| d.node_id == sa.id),
            "1→0 与 from=0 不符不应触发"
        );
        // 0→1：再次触发
        let outs = evaluate(&plan, &flow, &group_data(0.0, Some("1")), &mut state).unwrap();
        assert!(outs.iter().any(|d| d.node_id == sa.id));
    }

    #[test]
    fn route_dual_port_and_missing_value_goes_false() {
        let src = node("src", NodeKind::Source, None, json!({}));
        let rt = node(
            "rt",
            NodeKind::Operator,
            Some("route"),
            json!({"tag_name":"temperature","op":"gt","value":50.0}),
        );
        let hot = node(
            "hot",
            NodeKind::Sink,
            Some("sink_north"),
            json!({"node_id": Uuid::new_v4()}),
        );
        let cold = node(
            "cold",
            NodeKind::Sink,
            Some("sink_north"),
            json!({"node_id": Uuid::new_v4()}),
        );
        let flow = Flow {
            graph: FlowGraph {
                nodes: vec![src.clone(), rt.clone(), hot.clone(), cold.clone()],
                edges: vec![
                    edge(&src, "out", &rt),
                    edge(&rt, "true", &hot),
                    edge(&rt, "false", &cold),
                ],
            },
            ..alarm_graph().0
        };
        let plan = ExecutionPlan::build(&flow).expect("plan");
        let mut state = FlowState::default();

        let outs = evaluate(&plan, &flow, &group_data(60.0, None), &mut state).unwrap();
        assert_eq!(outs.len(), 1);
        assert_eq!(outs[0].node_id, hot.id, "60>50 走 true 端");

        let outs = evaluate(&plan, &flow, &group_data(10.0, None), &mut state).unwrap();
        assert_eq!(outs.len(), 1);
        assert_eq!(outs[0].node_id, cold.id, "10≤50 走 false 端");

        // 缺值（switch-only 帧没有 temperature）→ false 端，不误触发
        let only_switch = group_data(0.0, Some("x"));
        let outs = evaluate(&plan, &flow, &only_switch, &mut state).unwrap();
        assert_eq!(outs.len(), 1);
        assert_eq!(outs[0].node_id, cold.id, "缺值必须走 false 端");
    }

    #[test]
    fn operator_config_validation_messages() {
        assert!(
            validate_config("threshold_alarm", &json!({"tag_name":"t","level":"high"})).is_err(),
            "缺 high/low 应报错"
        );
        assert!(
            validate_config(
                "threshold_alarm",
                &json!({"tag_name":"t","high":1.0,"level":"pink"})
            )
            .is_err(),
            "未知级别应报错"
        );
        assert!(
            validate_config("route", &json!({"tag_name":"t","op":">>","value":1.0})).is_err(),
            "未知比较符应报错"
        );
        assert!(
            validate_config("sink_north", &json!({"node_id": Uuid::nil()})).is_err(),
            "nil 北向节点应报错"
        );
        assert!(validate_config("sink_alarm", &json!({})).is_ok());
        assert!(validate_config(
            "threshold_alarm",
            &json!({"tag_name":"t","high":1.0,"low":0.0,"level":"critical"})
        )
        .is_ok());
    }

    // ---------- rate_of_change ----------

    /// 固定时间戳的 group_data（rate 测试用；temperature 一路 + 可选 switch）
    fn group_data_ts(temp: f64, ts_ms: i64) -> GroupData {
        let mut gd = group_data(temp, None);
        gd.ts = Utc.timestamp_millis_opt(ts_ms).unwrap();
        gd
    }

    fn rate_graph() -> (Flow, FlowNode, FlowNode) {
        // source → rate_of_change(temperature, max 1.0/s) → { sink_alarm(alarm), sink_north(normal) }
        let src = node("src", NodeKind::Source, None, source_config());
        let rc = node(
            "rc",
            NodeKind::Operator,
            Some("rate_of_change"),
            json!({"tag_name":"temperature","max_rate":1.0,"level":"medium"}),
        );
        let sa = node("alarm-sink", NodeKind::Sink, Some("sink_alarm"), json!({}));
        let sn = node(
            "north-sink",
            NodeKind::Sink,
            Some("sink_north"),
            json!({"node_id": Uuid::new_v4()}),
        );
        let flow = Flow {
            graph: FlowGraph {
                nodes: vec![src.clone(), rc.clone(), sa.clone(), sn.clone()],
                edges: vec![
                    edge(&src, "out", &rc),
                    edge(&rc, "alarm", &sa),
                    edge(&rc, "normal", &sn),
                ],
            },
            ..alarm_graph().0
        };
        (flow, sa, sn)
    }

    #[test]
    fn rate_first_sighting_never_triggers() {
        let (flow, sa, _sn) = rate_graph();
        let plan = ExecutionPlan::build(&flow).expect("plan");
        let mut state = FlowState::default();
        let outs = evaluate(&plan, &flow, &group_data_ts(10.0, 1_000), &mut state).unwrap();
        assert!(!outs.iter().any(|d| d.node_id == sa.id), "首见只建基线");
    }

    #[test]
    fn rate_over_limit_triggers_both_directions() {
        let (flow, sa, _sn) = rate_graph();
        let plan = ExecutionPlan::build(&flow).expect("plan");
        let mut state = FlowState::default();
        // 基线 10.0 @1s
        evaluate(&plan, &flow, &group_data_ts(10.0, 1_000), &mut state).unwrap();
        // 上升：+5 in 2s = 2.5/s > 1.0 → 触发
        let outs = evaluate(&plan, &flow, &group_data_ts(15.0, 3_000), &mut state).unwrap();
        let hit = outs
            .iter()
            .find(|d| d.node_id == sa.id)
            .expect("上升超速应触发");
        match &hit.payload {
            FlowPayload::Alarm(e) => {
                assert_eq!(e.tag, "temperature");
                assert_eq!(e.value, Some(15.0));
                assert_eq!(e.threshold, Some(1.0));
                assert_eq!(e.level, AlarmLevel::Medium);
                assert!(
                    e.message.contains("rate=2.500/s"),
                    "默认消息含速率: {}",
                    e.message
                );
            }
            other => panic!("alarm 端应带 Alarm，实际 {other:?}"),
        }
        // 基线更新为 15.0 @3s；下降：−4 in 1s = 4/s → |rate| 触发
        let outs = evaluate(&plan, &flow, &group_data_ts(11.0, 4_000), &mut state).unwrap();
        assert!(outs.iter().any(|d| d.node_id == sa.id), "下降超速也应触发");
    }

    #[test]
    fn rate_within_limit_passes_normal() {
        let (flow, sa, sn) = rate_graph();
        let plan = ExecutionPlan::build(&flow).expect("plan");
        let mut state = FlowState::default();
        evaluate(&plan, &flow, &group_data_ts(10.0, 1_000), &mut state).unwrap();
        // +1 in 2s = 0.5/s ≤ 1.0 → 不触发，normal 端透传
        let outs = evaluate(&plan, &flow, &group_data_ts(11.0, 3_000), &mut state).unwrap();
        assert!(!outs.iter().any(|d| d.node_id == sa.id));
        assert!(outs.iter().any(|d| d.node_id == sn.id), "normal 端仍透传");
    }

    #[test]
    fn rate_missing_value_goes_normal_and_keeps_state() {
        let (flow, sa, _sn) = rate_graph();
        let plan = ExecutionPlan::build(&flow).expect("plan");
        let mut state = FlowState::default();
        evaluate(&plan, &flow, &group_data_ts(10.0, 1_000), &mut state).unwrap();
        // 帧2：switch-only 帧（无 temperature）→ normal 端、不触发
        let mut gd = group_data(0.0, Some("1"));
        gd.values.retain(|(tid, _)| {
            gd.tag_names.as_ref().unwrap().get(tid).map(String::as_str) != Some("temperature")
        });
        let outs = evaluate(&plan, &flow, &gd, &mut state).unwrap();
        assert!(!outs.iter().any(|d| d.node_id == sa.id), "缺值不触发");
        // 帧3：temperature 恢复 @2s，相对**帧1基线**算速率（缺值帧未更新状态）
        // (12−10)/1s = 2/s > 1.0 → 触发
        let outs = evaluate(&plan, &flow, &group_data_ts(12.0, 2_000), &mut state).unwrap();
        assert!(
            outs.iter().any(|d| d.node_id == sa.id),
            "缺值帧不应更新基线"
        );
    }

    #[test]
    fn rate_same_timestamp_is_zero_rate() {
        let (flow, sa, _sn) = rate_graph();
        let plan = ExecutionPlan::build(&flow).expect("plan");
        let mut state = FlowState::default();
        evaluate(&plan, &flow, &group_data_ts(10.0, 1_000), &mut state).unwrap();
        // 同 ts 值跳变：速率按 0 处理，不触发
        let outs = evaluate(&plan, &flow, &group_data_ts(99.0, 1_000), &mut state).unwrap();
        assert!(
            !outs.iter().any(|d| d.node_id == sa.id),
            "同 ts 不除零不触发"
        );
    }

    #[test]
    fn rate_validate_rejects_bad_config() {
        assert!(validate_config(
            "rate_of_change",
            &json!({"tag_name":"t","max_rate":-1.0,"level":"high"})
        )
        .is_err());
        assert!(validate_config(
            "rate_of_change",
            &json!({"tag_name":"","max_rate":1.0,"level":"high"})
        )
        .is_err());
        assert!(validate_config(
            "rate_of_change",
            &json!({"tag_name":"t","max_rate":f64::NAN,"level":"high"})
        )
        .is_err());
        assert!(validate_config(
            "rate_of_change",
            &json!({"tag_name":"t","max_rate":1.0,"level":"nope"})
        )
        .is_err());
        assert!(validate_config(
            "rate_of_change",
            &json!({"tag_name":"t","max_rate":1.0,"level":"high"})
        )
        .is_ok());
    }

    // ---------- deduplicate ----------

    fn dedup_graph() -> (Flow, FlowNode) {
        // source → deduplicate → sink_north
        let src = node("src", NodeKind::Source, None, source_config());
        let dd = node(
            "dd",
            NodeKind::Operator,
            Some("deduplicate"),
            json!({}),
        );
        let sn = node(
            "north-sink",
            NodeKind::Sink,
            Some("sink_north"),
            json!({"node_id": Uuid::new_v4()}),
        );
        let flow = Flow {
            graph: FlowGraph {
                nodes: vec![src.clone(), dd.clone(), sn.clone()],
                edges: vec![edge(&src, "out", &dd), edge(&dd, "out", &sn)],
            },
            ..alarm_graph().0
        };
        (flow, sn)
    }

    #[test]
    fn dedup_first_frame_forwards_then_identical_suppressed() {
        let (flow, _sn) = dedup_graph();
        let plan = ExecutionPlan::build(&flow).expect("plan");
        let mut state = FlowState::default();
        // 首帧转发
        let gd1 = group_data(10.0, None);
        let outs = evaluate(&plan, &flow, &gd1, &mut state).unwrap();
        assert_eq!(outs.len(), 1, "首帧应转发");
        // 完全相同帧（值同）抑制——ts 变化不影响比较（比的是 values）。
        // group_data 每次调用生成新 TagId，而逐位比较含 TagId，
        // 故复用首帧（仅换 ts）构造真正逐位相同的 values。
        let mut gd2 = gd1.clone();
        gd2.ts = Utc::now();
        let outs = evaluate(&plan, &flow, &gd2, &mut state).unwrap();
        assert_eq!(outs.len(), 0, "重复帧应被抑制");
        // 值变化转发
        let outs = evaluate(&plan, &flow, &group_data(11.0, None), &mut state).unwrap();
        assert_eq!(outs.len(), 1, "值变化应转发");
    }

    #[test]
    fn dedup_frame_shape_change_forwards() {
        let (flow, _sn) = dedup_graph();
        let plan = ExecutionPlan::build(&flow).expect("plan");
        let mut state = FlowState::default();
        evaluate(&plan, &flow, &group_data(10.0, None), &mut state).unwrap();
        // 多一路 switch 点位：长度变化 → 转发
        let outs = evaluate(&plan, &flow, &group_data(10.0, Some("1")), &mut state).unwrap();
        assert_eq!(outs.len(), 1, "点位数变化应转发");
    }

    #[test]
    fn dedup_nan_frame_always_forwards() {
        let (flow, _sn) = dedup_graph();
        let plan = ExecutionPlan::build(&flow).expect("plan");
        let mut state = FlowState::default();
        let gd1 = group_data(f64::NAN, None);
        let outs = evaluate(&plan, &flow, &gd1, &mut state).unwrap();
        assert_eq!(outs.len(), 1);
        // 同 TagId、逐位同帧仅值同为 NaN：NaN ≠ NaN（PartialEq 不特判）→ 仍每帧转发
        let gd2 = gd1.clone();
        let outs = evaluate(&plan, &flow, &gd2, &mut state).unwrap();
        assert_eq!(outs.len(), 1, "NaN ≠ NaN：连续 NaN 帧每帧转发");
    }

    #[test]
    fn dedup_validate_accepts_empty_config() {
        assert!(validate_config("deduplicate", &json!({})).is_ok());
    }

    // ---------- 断流告警注入（spec §4.3） ----------

    #[test]
    fn inject_reaches_alarm_to_data_and_sink_alarm_only() {
        // source → threshold → { alarm→sink_alarm, normal→sink_north }，旁挂 alarm_to_data → sink_north2
        let (flow, sa, sn) = alarm_graph();
        let a2d = node(
            "a2d",
            NodeKind::Operator,
            Some("alarm_to_data"),
            json!({}),
        );
        let sn2 = node(
            "north-sink-2",
            NodeKind::Sink,
            Some("sink_north"),
            json!({"node_id": Uuid::new_v4()}),
        );
        let mut flow = flow;
        flow.graph.nodes.push(a2d.clone());
        flow.graph.nodes.push(sn2.clone());
        flow.graph.edges.push(edge(&flow.graph.nodes[0], "out", &a2d));
        flow.graph.edges.push(edge(&a2d, "out", &sn2));
        let plan = ExecutionPlan::build(&flow).expect("plan");
        let mut state = FlowState::default();

        let ev = AlarmEvent {
            level: AlarmLevel::High,
            tag: "grp1".into(),
            value: None,
            threshold: Some(5000.0),
            message: "组 grp1 超过 5 秒未上报数据".into(),
            ts: Utc::now(),
        };
        let outs =
            evaluate_with_inject(&plan, &flow, &group_data(0.0, None), &mut state, Some(&ev))
                .expect("inject eval");

        // sink_alarm 收到 Alarm(ev)
        let to_alarm = outs.iter().find(|d| d.node_id == sa.id).expect("sink_alarm 应收到断流告警");
        match &to_alarm.payload {
            FlowPayload::Alarm(e) => assert_eq!(e.tag, "grp1"),
            other => panic!("sink_alarm 应收 Alarm，实际 {other:?}"),
        }
        // alarm_to_data 的下游 sink_north2 收到 Data（伪源 alarms/events）
        let to_north2 = outs.iter().find(|d| d.node_id == sn2.id).expect("alarm_to_data 下游应收到");
        match &to_north2.payload {
            FlowPayload::Data(gd) => {
                assert_eq!(gd.node_name.as_deref(), Some("alarms"));
            }
            other => panic!("a2d 下游应收 Data，实际 {other:?}"),
        }
        // 注入不进入 threshold 分支：原 sink_north（threshold normal 端）无投递
        assert!(!outs.iter().any(|d| d.node_id == sn.id), "注入模式下普通算子分支静默");
    }

    #[test]
    fn inject_none_matches_plain_evaluate() {
        let (flow, _sa, _sn) = alarm_graph();
        let plan = ExecutionPlan::build(&flow).expect("plan");
        let mut state = FlowState::default();
        let a = evaluate(&plan, &flow, &group_data(85.0, None), &mut state).unwrap();
        let b = evaluate_with_inject(&plan, &flow, &group_data(85.0, None), &mut state, None).unwrap();
        assert_eq!(a.len(), b.len());
        assert_eq!(
            a.iter().map(|d| d.node_id).collect::<Vec<_>>(),
            b.iter().map(|d| d.node_id).collect::<Vec<_>>()
        );
    }

    #[test]
    fn validate_source_stale_config_and_alarm_exit() {
        // stale_timeout_ms > 0 且 level 非法 → 校验报错
        let src_bad = node(
            "src",
            NodeKind::Source,
            None,
            json!({"south_node_id": Uuid::new_v4(), "group_id": Uuid::new_v4(),
                   "stale_timeout_ms": 5000, "stale_level": "nope"}),
        );
        let sa = node("sa", NodeKind::Sink, Some("sink_alarm"), json!({}));
        let mut flow = alarm_graph().0;
        flow.graph = FlowGraph {
            nodes: vec![src_bad.clone(), sa.clone()],
            edges: vec![edge(&src_bad, "out", &sa)],
        };
        let errs = validate(&flow);
        assert!(errs.iter().any(|e| e.to_string().contains("level")), "{errs:?}");

        // level 合法但图中无 sink_alarm/alarm_to_data → 拒绝发布
        let src_ok = node(
            "src",
            NodeKind::Source,
            None,
            json!({"south_node_id": Uuid::new_v4(), "group_id": Uuid::new_v4(),
                   "stale_timeout_ms": 5000, "stale_level": "high"}),
        );
        let sn = node(
            "sn",
            NodeKind::Sink,
            Some("sink_north"),
            json!({"node_id": Uuid::new_v4()}),
        );
        flow.graph = FlowGraph {
            nodes: vec![src_ok.clone(), sn.clone()],
            edges: vec![edge(&src_ok, "out", &sn)],
        };
        let errs = validate(&flow);
        assert!(
            errs.iter().any(|e| e.to_string().contains("sink_alarm/alarm_to_data")),
            "断流告警无出口应报错: {errs:?}"
        );

        // 接上 sink_alarm 后通过；stale_timeout_ms=0 时不要求出口
        let sa = node("sa", NodeKind::Sink, Some("sink_alarm"), json!({}));
        flow.graph.nodes.push(sa.clone());
        flow.graph.edges.push(edge(&src_ok, "out", &sa));
        assert!(validate(&flow).is_empty(), "{:?}", validate(&flow));
        // 保留原 id 换配置（struct update）：stale 关闭后无需告警出口仍应通过
        let src_off = FlowNode {
            config: json!({"south_node_id": Uuid::new_v4(), "group_id": Uuid::new_v4()}),
            ..flow.graph.nodes[0].clone()
        };
        flow.graph.nodes[0] = src_off;
        assert!(validate(&flow).is_empty(), "{:?}", validate(&flow));
    }
}
