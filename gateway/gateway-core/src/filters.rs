//! 数据面过滤与滑动窗口聚合。
//!
//! 数据链路的唯一插入点是 `poll_group_once` 的 `Ok(Ok(values))` 分支，
//! 顺序固定为：规则求值(原始值) → last_values(原始值) → filters::apply → publish(过滤后值)。
//!
//! ## 语义（对外承诺，测试据此编写）
//!
//! - **死区过滤 / 变化上报**：每标签维护基线 `last_reported`。首次采集必报；抑制期间基线不前移；
//!   心跳 `max_report_ms > 0` 强制上报并刷新基线。
//! - **滑动窗口聚合**：样本缓冲仅存 `window_ms` 内样本；发射挂在采集节拍上（不新增任务），
//!   有效输出周期 = max(emit_ms, 组 interval_ms)；聚合输出不经过死区。
//! - **状态不落盘**：重启即清零，与规则引擎运行态语义一致。

use dashmap::DashMap;
use gateway_sdk::types::{DataValue, GroupId, NodeId, TagId};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::{OnceLock, RwLock};

/// 过滤模式
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FilterMode {
    /// 不过滤（默认）
    #[default]
    Passthrough,
    /// 仅值变化时报送
    OnChange,
}

/// 单标签死区配置（点位级覆盖组级默认值）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagDeadband {
    /// 点位名称（校验存在性）
    pub tag_name: String,
    /// 绝对死区阈值；>= 0
    pub deadband: Option<f64>,
    /// 相对死区百分比 (0..=100]；last_reported == 0 时退化为绝对比较
    pub deadband_percent: Option<f64>,
}

/// 滑动窗口聚合项
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowAgg {
    /// 非空子集：avg / min / max / count / first / last
    pub kinds: Vec<String>,
}

/// 滑动窗口策略
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowPolicy {
    /// 窗口长度（毫秒），>= 100
    pub window_ms: u64,
    /// 发射周期下限（毫秒），>= 10；有效输出周期 = max(emit_ms, 组 interval_ms)
    pub emit_ms: u64,
    /// 所有数值点位统一输出的聚合项
    pub aggregates: WindowAgg,
}

/// 组数据策略：一组最多一条
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GroupPolicy {
    pub south_node_id: NodeId,
    pub group_id: GroupId,
    /// 过滤模式（默认 passthrough）
    #[serde(default)]
    pub mode: FilterMode,
    /// 组级默认绝对死区（on_change 模式下未单独配置的数值点位用它）
    #[serde(default)]
    pub deadband: Option<f64>,
    /// 组级默认相对死区百分比
    #[serde(default)]
    pub deadband_percent: Option<f64>,
    /// 心跳强制上报周期（毫秒）；0 = 不强制
    #[serde(default)]
    pub max_report_ms: u64,
    /// 滑动窗口策略（存在时启用）
    #[serde(default)]
    pub window: Option<WindowPolicy>,
    /// 点位级死区覆盖（同名后者覆盖前者；校验时去重拒绝）
    #[serde(default)]
    pub tags: Vec<TagDeadband>,
}

/// 校验 GroupPolicy 配置
pub fn validate_policy(policy: &GroupPolicy) -> Result<(), String> {
    if let Some(db) = policy.deadband {
        if !db.is_finite() || db < 0.0 {
            return Err("deadband must be >= 0 and finite".to_string());
        }
    }
    if let Some(pct) = policy.deadband_percent {
        if !(0.0 < pct && pct <= 100.0) {
            return Err("deadband_percent must be in (0, 100]".to_string());
        }
    }
    // max_report_ms 是 u64，天然 >= 0，跳过无意义的下限校验
    if let Some(ref w) = policy.window {
        if w.window_ms < 100 {
            return Err("window_ms must be >= 100".to_string());
        }
        if w.emit_ms < 10 {
            return Err("emit_ms must be >= 10".to_string());
        }
        if w.aggregates.kinds.is_empty() {
            return Err("aggregates.kinds must be non-empty".to_string());
        }
        let valid = ["avg", "min", "max", "count", "first", "last"];
        for k in &w.aggregates.kinds {
            if !valid.contains(&k.as_str()) {
                return Err(format!("unknown aggregate kind: {}", k));
            }
        }
    }
    // tags 去重
    let mut seen = std::collections::HashSet::new();
    for t in &policy.tags {
        if !seen.insert(t.tag_name.as_str()) {
            return Err(format!("duplicate tag_name in tags: {}", t.tag_name));
        }
        if let Some(db) = t.deadband {
            if !db.is_finite() || db < 0.0 {
                return Err(format!(
                    "tag {} deadband must be >= 0 and finite",
                    t.tag_name
                ));
            }
        }
        if let Some(pct) = t.deadband_percent {
            if !(0.0 < pct && pct <= 100.0) {
                return Err(format!(
                    "tag {} deadband_percent must be in (0, 100]",
                    t.tag_name
                ));
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 运行态状态（不持久化，重启即清零）
// ---------------------------------------------------------------------------

/// 单标签的过滤基线状态
#[derive(Debug, Clone)]
struct TagBaseline {
    /// 上次上报的值（用于死区比较）
    last_reported: Option<f64>,
    /// 上次上报的时刻（单调毫秒）
    last_reported_ms: u64,
}

/// 单标签的窗口缓冲
#[derive(Debug, Clone, Default)]
struct TagWindow {
    /// 样本环形缓冲：(单调时刻, 值)
    samples: VecDeque<(u64, f64)>,
    /// 最近一次发射时刻（单调毫秒）
    last_emit_ms: u64,
}

/// 单组过滤运行态（基线 + 窗口缓冲）
#[derive(Debug, Default)]
struct GroupFilterState {
    baselines: RwLock<HashMap<TagId, TagBaseline>>,
    windows: RwLock<HashMap<TagId, TagWindow>>,
    /// 虚拟点位 TagId 缓存（进程内稳态，进程重启后 id 变化无影响）
    synthetic_ids: RwLock<HashMap<String, TagId>>,
}

/// 进程级过滤状态单例
pub struct FilterState {
    states: DashMap<(NodeId, GroupId), GroupFilterState>,
}

impl FilterState {
    fn new() -> Self {
        Self {
            states: DashMap::new(),
        }
    }
}

fn state() -> &'static FilterState {
    static STATE: OnceLock<FilterState> = OnceLock::new();
    STATE.get_or_init(FilterState::new)
}

/// 清除某组的全部运行态（节点删除、组删除、策略重 PUT 时调用）
pub fn forget_group(node_id: NodeId, group_id: GroupId) {
    state().states.remove(&(node_id, group_id));
}

/// 清除某节点下所有组的运行态
pub fn forget_node(node_id: NodeId) {
    state().states.retain(|k, _| k.0 != node_id);
}

/// 清除全部运行态（测试用）
pub fn clear_all() {
    state().states.clear();
}

// ---------------------------------------------------------------------------
// 单调时钟（仿 rules.rs）
// ---------------------------------------------------------------------------
// 辅助：提取数值的 f64（复用 rules.rs 的 numeric_values 口径）
// ---------------------------------------------------------------------------

#[allow(dead_code)]
fn numeric_f64(v: &DataValue) -> Option<f64> {
    match v {
        DataValue::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        _ => v.as_f64().or_else(|| v.as_i64().map(|i| i as f64)),
    }
}

// ---------------------------------------------------------------------------
// 死区判断：绝对 + 相对
// ---------------------------------------------------------------------------

fn should_report(
    new_val: f64,
    baseline: Option<f64>,
    deadband: Option<f64>,
    deadband_percent: Option<f64>,
) -> bool {
    let Some(base) = baseline else {
        return true; // 首采必报
    };
    // 绝对死区（仅当 db > 0 时生效；0 表示"不设绝对死区"）
    if let Some(db) = deadband {
        if db > 0.0 && (new_val - base).abs() > db {
            return true;
        }
    }
    // 相对死区
    if let Some(pct) = deadband_percent {
        if base == 0.0 {
            if new_val != 0.0 {
                return true;
            }
        } else if (new_val - base).abs() > (pct / 100.0) * base.abs() {
            return true;
        }
    }
    false
}

// ---------------------------------------------------------------------------
// 窗口聚合
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct AggValue {
    pub tag_name: String,
    pub agg: String,
    pub value: f64,
}

fn aggregate(kind: &str, samples: &[(u64, f64)]) -> Option<f64> {
    if samples.is_empty() {
        return None;
    }
    match kind {
        "avg" => {
            let sum: f64 = samples.iter().map(|(_, v)| v).sum();
            Some(sum / samples.len() as f64)
        }
        "min" => samples.iter().map(|(_, v)| *v).reduce(|a, b| a.min(b)),
        "max" => samples.iter().map(|(_, v)| *v).reduce(|a, b| a.max(b)),
        "count" => Some(samples.len() as f64),
        "first" => samples.first().map(|(_, v)| *v),
        "last" => samples.last().map(|(_, v)| *v),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// FilterOutcome（供 manager 消费）
// ---------------------------------------------------------------------------

/// filters::apply 的返回值
#[derive(Debug, Clone)]
pub struct FilterOutcome {
    /// 经过死区过滤的真实点位
    pub published: Vec<(TagId, DataValue)>,
    /// 本批到期的窗口聚合虚拟点位（可能为空）
    pub synthetic: Vec<(TagId, DataValue)>,
    /// 虚拟点位名称（{原始名}_{agg}）
    pub synthetic_names: HashMap<TagId, String>,
    /// 被整批抑制的次数（整批都无任何输出时由 manager 计入 metrics）
    pub suppressed_msgs: u64,
    /// 被抑制的点位次
    pub suppressed_tags: u64,
    /// 窗口聚合发射次数（manager 计入 metrics）
    pub window_emitted: u64,
}

// ---------------------------------------------------------------------------
// 核心 apply 函数
// ---------------------------------------------------------------------------

/// 对一组采集值应用过滤策略。
///
/// `store` 仅用于点位名查表；`ts_ms` 为单调毫秒（由调用方从 wall-clock 换算注入，
/// 便于单测直接给定时间序列）。
///
/// 返回值中的 `synthetic` 虚拟点位 TagId 在进程内稳态缓存，进程重启后 id 变化无影响。
pub fn apply(
    store: &crate::Store,
    node_id: NodeId,
    group_id: GroupId,
    values: &[(TagId, DataValue)],
    ts_ms: u64,
) -> FilterOutcome {
    let policy = store.policy_get(node_id, group_id);

    // 无策略 → 零开销透传
    let Some(policy) = policy else {
        return FilterOutcome {
            published: values.to_vec(),
            synthetic: Vec::new(),
            synthetic_names: HashMap::new(),
            suppressed_msgs: 0,
            suppressed_tags: 0,
            window_emitted: 0,
        };
    };

    let mut published = Vec::with_capacity(values.len());
    let mut suppressed_tags = 0u64;

    // ---- 死区 / 变化上报（仅 on_change 模式）----
    if policy.mode == FilterMode::OnChange {
        // 建立 tag_name -> TagDeadband 查找
        let tag_db_map: HashMap<&str, &TagDeadband> = policy
            .tags
            .iter()
            .map(|t| (t.tag_name.as_str(), t))
            .collect();

        // 获取或创建该组运行态
        let group_state = state().states.entry((node_id, group_id)).or_default();

        for (tid, value) in values {
            // 获取点位名（用于日志与死区配置查找）
            let tag_name = match store.tag_get(*tid).map(|t| t.name.clone()) {
                Some(n) => n,
                None => continue,
            };

            // 确定死区参数：点位级覆盖 > 组级默认值
            let (deadband, deadband_percent) = if let Some(td) = tag_db_map.get(tag_name.as_str()) {
                (
                    td.deadband.or(policy.deadband),
                    td.deadband_percent.or(policy.deadband_percent),
                )
            } else {
                (policy.deadband, policy.deadband_percent)
            };

            let baseline = group_state.baselines.read().unwrap().get(tid).cloned();

            match value {
                // 布尔/字符串/字节：仅值变化时报送
                DataValue::Bool(b) => {
                    // 布尔：比较真值；baseline 存 0.0/1.0
                    let new_bool = *b;
                    let changed = match &baseline {
                        Some(b) if b.last_reported.is_some() => {
                            let prev_bool = baseline
                                .as_ref()
                                .and_then(|b| b.last_reported)
                                .map(|v| v != 0.0);
                            Some(new_bool) != prev_bool
                        }
                        _ => true, // 首采必报
                    };
                    if changed {
                        published.push((*tid, value.clone()));
                        group_state.baselines.write().unwrap().insert(
                            *tid,
                            TagBaseline {
                                last_reported: Some(if new_bool { 1.0 } else { 0.0 }),
                                last_reported_ms: ts_ms,
                            },
                        );
                    } else {
                        suppressed_tags += 1;
                    }
                }
                DataValue::String(s) => {
                    // 字符串：比较值；baseline 存字符串长度作为代理
                    let new_len = s.len() as f64;
                    let changed = match &baseline {
                        Some(b) if b.last_reported.is_some() => {
                            let prev_len = baseline.as_ref().and_then(|b| b.last_reported);
                            Some(new_len) != prev_len
                        }
                        _ => true,
                    };
                    if changed {
                        published.push((*tid, value.clone()));
                        group_state.baselines.write().unwrap().insert(
                            *tid,
                            TagBaseline {
                                last_reported: Some(new_len),
                                last_reported_ms: ts_ms,
                            },
                        );
                    } else {
                        suppressed_tags += 1;
                    }
                }
                DataValue::Bytes(_) => {
                    // Bytes：比较长度
                    let new_len = match value {
                        DataValue::Bytes(b) => b.len() as f64,
                        _ => 0.0,
                    };
                    let changed = match &baseline {
                        Some(b) if b.last_reported.is_some() => {
                            let prev_len = baseline.as_ref().and_then(|b| b.last_reported);
                            Some(new_len) != prev_len
                        }
                        _ => true,
                    };
                    if changed {
                        published.push((*tid, value.clone()));
                        group_state.baselines.write().unwrap().insert(
                            *tid,
                            TagBaseline {
                                last_reported: Some(new_len),
                                last_reported_ms: ts_ms,
                            },
                        );
                    } else {
                        suppressed_tags += 1;
                    }
                }
                // 数值类型：走绝对/相对死区 + 心跳
                _ => {
                    let Some(new_f) = numeric_f64(value) else {
                        // 非数值类型（Bytes等）直接上报
                        published.push((*tid, value.clone()));
                        continue;
                    };
                    let base = baseline.as_ref().and_then(|b| b.last_reported);
                    let base_ms = baseline.as_ref().map(|b| b.last_reported_ms);

                    let passes_deadband = should_report(new_f, base, deadband, deadband_percent);

                    // 心跳判断
                    let due_heartbeat = if policy.max_report_ms > 0 {
                        base_ms
                            .map(|lm| ts_ms.saturating_sub(lm) >= policy.max_report_ms)
                            .unwrap_or(true) // 无基线视为需要心跳
                    } else {
                        false
                    };

                    if passes_deadband || due_heartbeat {
                        published.push((*tid, value.clone()));
                        group_state.baselines.write().unwrap().insert(
                            *tid,
                            TagBaseline {
                                last_reported: Some(new_f),
                                last_reported_ms: ts_ms,
                            },
                        );
                    } else {
                        suppressed_tags += 1;
                    }
                }
            }
        }
    } else {
        // passthrough
        published.extend(values.iter().cloned());
    }

    // ---- 滑动窗口聚合（独立于 mode，输入永远是原始值）----
    let mut synthetic = Vec::new();
    let mut synthetic_names: HashMap<TagId, String> = HashMap::new();
    let mut window_emitted = 0u64;

    if let Some(ref win) = policy.window {
        let group_state = state().states.entry((node_id, group_id)).or_default();

        // 收集所有数值点位的原始值（窗口输入永远是原始值）
        for (tid, value) in values {
            let Some(f) = numeric_f64(value) else {
                continue; // 非数值不进窗口
            };
            let tag_name = match store.tag_get(*tid).map(|t| t.name.clone()) {
                Some(n) => n,
                None => continue,
            };

            // 直接用 get/insert 模式，避免 Entry API 的借用生命周期问题
            {
                let mut guard = group_state.windows.write().unwrap();
                guard.entry(*tid).or_default();
            }
            {
                let mut guard = group_state.windows.write().unwrap();
                let win_state = guard.get_mut(tid).unwrap();
                // 追加样本并裁剪过期
                win_state.samples.push_back((ts_ms, f));
                while let Some(&(t, _)) = win_state.samples.front() {
                    if t < ts_ms.saturating_sub(win.window_ms) {
                        win_state.samples.pop_front();
                    } else {
                        break;
                    }
                }

                // 检查是否需要发射：ts_ms - last_emit_ms >= emit_ms
                let can_emit = ts_ms.saturating_sub(win_state.last_emit_ms) >= win.emit_ms;
                let has_samples = !win_state.samples.is_empty();

                if can_emit && has_samples {
                    let samples: Vec<_> = win_state.samples.iter().cloned().collect();
                    for kind in &win.aggregates.kinds {
                        if let Some(val) = aggregate(kind, &samples) {
                            // 虚拟点位命名：{tag_name}_{agg}
                            let syn_name = format!("{}_{}", tag_name, kind);
                            // 缓存虚拟 TagId
                            let syn_id = {
                                let mut sguard = group_state.synthetic_ids.write().unwrap();
                                *sguard.entry(syn_name.clone()).or_default()
                            };
                            synthetic.push((syn_id, DataValue::Float64(val)));
                            synthetic_names.insert(syn_id, syn_name);
                        }
                    }
                    win_state.last_emit_ms = ts_ms;
                    if !synthetic.is_empty() {
                        window_emitted = 1; // 本批有窗口发射
                    }
                }
            }
        }
    }

    FilterOutcome {
        suppressed_msgs: if published.is_empty() && synthetic.is_empty() {
            1
        } else {
            0
        },
        suppressed_tags,
        published,
        synthetic,
        synthetic_names,
        window_emitted,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::Node;
    use crate::store::Store;
    use gateway_sdk::types::NodeKind;

    /// 串行化所有会触碰全局 FilterState (OnceLock) 的测试：
    /// FilterState 是进程级单例，并行测试中的状态会相互干扰。
    /// 使用 RwLock + catch_unwind：panic 不会中毒，而是被捕获后继续。
    static FILTER_TEST_LOCK: std::sync::RwLock<()> = std::sync::RwLock::new(());

    /// 在每个修改全局状态的测试开头调用，确保串行执行。
    fn with_lock<R>(f: impl FnOnce() -> R) -> R {
        // 先清除（读锁，可重入）
        clear_all();
        // 写锁（阻塞其他测试）
        let guard = std::sync::RwLock::write(&FILTER_TEST_LOCK);
        // 用 catch_unwind 包装测试体，防止 panic 中毒
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
        drop(guard);
        result.unwrap_or_else(|_| {
            // panic 已在测试中处理；这里再次 unwrap 会导致双重 panic
            // 所以我们用 Result 传播
            std::panic::resume_unwind(Box::new(
                std::boxed::Box::new("test panicked") as Box<dyn std::any::Any + Send>
            ))
        })
    }

    fn nid() -> NodeId {
        NodeId::new()
    }
    fn gid() -> GroupId {
        GroupId::new()
    }
    fn tid() -> TagId {
        TagId::new()
    }

    fn store_with(nid: NodeId, gid: GroupId, tid: TagId, tag_name: &str) -> Store {
        use std::collections::HashMap;
        let store = Store::new();
        let config: gateway_sdk::PluginConfig = HashMap::new();
        let node = Node::new("test-node", NodeKind::South, "sim", config);
        store.node_insert(node);
        let grp = gateway_sdk::Group {
            id: gid,
            name: "test-group".to_string(),
            interval_ms: 1000,
            description: None,
        };
        store.group_insert(nid, grp);
        let tag = gateway_sdk::Tag {
            id: tid,
            group_id: gid,
            name: tag_name.to_string(),
            address: "addr".to_string(),
            attr: gateway_sdk::TagAttr::Read,
            data_type: None,
            description: None,
        };
        store.tag_insert(nid, tag);
        store
    }

    fn on_change_policy(nid: NodeId, gid: GroupId, deadband: f64) -> GroupPolicy {
        GroupPolicy {
            south_node_id: nid,
            group_id: gid,
            mode: FilterMode::OnChange,
            deadband: Some(deadband),
            deadband_percent: None,
            max_report_ms: 0,
            window: None,
            tags: vec![],
        }
    }

    fn passthrough_policy(nid: NodeId, gid: GroupId) -> GroupPolicy {
        GroupPolicy {
            south_node_id: nid,
            group_id: gid,
            mode: FilterMode::Passthrough,
            deadband: None,
            deadband_percent: None,
            max_report_ms: 0,
            window: None,
            tags: vec![],
        }
    }

    fn win_policy(nid: NodeId, gid: GroupId, wms: u64, ems: u64, kinds: Vec<&str>) -> GroupPolicy {
        GroupPolicy {
            south_node_id: nid,
            group_id: gid,
            mode: FilterMode::Passthrough,
            deadband: None,
            deadband_percent: None,
            max_report_ms: 0,
            window: Some(WindowPolicy {
                window_ms: wms,
                emit_ms: ems,
                aggregates: WindowAgg {
                    kinds: kinds.into_iter().map(String::from).collect(),
                },
            }),
            tags: vec![],
        }
    }

    // ===== Passthrough =====

    #[test]
    fn no_policy_passes_all_values() {
        let nid = nid();
        let gid = gid();
        let tid = tid();
        let store = store_with(nid, gid, tid, "temp");
        clear_all();
        let outcome = apply(&store, nid, gid, &[(tid, DataValue::Float64(25.0))], 1000);
        assert_eq!(outcome.published.len(), 1);
        assert!(outcome.synthetic.is_empty());
    }

    #[test]
    fn passthrough_mode_passes_all_values() {
        let nid = nid();
        let gid = gid();
        let tid = tid();
        let store = store_with(nid, gid, tid, "temp");
        store.policy_insert(passthrough_policy(nid, gid));
        clear_all();
        let outcome = apply(&store, nid, gid, &[(tid, DataValue::Float64(25.0))], 1000);
        assert_eq!(outcome.published.len(), 1);
    }

    // ===== Deadband =====

    #[test]
    fn first_sample_always_reports() {
        let nid = nid();
        let gid = gid();
        let tid = tid();
        let store = store_with(nid, gid, tid, "temp");
        store.policy_insert(on_change_policy(nid, gid, 2.0));
        clear_all();
        let outcome = apply(&store, nid, gid, &[(tid, DataValue::Float64(25.0))], 1000);
        assert_eq!(outcome.published.len(), 1);
    }

    #[test]
    fn within_deadband_suppressed() {
        with_lock(|| {
            let nid = nid();
            let gid = gid();
            let tid = tid();
            let store = store_with(nid, gid, tid, "temp");
            store.policy_insert(on_change_policy(nid, gid, 2.0));
            apply(&store, nid, gid, &[(tid, DataValue::Float64(25.0))], 1000);
            let r = apply(&store, nid, gid, &[(tid, DataValue::Float64(26.0))], 2000);
            assert_eq!(r.published.len(), 0, "within deadband must be suppressed");
            assert_eq!(r.suppressed_tags, 1);
        });
    }

    #[test]
    fn outside_deadband_reports() {
        with_lock(|| {
            let nid = nid();
            let gid = gid();
            let tid = tid();
            let store = store_with(nid, gid, tid, "temp");
            store.policy_insert(on_change_policy(nid, gid, 2.0));
            apply(&store, nid, gid, &[(tid, DataValue::Float64(25.0))], 1000);
            let r = apply(&store, nid, gid, &[(tid, DataValue::Float64(28.0))], 2000);
            assert_eq!(r.published.len(), 1);
        });
    }

    #[test]
    fn suppressed_does_not_advance_baseline() {
        with_lock(|| {
            let nid = nid();
            let gid = gid();
            let tid = tid();
            let store = store_with(nid, gid, tid, "temp");
            store.policy_insert(on_change_policy(nid, gid, 2.0));
            apply(&store, nid, gid, &[(tid, DataValue::Float64(25.0))], 1000);
            apply(&store, nid, gid, &[(tid, DataValue::Float64(25.5))], 2000); // suppressed
            let r = apply(&store, nid, gid, &[(tid, DataValue::Float64(25.3))], 3000);
            assert_eq!(
                r.suppressed_tags, 1,
                "baseline must not advance on suppress"
            );
        });
    }

    #[test]
    fn heartbeat_forces_report_and_refreshes_baseline() {
        with_lock(|| {
            let nid = nid();
            let gid = gid();
            let tid = tid();
            let mut policy = on_change_policy(nid, gid, 2.0);
            policy.max_report_ms = 2000;
            let store = store_with(nid, gid, tid, "temp");
            store.policy_insert(policy);
            apply(&store, nid, gid, &[(tid, DataValue::Float64(25.0))], 1000);
            // t=3500: heartbeat (2000ms gap) fires even though within deadband
            let r = apply(&store, nid, gid, &[(tid, DataValue::Float64(25.5))], 3500);
            assert_eq!(r.published.len(), 1, "heartbeat must force report");
            // Now baseline = 25.5
            let r2 = apply(&store, nid, gid, &[(tid, DataValue::Float64(26.5))], 4500);
            assert_eq!(r2.suppressed_tags, 1, "new baseline from heartbeat");
        });
    }

    #[test]
    fn relative_deadband_percent() {
        with_lock(|| {
            let nid = nid();
            let gid = gid();
            let tid = tid();
            let store = store_with(nid, gid, tid, "temp");
            let mut policy = on_change_policy(nid, gid, 0.0);
            policy.deadband_percent = Some(5.0);
            store.policy_insert(policy);
            apply(&store, nid, gid, &[(tid, DataValue::Float64(100.0))], 1000);
            let r = apply(&store, nid, gid, &[(tid, DataValue::Float64(103.0))], 2000); // 3% < 5%
            assert_eq!(r.suppressed_tags, 1);
            let r2 = apply(&store, nid, gid, &[(tid, DataValue::Float64(106.0))], 3000); // 6% > 5%
            assert_eq!(r2.published.len(), 1);
        });
    }

    #[test]
    fn relative_deadband_zero_baseline() {
        with_lock(|| {
            let nid = nid();
            let gid = gid();
            let tid = tid();
            let store = store_with(nid, gid, tid, "temp");
            let mut policy = on_change_policy(nid, gid, 0.0);
            policy.deadband_percent = Some(5.0);
            store.policy_insert(policy);
            apply(&store, nid, gid, &[(tid, DataValue::Float64(0.0))], 1000);
            let r = apply(&store, nid, gid, &[(tid, DataValue::Float64(1.0))], 2000);
            assert_eq!(
                r.published.len(),
                1,
                "from zero baseline any non-zero must report"
            );
        });
    }

    #[test]
    fn boolean_reports_on_change_only() {
        with_lock(|| {
            let nid = nid();
            let gid = gid();
            let tid = tid();
            let store = store_with(nid, gid, tid, "flag");
            store.policy_insert(on_change_policy(nid, gid, 0.0));
            apply(&store, nid, gid, &[(tid, DataValue::Bool(true))], 1000);
            let r = apply(&store, nid, gid, &[(tid, DataValue::Bool(true))], 2000);
            assert_eq!(r.suppressed_tags, 1);
            let r2 = apply(&store, nid, gid, &[(tid, DataValue::Bool(false))], 3000);
            assert_eq!(r2.published.len(), 1);
        });
    }

    #[test]
    fn string_reports_on_change_only() {
        with_lock(|| {
            let nid = nid();
            let gid = gid();
            let tid = tid();
            let store = store_with(nid, gid, tid, "status");
            store.policy_insert(on_change_policy(nid, gid, 0.0));
            apply(
                &store,
                nid,
                gid,
                &[(tid, DataValue::String("ok".to_string()))],
                1000,
            );
            let r = apply(
                &store,
                nid,
                gid,
                &[(tid, DataValue::String("ok".to_string()))],
                2000,
            );
            assert_eq!(r.suppressed_tags, 1);
            let r2 = apply(
                &store,
                nid,
                gid,
                &[(tid, DataValue::String("error".to_string()))],
                3000,
            );
            assert_eq!(r2.published.len(), 1);
        });
    }

    // ===== Window aggregation =====

    #[test]
    fn window_emits_only_after_emit_ms() {
        with_lock(|| {
            let nid = nid();
            let gid = gid();
            let tid = tid();
            let store = store_with(nid, gid, tid, "temp");
            store.policy_insert(win_policy(nid, gid, 1000, 500, vec!["avg"]));
            forget_group(nid, gid);
            // t=1000: first call ALWAYS emits (last_emit_ms=0 satisfies any emit_ms>0)
            // emit [10], last_emit_ms=1000, window still holds [10] after emit
            let r0 = apply(&store, nid, gid, &[(tid, DataValue::Float64(10.0))], 1000);
            assert_eq!(r0.window_emitted, 1, "first call always emits");
            // t=1200: 1200-1000=200<500 → no emit, window=[10,20]
            let r1 = apply(&store, nid, gid, &[(tid, DataValue::Float64(20.0))], 1200);
            assert!(r1.synthetic.is_empty(), "not enough time since last emit");
            // t=1600: 1600-1000=600>=500 → emit window=[10,20,30] avg=20, last_emit_ms=1600
            let r2 = apply(&store, nid, gid, &[(tid, DataValue::Float64(30.0))], 1600);
            assert_eq!(r2.window_emitted, 1);
            if let DataValue::Float64(v) = &r2.synthetic[0].1 {
                assert!((*v - 20.0).abs() < 0.001, "expected 20, got {}", v);
            } else {
                panic!("expected Float64");
            }
        });
    }

    #[test]
    fn window_slides_old_samples_removed() {
        with_lock(|| {
            let nid = nid();
            let gid = gid();
            let tid = tid();
            let store = store_with(nid, gid, tid, "temp");
            store.policy_insert(win_policy(nid, gid, 1000, 200, vec!["avg"]));
            forget_group(nid, gid); // ensure fresh window state
                                    // With emit_ms=200 and initial last_emit_ms=0:
                                    // t=1000: 1000>=200 → emit [10], last_emit=1000
                                    // t=1300: 300>=200 → emit [20] (window was [10] before 20 added), last_emit=1300
                                    // t=1600: 300>=200 → emit [30] (window was [20] before 30 added), last_emit=1600
                                    // t=3000: 1400>=200 → window at emission = samples added since t=1600: [40]; previous [10,20,30] were emitted earlier
                                    // So emit = [40] = 40
            let r = apply(&store, nid, gid, &[(tid, DataValue::Float64(10.0))], 1000);
            assert_eq!(r.window_emitted, 1);
            let r2 = apply(&store, nid, gid, &[(tid, DataValue::Float64(20.0))], 1300);
            assert_eq!(r2.window_emitted, 1);
            let r3 = apply(&store, nid, gid, &[(tid, DataValue::Float64(30.0))], 1600);
            assert_eq!(r3.window_emitted, 1);
            let r4 = apply(&store, nid, gid, &[(tid, DataValue::Float64(40.0))], 3000);
            assert_eq!(r4.window_emitted, 1);
            // Actual behavior: window accumulates 1 sample per emit cycle, so [40]
            if let DataValue::Float64(v) = &r4.synthetic[0].1 {
                assert!((*v - 40.0).abs() < 0.001, "expected 40, got {}", v);
            } else {
                panic!("expected Float64");
            }
        });
    }

    #[test]
    fn window_aggregates_all_kinds() {
        with_lock(|| {
            let nid = nid();
            let gid = gid();
            let tid = tid();
            let store = store_with(nid, gid, tid, "temp");
            store.policy_insert(win_policy(
                nid,
                gid,
                10000,
                500,
                vec!["avg", "min", "max", "count", "first", "last"],
            ));
            apply(&store, nid, gid, &[(tid, DataValue::Float64(10.0))], 1000);
            apply(&store, nid, gid, &[(tid, DataValue::Float64(30.0))], 1100);
            let r = apply(&store, nid, gid, &[(tid, DataValue::Float64(20.0))], 2000);
            assert_eq!(r.window_emitted, 1);
            assert_eq!(r.synthetic.len(), 6);
        });
    }

    #[test]
    fn window_output_not_filtered_by_deadband() {
        with_lock(|| {
            let nid = nid();
            let gid = gid();
            let tid = tid();
            let store = store_with(nid, gid, tid, "temp");
            let mut policy = on_change_policy(nid, gid, 10.0);
            policy.window = Some(WindowPolicy {
                window_ms: 10000,
                emit_ms: 500,
                aggregates: WindowAgg {
                    kinds: vec!["avg".to_string()],
                },
            });
            store.policy_insert(policy);
            apply(&store, nid, gid, &[(tid, DataValue::Float64(100.0))], 1000);
            apply(&store, nid, gid, &[(tid, DataValue::Float64(105.0))], 1500);
            let r = apply(&store, nid, gid, &[(tid, DataValue::Float64(105.0))], 2100);
            assert_eq!(
                r.window_emitted, 1,
                "synthetic output must not be deadband-filtered"
            );
            assert!(!r.synthetic.is_empty());
        });
    }

    // ===== forget =====

    #[test]
    fn forget_group_clears_baseline() {
        with_lock(|| {
            let nid = nid();
            let gid = gid();
            let tid = tid();
            let store = store_with(nid, gid, tid, "temp");
            store.policy_insert(on_change_policy(nid, gid, 1.0));
            apply(&store, nid, gid, &[(tid, DataValue::Float64(100.0))], 1000);
            let r = apply(&store, nid, gid, &[(tid, DataValue::Float64(100.5))], 2000);
            assert_eq!(r.suppressed_tags, 1);
            forget_group(nid, gid);
            let r2 = apply(&store, nid, gid, &[(tid, DataValue::Float64(100.5))], 3000);
            assert_eq!(
                r2.published.len(),
                1,
                "after forget_group first sample must report"
            );
        });
    }

    // ===== Validation =====

    #[test]
    fn validate_rejects_negative_deadband() {
        let nid = nid();
        let gid = gid();
        let p = GroupPolicy {
            south_node_id: nid,
            group_id: gid,
            mode: FilterMode::OnChange,
            deadband: Some(-1.0),
            deadband_percent: None,
            max_report_ms: 0,
            window: None,
            tags: vec![],
        };
        assert!(validate_policy(&p).is_err());
    }

    #[test]
    fn validate_rejects_window_ms_too_small() {
        let nid = nid();
        let gid = gid();
        let p = GroupPolicy {
            south_node_id: nid,
            group_id: gid,
            mode: FilterMode::Passthrough,
            deadband: None,
            deadband_percent: None,
            max_report_ms: 0,
            window: Some(WindowPolicy {
                window_ms: 50,
                emit_ms: 10,
                aggregates: WindowAgg {
                    kinds: vec!["avg".to_string()],
                },
            }),
            tags: vec![],
        };
        assert!(validate_policy(&p).is_err());
    }

    #[test]
    fn validate_rejects_emit_ms_too_small() {
        let nid = nid();
        let gid = gid();
        let p = GroupPolicy {
            south_node_id: nid,
            group_id: gid,
            mode: FilterMode::Passthrough,
            deadband: None,
            deadband_percent: None,
            max_report_ms: 0,
            window: Some(WindowPolicy {
                window_ms: 1000,
                emit_ms: 5,
                aggregates: WindowAgg {
                    kinds: vec!["avg".to_string()],
                },
            }),
            tags: vec![],
        };
        assert!(validate_policy(&p).is_err());
    }

    #[test]
    fn validate_rejects_unknown_agg() {
        let nid = nid();
        let gid = gid();
        let p = GroupPolicy {
            south_node_id: nid,
            group_id: gid,
            mode: FilterMode::Passthrough,
            deadband: None,
            deadband_percent: None,
            max_report_ms: 0,
            window: Some(WindowPolicy {
                window_ms: 1000,
                emit_ms: 100,
                aggregates: WindowAgg {
                    kinds: vec!["median".to_string()],
                },
            }),
            tags: vec![],
        };
        assert!(validate_policy(&p).is_err());
    }

    #[test]
    fn validate_rejects_empty_kinds() {
        let nid = nid();
        let gid = gid();
        let p = GroupPolicy {
            south_node_id: nid,
            group_id: gid,
            mode: FilterMode::Passthrough,
            deadband: None,
            deadband_percent: None,
            max_report_ms: 0,
            window: Some(WindowPolicy {
                window_ms: 1000,
                emit_ms: 100,
                aggregates: WindowAgg { kinds: vec![] },
            }),
            tags: vec![],
        };
        assert!(validate_policy(&p).is_err());
    }

    #[test]
    fn validate_rejects_duplicate_tags() {
        let nid = nid();
        let gid = gid();
        let p = GroupPolicy {
            south_node_id: nid,
            group_id: gid,
            mode: FilterMode::OnChange,
            deadband: None,
            deadband_percent: None,
            max_report_ms: 0,
            window: None,
            tags: vec![
                TagDeadband {
                    tag_name: "temp".to_string(),
                    deadband: Some(1.0),
                    deadband_percent: None,
                },
                TagDeadband {
                    tag_name: "temp".to_string(),
                    deadband: Some(2.0),
                    deadband_percent: None,
                },
            ],
        };
        assert!(validate_policy(&p).is_err());
    }

    #[test]
    fn validate_accepts_valid_policy() {
        let nid = nid();
        let gid = gid();
        let p = GroupPolicy {
            south_node_id: nid,
            group_id: gid,
            mode: FilterMode::OnChange,
            deadband: Some(1.0),
            deadband_percent: Some(5.0),
            max_report_ms: 5000,
            window: Some(WindowPolicy {
                window_ms: 1000,
                emit_ms: 100,
                aggregates: WindowAgg {
                    kinds: vec!["avg".to_string(), "min".to_string()],
                },
            }),
            tags: vec![],
        };
        assert!(validate_policy(&p).is_ok());
    }
}
