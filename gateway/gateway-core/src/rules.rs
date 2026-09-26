//! 规则引擎：南向点位条件 → 动作。
//!
//! # 语义（对外承诺，测试据此编写）
//!
//! 每条规则绑定一个南向点位（`source`），条件为「点位值 OP 阈值」。触发规则：
//!
//! 1. 条件**从"不满足"变为"满足"并持续 `for_ms`** → 触发一次；
//! 2. 持续满足期间**不重复触发**；
//! 3. 条件恢复（不满足）并持续 `clear_ms` 后，才允许下一次触发；
//!    —— 中途再次满足会重置恢复计时，避免抖动导致动作被反复执行；
//! 4. `for_ms = 0` 表示满足即触发；`clear_ms = 0` 表示恢复即重新武装。
//!
//! # 为什么用「边沿触发」而不是「电平触发」
//!
//! 采集是周期性的：若按电平触发，"温度 > 80 就写一个值"会变成每个采集周期都写一次寄存器，
//! 对 PLC 是灾难。因此默认只在条件成立的那一刻动作一次，直到条件真正恢复后才会再次动作。
//!
//! # 状态
//!
//! 运行期状态（何时开始满足、是否已触发、触发次数）不落盘：规则本身随配置持久化，
//! 重启后重新从「未触发」开始，这是可预期的行为。

use dashmap::DashMap;
use gateway_sdk::{GroupId, NodeId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::OnceLock;

/// 比较运算符
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompareOp {
    Gt,
    Ge,
    Lt,
    Le,
    Eq,
    Ne,
}

impl CompareOp {
    pub fn as_str(self) -> &'static str {
        match self {
            CompareOp::Gt => "gt",
            CompareOp::Ge => "ge",
            CompareOp::Lt => "lt",
            CompareOp::Le => "le",
            CompareOp::Eq => "eq",
            CompareOp::Ne => "ne",
        }
    }

    /// 阈值比较。浮点等值比较用相对容差，避免 0.1+0.2 这类噪声把 eq 变成永远不成立。
    pub fn eval(self, value: f64, threshold: f64) -> bool {
        match self {
            CompareOp::Gt => value > threshold,
            CompareOp::Ge => value >= threshold,
            CompareOp::Lt => value < threshold,
            CompareOp::Le => value <= threshold,
            CompareOp::Eq => approx_eq(value, threshold),
            CompareOp::Ne => !approx_eq(value, threshold),
        }
    }
}

fn approx_eq(a: f64, b: f64) -> bool {
    if a == b {
        return true;
    }
    if !a.is_finite() || !b.is_finite() {
        return false;
    }
    (a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1.0)
}

/// 规则绑定的南向点位
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuleSource {
    pub south_node_id: NodeId,
    pub group_id: GroupId,
    pub tag_name: String,
}

/// 触发条件
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuleCondition {
    pub op: CompareOp,
    pub threshold: f64,
}

/// 触发后执行的动作
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RuleAction {
    /// 写回同一南向节点上的同名点位
    WriteTag { tag_name: String, value: f64 },
    /// 仅记录日志（用于调试与告警演示）
    Log { message: String },
}

/// 一条规则
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rule {
    pub id: String,
    pub name: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub source: RuleSource,
    pub condition: RuleCondition,
    /// 条件需持续满足的时长（毫秒）；0 = 满足即触发
    #[serde(default)]
    pub for_ms: u64,
    /// 条件恢复后需持续多久才允许再次触发（毫秒）；0 = 恢复即重新武装
    #[serde(default)]
    pub clear_ms: u64,
    pub action: RuleAction,
}

fn default_true() -> bool {
    true
}

impl Rule {
    /// 配置校验（API 层用）
    pub fn validate(&self) -> Result<(), String> {
        if self.id.trim().is_empty() {
            return Err("rule id must not be empty".to_string());
        }
        if self.name.trim().is_empty() {
            return Err("rule name must not be empty".to_string());
        }
        if self.source.tag_name.trim().is_empty() {
            return Err("source.tag_name must not be empty".to_string());
        }
        if !self.condition.threshold.is_finite() {
            return Err("threshold must be a finite number".to_string());
        }
        const MAX_MS: u64 = 3_600_000; // 1 小时
        if self.for_ms > MAX_MS {
            return Err(format!("for_ms must be <= {}", MAX_MS));
        }
        if self.clear_ms > MAX_MS {
            return Err(format!("clear_ms must be <= {}", MAX_MS));
        }
        match &self.action {
            RuleAction::WriteTag { tag_name, value } => {
                if tag_name.trim().is_empty() {
                    return Err("action.tag_name must not be empty".to_string());
                }
                if !value.is_finite() {
                    return Err("action.value must be a finite number".to_string());
                }
            }
            RuleAction::Log { message } => {
                if message.trim().is_empty() {
                    return Err("action.message must not be empty".to_string());
                }
            }
        }
        Ok(())
    }
}

/// 单条规则的运行期状态（不持久化）
#[derive(Debug, Clone, Default, Serialize)]
pub struct RuleRuntime {
    /// 条件开始连续满足的时刻（单调毫秒）
    #[serde(skip)]
    pub since_ms: Option<u64>,
    /// 条件开始连续不满足的时刻（用于 clear_ms 去抖）
    #[serde(skip)]
    pub clear_since_ms: Option<u64>,
    /// 本轮是否已触发（条件恢复后清零）
    pub fired: bool,
    pub last_value: Option<f64>,
    pub fire_count: u64,
    /// 最近一次触发时间（RFC3339，UTC）
    pub last_fired_at: Option<String>,
    pub last_error: Option<String>,
}

/// 规则的对外视图：配置 + 运行期状态
#[derive(Debug, Clone, Serialize)]
pub struct RuleView {
    #[serde(flatten)]
    pub rule: Rule,
    pub runtime: RuleRuntime,
}

/// 一次触发（供调用方执行动作）
#[derive(Debug, Clone)]
pub struct Firing {
    pub rule: Rule,
    /// 触发时的点位值
    pub value: f64,
}

/// 规则引擎：持有各规则的运行期状态，按时间推进判断是否触发。
///
/// 状态放在引擎里而不是规则对象里，是为了让「规则」保持纯配置、可直接进快照；
/// 引擎本身是进程级单例（一个进程只跑一个网关实例）。
#[derive(Default)]
pub struct RuleEngine {
    states: DashMap<String, RuleRuntime>,
}

impl RuleEngine {
    pub fn new() -> Self {
        Self::default()
    }

    /// 用本次采集到的点位值推进所有相关规则，返回需要执行的动作。
    ///
    /// `values` 为本次采集的 `tag_name -> f64`；`now_ms` 为单调时钟毫秒值（由调用方提供，
    /// 便于测试直接给定时间序列）。
    pub fn advance(
        &self,
        rules: &[Rule],
        values: &HashMap<String, f64>,
        now_ms: u64,
    ) -> Vec<Firing> {
        let mut firings = Vec::new();
        for rule in rules {
            if !rule.enabled {
                continue;
            }
            let Some(&value) = values.get(&rule.source.tag_name) else {
                continue; // 本次采集没有该点位（可能被删了或采集失败），不推进状态
            };

            let mut st = self.states.entry(rule.id.clone()).or_default();
            st.last_value = Some(value);

            if rule.condition.op.eval(value, rule.condition.threshold) {
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
                // 已触发但条件恢复：需持续 clear_ms 才重新武装
                let clear_since = *st.clear_since_ms.get_or_insert(now_ms);
                if now_ms.saturating_sub(clear_since) >= rule.clear_ms {
                    st.fired = false;
                    st.clear_since_ms = None;
                    st.since_ms = None;
                }
            } else {
                // 未触发且不满足：重置保持计时，下次满足重新开始计 for_ms
                st.since_ms = None;
                st.clear_since_ms = None;
            }
        }
        firings
    }

    /// 读取某规则的运行期状态
    pub fn runtime(&self, rule_id: &str) -> RuleRuntime {
        self.states
            .get(rule_id)
            .map(|r| r.clone())
            .unwrap_or_default()
    }

    /// 清空全部运行期状态（测试与规则删除时使用）
    pub fn clear(&self) {
        self.states.clear();
    }

    /// 删除单条规则的运行期状态
    pub fn forget(&self, rule_id: &str) {
        self.states.remove(rule_id);
    }
}

/// 进程级规则引擎实例
pub fn engine() -> &'static RuleEngine {
    static ENGINE: OnceLock<RuleEngine> = OnceLock::new();
    ENGINE.get_or_init(RuleEngine::new)
}

/// 单调时钟毫秒值（用于规则的持续时长判断，不受系统时间跳变影响）
fn monotonic_ms() -> u64 {
    static BASE: OnceLock<std::time::Instant> = OnceLock::new();
    BASE.get_or_init(std::time::Instant::now)
        .elapsed()
        .as_millis() as u64
}

/// 采集热路径上的规则求值 + 动作执行。
///
/// 返回 `(触发次数, 动作失败次数)`，由调用方计入指标。
/// 该点在每个采集周期都会执行，因此：
/// - 该组没有任何启用规则时**立即返回**，不构造任何中间结构；
/// - 动作失败只记录并计数，绝不让采集循环失败。
pub async fn evaluate_and_fire(
    store: &crate::store::Store,
    plugin: &std::sync::Arc<dyn gateway_sdk::SouthPlugin>,
    node_id: NodeId,
    group_id: GroupId,
    values: &[(gateway_sdk::TagId, gateway_sdk::types::DataValue)],
) -> (u64, u64) {
    let rules = store.rules_by_group(node_id, group_id);
    if rules.is_empty() {
        return (0, 0);
    }
    let numeric = numeric_values(store, values);
    let firings = engine().advance(&rules, &numeric, monotonic_ms());

    let mut fired = 0u64;
    let mut failed = 0u64;
    for firing in firings {
        fired += 1;
        let rule = &firing.rule;
        match &rule.action {
            RuleAction::Log { message } => {
                tracing::info!(
                    rule = %rule.name,
                    tag = %rule.source.tag_name,
                    value = firing.value,
                    threshold = rule.condition.threshold,
                    op = rule.condition.op.as_str(),
                    "rule fired: {}", message
                );
            }
            RuleAction::WriteTag { tag_name, value } => {
                let tag = store.tag_get_by_name(node_id, group_id, tag_name);
                match tag {
                    Some(tag) => {
                        let payload = [(tag, gateway_sdk::types::DataValue::Float64(*value))];
                        match plugin.write_tags(node_id, &payload).await {
                            Ok(()) => tracing::info!(
                                rule = %rule.name,
                                tag = %tag_name,
                                value = *value,
                                "rule fired and wrote tag"
                            ),
                            Err(e) => {
                                failed += 1;
                                tracing::warn!(
                                    rule = %rule.name,
                                    tag = %tag_name,
                                    "rule action write failed: {}", e
                                );
                            }
                        }
                    }
                    None => {
                        failed += 1;
                        tracing::warn!(
                            rule = %rule.name,
                            tag = %tag_name,
                            "rule action target tag not found in this group"
                        );
                    }
                }
            }
        }
    }
    (fired, failed)
}

/// 从 `(TagId, DataValue)` 列表构造 `tag_name -> f64`，非数值点位跳过
pub fn numeric_values(
    store: &crate::store::Store,
    values: &[(gateway_sdk::TagId, gateway_sdk::types::DataValue)],
) -> HashMap<String, f64> {
    let mut out = HashMap::new();
    for (tid, v) in values {
        let Some(name) = store.tag_get(*tid).map(|t| t.name) else {
            continue;
        };
        // 整型尽量避免精度损失：能转 i64 的按 i64 再转 f64（f64 精确表示 2^53 以内整数）
        let f = match v {
            gateway_sdk::types::DataValue::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
            _ => v.as_f64().or_else(|| v.as_i64().map(|i| i as f64)),
        };
        if let Some(f) = f {
            if f.is_finite() {
                out.insert(name, f);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(id: &str, op: CompareOp, threshold: f64, for_ms: u64, clear_ms: u64) -> Rule {
        Rule {
            id: id.to_string(),
            name: format!("rule-{}", id),
            enabled: true,
            source: RuleSource {
                south_node_id: NodeId::new(),
                group_id: GroupId::new(),
                tag_name: "temperature".to_string(),
            },
            condition: RuleCondition { op, threshold },
            for_ms,
            clear_ms,
            action: RuleAction::Log {
                message: "hot".to_string(),
            },
        }
    }

    fn vals(v: f64) -> HashMap<String, f64> {
        let mut m = HashMap::new();
        m.insert("temperature".to_string(), v);
        m
    }

    #[test]
    fn immediate_trigger_when_for_ms_is_zero() {
        let e = RuleEngine::new();
        let r = rule("r1", CompareOp::Gt, 10.0, 0, 0);
        let fires = e.advance(std::slice::from_ref(&r), &vals(11.0), 0);
        assert_eq!(fires.len(), 1, "满足即触发");
        assert_eq!(e.runtime("r1").fire_count, 1);
    }

    #[test]
    fn does_not_fire_before_for_ms_elapses() {
        let e = RuleEngine::new();
        let r = rule("r1", CompareOp::Gt, 10.0, 500, 0);
        assert!(e
            .advance(std::slice::from_ref(&r), &vals(11.0), 100)
            .is_empty());
        assert!(e
            .advance(std::slice::from_ref(&r), &vals(12.0), 400)
            .is_empty());
        // 累计满足 500ms 才触发
        let fires = e.advance(std::slice::from_ref(&r), &vals(13.0), 600);
        assert_eq!(fires.len(), 1);
        assert_eq!(fires[0].value, 13.0);
    }

    #[test]
    fn for_ms_timer_restarts_after_a_dip() {
        let e = RuleEngine::new();
        let r = rule("r1", CompareOp::Gt, 10.0, 300, 0);
        assert!(e
            .advance(std::slice::from_ref(&r), &vals(11.0), 0)
            .is_empty());
        // 中途掉回阈值以下：保持计时重置
        assert!(e
            .advance(std::slice::from_ref(&r), &vals(9.0), 200)
            .is_empty());
        assert!(
            e.advance(std::slice::from_ref(&r), &vals(11.0), 400)
                .is_empty(),
            "重新开始计 300ms"
        );
        assert_eq!(
            e.advance(std::slice::from_ref(&r), &vals(11.0), 700).len(),
            1
        );
    }

    #[test]
    fn does_not_refire_while_condition_holds() {
        let e = RuleEngine::new();
        let r = rule("r1", CompareOp::Gt, 10.0, 0, 0);
        assert_eq!(e.advance(std::slice::from_ref(&r), &vals(11.0), 0).len(), 1);
        for t in [100, 200, 300, 400] {
            assert!(
                e.advance(std::slice::from_ref(&r), &vals(12.0), t)
                    .is_empty(),
                "持续满足期间不应重复触发"
            );
        }
        assert_eq!(e.runtime("r1").fire_count, 1);
    }

    #[test]
    fn rearms_only_after_clear_ms() {
        let e = RuleEngine::new();
        let r = rule("r1", CompareOp::Gt, 10.0, 0, 400);
        assert_eq!(e.advance(std::slice::from_ref(&r), &vals(11.0), 0).len(), 1);
        // 恢复但未满 clear_ms：仍不重新武装
        assert!(e
            .advance(std::slice::from_ref(&r), &vals(5.0), 200)
            .is_empty());
        assert!(e
            .advance(std::slice::from_ref(&r), &vals(11.0), 300)
            .is_empty());
        // 恢复计时被中途满足打断 → 需要重新累计
        assert!(e
            .advance(std::slice::from_ref(&r), &vals(5.0), 400)
            .is_empty());
        assert!(e
            .advance(std::slice::from_ref(&r), &vals(11.0), 500)
            .is_empty());
        assert!(e
            .advance(std::slice::from_ref(&r), &vals(5.0), 600)
            .is_empty());
        // 持续不满足满 400ms 后重新武装
        assert!(e
            .advance(std::slice::from_ref(&r), &vals(5.0), 1100)
            .is_empty());
        assert!(!e.runtime("r1").fired);
    }

    #[test]
    fn fires_again_after_a_full_recovery() {
        let e = RuleEngine::new();
        let r = rule("r1", CompareOp::Gt, 10.0, 0, 100);
        assert_eq!(e.advance(std::slice::from_ref(&r), &vals(11.0), 0).len(), 1);
        // 恢复计时从 t=200 开始，需要持续到 t=300 才算满 clear_ms=100
        assert!(e
            .advance(std::slice::from_ref(&r), &vals(5.0), 200)
            .is_empty());
        assert!(e
            .advance(std::slice::from_ref(&r), &vals(5.0), 300)
            .is_empty());
        assert!(!e.runtime("r1").fired, "恢复满 clear_ms 后应重新武装");
        assert_eq!(
            e.advance(std::slice::from_ref(&r), &vals(11.0), 400).len(),
            1,
            "恢复后再次满足应再次触发"
        );
        assert_eq!(e.runtime("r1").fire_count, 2);
    }

    #[test]
    fn interrupted_recovery_does_not_rearm() {
        // 恢复途中又满足：恢复计时被重置，避免抖动导致动作被反复执行
        let e = RuleEngine::new();
        let r = rule("r1", CompareOp::Gt, 10.0, 0, 500);
        assert_eq!(e.advance(std::slice::from_ref(&r), &vals(11.0), 0).len(), 1);
        assert!(e
            .advance(std::slice::from_ref(&r), &vals(5.0), 100)
            .is_empty());
        // 恢复只持续了 200ms 就被打断 → 重置恢复计时
        assert!(
            e.advance(std::slice::from_ref(&r), &vals(11.0), 300)
                .is_empty(),
            "被打断的恢复不应触发新的动作"
        );
        assert!(e.runtime("r1").fired, "仍处于已触发状态");
        assert_eq!(e.runtime("r1").fire_count, 1);
    }

    #[test]
    fn disabled_rules_never_fire() {
        let e = RuleEngine::new();
        let mut r = rule("r1", CompareOp::Gt, 10.0, 0, 0);
        r.enabled = false;
        assert!(e.advance(&[r], &vals(99.0), 0).is_empty());
    }

    #[test]
    fn missing_tag_value_does_not_advance_state() {
        let e = RuleEngine::new();
        let r = rule("r1", CompareOp::Gt, 10.0, 0, 0);
        let empty = HashMap::new();
        assert!(e.advance(std::slice::from_ref(&r), &empty, 0).is_empty());
        assert_eq!(e.runtime("r1").last_value, None);
    }

    #[test]
    fn each_rule_tracks_its_own_state() {
        let e = RuleEngine::new();
        let a = rule("a", CompareOp::Gt, 10.0, 0, 0);
        let b = rule("b", CompareOp::Gt, 20.0, 0, 0);
        let fires = e.advance(&[a, b], &vals(15.0), 0);
        assert_eq!(fires.len(), 1);
        assert_eq!(fires[0].rule.id, "a");
        assert_eq!(e.runtime("b").fire_count, 0);
    }

    #[test]
    fn compare_ops_behave_as_expected() {
        assert!(CompareOp::Gt.eval(11.0, 10.0));
        assert!(!CompareOp::Gt.eval(10.0, 10.0));
        assert!(CompareOp::Ge.eval(10.0, 10.0));
        assert!(CompareOp::Lt.eval(9.0, 10.0));
        assert!(CompareOp::Le.eval(10.0, 10.0));
        assert!(CompareOp::Eq.eval(0.3, 0.1 + 0.2), "浮点等值应带容差");
        assert!(CompareOp::Ne.eval(1.0, 2.0));
        // 非有限值不应被当作相等
        assert!(!CompareOp::Eq.eval(f64::NAN, f64::NAN));
    }

    #[test]
    fn rule_validation_rejects_bad_input() {
        let mut r = rule("r1", CompareOp::Gt, 10.0, 0, 0);
        assert!(r.validate().is_ok());

        r.name = "  ".to_string();
        assert!(r.validate().is_err());
        r.name = "ok".to_string();

        r.for_ms = 3_600_001;
        assert!(r.validate().is_err());
        r.for_ms = 0;

        r.condition.threshold = f64::INFINITY;
        assert!(r.validate().is_err());
        r.condition.threshold = 1.0;

        r.action = RuleAction::WriteTag {
            tag_name: String::new(),
            value: 1.0,
        };
        assert!(r.validate().is_err());
    }
}
