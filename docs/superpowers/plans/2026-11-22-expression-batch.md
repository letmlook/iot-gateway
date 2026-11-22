# 表达式批（hysteresis / 多条件 / rhai expr_route）实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 按已确认 spec（`docs/superpowers/specs/2026-11-22-expression-batch-design.md`）落地三特性：规则独立恢复条件（hysteresis）、多条件 all/any 组合、rhai expr_route 沙箱算子。

**Architecture:** rules.rs 状态机恢复分支按 `clear_condition` 分流；触发判定改为 logic 作用于 [主条件+extras]；gateway-flow 新增第 9 个算子 `expr_route`（全局沙箱 Engine + 节点级 AST 缓存）；SQLite rules 表加 `extra TEXT` 列打包三个新字段（NULL = 全缺省）。

**Tech Stack:** Rust（gateway-core / gateway-flow / gateway-server）、rhai 1.27、Vue3 + Element Plus。

## Global Constraints

- spec 是需求唯一来源：`docs/superpowers/specs/2026-11-22-expression-batch-design.md`（沙箱参数、状态机语义、缺位口径都在 spec，本计划代码与之一致）
- **rhai 版本锁定 `1.27`**，不开任何 feature；gateway-flow/Cargo.toml 直接声明（仓库无 `[workspace.dependencies]` 集中段）
- **缺位显式 false**：extra 点位与 clear_condition 点位缺位一律 `map_or(false, ...)`——禁止用 NaN 代缺位（`Ne` 与 NaN 组合会判 true，破坏缺测不误触发）
- **零迁移兼容**：旧规则 JSON（serde default）、旧 SQLite 行（extra 列 NULL）→ `extra_conditions=[]`、`logic=All`、`clear_condition=None`，行为与现状逐分支等价
- `OPERATOR_NAMES` 8 → 9；`tests/flows_api.rs:331` 断言与前端 fixture（`web/tests/graphMapping.test.js`）同步改 9
- gateway-flow 保持纯函数（无 tokio/无 IO；rhai 求值是纯 CPU）
- 沙箱参数定死（spec §4.3）：max_operations=10_000、max_call_levels=16、max_expr_depths=32,32、max_string_size=4_096、max_array_size=64；不注册任何自定义函数
- 提交身份 letmlook / letmlook@aliyun.com（全局 gitconfig 已设）；commit 信息中文、尾行 `Co-Authored-By: Claude Code <noreply@anthropic.com>`
- 每任务结束对应 crate `cargo test` 全绿；全量回归在 Task 4

---

### Task 1: 规则 hysteresis 独立恢复条件 + 多条件组合（core 语义 / SQLite 持久化 / API DTO）

**Files:**
- Modify: `gateway/gateway-core/src/rules.rs`（结构 + advance + validate + tests）
- Modify: `gateway/gateway-core/src/persist.rs`（建表列、迁移、读写、:2286 测试构造点）
- Modify: `gateway/gateway-core/tests/rule_engine.rs:142`（rule() 辅助补字段）
- Modify: `gateway/gateway-server/src/api/handlers.rs:2209`（CreateRuleReq + into_rule）
- Modify: `gateway/gateway-server/src/api/openapi.rs:199`（CreateRuleReq schema）

**Interfaces:**
- Produces: `gateway_core::{ClearCondition, TagCondition, Logic, RuleExtras}`（pub）；`Rule` 新字段 `clear_condition: Option<ClearCondition>` / `extra_conditions: Vec<TagCondition>` / `logic: Logic`；`Logic::default() == All`
- Consumes: 现有 `CompareOp::eval(f64, f64) -> bool`、`RuleEngine::advance` 三分支骨架（不动触发分支与未触发分支）

- [ ] **Step 1: 写失败测试（rules.rs tests 模块末尾追加）**

在 `rules.rs` 的 `mod tests` 内（`prop_value_for_tag_writes_declared_type` 之后）追加：

```rust
    // ===== 第三批：hysteresis 独立恢复条件 + 多条件组合（spec §二/§三）=====

    fn vals2(v: f64, extra: &[(&str, f64)]) -> HashMap<String, f64> {
        let mut m = vals(v);
        for (k, x) in extra {
            m.insert(k.to_string(), *x);
        }
        m
    }

    #[test]
    fn hysteresis_holds_fired_in_deadband() {
        // 触发 >80、恢复 <75（主点位）；75~80 临界带保持 fired 不重复触发
        let e = RuleEngine::new();
        let mut r = rule("r1", CompareOp::Gt, 80.0, 0, 0);
        r.clear_condition = Some(ClearCondition {
            tag_name: None,
            op: CompareOp::Lt,
            threshold: 75.0,
        });
        assert_eq!(e.advance(std::slice::from_ref(&r), &vals(85.0), 0).len(), 1);
        for t in [100u64, 200, 300] {
            assert!(
                e.advance(std::slice::from_ref(&r), &vals(78.0), t)
                    .is_empty(),
                "临界带（78∈[75,80]）必须保持 fired"
            );
        }
        let st = e.runtime("r1");
        assert!(st.fired, "临界带保持 fired");
        assert_eq!(st.fire_count, 1, "临界带不得重复触发");
        // 恢复条件满足（clear_ms=0 立即）→ 重新武装
        assert!(e.advance(std::slice::from_ref(&r), &vals(70.0), 400).is_empty());
        assert!(!e.runtime("r1").fired, "低于 75 应重新武装");
        // 再次超温可再次触发
        assert_eq!(e.advance(std::slice::from_ref(&r), &vals(85.0), 500).len(), 1);
    }

    #[test]
    fn hysteresis_recovery_bounce_resets_clear_timer() {
        // 恢复条件满足中途反弹 → clear 计时归零重计（要求持续满足）
        let e = RuleEngine::new();
        let mut r = rule("r1", CompareOp::Gt, 80.0, 0, 500);
        r.clear_condition = Some(ClearCondition {
            tag_name: None,
            op: CompareOp::Lt,
            threshold: 75.0,
        });
        assert_eq!(e.advance(std::slice::from_ref(&r), &vals(85.0), 0).len(), 1);
        e.advance(std::slice::from_ref(&r), &vals(70.0), 100); // 恢复计时开始
        e.advance(std::slice::from_ref(&r), &vals(76.0), 300); // 反弹 76≥75：clear 归零
        assert!(e.runtime("r1").fired, "反弹后仍 fired");
        e.advance(std::slice::from_ref(&r), &vals(70.0), 700); // 700-300=400 < 500
        assert!(e.runtime("r1").fired, "clear 未满 500ms 不重新武装");
        e.advance(std::slice::from_ref(&r), &vals(70.0), 800); // 800-300=500 ≥ 500
        assert!(!e.runtime("r1").fired, "恢复条件持续 500ms 后重新武装");
    }

    #[test]
    fn hysteresis_clear_condition_on_other_tag() {
        // 恢复条件监测非主点位；该点位缺位判 false（保持 fired）
        let e = RuleEngine::new();
        let mut r = rule("r1", CompareOp::Gt, 80.0, 0, 0);
        r.clear_condition = Some(ClearCondition {
            tag_name: Some("cooler_on".into()),
            op: CompareOp::Eq,
            threshold: 1.0,
        });
        e.advance(std::slice::from_ref(&r), &vals(85.0), 0);
        assert!(e.runtime("r1").fired);
        // cooler_on 缺位 → false → 保持 fired
        e.advance(std::slice::from_ref(&r), &vals(85.0), 100);
        assert!(e.runtime("r1").fired, "恢复点位缺位不得重新武装");
        // cooler_on=0 → false → 保持 fired
        e.advance(std::slice::from_ref(&r), &vals2(85.0, &[("cooler_on", 0.0)]), 200);
        assert!(e.runtime("r1").fired);
        // cooler_on=1 → true → 重新武装
        e.advance(std::slice::from_ref(&r), &vals2(85.0, &[("cooler_on", 1.0)]), 300);
        assert!(!e.runtime("r1").fired);
    }

    #[test]
    fn multi_condition_all_and_any() {
        let e = RuleEngine::new();
        // All：主 + extra 全满足才触发
        let mut r = rule("r1", CompareOp::Gt, 80.0, 0, 0);
        r.extra_conditions = vec![TagCondition {
            tag_name: "pressure".into(),
            op: CompareOp::Gt,
            threshold: 10.0,
        }];
        r.logic = Logic::All;
        assert!(
            e.advance(std::slice::from_ref(&r), &vals(85.0), 0)
                .is_empty(),
            "仅主条件满足（pressure 缺位）不得触发"
        );
        e.advance(std::slice::from_ref(&r), &vals2(85.0, &[("pressure", 5.0)]), 100);
        assert_eq!(e.runtime("r1").fire_count, 0, "pressure=5≤10 不满足");
        assert_eq!(
            e.advance(std::slice::from_ref(&r), &vals2(85.0, &[("pressure", 12.0)]), 200)
                .len(),
            1,
            "全部满足应触发"
        );
        // Any：任一满足即触发（注意新 rule id，避免状态串扰）
        let mut r2 = rule("r2", CompareOp::Lt, 80.0, 0, 0); // 主条件 temp<80
        r2.extra_conditions = vec![TagCondition {
            tag_name: "pressure".into(),
            op: CompareOp::Gt,
            threshold: 10.0,
        }];
        r2.logic = Logic::Any;
        let fires = e.advance(std::slice::from_ref(&r2), &vals2(85.0, &[("pressure", 12.0)]), 300);
        assert_eq!(fires.len(), 1, "Any：主条件 false 但 extra true 应触发");
        assert_eq!(fires[0].value, 85.0, "Firing.value 仍是主点位值");
    }

    #[test]
    fn multi_condition_missing_extra_tag_is_false() {
        let e = RuleEngine::new();
        let mut r = rule("r1", CompareOp::Gt, 10.0, 0, 0);
        r.extra_conditions = vec![TagCondition {
            tag_name: "ghost".into(),
            op: CompareOp::Ne,
            threshold: 0.0,
        }];
        r.logic = Logic::All;
        // 显式 false 而非 NaN 代缺位：Ne 与 NaN 组合会判 true，必须不得触发
        assert!(
            e.advance(std::slice::from_ref(&r), &vals(11.0), 0)
                .is_empty(),
            "extra 点位缺位必须判 false（哪怕 op=ne）"
        );
        r.logic = Logic::Any;
        assert_eq!(
            e.advance(std::slice::from_ref(&r), &vals(11.0), 100).len(),
            1,
            "Any 下缺位项 false、主条件 true → 触发"
        );
    }

    #[test]
    fn multi_condition_missing_primary_tag_does_not_advance() {
        // 主点位缺位：整条不推进（runtime 不变）——extra 在也不推进
        let e = RuleEngine::new();
        let mut r = rule("r1", CompareOp::Gt, 10.0, 0, 0);
        r.extra_conditions = vec![TagCondition {
            tag_name: "pressure".into(),
            op: CompareOp::Gt,
            threshold: 1.0,
        }];
        r.logic = Logic::Any;
        let mut only_pressure = HashMap::new();
        only_pressure.insert("pressure".to_string(), 5.0);
        assert!(e.advance(std::slice::from_ref(&r), &only_pressure, 0).is_empty());
        assert_eq!(e.runtime("r1").last_value, None, "主点位缺位不推进状态");
    }

    #[test]
    fn multi_condition_trigger_with_clear_condition_recovery() {
        // 组合（spec §3.3）：多条件触发 + 独立单条恢复条件恢复
        let e = RuleEngine::new();
        let mut r = rule("r1", CompareOp::Gt, 80.0, 0, 100);
        r.extra_conditions = vec![TagCondition {
            tag_name: "pressure".into(),
            op: CompareOp::Gt,
            threshold: 10.0,
        }];
        r.logic = Logic::All;
        r.clear_condition = Some(ClearCondition {
            tag_name: Some("cooler_on".into()),
            op: CompareOp::Eq,
            threshold: 1.0,
        });
        e.advance(std::slice::from_ref(&r), &vals2(85.0, &[("pressure", 12.0)]), 0);
        assert!(e.runtime("r1").fired);
        // 温度回落但 pressure 仍满足 → triggered 仍真 → 保持 fired 不进恢复分支
        e.advance(std::slice::from_ref(&r), &vals2(70.0, &[("pressure", 12.0)]), 100);
        assert!(e.runtime("r1").fired, "多条件仍满足时不得进恢复分支");
        // 全部不满足且 cooler_on=1 → 恢复
        e.advance(
            std::slice::from_ref(&r),
            &vals2(70.0, &[("pressure", 5.0), ("cooler_on", 1.0)]),
            200,
        );
        assert!(!e.runtime("r1").fired, "多条件不满足+恢复条件满足应重新武装");
    }

    #[test]
    fn legacy_rule_json_deserializes_with_defaults() {
        // 旧 JSON（无新字段）零迁移兼容（spec §三）
        let raw = r#"{"id":"r1","name":"legacy","source":{"south_node_id":"550e8400-e29b-41d4-a716-446655440001","group_id":"550e8400-e29b-41d4-a716-446655440002","tag_name":"temperature"},"condition":{"op":"gt","threshold":80.0},"action":{"type":"log","message":"hot"}}"#;
        let r: Rule = serde_json::from_str(raw).unwrap();
        assert!(r.extra_conditions.is_empty());
        assert_eq!(r.logic, Logic::All);
        assert!(r.clear_condition.is_none());
        // 行为回归：advance 与现状一致（立即触发）
        let e = RuleEngine::new();
        assert_eq!(e.advance(std::slice::from_ref(&r), &vals(85.0), 0).len(), 1);
    }

    #[test]
    fn rule_validation_rejects_bad_extras_and_clear() {
        let mut r = rule("r1", CompareOp::Gt, 10.0, 0, 0);
        assert!(r.validate().is_ok());
        // extra 超 8 项
        r.extra_conditions = (0..9)
            .map(|i| TagCondition {
                tag_name: format!("t{i}"),
                op: CompareOp::Gt,
                threshold: 1.0,
            })
            .collect();
        assert!(r.validate().is_err(), "extra_conditions 上限 8");
        r.extra_conditions.truncate(8);
        assert!(r.validate().is_ok());
        // extra threshold 非有限
        r.extra_conditions[0].threshold = f64::NAN;
        assert!(r.validate().is_err());
        // extra tag_name 空串
        r.extra_conditions[0].threshold = 1.0;
        r.extra_conditions[0].tag_name = "  ".into();
        assert!(r.validate().is_err());
        r.extra_conditions.clear();
        // clear_condition：threshold 非有限 / tag_name 空串
        r.clear_condition = Some(ClearCondition {
            tag_name: None,
            op: CompareOp::Lt,
            threshold: f64::INFINITY,
        });
        assert!(r.validate().is_err());
        let cc = r.clear_condition.as_mut().unwrap();
        cc.threshold = 1.0;
        cc.tag_name = Some(String::new());
        assert!(r.validate().is_err(), "clear_condition.tag_name 空串拒绝");
        let cc = r.clear_condition.as_mut().unwrap();
        cc.tag_name = Some("ok".into());
        assert!(r.validate().is_ok());
    }
```

- [ ] **Step 2: 跑测试确认编译失败**

Run: `cargo test -p gateway-core --lib rules 2>&1 | tail -5`
Expected: 编译错误（`ClearCondition`/`TagCondition`/`Logic` 未定义、Rule 缺字段）

- [ ] **Step 3: rules.rs 实现结构与语义**

3a. 在 `RuleCondition` 定义后追加三个新类型（模块 doc 注释的规则语义说明同步补 hysteresis 一句：`clear_condition` 配置时恢复判定以恢复条件为准、要求持续满足）：

```rust
/// 追加触发条件（与主条件组成条件组，按 rule.logic 求值；spec §三）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TagCondition {
    pub tag_name: String,
    pub op: CompareOp,
    pub threshold: f64,
}

/// 独立恢复条件（hysteresis；spec §二）。None = 现状语义（触发条件不满足 + clear_ms 去抖）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClearCondition {
    /// 恢复条件监测的点位；None = 主点位（source.tag_name）
    #[serde(default)]
    pub tag_name: Option<String>,
    pub op: CompareOp,
    pub threshold: f64,
}

/// 条件组逻辑（spec §三）；单条件时 all/any 等价
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Logic {
    All,
    Any,
}

impl Default for Logic {
    fn default() -> Self {
        Logic::All
    }
}

/// rules 表 extra 列的 JSON 形状（三个新字段打包存储；persist 层用）
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RuleExtras {
    #[serde(default)]
    pub extra_conditions: Vec<TagCondition>,
    #[serde(default)]
    pub logic: Logic,
    #[serde(default)]
    pub clear_condition: Option<ClearCondition>,
}
```

3b. `Rule` 结构体在 `action` 字段前追加（`tenant_id` 之前的位置无关紧要，保持字段顺序清晰即可）：

```rust
    /// 追加触发条件（上限 8，validate 保证）；空 = 现状单条件
    #[serde(default)]
    pub extra_conditions: Vec<TagCondition>,
    /// 条件组逻辑；缺省 all
    #[serde(default)]
    pub logic: Logic,
    /// 独立恢复条件（hysteresis）；None = 触发条件不满足即恢复计时
    #[serde(default)]
    pub clear_condition: Option<ClearCondition>,
```

3c. `Rule::validate()` 在 `clear_ms > MAX_MS` 检查之后、action match 之前追加：

```rust
        if self.extra_conditions.len() > 8 {
            return Err("extra_conditions must have at most 8 items".to_string());
        }
        for c in &self.extra_conditions {
            if c.tag_name.trim().is_empty() {
                return Err("extra_conditions[].tag_name must not be empty".to_string());
            }
            if !c.threshold.is_finite() {
                return Err("extra_conditions[].threshold must be a finite number".to_string());
            }
        }
        if let Some(cc) = &self.clear_condition {
            if cc.tag_name.as_deref().map(str::trim) == Some("") {
                return Err("clear_condition.tag_name must not be empty".to_string());
            }
            if !cc.threshold.is_finite() {
                return Err("clear_condition.threshold must be a finite number".to_string());
            }
        }
```

3d. `advance()`：保留主点位取值 `continue` 与 `st.last_value` 赋值不变；将 `if rule.condition.op.eval(...)` 替换为 triggered 组合判定，并重写恢复分支。替换范围从 `if rule.condition.op.eval(value, rule.condition.threshold) {` 到 else-if/else 链结束：

```rust
            // 触发判定（spec §三）：logic 作用于 [主条件, ...extra_conditions]。
            // extra 点位缺位判 false（缺测不误触发；显式 false 而非 NaN 代缺位——
            // Ne 与 NaN 组合会判 true）；主点位缺位在上方 continue，整条不推进。
            let triggered = {
                let mut parts = vec![rule.condition.op.eval(value, rule.condition.threshold)];
                for c in &rule.extra_conditions {
                    parts.push(
                        values
                            .get(&c.tag_name)
                            .map_or(false, |v| c.op.eval(*v, c.threshold)),
                    );
                }
                match rule.logic {
                    Logic::All => parts.iter().all(|&p| p),
                    Logic::Any => parts.iter().any(|&p| p),
                }
            };

            if triggered {
                // 满足：清理恢复计时
                st.clear_since_ms = None;
                let since = *st.since_ms.get_or_insert(now_ms);
                let held_ms = now_ms.saturating_sub(since);
                if held_ms >= rule.for_ms && !st.fired {
                    st.fired = true;
                    st.fire_count += 1;
                    st.last_fired_at = Some(chrono::Utc::now().to_rfc3339());
                    firings.push(Firing {
                        rule: rule.clone(),
                        value,
                    });
                }
            } else if st.fired {
                // 恢复判定（spec §二）：配了 clear_condition 时以恢复条件为准，
                // 且要求恢复条件持续满足（不满足期间保持 fired、clear 计时归零）；
                // 未配时进入本分支即视为恢复（现状语义不变）。
                let cleared = match &rule.clear_condition {
                    Some(cc) => {
                        let tag = cc.tag_name.as_deref().unwrap_or(&rule.source.tag_name);
                        values.get(tag).map_or(false, |v| cc.op.eval(*v, cc.threshold))
                    }
                    None => true,
                };
                if cleared {
                    let clear_since = *st.clear_since_ms.get_or_insert(now_ms);
                    if now_ms.saturating_sub(clear_since) >= rule.clear_ms {
                        st.fired = false;
                        st.clear_since_ms = None;
                        st.since_ms = None;
                    }
                } else {
                    st.clear_since_ms = None;
                }
            } else {
                // 未触发且不满足：重置保持计时，下次满足重新开始计 for_ms
                st.since_ms = None;
                st.clear_since_ms = None;
            }
```

（`st.last_value = Some(value);` 与 `let mut st = ...` 两行保持原样不动。）

- [ ] **Step 4: 修 tests/rule_engine.rs 构造点**

`gateway/gateway-core/tests/rule_engine.rs` 的 `fn rule(...)`（:142）在 `tenant_id: "default".to_string(),` 之前追加：

```rust
        extra_conditions: Vec::new(),
        logic: gateway_core::Logic::All,
        clear_condition: None,
```

（该文件 use 区如无 `gateway_core::Logic` 引用问题，直接全路径即可。）

- [ ] **Step 5: 跑 core 测试确认通过**

Run: `cargo test -p gateway-core 2>&1 | tail -15`
Expected: 全部 PASS（含新增 8 个测试 + 既有 property 测试——property 测试无 clear_condition，走 `None => true` 现状路径逐分支等价）

- [ ] **Step 6: persist.rs 持久化**

6a. 建表 SQL（`CREATE TABLE IF NOT EXISTS rules`，:64 附近）在 `tenant_id TEXT NOT NULL DEFAULT 'default'` 后加一列：

```sql
  extra TEXT,
```

6b. 迁移（:233-245 的 tenant_id 迁移块之后追加，同模式）：

```rust
        // 迁移 rules 表：extra 列（表达式批三字段打包 JSON；NULL = 全缺省）
        let rules_has_extra: bool = conn
            .query_row("PRAGMA table_info(rules)", [], |r| {
                let col: String = r.get(1)?;
                Ok(col == "extra")
            })
            .unwrap_or(false);
        if !rules_has_extra {
            conn.execute("ALTER TABLE rules ADD COLUMN extra TEXT", [])?;
        }
```

6c. 读取侧（:543 SELECT 与 query_map 闭包、:566 解构）：SELECT 列表尾加 `, extra`；闭包尾加 `row.get::<_, Option<String>>(12)?`；解构尾加 `extra_json`；构造 Rule 处在 `tenant_id,` 前追加：

```rust
            // extra 列：三新字段打包 JSON；NULL / 空串 = 全缺省
            let extras: crate::rules::RuleExtras = match extra_json {
                Some(s) if !s.trim().is_empty() => serde_json::from_str(&s).map_err(|e| {
                    PersistError::Validation(format!("rule {}: bad extras: {}", id, e))
                })?,
                _ => crate::rules::RuleExtras::default(),
            };
```

并在 struct literal 中补 `extra_conditions: extras.extra_conditions, logic: extras.logic, clear_condition: extras.clear_condition,`（注意：`id` 在错误信息里使用后即被 move，若借用冲突则把错误格式化行改为使用 move 前的 clone 或先格式化字符串——按编译器指引调整，语义不变）。

6d. 写入侧（:850 INSERT）：列清单与 VALUES 占位符各加一项 `extra`（第 13 位）、`?13`；ON CONFLICT UPDATE 加 `extra = excluded.extra`；`params![]` 尾部（`r.tenant_id` 之后）加：

```rust
                rule_extra_json(r),
```

并在 persist.rs 内（rules 写入 impl 附近）加私有辅助：

```rust
/// 三新字段打包进 extra 列；全缺省存 NULL（旧行紧凑，spec 零迁移兼容）
fn rule_extra_json(r: &crate::rules::Rule) -> Option<String> {
    let all_default = r.extra_conditions.is_empty()
        && r.logic == crate::rules::Logic::All
        && r.clear_condition.is_none();
    if all_default {
        return None;
    }
    serde_json::to_string(&crate::rules::RuleExtras {
        extra_conditions: r.extra_conditions.clone(),
        logic: r.logic,
        clear_condition: r.clear_condition.clone(),
    })
    .ok()
}
```

6e. `persist.rs:2286` 测试构造的 `Rule { ... }` 在 `tenant_id: "tenant-a".to_string(),` 前补 `extra_conditions: Vec::new(), logic: crate::rules::Logic::All, clear_condition: None,`。

- [ ] **Step 7: server DTO**

7a. `handlers.rs` `CreateRuleReq`（:2209）在 `clear_ms` 后追加：

```rust
    #[serde(default)]
    pub extra_conditions: Vec<gateway_core::TagCondition>,
    /// all | any；缺省 all
    #[serde(default)]
    pub logic: gateway_core::Logic,
    #[serde(default)]
    pub clear_condition: Option<gateway_core::ClearCondition>,
```

`into_rule`（:2235）struct literal 同步补 `extra_conditions: self.extra_conditions, logic: self.logic, clear_condition: self.clear_condition,`。

7b. `openapi.rs` `CreateRuleReq`（:199）在 `clear_ms` 后追加（文档 schema，类型从宽）：

```rust
    /// 追加触发条件数组，元素 {tag_name, op, threshold}；上限 8
    #[schema(value_type = Object)]
    pub extra_conditions: Vec<serde_json::Value>,
    /// 条件组逻辑：all | any（缺省 all）
    pub logic: Option<String>,
    /// 独立恢复条件 {tag_name?, op, threshold}
    #[schema(value_type = Object)]
    pub clear_condition: Option<serde_json::Value>,
```

- [ ] **Step 8: 持久化往返测试（persist.rs tests 模块追加）**

```rust
    #[test]
    fn rule_extra_fields_round_trip_through_sqlite() {
        // extra 列读写往返；全缺省规则存 NULL（旧行紧凑）
        let dir = tempdir();
        let path = dir.path().join("extra-rt.db");
        let node = NodeId::new();
        let group = GroupId::new();
        let mut r = crate::rules::Rule {
            id: "rx".into(),
            name: "hysteresis".into(),
            enabled: true,
            source: crate::rules::RuleSource {
                south_node_id: node,
                group_id: group,
                tag_name: "temperature".into(),
            },
            condition: crate::rules::RuleCondition {
                op: crate::rules::CompareOp::Gt,
                threshold: 80.0,
            },
            for_ms: 0,
            clear_ms: 500,
            action: crate::rules::RuleAction::Log { message: "hot".into() },
            tenant_id: "default".into(),
            extra_conditions: vec![crate::rules::TagCondition {
                tag_name: "pressure".into(),
                op: crate::rules::CompareOp::Gt,
                threshold: 10.0,
            }],
            logic: crate::rules::Logic::All,
            clear_condition: Some(crate::rules::ClearCondition {
                tag_name: Some("cooler_on".into()),
                op: crate::rules::CompareOp::Lt,
                threshold: 75.0,
            }),
        };
        let mut snap = Snapshot { version: SNAPSHOT_VERSION, nodes: vec![], rules: vec![], ..Default::default() };
        // 若 Snapshot 无 Default：按本文件既有测试构造 Snapshot 的方式补齐空集合字段
        snap.nodes = vec![];
        snap.rules = vec![r.clone()];
        {
            let db = Db::open(&path, IntegrityMode::Off).unwrap();
            db.save_snapshot(&snap, &PersistOptions::default()).unwrap();
        }
        let db = Db::open(&path, IntegrityMode::Off).unwrap();
        let loaded = db.load_snapshot(None).unwrap().unwrap();
        assert_eq!(loaded.rules.len(), 1);
        let back = &loaded.rules[0];
        assert_eq!(back.extra_conditions.len(), 1);
        assert_eq!(back.extra_conditions[0].tag_name, "pressure");
        assert_eq!(back.logic, crate::rules::Logic::All);
        assert_eq!(
            back.clear_condition.as_ref().unwrap().tag_name.as_deref(),
            Some("cooler_on")
        );
        assert_eq!(back.clear_condition.as_ref().unwrap().threshold, 75.0);
        let _ = r; // 保持 clone 源存活至断言结束
    }
```

注意：本步测试代码中 `Snapshot` 的构造、`PersistOptions`、`tempdir` 辅助**以本文件既有测试的实际形状为准**（Step 8 前先读 `persist.rs` tests 模块里最近的 snapshot 往返测试，照抄其构造辅助与断言风格；上面的代码是语义规格，不是逐字模板——文件里已有的辅助函数必须复用，不要新造同名辅助）。若既有测试没有 snapshot 往返先例，则在本文件 tests 内照 :2123 的手工建库风格写：手工建新 schema 库 + `rule_extra_json` 直接单测（断言全缺省 → None、带 extras → Some(json) 且 round-trip 经 `serde_json::from_str::<RuleExtras>` 还原相等）。

- [ ] **Step 9: 全链路验证**

Run: `cargo test -p gateway-core && cargo test -p gateway-server 2>&1 | tail -10`
Expected: core 与 server 全部 PASS（server 侧含规则 API 既有测试）

- [ ] **Step 10: Commit**

```bash
git add -A && git commit -m "feat(rules): hysteresis 独立恢复条件 + 多条件 all/any 组合（SQLite extra 列持久化）"
```

---

### Task 2: rhai expr_route 算子（沙箱定死 + AST 缓存 + 发布门禁）

**Files:**
- Modify: `gateway/gateway-flow/Cargo.toml`（加 rhai 依赖）
- Modify: `gateway/gateway-flow/src/operators.rs`（配置/引擎/求值/校验/端口/清单/evaluate_node 臂/tests）
- Modify: `gateway/gateway-flow/src/engine.rs`（FlowState 加 script_asts）
- Modify: `gateway/gateway-server/tests/flows_api.rs:331`（算子数断言 8→9）

**Interfaces:**
- Consumes: Task 1 无依赖（并行可做）；现有 `value_by_name` 不复用（expr_route 要全点位 map，新写 `tags_map`）
- Produces: `gateway_flow::{ExprRouteConfig, CachedScript}`（pub）；`FlowState.script_asts: HashMap<Uuid, CachedScript>`；`OPERATOR_NAMES: [&str; 9]`

- [ ] **Step 1: 加依赖并确认版本**

`gateway/gateway-flow/Cargo.toml` `[dependencies]` 尾部加：

```toml
rhai = "1.27"
```

Run: `cargo check -p gateway-flow 2>&1 | tail -3`（外网拉取需代理：`CARGO_HTTP_PROXY=http://127.0.0.1:7892`，已写入记忆；若 crates.io index 慢多等一会）
Expected: 下载 rhai 1.27.x 并编译通过（API 名不符编译器会立刻指出——rhai 1.x 的 `set_max_operations/set_max_call_levels/set_max_expr_depths/set_max_string_size/set_max_array_size/compile` 是稳定 API）

- [ ] **Step 2: 写失败测试**

2a. `operators.rs` tests 模块（`alarm_to_data_validates_and_has_out_port` 之后）追加局部构造辅助（lib.rs tests 的 `group_data` 在本文件不可见，自建不动 lib.rs）：

```rust
    // ===== expr_route（spec §四）=====

    fn group_data_f(temp: f64) -> gateway_sdk::GroupData {
        let mut names = std::collections::HashMap::new();
        let tid = gateway_sdk::TagId::new();
        names.insert(tid, "temperature".to_string());
        gateway_sdk::GroupData {
            node_id: gateway_sdk::NodeId::new(),
            group_id: gateway_sdk::GroupId::new(),
            ts: Utc::now(),
            values: vec![(tid, DataValue::Float64(temp))],
            node_name: Some("sim".into()),
            group_name: Some("g".into()),
            tag_names: Some(names),
        }
    }

    #[test]
    fn expr_route_config_validation_rejects_syntax_errors() {
        assert!(validate_config("expr_route", &serde_json::json!({"expression": ""})).is_err());
        assert!(
            validate_config("expr_route", &serde_json::json!({"expression": "tags.temp >"}))
                .is_err(),
            "语法错误必须拒绝（publish 门禁）"
        );
        assert!(validate_config("expr_route", &serde_json::json!({})).is_err());
        assert!(
            validate_config("expr_route", &serde_json::json!({"expression": "tags.temperature > 50"}))
                .is_ok()
        );
    }

    #[test]
    fn expr_route_has_true_false_ports() {
        let spec = PortSpec::operator("expr_route");
        assert_eq!(spec.inputs, &["in"]);
        assert_eq!(spec.outputs, &["true", "false"]);
    }

    #[test]
    fn expr_route_eval_routes_and_recompiles_on_change() {
        let mut state = crate::engine::FlowState::default();
        let nid = Uuid::new_v4();
        let gd = group_data_f(60.0);
        let cfg1 = ExprRouteConfig { expression: "tags.temperature > 50".into() };
        let (t, f) = eval_expr_route(&cfg1, nid, &mut state.script_asts, &gd);
        assert!(t.is_some() && f.is_none(), "60>50 走 true 端");
        let cfg2 = ExprRouteConfig { expression: "tags.temperature < 50".into() };
        let (t, f) = eval_expr_route(&cfg2, nid, &mut state.script_asts, &gd);
        assert!(t.is_none() && f.is_some(), "换表达式后重新编译且语义生效");
    }

    #[test]
    fn expr_route_missing_tag_and_non_bool_go_false() {
        let mut state = crate::engine::FlowState::default();
        let nid = Uuid::new_v4();
        let gd = group_data_f(60.0);
        // 缺位 tag 访问 → 运行期错误 → false 端
        let cfg = ExprRouteConfig { expression: "tags.ghost > 10".into() };
        let (t, f) = eval_expr_route(&cfg, nid, &mut state.script_asts, &gd);
        assert!(t.is_none() && f.is_some());
        // 非 bool 返回 → false 端
        let cfg = ExprRouteConfig { expression: "tags.temperature".into() };
        let (t, f) = eval_expr_route(&cfg, nid, &mut state.script_asts, &gd);
        assert!(t.is_none() && f.is_some());
    }

    #[test]
    fn expr_route_infinite_loop_hits_operation_limit() {
        // 沙箱生效证明：while true {} 超操作数走 false 端而非挂死
        let mut state = crate::engine::FlowState::default();
        let nid = Uuid::new_v4();
        let gd = group_data_f(60.0);
        let cfg = ExprRouteConfig { expression: "while true {}".into() };
        let (t, f) = eval_expr_route(&cfg, nid, &mut state.script_asts, &gd);
        assert!(t.is_none() && f.is_some(), "超操作数必须走 false 端");
    }
```

- [ ] **Step 3: 跑测试确认编译失败**

Run: `cargo test -p gateway-flow 2>&1 | tail -5`
Expected: `ExprRouteConfig`/`eval_expr_route` 未定义

- [ ] **Step 4: operators.rs 实现**

4a. 文件头 `use` 区加 `use std::sync::OnceLock;`；模块 doc 注释的算子清单补 `expr_route`（脚本算子沙箱专项已落地，删去「rhai 明确不在 MVP」句，改为「expr_route（rhai 沙箱，spec §四）」）。

4b. `OPERATOR_NAMES` 改为：

```rust
pub const OPERATOR_NAMES: [&str; 9] = [
    "threshold_alarm",
    "state_change_alarm",
    "route",
    "alarm_to_data",
    "sink_north",
    "sink_alarm",
    "rate_of_change",
    "deduplicate",
    "expr_route",
];
```

4c. 配置与沙箱（放在 `RateOfChangeConfig` 之后）：

```rust
/// expr_route 配置：rhai 布尔表达式对组内点位求值路由（spec §四）。
#[derive(Debug, Deserialize)]
pub struct ExprRouteConfig {
    pub expression: String,
}

/// expr_route 的 AST + 引擎缓存条目（FlowState.script_asts；spec §4.5）。
pub struct CachedScript {
    /// 缓存 key：config 的 expression 字符串
    pub expression: String,
    pub ast: rhai::AST,
    /// 每节点私有引擎与 AST 同生命周期缓存（rhai Engine 求值状态在 Scope，可复用）
    pub engine: rhai::Engine,
    /// spec §4.4：首次错误 warn，后续 debug；求值成功后重置
    pub warned: bool,
}

impl std::fmt::Debug for CachedScript {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CachedScript")
            .field("expression", &self.expression)
            .field("warned", &self.warned)
            .finish_non_exhaustive()
    }
}

/// expr_route 专用沙箱引擎（spec §4.3 边界定死；默认无文件/目录/网络/系统访问，不注册自定义函数）。
pub fn rhai_engine() -> rhai::Engine {
    let mut engine = rhai::Engine::new();
    engine.set_max_operations(10_000); // 防死循环（超限 → 求值错误 → false 端）
    engine.set_max_call_levels(16); // 防深递归
    engine.set_max_expr_depths(32, 32); // 防深嵌套表达式
    engine.set_max_string_size(4_096); // 防内存爆炸
    engine.set_max_array_size(64);
    engine
}
```

4d. `validate_config` 在 `"deduplicate"` 臂后追加：

```rust
        "expr_route" => {
            let c: ExprRouteConfig = serde_json::from_value(config.clone())
                .map_err(|e| format!("invalid config: {e}"))?;
            if c.expression.trim().is_empty() {
                return Err("expression is required".into());
            }
            // 发布门禁：编译校验（只编译不执行），语法错误拒绝发布
            rhai_engine()
                .compile(&c.expression)
                .map_err(|e| format!("expression compile error: {e}"))?;
            Ok(())
        }
```

4e. `PortSpec::operator` 在 `"deduplicate"` 臂后追加：

```rust
            "expr_route" => PortSpec {
                inputs: &["in"],
                outputs: &["true", "false"],
            },
```

4f. 求值函数（`eval_deduplicate` 之后）：

```rust
/// GroupData → rhai Map（tag_name → Dynamic；spec §4.2 数据暴露）。
/// 数值全统一 f64；Bool→bool；String→String；Bytes→UNIT（null）。
fn tags_map(gd: &GroupData) -> rhai::Map {
    let mut map = rhai::Map::new();
    let Some(names) = &gd.tag_names else {
        return map;
    };
    for (tag_id, value) in &gd.values {
        let Some(name) = names.get(tag_id) else {
            continue;
        };
        let d = match value {
            DataValue::Bool(b) => rhai::Dynamic::from(*b),
            DataValue::Int8(v) => rhai::Dynamic::from(*v as f64),
            DataValue::Int16(v) => rhai::Dynamic::from(*v as f64),
            DataValue::Int32(v) => rhai::Dynamic::from(*v as f64),
            DataValue::Int64(v) => rhai::Dynamic::from(*v as f64),
            DataValue::UInt8(v) => rhai::Dynamic::from(*v as f64),
            DataValue::UInt16(v) => rhai::Dynamic::from(*v as f64),
            DataValue::UInt32(v) => rhai::Dynamic::from(*v as f64),
            DataValue::UInt64(v) => rhai::Dynamic::from(*v as f64),
            DataValue::Float32(v) => rhai::Dynamic::from(*v as f64),
            DataValue::Float64(v) => rhai::Dynamic::from(*v),
            DataValue::String(s) => rhai::Dynamic::from(s.clone()),
            DataValue::Bytes(_) => rhai::Dynamic::UNIT,
        };
        map.insert(name.clone().into(), d);
    }
    map
}

/// expr_route 求值（spec §4.4）：Ok(bool) 按值路由 true/false 端；
/// 求值错误（缺位 tag、类型不匹配、超操作数）或非 bool 返回一律走 false 端（缺测不误触发）。
/// AST 缓存：expression 变更即重编译并替换条目。
pub fn eval_expr_route(
    cfg: &ExprRouteConfig,
    node_id: Uuid,
    state: &mut HashMap<Uuid, CachedScript>,
    gd: &GroupData,
) -> (Option<FlowPayload>, Option<FlowPayload>) {
    let hit = state
        .get(&node_id)
        .map(|c| c.expression == cfg.expression)
        .unwrap_or(false);
    if !hit {
        match rhai_engine().compile(&cfg.expression) {
            Ok(ast) => {
                state.insert(
                    node_id,
                    CachedScript {
                        expression: cfg.expression.clone(),
                        ast,
                        engine: rhai_engine(),
                        warned: false,
                    },
                );
            }
            Err(e) => {
                // publish 门禁已拦语法错误；运行期防御走 false 端
                tracing::debug!(node = %node_id, "expr_route compile error: {e}");
                return (None, Some(FlowPayload::Data(gd.clone())));
            }
        }
    }
    let entry = state.get_mut(&node_id).expect("cache entry just ensured");
    let mut scope = rhai::Scope::new();
    scope.push("tags", rhai::Dynamic::from_map(tags_map(gd)));
    let result: Result<rhai::Dynamic, _> =
        entry.engine.eval_ast_with_scope(&mut scope, &entry.ast);
    let route = match result {
        Ok(d) => d.as_bool().map_err(|e| e.to_string()),
        Err(e) => Err(e.to_string()),
    };
    match route {
        Ok(true) => {
            entry.warned = false;
            (Some(FlowPayload::Data(gd.clone())), None)
        }
        Ok(false) => {
            entry.warned = false;
            (None, Some(FlowPayload::Data(gd.clone())))
        }
        Err(msg) => {
            if entry.warned {
                tracing::debug!(node = %node_id, "expr_route eval error: {msg}");
            } else {
                tracing::warn!(node = %node_id, "expr_route eval error: {msg}");
                entry.warned = true;
            }
            (None, Some(FlowPayload::Data(gd.clone())))
        }
    }
}
```

4g. `evaluate_node` 的 match 在 `"deduplicate"` 臂后追加：

```rust
        "expr_route" => {
            let cfg: ExprRouteConfig = serde_json::from_value(config.clone())
                .map_err(|e| FlowError::Operator(operator.into(), e.to_string()))?;
            let (t, f) = eval_expr_route(&cfg, node_id, &mut state.script_asts, &gd);
            let mut out = Vec::new();
            if let Some(t) = t {
                out.push(("true", t));
            }
            if let Some(f) = f {
                out.push(("false", f));
            }
            Ok(out)
        }
```

- [ ] **Step 5: engine.rs FlowState 加字段**

`FlowState`（engine.rs:18）`last_frames` 之后追加：

```rust
    /// expr_route AST 缓存：节点 id → 缓存条目（spec §4.5）。不落盘，重启清零。
    pub script_asts: HashMap<Uuid, crate::operators::CachedScript>,
```

- [ ] **Step 6: 跑 flow 测试**

Run: `cargo test -p gateway-flow 2>&1 | tail -8`
Expected: 全部 PASS（既有 8 算子测试不受影响；新增 5 个 expr_route 测试通过）

- [ ] **Step 7: flows_api 断言对齐**

`gateway/gateway-server/tests/flows_api.rs:331`：`8` → `9`，注释改为 `"算子 9 个（MVP 6 + 流算子第二批 2 + expr_route，表达式批）"`。

- [ ] **Step 8: server 全量测试**

Run: `cargo test -p gateway-server 2>&1 | tail -10`
Expected: 全部 PASS（含 flows_api 的 9 算子断言）

- [ ] **Step 9: Commit**

```bash
git add -A && git commit -m "feat(flow): rhai expr_route 沙箱算子（AST 缓存 + 求值错误走 false 端 + 发布编译门禁）"
```

---

### Task 3: 前端 expr_route 表单 + fixture

**Files:**
- Modify: `web/src/utils/graphMapping.js:128`（defaultConfig case）
- Modify: `web/src/views/FlowEditor.vue:156`（OPS_WITH_FORM）与模板（:435 rate_of_change 模板后）
- Modify: `web/src/locales/zh.js` / `web/src/locales/en.js`（flowsEditor 段，:647 附近）
- Modify: `web/tests/graphMapping.test.js`（fixture 9 项 + defaultConfig 测试）

**Interfaces:**
- Consumes: Task 2 的端口形状 `expr_route: inputs [in] / outputs [true, false]`、配置字段 `{expression}`
- Produces: 编辑器中 expr_route 节点可配置 expression 并发布

- [ ] **Step 1: 更新测试（先失败）**

`web/tests/graphMapping.test.js`：

1a. OPERATORS fixture（:17 注释与数组）改为 9 项，注释改 `// /flows/operators 的真实返回形状(9 个算子 = MVP 6 + 流算子第二批 rate_of_change/deduplicate + 表达式批 expr_route)`，数组尾加：

```js
  { name: 'expr_route', inputs: ['in'], outputs: ['true', 'false'] },
```

1b. `defaultConfig 新算子` 测试（:156）追加断言：

```js
  assert.deepEqual(defaultConfig('expr_route'), { expression: '' })
```

Run: `cd web && npm run test:unit 2>&1 | tail -5` → Expected: defaultConfig 测试 FAIL（case 未加）

- [ ] **Step 2: 实现**

2a. `graphMapping.js` defaultConfig 在 `case 'deduplicate':` 前加：

```js
    case 'expr_route':
      return { expression: '' }
```

2b. `FlowEditor.vue` :156 `OPS_WITH_FORM` 尾加 `'expr_route'`。

2c. `FlowEditor.vue` 模板在 rate_of_change 的 `</template>`（:435）之后、sink_north 模板之前插入：

```vue
            <!-- expr_route -->
            <template v-else-if="panelMode === 'expr_route'">
              <el-form-item :label="t('flowsEditor.expression')" required>
                <el-input
                  v-model="selectedNode.data.config.expression"
                  type="textarea"
                  :rows="4"
                  placeholder="tags.temperature > 80 && tags.pressure > 10"
                />
              </el-form-item>
              <div class="editor-hint">{{ t('flowsEditor.expressionHint') }}</div>
            </template>
```

2d. `zh.js` flowsEditor 段（`op_rate_of_change` 附近）加：

```js
    op_expr_route: '表达式路由',
    expression: '表达式（rhai）',
    expressionHint: '变量 tags：点位名 → 值；表达式须返回布尔值；求值错误或非布尔返回走 false 端；语法错误会在发布时被拒绝',
```

`en.js` 同位置加：

```js
    op_expr_route: 'Expression route',
    expression: 'Expression (rhai)',
    expressionHint: 'Variable `tags`: tag name → value; must return a boolean; eval errors and non-boolean results route to false; syntax errors are rejected at publish',
```

- [ ] **Step 3: 验证**

Run: `cd web && npm run test:unit && npm run build 2>&1 | tail -5`
Expected: 单测全绿 + build 成功

- [ ] **Step 4: Commit**

```bash
git add -A && git commit -m "feat(web): 流程编辑器 expr_route 表单 + fixture 对齐 9 算子 + i18n"
```

---

### Task 4: 文档同步 + 全量回归 + 真机冒烟

**Files:**
- Modify: `docs/架构说明.md`（:229 流算子清单加 expr_route）
- Modify: `docs/功能实现清单.md`（:72 规则引擎行补 hysteresis/多条件）

**Interfaces:**
- Consumes: Task 1-3 全部完成后的行为

- [ ] **Step 1: 文档**

1a. `docs/架构说明.md` 流算子清单（:229 起）`deduplicate` 行后加：

```markdown
- `expr_route`（表达式批）：rhai 布尔表达式路由（route 的表达式版）——变量 `tags` 暴露组内点位（数值统一 f64、Bool→bool、String→String、Bytes→null）；**沙箱定死**：max_operations=10000、call_levels=16、expr_depths=32、string=4096、array=64，默认无文件/网络/系统访问且不注册自定义函数；求值错误、超操作数或非布尔返回一律走 false 端（缺测不误触发），同节点首错 warn 后续 debug；AST 按节点缓存，表达式变更自动重编译；publish 时编译校验拒绝语法错误。
```

1b. `docs/功能实现清单.md` :72 规则引擎行，在「边沿触发 + `for_ms`/`clear_ms` 去抖」后补：

```markdown
；**hysteresis 独立恢复条件**（`clear_condition`：触发/恢复阈值分离，临界带保持 fired 不重复动作，恢复条件须持续满足 `clear_ms`、中途反弹重新计时）与**多条件组合**（`extra_conditions` ≤8 项 + `logic: all|any` 作用于主+追加条件；extra 点位缺位判 false，主点位缺位整条不推进）；随规则存 SQLite `extra` 列（NULL=旧格式零迁移兼容）
```

- [ ] **Step 2: 全量回归**

Run: `cargo test --workspace 2>&1 | tail -5 && cd web && npm run test:unit && npm run build 2>&1 | tail -3`
Expected: workspace 全绿 + 前端单测/build 通过

- [ ] **Step 3: 真机冒烟（gateway-smoke）**

用 `.claude/launch.json` 的 `gateway-smoke`（`GATEWAY_DATA_DIR=/tmp/gw-smoke-data GATEWAY_DISABLE_AUTH=1 GATEWAY_PORT=4000`）。管理员初始密码在 `/tmp/gw-smoke-data/.admin_initial_password`（如启用鉴权）。步骤（先清旧库验证全新建库路径，再验证迁移路径）：

1. `rm -rf /tmp/gw-smoke-data` 后启动（release 二进制需先 `cargo build --release`）
2. 建 sim 南向节点 + 组 + 两点位（temperature/pressure，POST `/api/nodes` → `/api/nodes/:id/groups` → `/api/nodes/:id/tags/batch`），启动节点
3. **expr_route 门禁**：建流程 source → expr_route（故意坏表达式 `tags.temperature >`）→ sink_alarm，POST `/api/flows/:id/publish` → 期望 **400**（compile error）
4. 修正表达式为 `tags.temperature > 50` 后 publish → 期望 200；sim 值波动下观察 `GET /api/alarms/events` 出现告警事件（true 端 sink_alarm 生效）；改表达式为 `tags.temperature < 50` 重新发布（PUT /api/flows/:id），观察告警语义反转（AST 重编译生效）
5. **hysteresis 规则**：POST `/api/rules` 带 `clear_condition`（主 temperature gt 80，恢复 lt 75，action log）→ 201；带 9 项 `extra_conditions` → 400（上限校验生效）
6. **重启持久化**：停网关重启，`GET /api/rules` 确认 hysteresis 规则三字段完整回读（extra 列往返）
7. `GET /api/flows/operators` 确认 9 个算子

冒烟中 flow 创建请求体形状：`POST /api/flows` `{"name": "...", "nodes": [...], "edges": [...]}`（顶层 nodes/edges，**不是** graph_json——第二批踩坑记录）。节点 id 用随机 UUID，edges 按端口连（source out → expr_route in；expr_route true/false → sink_alarm in）。

- [ ] **Step 4: 文档 Commit**

```bash
git add -A && git commit -m "docs: 表达式批文档同步（规则 hysteresis/多条件 + expr_route 沙箱算子）"
```

---

## Self-Review 记录（写计划时已核）

- spec 覆盖：§二（Task 1 Step 3d + 3 个 hysteresis 测试）、§三（Step 3d triggered 组合 + 4 个多条件测试 + 兼容测试）、§四（Task 2 全部）、§五（Task 1 validate + Task 2 validate_config）、§六测试计划 1-7 全部有对位测试、§六前端/文档（Task 3/4）
- 一致性：`CachedScript` 字段（expression/ast/warned + engine 实现缓存）与 spec §4.5 语义一致（spec 三字段为语义规格，engine 字段是实现所需缓存，未新增语义）；缺位口径三处（advance/恢复条件/tags_map）统一显式 false/UNIT
- 类型一致：`eval_expr_route(cfg, node_id, &mut HashMap<Uuid, CachedScript>, gd)` 在 Step 2 测试与 Step 4 实现签名一致；`state.script_asts` 类型匹配
- 已知风险：rhai 1.27 API 名（编译器兜底，Step 1 即验证）；persist.rs Step 8 的 Snapshot 构造以文件既有测试形状为准（已注明）
