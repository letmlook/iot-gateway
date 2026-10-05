//! S7 数据值 ↔ DataValue 转换（纯逻辑）。
//!
//! S7 内部数据类型：
//! - BIT（位）：一个 bit
//! - BYTE / USINT / SINT：1 字节
//! - WORD / UINT / INT：2 字节
//! - DWORD / UDINT / DINT：4 字节
//! - LWORD / ULINT / LINT：8 字节
//! - REAL：IEEE 754 float32
//! - LREAL：IEEE 754 float64
//! - STRING / WSTRING：可变长字符串
//!
//! 字节序：S7 默认大端（most-significant byte first）。
//! 部分设备使用小端（与 PLC 配置有关），`#B` 后缀触发字节交换。

use crate::address::{ParsedS7Address, S7Endian};
use gateway_sdk::types::DataValue;

/// 将 S7 读取的原始字节（缓冲区）按 data_type 转换为 DataValue。
///
/// `raw` 为从 S7 读取的原始字节切片；`data_type` 为点位配置的 data_type 字符串；
/// `addr` 包含位偏移信息用于位访问。
pub fn raw_to_value(raw: &[u8], data_type: &str, addr: &ParsedS7Address) -> DataValue {
    // 位访问
    if let Some(bit) = addr.bit_offset {
        if let Some(&byte) = raw.first() {
            return DataValue::Bool((byte >> bit) & 1 != 0);
        }
        return DataValue::Bool(false);
    }

    match data_type {
        "bool" => {
            if let Some(&byte) = raw.first() {
                DataValue::Bool(byte != 0)
            } else {
                DataValue::Bool(false)
            }
        }
        "int16" => {
            if raw.len() < 2 {
                return DataValue::Int16(0);
            }
            let v = u16::from_be_bytes([raw[0], raw[1]]);
            let v = if addr.endian == S7Endian::Swapped {
                u16::from_le_bytes([raw[0], raw[1]])
            } else {
                v
            };
            DataValue::Int16(v as i16)
        }
        "uint16" => {
            if raw.len() < 2 {
                return DataValue::UInt16(0);
            }
            let v = if addr.endian == S7Endian::Swapped {
                u16::from_le_bytes([raw[0], raw[1]])
            } else {
                u16::from_be_bytes([raw[0], raw[1]])
            };
            DataValue::UInt16(v)
        }
        "int32" => {
            if raw.len() < 4 {
                return DataValue::Int32(0);
            }
            let v = u32::from_be_bytes([raw[0], raw[1], raw[2], raw[3]]);
            let v = if addr.endian == S7Endian::Swapped {
                u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]])
            } else {
                v
            };
            DataValue::Int32(v as i32)
        }
        "uint32" => {
            if raw.len() < 4 {
                return DataValue::UInt32(0);
            }
            let v = if addr.endian == S7Endian::Swapped {
                u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]])
            } else {
                u32::from_be_bytes([raw[0], raw[1], raw[2], raw[3]])
            };
            DataValue::UInt32(v)
        }
        "int64" => {
            if raw.len() < 8 {
                return DataValue::Int64(0);
            }
            let v = u64::from_be_bytes([
                raw[0], raw[1], raw[2], raw[3], raw[4], raw[5], raw[6], raw[7],
            ]);
            let v = if addr.endian == S7Endian::Swapped {
                u64::from_le_bytes([
                    raw[0], raw[1], raw[2], raw[3], raw[4], raw[5], raw[6], raw[7],
                ])
            } else {
                v
            };
            DataValue::Int64(v as i64)
        }
        "uint64" => {
            if raw.len() < 8 {
                return DataValue::UInt64(0);
            }
            let v = if addr.endian == S7Endian::Swapped {
                u64::from_le_bytes([
                    raw[0], raw[1], raw[2], raw[3], raw[4], raw[5], raw[6], raw[7],
                ])
            } else {
                u64::from_be_bytes([
                    raw[0], raw[1], raw[2], raw[3], raw[4], raw[5], raw[6], raw[7],
                ])
            };
            DataValue::UInt64(v)
        }
        "float32" => {
            if raw.len() < 4 {
                return DataValue::Float32(0.0);
            }
            let bits = u32::from_be_bytes([raw[0], raw[1], raw[2], raw[3]]);
            let bits = if addr.endian == S7Endian::Swapped {
                u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]])
            } else {
                bits
            };
            DataValue::Float32(f32::from_bits(bits))
        }
        "float64" => {
            if raw.len() < 8 {
                return DataValue::Float64(0.0);
            }
            let bits = u64::from_be_bytes([
                raw[0], raw[1], raw[2], raw[3], raw[4], raw[5], raw[6], raw[7],
            ]);
            let bits = if addr.endian == S7Endian::Swapped {
                u64::from_le_bytes([
                    raw[0], raw[1], raw[2], raw[3], raw[4], raw[5], raw[6], raw[7],
                ])
            } else {
                bits
            };
            DataValue::Float64(f64::from_bits(bits))
        }
        "string" => {
            // S7 STRING: 第一个字节是最大长度，第二个是实际长度，后面是字符
            // S7 WSTRING: 类似但宽字符
            if raw.is_empty() {
                return DataValue::String(String::new());
            }
            let max_len = raw[0] as usize;
            let actual_len = if raw.len() > 1 { raw[1] as usize } else { 0 };
            let chars = &raw[2..];
            let actual_len = actual_len.min(max_len).min(chars.len());
            let s = String::from_utf8_lossy(&chars[..actual_len]).to_string();
            DataValue::String(s)
        }
        "bytes" => DataValue::Bytes(raw.to_vec()),
        _ => DataValue::Bytes(raw.to_vec()),
    }
}

/// 将 DataValue 转换为写入 S7 需要的字节向量。
///
/// `data_type` 决定转换格式；`addr` 包含长度信息和字节序。
pub fn value_to_raw(value: &DataValue, data_type: &str, addr: &ParsedS7Address) -> Vec<u8> {
    // 位写入
    if let Some(bit) = addr.bit_offset {
        let bit_val = value.as_bool().unwrap_or(false);
        // 返回单字节，调用方需要用 mask/set 方式写入
        return vec![if bit_val { 1 << bit } else { 0 }];
    }

    match (value, data_type) {
        (DataValue::Bool(b), "bool") => vec![if *b { 1u8 } else { 0u8 }],
        (DataValue::Int16(v), "int16") => {
            let bytes = if addr.endian == S7Endian::Swapped {
                (*v as u16).to_le_bytes()
            } else {
                (*v as u16).to_be_bytes()
            };
            bytes.to_vec()
        }
        (DataValue::UInt16(v), "uint16") => {
            let bytes = if addr.endian == S7Endian::Swapped {
                v.to_le_bytes()
            } else {
                v.to_be_bytes()
            };
            bytes.to_vec()
        }
        (DataValue::Int32(v), "int32") => {
            let bytes = if addr.endian == S7Endian::Swapped {
                (*v as u32).to_le_bytes()
            } else {
                (*v as u32).to_be_bytes()
            };
            bytes.to_vec()
        }
        (DataValue::UInt32(v), "uint32") => {
            let bytes = if addr.endian == S7Endian::Swapped {
                v.to_le_bytes()
            } else {
                v.to_be_bytes()
            };
            bytes.to_vec()
        }
        (DataValue::Int64(v), "int64") => {
            let bytes = if addr.endian == S7Endian::Swapped {
                (*v as u64).to_le_bytes()
            } else {
                (*v as u64).to_be_bytes()
            };
            bytes.to_vec()
        }
        (DataValue::UInt64(v), "uint64") => {
            let bytes = if addr.endian == S7Endian::Swapped {
                v.to_le_bytes()
            } else {
                v.to_be_bytes()
            };
            bytes.to_vec()
        }
        (DataValue::Float32(v), "float32") => {
            let bits = v.to_bits();
            if addr.endian == S7Endian::Swapped {
                bits.to_le_bytes().to_vec()
            } else {
                bits.to_be_bytes().to_vec()
            }
        }
        (DataValue::Float64(v), "float64") => {
            let bits = v.to_bits();
            if addr.endian == S7Endian::Swapped {
                bits.to_le_bytes().to_vec()
            } else {
                bits.to_be_bytes().to_vec()
            }
        }
        (DataValue::String(s), "string") => {
            // S7 STRING 格式: [max_len, actual_len, chars...]
            let bytes = s.as_bytes();
            let max_len = addr.count.max(1) as usize;
            let actual_len = bytes.len().min(max_len);
            let mut result = Vec::with_capacity(2 + max_len);
            result.push(max_len as u8);
            result.push(actual_len as u8);
            result.extend_from_slice(&bytes[..actual_len]);
            // 填充剩余空间（如果需要固定长度）
            while result.len() < 2 + max_len {
                result.push(0);
            }
            result
        }
        (DataValue::Bytes(b), "bytes") => {
            // 变长字节
            b.clone()
        }
        // 兼容其他类型（按最接近的类型处理）
        _ => {
            if let Some(u) = value.as_u64() {
                if addr.count == 2 {
                    let bytes = if addr.endian == S7Endian::Swapped {
                        (u as u16).to_le_bytes()
                    } else {
                        (u as u16).to_be_bytes()
                    };
                    return bytes.to_vec();
                } else if addr.count == 4 {
                    let bytes = if addr.endian == S7Endian::Swapped {
                        (u as u32).to_le_bytes()
                    } else {
                        (u as u32).to_be_bytes()
                    };
                    return bytes.to_vec();
                }
            }
            vec![0]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_int16_big_endian() {
        let addr = ParsedS7Address {
            db_number: Some(1),
            byte_offset: 0,
            bit_offset: None,
            count: 2,
            endian: S7Endian::Big,
            is_string: false,
        };
        let raw = [0x12u8, 0x34];
        let v = raw_to_value(&raw, "int16", &addr);
        assert_eq!(v, DataValue::Int16(0x1234_i16));
    }

    #[test]
    fn test_int16_swapped() {
        let addr = ParsedS7Address {
            db_number: Some(1),
            byte_offset: 0,
            bit_offset: None,
            count: 2,
            endian: S7Endian::Swapped,
            is_string: false,
        };
        let raw = [0x12u8, 0x34];
        let v = raw_to_value(&raw, "int16", &addr);
        assert_eq!(v, DataValue::Int16(0x3412_i16));
    }

    #[test]
    fn test_float32() {
        let addr = ParsedS7Address {
            db_number: Some(1),
            byte_offset: 0,
            bit_offset: None,
            count: 4,
            endian: S7Endian::Big,
            is_string: false,
        };
        let raw = 1.23f32.to_bits().to_be_bytes();
        let v = raw_to_value(&raw, "float32", &addr);
        assert_eq!(v, DataValue::Float32(1.23));
    }

    #[test]
    fn test_bit_access() {
        let addr = ParsedS7Address {
            db_number: Some(1),
            byte_offset: 0,
            bit_offset: Some(3),
            count: 1,
            endian: S7Endian::Big,
            is_string: false,
        };
        let raw = [0b00001000u8];
        let v = raw_to_value(&raw, "bool", &addr);
        assert_eq!(v, DataValue::Bool(true));
    }

    #[test]
    fn test_value_to_raw_int16() {
        let addr = ParsedS7Address {
            db_number: Some(1),
            byte_offset: 0,
            bit_offset: None,
            count: 2,
            endian: S7Endian::Big,
            is_string: false,
        };
        let raw = value_to_raw(&DataValue::Int16(0x1234), "int16", &addr);
        assert_eq!(raw, vec![0x12, 0x34]);
    }

    #[test]
    fn test_value_to_raw_float32() {
        let addr = ParsedS7Address {
            db_number: Some(1),
            byte_offset: 0,
            bit_offset: None,
            count: 4,
            endian: S7Endian::Big,
            is_string: false,
        };
        let raw = value_to_raw(&DataValue::Float32(1.5), "float32", &addr);
        assert_eq!(raw, 1.5f32.to_bits().to_be_bytes().to_vec());
    }

    #[test]
    fn test_value_to_raw_bool() {
        let addr = ParsedS7Address {
            db_number: Some(1),
            byte_offset: 0,
            bit_offset: Some(2),
            count: 1,
            endian: S7Endian::Big,
            is_string: false,
        };
        let raw = value_to_raw(&DataValue::Bool(true), "bool", &addr);
        assert_eq!(raw, vec![0b00000100]);
    }
}
