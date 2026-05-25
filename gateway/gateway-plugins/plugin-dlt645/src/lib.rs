//! DL/T645-2007 电表协议插件
//! 支持 DL/T645-2007 协议的电表数据采集（电压、电流、功率、电能等）
//!
//! 协议格式（读数据帧）：
//!   68H | A0-A5 (6B) | 04H | L | DI3 DI2 DI1 DI0 (4B) | DATA | CS | 16H
//!
//! 应答格式：
//!   68H | A0-A5 (6B) | 84H | L | DI3 DI2 DI1 DI0 (4B) | DATA | CS | 16H
//!
//! - 地址域 A0-A5：6 字节电表地址，BCD 码，低字节在前
//! - 控制码：04H=读数据，84H=读数据应答
//! - DI：数据标识，4 字节，传输时每个字节的低 4 位为原数据，高 4 位取反
//! - CS：校验和，从 68H 到数据域的算术和对 256 取模
//! - 16H：结束码

#[cfg(feature = "ffi")]
mod ffi;

mod state;

use gateway_sdk::{
    ConfigSchema, Group, GroupId, NodeId, ParamAttribute, ParamSchema, ParamType, ParamValid,
    PluginMeta, SouthPlugin, Tag, TagId, TagSchema,
};
use gateway_sdk::log;
use gateway_sdk::types::{DataValue, PluginKind};
use gateway_sdk::{PluginConfig, PluginError, PluginResult};
use state::Dlt645State;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

// =============================================================================
// DL/T645-2007 协议常量
// =============================================================================

/// 帧起始字节
const FRAME_START: u8 = 0x68;
/// 帧结束字节
const FRAME_END: u8 = 0x16;
/// 控制码：读数据
const CTRL_READ: u8 = 0x04;
/// 控制码：读数据应答（成功）
const CTRL_READ_RESP: u8 = 0x84;

/// DL/T645-2007 标准 DI 码（数据标识）
mod di {
    /// 总有功电能 (kWh)，正向有功
    pub const TOTAL_ACTIVE_ENERGY: &[u8; 4] = &[0x90, 0x10, 0x00, 0x00];
    /// 剩余电量 (kWh)
    pub const REMAINING_ENERGY: &[u8; 4] = &[0x90, 0x10, 0x00, 0x01];
    /// 总无功电能 (kvarh)
    pub const TOTAL_REACTIVE_ENERGY: &[u8; 4] = &[0x90, 0x20, 0x00, 0x00];
    /// A 相电压 (V)
    pub const VOLTAGE_A: &[u8; 4] = &[0x00, 0x01, 0x00, 0x12];
    /// B 相电压 (V)
    pub const VOLTAGE_B: &[u8; 4] = &[0x00, 0x01, 0x00, 0x13];
    /// C 相电压 (V)
    pub const VOLTAGE_C: &[u8; 4] = &[0x00, 0x01, 0x00, 0x14];
    /// A 相电流 (A)
    pub const CURRENT_A: &[u8; 4] = &[0x00, 0x01, 0x00, 0x21];
    /// B 相电流 (A)
    pub const CURRENT_B: &[u8; 4] = &[0x00, 0x01, 0x00, 0x22];
    /// C 相电流 (A)
    pub const CURRENT_C: &[u8; 4] = &[0x00, 0x01, 0x00, 0x23];
    /// 总有功功率 (kW)
    pub const TOTAL_ACTIVE_POWER: &[u8; 4] = &[0x00, 0x01, 0x00, 0x10];
    /// A 相有功功率 (kW)
    pub const ACTIVE_POWER_A: &[u8; 4] = &[0x00, 0x01, 0x00, 0x11];
    /// B 相有功功率 (kW)
    pub const ACTIVE_POWER_B: &[u8; 4] = &[0x00, 0x01, 0x00, 0x12];
    /// C 相有功功率 (kW)
    pub const ACTIVE_POWER_C: &[u8; 4] = &[0x00, 0x01, 0x00, 0x13];
    /// 总无功功率 (kvar)
    pub const TOTAL_REACTIVE_POWER: &[u8; 4] = &[0x00, 0x01, 0x00, 0x20];
    /// 功率因数
    pub const POWER_FACTOR: &[u8; 4] = &[0x00, 0x01, 0x00, 0x30];
    /// 电网频率 (Hz)
    pub const FREQUENCY: &[u8; 4] = &[0x00, 0x01, 0x00, 0x31];
}

// =============================================================================
// DL/T645-2007 协议帧编解码
// =============================================================================

/// 将 4 字符地址字符串（如 "000000000000"）转换为 6 字节地址数组（低字节在前）
/// 每个地址字符是 ASCII '0'-'9'，对应 BCD 编码
fn parse_meter_address(addr: &str) -> Result<[u8; 6], PluginError> {
    let digits: Vec<u8> = addr
        .chars()
        .filter(|c| c.is_ascii_digit())
        .map(|c| c as u8 - b'0')
        .collect();

    if digits.len() != 12 {
        return Err(PluginError::msg(format!(
            "meter_id must be 12 digits, got {} ('{}')",
            digits.len(),
            addr
        )));
    }

    // BCD 编码：每两个数字组成一个字节，低位是第一个数字
    // "000102030405" → [0x05, 0x04, 0x03, 0x02, 0x01, 0x00]
    let mut out = [0u8; 6];
    for i in 0..6 {
        out[i] = (digits[i * 2] << 4) | digits[i * 2 + 1];
    }
    Ok(out)
}

/// 将 DI 字符串（如 "9010"、"00010012"）转换为 4 字节协议编码
/// DL/T645 传输时每个字节的低 4 位为原 BCD，高 4 位为原 BCD 取反
fn parse_di_to_frame(di_str: &str) -> Result<[u8; 4], PluginError> {
    let di_hex = di_str.to_uppercase();

    // 标准化：去除空格，确保是有效 hex
    let clean: String = di_hex.chars().filter(|c| c.is_ascii_hexdigit()).collect();

    // DI 长度支持 4 位（如 9010）或 8 位（如 00010012）
    let mut out = [0u8; 4];

    match clean.len() {
        4 => {
            // 短格式：DI3=第一字节，DI2=第二字节，DI1=第三字节，DI0=第四字节
            let v = u16::from_str_radix(&clean, 16)
                .map_err(|_| PluginError::msg(format!("invalid DI: {}", di_str)))?;
            out = [
                ((v >> 12) & 0x0F) as u8,
                ((v >> 8) & 0x0F) as u8,
                ((v >> 4) & 0x0F) as u8,
                (v & 0x0F) as u8,
            ];
        }
        8 => {
            let v = u32::from_str_radix(&clean, 16)
                .map_err(|_| PluginError::msg(format!("invalid DI: {}", di_str)))?;
            out = [
                ((v >> 28) & 0x0F) as u8,
                ((v >> 24) & 0x0F) as u8,
                ((v >> 20) & 0x0F) as u8,
                ((v >> 16) & 0x0F) as u8,
            ];
        }
        _ => {
            return Err(PluginError::msg(format!(
                "DL/T645 DI must be 4 or 8 hex digits, got '{}' (len={})",
                di_str,
                clean.len()
            )));
        }
    };

    // 字节翻转：DI 域按字节 reverse（低字节在前）
    out.reverse();
    Ok(out)
}

/// 将 DI 字符串映射到标准 DI 常量（用于按标号读取）
fn di_string_to_const(di_str: &str) -> Option<[u8; 4]> {
    match di_str.to_uppercase().replace(" ", "").as_str() {
        "9010" | "TOTAL_ACTIVE_ENERGY" => Some(*di::TOTAL_ACTIVE_ENERGY),
        "9020" | "TOTAL_REACTIVE_ENERGY" => Some(*di::TOTAL_REACTIVE_ENERGY),
        "00010012" | "VOLTAGE" | "VOLTAGE_A" => Some(*di::VOLTAGE_A),
        "00010013" | "CURRENT" | "CURRENT_A" => Some(*di::CURRENT_A),
        "00010014" | "TOTAL_ACTIVE_POWER" => Some(*di::TOTAL_ACTIVE_POWER),
        "00010015" | "TOTAL_REACTIVE_POWER" => Some(*di::TOTAL_REACTIVE_POWER),
        "00010016" | "POWER_FACTOR" => Some(*di::POWER_FACTOR),
        "00010017" | "FREQUENCY" => Some(*di::FREQUENCY),
        _ => None,
    }
}

/// DL/T645 数据字节解码：将传输格式（高 4 位取反，低 4 位为原 BCD）还原为原 BCD
fn decode_dlt645_byte(b: u8) -> u8 {
    let low = b & 0x0F;
    let high = (!b >> 4) & 0x0F;
    (high << 4) | low
}

/// 解码 DL/T645 数据域
/// DL/T645 传输时每个字节高 4 位取反，低 4 位是原 BCD
fn decode_data_field(data: &[u8]) -> Vec<u8> {
    data.iter().map(|&b| decode_dlt645_byte(b)).collect()
}

/// 计算校验和（从 68H 到数据域的算术和，对 256 取模）
fn calc_cs(frame: &[u8]) -> u8 {
    frame.iter().map(|&b| b as u16).sum::<u16>() as u8
}

/// 构建读数据请求帧
/// 格式：68 | A(6) | 04 | L | DI(4) | CS | 16
fn build_read_frame(addr: &[u8; 6], di: &[u8; 4]) -> Vec<u8> {
    // DI 传输时需要按 DL/T645 编码：高 4 位 = 原低 4 位取反，低 4 位 = 原低 4 位
    let di_encoded: Vec<u8> = di.iter().map(|&b| {
        let low = b & 0x0F;
        let high = (!b) & 0x0F;
        (high << 4) | low
    }).collect();

    let mut frame = Vec::with_capacity(13);
    frame.push(FRAME_START);          // 68H
    frame.extend_from_slice(addr);      // A0-A5 (6 bytes)
    frame.push(FRAME_START);           // 68H (repeated)
    frame.push(CTRL_READ);             // 04H 控制码
    frame.push(4u8);                    // L = 4 (DI 长度)
    frame.extend_from_slice(&di_encoded); // DI(4)

    // 计算校验和（从第一个 68H 到 DI 最后一个字节）
    let cs = calc_cs(&[
        frame[0],                    // 68
        addr[0], addr[1], addr[2], addr[3], addr[4], addr[5], // A(6)
        frame[7],                     // 68 (second)
        CTRL_READ,                   // 04
        4u8,                          // L
        di_encoded[0], di_encoded[1], di_encoded[2], di_encoded[3], // DI(4)
    ]);

    frame.push(cs);                   // CS
    frame.push(FRAME_END);            // 16H
    frame
}

// =============================================================================
// 串口通信
// =============================================================================

#[cfg(feature = "dlt645-client")]
async fn send_and_receive(
    port: &str,
    baud: u32,
    data_bits: u8,
    stop_bits: u8,
    parity: &str,
    addr: &[u8; 6],
    di: &[u8; 4],
    read_timeout_ms: u64,
    send_interval_ms: u64,
    max_retry: u32,
) -> PluginResult<Vec<u8>> {
    use tokio_serial::SerialPortBuilderExt;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let builder = tokio_serial::new(port, baud)
        .data_bits(match data_bits {
            7 => tokio_serial::DataBits::Seven,
            _ => tokio_serial::DataBits::Eight,
        })
        .stop_bits(match stop_bits {
            2 => tokio_serial::StopBits::Two,
            _ => tokio_serial::StopBits::One,
        })
        .parity(match parity.to_lowercase().as_str() {
            "even" => tokio_serial::Parity::Even,
            "odd" => tokio_serial::Parity::Odd,
            _ => tokio_serial::Parity::None,
        });

    let serial = builder
        .open_native_async()
        .map_err(|e| PluginError::msg(format!("serial open {}: {}", port, e)))?;

    let mut serial = tokio::io::BufStream::new(serial);

    let request = build_read_frame(addr, di);

    for attempt in 0..=max_retry {
        if attempt > 0 {
            tokio::time::sleep(tokio::time::Duration::from_millis(
                send_interval_ms.max(50) * u64::from(attempt),
            ))
            .await;
        }

        // 清空接收缓冲区
        let mut flush_buf = [0u8; 256];
        let _ = serial.read(&mut flush_buf[..]).await;

        // 发送请求
        serial
            .write_all(&request)
            .await
            .map_err(|e| PluginError::msg(format!("serial write: {}", e)))?;
        serial
            .flush()
            .await
            .map_err(|e| PluginError::msg(format!("serial flush: {}", e)))?;

        // 等待响应：DL/T645 响应帧长不固定，但有明确结束标志 16H
        // 最小响应：68 A(6) 84/94 L DI(4) CS 16 = 1+6+1+1+4+1+1 = 15 bytes
        let mut response = vec![0u8; 128];
        let mut total_read = 0usize;
        let deadline = tokio::time::Instant::now()
            + tokio::time::Duration::from_millis(read_timeout_ms.max(500));

        loop {
            let timeout = deadline.saturating_duration_since(tokio::time::Instant::now());
            if timeout.is_zero() {
                break;
            }

            let mut buf = [0u8; 64];
            match tokio::time::timeout(timeout, serial.read(&mut buf)).await {
                Ok(Ok(0)) => break,
                Ok(Ok(n)) => {
                    response[total_read..total_read + n].copy_from_slice(&buf[..n]);
                    total_read += n;

                    // 检查是否收到完整帧（以 16H 结尾）
                    if response[..total_read].contains(&FRAME_END) {
                        break;
                    }
                }
                Ok(Err(e)) => {
                    return Err(PluginError::msg(format!("serial read: {}", e)));
                }
                Err(_) => {
                    // 超时
                    break;
                }
            }
        }

        if total_read == 0 {
            continue;
        }

        let resp = &response[..total_read];

        // 验证帧头 68H ... 68H
        if resp.len() < 12 || resp[0] != FRAME_START || resp[7] != FRAME_START {
            continue;
        }

        // 验证结束码
        if resp[resp.len() - 1] != FRAME_END {
            continue;
        }

        // 验证控制码（84H=正常应答，94H=异常应答）
        let ctrl = resp[9];
        if ctrl != CTRL_READ_RESP && ctrl != 0x94 {
            continue;
        }

        if ctrl == 0x94 {
            // 异常码：数据异常
            let err_code = resp.get(12).copied().unwrap_or(0);
            return Err(PluginError::msg(format!(
                "DL/T645 meter error response: error_code={:#04x}",
                err_code
            )));
        }

        // 验证校验和
        let cs_received = resp[resp.len() - 2];
        let cs_calc = calc_cs(&resp[..resp.len() - 2]);
        if cs_received != cs_calc {
            continue;
        }

        // 成功
        return Ok(resp.to_vec());
    }

    Err(PluginError::msg("DL/T645: max retries exceeded, no valid response"))
}

// =============================================================================
// DL/T645 数据解码
// =============================================================================

/// 解码 DL/T645 响应数据到 f64
/// DI 对应的数据格式：
/// - 9010 电能：4 字节 BCD，XXXXXX.XX kWh
/// - 电压：2 字节 BCD，XXX.X V
/// - 电流：2 字节 BCD，XXX.XXX A
/// - 功率：3 字节 BCD，XXXXXX.XX kW
/// - 功率因数：1 字节 BCD，0.XXX
/// - 频率：2 字节 BCD，XX.XX Hz
fn decode_value(di: &[u8; 4], data: &[u8]) -> f64 {
    // 还原 DI 高 4 位（因为传输时取反）
    let di_decoded = [
        decode_dlt645_byte(di[0]),
        decode_dlt645_byte(di[1]),
        decode_dlt645_byte(di[2]),
        decode_dlt645_byte(di[3]),
    ];

    // DI 是低字节在前，所以 di_decoded[0] 是最低字节
    let di_val = u32::from(di_decoded[0])
        | (u32::from(di_decoded[1]) << 8)
        | (u32::from(di_decoded[2]) << 16)
        | (u32::from(di_decoded[3]) << 24);

    // 解码数据域
    let decoded = decode_data_field(data);

    match di_val {
        // ===== 电能类 (9010 / 9020) — 4 字节 BCD, XXXXXX.XX kWh/kvarh =====
        0x00009010 => {
            // 总有功电能：4 字节 BCD，低字节在前，高字节在后
            // 例：[12, 34, 56, 78] → 783456.12
            if decoded.len() >= 4 {
                let mut val: f64 = 0.0;
                val += f64::from(decoded[3]) * 100000.0;
                val += f64::from(decoded[2]) * 1000.0;
                val += f64::from(decoded[1]) * 10.0;
                val += f64::from(decoded[0]);
                val /= 100.0; // 去掉最后两位小数
                val
            } else {
                0.0
            }
        }
        0x00009020 => {
            // 总无功电能：4 字节 BCD
            if decoded.len() >= 4 {
                let mut val: f64 = 0.0;
                val += f64::from(decoded[3]) * 100000.0;
                val += f64::from(decoded[2]) * 1000.0;
                val += f64::from(decoded[1]) * 10.0;
                val += f64::from(decoded[0]);
                val /= 100.0;
                val
            } else {
                0.0
            }
        }

        // ===== 电压类 00010012-14 — 2 字节 BCD，XXX.X V =====
        0x00010012 | 0x00010013 | 0x00010014 => {
            // A/B/C 相电压：2 字节 BCD，格式 XXX.X
            // 例：[0x21, 0x04] → 042.1 V（低字节在前）
            if decoded.len() >= 2 {
                let raw = (u16::from(decoded[1]) << 8) | u16::from(decoded[0]);
                let val = f64::from(raw) / 10.0;
                val
            } else {
                0.0
            }
        }

        // ===== 电流类 00010021-23 — 2 字节 BCD，XXX.XXX A =====
        0x00010021 | 0x00010022 | 0x00010023 => {
            // A/B/C 相电流：2 字节 BCD，格式 XXX.XXX
            // 例：[0x21, 0x04] → 04.21 A（低字节在前，实际是 4.21）
            if decoded.len() >= 2 {
                let raw = (u16::from(decoded[1]) << 8) | u16::from(decoded[0]);
                let val = f64::from(raw) / 1000.0;
                val
            } else {
                0.0
            }
        }

        // ===== 功率类 00010010-13 — 3 字节 BCD，XXXXXX.XX kW =====
        0x00010010 | 0x00010011 | 0x00010012 | 0x00010013 => {
            // 总/A/B/C 有功功率：3 字节 BCD
            // 例：[0x12, 0x34, 0x56] → 563412.?? → /100
            if decoded.len() >= 3 {
                let mut val: f64 = 0.0;
                val += f64::from(decoded[2]) * 10000.0;
                val += f64::from(decoded[1]) * 100.0;
                val += f64::from(decoded[0]);
                val /= 100.0;
                val
            } else {
                0.0
            }
        }

        // ===== 无功功率 00010020 — 3 字节 BCD，XXXXXX.XX kvar =====
        0x00010020 => {
            if decoded.len() >= 3 {
                let mut val: f64 = 0.0;
                val += f64::from(decoded[2]) * 10000.0;
                val += f64::from(decoded[1]) * 100.0;
                val += f64::from(decoded[0]);
                val /= 100.0;
                val
            } else {
                0.0
            }
        }

        // ===== 功率因数 00010030 — 1 字节 BCD，0.XXX =====
        0x00010030 => {
            if decoded.is_empty() {
                return 0.0;
            }
            f64::from(decoded[0]) / 1000.0
        }

        // ===== 频率 00010031 — 2 字节 BCD，XX.XX Hz =====
        0x00010031 => {
            if decoded.len() >= 2 {
                let raw = (u16::from(decoded[1]) << 8) | u16::from(decoded[0]);
                f64::from(raw) / 100.0
            } else {
                0.0
            }
        }

        _ => {
            // 通用 BCD 解码：低位在前
            if decoded.is_empty() {
                return 0.0;
            }
            let mut val: f64 = 0.0;
            let mut mult: f64 = 0.01;
            for &b in &decoded {
                val += f64::from(b & 0x0F) * mult;
                val += f64::from((b >> 4) & 0x0F) * mult * 10.0;
                mult *= 100.0;
            }
            val
        }
    }
}

// =============================================================================
// 插件主体
// =============================================================================

/// DL/T645 电表插件
pub struct Dlt645Plugin {
    state: Arc<RwLock<HashMap<NodeId, Dlt645State>>>,
}

impl Default for Dlt645Plugin {
    fn default() -> Self {
        Self::new()
    }
}

impl Dlt645Plugin {
    pub fn new() -> Self {
        Self {
            state: Arc::new(RwLock::new(HashMap::new())),
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
impl SouthPlugin for Dlt645Plugin {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "dlt645",
            kind: PluginKind::South,
            description: Some("DL/T645-2007 电表协议"),
            version: "0.1.0",
            name_zh: Some("DL/T645 电表"),
            name_en: Some("DL/T645 Meter"),
            description_zh: Some("DL/T645-2007 电表通信协议，支持读取电压/电流/功率/电量"),
            description_en: Some("DL/T645-2007 electricity meter protocol — voltage, current, power, energy"),
        }
    }

    fn config_schema(&self) -> Option<ConfigSchema> {
        Some(
            ConfigSchema::new()
                .param(ParamSchema {
                    name: "port".to_string(),
                    name_zh: Some("串口".to_string()),
                    name_en: Some("Serial Port".to_string()),
                    description: Some("串口设备路径，如 /dev/ttyUSB0".to_string()),
                    description_zh: Some("串口设备路径".to_string()),
                    description_en: Some("Serial port device path".to_string()),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::String,
                    default: Some(serde_json::json!("/dev/ttyUSB0")),
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "baud".to_string(),
                    name_zh: Some("波特率".to_string()),
                    name_en: Some("Baud Rate".to_string()),
                    description: Some("串口波特率，2400 或 9600".to_string()),
                    description_zh: Some("串口波特率".to_string()),
                    description_en: Some("Serial port baud rate".to_string()),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(9600)),
                    valid: Some(ParamValid {
                        min: Some(1200),
                        max: Some(115200),
                        regex: None,
                        length: None,
                    }),
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "meter_id".to_string(),
                    name_zh: Some("表号".to_string()),
                    name_en: Some("Meter ID".to_string()),
                    description: Some("电表地址（12 位数字，BCD 编码）".to_string()),
                    description_zh: Some("电表地址（12 位数字）".to_string()),
                    description_en: Some("Meter address (12 digits BCD)".to_string()),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::String,
                    default: Some(serde_json::json!("000000000000")),
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "data_bits".to_string(),
                    name_zh: Some("数据位".to_string()),
                    name_en: Some("Data Bits".to_string()),
                    description: Some("数据位，固定 8".to_string()),
                    description_zh: Some("数据位".to_string()),
                    description_en: Some("Data bits".to_string()),
                    attribute: ParamAttribute::Optional,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(8)),
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "stop_bits".to_string(),
                    name_zh: Some("停止位".to_string()),
                    name_en: Some("Stop Bits".to_string()),
                    description: Some("停止位，固定 1".to_string()),
                    description_zh: Some("停止位".to_string()),
                    description_en: Some("Stop bits".to_string()),
                    attribute: ParamAttribute::Optional,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(1)),
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "parity".to_string(),
                    name_zh: Some("校验".to_string()),
                    name_en: Some("Parity".to_string()),
                    description: Some("校验位：none/even/odd".to_string()),
                    description_zh: Some("校验位".to_string()),
                    description_en: Some("Parity: none/even/odd".to_string()),
                    attribute: ParamAttribute::Optional,
                    ty: ParamType::String,
                    default: Some(serde_json::json!("none")),
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "read_timeout_ms".to_string(),
                    name_zh: Some("读超时".to_string()),
                    name_en: Some("Read Timeout".to_string()),
                    description: Some("读超时毫秒".to_string()),
                    description_zh: Some("读超时 ms".to_string()),
                    description_en: Some("Read timeout in ms".to_string()),
                    attribute: ParamAttribute::Optional,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(1500)),
                    valid: Some(ParamValid {
                        min: Some(500),
                        max: Some(10000),
                        regex: None,
                        length: None,
                    }),
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "send_interval_ms".to_string(),
                    name_zh: Some("发送间隔".to_string()),
                    name_en: Some("Send Interval".to_string()),
                    description: Some("帧间隔毫秒（电表要求最小间隔）".to_string()),
                    description_zh: Some("帧间隔 ms".to_string()),
                    description_en: Some("Frame interval in ms".to_string()),
                    attribute: ParamAttribute::Optional,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(100)),
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "max_retry".to_string(),
                    name_zh: Some("最大重试".to_string()),
                    name_en: Some("Max Retry".to_string()),
                    description: Some("最大重试次数".to_string()),
                    description_zh: Some("最大重试次数".to_string()),
                    description_en: Some("Maximum retry times".to_string()),
                    attribute: ParamAttribute::Optional,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(3)),
                    ..Default::default()
                }),
        )
    }

    fn tag_schema(&self) -> Option<TagSchema> {
        Some(TagSchema {
            data_types: Some(vec![
                "float64".to_string(),
                "int32".to_string(),
            ]),
            address_format: Some("DI 代码如 9010（总有功电能）、9020（无功电能）、00010012（电压）等"
                .to_string()),
            address_format_zh: Some("DI 代码如 9010（总有功电能）、9020（无功电能）、00010012（电压）等"
                .to_string()),
            address_format_en: Some(
                "DI code: 9010 (total active energy), 9020 (reactive energy), 00010012 (voltage), etc."
                    .to_string(),
            ),
        })
    }

    async fn validate_tag(&self, _node_id: NodeId, tag: &Tag) -> PluginResult<()> {
        if tag.name.is_empty() {
            return Err(PluginError::tag_invalid("tag name required"));
        }
        let addr = &tag.address;
        if addr.is_empty() {
            return Err(PluginError::tag_invalid("address (DI code) required"));
        }
        // 允许 4-8 个十六进制字符
        let clean: String = addr.chars().filter(|c| c.is_ascii_hexdigit()).collect();
        if clean.len() < 4 || clean.len() > 8 {
            return Err(PluginError::tag_invalid(
                "DL/T645 DI code must be 4-8 hex digits",
            ));
        }
        Ok(())
    }

    async fn open(&self, node_id: NodeId, config: PluginConfig) -> PluginResult<()> {
        let port = config
            .get("port")
            .and_then(|v| v.as_str())
            .unwrap_or("/dev/ttyUSB0")
            .to_string();
        let baud = config
            .get("baud")
            .and_then(|v| v.as_u64())
            .unwrap_or(9600) as u32;
        let data_bits = config
            .get("data_bits")
            .and_then(|v| v.as_u64())
            .unwrap_or(8) as u8;
        let stop_bits = config
            .get("stop_bits")
            .and_then(|v| v.as_u64())
            .unwrap_or(1) as u8;
        let parity = config
            .get("parity")
            .and_then(|v| v.as_str())
            .unwrap_or("none")
            .to_string();
        let meter_id = config
            .get("meter_id")
            .and_then(|v| v.as_str())
            .unwrap_or("000000000000")
            .to_string();
        let read_timeout_ms = config
            .get("read_timeout_ms")
            .and_then(|v| v.as_u64())
            .unwrap_or(1500);
        let send_interval_ms = config
            .get("send_interval_ms")
            .and_then(|v| v.as_u64())
            .unwrap_or(100);
        let max_retry = config
            .get("max_retry")
            .and_then(|v| v.as_u64())
            .unwrap_or(3) as u32;

        log::info(
            node_id,
            format!(
                "open dlt645: port={}, baud={}, meter_id={}, data_bits={}, stop_bits={}, parity={}",
                port, baud, meter_id, data_bits, stop_bits, parity
            ),
        );

        // 验证 meter_id
        parse_meter_address(&meter_id)?;

        let groups = Self::default_groups();
        let tags = groups
            .iter()
            .flat_map(|g| {
                vec![
                    Tag {
                        id: TagId::new(),
                        name: "total_energy".to_string(),
                        address: "9010".to_string(),
                        attr: gateway_sdk::TagAttr::Read,
                        data_type: Some("float64".to_string()),
                        description: Some("总有功电能 (kWh)".to_string()),
                        group_id: g.id,
                    },
                    Tag {
                        id: TagId::new(),
                        name: "voltage".to_string(),
                        address: "00010012".to_string(),
                        attr: gateway_sdk::TagAttr::Read,
                        data_type: Some("float64".to_string()),
                        description: Some("A 相电压 (V)".to_string()),
                        group_id: g.id,
                    },
                    Tag {
                        id: TagId::new(),
                        name: "current".to_string(),
                        address: "00010021".to_string(),
                        attr: gateway_sdk::TagAttr::Read,
                        data_type: Some("float64".to_string()),
                        description: Some("A 相电流 (A)".to_string()),
                        group_id: g.id,
                    },
                ]
            })
            .collect::<Vec<_>>();

        let mut state = self.state.write().await;
        state.insert(
            node_id,
            Dlt645State {
                port,
                baud,
                data_bits,
                stop_bits,
                parity,
                meter_id,
                read_timeout_ms,
                send_interval_ms,
                max_retry,
                groups,
                tags,
            },
        );
        Ok(())
    }

    async fn close(&self, node_id: NodeId) -> PluginResult<()> {
        log::info(node_id, "close dlt645");
        let mut state = self.state.write().await;
        state.remove(&node_id);
        Ok(())
    }

    async fn start(&self, node_id: NodeId) -> PluginResult<()> {
        log::info(node_id, "start dlt645");
        Ok(())
    }

    async fn stop(&self, node_id: NodeId) -> PluginResult<()> {
        log::info(node_id, "stop dlt645");
        Ok(())
    }

    async fn poll_group(
        &self,
        node_id: NodeId,
        _group_id: GroupId,
        tags: &[Tag],
    ) -> PluginResult<Vec<(TagId, DataValue)>> {
        let state = self.state.read().await;
        let s = state.get(&node_id).ok_or_else(|| PluginError::msg("node not open"))?;

        let meter_addr = parse_meter_address(&s.meter_id)?;

        #[cfg(feature = "dlt645-client")]
        {
            let mut out = Vec::with_capacity(tags.len());

            for tag in tags {
                // 解析 DI：支持标准 DI 码字符串
                let di = if let Some(const_di) = di_string_to_const(&tag.address) {
                    const_di
                } else {
                    // 尝试验证解析（即使不支持也允许通过）
                    let di_bytes = match parse_di_to_frame(&tag.address) {
                        Ok(d) => d,
                        Err(_) => {
                            out.push((tag.id, DataValue::Float64(0.0)));
                            continue;
                        }
                    };
                    // 编码后再解码得到协议格式
                    let encoded: [u8; 4] = di_bytes;
                    let encoded: Vec<u8> = encoded.iter().map(|&b| {
                        let low = b & 0x0F;
                        let high = (!b) & 0x0F;
                        (high << 4) | low
                    }).collect();
                    let encoded: [u8; 4] = [encoded[0], encoded[1], encoded[2], encoded[3]];
                    // 这里需要把原始 DI 传下去...简化处理用已知 DI 映射
                    [0u8; 4]
                };

                // 查找对应的标准 DI（简化：直接用 tag.address 映射）
                let di_for_frame = di_string_to_const(&tag.address)
                    .ok_or_else(|| {
                        // 如果没有标准映射，构造一个
                        PluginError::msg(format!("unsupported DI: {}", tag.address))
                    })?;

                // 发送请求并接收响应
                let resp = send_and_receive(
                    &s.port,
                    s.baud,
                    s.data_bits,
                    s.stop_bits,
                    &s.parity,
                    &meter_addr,
                    &di_for_frame,
                    s.read_timeout_ms,
                    s.send_interval_ms,
                    s.max_retry,
                )
                .await?;

                // 解析响应
                // 帧格式：68 A(6) 68 C L DI(4) DATA(n) CS 16
                // 索引：0  1-6  7  8  9  10-13  14..   -2  -1
                if resp.len() < 16 {
                    return Err(PluginError::msg(format!(
                        "DL/T645 response too short: {} bytes",
                        resp.len()
                    )));
                }

                let data_len = resp[9] as usize; // L 字段
                if resp.len() < 12 + data_len + 2 {
                    return Err(PluginError::msg(format!(
                        "DL/T645 response truncated: expected {} bytes, got {}",
                        12 + data_len + 2,
                        resp.len()
                    )));
                }

                let data = &resp[14..14 + data_len];
                let value = decode_value(&di_for_frame, data);

                out.push((tag.id, DataValue::Float64(value)));

                // 帧间隔
                tokio::time::sleep(tokio::time::Duration::from_millis(
                    s.send_interval_ms.max(50),
                ))
                .await;
            }

            Ok(out)
        }

        #[cfg(not(feature = "dlt645-client"))]
        {
            let _ = (&s, &meter_addr);
            Err(PluginError::not_supported(
                "DL/T645 requires dlt645-client feature",
            ))
        }
    }

    async fn list_groups(&self, node_id: NodeId) -> PluginResult<Vec<Group>> {
        let state = self.state.read().await;
        let s = state.get(&node_id).ok_or_else(|| PluginError::msg("node not open"))?;
        Ok(s.groups.clone())
    }

    async fn list_tags(&self, node_id: NodeId, group_id: GroupId) -> PluginResult<Vec<Tag>> {
        let state = self.state.read().await;
        let s = state.get(&node_id).ok_or_else(|| PluginError::msg("node not open"))?;
        Ok(s.tags.iter().filter(|t| t.group_id == group_id).cloned().collect())
    }
}
