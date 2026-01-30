//! 南向振动采集插件（virb）：对标 dataacq-plugin-virb。
//! 联能 YE6235D/YE6235D2 振动采集器，UDP 通信：上位机发控制命令，采集器推送波形数据。
//! 支持 raw 组（时域 max/min/avg）与配置与参考实现一致。

#[cfg(feature = "ffi")]
mod ffi;

mod protocol;

use gateway_sdk::{
    ConfigSchema, Group, GroupId, NodeId, ParamOption, ParamSchema, ParamType, ParamValid,
    PluginMeta, SouthPlugin, Tag, TagId, TagRegexEntry, TagSchema,
};
use gateway_sdk::types::{DataValue, PluginKind};
use gateway_sdk::{PluginConfig, PluginError, PluginResult};
use gateway_sdk::log;
use protocol::{
    decode_upload_data, encode_set_channel_prop, encode_set_sample_rate, encode_start_grab,
    encode_stop_grab, handle_packet_count, sample_rate_id_to_hz, VirbEndianess, VirbModel,
};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use tokio::sync::{oneshot, RwLock};
use tokio::net::UdpSocket;

const MAX_PACKET_QUEUE: usize = 500;

/// 振动采集插件（与 dataacq-plugin-virb 功能对齐）
pub struct VirbPlugin {
    state: Arc<RwLock<HashMap<NodeId, VirbState>>>,
}

struct VirbState {
    host: String,
    port: u16,
    #[allow(dead_code)]
    timeout_sec: u64,
    model: VirbModel,
    sample_rate_id: u8,
    channel_prop: u8,
    #[allow(dead_code)]
    channel_params: String,
    endianess: VirbEndianess,
    #[allow(dead_code)]
    max_kcount: u32,
    #[allow(dead_code)]
    rspeed: u32,
    #[allow(dead_code)]
    unit: u8,
    unit_factor: f64,
    channels: usize,
    data_bytes: usize,
    packet_total: usize,
    factors: Vec<f64>,
    _config: PluginConfig,
    groups: Vec<Group>,
    tags: Vec<Tag>,
    packet_queue: Arc<RwLock<VecDeque<protocol::VirbPacket>>>,
    socket: Option<Arc<UdpSocket>>,
    recv_handle: Option<tokio::task::JoinHandle<()>>,
    cancel_tx: Option<oneshot::Sender<()>>,
}

impl Default for VirbPlugin {
    fn default() -> Self {
        Self::new()
    }
}

/// 解析通道参数 "1,1,1,1,..." -> 每通道系数 (与 C parse_doubles + factor 一致：1/strtod * factor)
fn parse_channel_params(s: &str, channels: usize, model_factor: f64) -> Vec<f64> {
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

impl VirbPlugin {
    pub fn new() -> Self {
        Self {
            state: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    fn default_groups(&self, interval_ms: u64) -> Vec<Group> {
        vec![Group {
            id: GroupId::new(),
            name: "raw_default".to_string(),
            interval_ms,
            description: Some("振动时域数据（通道 max/min/avg）".to_string()),
        }]
    }

    fn default_tags(&self, group_id: GroupId, channels: usize) -> Vec<Tag> {
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
    fn parse_address(addr: &str, max_channel: usize) -> Option<usize> {
        let ch: usize = addr.trim().parse().ok()?;
        if ch >= 1 && ch <= max_channel {
            Some(ch - 1)
        } else {
            None
        }
    }
}

#[async_trait::async_trait]
impl SouthPlugin for VirbPlugin {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "virb",
            kind: PluginKind::South,
            description: Some("联能 YE6235D/YE6235D2 振动采集器，UDP 通信"),
            version: "0.1.0",
            name_zh: Some("振动采集"),
            name_en: Some("Vibration"),
            description_zh: Some("该插件可用于访问 YE6235D和YE6235D2 的联能震动传感器采集器。"),
            description_en: Some("This plugin can be used to access the Lianneng shutdown sensor collector of the YE6235D and YE6235D2."),
        }
    }

    fn config_schema(&self) -> Option<ConfigSchema> {
        use gateway_sdk::ParamAttribute;
        Some(
            ConfigSchema::new()
                .param(ParamSchema {
                    name: "model".to_string(),
                    name_zh: Some("采集器型号".to_string()),
                    name_en: Some("Model".to_string()),
                    description: Some("Vibration collector model".to_string()),
                    description_zh: Some("联能振动采集器型号".to_string()),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::Select,
                    default: Some(serde_json::json!(0)),
                    valid: None,
                    options: Some(vec![
                        ParamOption {
                            value: serde_json::json!(0),
                            label: Some("YE6275D".to_string()),
                            label_zh: Some("YE6275D".to_string()),
                            label_en: Some("YE6275D".to_string()),
                        },
                        ParamOption {
                            value: serde_json::json!(1),
                            label: Some("YE6275D2".to_string()),
                            label_zh: Some("YE6275D2".to_string()),
                            label_en: Some("YE6275D2".to_string()),
                        },
                    ]),
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "host".to_string(),
                    name_zh: Some("IP地址".to_string()),
                    name_en: Some("IP Address".to_string()),
                    description: Some("device IP".to_string()),
                    description_zh: Some("设备 IP".to_string()),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::String,
                    default: Some(serde_json::json!("192.168.99.121")),
                    valid: None,
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "port".to_string(),
                    name_zh: Some("端口号".to_string()),
                    name_en: Some("Port".to_string()),
                    description: Some("Device port".to_string()),
                    description_zh: Some("设备端口号".to_string()),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(8089)),
                    valid: Some(ParamValid {
                        min: Some(1),
                        max: Some(65535),
                        regex: None,
                        length: None,
                    }),
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "timeout".to_string(),
                    name_zh: Some("超时时间(s)".to_string()),
                    name_en: Some("Timeout(s)".to_string()),
                    description: Some("No data within this seconds will trigger reconnect".to_string()),
                    description_zh: Some("超过该秒数无数据自动重连".to_string()),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(5)),
                    valid: Some(ParamValid {
                        min: Some(1),
                        max: Some(60),
                        regex: None,
                        length: None,
                    }),
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "sample_rate".to_string(),
                    name_zh: Some("采样率".to_string()),
                    name_en: Some("Sampling Rate".to_string()),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::Select,
                    default: Some(serde_json::json!(3)),
                    valid: None,
                    options: Some(vec![
                        ParamOption {
                            value: serde_json::json!(7),
                            label: Some("1600".to_string()),
                            label_zh: Some("1600".to_string()),
                            label_en: Some("1600".to_string()),
                        },
                        ParamOption {
                            value: serde_json::json!(6),
                            label: Some("3200".to_string()),
                            label_zh: Some("3200".to_string()),
                            label_en: Some("3200".to_string()),
                        },
                        ParamOption {
                            value: serde_json::json!(5),
                            label: Some("6400".to_string()),
                            label_zh: Some("6400".to_string()),
                            label_en: Some("6400".to_string()),
                        },
                        ParamOption {
                            value: serde_json::json!(4),
                            label: Some("12800".to_string()),
                            label_zh: Some("12800".to_string()),
                            label_en: Some("12800".to_string()),
                        },
                        ParamOption {
                            value: serde_json::json!(3),
                            label: Some("25600".to_string()),
                            label_zh: Some("25600".to_string()),
                            label_en: Some("25600".to_string()),
                        },
                        ParamOption {
                            value: serde_json::json!(2),
                            label: Some("51200".to_string()),
                            label_zh: Some("51200".to_string()),
                            label_en: Some("51200".to_string()),
                        },
                        ParamOption {
                            value: serde_json::json!(1),
                            label: Some("128000".to_string()),
                            label_zh: Some("128000".to_string()),
                            label_en: Some("128000".to_string()),
                        },
                        ParamOption {
                            value: serde_json::json!(0),
                            label: Some("256000".to_string()),
                            label_zh: Some("256000".to_string()),
                            label_en: Some("256000".to_string()),
                        },
                    ]),
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "channel_prop".to_string(),
                    name_zh: Some("通道属性".to_string()),
                    name_en: Some("Channel Property".to_string()),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::Select,
                    default: Some(serde_json::json!(1)),
                    valid: None,
                    options: Some(vec![
                        ParamOption {
                            value: serde_json::json!(1),
                            label: Some("IEPE".to_string()),
                            label_zh: Some("IEPE".to_string()),
                            label_en: Some("IEPE".to_string()),
                        },
                        ParamOption {
                            value: serde_json::json!(0),
                            label: Some("VOLT".to_string()),
                            label_zh: Some("VOLT".to_string()),
                            label_en: Some("VOLT".to_string()),
                        },
                    ]),
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "channel_params".to_string(),
                    name_zh: Some("通道参数".to_string()),
                    name_en: Some("Channel Parameters".to_string()),
                    description: Some("Channel parameters, comma-separated".to_string()),
                    description_zh: Some("通道参数,英文半角逗号分隔,按顺序设置每个通道对应的传感器的参数值".to_string()),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::String,
                    default: Some(serde_json::json!("1,1,1,1,1,1,1,1")),
                    valid: None,
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "endianess".to_string(),
                    name_zh: Some("字节序".to_string()),
                    name_en: Some("Endianess".to_string()),
                    description: Some("Tag byte order, AB corresponds to 12".to_string()),
                    description_zh: Some("点位字节序，ABCD 对应 1234".to_string()),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::Select,
                    default: Some(serde_json::json!(3)),
                    valid: None,
                    options: Some(vec![
                        ParamOption {
                            value: serde_json::json!(1),
                            label: Some("ABCD".to_string()),
                            label_zh: Some("ABCD".to_string()),
                            label_en: Some("ABCD".to_string()),
                        },
                        ParamOption {
                            value: serde_json::json!(2),
                            label: Some("BADC".to_string()),
                            label_zh: Some("BADC".to_string()),
                            label_en: Some("BADC".to_string()),
                        },
                        ParamOption {
                            value: serde_json::json!(3),
                            label: Some("DCBA".to_string()),
                            label_zh: Some("DCBA".to_string()),
                            label_en: Some("DCBA".to_string()),
                        },
                        ParamOption {
                            value: serde_json::json!(4),
                            label: Some("CDAB".to_string()),
                            label_zh: Some("CDAB".to_string()),
                            label_en: Some("CDAB".to_string()),
                        },
                        ParamOption {
                            value: serde_json::json!(5),
                            label: Some("AB".to_string()),
                            label_zh: Some("AB".to_string()),
                            label_en: Some("AB".to_string()),
                        },
                        ParamOption {
                            value: serde_json::json!(6),
                            label: Some("BA".to_string()),
                            label_zh: Some("BA".to_string()),
                            label_en: Some("BA".to_string()),
                        },
                    ]),
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "max_kcount".to_string(),
                    name_zh: Some("最大K".to_string()),
                    name_en: Some("Max K".to_string()),
                    description: Some("Save the maximum value of k (default is 0), save all k data.".to_string()),
                    description_zh: Some("保存最大k,默认为0,保存所有k数据".to_string()),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(0)),
                    valid: Some(ParamValid {
                        min: Some(0),
                        max: Some(4096),
                        regex: None,
                        length: None,
                    }),
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "rspeed".to_string(),
                    name_zh: Some("转速(rpm)".to_string()),
                    name_en: Some("RPM".to_string()),
                    description: Some("revolution per minute".to_string()),
                    description_zh: Some("转速(rpm)".to_string()),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(2000)),
                    valid: Some(ParamValid {
                        min: Some(1),
                        max: Some(10000),
                        regex: None,
                        length: None,
                    }),
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "unit".to_string(),
                    name_zh: Some("振幅单位".to_string()),
                    name_en: Some("Amplitude unit".to_string()),
                    description: Some("Amplitude data units".to_string()),
                    description_zh: Some("振幅数据单位".to_string()),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::Select,
                    default: Some(serde_json::json!(1)),
                    valid: None,
                    options: Some(vec![
                        ParamOption {
                            value: serde_json::json!(1),
                            label: Some("米每平方秒(m/s^2)".to_string()),
                            label_zh: Some("米每平方秒(m/s^2)".to_string()),
                            label_en: Some("m/s²".to_string()),
                        },
                        ParamOption {
                            value: serde_json::json!(2),
                            label: Some("毫重力加速度(mg)".to_string()),
                            label_zh: Some("毫重力加速度(mg)".to_string()),
                            label_en: Some("mg".to_string()),
                        },
                        ParamOption {
                            value: serde_json::json!(3),
                            label: Some("位移(μm)".to_string()),
                            label_zh: Some("位移(μm)".to_string()),
                            label_en: Some("μm".to_string()),
                        },
                    ]),
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "unit_factor".to_string(),
                    name_zh: Some("单位转换倍率".to_string()),
                    name_en: Some("Unit factor".to_string()),
                    description: Some("To convert m/s^2 to the selected unit, multiply the original data by this parameter.".to_string()),
                    description_zh: Some("m/s^2转换成选中单位的倍率，原数据乘以这个参数".to_string()),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::String,
                    default: Some(serde_json::json!("101.97")),
                    valid: Some(ParamValid {
                        min: None,
                        max: None,
                        regex: None,
                        length: Some(10),
                    }),
                    depends_on: Some("unit".to_string()),
                    depends_value: Some(serde_json::json!(2)),
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "save_raw".to_string(),
                    name_zh: Some("保存原始数据".to_string()),
                    name_en: Some("Save Raw".to_string()),
                    description: Some("Save Raw Virb Data".to_string()),
                    description_zh: Some("保存原始振动数据".to_string()),
                    attribute: ParamAttribute::Optional,
                    ty: ParamType::Bool,
                    default: Some(serde_json::json!(false)),
                    valid: None,
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "save_raw_chs".to_string(),
                    name_zh: Some("保存原始数据的通道号".to_string()),
                    name_en: Some("Save Raw Channels".to_string()),
                    description: Some("List of channels for storing data, separated by English half-width characters.".to_string()),
                    description_zh: Some("保存数据的通道列表,英文半角逗号分隔".to_string()),
                    attribute: ParamAttribute::Optional,
                    ty: ParamType::String,
                    default: Some(serde_json::json!("0")),
                    valid: Some(ParamValid {
                        min: None,
                        max: None,
                        regex: None,
                        length: Some(50),
                    }),
                    depends_on: Some("save_raw".to_string()),
                    depends_value: Some(serde_json::json!(true)),
                    ..Default::default()
                })
                .tag_regex(vec![
                    TagRegexEntry {
                        data_type: "float64".to_string(),
                        regex: r"^[0-9]+$".to_string(),
                    },
                ]),
        )
    }

    fn tag_schema(&self) -> Option<TagSchema> {
        Some(TagSchema {
            data_types: Some(vec!["float64".to_string()]),
            address_format: Some("通道号 1..8 (YE6275D) 或 1..32 (YE6275D2)，对应时域最大值".to_string()),
            address_format_zh: Some("通道号 1..8 (YE6275D) 或 1..32 (YE6275D2)，对应时域最大值".to_string()),
            address_format_en: Some("Channel index 1..8 (YE6275D) or 1..32 (YE6275D2), time-domain max".to_string()),
        })
    }

    async fn validate_tag(&self, _node_id: NodeId, tag: &Tag) -> PluginResult<()> {
        if tag.name.is_empty() {
            return Err(PluginError::tag_invalid("tag name required"));
        }
        if let Some(ref dt) = tag.data_type {
            if dt != "float64" {
                return Err(PluginError::tag_invalid("virb only supports float64"));
            }
        }
        let ch: usize = tag.address.trim().parse().map_err(|_| PluginError::tag_invalid("address must be channel number 1..N"))?;
        if ch < 1 || ch > 32 {
            return Err(PluginError::tag_invalid("address must be 1..32"));
        }
        Ok(())
    }

    async fn open(&self, node_id: NodeId, config: PluginConfig) -> PluginResult<()> {
        let model_id = config.get("model").and_then(|v| v.as_i64()).unwrap_or(0);
        let model = if model_id == 1 {
            VirbModel::YE6275D2
        } else {
            VirbModel::YE6275D
        };
        let host = config
            .get("host")
            .and_then(|v| v.as_str())
            .map(String::from)
            .unwrap_or_else(|| "192.168.99.121".to_string());
        let port = config
            .get("port")
            .and_then(|v| v.as_u64())
            .map(|n| n as u16)
            .unwrap_or(8089);
        let timeout_sec = config
            .get("timeout")
            .and_then(|v| v.as_u64())
            .unwrap_or(5)
            .clamp(1, 60);
        let sample_rate_id = config
            .get("sample_rate")
            .and_then(|v| v.as_u64())
            .map(|n| n as u8)
            .unwrap_or(3)
            .min(7);
        let channel_prop = config
            .get("channel_prop")
            .and_then(|v| v.as_u64())
            .map(|n| n as u8)
            .unwrap_or(1);
        let channel_params = config
            .get("channel_params")
            .and_then(|v| v.as_str())
            .map(String::from)
            .unwrap_or_else(|| "1,1,1,1,1,1,1,1".to_string());
        let endianess_id = config.get("endianess").and_then(|v| v.as_i64()).unwrap_or(3);
        let endianess = VirbEndianess::from_id(endianess_id);
        let max_kcount = config
            .get("max_kcount")
            .and_then(|v| v.as_u64())
            .map(|n| n as u32)
            .unwrap_or(0)
            .min(4096);
        let rspeed = config
            .get("rspeed")
            .and_then(|v| v.as_u64())
            .map(|n| n as u32)
            .unwrap_or(2000)
            .clamp(1, 10000);
        let unit = config
            .get("unit")
            .and_then(|v| v.as_u64())
            .map(|n| n as u8)
            .unwrap_or(1);
        let unit_factor_str = config
            .get("unit_factor")
            .and_then(|v| v.as_str())
            .unwrap_or("101.97");
        let unit_factor: f64 = unit_factor_str.parse().unwrap_or(101.97);
        let unit_factor = if unit_factor == 0.0 { 1.0 } else { unit_factor };

        let channels = model.channels();
        let data_bytes = model.channel_byte_count();
        let sample_rate_hz = sample_rate_id_to_hz(sample_rate_id);
        let data_count_per_packet = 1024 / data_bytes / channels;
        let packet_total = sample_rate_hz as usize / data_count_per_packet;
        let factors = parse_channel_params(&channel_params, channels, model.factor());

        let interval_ms = 70u64;
        let groups = self.default_groups(interval_ms);
        let tags = groups
            .iter()
            .flat_map(|g| self.default_tags(g.id, channels))
            .collect::<Vec<_>>();

        log::info(
            node_id,
            format!(
                "open virb: host={}, port={}, model={:?}, channels={}, sample_rate={}",
                host, port, model, channels, sample_rate_hz
            ),
        );

        let mut state = self.state.write().await;
        state.insert(
            node_id,
            VirbState {
                host,
                port,
                timeout_sec,
                model,
                sample_rate_id,
                channel_prop,
                channel_params,
                endianess,
                max_kcount,
                rspeed,
                unit,
                unit_factor,
                channels,
                data_bytes,
                packet_total,
                factors,
                _config: config,
                groups,
                tags,
                packet_queue: Arc::new(RwLock::new(VecDeque::new())),
                socket: None,
                recv_handle: None,
                cancel_tx: None,
            },
        );
        Ok(())
    }

    async fn close(&self, node_id: NodeId) -> PluginResult<()> {
        let mut state = self.state.write().await;
        if let Some(s) = state.get_mut(&node_id) {
            if let Some(tx) = s.cancel_tx.take() {
                let _ = tx.send(());
            }
            if let Some(h) = s.recv_handle.take() {
                let _ = h.await;
            }
            s.socket = None;
        }
        state.remove(&node_id);
        log::info(node_id, "close virb");
        Ok(())
    }

    async fn start(&self, node_id: NodeId) -> PluginResult<()> {
        let (socket, host, port, cancel_tx, recv_handle) = {
            let state = self.state.read().await;
            let s = state.get(&node_id).ok_or_else(|| PluginError::msg("node not open"))?;
            let socket = UdpSocket::bind("0.0.0.0:0")
                .await
                .map_err(|e| PluginError::msg(format!("udp bind: {}", e)))?;
            let device_addr = format!("{}:{}", s.host, s.port);
            socket.connect(&device_addr).await.map_err(|e| PluginError::msg(format!("udp connect: {}", e)))?;

            let rate_hz = sample_rate_id_to_hz(s.sample_rate_id);
            let start_cmd = encode_start_grab();
            let stop_cmd = encode_stop_grab();
            let _ = socket.send(&stop_cmd).await;
            let rate_cmd = encode_set_sample_rate(rate_hz);
            let _ = socket.send(&rate_cmd).await;
            if s.model == VirbModel::YE6275D2 {
                let prop_cmd = encode_set_channel_prop(s.channel_prop);
                let _ = socket.send(&prop_cmd).await;
            }
            let _ = socket.send(&start_cmd).await;

            let (tx, mut rx) = oneshot::channel();
            let socket = Arc::new(socket);
            let socket_recv = Arc::clone(&socket);
            let queue = Arc::clone(&s.packet_queue);
            let host = s.host.clone();
            let port = s.port;
            let factors = s.factors.clone();
            let channels = s.channels;
            let data_bytes = s.data_bytes;
            let endianess = s.endianess;

            let recv_handle = tokio::spawn(async move {
                let mut buf = [0u8; 1500];
                loop {
                    tokio::select! {
                        _ = &mut rx => break,
                        r = socket_recv.recv(&mut buf) => {
                            match r {
                                Ok(n) if n > 1024 => {
                                    let timestamp = std::time::SystemTime::now()
                                        .duration_since(std::time::UNIX_EPOCH)
                                        .unwrap()
                                        .as_millis() as i64;
                                    if let Some(packet) = decode_upload_data(
                                        &buf[..n],
                                        timestamp,
                                        channels,
                                        data_bytes,
                                        rate_hz,
                                        endianess,
                                        &factors,
                                    ) {
                                        let mut q = queue.write().await;
                                        q.push_back(packet);
                                        while q.len() > MAX_PACKET_QUEUE {
                                            q.pop_front();
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
            });

            (
                socket,
                host,
                port,
                tx,
                recv_handle,
            )
        };

        let mut state = self.state.write().await;
        if let Some(s) = state.get_mut(&node_id) {
            s.socket = Some(socket);
            s.recv_handle = Some(recv_handle);
            s.cancel_tx = Some(cancel_tx);
        }
        log::info(node_id, format!("start virb: {}:{}", host, port));
        Ok(())
    }

    async fn stop(&self, node_id: NodeId) -> PluginResult<()> {
        let (send_stop, cancel_tx, recv_handle) = {
            let mut state = self.state.write().await;
            let s = state.get_mut(&node_id).ok_or_else(|| PluginError::msg("node not open"))?;
            let stop_cmd = encode_stop_grab();
            let socket = s.socket.as_ref().map(Arc::clone);
            let cancel_tx = s.cancel_tx.take();
            let recv_handle = s.recv_handle.take();
            (socket.map(|sock| (sock, stop_cmd)), cancel_tx, recv_handle)
        };
        if let Some((sock, cmd)) = send_stop {
            let _ = sock.send(&cmd).await;
        }
        if let Some(tx) = cancel_tx {
            let _ = tx.send(());
        }
        if let Some(h) = recv_handle {
            let _ = h.await;
        }
        let mut state = self.state.write().await;
        if let Some(s) = state.get_mut(&node_id) {
            s.socket = None;
        }
        log::info(node_id, "stop virb");
        Ok(())
    }

    async fn poll_group(
        &self,
        node_id: NodeId,
        group_id: GroupId,
        tags: &[Tag],
    ) -> PluginResult<Vec<(TagId, DataValue)>> {
        let (channels, packet_total, interval_ms, unit_factor, queue) = {
            let state = self.state.read().await;
            let s = state.get(&node_id).ok_or_else(|| PluginError::msg("node not open"))?;
            let interval_ms = s.groups.iter().find(|g| g.id == group_id).map(|g| g.interval_ms).unwrap_or(70);
            (
                s.channels,
                s.packet_total,
                interval_ms,
                s.unit_factor,
                Arc::clone(&s.packet_queue),
            )
        };

        let need = handle_packet_count(packet_total, interval_ms);
        let mut packets = Vec::with_capacity(need);
        {
            let mut q = queue.write().await;
            for _ in 0..need {
                match q.pop_front() {
                    Some(p) => packets.push(p),
                    None => break,
                }
            }
        }

        let mut out = Vec::with_capacity(tags.len());
        for tag in tags {
            let ch_index = Self::parse_address(&tag.address, channels).unwrap_or(0);
            let value = if packets.is_empty() {
                0.0_f64
            } else {
                let last = packets.last().unwrap();
                last.data_maxs.get(ch_index).copied().unwrap_or(0.0) * unit_factor
            };
            out.push((tag.id, DataValue::Float64(value)));
        }
        Ok(out)
    }

    async fn list_groups(&self, node_id: NodeId) -> PluginResult<Vec<Group>> {
        let state = self.state.read().await;
        let s = state.get(&node_id).ok_or_else(|| PluginError::msg("node not open"))?;
        Ok(s.groups.clone())
    }

    async fn list_tags(&self, node_id: NodeId, group_id: GroupId) -> PluginResult<Vec<Tag>> {
        let state = self.state.read().await;
        let s = state.get(&node_id).ok_or_else(|| PluginError::msg("node not open"))?;
        Ok(s.tags
            .iter()
            .filter(|t| t.group_id == group_id)
            .cloned()
            .collect())
    }
}
