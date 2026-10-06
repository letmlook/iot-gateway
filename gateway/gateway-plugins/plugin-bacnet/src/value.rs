//! BACnet 应用标签值 ↔ DataValue 转换（纯逻辑，不触网）。
//!
//! 标签字节布局（ASHRAE 135 §20.2.1）：高 4 位 = 标签号、bit3 = 类别
//! （0 application / 1 context）、低 3 位 = 长度域。
//! 长度域：0–4 直接长度；5 = 扩展（下一字节为长度）；6 = 开标签；7 = 闭标签。
//! Boolean 的真值放在长度域 bit0（0x10/0x11）。

use gateway_sdk::types::DataValue;

use crate::address::BacnetObjectType;
use crate::protocol::{
    app_tag, TAG_BIT_STRING, TAG_BOOLEAN, TAG_CHARACTER_STRING, TAG_DOUBLE, TAG_ENUMERATED,
    TAG_INTEGER, TAG_NULL, TAG_OCTET_STRING, TAG_REAL, TAG_UNSIGNED,
};

/// BACnet 抽象值：应用标签级别的中间表示
#[derive(Debug, Clone, PartialEq)]
pub enum BacnetValue {
    Null,
    Bool(bool),
    Unsigned(u64),
    Integer(i32),
    Real(f32),
    Double(f64),
    OctetString(Vec<u8>),
    CharacterString(String),
    BitString { bytes: Vec<u8>, unused_bits: u8 },
    Enumerated(u32),
}

impl BacnetValue {
    /// 编码为应用标签序列，追加到 buf
    pub fn encode_application(&self, buf: &mut Vec<u8>) {
        match self {
            BacnetValue::Null => buf.push(app_tag(TAG_NULL, 0)),
            BacnetValue::Bool(b) => buf.push(app_tag(TAG_BOOLEAN, 0) | if *b { 1 } else { 0 }),
            BacnetValue::Unsigned(v) => {
                let bytes = minimal_be_u64(*v);
                buf.push(app_tag(TAG_UNSIGNED, bytes.len() as u8));
                buf.extend_from_slice(&bytes);
            }
            BacnetValue::Integer(v) => {
                let bytes = minimal_be_i32(*v);
                buf.push(app_tag(TAG_INTEGER, bytes.len() as u8));
                buf.extend_from_slice(&bytes);
            }
            BacnetValue::Real(v) => {
                buf.push(app_tag(TAG_REAL, 4));
                buf.extend_from_slice(&v.to_be_bytes());
            }
            BacnetValue::Double(v) => {
                buf.push(app_tag(TAG_DOUBLE, 8));
                buf.extend_from_slice(&v.to_be_bytes());
            }
            BacnetValue::OctetString(b) => push_variable(buf, TAG_OCTET_STRING, b),
            BacnetValue::CharacterString(s) => {
                let mut enc = Vec::with_capacity(s.len() + 1);
                enc.push(0x00); // 编码字节：UTF-8
                enc.extend_from_slice(s.as_bytes());
                push_variable(buf, TAG_CHARACTER_STRING, &enc);
            }
            BacnetValue::BitString { bytes, unused_bits } => {
                let mut enc = Vec::with_capacity(bytes.len() + 1);
                enc.push(*unused_bits);
                enc.extend_from_slice(bytes);
                push_variable(buf, TAG_BIT_STRING, &enc);
            }
            BacnetValue::Enumerated(v) => {
                let bytes = minimal_be_u64(*v as u64);
                buf.push(app_tag(TAG_ENUMERATED, bytes.len() as u8));
                buf.extend_from_slice(&bytes);
            }
        }
    }

    /// 从缓冲解码一个应用标签值，返回 (值, 消费字节数)
    pub fn decode_application(buf: &[u8]) -> Option<(BacnetValue, usize)> {
        let first = *buf.first()?;
        let tag = first >> 4;
        let class_ctx = first & 0x08 != 0;
        let len3 = first & 0x07;
        if class_ctx {
            return None; // 本函数只解应用标签
        }
        match (tag, len3) {
            (TAG_NULL, 0) => Some((BacnetValue::Null, 1)),
            // Boolean：长度域 bit0 即真值
            (TAG_BOOLEAN, l) if l <= 1 => Some((BacnetValue::Bool(l == 1), 1)),
            (TAG_REAL, 4) => {
                let b = buf.get(1..5)?;
                Some((
                    BacnetValue::Real(f32::from_be_bytes([b[0], b[1], b[2], b[3]])),
                    5,
                ))
            }
            (TAG_DOUBLE, 8) => {
                let b = buf.get(1..9)?;
                Some((
                    BacnetValue::Double(f64::from_be_bytes([
                        b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
                    ])),
                    9,
                ))
            }
            (TAG_UNSIGNED, n) | (TAG_INTEGER, n) | (TAG_ENUMERATED, n) if (1..=4).contains(&n) => {
                let b = buf.get(1..1 + n as usize)?;
                let mut v: u64 = 0;
                for &x in b {
                    v = (v << 8) | x as u64;
                }
                let consumed = 1 + n as usize;
                let val = match tag {
                    TAG_UNSIGNED => BacnetValue::Unsigned(v),
                    TAG_INTEGER => BacnetValue::Integer(v as i32),
                    _ => BacnetValue::Enumerated(v as u32),
                };
                Some((val, consumed))
            }
            // 变长类型的短形式（长度 ≤4）
            (TAG_OCTET_STRING, n) | (TAG_BIT_STRING, n) | (TAG_CHARACTER_STRING, n)
                if (1..=4).contains(&n) =>
            {
                let data = buf.get(1..1 + n as usize)?;
                let val = match tag {
                    TAG_OCTET_STRING => BacnetValue::OctetString(data.to_vec()),
                    TAG_BIT_STRING => BacnetValue::BitString {
                        unused_bits: 0,
                        bytes: data.to_vec(),
                    },
                    _ => BacnetValue::CharacterString(String::from_utf8_lossy(data).into_owned()),
                };
                Some((val, 1 + n as usize))
            }
            // 变长类型的扩展长度形式（长度域 = 5）
            (TAG_OCTET_STRING, 5) | (TAG_CHARACTER_STRING, 5) | (TAG_BIT_STRING, 5) => {
                let ext = *buf.get(1)? as usize;
                let data = buf.get(2..2 + ext)?;
                let val = match tag {
                    TAG_OCTET_STRING => BacnetValue::OctetString(data.to_vec()),
                    TAG_BIT_STRING => BacnetValue::BitString {
                        unused_bits: data.first().copied().unwrap_or(0),
                        bytes: data[1.min(ext)..].to_vec(),
                    },
                    _ => BacnetValue::CharacterString(
                        String::from_utf8_lossy(&data[1.min(ext)..]).into_owned(),
                    ),
                };
                Some((val, 2 + ext))
            }
            _ => None,
        }
    }

    /// 按对象类型把 BACnet 值转成 DataValue（采集方向）
    pub fn to_data_value(&self, object_type: &BacnetObjectType) -> Option<DataValue> {
        use BacnetObjectType as T;
        let v = match (object_type, self) {
            (
                T::AnalogInput | T::AnalogOutput | T::AnalogValue | T::LargeAnalogValue,
                BacnetValue::Real(f),
            ) => DataValue::Float64(*f as f64),
            (
                T::AnalogInput | T::AnalogOutput | T::AnalogValue | T::LargeAnalogValue,
                BacnetValue::Double(f),
            ) => DataValue::Float64(*f),
            (
                T::AnalogInput | T::AnalogOutput | T::AnalogValue | T::LargeAnalogValue,
                BacnetValue::Unsigned(u),
            ) => DataValue::Float64(*u as f64),
            (T::BinaryInput | T::BinaryOutput | T::BinaryValue, BacnetValue::Enumerated(e)) => {
                DataValue::Bool(*e == 2)
            } // BinaryPV: inactive=1, active=2
            (T::BinaryInput | T::BinaryOutput | T::BinaryValue, BacnetValue::Bool(b)) => {
                DataValue::Bool(*b)
            }
            (
                T::MultiStateInput | T::MultiStateOutput | T::MultiStateValue,
                BacnetValue::Unsigned(u),
            ) => DataValue::Int32(*u as i32),
            (
                T::MultiStateInput | T::MultiStateOutput | T::MultiStateValue,
                BacnetValue::Enumerated(e),
            ) => DataValue::Int32(*e as i32),
            (T::IntegerValue, BacnetValue::Integer(i)) => DataValue::Int32(*i),
            (T::IntegerValue, BacnetValue::Unsigned(u)) => DataValue::Int64(*u as i64),
            (T::PositiveIntegerValue, BacnetValue::Unsigned(u)) => DataValue::UInt64(*u),
            (T::CharacterstringValue, BacnetValue::CharacterString(s)) => {
                DataValue::String(s.clone())
            }
            (T::OctetstringValue, BacnetValue::OctetString(b)) => DataValue::Bytes(b.clone()),
            (T::BitstringValue, BacnetValue::BitString { bytes, .. }) => {
                DataValue::Bytes(bytes.clone())
            }
            _ => return None,
        };
        Some(v)
    }

    /// 按对象类型把 DataValue 转成 BACnet 值（写值方向）
    pub fn from_data_value(
        object_type: &BacnetObjectType,
        v: &DataValue,
    ) -> Result<BacnetValue, String> {
        use BacnetObjectType as T;
        match (object_type, v) {
            (
                T::AnalogInput | T::AnalogOutput | T::AnalogValue | T::LargeAnalogValue,
                DataValue::Float64(f),
            ) => Ok(BacnetValue::Real(*f as f32)),
            (
                T::AnalogInput | T::AnalogOutput | T::AnalogValue | T::LargeAnalogValue,
                DataValue::Float32(f),
            ) => Ok(BacnetValue::Real(*f)),
            (
                T::AnalogInput | T::AnalogOutput | T::AnalogValue | T::LargeAnalogValue,
                DataValue::Int32(i),
            ) => Ok(BacnetValue::Real(*i as f32)),
            (
                T::AnalogInput | T::AnalogOutput | T::AnalogValue | T::LargeAnalogValue,
                DataValue::Int16(i),
            ) => Ok(BacnetValue::Real(*i as f32)),
            (T::BinaryInput | T::BinaryOutput | T::BinaryValue, DataValue::Bool(b)) => {
                Ok(BacnetValue::Enumerated(if *b { 2 } else { 1 }))
            }
            (
                T::MultiStateInput | T::MultiStateOutput | T::MultiStateValue,
                DataValue::Int32(i),
            ) => Ok(BacnetValue::Unsigned((*i).max(0) as u64)),
            (T::IntegerValue, DataValue::Int32(i)) => Ok(BacnetValue::Integer(*i)),
            (T::PositiveIntegerValue, DataValue::UInt64(u)) => Ok(BacnetValue::Unsigned(*u)),
            (T::PositiveIntegerValue, DataValue::Int32(i)) if *i >= 0 => {
                Ok(BacnetValue::Unsigned(*i as u64))
            }
            (T::CharacterstringValue, DataValue::String(s)) => {
                Ok(BacnetValue::CharacterString(s.clone()))
            }
            (T::OctetstringValue, DataValue::Bytes(b)) => Ok(BacnetValue::OctetString(b.clone())),
            (T::BitstringValue, DataValue::Bytes(b)) => Ok(BacnetValue::BitString {
                bytes: b.clone(),
                unused_bits: 0,
            }),
            (t, _) => Err(format!(
                "data value {:?} is not writable to BACnet object type {:?}",
                v, t
            )),
        }
    }
}

/// 最小长度大端无符号编码
fn minimal_be_u64(v: u64) -> Vec<u8> {
    if v == 0 {
        return vec![0];
    }
    let be = v.to_be_bytes();
    let first = be.iter().position(|&b| b != 0).unwrap_or(7);
    be[first..].to_vec()
}

/// 最小长度大端有符号编码（保号）
fn minimal_be_i32(v: i32) -> Vec<u8> {
    let be = v.to_be_bytes();
    if v >= 0 {
        let first = be.iter().position(|&b| b != 0).unwrap_or(3);
        be[first..].to_vec()
    } else {
        let first = be.iter().position(|&b| b != 0xFF).unwrap_or(3);
        be[first..].to_vec()
    }
}

/// 变长类型（字符串/字节串/位串）：长度 ≤4 用短形式，否则扩展长度域
fn push_variable(buf: &mut Vec<u8>, tag: u8, data: &[u8]) {
    if data.len() <= 4 {
        buf.push(app_tag(tag, data.len() as u8));
    } else {
        buf.push(app_tag(tag, 5)); // 扩展长度
        buf.push(data.len() as u8);
    }
    buf.extend_from_slice(data);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_roundtrip() {
        let mut buf = Vec::new();
        BacnetValue::Real(42.5).encode_application(&mut buf);
        assert_eq!(buf, vec![0x44, 0x42, 0x2A, 0x00, 0x00]);
        let (v, n) = BacnetValue::decode_application(&buf).unwrap();
        assert_eq!(v, BacnetValue::Real(42.5));
        assert_eq!(n, 5);
    }

    #[test]
    fn boolean_tag_carries_value_in_length_bits() {
        let mut buf = Vec::new();
        BacnetValue::Bool(true).encode_application(&mut buf);
        assert_eq!(buf, vec![0x11]);
        let (v, n) = BacnetValue::decode_application(&[0x11]).unwrap();
        assert_eq!(v, BacnetValue::Bool(true));
        assert_eq!(n, 1);
        let (v, _) = BacnetValue::decode_application(&[0x10]).unwrap();
        assert_eq!(v, BacnetValue::Bool(false));
    }

    #[test]
    fn enumerated_golden() {
        let mut buf = Vec::new();
        BacnetValue::Enumerated(2).encode_application(&mut buf);
        assert_eq!(buf, vec![0x91, 0x02]);
    }

    #[test]
    fn unsigned_minimal_encoding() {
        let mut buf = Vec::new();
        BacnetValue::Unsigned(300).encode_application(&mut buf);
        assert_eq!(buf, vec![0x22, 0x01, 0x2C]); // 2 字节
        let mut buf = Vec::new();
        BacnetValue::Unsigned(0).encode_application(&mut buf);
        assert_eq!(buf, vec![0x21, 0x00]);
    }

    #[test]
    fn character_string_extended_length() {
        let mut buf = Vec::new();
        BacnetValue::CharacterString("hello".into()).encode_application(&mut buf);
        // 0x75（扩展长度）+ 长度 6（编码字节 0x00 + 5 字符）+ 编码字节 + 内容
        assert_eq!(buf[0], 0x75);
        assert_eq!(buf[1], 6);
        assert_eq!(&buf[2..8], &[0x00, b'h', b'e', b'l', b'l', b'o']);
        let (v, n) = BacnetValue::decode_application(&buf).unwrap();
        assert_eq!(v, BacnetValue::CharacterString("hello".into()));
        assert_eq!(n, buf.len());
    }

    #[test]
    fn octet_string_short_form() {
        let mut buf = Vec::new();
        BacnetValue::OctetString(vec![0xDE, 0xAD]).encode_application(&mut buf);
        assert_eq!(buf, vec![0x62, 0xDE, 0xAD]);
        let (v, _) = BacnetValue::decode_application(&buf).unwrap();
        assert_eq!(v, BacnetValue::OctetString(vec![0xDE, 0xAD]));
    }

    /// 二进制对象：Bool(true) → enumerated 2（active）；读取 enumerated 1 → false
    #[test]
    fn binary_mapping_uses_binary_pv_semantics() {
        let v =
            BacnetValue::from_data_value(&BacnetObjectType::BinaryOutput, &DataValue::Bool(true))
                .unwrap();
        assert_eq!(v, BacnetValue::Enumerated(2));
        let read = BacnetValue::Enumerated(1)
            .to_data_value(&BacnetObjectType::BinaryOutput)
            .unwrap();
        assert_eq!(read, DataValue::Bool(false));
    }

    /// 模拟对象读值：unsigned → Int32
    #[test]
    fn multistate_read() {
        let v = BacnetValue::Unsigned(3)
            .to_data_value(&BacnetObjectType::MultiStateValue)
            .unwrap();
        assert_eq!(v, DataValue::Int32(3));
    }

    /// 类型不匹配的写值拒绝
    #[test]
    fn write_type_mismatch_rejected() {
        assert!(BacnetValue::from_data_value(
            &BacnetObjectType::AnalogInput,
            &DataValue::Bool(true)
        )
        .is_err());
    }
}
