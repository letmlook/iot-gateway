//! MVP 算子集（design §1/§4.1）：threshold_alarm、state_change_alarm、route、
//! sink_north、sink_alarm。脚本算子（rhai）与 rate_of_change 明确不在 MVP（沙箱/语义专项）。

use chrono::Utc;
use gateway_sdk::{DataValue, GroupData};
use serde::Deserialize;
use std::collections::HashMap;
use uuid::Uuid;

use crate::error::FlowError;
use crate::payload::{AlarmEvent, AlarmLevel, FlowPayload};

/// 算子/端点注册表（server 侧 `GET /flows/operators` 发现用，前端不硬编码——踩坑 P6 对策）。
pub const OPERATOR_NAMES: [&str; 5] = [
    "threshold_alarm",
    "state_change_alarm",
    "route",
    "sink_north",
    "sink_alarm",
];

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

/// 算子统一入口（engine 按计划调用）。返回 (输出 port → 载荷) 映射。
pub fn evaluate_node(
    operator: &str,
    node_id: Uuid,
    config: &serde_json::Value,
    input: FlowPayload,
    state: &mut HashMap<Uuid, HashMap<String, String>>,
) -> Result<Vec<(&'static str, FlowPayload)>, FlowError> {
    // MVP 算子都只接受 Data 输入（alarm 事件流不进处理算子——validate 保证图形态）
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
