//! 南向 Modbus RTU 插件：支持串口/DTU、线圈/离散/输入/保持寄存器、
//! 地址格式（SLAVE!ADDRESS[.BIT][#ENDIAN]）、读写、超时重试、字节序、STRING/BYTES 等。

#[cfg(feature = "ffi")]
mod ffi;

mod address;
mod config;
mod state;
mod value;

use gateway_sdk::log;
use gateway_sdk::types::{DataValue, PluginKind};
use gateway_sdk::{
    ConfigSchema, Group, GroupId, NodeId, PluginMeta, SouthPlugin, Tag, TagId, TagSchema,
};
use gateway_sdk::{PluginConfig, PluginError, PluginResult};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use address::parse_address;
#[cfg(feature = "modbus-client")]
use address::{parse_address_full, ModbusArea};
use config::{config_schema, config_str, tag_schema};
use state::ModbusRtuState;
#[cfg(feature = "modbus-client")]
use state::SerialConn;
#[cfg(feature = "modbus-client")]
use std::time::Duration;
#[cfg(feature = "modbus-client")]
use value::{register_to_value_ext, value_to_registers};

/// 批量读合并的地址 gap 阈值（寄存器数）：两段地址间隔 ≤ 8 时并成一次读。
/// 串口链路一问一答、每问毫秒级 RTT，逐点读是延迟大头；少量冗余寄存器
/// 换一次 RTT 划算。再大就不合并（避免拉回大段无关数据）。
#[cfg(feature = "modbus-client")]
const MERGE_GAP: u16 = 8;

/// Modbus 协议单次读上限（数量超限会被设备回异常）
#[cfg(feature = "modbus-client")]
const MAX_COILS_PER_READ: u16 = 2000;
#[cfg(feature = "modbus-client")]
const MAX_REGS_PER_READ: u16 = 125;

/// 一次批量读请求：同 area 的连续（或小 gap 合并后）地址段
#[cfg(feature = "modbus-client")]
#[derive(Debug, Clone, Copy, PartialEq)]
struct BatchRead {
    area: ModbusArea,
    start: u16,
    count: u16,
}

/// 把 `(area, start, count)` 列表贪心合并成最少的批量读请求。
///
/// 同 area 且地址 gap ≤ MERGE_GAP 的相邻段并成一次请求（一次 RTT 读出多点位）；
/// 分段遵守协议单次读上限。返回按 `(area, start)` 升序；重叠/包含的段同样合并
/// （起点取小、终点取大）。
#[cfg(feature = "modbus-client")]
fn merge_reads(mut reqs: Vec<(ModbusArea, u16, u16)>) -> Vec<BatchRead> {
    if reqs.is_empty() {
        return Vec::new();
    }
    // area 排序用 discriminant 顺序即可（同 area 的才合并，跨 area 顺序无语义）
    reqs.sort_by_key(|&(a, s, _)| (a as u8, s));
    let mut out: Vec<BatchRead> = Vec::with_capacity(reqs.len());
    let max_len = |a: ModbusArea| {
        if matches!(a, ModbusArea::Coil | ModbusArea::DiscreteInput) {
            MAX_COILS_PER_READ
        } else {
            MAX_REGS_PER_READ
        }
    };
    for (area, start, count) in reqs {
        let end = start.saturating_add(count.saturating_sub(1)); // 闭区间末址
        let extendable = match out.last_mut() {
            Some(last) if last.area == area => {
                // 新段起点在前段闭区间末址 + MERGE_GAP 之内 → 并段
                start <= last.start.saturating_add(last.count.saturating_sub(1)) + MERGE_GAP
            }
            _ => false,
        };
        if extendable {
            let last = out.last_mut().expect("checked above");
            let new_end = end.max(last.start + last.count.saturating_sub(1));
            last.count = new_end.saturating_sub(last.start) + 1;
        } else {
            out.push(BatchRead { area, start, count });
        }
    }
    // 合并后可能超协议单读上限 → 二次按上限切分
    let mut capped = Vec::with_capacity(out.len());
    for r in out {
        let limit = max_len(r.area) as u32;
        let mut off = 0u32;
        while (off as u16) < r.count {
            let take = r.count.saturating_sub(off as u16).min(limit as u16);
            capped.push(BatchRead {
                area: r.area,
                start: r.start + off as u16,
                count: take,
            });
            off += take as u32;
        }
    }
    capped
}

/// Modbus RTU 南向插件
pub struct ModbusRtuPlugin {
    state: Arc<RwLock<HashMap<NodeId, ModbusRtuState>>>,
    /// 串口连接缓存（节点 id → 已建立连接）。D 级审计修复：此前每轮 poll_group
    /// 都重开串口（Unix open 毫秒级开销；Windows COM 句柄反复分配）。IO 失败
    /// 即移除缓存，下一轮自动重建（自愈重连）；setting 变更按参数比对重建。
    #[cfg(feature = "modbus-client")]
    conns: tokio::sync::Mutex<HashMap<NodeId, SerialConn>>,
}

impl Default for ModbusRtuPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl ModbusRtuPlugin {
    pub fn new() -> Self {
        Self {
            state: Arc::new(RwLock::new(HashMap::new())),
            #[cfg(feature = "modbus-client")]
            conns: tokio::sync::Mutex::new(HashMap::new()),
        }
    }

    fn default_groups() -> Vec<Group> {
        vec![Group {
            id: GroupId::new(),
            name: "default".to_string(),
            interval_ms: 1000,
            description: Some("默认采集组".to_string()),
        }]
    }
}

#[async_trait::async_trait]
impl SouthPlugin for ModbusRtuPlugin {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "modbus-rtu",
            kind: PluginKind::South,
            description: Some("Modbus RTU 南向驱动，通过串口连接 Modbus 设备"),
            version: "0.1.0",
            name_zh: Some("Modbus RTU"),
            name_en: Some("Modbus RTU"),
            description_zh: Some("Modbus RTU 南向驱动，通过串口/DTU 连接 Modbus 设备"),
            description_en: Some(
                "Modbus RTU south driver, connect to Modbus devices via serial/DTU",
            ),
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
        parse_address(&tag.address).ok_or_else(|| {
            PluginError::tag_invalid("address format: 0x!addr / 1x!addr / 3x!addr / 4x!addr")
        })?;
        Ok(())
    }

    async fn open(&self, node_id: NodeId, config: PluginConfig) -> PluginResult<()> {
        let port = config_str(&config, "port", "COM1");
        log::info(node_id, format!("open modbus-rtu: port={}", port));
        let baud_rate = config
            .get("baud_rate")
            .and_then(|v| v.as_u64())
            .map(|n| n as u32)
            .unwrap_or(9600);
        let data_bits = config
            .get("data_bits")
            .and_then(|v| v.as_u64())
            .map(|n| n as u8)
            .unwrap_or(8);
        let stop_bits = config
            .get("stop_bits")
            .and_then(|v| v.as_u64())
            .map(|n| n as u8)
            .unwrap_or(1);
        let parity = config_str(&config, "parity", "none");
        let slave_id = config
            .get("slave_id")
            .and_then(|v| v.as_u64())
            .map(|n| n as u8)
            .unwrap_or(1);
        let connection_timeout_ms = config
            .get("connection_timeout_ms")
            .and_then(|v| v.as_u64())
            .unwrap_or(3000);
        let send_interval_ms = config
            .get("send_interval_ms")
            .and_then(|v| v.as_u64())
            .unwrap_or(20);
        let max_retry_times = config
            .get("max_retry_times")
            .and_then(|v| v.as_u64())
            .map(|n| n as u32)
            .unwrap_or(3);
        let retry_interval_ms = config
            .get("retry_interval_ms")
            .and_then(|v| v.as_u64())
            .unwrap_or(100);
        let start_address = config
            .get("start_address")
            .and_then(|v| v.as_u64())
            .map(|n| n as u8)
            .unwrap_or(1)
            .min(1);
        let groups = Self::default_groups();
        let tags = groups
            .iter()
            .flat_map(|g| {
                vec![Tag {
                    id: TagId::new(),
                    name: "holding_0".to_string(),
                    address: "4x!0".to_string(),
                    attr: gateway_sdk::TagAttr::Read,
                    data_type: Some("uint16".to_string()),
                    description: Some("保持寄存器 0".to_string()),
                    group_id: g.id,
                }]
            })
            .collect::<Vec<_>>();
        let mut state = self.state.write().await;
        state.insert(
            node_id,
            ModbusRtuState {
                port,
                baud_rate,
                data_bits,
                stop_bits,
                parity,
                slave_id,
                connection_timeout_ms,
                send_interval_ms,
                max_retry_times,
                retry_interval_ms,
                start_address,
                groups,
                tags,
            },
        );
        Ok(())
    }

    async fn close(&self, node_id: NodeId) -> PluginResult<()> {
        log::info(node_id, "close modbus-rtu");
        let mut state = self.state.write().await;
        state.remove(&node_id);
        // 连接缓存一并释放（否则 COM 句柄滞留到下一轮 poll 的参数比对）
        #[cfg(feature = "modbus-client")]
        self.conns.lock().await.remove(&node_id);
        Ok(())
    }

    async fn start(&self, node_id: NodeId) -> PluginResult<()> {
        log::info(node_id, "start modbus-rtu");
        Ok(())
    }

    async fn stop(&self, node_id: NodeId) -> PluginResult<()> {
        log::info(node_id, "stop modbus-rtu");
        Ok(())
    }

    async fn setting(&self, node_id: NodeId, config: PluginConfig) -> PluginResult<()> {
        log::info(node_id, "setting modbus-rtu (config updated)");
        let port = config_str(&config, "port", "COM1");
        let mut state = self.state.write().await;
        if let Some(s) = state.get_mut(&node_id) {
            s.port = port;
            s.baud_rate = config
                .get("baud_rate")
                .and_then(|v| v.as_u64())
                .map(|n| n as u32)
                .unwrap_or(s.baud_rate);
            s.slave_id = config
                .get("slave_id")
                .and_then(|v| v.as_u64())
                .map(|n| n as u8)
                .unwrap_or(s.slave_id);
            s.connection_timeout_ms = config
                .get("connection_timeout_ms")
                .and_then(|v| v.as_u64())
                .unwrap_or(s.connection_timeout_ms);
            s.send_interval_ms = config
                .get("send_interval_ms")
                .and_then(|v| v.as_u64())
                .unwrap_or(s.send_interval_ms);
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

        #[cfg(feature = "modbus-client")]
        {
            use tokio_modbus::client::rtu::attach_slave;
            use tokio_modbus::prelude::*;
            use tokio_modbus::slave::Slave;
            use tokio_serial::SerialPortBuilderExt;

            // ---- 1. 地址解析（解析失败的点位直接占位，不参与批量读）----
            let mut out: Vec<Option<DataValue>> = vec![None; tags.len()];
            let mut parsed: Vec<(usize, address::ParsedAddress)> = Vec::new();
            let mut raw_reqs: Vec<(ModbusArea, u16, u16)> = Vec::new();
            for (i, tag) in tags.iter().enumerate() {
                match parse_address_full(&tag.address, s.start_address) {
                    Some(p) => {
                        raw_reqs.push((p.area, p.start, p.count));
                        parsed.push((i, p));
                    }
                    None => out[i] = Some(DataValue::UInt16(0)),
                }
            }

            // ---- 2. 批量合并（一次 RTT 读出连续多点位；纯函数，单测覆盖）----
            let batches = merge_reads(raw_reqs);
            let send_interval = Duration::from_millis(s.send_interval_ms.min(5000));

            // ---- 3. 连接复用：参数匹配用缓存，否则重建（自愈：IO 失败移除后下轮重开）----
            let mut conns = self.conns.lock().await;
            let stale = match conns.get(&node_id) {
                Some(c) => {
                    c.port != s.port || c.baud_rate != s.baud_rate || c.slave_id != s.slave_id
                }
                None => true,
            };
            if stale {
                let builder = tokio_serial::new(&s.port, s.baud_rate)
                    .data_bits(match s.data_bits {
                        7 => tokio_serial::DataBits::Seven,
                        _ => tokio_serial::DataBits::Eight,
                    })
                    .stop_bits(match s.stop_bits {
                        2 => tokio_serial::StopBits::Two,
                        _ => tokio_serial::StopBits::One,
                    })
                    .parity(match s.parity.to_lowercase().as_str() {
                        "even" => tokio_serial::Parity::Even,
                        "odd" => tokio_serial::Parity::Odd,
                        _ => tokio_serial::Parity::None,
                    });
                let serial = builder
                    .open_native_async()
                    .map_err(|e| PluginError::msg(format!("serial open: {}", e)))?;
                conns.insert(
                    node_id,
                    SerialConn {
                        ctx: attach_slave(serial, Slave(s.slave_id)),
                        port: s.port.clone(),
                        baud_rate: s.baud_rate,
                        slave_id: s.slave_id,
                    },
                );
            }
            let conn = conns.get_mut(&node_id).expect("conn just ensured");

            // ---- 4. 依序执行批量读并分发回各点位 ----
            for batch in batches {
                // 线圈/离散（bool 向量）与寄存器（u16 向量）返回类型不同，分开求值
                let bit_result = match batch.area {
                    ModbusArea::Coil => Some(conn.ctx.read_coils(batch.start, batch.count).await),
                    ModbusArea::DiscreteInput => Some(
                        conn.ctx
                            .read_discrete_inputs(batch.start, batch.count)
                            .await,
                    ),
                    _ => None,
                };
                let reg_result = match batch.area {
                    ModbusArea::InputRegister => Some(
                        conn.ctx
                            .read_input_registers(batch.start, batch.count)
                            .await,
                    ),
                    ModbusArea::HoldingRegister => Some(
                        conn.ctx
                            .read_holding_registers(batch.start, batch.count)
                            .await,
                    ),
                    _ => None,
                };
                // 统一拆包：任一 IO/协议失败 → 重建连接（设备离线/总线扰动的常见
                // 表现是连续异常帧）并整轮失败
                enum ReadVals {
                    Bits(Vec<bool>),
                    Regs(Vec<u16>),
                }
                let vals = if let Some(r) = bit_result {
                    match r {
                        Ok(Ok(v)) => ReadVals::Bits(v),
                        Ok(Err(e)) => {
                            conns.remove(&node_id);
                            return Err(PluginError::msg(format!("modbus exception: {}", e)));
                        }
                        Err(e) => {
                            conns.remove(&node_id);
                            return Err(PluginError::msg(format!("modbus io: {}", e)));
                        }
                    }
                } else {
                    match reg_result.expect("bit/reg one side must be Some") {
                        Ok(Ok(v)) => ReadVals::Regs(v),
                        Ok(Err(e)) => {
                            conns.remove(&node_id);
                            return Err(PluginError::msg(format!("modbus exception: {}", e)));
                        }
                        Err(e) => {
                            conns.remove(&node_id);
                            return Err(PluginError::msg(format!("modbus io: {}", e)));
                        }
                    }
                };
                for (tag_idx, p) in parsed.iter() {
                    if p.area != batch.area || p.start < batch.start {
                        continue;
                    }
                    let rel = (p.start - batch.start) as usize;
                    if rel + p.count as usize > {
                        match &vals {
                            ReadVals::Bits(v) => v.len(),
                            ReadVals::Regs(v) => v.len(),
                        }
                    } {
                        continue;
                    }
                    let tag = &tags[*tag_idx];
                    let dt = tag.data_type.as_deref().unwrap_or("uint16");
                    let v = match (&vals, p.area) {
                        (ReadVals::Bits(v), _) => DataValue::Bool(v[rel]),
                        (ReadVals::Regs(v), _) => register_to_value_ext(
                            &v[rel..rel + p.count as usize],
                            dt,
                            &p.endian,
                            p.bit_index,
                        ),
                    };
                    out[*tag_idx] = Some(v);
                }
                // 批次间隔（保留原逐点间隔的防总线拥挤语义，现在是批间间隔）
                tokio::time::sleep(send_interval).await;
            }
            // 解析失败/未命中的占位兜底（理论上已在第 1 步填过）
            Ok(tags
                .iter()
                .zip(out)
                .map(|(t, v)| (t.id, v.unwrap_or(DataValue::UInt16(0))))
                .collect())
        }

        #[cfg(not(feature = "modbus-client"))]
        {
            let _ = (node_id, s);
            let mut out = Vec::with_capacity(tags.len());
            for tag in tags {
                let _ = parse_address(&tag.address);
                out.push((tag.id, DataValue::UInt16(0)));
            }
            Ok(out)
        }
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

    async fn write_tags(&self, node_id: NodeId, values: &[(Tag, DataValue)]) -> PluginResult<()> {
        if values.is_empty() {
            return Ok(());
        }
        let state = self.state.read().await;
        let s = state
            .get(&node_id)
            .ok_or_else(|| PluginError::msg("node not open"))?;

        #[cfg(feature = "modbus-client")]
        {
            use tokio_modbus::client::rtu::attach_slave;
            use tokio_modbus::prelude::*;
            use tokio_modbus::slave::Slave;
            use tokio_serial::SerialPortBuilderExt;

            // 连接复用（与 poll_group 同一缓存；反控写频率低但同样没必要重开串口）。
            let mut conns = self.conns.lock().await;
            let stale = match conns.get(&node_id) {
                Some(c) => {
                    c.port != s.port || c.baud_rate != s.baud_rate || c.slave_id != s.slave_id
                }
                None => true,
            };
            if stale {
                let builder = tokio_serial::new(&s.port, s.baud_rate)
                    .data_bits(match s.data_bits {
                        7 => tokio_serial::DataBits::Seven,
                        _ => tokio_serial::DataBits::Eight,
                    })
                    .stop_bits(match s.stop_bits {
                        2 => tokio_serial::StopBits::Two,
                        _ => tokio_serial::StopBits::One,
                    })
                    .parity(match s.parity.to_lowercase().as_str() {
                        "even" => tokio_serial::Parity::Even,
                        "odd" => tokio_serial::Parity::Odd,
                        _ => tokio_serial::Parity::None,
                    });
                let serial = builder
                    .open_native_async()
                    .map_err(|e| PluginError::msg(format!("serial open: {}", e)))?;
                conns.insert(
                    node_id,
                    SerialConn {
                        ctx: attach_slave(serial, Slave(s.slave_id)),
                        port: s.port.clone(),
                        baud_rate: s.baud_rate,
                        slave_id: s.slave_id,
                    },
                );
            }
            let conn = conns.get_mut(&node_id).expect("conn just ensured");
            let interval = Duration::from_millis(s.send_interval_ms.min(5000));

            for (tag, value) in values {
                let Some(parsed) = parse_address_full(&tag.address, s.start_address) else {
                    continue;
                };
                match parsed.area {
                    ModbusArea::Coil => {
                        let b = value.as_bool().unwrap_or(false);
                        if parsed.count == 1 {
                            if let Err(e) = conn.ctx.write_single_coil(parsed.start, b).await {
                                conns.remove(&node_id);
                                return Err(PluginError::msg(format!("write_single_coil: {}", e)));
                            }
                        } else {
                            let coils: Vec<bool> = (0..parsed.count).map(|_| b).collect();
                            if let Err(e) =
                                conn.ctx.write_multiple_coils(parsed.start, &coils).await
                            {
                                conns.remove(&node_id);
                                return Err(PluginError::msg(format!(
                                    "write_multiple_coils: {}",
                                    e
                                )));
                            }
                        }
                    }
                    ModbusArea::HoldingRegister => {
                        let regs =
                            value_to_registers(value, tag.data_type.as_deref().unwrap_or("uint16"));
                        if regs.is_empty() {
                            continue;
                        }
                        if regs.len() == 1 {
                            if let Err(e) =
                                conn.ctx.write_single_register(parsed.start, regs[0]).await
                            {
                                conns.remove(&node_id);
                                return Err(PluginError::msg(format!(
                                    "write_single_register: {}",
                                    e
                                )));
                            }
                        } else if let Err(e) =
                            conn.ctx.write_multiple_registers(parsed.start, &regs).await
                        {
                            conns.remove(&node_id);
                            return Err(PluginError::msg(format!(
                                "write_multiple_registers: {}",
                                e
                            )));
                        }
                    }
                    _ => {
                        return Err(PluginError::tag_invalid(
                            "only coil and holding register support write",
                        ))
                    }
                }
                tokio::time::sleep(interval).await;
            }
            Ok(())
        }

        #[cfg(not(feature = "modbus-client"))]
        {
            let _ = (node_id, s, values);
            Err(PluginError::not_supported(
                "write requires modbus-client feature",
            ))
        }
    }
}

// ---------------------------------------------------------------------------
// 测试（D 级审计 2026-11-22：modbus-rtu 此前零测试——批量合并是纯函数，不依赖
// 串口即可完整验证；连接复用/自愈路径由特性构建的编译检查 + 现场联调覆盖）
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    fn hr(start: u16, count: u16) -> (ModbusArea, u16, u16) {
        (ModbusArea::HoldingRegister, start, count)
    }

    #[test]
    fn merge_reads_empty_input() {
        assert!(merge_reads(Vec::new()).is_empty());
    }

    #[test]
    fn merge_reads_single_request_passthrough() {
        let out = merge_reads(vec![hr(10, 2)]);
        assert_eq!(
            out,
            vec![BatchRead {
                area: ModbusArea::HoldingRegister,
                start: 10,
                count: 2
            }]
        );
    }

    #[test]
    fn merge_reads_contiguous_same_area() {
        // 4x!0(2) + 4x!2(3) 地址连续 → 一次读 5 个
        let out = merge_reads(vec![hr(0, 2), hr(2, 3)]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].start, 0);
        assert_eq!(out[0].count, 5);
    }

    #[test]
    fn merge_reads_small_gap_merges() {
        // gap 2 ≤ MERGE_GAP(8) → 合并（多读 2 个冗余寄存器换一次 RTT）
        let out = merge_reads(vec![hr(0, 2), hr(4, 2)]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].count, 6);
    }

    #[test]
    fn merge_reads_large_gap_stays_separate() {
        // gap 100 > MERGE_GAP → 各自独立
        let out = merge_reads(vec![hr(0, 2), hr(102, 2)]);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn merge_reads_different_areas_never_merge() {
        let out = merge_reads(vec![
            hr(0, 2),
            (ModbusArea::InputRegister, 2, 2),
            (ModbusArea::Coil, 4, 1),
        ]);
        assert_eq!(out.len(), 3);
    }

    #[test]
    fn merge_reads_overlapping_ranges_union() {
        // [10,12) 与 [11,15) 重叠 → 并段 [10,15)
        let out = merge_reads(vec![hr(11, 4), hr(10, 2)]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].start, 10);
        assert_eq!(out[0].count, 5);
    }

    #[test]
    fn merge_reads_respects_protocol_read_limit() {
        // 合并后 130 个寄存器 > 单读上限 125 → 切成两段
        let out = merge_reads(vec![hr(0, 130)]);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].count, 125);
        assert_eq!(out[1].start, 125);
        assert_eq!(out[1].count, 5);
    }

    #[test]
    fn merge_reads_coil_limit_is_2000() {
        let out = merge_reads(vec![(ModbusArea::Coil, 0, 2500)]);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].count, 2000);
        assert_eq!(out[1].start, 2000);
    }

    #[test]
    fn merge_reads_sorts_by_start_before_merging() {
        // 乱序输入：0/5/3 三段 → 全并成 [0, 7)
        let out = merge_reads(vec![hr(5, 2), hr(0, 1), hr(3, 2)]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].start, 0);
        assert_eq!(out[0].count, 7);
    }
}
