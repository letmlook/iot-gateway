//! 振动采集插件状态与默认组/标签。

use gateway_sdk::{Group, GroupId, Tag};
use gateway_sdk::{PluginConfig, TagId};

use crate::protocol::{VirbEndianess, VirbModel};

/// 振动采集节点运行时状态
pub struct VirbState {
    pub host: String,
    pub port: u16,
    #[allow(dead_code)]
    pub timeout_sec: u64,
    pub model: VirbModel,
    pub sample_rate_id: u8,
    pub channel_prop: u8,
    #[allow(dead_code)]
    pub channel_params: String,
    pub endianess: VirbEndianess,
    #[allow(dead_code)]
    pub max_kcount: u32,
    #[allow(dead_code)]
    pub rspeed: u32,
    #[allow(dead_code)]
    pub unit: u8,
    pub unit_factor: f64,
    pub channels: usize,
    pub data_bytes: usize,
    pub packet_total: usize,
    pub factors: Vec<f64>,
    #[allow(dead_code)]
    pub _config: PluginConfig,
    pub groups: Vec<Group>,
    pub tags: Vec<Tag>,
    pub packet_queue: std::sync::Arc<tokio::sync::RwLock<std::collections::VecDeque<crate::protocol::VirbPacket>>>,
    pub socket: Option<std::sync::Arc<tokio::net::UdpSocket>>,
    pub recv_handle: Option<tokio::task::JoinHandle<()>>,
    pub cancel_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

/// 解析通道参数 "1,1,1,1,..." -> 每通道系数 (与 C parse_doubles + factor 一致：1/strtod * factor)
pub fn parse_channel_params(s: &str, channels: usize, model_factor: f64) -> Vec<f64> {
    let mut out = Vec::with_capacity(channels);
    for part in s.split(',').take(channels) {
        let part = part.trim();
        let v: f64 = part.parse().unwrap_or(1.0);
        let factor = if v > 0.0 { (1.0 / v) * model_factor } else { model_factor };
        out.push(factor);
    }
    while out.len() < channels {
        out.push(model_factor);
    }
    out
}

pub fn default_groups(interval_ms: u64) -> Vec<Group> {
    vec![Group {
        id: GroupId::new(),
        name: "raw_default".to_string(),
        interval_ms,
        description: Some("振动时域数据（通道 max/min/avg）".to_string()),
    }]
}

pub fn default_tags(group_id: GroupId, channels: usize) -> Vec<Tag> {
    use gateway_sdk::TagAttr;
    (1..=channels)
        .map(|ch| Tag {
            id: TagId::new(),
            name: format!("ch{}", ch),
            address: ch.to_string(),
            attr: TagAttr::Read,
            data_type: Some("float64".to_string()),
            description: Some(format!("通道 {} 时域最大值", ch)),
            group_id,
        })
        .collect()
}

/// 地址为通道号 1-based（与 virb_point.c char_to_int(tag->address)-1 一致）
pub fn parse_address(addr: &str, max_channel: usize) -> Option<usize> {
    let ch: usize = addr.trim().parse().ok()?;
    if ch >= 1 && ch <= max_channel {
        Some(ch - 1)
    } else {
        None
    }
}
