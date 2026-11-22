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
    /// 非空子集：avg / min / max / count / first / last / sum / stddev / median / delta / p50 / p90 / p95 / p99
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
    /// 表计累计量；None 或空数组 = 未启用。输出虚拟点位 `{tag_name}_total`，
    /// 走 synthetic 通道（不经过死区）
    #[serde(default)]
    pub totalizer: Option<Vec<TotalizerTag>>,
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
        let valid = [
            "avg", "min", "max", "count", "first", "last", "sum", "stddev", "median",
            "delta", "p50", "p90", "p95", "p99",
        ];
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

/// 单点位 totalizer 运行态（不落盘，重启清零）
#[derive(Debug, Clone, Default)]
struct TotalizerState {
    total: f64,
    last_counter: Option<f64>,
    period_start_ms: u64,
    last_emitted_total: Option<f64>,
    last_emit_ms: u64,
}

/// 单组过滤运行态（基线 + 窗口缓冲）
#[derive(Debug, Default)]
struct GroupFilterState {
    baselines: RwLock<HashMap<TagId, TagBaseline>>,
    windows: RwLock<HashMap<TagId, TagWindow>>,
    /// totalizer 运行态（按真实点位 TagId 索引）
    totalizers: RwLock<HashMap<TagId, TotalizerState>>,
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
// totalizer 状态机
// ---------------------------------------------------------------------------

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
        _ => None,
    }
}

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
                // NaN/Inf 不得进累计器：配 rollover 时 total += NaN 会永久毒化累计值
                if !f.is_finite() {
                    continue;
                }
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
        // 写锁（阻塞其他测试）——必须先拿锁再清空：若先 clear_all 再等锁，
        // 并行测试会在本测试函数体执行中途再次清空全局状态，把 baseline 洗掉，
        // 造成「首采必报」的假阳性（suppress 断言随机失败）。
        let guard = std::sync::RwLock::write(&FILTER_TEST_LOCK);
        clear_all();
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
            totalizer: None,
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
            totalizer: None,
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
            totalizer: None,
        }
    }

    // ===== Passthrough =====

    #[test]
    fn no_policy_passes_all_values() {
        let nid = nid();
        let gid = gid();
        let tid = tid();
        let store = store_with(nid, gid, tid, "temp");
        with_lock(clear_all);
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
        with_lock(clear_all);
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
        with_lock(clear_all);
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

    #[test]
    fn aggregate_sum_stddev_delta() {
        // 教科书样本 [2,4,4,4,5,5,7,9]：sum=40，平方偏差和=32，样本方差=32/7，stddev=√(32/7)
        let vals = [2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0];
        let s: Vec<(u64, f64)> = vals.iter().enumerate().map(|(i, v)| (i as u64, *v)).collect();
        assert_eq!(aggregate("sum", &s), Some(40.0));
        let sd = aggregate("stddev", &s).unwrap();
        assert!((sd - (32.0f64 / 7.0).sqrt()).abs() < 1e-12, "stddev got {sd}");

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
            store.policy_insert(win_policy(nid, gid, 10000, 500, all.clone()));
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

    #[test]
    fn whitelist_kinds_all_implemented() {
        // 守护：白名单 14 种 kind 必须在 aggregate / aggregate_sorted 至少一处有实现分支，
        // 防止白名单与实现分支靠人工同步而脱节。
        let s: Vec<(u64, f64)> = vec![(1, 1.0), (2, 2.0), (3, 3.0)];
        let sorted = [1.0f64, 2.0, 3.0];
        for kind in [
            "avg", "min", "max", "count", "first", "last", "sum", "stddev",
            "median", "delta", "p50", "p90", "p95", "p99",
        ] {
            assert!(
                aggregate(kind, &s).is_some() || aggregate_sorted(kind, &sorted).is_some(),
                "白名单 kind {kind:?} 没有任何实现分支"
            );
        }
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
            totalizer: None,
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
            totalizer: None,
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
            totalizer: None,
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
                    kinds: vec!["bogus".to_string()],
                },
            }),
            tags: vec![],
            totalizer: None,
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
            totalizer: None,
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
            totalizer: None,
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
            totalizer: None,
        };
        assert!(validate_policy(&p).is_ok());
    }

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

    #[test]
    fn totalizer_field_backward_compatible() {
        let json = r#"{"south_node_id":"00000000-0000-0000-0000-000000000000","group_id":"00000000-0000-0000-0000-000000000000","mode":"passthrough"}"#;
        let p: GroupPolicy = serde_json::from_str(json).expect("旧 JSON 必须可反序列化");
        assert!(p.totalizer.is_none());
    }

    // ===== Totalizer 状态机（纯函数）=====

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

    #[test]
    fn totalizer_ignores_non_finite_values() {
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
            // NaN 首见：在状态机入口前被守卫拦截，从未进入状态，不产生输出
            let out = apply(&store, nid, gid, &[(tid, DataValue::Float64(f64::NAN))], 1000);
            assert!(out.synthetic.is_empty());
            // Inf 同样被跳过
            let out = apply(&store, nid, gid, &[(tid, DataValue::Float64(f64::INFINITY))], 2000);
            assert!(out.synthetic.is_empty());
            // 正常读数继续累计：100 → 110 → total=10（NaN/Inf 从未进入状态）
            let out = apply(&store, nid, gid, &[(tid, DataValue::Float64(100.0))], 3000);
            assert!(out.synthetic.is_empty(), "100 是首个有限读数=首见，不输出");
            let out = apply(&store, nid, gid, &[(tid, DataValue::Float64(110.0))], 4000);
            assert_eq!(out.synthetic.len(), 1);
            if let DataValue::Float64(f) = &out.synthetic[0].1 {
                assert!((*f - 10.0).abs() < 1e-9, "total got {f}");
            } else {
                panic!("expect Float64");
            }
        });
    }

    // ===== Property tests（T5 质量纵深）=====
    //
    // 不引入 proptest/quickcheck 依赖：用确定性 xorshift 伪随机批量生成输入，
    // 对文件头「对外承诺」语义的不变量逐条断言。种子固定 → 失败可重现。

    struct PropRng(u64);

    impl PropRng {
        fn new(seed: u64) -> Self {
            Self(seed | 1) // xorshift 全零态是吸收态，保证非零种子
        }
        fn next_u64(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            x
        }
        /// [lo, hi) 内的有限 f64
        fn f64_range(&mut self, lo: f64, hi: f64) -> f64 {
            lo + (self.next_u64() as f64 / u64::MAX as f64) * (hi - lo)
        }
        fn below(&mut self, n: u64) -> u64 {
            self.next_u64() % n
        }
    }

    #[test]
    fn prop_should_report_absolute_deadband_two_sides() {
        // 性质（绝对死区，db>0）：|new-base| <= db 必抑制；|new-base| > db 必上报
        let mut rng = PropRng::new(0xF100_0001);
        for _ in 0..2000 {
            let base = rng.f64_range(-1e6, 1e6);
            let db = rng.f64_range(1e-3, 1e5);
            let inside = rng.f64_range(-db, db); // |Δ| <= db
            let outside = db + rng.f64_range(1e-3, db); // |Δ| > db
            assert!(
                !should_report(base + inside, Some(base), Some(db), None),
                "死区内必须抑制: base={base} db={db} new={}",
                base + inside
            );
            assert!(
                should_report(base + outside, Some(base), Some(db), None),
                "死区外必须上报: base={base} db={db} new={}",
                base + outside
            );
        }
    }

    #[test]
    fn prop_should_report_relative_deadband_two_sides() {
        // 性质（相对死区，base != 0）：|Δ| > pct% * |base| 必上报；<= 必抑制
        let mut rng = PropRng::new(0xF100_0002);
        for _ in 0..2000 {
            let base = rng.f64_range(-1e6, 1e6);
            if base.abs() < 1.0 {
                continue; // 避开退化基线（base==0 有专门语义，已有单测覆盖）
            }
            let pct = rng.f64_range(0.5, 100.0);
            let allowed = (pct / 100.0) * base.abs();
            let inside = rng.f64_range(-allowed, allowed);
            let outside = allowed + rng.f64_range(1e-3, allowed.max(1.0));
            assert!(
                !should_report(base + inside, Some(base), None, Some(pct)),
                "相对死区内必须抑制: base={base} pct={pct} new={}",
                base + inside
            );
            assert!(
                should_report(base + outside, Some(base), None, Some(pct)),
                "相对死区外必须上报: base={base} pct={pct} new={}",
                base + outside
            );
        }
    }

    #[test]
    fn prop_aggregate_order_invariants() {
        // 性质：非空样本集上 min <= avg <= max；count == len；first/last 与首尾一致；空集 → None
        let mut rng = PropRng::new(0xF100_0003);
        let kinds = ["avg", "min", "max", "count", "first", "last"];
        for _ in 0..500 {
            let n = (rng.below(50) + 1) as usize;
            let samples: Vec<(u64, f64)> = (0..n)
                .map(|i| (i as u64, rng.f64_range(-1e9, 1e9)))
                .collect();
            let vals: Vec<f64> = samples.iter().map(|(_, v)| *v).collect();
            let min = vals.iter().cloned().reduce(f64::min).unwrap();
            let max = vals.iter().cloned().reduce(f64::max).unwrap();

            for kind in kinds {
                let a = aggregate(kind, &samples).expect("非空样本必有聚合值");
                match kind {
                    "avg" => {
                        let sum: f64 = vals.iter().sum();
                        assert!((a - sum / n as f64).abs() <= 1e-9 * a.abs().max(1.0));
                        assert!(min <= a && a <= max, "avg 必须落在 [min, max]");
                    }
                    "min" => assert_eq!(a, min),
                    "max" => assert_eq!(a, max),
                    "count" => assert_eq!(a, n as f64),
                    "first" => assert_eq!(a, vals[0]),
                    "last" => assert_eq!(a, vals[n - 1]),
                    _ => unreachable!(),
                }
            }
        }
        assert!(aggregate("avg", &[]).is_none(), "空样本集必须返回 None");
    }

    #[test]
    fn prop_validate_policy_binary_never_panics() {
        // 性质：任意（含非法组合的）policy，validate_policy 要么 Ok 要么返回非空错误，绝不 panic
        let mut rng = PropRng::new(0xF100_0004);
        for _ in 0..1000 {
            let pick = |r: &mut PropRng| {
                let x = r.below(5);
                match x {
                    0 => None,
                    1 => Some(r.f64_range(-1e3, 1e3)), // 含负数（非法）
                    2 => Some(r.f64_range(0.0, 100.0)),
                    3 => Some(f64::INFINITY),            // 非有限（非法）
                    _ => Some(r.f64_range(-5.0, -0.01)), // 负死区（非法）
                }
            };
            let nid = nid();
            let gid = gid();
            let kinds_pool = ["avg", "min", "bogus", ""];
            let kinds: Vec<String> = (0..rng.below(3) + 1)
                .map(|_| kinds_pool[rng.below(4) as usize].to_string())
                .collect();
            let tag_count = rng.below(3);
            let mut names = std::collections::HashSet::<String>::new();
            let mut tags = Vec::new();
            for i in 0..tag_count {
                // 一半概率构造重名（触发去重校验分支）
                let name = if rng.below(2) == 0 && !names.is_empty() {
                    names.iter().next().unwrap().to_string()
                } else {
                    let n = format!("t{i}");
                    names.insert(n.clone());
                    n
                };
                tags.push(TagDeadband {
                    tag_name: name,
                    deadband: pick(&mut rng),
                    deadband_percent: pick(&mut rng),
                });
            }
            let p = GroupPolicy {
                south_node_id: nid,
                group_id: gid,
                mode: if rng.below(2) == 0 {
                    FilterMode::OnChange
                } else {
                    FilterMode::Passthrough
                },
                deadband: pick(&mut rng),
                deadband_percent: pick(&mut rng),
                max_report_ms: rng.below(10_000),
                window: if rng.below(2) == 0 {
                    None
                } else {
                    Some(WindowPolicy {
                        window_ms: [0, 50, 1000][rng.below(3) as usize], // 含 <100（非法）
                        emit_ms: [0, 5, 100][rng.below(3) as usize],     // 含 <10（非法）
                        aggregates: WindowAgg { kinds },
                    })
                },
                tags,
                totalizer: None,
            };
            match validate_policy(&p) {
                Ok(()) => {}
                Err(e) => assert!(!e.is_empty(), "校验失败必须带非空错误信息"),
            }
        }
    }

    #[test]
    fn prop_passthrough_identity() {
        // 性质：passthrough 模式（与无策略）对任意输入恒等透传，顺序与值都不变
        let mut rng = PropRng::new(0xF100_0005);
        let nid = nid();
        let gid = gid();
        let tid = tid();
        let store = store_with(nid, gid, tid, "temp");
        store.policy_insert(passthrough_policy(nid, gid));
        with_lock(clear_all);
        let values: Vec<(TagId, DataValue)> = (0..200)
            .map(|_i| {
                let v = match rng.below(3) {
                    0 => DataValue::Float64(rng.f64_range(-1e6, 1e6)),
                    1 => DataValue::Bool(rng.below(2) == 1),
                    _ => DataValue::String(rng.next_u64().to_string()),
                };
                (tid, v)
            })
            .collect();
        let expect = values.clone();
        let out = apply(&store, nid, gid, &values, 1000);
        assert_eq!(out.published, expect, "passthrough 必须逐值恒等透传");
        assert!(out.synthetic.is_empty());
        assert_eq!(out.suppressed_tags, 0);
        assert_eq!(out.suppressed_msgs, 0);
    }

    #[test]
    fn prop_apply_onchange_conservation() {
        // 性质（onChange，数值点位）：published + suppressed_tags == 输入点位数（每个点位
        // 要么发布要么抑制，不存在第三种去向）；suppressed_msgs == 1 当且仅当本批零输出
        let mut rng = PropRng::new(0xF100_0006);
        let nid = nid();
        let gid = gid();
        let tid = tid();
        let store = store_with(nid, gid, tid, "temp");
        let mut policy = on_change_policy(nid, gid, rng.f64_range(0.01, 50.0));
        if rng.below(2) == 1 {
            policy.deadband_percent = Some(rng.f64_range(1.0, 50.0));
        }
        if rng.below(3) == 1 {
            policy.max_report_ms = rng.below(5000);
        }
        store.policy_insert(policy);
        with_lock(clear_all);

        let mut ts = 1000u64;
        for _ in 0..300 {
            let batch: Vec<(TagId, DataValue)> = (0..rng.below(4) + 1)
                .map(|_| (tid, DataValue::Float64(rng.f64_range(-100.0, 100.0))))
                .collect();
            let expect_len = batch.len();
            let out = apply(&store, nid, gid, &batch, ts);
            assert_eq!(
                out.published.len() as u64 + out.suppressed_tags,
                expect_len as u64,
                "每个点位必须恰好有一个去向（发布或抑制）"
            );
            let zero_output = out.published.is_empty() && out.synthetic.is_empty();
            assert_eq!(
                out.suppressed_msgs,
                u64::from(zero_output),
                "suppressed_msgs 必须等于「本批零输出」的指示位"
            );
            ts += rng.below(3000) + 1; // 单调推进，覆盖心跳区间
        }
    }

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
}
