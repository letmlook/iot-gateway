//! 南向振动采集插件（virb）：对标 dataacq-plugin-virb。
//! 联能 YE6235D/YE6235D2 振动采集器，UDP 通信：上位机发控制命令，采集器推送波形数据。
//! 支持 raw 组（时域 max/min/avg）与配置与参考实现一致。

#[cfg(feature = "ffi")]
mod ffi;

mod protocol;
mod schema;
mod state;

use gateway_sdk::{
    ConfigSchema, Group, GroupId, NodeId, PluginMeta, SouthPlugin, Tag, TagId, TagSchema,
};
use gateway_sdk::log;
use gateway_sdk::types::{DataValue, PluginKind};
use gateway_sdk::{PluginConfig, PluginError, PluginResult};
use protocol::{
    decode_upload_data, encode_set_channel_prop, encode_set_sample_rate, encode_start_grab,
    encode_stop_grab, handle_packet_count, sample_rate_id_to_hz, VirbEndianess, VirbModel,
};
use schema::{config_schema, tag_schema};
use state::{default_groups, default_tags, parse_address, parse_channel_params, VirbState};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use tokio::sync::{oneshot, RwLock};
use tokio::net::UdpSocket;

const MAX_PACKET_QUEUE: usize = 500;

/// 振动采集插件（与 dataacq-plugin-virb 功能对齐）
pub struct VirbPlugin {
    state: Arc<RwLock<HashMap<NodeId, VirbState>>>,
}

impl Default for VirbPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl VirbPlugin {
    pub fn new() -> Self {
        Self {
            state: Arc::new(RwLock::new(HashMap::new())),
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
        Some(config_schema())
    }

    fn tag_schema(&self) -> Option<TagSchema> {
        Some(tag_schema())
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
        let groups = default_groups(interval_ms);
        let tags = groups
            .iter()
            .flat_map(|g| default_tags(g.id, channels))
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

    async fn setting(&self, node_id: NodeId, config: PluginConfig) -> PluginResult<()> {
        log::info(node_id, "setting virb (config updated)");
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
        let mut state = self.state.write().await;
        if let Some(s) = state.get_mut(&node_id) {
            s.host = host;
            s.port = port;
        }
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
            let ch_index = parse_address(&tag.address, channels).unwrap_or(0);
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
