//! 南向 BACnet/IP 插件（ASHRAE 135）：自研协议栈，零第三方 BACnet 依赖。
//!
//! 范围（见 docs/design/南向驱动.md §3.3）：
//! - Who-Is / I-Am 设备验证（可关）
//! - ReadProperty 读取 AI/AO/AV/BI/BO/BV/MSI/MSO/MSV/BSV/CSV/IV/LAV/OSV/PIV 的属性
//! - WriteProperty 写 present-value（Bool→二进制对象、数值→模拟对象、整数→多状态对象）
//! - UDP 无连接：不维护长连接，每请求独立超时
//!
//! 地址语法见 [`address`]；FFI/静态两种形态行为一致（std socket + spawn_blocking）。

#[cfg(feature = "ffi")]
mod ffi;

mod address;
mod config;
mod protocol;
mod state;
mod value;

use crate::address::parse_address_full;
use crate::config::{config_bool, config_str, config_u16, config_u32};
use crate::protocol::{
    bvlc_payload, bvlc_wrap, decode_error, decode_i_am, decode_read_property_ack, encode_who_is,
    encode_write_property, npdu_payload, npdu_wrap, APDU_SIMPLE_ACK, APDU_UNCONFIRMED_REQ,
};
use crate::state::{BacnetNodeState, TIMEOUT_MAX_MS, TIMEOUT_MIN_MS};
use crate::value::BacnetValue;
use gateway_sdk::log;
use gateway_sdk::types::{DataValue, PluginKind};
use gateway_sdk::{
    ConfigSchema, Group, GroupId, NodeId, ParamAttribute, ParamSchema, ParamType, ParamValid,
    PluginMeta, SouthPlugin, Tag, TagId, TagSchema,
};
use gateway_sdk::{PluginConfig, PluginError, PluginResult};
use std::collections::HashMap;
use std::net::UdpSocket;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

/// 属性名 → 属性标识符（本插件支持的子集）
fn property_id(name: &str) -> Option<u8> {
    match name {
        "present-value" => Some(protocol::PROP_PRESENT_VALUE),
        "status-flags" => Some(protocol::PROP_STATUS_FLAGS),
        "units" => Some(protocol::PROP_UNITS),
        "object-name" => Some(protocol::PROP_OBJECT_NAME),
        "out-of-service" => Some(protocol::PROP_OUT_OF_SERVICE),
        _ => None,
    }
}

/// BACnet/IP 南向插件
pub struct BacnetPlugin {
    state: Arc<RwLock<HashMap<NodeId, BacnetNodeState>>>,
}

impl Default for BacnetPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl BacnetPlugin {
    pub fn new() -> Self {
        Self {
            state: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

/// 一次 UDP 往返：发请求帧，收包直到 invoke id 匹配（或无需匹配）。
/// 阻塞实现，经由 spawn_blocking 调用；超时/网络错误统一为字符串。
fn udp_roundtrip(
    socket: &UdpSocket,
    remote: std::net::SocketAddr,
    frame: &[u8],
    want_invoke: Option<u8>,
    timeout: Duration,
) -> Result<Vec<u8>, String> {
    socket
        .send_to(frame, remote)
        .map_err(|e| format!("bacnet send: {e}"))?;
    let deadline = Instant::now() + timeout;
    let mut buf = [0u8; 2048];
    loop {
        let now = Instant::now();
        if now >= deadline {
            return Err("bacnet timeout".into());
        }
        socket
            .set_read_timeout(Some(deadline - now))
            .map_err(|e| format!("bacnet set_read_timeout: {e}"))?;
        let n = match socket.recv_from(&mut buf) {
            Ok((n, _)) => n,
            Err(e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                return Err("bacnet timeout".into())
            }
            Err(e) => return Err(format!("bacnet recv: {e}")),
        };
        let Some(npdu) = bvlc_payload(&buf[..n]) else {
            continue;
        };
        let Some(apdu) = npdu_payload(npdu) else {
            continue;
        };
        if apdu.is_empty() {
            continue;
        }
        // I-Am 等 Unconfirmed 报文无 invoke id，直接交出
        if apdu[0] == APDU_UNCONFIRMED_REQ {
            return Ok(apdu.to_vec());
        }
        // 有 invoke 的响应：匹配则交出，迟到的旧响应丢弃
        if apdu.len() >= 2 {
            match want_invoke {
                Some(id) if apdu[1] == id => return Ok(apdu.to_vec()),
                Some(_) => continue,
                None => return Ok(apdu.to_vec()),
            }
        }
    }
}

async fn roundtrip(
    socket: &Arc<UdpSocket>,
    remote: std::net::SocketAddr,
    frame: &[u8],
    want_invoke: Option<u8>,
    timeout: Duration,
) -> Result<Vec<u8>, PluginError> {
    let socket = socket.clone();
    let frame = frame.to_vec();
    tokio::task::spawn_blocking(move || {
        udp_roundtrip(&socket, remote, &frame, want_invoke, timeout)
    })
    .await
    .map_err(|e| PluginError::msg(format!("bacnet task join: {e}")))?
    .map_err(PluginError::msg)
}

/// 读单个属性并解码为 BACnet 值
async fn read_property(
    s: &BacnetNodeState,
    obj_type: u16,
    instance: u32,
    property: u8,
) -> Result<BacnetValue, PluginError> {
    let invoke = s.next_invoke();
    let frame = bvlc_wrap(
        &npdu_wrap(
            &protocol::encode_read_property(invoke, obj_type, instance, property),
            true,
        ),
        false,
    );
    let apdu = roundtrip(&s.socket, s.remote, &frame, Some(invoke), s.timeout()).await?;
    if let Some(err) = decode_error(&apdu) {
        return Err(PluginError::msg(err));
    }
    match decode_read_property_ack(&apdu) {
        Some(Ok(ack)) => Ok(ack.value),
        Some(Err(msg)) => Err(PluginError::msg(format!("bacnet ack: {msg}"))),
        None => Err(PluginError::msg("bacnet: unexpected response PDU")),
    }
}

/// 写单个属性（present-value）
async fn write_property(
    s: &BacnetNodeState,
    obj_type: u16,
    instance: u32,
    value: BacnetValue,
) -> Result<(), PluginError> {
    let invoke = s.next_invoke();
    let apdu = encode_write_property(
        invoke,
        obj_type,
        instance,
        protocol::PROP_PRESENT_VALUE,
        &value,
        s.write_priority,
    );
    let frame = bvlc_wrap(&npdu_wrap(&apdu, true), false);
    let reply = roundtrip(&s.socket, s.remote, &frame, Some(invoke), s.timeout()).await?;
    if let Some(err) = decode_error(&reply) {
        return Err(PluginError::msg(err));
    }
    if reply.first() == Some(&APDU_SIMPLE_ACK) {
        return Ok(());
    }
    Err(PluginError::msg("bacnet: write got unexpected response"))
}

#[async_trait::async_trait]
impl SouthPlugin for BacnetPlugin {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "bacnet",
            kind: PluginKind::South,
            description: Some("BACnet/IP south driver (self-contained protocol stack)"),
            version: "0.1.0",
            name_zh: Some("BACnet/IP"),
            name_en: Some("BACnet/IP"),
            description_zh: Some("BACnet/IP 南向驱动：Who-Is/I-Am、ReadProperty/WriteProperty"),
            description_en: Some("BACnet/IP south driver: Who-Is/I-Am, ReadProperty/WriteProperty"),
        }
    }

    fn config_schema(&self) -> Option<ConfigSchema> {
        Some(
            ConfigSchema::new()
                .param(ParamSchema {
                    name: "host".to_string(),
                    name_zh: Some("设备 IP".to_string()),
                    name_en: Some("Device IP".to_string()),
                    description: Some("BACnet device IP address".to_string()),
                    description_zh: Some("BACnet 设备 IP 地址".to_string()),
                    description_en: Some("BACnet device IP address".to_string()),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::String,
                    default: Some(serde_json::json!("127.0.0.1")),
                    valid: None,
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "port".to_string(),
                    name_zh: Some("端口".to_string()),
                    name_en: Some("Port".to_string()),
                    description: Some("BACnet/IP UDP port (default 47808)".to_string()),
                    description_zh: Some("BACnet/IP UDP 端口（默认 47808）".to_string()),
                    description_en: Some("BACnet/IP UDP port (default 47808)".to_string()),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(47808)),
                    valid: Some(ParamValid {
                        min: Some(1),
                        max: Some(65535),
                        regex: None,
                        length: None,
                    }),
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "device_instance".to_string(),
                    name_zh: Some("设备实例号".to_string()),
                    name_en: Some("Device Instance".to_string()),
                    description: Some(
                        "Device object instance (0 = accept the first I-Am responder)".to_string(),
                    ),
                    description_zh: Some(
                        "设备对象实例号（0 = 接受首个响应 I-Am 的设备）".to_string(),
                    ),
                    description_en: Some(
                        "Device object instance (0 = accept the first I-Am responder)".to_string(),
                    ),
                    attribute: ParamAttribute::Optional,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(0)),
                    valid: Some(ParamValid {
                        min: Some(0),
                        max: Some(4_194_303),
                        regex: None,
                        length: None,
                    }),
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "timeout_ms".to_string(),
                    name_zh: Some("请求超时 (ms)".to_string()),
                    name_en: Some("Request Timeout (ms)".to_string()),
                    description: Some("Per-request timeout in milliseconds".to_string()),
                    description_zh: Some("单次请求超时时间（毫秒）".to_string()),
                    description_en: Some("Per-request timeout in milliseconds".to_string()),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(1000)),
                    valid: Some(ParamValid {
                        min: Some(TIMEOUT_MIN_MS as i64),
                        max: Some(TIMEOUT_MAX_MS as i64),
                        regex: None,
                        length: None,
                    }),
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "write_priority".to_string(),
                    name_zh: Some("写优先级".to_string()),
                    name_en: Some("Write Priority".to_string()),
                    description: Some("BACnet write priority 1(highest)-16(lowest)".to_string()),
                    description_zh: Some("BACnet 写优先级 1（最高）–16（最低）".to_string()),
                    description_en: Some("BACnet write priority 1(highest)-16(lowest)".to_string()),
                    attribute: ParamAttribute::Optional,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(16)),
                    valid: Some(ParamValid {
                        min: Some(1),
                        max: Some(16),
                        regex: None,
                        length: None,
                    }),
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "verify_on_open".to_string(),
                    name_zh: Some("打开时验证设备".to_string()),
                    name_en: Some("Verify Device On Open".to_string()),
                    description: Some(
                        "Send Who-Is on open and check the I-Am device instance".to_string(),
                    ),
                    description_zh: Some("打开节点时发 Who-Is 验证设备实例号".to_string()),
                    description_en: Some(
                        "Send Who-Is on open and check the I-Am device instance".to_string(),
                    ),
                    attribute: ParamAttribute::Optional,
                    ty: ParamType::Bool,
                    default: Some(serde_json::json!(true)),
                    valid: None,
                    ..Default::default()
                }),
        )
    }

    fn tag_schema(&self) -> Option<TagSchema> {
        Some(TagSchema {
            data_types: Some(vec![
                "bool".to_string(),
                "int32".to_string(),
                "int64".to_string(),
                "uint64".to_string(),
                "float64".to_string(),
                "string".to_string(),
                "bytes".to_string(),
            ]),
            address_format: Some("object-type:instance[:property]".to_string()),
            address_format_zh: Some(
                "对象类型:实例号[:属性]，如 ai:1 / bo:3 / msv:2:present-value".to_string(),
            ),
            address_format_en: Some(
                "object-type:instance[:property], e.g. ai:1 / bo:3 / msv:2:present-value"
                    .to_string(),
            ),
        })
    }

    async fn open(&self, node_id: NodeId, config: PluginConfig) -> PluginResult<()> {
        let host = config_str(&config, "host", "127.0.0.1");
        let port = config_u16(&config, "port", 47808);
        let device_cfg_num = config_u32(&config, "device_instance", 0);
        let device_cfg = (device_cfg_num > 0).then_some(device_cfg_num);
        let timeout_ms =
            config_u32(&config, "timeout_ms", 1000).clamp(TIMEOUT_MIN_MS, TIMEOUT_MAX_MS);
        let priority = config_u32(&config, "write_priority", 16).clamp(1, 16);
        let verify = config_bool(&config, "verify_on_open", true);

        let remote: std::net::SocketAddr = format!("{host}:{port}").parse().map_err(|_| {
            PluginError::config_invalid(format!("invalid host:port: {host}:{port}"))
        })?;
        let socket = UdpSocket::bind("0.0.0.0:0")
            .map_err(|e| PluginError::connection_failed(format!("bacnet bind: {e}")))?;
        socket
            .set_broadcast(true)
            .map_err(|e| PluginError::connection_failed(format!("bacnet set_broadcast: {e}")))?;
        let socket = Arc::new(socket);

        let state = BacnetNodeState {
            host: host.clone(),
            port,
            device_instance: device_cfg,
            timeout_ms,
            write_priority: Some(priority as u8),
            groups: Vec::new(),
            tags: Vec::new(),
            socket: socket.clone(),
            remote,
            invoke: std::sync::atomic::AtomicU8::new(0),
        };

        if verify {
            let frame = bvlc_wrap(&npdu_wrap(&encode_who_is(), true), false);
            let apdu = roundtrip(&socket, remote, &frame, None, state.timeout()).await?;
            match decode_i_am(&apdu) {
                Some(instance) => {
                    if let Some(want) = device_cfg {
                        if want != instance {
                            return Err(PluginError::connection_failed(format!(
                                "bacnet device instance mismatch: want {want}, got {instance}"
                            )));
                        }
                    }
                    log::info(
                        node_id,
                        format!("bacnet device discovered: instance {instance}"),
                    );
                }
                None => {
                    return Err(PluginError::connection_failed(
                        "bacnet device did not answer Who-Is (no I-Am received)",
                    ));
                }
            }
        }

        log::info(node_id, format!("bacnet node opened: {host}:{port}"));
        self.state.write().await.insert(node_id, state);
        Ok(())
    }

    async fn close(&self, node_id: NodeId) -> PluginResult<()> {
        self.state.write().await.remove(&node_id);
        Ok(())
    }

    async fn setting(&self, node_id: NodeId, config: PluginConfig) -> PluginResult<()> {
        let mut state = self.state.write().await;
        let Some(s) = state.get_mut(&node_id) else {
            return Err(PluginError::msg("node not open"));
        };
        let host = config_str(&config, "host", &s.host);
        let port = config_u16(&config, "port", s.port);
        let device_cfg_num = config_u32(&config, "device_instance", 0);
        let timeout_ms =
            config_u32(&config, "timeout_ms", s.timeout_ms).clamp(TIMEOUT_MIN_MS, TIMEOUT_MAX_MS);
        let priority = config_u32(&config, "write_priority", 16).clamp(1, 16);
        s.remote = format!("{host}:{port}").parse().map_err(|_| {
            PluginError::config_invalid(format!("invalid host:port: {host}:{port}"))
        })?;
        s.host = host;
        s.port = port;
        s.device_instance = (device_cfg_num > 0).then_some(device_cfg_num);
        s.timeout_ms = timeout_ms;
        s.write_priority = Some(priority as u8);
        Ok(())
    }

    async fn validate_tag(&self, _node_id: NodeId, tag: &Tag) -> PluginResult<()> {
        if tag.name.trim().is_empty() {
            return Err(PluginError::tag_invalid("tag name required"));
        }
        let parsed = parse_address_full(&tag.address).ok_or_else(|| {
            PluginError::tag_invalid(format!("invalid bacnet address: {}", tag.address))
        })?;
        if parsed.instance > 0x003F_FFFF {
            return Err(PluginError::tag_invalid(format!(
                "bacnet instance out of range: {}",
                parsed.instance
            )));
        }
        if property_id(&parsed.property).is_none() {
            return Err(PluginError::tag_invalid(format!(
                "unsupported property: {}",
                parsed.property
            )));
        }
        Ok(())
    }

    async fn poll_group(
        &self,
        node_id: NodeId,
        _group_id: GroupId,
        tags: &[Tag],
    ) -> PluginResult<Vec<(TagId, DataValue)>> {
        let state = self.state.read().await;
        let s = state
            .get(&node_id)
            .ok_or_else(|| PluginError::msg("node not open"))?;

        let mut out = Vec::with_capacity(tags.len());
        for tag in tags {
            let Some(parsed) = parse_address_full(&tag.address) else {
                log::warn(
                    node_id,
                    format!("bacnet poll: invalid address {}, skipped", tag.address),
                );
                continue;
            };
            let Some(prop) = property_id(&parsed.property) else {
                log::warn(
                    node_id,
                    format!(
                        "bacnet poll: unsupported property {}, skipped",
                        parsed.property
                    ),
                );
                continue;
            };
            let obj = parsed.object_type as u16;
            match read_property(s, obj, parsed.instance, prop).await {
                Ok(bv) => match bv.to_data_value(&parsed.object_type) {
                    Some(dv) => out.push((tag.id, dv)),
                    None => {
                        log::warn(
                            node_id,
                            format!(
                                "bacnet poll: value of {} not representable, skipped",
                                tag.address
                            ),
                        );
                    }
                },
                Err(e) => {
                    log::warn(node_id, format!("bacnet poll {} failed: {e}", tag.address));
                }
            }
        }
        Ok(out)
    }

    async fn write_tags(&self, node_id: NodeId, values: &[(Tag, DataValue)]) -> PluginResult<()> {
        let state = self.state.read().await;
        let s = state
            .get(&node_id)
            .ok_or_else(|| PluginError::msg("node not open"))?;

        for (tag, dv) in values {
            let parsed = parse_address_full(&tag.address).ok_or_else(|| {
                PluginError::tag_invalid(format!("invalid bacnet address: {}", tag.address))
            })?;
            if parsed.property != "present-value" {
                return Err(PluginError::tag_invalid(
                    "bacnet write supports present-value only",
                ));
            }
            let bv = BacnetValue::from_data_value(&parsed.object_type, dv)
                .map_err(PluginError::tag_invalid)?;
            write_property(s, parsed.object_type as u16, parsed.instance, bv).await?;
            log::info(node_id, format!("bacnet write {} ok", tag.address));
        }
        Ok(())
    }

    async fn list_groups(&self, node_id: NodeId) -> PluginResult<Vec<Group>> {
        let state = self.state.read().await;
        let s = state
            .get(&node_id)
            .ok_or_else(|| PluginError::msg("node not open"))?;
        Ok(s.groups.clone())
    }

    async fn list_tags(&self, node_id: NodeId, group_id: GroupId) -> PluginResult<Vec<Tag>> {
        let state = self.state.read().await;
        let s = state
            .get(&node_id)
            .ok_or_else(|| PluginError::msg("node not open"))?;
        Ok(s.tags
            .iter()
            .filter(|t| t.group_id == group_id)
            .cloned()
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn tag_of(address: &str) -> Tag {
        let gid = GroupId::new();
        Tag {
            id: TagId::new(),
            group_id: gid,
            name: address.replace(':', "_"),
            address: address.to_string(),
            attr: gateway_sdk::TagAttr::Read,
            data_type: None,
            description: None,
        }
    }

    /// 起一个假设备：收 ReadProperty 回 ComplexAck(real 42.5)，
    /// 收 WriteProperty 回 SimpleACK 并记录请求帧。
    fn spawn_fake_device() -> (std::net::SocketAddr, Arc<Mutex<Vec<Vec<u8>>>>) {
        let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
        let addr = sock.local_addr().unwrap();
        let received: Arc<Mutex<Vec<Vec<u8>>>> = Arc::new(Mutex::new(Vec::new()));
        let rx = received.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 2048];
            loop {
                let Ok((n, src)) = sock.recv_from(&mut buf) else {
                    return;
                };
                let Some(npdu) = bvlc_payload(&buf[..n]) else {
                    continue;
                };
                let Some(apdu) = npdu_payload(npdu) else {
                    continue;
                };
                if apdu.len() < 4 {
                    continue;
                }
                rx.lock().unwrap().push(apdu.to_vec());
                match apdu[3] {
                    protocol::SERVICE_READ_PROPERTY => {
                        // invoke id 原样回填
                        let ack = vec![
                            0x02, apdu[2], 0x0C, 0xC4, 0x00, 0x00, 0x00, 0x01, 0x19, 0x55, 0x3E,
                            0x44, 0x42, 0x2A, 0x00, 0x00, 0x3F,
                        ];
                        let frame = bvlc_wrap(&npdu_wrap(&ack, false), false);
                        let _ = sock.send_to(&frame, src);
                    }
                    protocol::SERVICE_WRITE_PROPERTY => {
                        let ack = vec![APDU_SIMPLE_ACK, apdu[2], 0x0F];
                        let frame = bvlc_wrap(&npdu_wrap(&ack, false), false);
                        let _ = sock.send_to(&frame, src);
                    }
                    _ => {}
                }
            }
        });
        (addr, received)
    }

    #[tokio::test]
    async fn poll_group_reads_real_value_over_udp() {
        let (addr, _rx) = spawn_fake_device();
        let plugin = BacnetPlugin::new();
        let mut cfg = PluginConfig::new();
        cfg.insert("host".into(), serde_json::json!("127.0.0.1"));
        cfg.insert("port".into(), serde_json::json!(addr.port()));
        cfg.insert("verify_on_open".into(), serde_json::json!(false));
        let nid = NodeId::new();
        plugin.open(nid, cfg).await.unwrap();

        let tag = tag_of("ai:1");
        let out = plugin
            .poll_group(nid, GroupId::new(), std::slice::from_ref(&tag))
            .await
            .unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].0, tag.id);
        assert_eq!(out[0].1, DataValue::Float64(42.5));
    }

    #[tokio::test]
    async fn write_tags_sends_write_property() {
        let (addr, rx) = spawn_fake_device();
        let plugin = BacnetPlugin::new();
        let mut cfg = PluginConfig::new();
        cfg.insert("host".into(), serde_json::json!("127.0.0.1"));
        cfg.insert("port".into(), serde_json::json!(addr.port()));
        cfg.insert("verify_on_open".into(), serde_json::json!(false));
        let nid = NodeId::new();
        plugin.open(nid, cfg).await.unwrap();

        let tag = tag_of("bo:3");
        plugin
            .write_tags(nid, &[(tag, DataValue::Bool(true))])
            .await
            .unwrap();

        let sent = rx.lock().unwrap();
        let last = sent.last().unwrap();
        assert_eq!(last[3], protocol::SERVICE_WRITE_PROPERTY);
        // 值部分：Bool(true) → enumerated 2（BinaryPV active）
        assert!(last.windows(2).any(|w| w == [0x91, 0x02]));
    }

    #[tokio::test]
    async fn verify_on_open_rejects_unreachable_device() {
        let plugin = BacnetPlugin::new();
        let mut cfg = PluginConfig::new();
        cfg.insert("host".into(), serde_json::json!("127.0.0.1"));
        // 空闲 UDP 端口：绑一个不入循环的 socket 占位，Who-Is 必然超时
        let sink = UdpSocket::bind("127.0.0.1:0").unwrap();
        cfg.insert(
            "port".into(),
            serde_json::json!(sink.local_addr().unwrap().port()),
        );
        cfg.insert("timeout_ms".into(), serde_json::json!(200));
        let err = plugin.open(NodeId::new(), cfg).await.unwrap_err();
        assert!(err.to_string().contains("Who-Is") || err.to_string().contains("timeout"));
    }

    #[tokio::test]
    async fn poll_skips_invalid_address_but_keeps_others() {
        let (addr, _rx) = spawn_fake_device();
        let plugin = BacnetPlugin::new();
        let mut cfg = PluginConfig::new();
        cfg.insert("host".into(), serde_json::json!("127.0.0.1"));
        cfg.insert("port".into(), serde_json::json!(addr.port()));
        cfg.insert("verify_on_open".into(), serde_json::json!(false));
        let nid = NodeId::new();
        plugin.open(nid, cfg).await.unwrap();

        let good = tag_of("ai:1");
        let mut bad = tag_of("ai:2");
        bad.address = "not-an-address".into();

        let out = plugin
            .poll_group(nid, GroupId::new(), &[good.clone(), bad])
            .await
            .unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].0, good.id);
    }

    #[tokio::test]
    async fn validate_tag_rejects_unknown_property() {
        let plugin = BacnetPlugin::new();
        let good = tag_of("ai:1:present-value");
        plugin.validate_tag(NodeId::new(), &good).await.unwrap();
        let bad = tag_of("ai:1:not-a-prop");
        assert!(plugin.validate_tag(NodeId::new(), &bad).await.is_err());
    }

    #[test]
    fn property_mapping() {
        assert_eq!(property_id("present-value"), Some(85));
        assert_eq!(property_id("units"), Some(117));
        assert_eq!(property_id("nope"), None);
    }
}
