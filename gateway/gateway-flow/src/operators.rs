//! MVP 算子集（design §1/§4.1）：threshold_alarm、state_change_alarm、route、
//! alarm_to_data、sink_north、sink_alarm、rate_of_change、deduplicate（流算子第二批补入）、
//! expr_route（rhai 沙箱，spec §四）。

use chrono::Utc;
use gateway_sdk::{DataValue, GroupData};
use serde::Deserialize;
use std::collections::HashMap;
use uuid::Uuid;

use crate::error::FlowError;
use crate::payload::{AlarmEvent, AlarmLevel, FlowPayload};

/// 算子/端点注册表（server 侧 `GET /flows/operators` 发现用，前端不硬编码——踩坑 P6 对策）。
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

/// Source 节点配置：绑定的南向 (节点, 组)。流程输入只允许来自南向组
/// （设计 §3.4：flow 不订阅 flow 输出，构造性禁环）。
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

/// Source 配置校验（publish 门禁的一部分）。
pub fn validate_source_config(config: &serde_json::Value) -> Result<(), String> {
    let c: SourceConfig = serde_json::from_value(config.clone())
        .map_err(|e| format!("invalid source config (need south_node_id + group_id): {e}"))?;
    if c.stale_timeout_ms > 0 {
        AlarmLevel::parse(&c.stale_level)?;
    }
    Ok(())
}

/// 节点端口规范（校验与编辑器面板共用）。
pub struct PortSpec {
    pub inputs: &'static [&'static str],
    pub outputs: &'static [&'static str],
}

impl PortSpec {
    pub fn source() -> Self {
        PortSpec {
            inputs: &[],
            outputs: &["out"],
        }
    }
    pub fn operator(name: &str) -> Self {
        match name {
            "threshold_alarm" | "state_change_alarm" => PortSpec {
                inputs: &["in"],
                outputs: &["alarm", "normal"],
            },
            "route" => PortSpec {
                inputs: &["in"],
                outputs: &["true", "false"],
            },
            "rate_of_change" => PortSpec {
                inputs: &["in"],
                outputs: &["alarm", "normal"],
            },
            "deduplicate" => PortSpec {
                inputs: &["in"],
                outputs: &["out"],
            },
            "expr_route" => PortSpec {
                inputs: &["in"],
                outputs: &["true", "false"],
            },
            // 告警 → 数据桥：把 Alarm 载荷转成 GroupData，交给 sink_north（MQTT/Kafka/
            // Webhook/SMTP 等北向节点）推送——告警通知渠道（蓝图 §7 Q6）的实现基础
            "alarm_to_data" => PortSpec {
                inputs: &["in"],
                outputs: &["out"],
            },
            _ => PortSpec {
                inputs: &["in"],
                outputs: &[],
            },
        }
    }
    pub fn sink(name: &str) -> Self {
        PortSpec {
            inputs: &["in"],
            outputs: &[],
        }
        .with_kind_check(name)
    }
    fn with_kind_check(self, _name: &str) -> Self {
        self
    }
}

/// 配置校验（publish 门禁的一部分；错误信息需能直接呈给用户）。
pub fn validate_config(operator: &str, config: &serde_json::Value) -> Result<(), String> {
    match operator {
        "threshold_alarm" => {
            let c: ThresholdConfig = serde_json::from_value(config.clone())
                .map_err(|e| format!("invalid config: {e}"))?;
            if c.tag_name.is_empty() {
                return Err("tag_name is required".into());
            }
            if c.high.is_none() && c.low.is_none() {
                return Err("at least one of high/low is required".into());
            }
            AlarmLevel::parse(&c.level)?;
            Ok(())
        }
        "state_change_alarm" => {
            let c: StateChangeConfig = serde_json::from_value(config.clone())
                .map_err(|e| format!("invalid config: {e}"))?;
            if c.tag_name.is_empty() {
                return Err("tag_name is required".into());
            }
            if c.to.is_empty() {
                return Err("to is required".into());
            }
            AlarmLevel::parse(&c.level)?;
            Ok(())
        }
        "route" => {
            let c: RouteConfig = serde_json::from_value(config.clone())
                .map_err(|e| format!("invalid config: {e}"))?;
            if c.tag_name.is_empty() {
                return Err("tag_name is required".into());
            }
            if !matches!(c.op.as_str(), "gt" | "lt" | "ge" | "le" | "eq" | "ne") {
                return Err(format!("unknown op `{}`", c.op));
            }
            Ok(())
        }
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
        "sink_north" => {
            let c: SinkNorthConfig = serde_json::from_value(config.clone())
                .map_err(|e| format!("invalid config: {e}"))?;
            // 北向节点存在性在 server 接线层校验（crate 不感知节点表）
            if c.node_id.is_nil() {
                return Err("node_id (north node) is required".into());
            }
            Ok(())
        }
        "alarm_to_data" => Ok(()),
        "sink_alarm" => Ok(()),
        "deduplicate" => Ok(()),
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
        other => Err(format!("unknown operator `{other}`")),
    }
}

// ---------- 各算子配置 ----------

#[derive(Debug, Deserialize)]
pub struct ThresholdConfig {
    pub tag_name: String,
    #[serde(default)]
    pub high: Option<f64>,
    #[serde(default)]
    pub low: Option<f64>,
    pub level: String,
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct StateChangeConfig {
    pub tag_name: String,
    /// 上一态；`*` = 任意（缺省 `*`）
    #[serde(default = "default_any")]
    pub from: String,
    pub to: String,
    pub level: String,
    #[serde(default)]
    pub message: Option<String>,
}

fn default_any() -> String {
    "*".into()
}

#[derive(Debug, Deserialize)]
pub struct RouteConfig {
    pub tag_name: String,
    /// gt | lt | ge | le | eq | ne
    pub op: String,
    pub value: f64,
}

#[derive(Debug, Deserialize)]
pub struct RateOfChangeConfig {
    pub tag_name: String,
    /// 速率上限（正值，单位 = 值/秒）
    pub max_rate: f64,
    pub level: String,
    #[serde(default)]
    pub message: Option<String>,
}

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

/// deduplicate 配置：无字段（spec §三）。
#[derive(Debug, Deserialize)]
pub struct DeduplicateConfig {}

#[derive(Debug, Deserialize)]
pub struct SinkNorthConfig {
    /// 目标北向节点 id（server 接线层据此投递 `on_group_data`）
    pub node_id: Uuid,
}

// ---------- 求值 ----------

/// name → value 视图（GroupData.values + tag_names 反查）。
pub fn value_by_name<'a>(gd: &'a GroupData, tag_name: &str) -> Option<&'a DataValue> {
    let names = gd.tag_names.as_ref()?;
    for (tag_id, value) in &gd.values {
        if names.get(tag_id).map(String::as_str) == Some(tag_name) {
            return Some(value);
        }
    }
    None
}

pub fn eval_threshold(
    cfg: &ThresholdConfig,
    gd: &GroupData,
) -> (Option<FlowPayload>, Option<FlowPayload>) {
    let hit = value_by_name(gd, &cfg.tag_name)
        .and_then(|v| v.as_f64())
        .and_then(|v| {
            let over_high = cfg.high.map(|h| v > h).unwrap_or(false);
            let under_low = cfg.low.map(|l| v < l).unwrap_or(false);
            (over_high || under_low).then_some(v)
        });
    match hit {
        Some(v) => {
            let event = AlarmEvent {
                level: AlarmLevel::parse(&cfg.level).unwrap_or(AlarmLevel::Info),
                tag: cfg.tag_name.clone(),
                value: Some(v),
                threshold: cfg.high.or(cfg.low),
                message: cfg.message.clone().unwrap_or_else(|| {
                    format!("{}={} crossed {:?}", cfg.tag_name, v, cfg.high.or(cfg.low))
                }),
                ts: Utc::now(),
            };
            (
                Some(FlowPayload::Alarm(event)),
                Some(FlowPayload::Data(gd.clone())),
            )
        }
        None => (None, Some(FlowPayload::Data(gd.clone()))),
    }
}

/// state_change 求值。`state`：flow 节点 id → tag_name → 上一态（显示串）。
/// 状态不落盘、重启清零（与 filters/rules 运行态语义一致）。
pub fn eval_state_change(
    cfg: &StateChangeConfig,
    node_id: Uuid,
    state: &mut HashMap<Uuid, HashMap<String, String>>,
    gd: &GroupData,
) -> (Option<FlowPayload>, Option<FlowPayload>) {
    let current = value_by_name(gd, &cfg.tag_name).map(display_value);
    let node_state = state.entry(node_id).or_default();
    let last = current
        .as_ref()
        .and_then(|_| node_state.get(&cfg.tag_name).cloned());
    let triggered = match (&last, &current) {
        (Some(last), Some(cur)) => (cfg.from == "*" || last == &cfg.from) && cur == &cfg.to,
        _ => false,
    };
    if let Some(cur) = &current {
        node_state.insert(cfg.tag_name.clone(), cur.clone());
    }
    if !triggered {
        return (None, Some(FlowPayload::Data(gd.clone())));
    }
    let event = AlarmEvent {
        level: AlarmLevel::parse(&cfg.level).unwrap_or(AlarmLevel::Info),
        tag: cfg.tag_name.clone(),
        value: current
            .as_ref()
            .and_then(|_| value_by_name(gd, &cfg.tag_name))
            .and_then(|v| v.as_f64()),
        threshold: None,
        message: cfg.message.clone().unwrap_or_else(|| {
            format!(
                "{}: {} -> {}",
                cfg.tag_name,
                last.as_deref().unwrap_or("?"),
                cfg.to
            )
        }),
        ts: Utc::now(),
    };
    (
        Some(FlowPayload::Alarm(event)),
        Some(FlowPayload::Data(gd.clone())),
    )
}

pub fn eval_route(cfg: &RouteConfig, gd: &GroupData) -> (Option<FlowPayload>, Option<FlowPayload>) {
    let v = value_by_name(gd, &cfg.tag_name).and_then(|v| v.as_f64());
    let cond = v.map(|v| match cfg.op.as_str() {
        "gt" => v > cfg.value,
        "lt" => v < cfg.value,
        "ge" => v >= cfg.value,
        "le" => v <= cfg.value,
        "eq" => v == cfg.value,
        "ne" => v != cfg.value,
        _ => false,
    });
    // 缺值走 false 端（缺测不误触发）
    match cond {
        Some(true) => (Some(FlowPayload::Data(gd.clone())), None),
        _ => (None, Some(FlowPayload::Data(gd.clone()))),
    }
}

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
    let result: Result<rhai::Dynamic, _> = entry.engine.eval_ast_with_scope(&mut scope, &entry.ast);
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

fn display_value(v: &DataValue) -> String {
    match v {
        DataValue::String(s) => s.clone(),
        other => other
            .as_f64()
            .map(|f| f.to_string())
            .unwrap_or_else(|| format!("{other:?}")),
    }
}

// ---------- 告警 → 数据桥（alarm_to_data） ----------

/// 告警伪源（GroupData.node_name/group_name）：北向 topic 模板可引用
/// `{node_name}`/`{group_name}`，告警帧统一以 alarms/events 呈现。
pub const ALARM_SOURCE_NODE: &str = "alarms";
pub const ALARM_SOURCE_GROUP: &str = "events";

/// 告警字段的显示名（GroupData.tag_names 用；北向 payload 按此作为字段 key）。
/// tag_id 本身无全局意义，只需 GroupData 内自洽——用固定 u128 生成的稳定 UUID。
pub const ALARM_FIELD_NAMES: [&str; 6] = [
    "alarm_level",
    "alarm_tag",
    "alarm_value",
    "alarm_threshold",
    "alarm_message",
    "alarm_ts",
];

fn alarm_field_tag_id(idx: usize) -> gateway_sdk::TagId {
    // idx 0..=5，one-based u128 避免与 nil UUID 撞车
    gateway_sdk::TagId(uuid::Uuid::from_u128(idx as u128 + 1))
}

/// 把告警事件包装成 GroupData（北向 on_group_data 的载荷形态）：
/// 每个告警字段一个「点位」，value/threshold 缺省时不输出该字段。
/// 北向插件（MQTT/Kafka/Webhook/SMTP）按 tag_names 作为字段 key 正常序列化，
/// 告警通知渠道由此全部复用既有北向能力（蓝图 §7 Q6）。
pub fn alarm_to_group_data(ev: &AlarmEvent) -> GroupData {
    let mut values = Vec::with_capacity(ALARM_FIELD_NAMES.len());
    let mut names = HashMap::new();
    let mut push = |idx: usize, v: DataValue| {
        let tag_id = alarm_field_tag_id(idx);
        names.insert(tag_id, ALARM_FIELD_NAMES[idx].to_string());
        values.push((tag_id, v));
    };
    push(0, DataValue::String(ev.level.as_str().into()));
    push(1, DataValue::String(ev.tag.clone()));
    if let Some(v) = ev.value {
        push(2, DataValue::Float64(v));
    }
    if let Some(t) = ev.threshold {
        push(3, DataValue::Float64(t));
    }
    push(4, DataValue::String(ev.message.clone()));
    push(5, DataValue::String(ev.ts.to_rfc3339()));
    GroupData {
        node_id: gateway_sdk::NodeId(uuid::Uuid::nil()),
        group_id: gateway_sdk::GroupId(uuid::Uuid::nil()),
        ts: ev.ts,
        values,
        node_name: Some(ALARM_SOURCE_NODE.into()),
        group_name: Some(ALARM_SOURCE_GROUP.into()),
        tag_names: Some(names),
    }
}

/// 算子统一入口（engine 按计划调用）。返回 (输出 port → 载荷) 映射。
pub fn evaluate_node(
    operator: &str,
    node_id: Uuid,
    config: &serde_json::Value,
    input: FlowPayload,
    state: &mut crate::engine::FlowState,
) -> Result<Vec<(&'static str, FlowPayload)>, FlowError> {
    // alarm_to_data 是唯一接受 Alarm 输入的算子（告警 → 数据桥）
    if operator == "alarm_to_data" {
        let ev = match input {
            FlowPayload::Alarm(ev) => ev,
            FlowPayload::Data(_) => {
                return Err(FlowError::Operator(
                    operator.to_string(),
                    "alarm_to_data requires Alarm input, got Data".into(),
                ))
            }
        };
        return Ok(vec![("out", FlowPayload::Data(alarm_to_group_data(&ev)))]);
    }
    // 其余 MVP 算子都只接受 Data 输入（alarm 事件流不进处理算子——validate 保证图形态）
    let gd = match input {
        FlowPayload::Data(gd) => gd,
        FlowPayload::Alarm(_) => {
            return Err(FlowError::Operator(
                operator.to_string(),
                "operator requires Data input, got Alarm".into(),
            ))
        }
    };
    match operator {
        "threshold_alarm" => {
            let cfg: ThresholdConfig = serde_json::from_value(config.clone())
                .map_err(|e| FlowError::Operator(operator.into(), e.to_string()))?;
            let (alarm, normal) = eval_threshold(&cfg, &gd);
            let mut out = Vec::new();
            if let Some(a) = alarm {
                out.push(("alarm", a));
            }
            if let Some(n) = normal {
                out.push(("normal", n));
            }
            Ok(out)
        }
        "state_change_alarm" => {
            let cfg: StateChangeConfig = serde_json::from_value(config.clone())
                .map_err(|e| FlowError::Operator(operator.into(), e.to_string()))?;
            let (alarm, normal) = eval_state_change(&cfg, node_id, &mut state.last_values, &gd);
            let mut out = Vec::new();
            if let Some(a) = alarm {
                out.push(("alarm", a));
            }
            if let Some(n) = normal {
                out.push(("normal", n));
            }
            Ok(out)
        }
        "route" => {
            let cfg: RouteConfig = serde_json::from_value(config.clone())
                .map_err(|e| FlowError::Operator(operator.into(), e.to_string()))?;
            let (t, f) = eval_route(&cfg, &gd);
            let mut out = Vec::new();
            if let Some(t) = t {
                out.push(("true", t));
            }
            if let Some(f) = f {
                out.push(("false", f));
            }
            Ok(out)
        }
        "rate_of_change" => {
            let cfg: RateOfChangeConfig = serde_json::from_value(config.clone())
                .map_err(|e| FlowError::Operator(operator.into(), e.to_string()))?;
            let (alarm, normal) = eval_rate_of_change(&cfg, node_id, &mut state.last_rate, &gd);
            let mut out = Vec::new();
            if let Some(a) = alarm {
                out.push(("alarm", a));
            }
            if let Some(n) = normal {
                out.push(("normal", n));
            }
            Ok(out)
        }
        "deduplicate" => {
            serde_json::from_value::<DeduplicateConfig>(config.clone())
                .map_err(|e| FlowError::Operator(operator.into(), e.to_string()))?;
            match eval_deduplicate(node_id, &mut state.last_frames, &gd) {
                Some(out) => Ok(vec![("out", out)]),
                None => Ok(vec![]),
            }
        }
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
        other => Err(FlowError::Operator(
            other.to_string(),
            "not an operator".into(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn sample_alarm(value: Option<f64>, threshold: Option<f64>) -> AlarmEvent {
        AlarmEvent {
            level: AlarmLevel::High,
            tag: "temperature".into(),
            value,
            threshold,
            message: "temperature=95.5 crossed Some(80)".into(),
            ts: chrono::Utc.with_ymd_and_hms(2026, 11, 21, 8, 0, 0).unwrap(),
        }
    }

    #[test]
    fn alarm_to_group_data_carries_all_fields() {
        let gd = alarm_to_group_data(&sample_alarm(Some(95.5), Some(80.0)));
        // 伪源名称：北向 topic 模板可引用 alarms/events
        assert_eq!(gd.node_name.as_deref(), Some(ALARM_SOURCE_NODE));
        assert_eq!(gd.group_name.as_deref(), Some(ALARM_SOURCE_GROUP));
        // 每个字段都有显示名，value/threshold 存在时全部 6 字段输出
        let names = gd.tag_names.as_ref().unwrap();
        assert_eq!(names.len(), 6);
        assert_eq!(
            values_by_field(&gd),
            vec![
                ("alarm_level", "high".to_string()),
                ("alarm_tag", "temperature".to_string()),
                ("alarm_value", "95.5".to_string()),
                ("alarm_threshold", "80".to_string()),
                (
                    "alarm_message",
                    "temperature=95.5 crossed Some(80)".to_string()
                ),
                ("alarm_ts", gd.ts.to_rfc3339()),
            ]
        );
    }

    #[test]
    fn alarm_without_value_threshold_omits_fields() {
        let gd = alarm_to_group_data(&sample_alarm(None, None));
        let names = gd.tag_names.as_ref().unwrap();
        // value/threshold 缺省：不输出对应字段（4 个）
        assert_eq!(names.len(), 4);
        let fields = values_by_field(&gd);
        assert!(!fields.iter().any(|(k, _)| *k == "alarm_value"));
        assert!(!fields.iter().any(|(k, _)| *k == "alarm_threshold"));
    }

    /// 按 tag_names 反查字段序列（显示名 → 值的显示串），供断言用
    fn values_by_field(gd: &GroupData) -> Vec<(&'static str, String)> {
        let names = gd.tag_names.as_ref().unwrap();
        let mut out = Vec::new();
        for (tag_id, v) in &gd.values {
            let name = names.get(tag_id).unwrap().as_str();
            let idx = ALARM_FIELD_NAMES.iter().position(|n| *n == name).unwrap();
            let display = match v {
                DataValue::String(s) => s.clone(),
                other => other.as_f64().unwrap().to_string(),
            };
            out.push((ALARM_FIELD_NAMES[idx], display));
        }
        out
    }

    #[test]
    fn evaluate_node_accepts_alarm_input_only() {
        let mut state = crate::engine::FlowState::default();
        let cfg = serde_json::json!({});
        // Alarm 输入 → ("out", Data)
        let out = evaluate_node(
            "alarm_to_data",
            Uuid::new_v4(),
            &cfg,
            FlowPayload::Alarm(sample_alarm(Some(1.0), None)),
            &mut state,
        )
        .unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].0, "out");
        assert!(matches!(out[0].1, FlowPayload::Data(_)));
        // Data 输入 → 报错
        let gd = GroupData {
            node_id: gateway_sdk::NodeId(Uuid::nil()),
            group_id: gateway_sdk::GroupId(Uuid::nil()),
            ts: Utc::now(),
            values: vec![],
            node_name: None,
            group_name: None,
            tag_names: None,
        };
        assert!(evaluate_node(
            "alarm_to_data",
            Uuid::new_v4(),
            &cfg,
            FlowPayload::Data(gd),
            &mut state
        )
        .is_err());
    }

    #[test]
    fn alarm_to_data_validates_and_has_out_port() {
        assert!(validate_config("alarm_to_data", &serde_json::json!({})).is_ok());
        let spec = PortSpec::operator("alarm_to_data");
        assert!(spec.inputs.contains(&"in"));
        assert!(spec.outputs.contains(&"out"));
    }

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
            validate_config(
                "expr_route",
                &serde_json::json!({"expression": "tags.temp >"})
            )
            .is_err(),
            "语法错误必须拒绝（publish 门禁）"
        );
        assert!(validate_config("expr_route", &serde_json::json!({})).is_err());
        assert!(validate_config(
            "expr_route",
            &serde_json::json!({"expression": "tags.temperature > 50"})
        )
        .is_ok());
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
        let cfg1 = ExprRouteConfig {
            expression: "tags.temperature > 50".into(),
        };
        let (t, f) = eval_expr_route(&cfg1, nid, &mut state.script_asts, &gd);
        assert!(t.is_some() && f.is_none(), "60>50 走 true 端");
        let cfg2 = ExprRouteConfig {
            expression: "tags.temperature < 50".into(),
        };
        let (t, f) = eval_expr_route(&cfg2, nid, &mut state.script_asts, &gd);
        assert!(t.is_none() && f.is_some(), "换表达式后重新编译且语义生效");
    }

    #[test]
    fn expr_route_missing_tag_and_non_bool_go_false() {
        let mut state = crate::engine::FlowState::default();
        let nid = Uuid::new_v4();
        let gd = group_data_f(60.0);
        // 缺位 tag 访问 → 运行期错误 → false 端
        let cfg = ExprRouteConfig {
            expression: "tags.ghost > 10".into(),
        };
        let (t, f) = eval_expr_route(&cfg, nid, &mut state.script_asts, &gd);
        assert!(t.is_none() && f.is_some());
        // 非 bool 返回 → false 端
        let cfg = ExprRouteConfig {
            expression: "tags.temperature".into(),
        };
        let (t, f) = eval_expr_route(&cfg, nid, &mut state.script_asts, &gd);
        assert!(t.is_none() && f.is_some());
    }

    #[test]
    fn expr_route_infinite_loop_hits_operation_limit() {
        // 沙箱生效证明：while true {} 超操作数走 false 端而非挂死
        let mut state = crate::engine::FlowState::default();
        let nid = Uuid::new_v4();
        let gd = group_data_f(60.0);
        let cfg = ExprRouteConfig {
            expression: "while true {}".into(),
        };
        let (t, f) = eval_expr_route(&cfg, nid, &mut state.script_asts, &gd);
        assert!(t.is_none() && f.is_some(), "超操作数必须走 false 端");
    }
}
