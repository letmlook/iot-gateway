//! 寄存器与 DataValue 互转、字节序。

use gateway_sdk::types::DataValue;

use crate::address::Endianness;

pub fn regs_to_u32(regs: &[u16], e: &Endianness) -> u32 {
    if regs.len() < 2 {
        return 0;
    }
    let (a, b) = (regs[0], regs[1]);
    let bytes = match e.order32 {
        1 => [(b & 0xff) as u8, (b >> 8) as u8, (a & 0xff) as u8, (a >> 8) as u8],
        2 => [(a >> 8) as u8, (a & 0xff) as u8, (b >> 8) as u8, (b & 0xff) as u8],
        3 => [(b >> 8) as u8, (b & 0xff) as u8, (a >> 8) as u8, (a & 0xff) as u8],
        _ => [(a & 0xff) as u8, (a >> 8) as u8, (b & 0xff) as u8, (b >> 8) as u8],
    };
    u32::from_le_bytes(bytes)
}

/// 按字节序取 64 位：order32 0=LL 1=LB 2=BL 3=BB
pub fn regs_to_u64(regs: &[u16], e: &Endianness) -> u64 {
    if regs.len() < 4 {
        return 0;
    }
    let bytes = match e.order32 {
        1 => [
            (regs[1] & 0xff) as u8, (regs[1] >> 8) as u8, (regs[0] & 0xff) as u8, (regs[0] >> 8) as u8,
            (regs[3] & 0xff) as u8, (regs[3] >> 8) as u8, (regs[2] & 0xff) as u8, (regs[2] >> 8) as u8,
        ],
        2 => [
            (regs[0] >> 8) as u8, (regs[0] & 0xff) as u8, (regs[1] >> 8) as u8, (regs[1] & 0xff) as u8,
            (regs[2] >> 8) as u8, (regs[2] & 0xff) as u8, (regs[3] >> 8) as u8, (regs[3] & 0xff) as u8,
        ],
        3 => [
            (regs[3] >> 8) as u8, (regs[3] & 0xff) as u8, (regs[2] >> 8) as u8, (regs[2] & 0xff) as u8,
            (regs[1] >> 8) as u8, (regs[1] & 0xff) as u8, (regs[0] >> 8) as u8, (regs[0] & 0xff) as u8,
        ],
        _ => [
            (regs[0] & 0xff) as u8, (regs[0] >> 8) as u8, (regs[1] & 0xff) as u8, (regs[1] >> 8) as u8,
            (regs[2] & 0xff) as u8, (regs[2] >> 8) as u8, (regs[3] & 0xff) as u8, (regs[3] >> 8) as u8,
        ],
    };
    u64::from_le_bytes(bytes)
}

pub fn register_to_value_ext(regs: &[u16], data_type: &str, endian: &Endianness, bit_index: Option<u8>) -> DataValue {
    if let Some(bit) = bit_index {
        if let Some(&r) = regs.first() {
            return DataValue::Bool((r >> bit) & 1 != 0);
        }
        return DataValue::Bool(false);
    }
    match data_type {
        "bool" => DataValue::Bool(regs.first().map(|&r| r != 0).unwrap_or(false)),
        "int16" => {
            let r = regs.first().copied().unwrap_or(0);
            DataValue::Int16((if endian.swap16 { r.swap_bytes() } else { r }) as i16)
        }
        "uint16" => {
            let r = regs.first().copied().unwrap_or(0);
            DataValue::UInt16(if endian.swap16 { r.swap_bytes() } else { r })
        }
        "int32" | "uint32" => {
            let v = regs_to_u32(regs, endian);
            if data_type == "int32" { DataValue::Int32(v as i32) } else { DataValue::UInt32(v) }
        }
        "int64" | "uint64" => {
            let v = regs_to_u64(regs, endian);
            if data_type == "int64" { DataValue::Int64(v as i64) } else { DataValue::UInt64(v) }
        }
        "float32" => DataValue::Float32(f32::from_bits(regs_to_u32(regs, endian))),
        "float64" => DataValue::Float64(f64::from_bits(regs_to_u64(regs, endian))),
        "string" => {
            let mut bytes: Vec<u8> = Vec::new();
            for &r in regs {
                bytes.push((r & 0xff) as u8);
                bytes.push((r >> 8) as u8);
            }
            DataValue::String(String::from_utf8_lossy(&bytes).trim_end_matches('\0').to_string())
        }
        "bytes" => {
            let mut bytes: Vec<u8> = Vec::new();
            for &r in regs {
                bytes.push((r & 0xff) as u8);
                bytes.push((r >> 8) as u8);
            }
            DataValue::Bytes(bytes)
        }
        _ => DataValue::UInt16(regs.first().copied().unwrap_or(0)),
    }
}

pub fn value_to_registers(value: &DataValue, data_type: &str) -> Vec<u16> {
    match value {
        DataValue::Int16(v) => vec![*v as u16],
        DataValue::UInt16(v) => vec![*v],
        DataValue::Int32(v) => {
            let b = (*v as u32).to_le_bytes();
            vec![u16::from_le_bytes([b[0], b[1]]), u16::from_le_bytes([b[2], b[3]])]
        }
        DataValue::UInt32(v) => {
            let b = v.to_le_bytes();
            vec![u16::from_le_bytes([b[0], b[1]]), u16::from_le_bytes([b[2], b[3]])]
        }
        DataValue::Int64(v) => {
            let b = (*v as u64).to_le_bytes();
            vec![
                u16::from_le_bytes([b[0], b[1]]), u16::from_le_bytes([b[2], b[3]]),
                u16::from_le_bytes([b[4], b[5]]), u16::from_le_bytes([b[6], b[7]]),
            ]
        }
        DataValue::UInt64(v) => {
            let b = v.to_le_bytes();
            vec![
                u16::from_le_bytes([b[0], b[1]]), u16::from_le_bytes([b[2], b[3]]),
                u16::from_le_bytes([b[4], b[5]]), u16::from_le_bytes([b[6], b[7]]),
            ]
        }
        DataValue::Float32(v) => {
            let b = v.to_bits().to_le_bytes();
            vec![u16::from_le_bytes([b[0], b[1]]), u16::from_le_bytes([b[2], b[3]])]
        }
        DataValue::Float64(v) => {
            let b = v.to_bits().to_le_bytes();
            vec![
                u16::from_le_bytes([b[0], b[1]]), u16::from_le_bytes([b[2], b[3]]),
                u16::from_le_bytes([b[4], b[5]]), u16::from_le_bytes([b[6], b[7]]),
            ]
        }
        DataValue::String(s) => {
            let bytes = s.as_bytes();
            let mut regs = Vec::with_capacity(bytes.len().div_ceil(2));
            for chunk in bytes.chunks(2) {
                regs.push(u16::from_le_bytes([chunk.first().copied().unwrap_or(0), chunk.get(1).copied().unwrap_or(0)]));
            }
            regs
        }
        DataValue::Bytes(b) => {
            let mut regs = Vec::with_capacity(b.len().div_ceil(2));
            for chunk in b.chunks(2) {
                regs.push(u16::from_le_bytes([chunk.first().copied().unwrap_or(0), chunk.get(1).copied().unwrap_or(0)]));
            }
            regs
        }
        DataValue::Bool(v) if data_type == "bool" => vec![if *v { 0xff00 } else { 0 }],
        _ => vec![value.as_u64().map(|u| u as u16).unwrap_or(0)],
    }
}
