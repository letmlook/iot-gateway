//! Alarm operator: threshold/rate-of-change/state-change alarm rules.
//!
//! Evaluates incoming data against configured alarm rules and routes
//! to alarm or normal port based on whether any rule triggered.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use gateway_sdk::{
    Operable, PipelineData, PluginConfig, PluginMeta, PluginResult,
    NodeId,
};
use chrono::{DateTime, Utc};

/// Alarm level (severity)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AlarmLevel {
    Critical,
    High,
    Medium,
    Low,
    Info,
}

impl AlarmLevel {
    fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "critical" => AlarmLevel::Critical,
            "high" => AlarmLevel::High,
            "medium" => AlarmLevel::Medium,
            "low" => AlarmLevel::Low,
            _ => AlarmLevel::Info,
        }
    }
}

/// Alarm rule types
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AlarmRule {
    /// Threshold alarm: triggers when value exceeds limits
    Threshold {
        tag: String,
        #[serde(default)]
        high: Option<f64>,
        #[serde(default)]
        low: Option<f64>,
        level: String,
        #[serde(default)]
        message: Option<String>,
    },
    /// Rate-of-change alarm: triggers when value changes too fast
    RateOfChange {
        tag: String,
        threshold: f64,       // max change per second
        level: String,
        #[serde(default)]
        message: Option<String>,
    },
    /// State-change alarm: triggers on specific state transitions
    StateChange {
        tag: String,
        from: String,         // previous value ("*" for any)
        to: String,           // new value
        level: String,
        #[serde(default)]
        message: Option<String>,
    },
}

/// Serialized alarm event stored in pipeline data metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlarmEvent {
    pub rule_id: String,
    pub rule_type: String,
    pub level: AlarmLevel,
    pub message: String,
    pub tag: String,
    pub trigger_value: f64,
    pub threshold: String,   // human-readable threshold description
    pub timestamp: DateTime<Utc>,
}

/// Alarm state for a specific tag (used by rate-of-change and state-change)
#[derive(Debug, Clone)]
pub enum AlarmStateValue {
    Numeric(f64, i64),       // (value, timestamp_ms)
    Text(String, i64),       // (value, timestamp_ms)
}

pub struct AlarmOperator {
    rules: Vec<(String, AlarmRule)>,  // (rule_id, rule)
    /// Per-tag state for rate-of-change and state-change tracking
    states: Arc<RwLock<HashMap<String, AlarmStateValue>>>,
    /// Alarm level per rule
    levels: HashMap<String, AlarmLevel>,
}

impl AlarmOperator {
    pub fn new(config: &PluginConfig) -> Self {
        let rules_json = config.get("rules")
            .and_then(|v| v.as_array())
            .map(|arr| arr.clone())
            .unwrap_or_default();

        let mut rules = Vec::new();
        let mut levels = HashMap::new();

        for rule_val in rules_json {
            if let Ok(rule) = serde_json::from_value::<AlarmRule>(rule_val.clone()) {
                let id = rule_val.get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                let level_str = rule_val.get("level")
                    .and_then(|v| v.as_str())
                    .unwrap_or("info");
                let level = AlarmLevel::from_str(level_str);
                levels.insert(id.clone(), level);
                rules.push((id, rule));
            }
        }

        Self {
            rules,
            states: Arc::new(RwLock::new(HashMap::new())),
            levels,
        }
    }

    fn evaluate_threshold(data: &PipelineData, rule: &AlarmRule) -> Option<(f64, String)> {
        if let AlarmRule::Threshold { tag, high, low, .. } = rule {
            let value = data.payload.get(tag).and_then(|v| v.as_f64())?;
            let mut triggered = false;
            let mut desc = String::new();

            if let Some(high_val) = high {
                if value > *high_val {
                    triggered = true;
                    desc = format!("{} > {}", value, high_val);
                }
            }
            if let Some(low_val) = low {
                if value < *low_val {
                    triggered = true;
                    if !desc.is_empty() { desc.push_str(", "); }
                    desc.push_str(&format!("{} < {}", value, low_val));
                }
            }

            if triggered {
                Some((value, desc))
            } else {
                None
            }
        } else {
            None
        }
    }

    async fn evaluate_rate_of_change(
        states: &Arc<RwLock<HashMap<String, AlarmStateValue>>>,
        data: &PipelineData,
        rule: &AlarmRule,
    ) -> Option<(f64, String)> {
        if let AlarmRule::RateOfChange { tag, threshold, .. } = rule {
            let value = data.payload.get(tag).and_then(|v| v.as_f64())?;
            let now_ms = data.ts.timestamp_millis();

            let (rate, desc) = {
                let mut states_guard = states.write().await;
                let entry = states_guard.entry(tag.clone()).or_insert_with(|| {
                    AlarmStateValue::Numeric(value, now_ms)
                });

                if let AlarmStateValue::Numeric(prev_value, prev_ts) = entry {
                    let dt = ((now_ms - *prev_ts) as f64) / 1000.0;
                    let rate = if dt > 0.0 { (value - *prev_value).abs() / dt } else { 0.0 };
                    *prev_value = value;
                    *prev_ts = now_ms;
                    (rate, format!("{:.2}/s (limit: {}/s)", rate, threshold))
                } else {
                    *entry = AlarmStateValue::Numeric(value, now_ms);
                    (0.0, format!("0/s (limit: {}/s)", threshold))
                }
            };

            if rate > *threshold {
                Some((value, desc))
            } else {
                None
            }
        } else {
            None
        }
    }

    async fn evaluate_state_change(
        states: &Arc<RwLock<HashMap<String, AlarmStateValue>>>,
        data: &PipelineData,
        rule: &AlarmRule,
    ) -> Option<(f64, String)> {
        if let AlarmRule::StateChange { tag, from, to, .. } = rule {
            let value_opt = data.payload.get(tag).and_then(|v| v.as_string());
            let Some(value) = value_opt else { return None; };
            let now_ms = data.ts.timestamp_millis();

            let triggered = {
                let mut states_guard = states.write().await;
                let entry = states_guard.entry(tag.clone()).or_insert_with(|| {
                    AlarmStateValue::Text(value.clone(), now_ms)
                });

                if let AlarmStateValue::Text(prev_value, _) = entry {
                    let matches_from = from == "*" || *prev_value == *from;
                    let matches_to = *value == *to;
                    *entry = AlarmStateValue::Text(value.clone(), now_ms);
                    matches_from && matches_to
                } else {
                    false
                }
            };

            if triggered {
                Some((0.0, format!("{} → {}", from, to)))
            } else {
                None
            }
        } else {
            None
        }
    }

    fn make_alarm_event(
        rule_id: &str,
        rule: &AlarmRule,
        level: AlarmLevel,
        trigger_value: f64,
        threshold_desc: String,
        data: &PipelineData,
    ) -> AlarmEvent {
        let rule_type = match rule {
            AlarmRule::Threshold { .. } => "threshold",
            AlarmRule::RateOfChange { .. } => "rate_of_change",
            AlarmRule::StateChange { .. } => "state_change",
        };

        let tag = match rule {
            AlarmRule::Threshold { tag, .. } => tag.clone(),
            AlarmRule::RateOfChange { tag, .. } => tag.clone(),
            AlarmRule::StateChange { tag, .. } => tag.clone(),
        };

        let message = match rule {
            AlarmRule::Threshold { message, .. } => message.clone(),
            AlarmRule::RateOfChange { message, .. } => message.clone(),
            AlarmRule::StateChange { message, .. } => message.clone(),
        }.unwrap_or_else(|| format!("[{:?}] {} triggered", level, rule_type));

        AlarmEvent {
            rule_id: rule_id.to_string(),
            rule_type: rule_type.to_string(),
            level,
            message,
            tag,
            trigger_value,
            threshold: threshold_desc,
            timestamp: data.ts,
        }
    }
}

#[async_trait]
impl Operable for AlarmOperator {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "alarm",
            kind: gateway_sdk::PluginKind::Operator,
            description: Some("Alarm rule engine with threshold/rate-of-change/state-change rules"),
            version: "0.1.0",
            name_zh: Some("告警"),
            name_en: Some("Alarm"),
            description_zh: Some("告警规则引擎，支持阈值告警、变化率告警、状态变化告警"),
            description_en: Some("Alarm rule engine with threshold, rate-of-change, and state-change rules"),
        }
    }

    async fn open(&self, _node_id: NodeId, _config: PluginConfig) -> PluginResult<()> {
        Ok(())
    }

    async fn close(&self, _node_id: NodeId) -> PluginResult<()> {
        Ok(())
    }

    async fn process(&self, _node_id: NodeId, data: PipelineData) -> PluginResult<Vec<PipelineData>> {
        let mut triggered_alarms = Vec::new();

        for (rule_id, rule) in &self.rules {
            let level = self.levels.get(rule_id).copied().unwrap_or(AlarmLevel::Info);

            let triggered = match rule {
                AlarmRule::Threshold { .. } => {
                    Self::evaluate_threshold(&data, rule)
                        .map(|(v, desc)| (v, desc))
                }
                AlarmRule::RateOfChange { .. } => {
                    Self::evaluate_rate_of_change(&self.states, &data, rule).await
                }
                AlarmRule::StateChange { .. } => {
                    Self::evaluate_state_change(&self.states, &data, rule).await
                }
            };

            if let Some((trigger_value, threshold_desc)) = triggered {
                let event = Self::make_alarm_event(
                    rule_id, rule, level, trigger_value, threshold_desc, &data
                );
                triggered_alarms.push(event);
            }
        }

        // If any alarm triggered, route to alarm port; otherwise to normal port
        if !triggered_alarms.is_empty() {
            let mut out = data;
            out.metadata.insert("alarm.port".into(), "alarm".to_string());
            // Store first alarm event (could store all as JSON array)
            if let Ok(event_json) = serde_json::to_string(&triggered_alarms[0]) {
                out.metadata.insert("alarm.event".into(), event_json);
            }
            out.metadata.insert("alarm.is_triggered".into(), "true".to_string());
            out.metadata.insert("alarm.level".into(), format!("{:?}", triggered_alarms[0].level));
            out.metadata.insert("alarm.rule_id".into(), triggered_alarms[0].rule_id.clone());
            Ok(vec![out])
        } else {
            let mut out = data;
            out.metadata.insert("alarm.port".into(), "normal".to_string());
            out.metadata.insert("alarm.is_triggered".into(), "false".to_string());
            Ok(vec![out])
        }
    }

    async fn reset(&self, _node_id: NodeId) -> PluginResult<()> {
        let mut states = self.states.write().await;
        states.clear();
        Ok(())
    }
}

/// Create an alarm operator from config.
/// config expects:
/// {
///   "rules": [
///     {
///       "id": "temp_high",
///       "tag": "temperature",
///       "type": "threshold",
///       "high": 80.0,
///       "low": null,
///       "level": "critical"
///     }
///   ]
/// }
pub fn create_alarm_operator(config: &PluginConfig) -> Box<dyn Operable> {
    Box::new(AlarmOperator::new(config))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_alarm_level_from_str() {
        assert_eq!(AlarmLevel::from_str("critical"), AlarmLevel::Critical);
        assert_eq!(AlarmLevel::from_str("HIGH"), AlarmLevel::High);
        assert_eq!(AlarmLevel::from_str("medium"), AlarmLevel::Medium);
        assert_eq!(AlarmLevel::from_str("low"), AlarmLevel::Low);
        assert_eq!(AlarmLevel::from_str("unknown"), AlarmLevel::Info);
    }

    #[tokio::test]
    async fn test_threshold_alarm_high() {
        let config = PluginConfig::new();
        let alarm = AlarmOperator::new(&config);
        
        // This test just verifies the operator can be created
        assert_eq!(alarm.rules.len(), 0);
    }
}
