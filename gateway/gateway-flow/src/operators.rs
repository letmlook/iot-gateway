//! MVP 算子集（design §1/§4.1）：threshold_alarm、state_change_alarm、route、
//! alarm_to_data、sink_north、sink_alarm。脚本算子（rhai）与 rate_of_change 明确
//! 不在 MVP（沙箱/语义专项）。

use chrono::Utc;
use gateway_sdk::{DataValue, GroupData};
use serde::Deserialize;
use std::collections::HashMap;
use uuid::Uuid;

use crate::error::FlowError;
use crate::payload::{AlarmEvent, AlarmLevel, FlowPayload};

/// 算子/端点注册表（server 侧 `GET /flows/operators` 发现用，前端不硬编码——踩坑 P6 对策）。
pub const OPERATOR_NAMES: [&str; 6] = [
    "threshold_alarm",
    "state_change_alarm",
    "route",
    "alarm_to_data",
    "sink_north",
    "sink_alarm",
];

/// Source 节点配置：绑定的南向 (节点, 组)。流程输入只允许来自南向组
/// （设计 §3.4：flow 不订阅 flow 输出，构造性禁环）。
#[derive(Debug, Deserialize)]
pub struct SourceConfig {
    pub south_node_id: Uuid,
    pub group_id: Uuid,
}

/// Source 配置校验（publish 门禁的一部分）。
pub fn validate_source_config(config: &serde_json::Value) -> Result<(), String> {
    serde_json::from_value::<SourceConfig>(config.clone())
        .map(|_| ())
        .map_err(|e| format!("invalid source config (need south_node_id + group_id): {e}"))
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
    state: &mut HashMap<Uuid, HashMap<String, String>>,
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
            let (alarm, normal) = eval_state_change(&cfg, node_id, state, &gd);
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
        let mut state = HashMap::new();
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
}
