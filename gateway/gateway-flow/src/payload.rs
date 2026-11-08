//! 流程内载荷：数据帧或告警事件（design §4.1 双端口语义）。

use chrono::{DateTime, Utc};
use gateway_sdk::GroupData;
use serde::{Deserialize, Serialize};

/// 算子间流动的载荷。alarm 端口携带 `Alarm`，normal/true/false 端口携带 `Data`。
#[derive(Debug, Clone)]
pub enum FlowPayload {
    Data(GroupData),
    Alarm(AlarmEvent),
}

/// 告警等级（redesign `AlarmLevel` 沿用）。
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
    pub fn parse(s: &str) -> Result<Self, String> {
        match s.to_lowercase().as_str() {
            "critical" => Ok(AlarmLevel::Critical),
            "high" => Ok(AlarmLevel::High),
            "medium" => Ok(AlarmLevel::Medium),
            "low" => Ok(AlarmLevel::Low),
            "info" => Ok(AlarmLevel::Info),
            other => Err(format!("unknown alarm level `{other}`")),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            AlarmLevel::Critical => "critical",
            AlarmLevel::High => "high",
            AlarmLevel::Medium => "medium",
            AlarmLevel::Low => "low",
            AlarmLevel::Info => "info",
        }
    }
}

/// 告警事件（入库与去重在 server 侧 alarm store 完成：同键 active 只前移 last_ts）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlarmEvent {
    pub level: AlarmLevel,
    pub tag: String,
    pub value: Option<f64>,
    pub threshold: Option<f64>,
    pub message: String,
    pub ts: DateTime<Utc>,
}
