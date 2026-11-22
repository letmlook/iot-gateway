# 数据面聚合扩容与累计量（第一批）Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 窗口聚合 kinds 6→14 种（sum/stddev/median/delta/p50/p90/p95/p99）+ totalizer 跨窗口累计量（周期重置/翻转补差/变化+心跳发射）+ 前端组策略配置 UI。

**Architecture:** 全部挂在既有 `GroupPolicy` / `filters::apply()` 热路径上（spec 方案 A）：聚合扩展 `aggregate()`，分位数走新增的 `aggregate_sorted()`（调用方单次排序共用）；totalizer 是独立纯状态函数 `totalizer_advance()`，输出并入既有 `synthetic` 虚拟点位通道。前端为**新建**策略对话框（现状无任何策略 UI，spec 偏差已在 Task 6 头部注明）。

**Tech Stack:** Rust（workspace，无新依赖）、Vue 3 + Element Plus + vue-i18n。

**Spec:** `docs/superpowers/specs/2026-11-22-dataplane-agg-expansion-design.md`（口径以 spec 为准）

## Global Constraints

- 运行态（窗口缓冲/totalizer 累计）不落盘、重启清零——与 [rules.rs](../../../gateway/gateway-core/src/rules.rs) / filters 既有语义一致，禁止给运行态加持久化。
- 凡通过 `apply()` 触碰全局 `FilterState` 的测试必须包在 `with_lock(...)` 里（进程级单例，并行测试互扰）；纯函数测试（`aggregate` / `percentile_linear` / `totalizer_advance`）不需要锁。
- property test 用文件内既有的确定性 `PropRng`（xorshift），种子写死，禁止引入 proptest。
- 不引入任何新 crate 依赖。
- 校验错误信息：英文、含出错点名/字段值，风格对齐 `validate_policy` 现有文案。
- API 零新增端点：policy 走既有 `GET/PUT/DELETE /api/nodes/{id}/groups/{gid}/policy`。
- 每个任务结束跑 `cargo test -p gateway-core filters::`，全绿才 commit。commit 尾部带 `Co-Authored-By: Claude Code <noreply@anthropic.com>`（全局 gitconfig 已是 letmlook / letmlook@aliyun.com，无需 -c）。
- NaN 保持现状语义（窗口样本可能含 NaN，聚合结果随传染），本批不处理。

---

### Task 1: 简单聚合扩容——sum / stddev / delta

**Files:**
- Modify: `gateway/gateway-core/src/filters.rs:263-279`（`aggregate()` 加 3 分支）
- Modify: `gateway/gateway-core/src/filters.rs:109`（kinds 白名单 6→9）
- Modify: `gateway/gateway-core/src/filters.rs:1069-1089`（`validate_rejects_unknown_agg` 用例从 `"median"` 改 `"bogus"`——median 在 Task 2 会变合法）
- Test: `gateway/gateway-core/src/filters.rs`（tests 模块内新增）

**Interfaces:**
- Consumes: 现有 `fn aggregate(kind: &str, samples: &[(u64, f64)]) -> Option<f64>`（空窗返回 None 的既有机制）。
- Produces: `aggregate()` 支持新增 kind `"sum"`（Σv）、`"stddev"`（n−1 样本标准差，n<2 → None）、`"delta"`（`last.v − first.v`，按时间序，单样本 → 0.0）；白名单数组 `["avg","min","max","count","first","last","sum","stddev","delta"]`。Task 2 在此数组上追加 5 个分位 kind。

- [ ] **Step 1: 写失败测试**（加到 filters.rs tests 模块，`// ===== Window aggregation =====` 区段之后）

```rust
    #[test]
    fn aggregate_sum_stddev_delta() {
        // 教科书样本 [2,4,4,4,5,5,7,9]：sum=40，样本方差=8，stddev=√8
        let vals = [2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0];
        let s: Vec<(u64, f64)> = vals.iter().enumerate().map(|(i, v)| (i as u64, *v)).collect();
        assert_eq!(aggregate("sum", &s), Some(40.0));
        let sd = aggregate("stddev", &s).unwrap();
        assert!((sd - 8.0f64.sqrt()).abs() < 1e-12, "stddev got {sd}");

        // n=1：stddev 无定义 → None；delta → 0
        let one = vec![(0u64, 7.0)];
        assert_eq!(aggregate("stddev", &one), None);
        assert_eq!(aggregate("delta", &one), Some(0.0));

        // delta = 时间序 last − first
        let s2 = vec![(1u64, 10.0), (2, 25.0), (3, 15.0)];
        assert_eq!(aggregate("delta", &s2), Some(5.0));
        assert_eq!(aggregate("sum", &s2), Some(50.0));

        // 空窗 → None（现有机制）
        let empty: Vec<(u64, f64)> = Vec::new();
        assert_eq!(aggregate("sum", &empty), None);
        assert_eq!(aggregate("stddev", &empty), None);
        assert_eq!(aggregate("delta", &empty), None);
    }

    #[test]
    fn validate_accepts_new_simple_kinds() {
        let nid = nid();
        let gid = gid();
        let p = win_policy(nid, gid, 1000, 100, vec!["sum", "stddev", "delta"]);
        assert!(validate_policy(&p).is_ok());
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p gateway-core filters::tests::aggregate_sum_stddev_delta -- --nocapture`
Expected: FAIL，`aggregate("sum", ...)` 返回 `None`（未知 kind 分支）

- [ ] **Step 3: 实现 aggregate() 三个分支**（在 `"last"` 分支之后、`_ => None` 之前插入）

```rust
        "sum" => Some(samples.iter().map(|(_, v)| v).sum()),
        "stddev" => {
            // 样本标准差（n−1 分母）；n<2 统计上无定义 → None（不输出该虚拟点位）
            if samples.len() < 2 {
                return None;
            }
            let mean = samples.iter().map(|(_, v)| v).sum::<f64>() / samples.len() as f64;
            let var = samples
                .iter()
                .map(|(_, v)| {
                    let d = v - mean;
                    d * d
                })
                .sum::<f64>()
                / (samples.len() - 1) as f64;
            Some(var.sqrt())
        }
        "delta" => {
            // 窗口首尾差（时间序）；单样本 → 0（该窗无变化）
            let first = samples.first().map(|(_, v)| *v)?;
            let last = samples.last().map(|(_, v)| *v)?;
            Some(last - first)
        }
```

- [ ] **Step 4: 白名单 6→9**（filters.rs:109）

```rust
        let valid = [
            "avg", "min", "max", "count", "first", "last", "sum", "stddev", "delta",
        ];
```

- [ ] **Step 5: 修 `validate_rejects_unknown_agg` 用例**——`"median"` 尚未实现，改用 `"bogus"`（否则本任务窗口内该测试失败）：

```rust
                aggregates: WindowAgg {
                    kinds: vec!["bogus".to_string()],
                },
```

- [ ] **Step 6: 跑 filters 全量测试**

Run: `cargo test -p gateway-core filters::`
Expected: 全 PASS（含新增 2 个测试与既有全部回归）

- [ ] **Step 7: Commit**

```bash
git add gateway/gateway-core/src/filters.rs
git commit -m "feat(filters): 窗口聚合新增 sum/stddev/delta（白名单 6→9）

Co-Authored-By: Claude Code <noreply@anthropic.com>"
```

---

### Task 2: 分位数聚合——median / p50 / p90 / p95 / p99（单次排序共用）

**Files:**
- Modify: `gateway/gateway-core/src/filters.rs:263-279` 之后（新增 `percentile_linear` / `aggregate_sorted` / `needs_sorted`）
- Modify: `gateway/gateway-core/src/filters.rs:525-545`（`apply()` 发射循环：排序一次共用）
- Modify: `gateway/gateway-core/src/filters.rs:109`（白名单 9→14）
- Test: `gateway/gateway-core/src/filters.rs`（tests 模块）

**Interfaces:**
- Consumes: Task 1 的白名单数组与 `aggregate()`。
- Produces:
  - `fn percentile_linear(sorted: &[f64], p: f64) -> f64`（前置：sorted 升序非空）
  - `fn aggregate_sorted(kind: &str, sorted: &[f64]) -> Option<f64>`（`"median"|"p50"|"p90"|"p95"|"p99"`，空 → None）
  - `fn needs_sorted(kind: &str) -> bool`
  - 白名单全集 `["avg","min","max","count","first","last","sum","stddev","median","delta","p50","p90","p95","p99"]`（Task 3-7 引用此全集）

- [ ] **Step 1: 写失败测试**（纯函数 + apply 级）

```rust
    #[test]
    fn percentile_linear_and_sorted_aggregates() {
        // median：奇数取中、偶数取中间两值平均
        let mut odd = vec![3.0, 1.0, 2.0];
        odd.sort_by(f64::total_cmp);
        assert_eq!(aggregate_sorted("median", &odd), Some(2.0));
        let mut even = vec![4.0, 1.0, 3.0, 2.0];
        even.sort_by(f64::total_cmp);
        assert_eq!(aggregate_sorted("median", &even), Some(2.5));

        // 线性插值：[0..9] p90 → rank=0.9*9=8.1 → 8*0.9+9*0.1=8.1
        let s: Vec<f64> = (0..10).map(|i| i as f64).collect();
        assert!((aggregate_sorted("p90", &s).unwrap() - 8.1).abs() < 1e-9);
        // [10,20] p95 → rank=0.95 → 10*0.05+20*0.95=19.5
        assert!((aggregate_sorted("p95", &vec![10.0, 20.0]).unwrap() - 19.5).abs() < 1e-9);
        // n=1 → 该样本；p50 == median；空 → None
        assert_eq!(aggregate_sorted("p99", &vec![42.0]), Some(42.0));
        assert_eq!(aggregate_sorted("p50", &even), aggregate_sorted("median", &even));
        let empty: Vec<f64> = Vec::new();
        assert_eq!(aggregate_sorted("p95", &empty), None);
    }

    #[test]
    fn prop_window_emit_count_matches_kinds() {
        // 性质：非空窗口发射时，synthetic.len() == 启用的 kinds 数（14 种全集不变量）
        with_lock(|| {
            let nid = nid();
            let gid = gid();
            let tid = tid();
            let store = store_with(nid, gid, tid, "temp");
            let all = vec![
                "avg", "min", "max", "count", "first", "last", "sum", "stddev",
                "median", "delta", "p50", "p90", "p95", "p99",
            ];
            store.policy_insert(win_policy(nid, gid, 10000, 500, all));
            forget_group(nid, gid);
            apply(&store, nid, gid, &[(tid, DataValue::Float64(1.0))], 1000);
            let r = apply(&store, nid, gid, &[(tid, DataValue::Float64(2.0))], 1500);
            assert_eq!(r.window_emitted, 1);
            assert_eq!(r.synthetic.len(), 14, "14 种 kinds 必须各产出一个虚拟点位");
            let names: std::collections::HashSet<&str> =
                r.synthetic_names.values().map(String::as_str).collect();
            assert_eq!(names.len(), 14, "虚拟点位名不得互相覆盖");
            for k in all {
                assert!(names.contains(format!("temp_{k}").as_str()), "缺少 temp_{k}");
            }
        });
    }

    #[test]
    fn validate_accepts_percentile_kinds() {
        let nid = nid();
        let gid = gid();
        let p = win_policy(nid, gid, 1000, 100, vec!["median", "p50", "p90", "p95", "p99"]);
        assert!(validate_policy(&p).is_ok());
    }

    #[test]
    fn window_emits_percentiles_from_stream() {
        with_lock(|| {
            let nid = nid();
            let gid = gid();
            let tid = tid();
            let store = store_with(nid, gid, tid, "temp");
            store.policy_insert(win_policy(nid, gid, 10000, 500, vec!["median", "p95"]));
            forget_group(nid, gid);
            apply(&store, nid, gid, &[(tid, DataValue::Float64(10.0))], 1000);
            apply(&store, nid, gid, &[(tid, DataValue::Float64(30.0))], 1200);
            let r = apply(&store, nid, gid, &[(tid, DataValue::Float64(20.0))], 2000);
            assert_eq!(r.window_emitted, 1);
            assert_eq!(r.synthetic.len(), 2);
            // 窗口 [10,30,20] 排序 [10,20,30]：median=20；p95 rank=1.9 → 20*0.1+30*0.9=29
            for (id, v) in &r.synthetic {
                let DataValue::Float64(f) = v else {
                    panic!("expect Float64");
                };
                match r.synthetic_names.get(id).map(String::as_str) {
                    Some("temp_median") => assert!((*f - 20.0).abs() < 1e-9, "median got {f}"),
                    Some("temp_p95") => assert!((*f - 29.0).abs() < 1e-9, "p95 got {f}"),
                    other => panic!("unexpected synthetic {other:?}"),
                }
            }
        });
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p gateway-core filters::tests::percentile_linear_and_sorted_aggregates -- --nocapture`
Expected: FAIL，`aggregate_sorted` 未定义（编译错误即失败）

- [ ] **Step 3: 实现三个纯函数**（紧跟 `aggregate()` 之后）

```rust
/// 分位数线性插值（numpy 默认 linear 法）：rank = p·(n−1)，相邻两样本插值。
/// 前置条件：sorted 升序且非空。
fn percentile_linear(sorted: &[f64], p: f64) -> f64 {
    let n = sorted.len();
    let rank = p * (n - 1) as f64;
    let lo = rank.floor() as usize;
    let hi = rank.ceil() as usize;
    if lo == hi {
        return sorted[lo];
    }
    let frac = rank - lo as f64;
    sorted[lo] * (1.0 - frac) + sorted[hi] * frac
}

/// 排序类聚合（median/p50/p90/p95/p99）：共用调用方提供的一次升序排序结果，
/// 禁止在每个 kind 内各自排序。
fn aggregate_sorted(kind: &str, sorted: &[f64]) -> Option<f64> {
    if sorted.is_empty() {
        return None;
    }
    match kind {
        "median" => Some(percentile_linear(sorted, 0.5)),
        "p50" => Some(percentile_linear(sorted, 0.50)),
        "p90" => Some(percentile_linear(sorted, 0.90)),
        "p95" => Some(percentile_linear(sorted, 0.95)),
        "p99" => Some(percentile_linear(sorted, 0.99)),
        _ => None,
    }
}

fn needs_sorted(kind: &str) -> bool {
    matches!(kind, "median" | "p50" | "p90" | "p95" | "p99")
}
```

- [ ] **Step 4: 改 `apply()` 发射循环**——把现有（filters.rs:525-545）的

```rust
                if can_emit && has_samples {
                    let samples: Vec<_> = win_state.samples.iter().cloned().collect();
                    for kind in &win.aggregates.kinds {
                        if let Some(val) = aggregate(kind, &samples) {
```

改为：

```rust
                if can_emit && has_samples {
                    let samples: Vec<_> = win_state.samples.iter().cloned().collect();
                    // 排序类聚合共用一次升序排序（median/p50/p90/p95/p99）
                    let sorted: Vec<f64> = if win.aggregates.kinds.iter().any(|k| needs_sorted(k)) {
                        let mut s: Vec<f64> = samples.iter().map(|(_, v)| *v).collect();
                        s.sort_by(f64::total_cmp);
                        s
                    } else {
                        Vec::new()
                    };
                    for kind in &win.aggregates.kinds {
                        let val = if needs_sorted(kind) {
                            aggregate_sorted(kind, &sorted)
                        } else {
                            aggregate(kind, &samples)
                        };
                        if let Some(val) = val {
```

（后续虚拟点位发射体不变，注意原 `if let Some(val) = aggregate(...)` 已被拆开，闭合括号层级对齐。）

- [ ] **Step 5: 白名单 9→14**

```rust
        let valid = [
            "avg", "min", "max", "count", "first", "last", "sum", "stddev", "median",
            "delta", "p50", "p90", "p95", "p99",
        ];
```

- [ ] **Step 6: 跑 filters 全量测试**

Run: `cargo test -p gateway-core filters::`
Expected: 全 PASS（含既有 `validate_rejects_unknown_agg`——Task 1 已改为 `"bogus"`）

- [ ] **Step 7: Commit**

```bash
git add gateway/gateway-core/src/filters.rs
git commit -m "feat(filters): 分位数聚合 median/p50/p90/p95/p99，单次排序共用（白名单 9→14）

Co-Authored-By: Claude Code <noreply@anthropic.com>"
```

---

### Task 3: totalizer 配置结构 + 校验 + 运行态骨架

**Files:**
- Modify: `gateway/gateway-core/src/filters.rs:42-84`（`TotalizerTag` 新结构 + `GroupPolicy.totalizer` 字段）
- Modify: `gateway/gateway-core/src/filters.rs:146-171`（`TotalizerState` 新结构 + `GroupFilterState.totalizers` 字段）
- Modify: `gateway/gateway-core/src/filters.rs:87-140`（`validate_policy` 扩展）
- Modify: filters.rs 内 11 处 `GroupPolicy {` 字面量构造（:631、:644、:657 三个 helper + :1009、:1026、:1049、:1072、:1095、:1116、:1144、:1312 测试字面量）——每处加 `totalizer: None,`
- Test: `gateway/gateway-core/src/filters.rs`

**Interfaces:**
- Consumes: 无（新结构）。
- Produces:
  - `pub struct TotalizerTag { pub tag_name: String, pub reset_period_ms: u64, pub rollover: Option<f64>, pub emit_ms: u64 }`（serde default 全部字段除 tag_name）
  - `GroupPolicy { ..., pub totalizer: Option<Vec<TotalizerTag>> }`（`#[serde(default)]`，旧 JSON 兼容）
  - `struct TotalizerState { total: f64, last_counter: Option<f64>, period_start_ms: u64, last_emitted_total: Option<f64>, last_emit_ms: u64 }`（Default）
  - `GroupFilterState` 新增 `totalizers: RwLock<HashMap<TagId, TotalizerState>>`
  - Task 4 的 `totalizer_advance(&mut TotalizerState, &TotalizerTag, f64, u64) -> Vec<f64>`

- [ ] **Step 1: 写失败测试**

```rust
    #[test]
    fn validate_totalizer_rules() {
        let nid = nid();
        let gid = gid();
        let tag = |name: &str, ro: Option<f64>| TotalizerTag {
            tag_name: name.to_string(),
            reset_period_ms: 0,
            rollover: ro,
            emit_ms: 0,
        };
        let mut p = passthrough_policy(nid, gid);
        p.totalizer = Some(vec![tag("meter", None)]);
        assert!(validate_policy(&p).is_ok());
        // 空数组 = 未启用，合法
        p.totalizer = Some(vec![]);
        assert!(validate_policy(&p).is_ok());
        // 去重拒绝
        p.totalizer = Some(vec![tag("meter", None), tag("meter", None)]);
        assert!(validate_policy(&p).is_err());
        // rollover 非法：0 / 负数 / 非有限
        p.totalizer = Some(vec![tag("m", Some(0.0))]);
        assert!(validate_policy(&p).is_err());
        p.totalizer = Some(vec![tag("m", Some(-1.0))]);
        assert!(validate_policy(&p).is_err());
        p.totalizer = Some(vec![tag("m", Some(f64::INFINITY))]);
        assert!(validate_policy(&p).is_err());
        // 空名拒绝
        p.totalizer = Some(vec![tag("  ", None)]);
        assert!(validate_policy(&p).is_err());
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p gateway-core filters::tests::validate_totalizer_rules -- --nocapture`
Expected: FAIL（编译错误：`TotalizerTag` 未定义）

- [ ] **Step 3: 实现配置结构**（`WindowPolicy` 之后）

```rust
/// 表计累计量配置项（跨窗口持续累计读数）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TotalizerTag {
    /// 表读数点位名（同组内去重拒绝；不要求组内已存在，与 window 口径一致）
    pub tag_name: String,
    /// 累计重置周期（毫秒）；0 = 永不重置
    #[serde(default)]
    pub reset_period_ms: u64,
    /// 表满量程读数；配置后读数高位回绕按翻转补差
    #[serde(default)]
    pub rollover: Option<f64>,
    /// 输出节拍（毫秒）；0 = 仅 total 变化时输出；> 0 时周期心跳强制输出
    #[serde(default)]
    pub emit_ms: u64,
}
```

`GroupPolicy` 追加字段（`tags` 之后）：

```rust
    /// 表计累计量；None 或空数组 = 未启用。输出虚拟点位 `{tag_name}_total`，
    /// 走 synthetic 通道（不经过死区）
    #[serde(default)]
    pub totalizer: Option<Vec<TotalizerTag>>,
```

- [ ] **Step 4: 运行态骨架**（`TagWindow` 之后加结构；`GroupFilterState` 加字段）

```rust
/// 单点位 totalizer 运行态（不落盘，重启清零）
#[derive(Debug, Clone, Default)]
struct TotalizerState {
    total: f64,
    last_counter: Option<f64>,
    period_start_ms: u64,
    last_emitted_total: Option<f64>,
    last_emit_ms: u64,
}
```

`GroupFilterState` 内追加：

```rust
    /// totalizer 运行态（按真实点位 TagId 索引）
    totalizers: RwLock<HashMap<TagId, TotalizerState>>,
```

- [ ] **Step 5: validate 扩展**（`validate_policy` 的 tags 去重校验之后追加）

```rust
    if let Some(ref tots) = policy.totalizer {
        let mut seen = std::collections::HashSet::new();
        for t in tots {
            if t.tag_name.trim().is_empty() {
                return Err("totalizer tag_name must not be empty".to_string());
            }
            if !seen.insert(t.tag_name.as_str()) {
                return Err(format!("duplicate tag_name in totalizer: {}", t.tag_name));
            }
            if let Some(ro) = t.rollover {
                if !ro.is_finite() || ro <= 0.0 {
                    return Err(format!(
                        "tag {} rollover must be > 0 and finite",
                        t.tag_name
                    ));
                }
            }
        }
    }
```

- [ ] **Step 6: 修 11 处构造点**——以下每处 `GroupPolicy {` 字面量补 `totalizer: None,`（放在 `tags:` 字段后）：filters.rs:631、:644、:657、:1009、:1026、:1049、:1072、:1095、:1116、:1144、:1312。改完 `grep -n "totalizer: None" gateway/gateway-core/src/filters.rs | wc -l` 应为 11。

- [ ] **Step 7: 跑 filters 全量测试**

Run: `cargo test -p gateway-core filters::`
Expected: 全 PASS。若报「missing field `totalizer`」说明有构造点遗漏——grep `GroupPolicy {` 逐一补齐。

- [ ] **Step 8: 兼容性冒烟**——旧 JSON 反序列化（加到 tests）：

```rust
    #[test]
    fn totalizer_field_backward_compatible() {
        let json = r#"{"south_node_id":"00000000-0000-0000-0000-000000000000","group_id":"00000000-0000-0000-0000-000000000000","mode":"passthrough"}"#;
        let p: GroupPolicy = serde_json::from_str(json).expect("旧 JSON 必须可反序列化");
        assert!(p.totalizer.is_none());
    }
```

Run: `cargo test -p gateway-core filters::tests::totalizer_field_backward_compatible`
Expected: PASS

- [ ] **Step 9: Commit**

```bash
git add gateway/gateway-core/src/filters.rs
git commit -m "feat(filters): totalizer 累计量配置结构与校验（周期重置/翻转/节拍）

Co-Authored-By: Claude Code <noreply@anthropic.com>"
```

---

### Task 4: totalizer 状态机 `totalizer_advance()` + apply() 接线

**Files:**
- Modify: `gateway/gateway-core/src/filters.rs`（`should_report` 之后新增纯函数；`apply()` 窗口段之后接线）
- Test: `gateway/gateway-core/src/filters.rs`

**Interfaces:**
- Consumes: Task 3 的 `TotalizerTag` / `TotalizerState` / `GroupFilterState.totalizers`。
- Produces:
  - `fn totalizer_advance(st: &mut TotalizerState, cfg: &TotalizerTag, new_val: f64, ts_ms: u64) -> Vec<f64>`——返回本批应发射的 total 值（0 / 1 或 2 个：周期重置边界发 `[旧total, 0]` 两条）
  - `apply()` 对 `policy.totalizer` 非空时逐数值点位推进，输出并入 `synthetic` + `synthetic_names`（命名 `{tag_name}_total`，复用 `synthetic_ids` 缓存）；**不**修改 `window_emitted`（该指标只统计窗口发射）

- [ ] **Step 1: 写失败测试**（纯函数，无需 with_lock）

```rust
    #[test]
    fn totalizer_advance_monotonic_accumulation() {
        let cfg = TotalizerTag {
            tag_name: "m".into(),
            reset_period_ms: 0,
            rollover: None,
            emit_ms: 0,
        };
        let mut st = TotalizerState::default();
        // 首见读数：不输出
        assert_eq!(totalizer_advance(&mut st, &cfg, 100.0, 1000), Vec::<f64>::new());
        // 首见后首次推进：确立基线必发一次（total=10）
        assert_eq!(totalizer_advance(&mut st, &cfg, 110.0, 2000), vec![10.0]);
        // 读数不变：不发
        assert_eq!(totalizer_advance(&mut st, &cfg, 110.0, 3000), Vec::<f64>::new());
        // 110→111：total=11
        assert_eq!(totalizer_advance(&mut st, &cfg, 111.0, 4000), vec![11.0]);
        // 下降（无 rollover → 换表）：total 保持 11，不输出
        assert_eq!(totalizer_advance(&mut st, &cfg, 90.0, 5000), Vec::<f64>::new());
        // 换表后继续累计：90→95 → total=16
        assert_eq!(totalizer_advance(&mut st, &cfg, 95.0, 6000), vec![16.0]);
    }

    #[test]
    fn totalizer_advance_rollover_wraps() {
        let cfg = TotalizerTag {
            tag_name: "m".into(),
            reset_period_ms: 0,
            rollover: Some(1000.0),
            emit_ms: 0,
        };
        let mut st = TotalizerState::default();
        assert_eq!(totalizer_advance(&mut st, &cfg, 990.0, 1000), Vec::<f64>::new());
        assert_eq!(totalizer_advance(&mut st, &cfg, 990.0, 2000), vec![0.0]);
        // 990→10：last=990 > 1000*0.5 → 翻转补差 (1000-990)+10 → total=20
        assert_eq!(totalizer_advance(&mut st, &cfg, 10.0, 3000), vec![20.0]);
        // 10→20：total=30
        assert_eq!(totalizer_advance(&mut st, &cfg, 20.0, 4000), vec![30.0]);
    }

    #[test]
    fn totalizer_advance_rollover_low_drop_is_table_swap() {
        // last 低于 rollover 一半时的下降：视为换表，total 不变
        let cfg = TotalizerTag {
            tag_name: "m".into(),
            reset_period_ms: 0,
            rollover: Some(1000.0),
            emit_ms: 0,
        };
        let mut st = TotalizerState::default();
        assert_eq!(totalizer_advance(&mut st, &cfg, 100.0, 1000), Vec::<f64>::new());
        assert_eq!(totalizer_advance(&mut st, &cfg, 120.0, 2000), vec![20.0]);
        // 120→5：last=120 < 500 → 换表，total 保持 20
        assert_eq!(totalizer_advance(&mut st, &cfg, 5.0, 3000), Vec::<f64>::new());
        // 5→15：total=30
        assert_eq!(totalizer_advance(&mut st, &cfg, 15.0, 4000), vec![30.0]);
    }

    #[test]
    fn totalizer_advance_period_reset_emits_old_then_zero() {
        let cfg = TotalizerTag {
            tag_name: "m".into(),
            reset_period_ms: 1000,
            rollover: None,
            emit_ms: 0,
        };
        let mut st = TotalizerState::default();
        assert_eq!(totalizer_advance(&mut st, &cfg, 10.0, 0), Vec::<f64>::new());
        assert_eq!(totalizer_advance(&mut st, &cfg, 30.0, 500), vec![20.0]);
        // t=1200 ≥ period_start(0)+1000：本次增量先累计（20+2=22），再重置 → 发 [22, 0]
        assert_eq!(totalizer_advance(&mut st, &cfg, 32.0, 1200), vec![22.0, 0.0]);
        // 新周期：基线 32 继续累计
        assert_eq!(totalizer_advance(&mut st, &cfg, 35.0, 1500), vec![3.0]);
    }

    #[test]
    fn totalizer_advance_heartbeat_forces_emit() {
        let cfg = TotalizerTag {
            tag_name: "m".into(),
            reset_period_ms: 0,
            rollover: None,
            emit_ms: 1000,
        };
        let mut st = TotalizerState::default();
        assert_eq!(totalizer_advance(&mut st, &cfg, 10.0, 1000), Vec::<f64>::new());
        // 首见后首次推进必发（基线 0）
        assert_eq!(totalizer_advance(&mut st, &cfg, 10.0, 1500), vec![0.0]);
        // t=2000 距上次发射 500ms < emit_ms(1000)：未到期且无变化 → 不发
        assert_eq!(totalizer_advance(&mut st, &cfg, 10.0, 2000), Vec::<f64>::new());
        // t=2600 距上次发射 1100ms ≥ 1000：心跳到期 → 强制输出（total 仍 0）
        assert_eq!(totalizer_advance(&mut st, &cfg, 10.0, 2600), vec![0.0]);
        // 心跳未到且无变化：不发
        assert_eq!(totalizer_advance(&mut st, &cfg, 10.0, 2800), Vec::<f64>::new());
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p gateway-core filters::tests::totalizer_advance -- --nocapture`
Expected: FAIL（`totalizer_advance` 未定义）

- [ ] **Step 3: 实现纯函数**（`should_report` 之后）

```rust
/// totalizer 状态推进（纯函数，便于测试）：
/// 1) 累计：上升累加；高位回绕按 rollover 补差；未配 rollover（或低位）下降视为
///    换表——total 保留、基线重置（不累计负增量）并 warn；
/// 2) 周期重置：到期时先累计本次增量，再发 [旧total, 0] 两条（周期值不丢，下游以 0 为新周期起点）；
/// 3) 发射：首见后首次推进必发（确立基线）；此后仅变化发射；emit_ms > 0 时周期心跳强发。
fn totalizer_advance(
    st: &mut TotalizerState,
    cfg: &TotalizerTag,
    new_val: f64,
    ts_ms: u64,
) -> Vec<f64> {
    if st.last_counter.is_none() {
        // 首见：只记基线
        st.last_counter = Some(new_val);
        st.period_start_ms = ts_ms;
        return Vec::new();
    }
    if let Some(last) = st.last_counter {
        if new_val >= last {
            st.total += new_val - last;
        } else if let Some(ro) = cfg.rollover {
            if last > ro * 0.5 {
                st.total += (ro - last) + new_val; // 高位回绕：翻转补差
            } else {
                tracing::warn!(
                    tag = %cfg.tag_name,
                    total = st.total,
                    "totalizer counter dropped below half-rollover, treating as table swap"
                );
            }
        } else {
            tracing::warn!(
                tag = %cfg.tag_name,
                total = st.total,
                "totalizer counter dropped without rollover, treating as table swap"
            );
        }
    }
    st.last_counter = Some(new_val);

    // 周期重置（先累计后重置，发 [旧total, 0]）
    if cfg.reset_period_ms > 0 && ts_ms.saturating_sub(st.period_start_ms) >= cfg.reset_period_ms {
        let old = st.total;
        st.total = 0.0;
        st.period_start_ms = ts_ms;
        st.last_emitted_total = Some(0.0);
        st.last_emit_ms = ts_ms;
        return vec![old, 0.0];
    }

    let first_advance = st.last_emitted_total.is_none();
    let changed = first_advance
        || st.last_emitted_total.map(|e| e != st.total).unwrap_or(false);
    let due = cfg.emit_ms > 0 && ts_ms.saturating_sub(st.last_emit_ms) >= cfg.emit_ms;
    if changed || due {
        st.last_emitted_total = Some(st.total);
        st.last_emit_ms = ts_ms;
        vec![st.total]
    } else {
        Vec::new()
    }
}
```

- [ ] **Step 4: 跑纯函数测试**

Run: `cargo test -p gateway-core filters::tests::totalizer_advance -- --nocapture`
Expected: 5 个测试全 PASS

- [ ] **Step 5: apply() 接线**——窗口段（`if let Some(ref win) = policy.window { ... }` 块）之后、`FilterOutcome` 构造之前插入：

```rust
    // ---- totalizer（独立于 mode 与 window，输入永远是原始值）----
    if let Some(tots) = &policy.totalizer {
        if !tots.is_empty() {
            let group_state = state().states.entry((node_id, group_id)).or_default();
            let cfg_by_name: HashMap<&str, &TotalizerTag> =
                tots.iter().map(|t| (t.tag_name.as_str(), t)).collect();
            for (tid, value) in values {
                let Some(f) = numeric_f64(value) else {
                    continue; // 非数值不进 totalizer（与窗口聚合口径一致，静默跳过）
                };
                let Some(tag_name) = store.tag_get(*tid).map(|t| t.name) else {
                    continue;
                };
                let Some(cfg) = cfg_by_name.get(tag_name.as_str()) else {
                    continue;
                };
                // 锁顺序与窗口段一致：totalizers → synthetic_ids（无反向获取，无死锁）
                let mut guard = group_state.totalizers.write().unwrap();
                let st = guard.entry(*tid).or_default();
                let syn_name = format!("{}_total", tag_name);
                for emit_val in totalizer_advance(st, cfg, f, ts_ms) {
                    let syn_id = {
                        let mut sguard = group_state.synthetic_ids.write().unwrap();
                        *sguard.entry(syn_name.clone()).or_default()
                    };
                    synthetic.push((syn_id, DataValue::Float64(emit_val)));
                    synthetic_names.insert(syn_id, syn_name.clone());
                }
            }
        }
    }
```

注意：`suppressed_msgs` 的现有判定是 `published.is_empty() && synthetic.is_empty()`——totalizer 输出计入 synthetic 后，只有 totalizer 输出的批次不再被计为整批抑制，这正是期望行为，无需改动。

- [ ] **Step 6: 加 apply 级集成测试**（须 with_lock）

```rust
    #[test]
    fn totalizer_flows_through_apply() {
        with_lock(|| {
            let nid = nid();
            let gid = gid();
            let tid = tid();
            let store = store_with(nid, gid, tid, "meter");
            let mut p = passthrough_policy(nid, gid);
            p.totalizer = Some(vec![TotalizerTag {
                tag_name: "meter".into(),
                reset_period_ms: 0,
                rollover: None,
                emit_ms: 0,
            }]);
            store.policy_insert(p);
            forget_group(nid, gid);
            let out = apply(&store, nid, gid, &[(tid, DataValue::Float64(100.0))], 1000);
            assert!(out.synthetic.is_empty(), "首见不输出");
            let out = apply(&store, nid, gid, &[(tid, DataValue::Float64(110.0))], 2000);
            assert_eq!(out.synthetic.len(), 1);
            assert_eq!(
                out.synthetic_names.values().next().map(String::as_str),
                Some("meter_total")
            );
            if let DataValue::Float64(f) = &out.synthetic[0].1 {
                assert!((*f - 10.0).abs() < 1e-9, "total got {f}");
            } else {
                panic!("expect Float64");
            }
        });
    }
```

- [ ] **Step 7: 跑 filters 全量测试**

Run: `cargo test -p gateway-core filters::`
Expected: 全 PASS（既有死区/窗口/passthrough 回归不受影响）

- [ ] **Step 8: Commit**

```bash
git add gateway/gateway-core/src/filters.rs
git commit -m "feat(filters): totalizer 状态机接入 apply()（翻转补差/周期重置/心跳发射）

Co-Authored-By: Claude Code <noreply@anthropic.com>"
```

---

### Task 5: totalizer property test（影子模型一致性）

**Files:**
- Test: `gateway/gateway-core/src/filters.rs`（tests 模块，`// ===== Property tests =====` 区段末尾追加）

**Interfaces:**
- Consumes: Task 4 的 `totalizer_advance`、Task 3 的 `TotalizerTag`/`TotalizerState`、既有 `PropRng`。
- Produces: 无新接口。

- [ ] **Step 1: 写 property 测试**

```rust
    #[test]
    fn prop_totalizer_matches_shadow_model() {
        // 性质（任意读数序列，确定性 xorshift；本性质不配 reset_period——重置语义有专门单测）：
        //   1. totalizer_advance 发射的最后一条 == 影子模型 total（逐步独立推演）
        //   2. 发射次数 ≤ 1（非重置配置下单步至多一条）
        //   3. 无发射步 total 不变
        let mut rng = PropRng::new(0xE500_0001);
        for _round in 0..200 {
            let ro = if rng.below(2) == 0 {
                Some(rng.f64_range(100.0, 1000.0))
            } else {
                None
            };
            let cfg = TotalizerTag {
                tag_name: "m".into(),
                reset_period_ms: 0,
                rollover: ro,
                emit_ms: 0,
            };
            let mut st = TotalizerState::default();
            let mut shadow_total = 0.0f64;
            let mut shadow_last: Option<f64> = None;
            let mut ts = 0u64;
            for _step in 0..200 {
                // 60% 缓步上升（可能跨翻转），40% 全范围随机（含下降=换表）
                let v = if rng.below(10) < 6 {
                    shadow_last.unwrap_or(0.0) + rng.f64_range(0.0, 80.0)
                } else {
                    rng.f64_range(0.0, 1200.0)
                };
                let emits = totalizer_advance(&mut st, &cfg, v, ts);
                // 影子模型推进
                if let Some(last) = shadow_last {
                    if v >= last {
                        shadow_total += v - last;
                    } else if let Some(r) = ro {
                        if last > r * 0.5 {
                            shadow_total += (r - last) + v;
                        }
                    }
                }
                shadow_last = Some(v);
                assert!(emits.len() <= 1, "非重置配置单步至多一条发射");
                if let Some(&e) = emits.last() {
                    assert!(
                        (e - shadow_total).abs() < 1e-6,
                        "发射 {e} ≠ 影子 total {shadow_total} (rollover={ro:?})"
                    );
                }
                assert!(
                    (st.total - shadow_total).abs() < 1e-6,
                    "内部 total {} ≠ 影子 {shadow_total}",
                    st.total
                );
                ts += rng.below(5000) + 1;
            }
        }
    }
```

- [ ] **Step 2: 跑 property 测试**

Run: `cargo test -p gateway-core filters::tests::prop_totalizer_matches_shadow_model -- --nocapture`
Expected: PASS（固定种子可重现）

- [ ] **Step 3: 跑 filters 全量 + core 全量**

Run: `cargo test -p gateway-core`
Expected: 全 PASS

- [ ] **Step 4: Commit**

```bash
git add gateway/gateway-core/src/filters.rs
git commit -m "test(filters): totalizer 影子模型 property test（翻转展开一致性）

Co-Authored-By: Claude Code <noreply@anthropic.com>"
```

---

### Task 6: 前端组策略配置 UI（新建对话框）

> **spec 偏差注明**：spec §六写的是「DataMonitor.vue 策略表单扩展」，实际探察发现前端**没有任何**策略 UI（`grep -rn policy web/src` 仅命中 openapi 生成的类型文件）。本任务为**新建**：策略对话框覆盖 GroupPolicy 全部字段（mode/死区/心跳/窗口聚合/点位死区/totalizer）。这是本批唯一的新 UI 面。

**Files:**
- Modify: `web/src/api.js`（groups 端点附近加 3 个方法）
- Modify: `web/src/views/DataMonitor.vue`（state + 函数 + 组选择器旁按钮 + 对话框）
- Modify: `web/src/locales/zh.js` / `web/src/locales/en.js`（monitor 命名空间下加 key）

**Interfaces:**
- Consumes: 既有 `api.js` 的 `req()` 模式、`getErrorMessage(t, e)`（DataMonitor.vue 已有）、Element Plus 组件（el-radio 用 `:value` prop，与 NodeConfigForm.vue 一致）、i18n `monitor.*` 命名空间。
- Produces: `api.getPolicy(nodeId, gid)` / `api.putPolicy(nodeId, gid, body)` / `api.deletePolicy(nodeId, gid)`。

- [ ] **Step 1: api.js 加端点**（`deleteGroup` 之后）

```js
  getPolicy: (nodeId, gid) => req('GET', `/nodes/${nodeId}/groups/${gid}/policy`),
  putPolicy: (nodeId, gid, body) => req('PUT', `/nodes/${nodeId}/groups/${gid}/policy`, body),
  deletePolicy: (nodeId, gid) => req('DELETE', `/nodes/${nodeId}/groups/${gid}/policy`),
```

- [ ] **Step 2: DataMonitor.vue 加 state 与函数**（script 内，`selectedNodeInfo` 定义之前）

```js
// ===== 组数据策略（GroupPolicy）=====
const policyVisible = ref(false)
const policySaving = ref(false)
const POLICY_KINDS = ['avg', 'min', 'max', 'count', 'first', 'last', 'sum', 'stddev', 'median', 'delta', 'p50', 'p90', 'p95', 'p99']
const policyForm = ref(emptyPolicyForm())
const tagDeadbandRows = ref([])
const totalizerRows = ref([])

function emptyPolicyForm() {
  return {
    mode: 'passthrough',
    deadband: null,
    deadband_percent: null,
    max_report_ms: 0,
    windowEnabled: false,
    window_ms: 60000,
    emit_ms: 1000,
    kinds: []
  }
}

function applyPolicyToForm(p) {
  const f = emptyPolicyForm()
  if (p) {
    f.mode = p.mode || 'passthrough'
    f.deadband = p.deadband ?? null
    f.deadband_percent = p.deadband_percent ?? null
    f.max_report_ms = p.max_report_ms || 0
    if (p.window) {
      f.windowEnabled = true
      f.window_ms = p.window.window_ms
      f.emit_ms = p.window.emit_ms
      f.kinds = [...(p.window.aggregates?.kinds || [])]
    }
    tagDeadbandRows.value = (p.tags || []).map(t => ({
      tag_name: t.tag_name, deadband: t.deadband ?? null, deadband_percent: t.deadband_percent ?? null
    }))
    totalizerRows.value = (p.totalizer || []).map(x => ({
      tag_name: x.tag_name, reset_period_ms: x.reset_period_ms || 0,
      rollover: x.rollover ?? null, emit_ms: x.emit_ms || 0
    }))
  } else {
    tagDeadbandRows.value = []
    totalizerRows.value = []
  }
  policyForm.value = f
}

async function openPolicy() {
  if (!selectedNode.value || !selectedGroup.value) return
  policyVisible.value = true
  try {
    applyPolicyToForm(await api.getPolicy(selectedNode.value, selectedGroup.value))
  } catch {
    applyPolicyToForm(null)
  }
}

function numOrNull(v) {
  return v === '' || v === null || v === undefined ? null : Number(v)
}

async function savePolicy() {
  const f = policyForm.value
  // south_node_id/group_id 后端以路径参数覆盖，这里带上保证 body 结构完整
  const body = {
    south_node_id: selectedNode.value,
    group_id: selectedGroup.value,
    mode: f.mode,
    max_report_ms: Number(f.max_report_ms) || 0,
    tags: tagDeadbandRows.value.filter(r => r.tag_name).map(r => ({
      tag_name: r.tag_name,
      deadband: numOrNull(r.deadband),
      deadband_percent: numOrNull(r.deadband_percent)
    })),
    totalizer: totalizerRows.value.filter(r => r.tag_name).map(r => ({
      tag_name: r.tag_name,
      reset_period_ms: Number(r.reset_period_ms) || 0,
      rollover: numOrNull(r.rollover),
      emit_ms: Number(r.emit_ms) || 0
    }))
  }
  const db = numOrNull(f.deadband)
  if (db !== null) body.deadband = db
  const dbp = numOrNull(f.deadband_percent)
  if (dbp !== null) body.deadband_percent = dbp
  if (f.windowEnabled) {
    body.window = {
      window_ms: Number(f.window_ms),
      emit_ms: Number(f.emit_ms),
      aggregates: { kinds: f.kinds }
    }
  }
  policySaving.value = true
  try {
    await api.putPolicy(selectedNode.value, selectedGroup.value, body)
    ElMessage.success(t('monitor.policySaved'))
    policyVisible.value = false
  } catch (e) {
    ElMessage.error(t('monitor.policySaveFailed') + getErrorMessage(t, e))
  } finally {
    policySaving.value = false
  }
}

async function clearPolicy() {
  try {
    await api.deletePolicy(selectedNode.value, selectedGroup.value)
    ElMessage.success(t('monitor.policyCleared'))
    policyVisible.value = false
  } catch (e) {
    ElMessage.error(t('monitor.policySaveFailed') + getErrorMessage(t, e))
  }
}

function addTagDeadbandRow() {
  tagDeadbandRows.value.push({ tag_name: '', deadband: null, deadband_percent: null })
}
function removeTagDeadbandRow(i) { tagDeadbandRows.value.splice(i, 1) }
function addTotalizerRow() {
  totalizerRows.value.push({ tag_name: '', reset_period_ms: 0, rollover: null, emit_ms: 0 })
}
function removeTotalizerRow(i) { totalizerRows.value.splice(i, 1) }
```

- [ ] **Step 3: 模板——组选择器旁加按钮**（DataMonitor.vue template，组 `el-select` 的 `</div>` 之后）

```html
          <div class="control-group">
            <el-button :disabled="!selectedGroup" @click="openPolicy">
              {{ t('monitor.dataPolicy') }}
            </el-button>
          </div>
```

- [ ] **Step 4: 模板——策略对话框**（`</template>` 结束前，页面根 div 内末尾）

```html
    <el-dialog v-model="policyVisible" :title="t('monitor.dataPolicy')" width="680px">
      <el-form label-width="150px">
        <el-form-item :label="t('monitor.policyMode')">
          <el-radio-group v-model="policyForm.mode">
            <el-radio value="passthrough">{{ t('monitor.policyModePassthrough') }}</el-radio>
            <el-radio value="on_change">{{ t('monitor.policyModeOnChange') }}</el-radio>
          </el-radio-group>
        </el-form-item>
        <el-form-item :label="t('monitor.policyDeadband')">
          <el-input-number v-model="policyForm.deadband" :min="0" :controls="false" class="w-40" />
        </el-form-item>
        <el-form-item :label="t('monitor.policyDeadbandPercent')">
          <el-input-number v-model="policyForm.deadband_percent" :min="0" :max="100" :controls="false" class="w-40" />
        </el-form-item>
        <el-form-item :label="t('monitor.policyMaxReport')">
          <el-input-number v-model="policyForm.max_report_ms" :min="0" :controls="false" class="w-40" />
        </el-form-item>

        <el-form-item :label="t('monitor.policyWindow')">
          <el-checkbox v-model="policyForm.windowEnabled">{{ t('monitor.policyEnable') }}</el-checkbox>
        </el-form-item>
        <template v-if="policyForm.windowEnabled">
          <el-form-item :label="t('monitor.policyWindowMs')">
            <el-input-number v-model="policyForm.window_ms" :min="100" :controls="false" class="w-40" />
          </el-form-item>
          <el-form-item :label="t('monitor.policyEmitMs')">
            <el-input-number v-model="policyForm.emit_ms" :min="10" :controls="false" class="w-40" />
          </el-form-item>
          <el-form-item :label="t('monitor.policyKinds')">
            <el-select v-model="policyForm.kinds" multiple class="w-full">
              <el-option v-for="k in POLICY_KINDS" :key="k" :label="k" :value="k" />
            </el-select>
          </el-form-item>
        </template>

        <el-form-item :label="t('monitor.policyTagDeadband')">
          <div class="rows-editor">
            <div v-for="(r, i) in tagDeadbandRows" :key="i" class="row-line">
              <el-select v-model="r.tag_name" filterable class="row-tag">
                <el-option v-for="tg in allTags" :key="tg.id" :label="tg.name" :value="tg.name" />
              </el-select>
              <el-input-number v-model="r.deadband" :min="0" :controls="false" placeholder="±" class="row-num" />
              <el-input-number v-model="r.deadband_percent" :min="0" :max="100" :controls="false" placeholder="%" class="row-num" />
              <el-button link type="danger" @click="removeTagDeadbandRow(i)">✕</el-button>
            </div>
            <el-button link type="primary" @click="addTagDeadbandRow">+ {{ t('monitor.addRow') }}</el-button>
          </div>
        </el-form-item>

        <el-form-item :label="t('monitor.policyTotalizer')">
          <div class="rows-editor">
            <div v-for="(r, i) in totalizerRows" :key="i" class="row-line">
              <el-select v-model="r.tag_name" filterable class="row-tag">
                <el-option v-for="tg in allTags" :key="tg.id" :label="tg.name" :value="tg.name" />
              </el-select>
              <el-input-number v-model="r.reset_period_ms" :min="0" :controls="false" :placeholder="t('monitor.policyResetPeriod')" class="row-num" />
              <el-input-number v-model="r.rollover" :min="0" :controls="false" :placeholder="t('monitor.policyRollover')" class="row-num" />
              <el-input-number v-model="r.emit_ms" :min="0" :controls="false" :placeholder="t('monitor.policyTotalizerEmit')" class="row-num" />
              <el-button link type="danger" @click="removeTotalizerRow(i)">✕</el-button>
            </div>
            <el-button link type="primary" @click="addTotalizerRow">+ {{ t('monitor.addRow') }}</el-button>
          </div>
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="clearPolicy">{{ t('monitor.policyClear') }}</el-button>
        <el-button type="primary" :loading="policySaving" @click="savePolicy">{{ t('common.save') }}</el-button>
      </template>
    </el-dialog>
```

- [ ] **Step 5: 行编辑样式**（`<style scoped>` 末尾）

```css
.rows-editor { width: 100%; }
.row-line { display: flex; gap: 8px; align-items: center; margin-bottom: 6px; }
.row-tag { width: 180px; }
.row-num { width: 120px; }
.w-40 { width: 160px; }
.w-full { width: 100%; }
```

- [ ] **Step 6: i18n**——`web/src/locales/zh.js` 的 `monitor: {` 对象内追加（en.js 同键英文）：

```js
    dataPolicy: '数据策略',
    policyMode: '过滤模式',
    policyModePassthrough: '透传',
    policyModeOnChange: '变化上报',
    policyDeadband: '绝对死区',
    policyDeadbandPercent: '相对死区(%)',
    policyMaxReport: '心跳上报(ms)',
    policyWindow: '滑动窗口聚合',
    policyWindowMs: '窗口长度(ms)',
    policyEmitMs: '发射周期(ms)',
    policyKinds: '聚合项',
    policyEnable: '启用',
    policyTagDeadband: '点位死区覆盖',
    policyTotalizer: '累计量(totalizer)',
    policyResetPeriod: '重置周期(ms)',
    policyRollover: '满量程',
    policyTotalizerEmit: '输出节拍(ms)',
    policySaved: '策略已保存',
    policySaveFailed: '保存失败：',
    policyCleared: '策略已清除',
    policyClear: '清除策略',
    addRow: '添加一行',
```

en.js：

```js
    dataPolicy: 'Data Policy',
    policyMode: 'Filter Mode',
    policyModePassthrough: 'Passthrough',
    policyModeOnChange: 'Report on Change',
    policyDeadband: 'Absolute Deadband',
    policyDeadbandPercent: 'Relative Deadband (%)',
    policyMaxReport: 'Heartbeat (ms)',
    policyWindow: 'Sliding Window Aggregation',
    policyWindowMs: 'Window Length (ms)',
    policyEmitMs: 'Emit Interval (ms)',
    policyKinds: 'Aggregates',
    policyEnable: 'Enable',
    policyTagDeadband: 'Per-tag Deadband',
    policyTotalizer: 'Totalizer',
    policyResetPeriod: 'Reset Period (ms)',
    policyRollover: 'Rollover',
    policyTotalizerEmit: 'Emit Interval (ms)',
    policySaved: 'Policy saved',
    policySaveFailed: 'Save failed: ',
    policyCleared: 'Policy cleared',
    policyClear: 'Clear Policy',
    addRow: 'Add row',
```

- [ ] **Step 7: 构建验证**

Run: `cd web && npm run build`
Expected: 构建成功无报错

- [ ] **Step 8: 浏览器冒烟**（`./scripts/build_and_run.sh --dev` 或既有 dev 流程；无 PLC 也可用 sim 插件）——确认：组选择后按钮可点、对话框打开、保存后重新打开字段回显、清除策略后重新打开回到空表单。

- [ ] **Step 9: Commit**

```bash
git add web/src/api.js web/src/views/DataMonitor.vue web/src/locales/zh.js web/src/locales/en.js
git commit -m "feat(web): 组数据策略配置对话框（死区/心跳/窗口聚合14种/totalizer）

Co-Authored-By: Claude Code <noreply@anthropic.com>"
```

---

### Task 7: 文档与全量回归

**Files:**
- Modify: `docs/架构说明.md`（数据面小节补策略说明）
- Modify: `docs/功能实现清单.md`（若存在数据策略条目则更新；无则不加）

**Interfaces:** 无代码接口。

- [ ] **Step 1: 架构说明补段**——先 `grep -n "过滤\|死区\|数据面" docs/架构说明.md` 定位小节，在相应位置补：

```markdown
### 组数据策略（GroupPolicy）

`GET/PUT/DELETE /api/nodes/:id/groups/:gid/policy`：

- **过滤**：`passthrough` / `on_change`（绝对/相对死区、心跳 `max_report_ms`、点位级覆盖 `tags`）；
- **窗口聚合**：14 种 kinds（`avg/min/max/count/first/last/sum/stddev/median/delta/p50/p90/p95/p99`），虚拟点位 `{tag}_{agg}`，median 与分位共用单次排序；聚合输出不经过死区；
- **累计量 totalizer**：`{tag}_total`，支持周期重置（先发旧值再发 0）、表读数翻转补差（`rollover`）、变化发射 + `emit_ms` 心跳；
- 运行态不落盘、重启清零；`on_change` 模式下 totalizer 输出仍全量输出（走 synthetic 通道）。
```

- [ ] **Step 2: openapi 说明**——`gateway/gateway-server/src/api/openapi.rs` 的 policy 端点 body 已是泛型 `Object`（utoipa），**无需改动**；spec §六中「openapi.json 同步 GroupPolicy schema」一项即此，确认即可。

- [ ] **Step 3: 后端全量回归**

Run: `cargo test -p gateway-core`
Expected: 全 PASS

- [ ] **Step 4: server 编译冒烟**（handler 依赖 gateway_core::GroupPolicy，加字段后确认无破坏）

Run: `cargo check -p gateway-server`
Expected: exit 0

- [ ] **Step 5: 前端构建**

Run: `cd web && npm run build`
Expected: 构建成功

- [ ] **Step 6: Commit**

```bash
git add docs/架构说明.md docs/功能实现清单.md
git commit -m "docs: 数据面聚合扩容与 totalizer 写入架构说明

Co-Authored-By: Claude Code <noreply@anthropic.com>"
```

---

## 任务依赖

Task 1 → Task 2 → Task 3 → Task 4 → Task 5 严格串行（同文件顺序演进）；Task 6 依赖 Task 3 的配置字段名（前端表单 JSON 键）；Task 7 收尾。后端 5 任务与 Task 6 在 Task 3 合入后可并行（不同文件），但同分支串行执行更稳。
