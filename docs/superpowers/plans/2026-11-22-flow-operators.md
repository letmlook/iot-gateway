# 流算子（rate_of_change / deduplicate / 断流告警）实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** gateway-flow 新增 rate_of_change 与 deduplicate 两个算子，并为流程引擎补上断流检测（source watchdog + 告警注入图）。

**Architecture:** 两个算子沿用现有「config struct + validate_config + eval_* + evaluate_node 分派」模式；断流告警在 flow_runtime 消费循环用 `tokio::select!` 超时检测，超时事件经 `evaluate_with_inject` 注入图（alarm_to_data / sink_alarm 收到，其他算子静默）。纯函数层（gateway-flow）与接线层（gateway-server）职责边界不变。

**Tech Stack:** Rust（gateway-flow 纯函数 crate、gateway-server tokio 接线层）、Vue 3 + Element Plus（流程编辑器表单）。

**Spec:** `docs/superpowers/specs/2026-11-22-flow-operators-design.md`

## Global Constraints

- gateway-flow crate 保持纯函数（无 tokio / 无 I/O）；运行态（FlowState）不落盘、重启清零。
- 缺测不误触发：缺值/非数值点位走 normal / false 端（route 既有口径）；rate_of_change 缺值还**不更新状态**。
- `evaluate` 现有签名**保留不变**（lib.rs 10+ 测试调用点零改动）；注入能力以新增平级函数 `evaluate_with_inject` 提供——与 spec §4.3 "签名扩展"的字面偏差，理由：API 兼容、diff 最小（controller 已裁定）。
- publish 门禁（断流告警必须有投递出口）落在 `gateway-flow/src/validate.rs` 的 `validate()` 内——与 spec §五 "server 接线层"的字面偏差，理由：校验集中在 crate，server publish 流程自动受益（controller 已裁定）。
- 每任务跑完对应包的测试全绿才 commit：Task 1-3 `cargo test -p gateway-flow`，Task 4 `cargo test -p gateway-server`，Task 5 `cd web && npm run test:unit && npm run build`。
- 注释风格与现有文件一致（`//!` 模块头 + 中文行注释，引用 spec 章节号）。

---

### Task 1: rate_of_change 算子（gateway-flow）

**Files:**
- Modify: `gateway/gateway-flow/src/operators.rs`（配置结构、validate、求值、分派、PortSpec、OPERATOR_NAMES）
- Modify: `gateway/gateway-flow/src/engine.rs:17-20`（FlowState 加 last_rate 字段；evaluate_node 调用点改传 &mut FlowState）
- Modify: `gateway/gateway-flow/src/operators.rs:359-390`（evaluate_node 签名从 `&mut HashMap<Uuid, HashMap<String, String>>` 改为 `state: &mut crate::engine::FlowState`；eval_state_change 调用改传 `&mut state.last_values`）
- Test: `gateway/gateway-flow/src/lib.rs`（tests 模块内追加）

**Interfaces:**
- Consumes: `value_by_name`（operators.rs 既有）、`AlarmEvent/AlarmLevel`（payload.rs 既有）、`FlowState`（engine.rs :17）
- Produces: `RateOfChangeConfig`、`eval_rate_of_change`；`FlowState.last_rate: HashMap<Uuid, HashMap<String, (f64, u64)>>`；`evaluate_node` 新签名 `(&str, Uuid, &Value, FlowPayload, &mut FlowState)`（Task 2/3 复用）

- [ ] **Step 1: 写失败测试**（lib.rs tests 模块尾部追加）

```rust
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
        let hit = outs.iter().find(|d| d.node_id == sa.id).expect("上升超速应触发");
        match &hit.payload {
            FlowPayload::Alarm(e) => {
                assert_eq!(e.tag, "temperature");
                assert_eq!(e.value, Some(15.0));
                assert_eq!(e.threshold, Some(1.0));
                assert_eq!(e.level, AlarmLevel::Medium);
                assert!(e.message.contains("rate=2.500/s"), "默认消息含速率: {}", e.message);
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
        assert!(outs.iter().any(|d| d.node_id == sa.id), "缺值帧不应更新基线");
    }

    #[test]
    fn rate_same_timestamp_is_zero_rate() {
        let (flow, sa, _sn) = rate_graph();
        let plan = ExecutionPlan::build(&flow).expect("plan");
        let mut state = FlowState::default();
        evaluate(&plan, &flow, &group_data_ts(10.0, 1_000), &mut state).unwrap();
        // 同 ts 值跳变：速率按 0 处理，不触发
        let outs = evaluate(&plan, &flow, &group_data_ts(99.0, 1_000), &mut state).unwrap();
        assert!(!outs.iter().any(|d| d.node_id == sa.id), "同 ts 不除零不触发");
    }

    #[test]
    fn rate_validate_rejects_bad_config() {
        assert!(validate_config("rate_of_change", &json!({"tag_name":"t","max_rate":-1.0,"level":"high"})).is_err());
        assert!(validate_config("rate_of_change", &json!({"tag_name":"","max_rate":1.0,"level":"high"})).is_err());
        assert!(validate_config("rate_of_change", &json!({"tag_name":"t","max_rate":f64::NAN,"level":"high"})).is_err());
        assert!(validate_config("rate_of_change", &json!({"tag_name":"t","max_rate":1.0,"level":"nope"})).is_err());
        assert!(validate_config("rate_of_change", &json!({"tag_name":"t","max_rate":1.0,"level":"high"})).is_ok());
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p gateway-flow`
Expected: 编译失败（`rate_of_change` 未知算子 / `FlowState` 无 `last_rate`）

- [ ] **Step 3: 实现**（operators.rs + engine.rs）

operators.rs 头部（:15-22）`OPERATOR_NAMES` 改为：

```rust
pub const OPERATOR_NAMES: [&str; 7] = [
    "threshold_alarm",
    "state_change_alarm",
    "route",
    "alarm_to_data",
    "sink_north",
    "sink_alarm",
    "rate_of_change",
];
```

`PortSpec::operator`（:52-73）match 加分支（放在 `"route"` 分支后）：

```rust
            "rate_of_change" => PortSpec {
                inputs: &["in"],
                outputs: &["alarm", "normal"],
            },
```

`validate_config`（:87-137）`"route"` 分支后加：

```rust
        "rate_of_change" => {
            let c: RateOfChangeConfig = serde_json::from_value(config.clone())
                .map_err(|e| format!("invalid config: {e}"))?;
            if c.tag_name.is_empty() {
                return Err("tag_name is required".into());
            }
            if !(c.max_rate.is_finite() && c.max_rate > 0.0) {
                return Err("max_rate must be a finite positive number".into());
            }
            AlarmLevel::parse(&c.level)?;
            Ok(())
        }
```

配置结构区（`RouteConfig` 之后）加：

```rust
#[derive(Debug, Deserialize)]
pub struct RateOfChangeConfig {
    pub tag_name: String,
    /// 速率上限（正值，单位 = 值/秒）
    pub max_rate: f64,
    pub level: String,
    #[serde(default)]
    pub message: Option<String>,
}
```

求值区（`eval_route` 之后）加：

```rust
/// rate_of_change 求值（spec §二）。`state`：flow 节点 id → tag_name → (上一值, 上一 ts 毫秒)。
/// 缺值/非数值走 normal 端且不更新状态（缺测不误触发）；首见只建基线；
/// 同时间戳视速率 0（防除零）；每次有效读数都更新基线。
pub fn eval_rate_of_change(
    cfg: &RateOfChangeConfig,
    node_id: Uuid,
    state: &mut HashMap<Uuid, HashMap<String, (f64, u64)>>,
    gd: &GroupData,
) -> (Option<FlowPayload>, Option<FlowPayload>) {
    let now_ms = gd.ts.timestamp_millis().max(0) as u64;
    let Some(v) = value_by_name(gd, &cfg.tag_name).and_then(|v| v.as_f64()) else {
        return (None, Some(FlowPayload::Data(gd.clone())));
    };
    let node_state = state.entry(node_id).or_default();
    let rate = match node_state.get(&cfg.tag_name) {
        None => None,
        Some(&(pv, pts)) => {
            if now_ms == pts {
                Some(0.0)
            } else {
                Some((v - pv) / ((now_ms - pts) as f64 / 1000.0))
            }
        }
    };
    node_state.insert(cfg.tag_name.clone(), (v, now_ms));
    let over = rate.map(|r| r.abs() > cfg.max_rate).unwrap_or(false);
    if !over {
        return (None, Some(FlowPayload::Data(gd.clone())));
    }
    let r = rate.unwrap_or(0.0);
    let event = AlarmEvent {
        level: AlarmLevel::parse(&cfg.level).unwrap_or(AlarmLevel::Info),
        tag: cfg.tag_name.clone(),
        value: Some(v),
        threshold: Some(cfg.max_rate),
        message: cfg.message.clone().unwrap_or_else(|| {
            format!(
                "{}={} rate={:.3}/s crossed max {}/s",
                cfg.tag_name, v, r, cfg.max_rate
            )
        }),
        ts: Utc::now(),
    };
    (
        Some(FlowPayload::Alarm(event)),
        Some(FlowPayload::Data(gd.clone())),
    )
}
```

engine.rs `FlowState`（:17-20）改为：

```rust
/// flow 节点运行态（state_change 上一态 / rate 上一读数）。不落盘，重启清零。
#[derive(Debug, Default)]
pub struct FlowState {
    pub last_values: HashMap<Uuid, HashMap<String, String>>,
    /// rate_of_change 基线：节点 id → tag_name → (上一值, 上一 ts 毫秒)
    pub last_rate: HashMap<Uuid, HashMap<String, (f64, u64)>>,
}
```

operators.rs `evaluate_node`（:360-366 签名与 :407 state_change 调用处）：签名最后一个参数从 `state: &mut HashMap<Uuid, HashMap<String, String>>` 改为 `state: &mut crate::engine::FlowState`；`eval_state_change(&cfg, node_id, state, &gd)` 改为 `eval_state_change(&cfg, node_id, &mut state.last_values, &gd)`；文件头 `use` 无需变（用全路径）。末尾 match 加：

```rust
        "rate_of_change" => {
            let cfg: RateOfChangeConfig = serde_json::from_value(config.clone())
                .map_err(|e| FlowError::Operator(operator.into(), e.to_string()))?;
            let (alarm, normal) =
                eval_rate_of_change(&cfg, node_id, &mut state.last_rate, &gd);
            let mut out = Vec::new();
            if let Some(a) = alarm {
                out.push(("alarm", a));
            }
            if let Some(n) = normal {
                out.push(("normal", n));
            }
            Ok(out)
        }
```

operators.rs tests 里 `evaluate_node_accepts_alarm_input_only`（:506-539）的 `let mut state = HashMap::new();` 改为 `let mut state = crate::engine::FlowState::default();`。

engine.rs evaluate_operator 调用处（:84-90）改：

```rust
                    let outs = crate::operators::evaluate_node(
                        &op,
                        node_id,
                        &node.config,
                        payload,
                        state,
                    )?;
```

（`state` 已经是 `&mut FlowState`，直接传 re-borrow。）

lib.rs tests 模块若尚无 `use chrono::TimeZone;`（`group_data_ts` 的 `timestamp_millis_opt` 需要 trait 在 scope），按现有 use 风格补上。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p gateway-flow`
Expected: 全部 PASS（含新增 6 个 rate 测试与既有测试）

- [ ] **Step 5: Commit**

```bash
git add gateway/gateway-flow
git commit -m "feat(flow): rate_of_change 算子——变化速率超限告警（单差分/缺测不误触发）"
```

---

### Task 2: deduplicate 算子（gateway-flow）

**Files:**
- Modify: `gateway/gateway-flow/src/operators.rs`（配置、validate、求值、分派、PortSpec、OPERATOR_NAMES 7→8）
- Modify: `gateway/gateway-flow/src/engine.rs`（FlowState 加 last_frames 字段）
- Test: `gateway/gateway-flow/src/lib.rs`

**Interfaces:**
- Consumes: `FlowState`（Task 1 新签名 evaluate_node）
- Produces: `eval_deduplicate`；`FlowState.last_frames: HashMap<Uuid, Vec<(TagId, DataValue)>>`

- [ ] **Step 1: 写失败测试**（lib.rs tests 尾部追加）

```rust
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
        let (flow, sn) = dedup_graph();
        let plan = ExecutionPlan::build(&flow).expect("plan");
        let mut state = FlowState::default();
        // 首帧转发
        let outs = evaluate(&plan, &flow, &group_data(10.0, None), &mut state).unwrap();
        assert_eq!(outs.len(), 1, "首帧应转发");
        // 完全相同帧（值同）抑制——ts 变化不影响比较（比的是 values）
        let outs = evaluate(&plan, &flow, &group_data(10.0, None), &mut state).unwrap();
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
        let outs = evaluate(&plan, &flow, &group_data(f64::NAN, None), &mut state).unwrap();
        assert_eq!(outs.len(), 1);
        let outs = evaluate(&plan, &flow, &group_data(f64::NAN, None), &mut state).unwrap();
        assert_eq!(outs.len(), 1, "NaN ≠ NaN：连续 NaN 帧每帧转发");
    }

    #[test]
    fn dedup_validate_accepts_empty_config() {
        assert!(validate_config("deduplicate", &json!({})).is_ok());
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p gateway-flow`
Expected: 编译失败（`deduplicate` 未知算子 / `FlowState` 无 `last_frames`）

- [ ] **Step 3: 实现**

operators.rs：`OPERATOR_NAMES` 改 `[&str; 8]` 并追加 `"deduplicate"`；`PortSpec::operator` 加分支：

```rust
            "deduplicate" => PortSpec {
                inputs: &["in"],
                outputs: &["out"],
            },
```

`validate_config` 加：

```rust
        "deduplicate" => Ok(()),
```

配置与求值（`RateOfChangeConfig` / `eval_rate_of_change` 之后）：

```rust
/// deduplicate 配置：无字段（spec §三）。
#[derive(Debug, Deserialize)]
pub struct DeduplicateConfig {}

/// 整帧去重求值（spec §三）：与上一帧 values 逐位相等（含 TagId、类型、长度）→ 无输出
/// （下游静默跳过）；任一差异 → 转发完整帧并更新快照。NaN ≠ NaN：含 NaN 帧每帧转发。
pub fn eval_deduplicate(
    node_id: Uuid,
    state: &mut HashMap<Uuid, Vec<(gateway_sdk::TagId, DataValue)>>,
    gd: &GroupData,
) -> Option<FlowPayload> {
    let snapshot = gd.values.clone();
    let changed = match state.get(&node_id) {
        None => true,
        Some(prev) => *prev != snapshot,
    };
    if changed {
        state.insert(node_id, snapshot);
        Some(FlowPayload::Data(gd.clone()))
    } else {
        None
    }
}
```

（`gateway_sdk::TagId` 若未在 operators.rs 引入，按文件现有 use 风格补 `use gateway_sdk::TagId;` 并用短名。）

evaluate_node 末尾 match 加：

```rust
        "deduplicate" => {
            serde_json::from_value::<DeduplicateConfig>(config.clone())
                .map_err(|e| FlowError::Operator(operator.into(), e.to_string()))?;
            match eval_deduplicate(node_id, &mut state.last_frames, &gd) {
                Some(out) => Ok(vec![("out", out)]),
                None => Ok(vec![]),
            }
        }
```

engine.rs `FlowState` 加字段：

```rust
    /// deduplicate 快照：节点 id → 上一帧 values
    pub last_frames: HashMap<Uuid, Vec<(gateway_sdk::TagId, DataValue)>>,
```

（engine.rs 已 `use gateway_sdk::GroupData;`——按现有 use 风格补 `TagId, DataValue` 或用全路径。）

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p gateway-flow`
Expected: 全部 PASS

- [ ] **Step 5: Commit**

```bash
git add gateway/gateway-flow
git commit -m "feat(flow): deduplicate 算子——整帧去重（逐位比较/NaN 帧恒转发）"
```

---

### Task 3: 引擎注入 evaluate_with_inject + source 断流配置校验（gateway-flow）

**Files:**
- Modify: `gateway/gateway-flow/src/engine.rs`（evaluate 重构为 evaluate_with_inject 的包装；payload 获取改 Option 语义）
- Modify: `gateway/gateway-flow/src/operators.rs`（SourceConfig 加 stale 字段；validate_source_config 扩展）
- Modify: `gateway/gateway-flow/src/validate.rs`（断流告警必须有投递出口）
- Test: `gateway/gateway-flow/src/lib.rs`

**Interfaces:**
- Consumes: `AlarmEvent`（payload.rs）
- Produces: `evaluate_with_inject(plan, flow, input, state, inject: Option<&AlarmEvent>)`（Task 4 runtime 调用）；`operators::SourceConfig { stale_timeout_ms, stale_level }`（Task 4 解析）

- [ ] **Step 1: 写失败测试**（lib.rs tests 尾部追加）

```rust
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
        let (flow, sa, sn) = alarm_graph();
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
```

注意：测试里 `evaluate_with_inject`、`AlarmEvent`、`AlarmLevel` 需在 lib.rs tests 的 use 中补齐（`use crate::engine::evaluate_with_inject;` 等按现有 use 风格）。

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p gateway-flow`
Expected: 编译失败（`evaluate_with_inject` 不存在 / SourceConfig 无 stale 字段）

- [ ] **Step 3: 实现**

operators.rs `SourceConfig`（:26-30）改为：

```rust
#[derive(Debug, Deserialize)]
pub struct SourceConfig {
    pub south_node_id: Uuid,
    pub group_id: Uuid,
    /// 断流检测超时（毫秒）；0 或缺省 = 不检测（spec §4.1）
    #[serde(default)]
    pub stale_timeout_ms: u64,
    /// 断流告警等级；stale_timeout_ms > 0 时必须是 AlarmLevel 合法值
    #[serde(default = "default_stale_level")]
    pub stale_level: String,
}

fn default_stale_level() -> String {
    "high".into()
}
```

`validate_source_config`（:33-37）改为：

```rust
pub fn validate_source_config(config: &serde_json::Value) -> Result<(), String> {
    let c: SourceConfig = serde_json::from_value(config.clone())
        .map_err(|e| format!("invalid source config (need south_node_id + group_id): {e}"))?;
    if c.stale_timeout_ms > 0 {
        AlarmLevel::parse(&c.stale_level)?;
    }
    Ok(())
}
```

engine.rs `evaluate`（:29-97）重构为：

```rust
pub fn evaluate(
    plan: &ExecutionPlan,
    flow: &Flow,
    input: &GroupData,
    state: &mut FlowState,
) -> Result<Vec<SinkDelivery>, FlowError> {
    evaluate_with_inject(plan, flow, input, state, None)
}

/// 断流告警注入求值（spec §4.3）：`inject = Some(ev)` 时 alarm_to_data 算子与
/// sink_alarm sink 直接收 `Alarm(ev)`，其余节点（含 Source）无输入静默——
/// 缺测不误触发。纯函数，下游传播复用分层循环。
pub fn evaluate_with_inject(
    plan: &ExecutionPlan,
    flow: &Flow,
    input: &GroupData,
    state: &mut FlowState,
    inject: Option<&crate::payload::AlarmEvent>,
) -> Result<Vec<SinkDelivery>, FlowError> {
    let mut outputs: HashMap<Uuid, Vec<(&'static str, FlowPayload)>> = HashMap::new();
    let mut deliveries: Vec<SinkDelivery> = Vec::new();

    for tier in &plan.tiers {
        for &node_id in tier {
            let node = plan
                .node(flow, node_id)
                .ok_or(FlowError::NodeNotFound(node_id))?;
            // 注入模式：告警直接送达 alarm_to_data / sink_alarm，其他节点无输入
            let injected = match (inject, node.operator.as_deref(), node.kind) {
                (Some(ev), Some("alarm_to_data"), NodeKind::Operator) => {
                    Some(FlowPayload::Alarm(ev.clone()))
                }
                (Some(ev), Some("sink_alarm"), NodeKind::Sink) => {
                    Some(FlowPayload::Alarm(ev.clone()))
                }
                _ => None,
            };
            let payload: Option<FlowPayload> = if let Some(p) = injected {
                Some(p)
            } else if inject.is_some() {
                None
            } else {
                match node.kind {
                    NodeKind::Source => Some(FlowPayload::Data(input.clone())),
                    NodeKind::Operator | NodeKind::Sink => {
                        let edge = flow
                            .graph
                            .edges
                            .iter()
                            .find(|e| e.target_node_id == node_id)
                            .ok_or_else(|| {
                                FlowError::Validation(format!(
                                    "node `{}` has no input edge",
                                    node.name
                                ))
                            })?;
                        outputs
                            .get(&edge.source_node_id)
                            .and_then(|outs| {
                                outs.iter().find(|(port, _)| *port == edge.source_port)
                            })
                            .map(|(_, p)| p.clone())
                    }
                }
            };
            let Some(payload) = payload else {
                continue;
            };

            match node.kind {
                NodeKind::Sink => {
                    deliveries.push(SinkDelivery { node_id, payload });
                    outputs.insert(node_id, Vec::new());
                }
                NodeKind::Source => {
                    outputs.insert(node_id, vec![("out", payload)]);
                }
                NodeKind::Operator => {
                    let op = node
                        .operator
                        .as_deref()
                        .ok_or_else(|| {
                            FlowError::Validation(format!(
                                "node `{}` has no operator",
                                node.name
                            ))
                        })?
                        .to_string();
                    let outs = crate::operators::evaluate_node(
                        &op,
                        node_id,
                        &node.config,
                        payload,
                        state,
                    )?;
                    outputs.insert(node_id, outs);
                }
            }
        }
    }
    Ok(deliveries)
}
```

（行为与原 evaluate 在 `inject = None` 时逐行等价：payload 从"取不到就 continue"改为 Option + 统一 continue。）

validate.rs `validate()` 在「环」检查之前加一段（放在孤立节点检查后）：

```rust
    // 断流检测配置了必须有告警投递出口（spec §五 publish 门禁）
    let stale_on = nodes.iter().any(|n| {
        n.kind == NodeKind::Source
            && serde_json::from_value::<operators::SourceConfig>(n.config.clone())
                .map(|c| c.stale_timeout_ms > 0)
                .unwrap_or(false)
    });
    if stale_on {
        let has_alarm_out = nodes
            .iter()
            .any(|n| matches!(n.operator.as_deref(), Some("sink_alarm") | Some("alarm_to_data")));
        if !has_alarm_out {
            errs.push(FlowError::Validation(
                "stale detection is configured but the graph has no sink_alarm/alarm_to_data node to deliver the alarm".into(),
            ));
        }
    }
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p gateway-flow`
Expected: 全部 PASS（evaluate_with_inject(None) 与 evaluate 等价性有测试钉住）

- [ ] **Step 5: Commit**

```bash
git add gateway/gateway-flow
git commit -m "feat(flow): 断流告警注入求值 evaluate_with_inject + source 断流配置与 publish 门禁"
```

---

### Task 4: runtime 断流 watchdog（gateway-server）

**Files:**
- Modify: `gateway/gateway-server/src/flow_runtime.rs`（消费循环 select! 重构；deliver 抽取；stale_alarm_event 纯函数）
- Test: `gateway/gateway-server/src/flow_runtime.rs`（文件尾 tests 模块，若无则新建）

**Interfaces:**
- Consumes: `gateway_flow::evaluate_with_inject`、`operators::SourceConfig`（Task 3）、`AlarmStore::raise/resolve`（alarms.rs 既有）、`manager.store.group_get`（store.rs :115）
- Produces: `StaleDetection { timeout_ms, level }`、`stale_alarm_event`（纯函数，供测试）

- [ ] **Step 1: 写失败测试**（flow_runtime.rs 文件尾追加；本任务只测纯函数，select 循环逻辑已提取）

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_alarm_event_shape() {
        let ev = stale_alarm_event("grp1", 5000, "medium");
        assert_eq!(ev.tag, "grp1");
        assert_eq!(ev.level, gateway_flow::AlarmLevel::Medium);
        assert_eq!(ev.value, None);
        assert_eq!(ev.threshold, Some(5000.0));
        assert!(ev.message.contains("组 grp1"), "{}", ev.message);
        assert!(ev.message.contains("5 秒"), "{}", ev.message);
    }

    #[test]
    fn stale_alarm_event_falls_back_level() {
        // 非法 level 回退 high（校验层已挡，此处纯防御）
        let ev = stale_alarm_event("g", 1000, "nope");
        assert_eq!(ev.level, gateway_flow::AlarmLevel::High);
    }

    #[test]
    fn parse_stale_detection_gates_on_zero() {
        let on = parse_stale_detection(&serde_json::json!({
            "south_node_id": uuid::Uuid::new_v4(),
            "group_id": uuid::Uuid::new_v4(),
            "stale_timeout_ms": 5000,
            "stale_level": "high"
        }));
        assert_eq!(on.as_ref().map(|s| s.timeout_ms), Some(5000));
        let off = parse_stale_detection(&serde_json::json!({
            "south_node_id": uuid::Uuid::new_v4(),
            "group_id": uuid::Uuid::new_v4()
        }));
        assert!(off.is_none(), "缺省 0 = 不检测");
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p gateway-server stale`
Expected: 编译失败（`stale_alarm_event` / `parse_stale_detection` 不存在）

- [ ] **Step 3: 实现**（flow_runtime.rs）

文件尾（impl FlowRuntime 之外）加纯函数与结构：

```rust
/// 断流检测配置（source config 解析结果；spec §4.2）。
#[derive(Debug, Clone)]
pub(crate) struct StaleDetection {
    pub timeout_ms: u64,
    pub level: String,
}

/// 解析 source config 的断流检测配置；0 或缺省 = None。
pub(crate) fn parse_stale_detection(config: &serde_json::Value) -> Option<StaleDetection> {
    let c: serde_json::Value = config.clone();
    let timeout = c.get("stale_timeout_ms").and_then(|v| v.as_u64()).unwrap_or(0);
    if timeout == 0 {
        return None;
    }
    Some(StaleDetection {
        timeout_ms: timeout,
        level: c
            .get("stale_level")
            .and_then(|v| v.as_str())
            .unwrap_or("high")
            .to_string(),
    })
}

/// 断流告警事件构造（纯函数，便于单测）。level 非法回退 high（校验层已挡，防御性兜底）。
pub(crate) fn stale_alarm_event(
    group_name: &str,
    timeout_ms: u64,
    level: &str,
) -> gateway_flow::AlarmEvent {
    gateway_flow::AlarmEvent {
        level: gateway_flow::AlarmLevel::parse(level)
            .unwrap_or(gateway_flow::AlarmLevel::High),
        tag: group_name.to_string(),
        value: None,
        threshold: Some(timeout_ms as f64),
        message: format!("组 {group_name} 超过 {} 秒未上报数据", timeout_ms / 1000),
        ts: chrono::Utc::now(),
    }
}
```

`start()` 内、`let mut rxs = ...` 之前加（group_name 查询）：

```rust
        // 断流检测配置与组名（告警 tag 用；查不到回退 group_id 字符串）
        let stale = parse_stale_detection(&src.config);
        let group_name = state
            .manager
            .store
            .group_get(
                gateway_sdk::NodeId(sc.south_node_id),
                gateway_sdk::GroupId(sc.group_id),
            )
            .map(|g| g.name)
            .unwrap_or_else(|| sc.group_id.to_string());
```

任务闭包内（`let mut flow_state = ...` 之后）加状态变量：

```rust
            let mut stale_alarm_id: Option<Uuid> = None;
            let mut deadline = match &stale {
                Some(s) => {
                    tokio::time::Instant::now()
                        + tokio::time::Duration::from_millis(s.timeout_ms)
                }
                None => tokio::time::Instant::now()
                    + tokio::time::Duration::from_secs(365 * 24 * 3600),
            };
```

`loop { match rx.recv().await {...} }` 整段重构为 `loop { tokio::select! {...} }`（分发逻辑抽为闭包内 async fn `deliver_deliveries`，返回本批落库的告警 id 列表）：

```rust
            loop {
                tokio::select! {
                    msg = rx.recv() => {
                        match msg {
                            Ok(data) => {
                                // 数据恢复：自动解除 active 断流告警（spec §4.2）
                                if let Some(id) = stale_alarm_id.take() {
                                    if let Err(e) = alarm_store.resolve(id, &tenant) {
                                        warn!(flow_id = ?flow_id, "stale alarm resolve error: {e}");
                                    }
                                }
                                let deliveries = gateway_flow::evaluate(
                                    &plan, &engine_flow, &data, &mut flow_state,
                                );
                                match deliveries {
                                    Ok(d) => {
                                        deliver_deliveries(
                                            &sinks, &manager, &alarm_store, &alarm_feed,
                                            &tenant, flow_id, d,
                                        ).await;
                                        if stale.is_some() {
                                            deadline = tokio::time::Instant::now()
                                                + tokio::time::Duration::from_millis(
                                                    stale.as_ref().unwrap().timeout_ms,
                                                );
                                        }
                                    }
                                    Err(e) => {
                                        error!(flow_id = ?flow_id, "evaluate error: {e}");
                                    }
                                }
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                                warn!(flow_id = ?flow_id, lagged = n, "flow bus recv lagged, skipped messages");
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                        }
                    }
                    _ = tokio::time::sleep_until(deadline) => {
                        let Some(s) = stale.clone() else { continue };
                        let ev = stale_alarm_event(&group_name, s.timeout_ms, &s.level);
                        let stub = gateway_sdk::GroupData {
                            node_id: gateway_sdk::NodeId(sc.south_node_id),
                            group_id: gateway_sdk::GroupId(sc.group_id),
                            ts: chrono::Utc::now(),
                            values: vec![],
                            node_name: None,
                            group_name: None,
                            tag_names: None,
                        };
                        let deliveries = gateway_flow::evaluate_with_inject(
                            &plan, &engine_flow, &stub, &mut flow_state, Some(&ev),
                        );
                        match deliveries {
                            Ok(d) => {
                                let ids = deliver_deliveries(
                                    &sinks, &manager, &alarm_store, &alarm_feed,
                                    &tenant, flow_id, d,
                                ).await;
                                // 只记录断流告警自身的 id（本批 raise 的最后一个）
                                stale_alarm_id = ids.last().copied();
                            }
                            Err(e) => {
                                error!(flow_id = ?flow_id, "stale inject evaluate error: {e}");
                            }
                        }
                        deadline = tokio::time::Instant::now()
                            + tokio::time::Duration::from_millis(s.timeout_ms);
                    }
                }
            }
```

原 loop 内的 sink 分发逻辑抽成文件级 async fn（`deliver_north` 之前）：

```rust
/// deliveries 分发（数据帧与断流注入共用）。返回本批落库的告警 id（断流路径据此自动 resolve）。
async fn deliver_deliveries(
    sinks: &HashMap<Uuid, (String, SinkTarget)>,
    manager: &Arc<gateway_core::Manager>,
    alarm_store: &Arc<crate::alarms::AlarmStore>,
    alarm_feed: &tokio::sync::broadcast::Sender<crate::ws::WsAlarmData>,
    tenant: &str,
    flow_id: Uuid,
    deliveries: Vec<gateway_flow::SinkDelivery>,
) -> Vec<Uuid> {
    let mut raised: Vec<Uuid> = Vec::new();
    for d in deliveries {
        let Some((node_name, target)) = sinks.get(&d.node_id) else {
            continue;
        };
        match (target, &d.payload) {
            (SinkTarget::Alarm, gateway_flow::FlowPayload::Alarm(ev)) => {
                match alarm_store.raise(tenant, &flow_id, node_name, ev) {
                    Ok(id) => {
                        raised.push(id);
                        // WS alarm 帧（机会式；REST 为可靠来源）
                        alarm_feed.notify(crate::ws::WsAlarmData {
                            id: id.to_string(),
                            flow_id: flow_id.to_string(),
                            node_name: node_name.clone(),
                            level: ev.level.as_str().into(),
                            tag: ev.tag.clone(),
                            value: ev.value,
                            threshold: ev.threshold,
                            message: ev.message.clone(),
                            ts: ev.ts.timestamp_millis(),
                        });
                    }
                    Err(e) => {
                        error!(flow_id = ?flow_id, "alarm raise error: {e}");
                    }
                }
            }
            (
                SinkTarget::North { north_node_id },
                gateway_flow::FlowPayload::Data(gd),
            ) => {
                deliver_north(manager, *north_node_id, Arc::new(gd.clone())).await;
            }
            _ => {}
        }
    }
    raised
}
```

（`alarm_feed` 的实际类型以 `state.alarm_feed` 字段为准——实现时先 grep `alarm_feed` 定义核对类型再写签名；`crate::ws::WsAlarmData` 同理。）

闭包 move 调整：`sinks`/`manager`/`alarm_store`/`alarm_feed`/`tenant` 原本 move 进闭包，改为闭包内以引用传给 `deliver_deliveries`（闭包仍持有所有权，不需要 Arc 化——deliver_deliveries 借用即可）。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p gateway-server`
Expected: 全部 PASS（含既有 flow/告警测试）

- [ ] **Step 5: Commit**

```bash
git add gateway/gateway-server
git commit -m "feat(server): flow 断流 watchdog——select! 超时检测/周期 re-raise/数据恢复自动 resolve"
```

---

### Task 5: 前端流程编辑器表单 + i18n（web）

**Files:**
- Modify: `web/src/utils/graphMapping.js:118-131`（defaultConfig 两 case）
- Modify: `web/src/views/FlowEditor.vue:156`（OPS_WITH_FORM 加 rate_of_change）
- Modify: `web/src/views/FlowEditor.vue`（rate_of_change 表单分支；source 面板 stale 两字段）
- Modify: `web/src/locales/zh.js` / `web/src/locales/en.js`（flowsEditor 命名空间）
- Test: `web/tests/graphMapping.test.js`

**Interfaces:**
- Consumes: `/flows/operators` 自动发现（OPERATOR_NAMES 8 种，Task 1/2 已扩）；后端 SourceConfig 字段名 `stale_timeout_ms` / `stale_level`
- Produces: 无（纯 UI）

- [ ] **Step 1: 写失败测试**（graphMapping.test.js）

`OPERATORS` 常量（:14-21）追加两项：

```js
  { name: 'rate_of_change', inputs: ['in'], outputs: ['alarm', 'normal'] },
  { name: 'deduplicate', inputs: ['in'], outputs: ['out'] },
```

文件尾追加：

```js
test('defaultConfig 新算子', () => {
  assert.deepEqual(defaultConfig('rate_of_change'), {
    tag_name: '', max_rate: null, level: 'high', message: '',
  })
  assert.deepEqual(defaultConfig('deduplicate'), {})
})
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd web && npm run test:unit`
Expected: FAIL（defaultConfig 无 rate_of_change case → `undefined`）

- [ ] **Step 3: 实现**

graphMapping.js `defaultConfig` switch（:119-130）`case 'sink_north'` 后加：

```js
    case 'rate_of_change':
      return { tag_name: '', max_rate: null, level: 'high', message: '' }
    case 'deduplicate':
      return {}
```

FlowEditor.vue `OPS_WITH_FORM`（:156）改为：

```js
const OPS_WITH_FORM = ['threshold_alarm', 'state_change_alarm', 'route', 'sink_north', 'rate_of_change']
```

模板：route 分支（`panelMode === 'route'` 的 template 块）之后加：

```html
            <!-- rate_of_change -->
            <template v-else-if="panelMode === 'rate_of_change'">
              <el-form-item :label="t('flowsEditor.tagName')" required>
                <el-input v-model="selectedNode.data.config.tag_name" />
              </el-form-item>
              <el-form-item :label="t('flowsEditor.maxRate')" required>
                <el-input-number v-model="selectedNode.data.config.max_rate" :min="0" :controls="false" class="w-full" />
              </el-form-item>
              <el-form-item :label="t('flowsEditor.level')">
                <el-select v-model="selectedNode.data.config.level">
                  <el-option v-for="lv in ['critical', 'high', 'medium', 'low', 'info']" :key="lv" :value="lv" :label="lv" />
                </el-select>
              </el-form-item>
              <el-form-item :label="t('flowsEditor.message')">
                <el-input v-model="selectedNode.data.config.message" />
              </el-form-item>
            </template>
```

（level 的 el-options 写法照抄 threshold_alarm 分支 :348-353 的现有实现——若现有实现用了别的 options 来源，以现有为准。）

source 面板（`panelMode === 'source'` template 内、southGroup 之后）加：

```html
              <el-form-item :label="t('flowsEditor.staleTimeout')">
                <el-input-number v-model="selectedNode.data.config.stale_timeout_ms" :min="0" :controls="false" class="w-full" />
              </el-form-item>
              <el-form-item v-if="selectedNode.data.config.stale_timeout_ms > 0" :label="t('flowsEditor.staleLevel')">
                <el-select v-model="selectedNode.data.config.stale_level">
                  <el-option v-for="lv in ['critical', 'high', 'medium', 'low', 'info']" :key="lv" :value="lv" :label="lv" />
                </el-select>
              </el-form-item>
```

（`stale_timeout_ms` 初始为 undefined 时 el-input-number 显示空；`undefined > 0` 为 false → level 选择器隐藏；serde default 兜底，无需前端初始化。）

zh.js flowsEditor（`op_sink_alarm` 之后）加：

```js
    op_rate_of_change: '变化率告警',
    op_deduplicate: '帧去重',
    maxRate: '速率上限（值/秒）',
    staleTimeout: '断流检测超时（ms，0 关闭）',
    staleLevel: '断流告警级别',
```

en.js 对应位置加：

```js
    op_rate_of_change: 'Rate-of-change alarm',
    op_deduplicate: 'Frame dedup',
    maxRate: 'Max rate (units/s)',
    staleTimeout: 'Stale timeout (ms, 0 = off)',
    staleLevel: 'Stale alarm level',
```

- [ ] **Step 4: 跑测试与构建确认通过**

Run: `cd web && npm run test:unit && npm run build`
Expected: 测试 PASS、build 成功

- [ ] **Step 5: Commit**

```bash
git add web
git commit -m "feat(web): 流程编辑器 rate_of_change 表单与 source 断流配置 + i18n"
```

---

### Task 6: 文档同步 + 全量回归

**Files:**
- Modify: `docs/架构说明.md`（流程编排算子清单补段）
- Modify: `docs/功能实现清单.md`（flow 算子与断流告警条目更新）

- [ ] **Step 1: 架构说明补段**（流程编排/flow 一节，算子清单处）

在算子清单处追加（措辞与既有清单风格一致）：

```markdown
**流算子（第二批扩展）**：
- `rate_of_change`：变化速率超限告警（相邻拍单差分，|Δv/Δt| > max_rate 触发 alarm 端；缺值走 normal 端且不更新基线；首见只建基线）；
- `deduplicate`：整帧去重（与上一帧 values 逐位相等则抑制，下游静默；NaN ≠ NaN 帧每帧转发）；
- **断流告警**：source 节点配置 `stale_timeout_ms`（0=关闭）与 `stale_level`，组采集超时未上报 → Alarm 事件注入图（alarm_to_data / sink_alarm 收到，其他算子静默），持续断流按超时周期 re-raise（AlarmStore 同键 active 去重累计 occurrences），数据恢复自动 resolve；配置了断流检测的流程必须接至少一个告警出口（publish 门禁）。
```

- [ ] **Step 2: 功能实现清单更新**（flow 编排相关条目）

算子清单从 6 种改为 8 种；补断流告警条目与 `evaluate_with_inject` 注入语义一句话。

- [ ] **Step 3: 全量回归**

Run: `cargo test --workspace`（预期 60+ 测试目标全绿）与 `cd web && npm run build`（预期成功）

- [ ] **Step 4: Commit**

```bash
git add docs
git commit -m "docs: 流算子第二批（rate_of_change/deduplicate/断流告警）文档同步"
```
