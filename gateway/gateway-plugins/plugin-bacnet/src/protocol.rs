//! BACnet/IP 协议编解码（BVLC / NPDU / APDU），零第三方依赖。
//!
//! 只实现本插件用到的子集（ASHRAE 135）：
//! - BVLC：Original-Unicast-NPDU（0x0A）/ Original-Broadcast-NPDU（0x0B）的编与解，
//!   对 Forwarded-NPDU（0x04）等仅做头剥离；
//! - NPDU：版本 1；请求带 control=0x04（expecting-reply），解析时按控制位跳过 DADR/SADR；
//! - APDU：Confirmed-Request（ReadProperty=12 / WriteProperty=15）、Unconfirmed-Request
//!   （Who-Is=8 / I-Am=0），ComplexACK / SimpleACK / Error / Reject / Abort 的识别；
//! - 应用标签编码：Real(4) / Enumerated(9) / Unsigned(2) / Integer(3) / Boolean(1) /
//!   Double(5) / OctetString(6) / CharacterString(7) / BitString(8) / Null(0) / ObjectIdentifier(12)。
//!
//! 字节序：所有多字节整数均为大端（BACnet 规定）。

// ---------------------------------------------------------------------------
// 常量
// ---------------------------------------------------------------------------

/// BVLC 类型：BACnet/IP
pub const BVLC_TYPE_BACNET_IP: u8 = 0x81;
/// BVLC 功能码：Original-Unicast-NPDU
pub const BVLC_ORIGINAL_UNICAST: u8 = 0x0A;
/// BVLC 功能码：Original-Broadcast-NPDU
pub const BVLC_ORIGINAL_BROADCAST: u8 = 0x0B;

/// APDU 类型 + 控制位（首字节）
pub const APDU_CONFIRMED_REQ: u8 = 0x00;
pub const APDU_UNCONFIRMED_REQ: u8 = 0x10;
pub const APDU_SIMPLE_ACK: u8 = 0x06;
pub const APDU_COMPLEX_ACK: u8 = 0x02;
pub const APDU_ERROR: u8 = 0x14;
pub const APDU_REJECT: u8 = 0x18;
pub const APDU_ABORT: u8 = 0x24;

/// 服务选择
pub const SERVICE_I_AM: u8 = 0;
pub const SERVICE_WHO_IS: u8 = 8;
pub const SERVICE_READ_PROPERTY: u8 = 12;
pub const SERVICE_WRITE_PROPERTY: u8 = 15;

/// 属性标识符（本插件支持的子集）
pub const PROP_OBJECT_NAME: u8 = 77;
pub const PROP_PRESENT_VALUE: u8 = 85;
pub const PROP_UNITS: u8 = 117;
pub const PROP_STATUS_FLAGS: u8 = 111;
pub const PROP_OUT_OF_SERVICE: u8 = 81;

/// 对象类型：device（I-Am 解析用）
pub const OBJECT_TYPE_DEVICE: u16 = 8;

// ---------------------------------------------------------------------------
// 标签字节
// ---------------------------------------------------------------------------

/// 应用标签号
pub const TAG_NULL: u8 = 0;
pub const TAG_BOOLEAN: u8 = 1;
pub const TAG_UNSIGNED: u8 = 2;
pub const TAG_INTEGER: u8 = 3;
pub const TAG_REAL: u8 = 4;
pub const TAG_DOUBLE: u8 = 5;
pub const TAG_OCTET_STRING: u8 = 6;
pub const TAG_CHARACTER_STRING: u8 = 7;
pub const TAG_BIT_STRING: u8 = 8;
pub const TAG_ENUMERATED: u8 = 9;
pub const TAG_OBJECT_IDENTIFIER: u8 = 12;

/// 组装应用标签首字节（class=0 application，长度 ≤4 直接编入低 3 位）
#[inline]
pub fn app_tag(tag: u8, len: u8) -> u8 {
    (tag << 4) | (len & 0x07)
}

/// 组装上下文标签首字节（class=1 context，class 位在 bit3）
#[inline]
pub fn ctx_tag(tag: u8, len: u8) -> u8 {
    0x08 | (tag << 4) | (len & 0x07)
}

/// 开标签（context class，长度域 6）
#[inline]
pub fn ctx_opening(tag: u8) -> u8 {
    0x08 | (tag << 4) | 0x06
}

/// 闭标签（context class，长度域 7）
#[inline]
pub fn ctx_closing(tag: u8) -> u8 {
    0x08 | (tag << 4) | 0x07
}

// ---------------------------------------------------------------------------
// BVLC / NPDU
// ---------------------------------------------------------------------------

/// 把 NPDU+APDU 载荷包成完整 BACnet/IP 帧
pub fn bvlc_wrap(payload: &[u8], broadcast: bool) -> Vec<u8> {
    let func = if broadcast {
        BVLC_ORIGINAL_BROADCAST
    } else {
        BVLC_ORIGINAL_UNICAST
    };
    let len = (payload.len() + 4) as u16;
    let mut out = Vec::with_capacity(len as usize);
    out.push(BVLC_TYPE_BACNET_IP);
    out.push(func);
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(payload);
    out
}

/// 剥掉 BVLC 头，返回 NPDU 起始切片。函数码不是本插件关心的四类时返回 None。
pub fn bvlc_payload(buf: &[u8]) -> Option<&[u8]> {
    if buf.len() < 4 || buf[0] != BVLC_TYPE_BACNET_IP {
        return None;
    }
    let len = u16::from_be_bytes([buf[2], buf[3]]) as usize;
    if len < 4 || buf.len() < len {
        return None;
    }
    match buf[1] {
        BVLC_ORIGINAL_UNICAST | BVLC_ORIGINAL_BROADCAST => Some(&buf[4..len]),
        // Forwarded-NPDU（BBMD）：4 字节原始发送者地址后跟 NPDU
        0x04 => Some(&buf[8..len]),
        _ => None,
    }
}

/// 包一层本地 NPDU：版本 1；请求 expecting-reply=1，响应为 0
pub fn npdu_wrap(apdu: &[u8], expecting_reply: bool) -> Vec<u8> {
    let control = if expecting_reply { 0x04 } else { 0x00 };
    let mut out = Vec::with_capacity(apdu.len() + 2);
    out.push(0x01);
    out.push(control);
    out.extend_from_slice(apdu);
    out
}

/// 按 NPCI 控制位跳过路由字段，返回 APDU 起始切片
pub fn npdu_payload(buf: &[u8]) -> Option<&[u8]> {
    if buf.len() < 2 || buf[0] != 0x01 {
        return None;
    }
    let control = buf[1];
    let mut idx = 2usize;
    if control & 0x20 != 0 {
        // DADR：长度字节 + 地址
        let n = *buf.get(idx)? as usize;
        idx += 1 + n;
    }
    if control & 0x08 != 0 {
        let n = *buf.get(idx)? as usize;
        idx += 1 + n;
    }
    // 控制位 0x80 = 网络层报文（无 APDU），本插件不处理
    if control & 0x80 != 0 {
        return None;
    }
    buf.get(idx..)
}

// ---------------------------------------------------------------------------
// APDU：请求
// ---------------------------------------------------------------------------

/// 编码 ReadProperty-Request（Confirmed，服务 12）：
/// [0x00, 0x05, invoke, 12, C4 对象, 19 属性]
pub fn encode_read_property(invoke_id: u8, obj_type: u16, instance: u32, property: u8) -> Vec<u8> {
    let mut apdu = Vec::with_capacity(10);
    apdu.push(APDU_CONFIRMED_REQ);
    apdu.push(0x05); // max segments 未指定 + max APDU 1476
    apdu.push(invoke_id);
    apdu.push(SERVICE_READ_PROPERTY);
    push_object_identifier(&mut apdu, obj_type, instance);
    apdu.push(ctx_tag(1, 1));
    apdu.push(property);
    apdu
}

/// 编码 WriteProperty-Request（Confirmed，服务 15）：
/// [0x00, 0x05, invoke, 15, C4 对象, 19 属性, 3E 值开标签, 应用标签值, 3F 闭标签, 可选优先级]
pub fn encode_write_property(
    invoke_id: u8,
    obj_type: u16,
    instance: u32,
    property: u8,
    value: &super::value::BacnetValue,
    priority: Option<u8>,
) -> Vec<u8> {
    let mut apdu = vec![APDU_CONFIRMED_REQ, 0x05, invoke_id, SERVICE_WRITE_PROPERTY];
    push_object_identifier(&mut apdu, obj_type, instance);
    apdu.push(ctx_tag(1, 1));
    apdu.push(property);
    apdu.push(ctx_opening(3));
    value.encode_application(&mut apdu);
    apdu.push(ctx_closing(3));
    if let Some(p) = priority {
        if (1..=16).contains(&p) {
            apdu.push(ctx_tag(4, 1));
            apdu.push(p);
        }
    }
    apdu
}

/// 编码 Who-Is（Unconfirmed，服务 8，空请求体 = 全范围）
pub fn encode_who_is() -> Vec<u8> {
    vec![APDU_UNCONFIRMED_REQ, SERVICE_WHO_IS]
}

/// 追加 4 字节对象标识符（应用标签 12）：type(10bit)<<22 | instance(22bit)
pub fn push_object_identifier(buf: &mut Vec<u8>, obj_type: u16, instance: u32) {
    let v = ((obj_type as u32) << 22) | (instance & 0x003F_FFFF);
    buf.push(app_tag(TAG_OBJECT_IDENTIFIER, 4));
    buf.push((v >> 24) as u8);
    buf.push((v >> 16) as u8);
    buf.push((v >> 8) as u8);
    buf.push(v as u8);
}

// ---------------------------------------------------------------------------
// APDU：响应解析
// ---------------------------------------------------------------------------

/// I-Am 解析结果：设备实例号
pub fn decode_i_am(apdu: &[u8]) -> Option<u32> {
    if apdu.len() < 2 || apdu[0] != APDU_UNCONFIRMED_REQ || apdu[1] != SERVICE_I_AM {
        return None;
    }
    // 找应用标签 12（0xC4），对象类型须为 device(8)
    let mut i = 2usize;
    while i < apdu.len() {
        let tag = apdu[i];
        if tag == app_tag(TAG_OBJECT_IDENTIFIER, 4) {
            let b = apdu.get(i + 1..i + 5)?;
            let v = u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
            let obj_type = (v >> 22) as u16;
            let instance = v & 0x003F_FFFF;
            if obj_type == OBJECT_TYPE_DEVICE {
                return Some(instance);
            }
            return None;
        }
        i += 1;
    }
    None
}

/// 复杂 ACK 的一次解析结果
#[derive(Debug, Clone, PartialEq)]
pub struct ReadPropertyAck {
    pub invoke_id: u8,
    pub value: super::value::BacnetValue,
}

/// 解析 ReadProperty-ComplexACK：[0x02, invoke, 12, C4 对象, 21/22 属性, 3E 值, 3F]
/// 解析策略：校验头与属性上下文标签，取 3E 开标签后的第一个应用标签值。
pub fn decode_read_property_ack(apdu: &[u8]) -> Option<Result<ReadPropertyAck, String>> {
    if apdu.len() < 4 || apdu[0] != APDU_COMPLEX_ACK {
        return None;
    }
    if apdu[2] != SERVICE_READ_PROPERTY {
        return None;
    }
    let invoke_id = apdu[1];
    // 属性标识符：context tag 1，长度 1..4（0x18 = class 位 | tag 1 左移 4 位）
    let mut i = 3usize;
    if apdu.get(i) != Some(&app_tag(TAG_OBJECT_IDENTIFIER, 4)) {
        return Some(Err("ack: object identifier tag expected".into()));
    }
    i += 5;
    let prop_tag = *apdu.get(i)?;
    if prop_tag & 0xF8 != 0x18 || (prop_tag & 0x07) == 0 || (prop_tag & 0x07) > 4 {
        return Some(Err("ack: property tag expected".into()));
    }
    i += 1 + (prop_tag & 0x07) as usize;
    // 开标签 3
    if apdu.get(i) != Some(&ctx_opening(3)) {
        return Some(Err("ack: opening tag 3 expected".into()));
    }
    i += 1;
    // 应用标签值
    let (value, _consumed) = super::value::BacnetValue::decode_application(&apdu[i..])?;
    Some(Ok(ReadPropertyAck { invoke_id, value }))
}

/// 识别并解释错误类响应（Error / Reject / Abort），返回可读消息。
/// 不是错误类响应时返回 None。
pub fn decode_error(apdu: &[u8]) -> Option<String> {
    match *apdu.first()? {
        APDU_ERROR => {
            if apdu.len() >= 5 {
                Some(format!("bacnet error: class={} code={}", apdu[3], apdu[4]))
            } else {
                Some("bacnet error (malformed)".into())
            }
        }
        APDU_REJECT => Some(format!(
            "bacnet reject: reason={}",
            apdu.get(2).copied().unwrap_or(0)
        )),
        APDU_ABORT => Some(format!(
            "bacnet abort: reason={}",
            apdu.get(2).copied().unwrap_or(0)
        )),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// 测试：黄金字节 + 往返
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::BacnetValue;

    /// BVLC 头：类型 0x81 + 功能码 + 大端长度（含 4 字节头自身）
    #[test]
    fn bvlc_wrap_golden() {
        let payload = [0x01, 0x04, 0xAA];
        let frame = bvlc_wrap(&payload, true);
        assert_eq!(frame, vec![0x81, 0x0B, 0x00, 0x07, 0x01, 0x04, 0xAA]);
        assert_eq!(bvlc_payload(&frame).unwrap(), &payload);
    }

    #[test]
    fn bvlc_payload_rejects_truncated() {
        assert!(bvlc_payload(&[0x81, 0x0A, 0x00]).is_none());
        assert!(bvlc_payload(&[0x81, 0x0A, 0x00, 0x20, 0x01]).is_none()); // 长度越界
        assert!(bvlc_payload(&[0x82, 0x0A, 0x00, 0x04]).is_none()); // 非 BACnet/IP
    }

    /// NPCI：请求 control=0x04（expecting-reply），无路由字段
    #[test]
    fn npdu_wrap_golden() {
        let apdu = [0x10, 0x08];
        assert_eq!(npdu_wrap(&apdu, true), vec![0x01, 0x04, 0x10, 0x08]);
        assert_eq!(npdu_payload(&[0x01, 0x04, 0x10, 0x08]).unwrap(), &apdu);
    }

    /// NPCI 携带 DADR/SADR 时按控制位跳过
    #[test]
    fn npdu_payload_skips_routing_fields() {
        // control 0x28 = DADR(0x20) + SADR(0x08)
        let buf = [0x01, 0x28, 0x01, 0xFF, 0x01, 0xAA, 0x10, 0x00];
        assert_eq!(npdu_payload(&buf).unwrap(), &[0x10, 0x00]);
    }

    /// ReadProperty 请求黄金字节：读 AI:1 的 present-value
    /// （对照典型抓包：00 05 01 0c c4 00 00 00 01 19 55）
    #[test]
    fn read_property_golden() {
        let apdu = encode_read_property(1, 0, 1, PROP_PRESENT_VALUE);
        assert_eq!(
            apdu,
            vec![0x00, 0x05, 0x01, 0x0C, 0xC4, 0x00, 0x00, 0x00, 0x01, 0x19, 0x55]
        );
    }

    /// 对象标识符编码：type=8(device) instance=11 → C4 02 00 00 0B
    #[test]
    fn object_identifier_golden() {
        let mut buf = Vec::new();
        push_object_identifier(&mut buf, OBJECT_TYPE_DEVICE, 11);
        assert_eq!(buf, vec![0xC4, 0x02, 0x00, 0x00, 0x0B]);
    }

    /// WriteProperty 黄金字节：BO:3 写 active（enumerated 2），优先级 16
    #[test]
    fn write_property_golden() {
        let apdu = encode_write_property(
            7,
            1, // binary-output
            3,
            PROP_PRESENT_VALUE,
            &BacnetValue::Enumerated(2),
            Some(16),
        );
        assert_eq!(
            apdu,
            vec![
                0x00, 0x05, 0x07, 0x0F, 0xC4, 0x00, 0x40, 0x00, 0x03, 0x19, 0x55, 0x3E, 0x91, 0x02,
                0x3F, 0x49, 0x10
            ]
        );
    }

    /// I-Am 解析：10 00 C4 02 00 00 0B 22 05 91 00 91 1B
    #[test]
    fn i_am_golden() {
        let apdu = vec![
            0x10, 0x00, 0xC4, 0x02, 0x00, 0x00, 0x0B, 0x22, 0x05, 0x91, 0x00, 0x91, 0x1B,
        ];
        assert_eq!(decode_i_am(&apdu), Some(11));
        // 对象类型不是 device → None
        let apdu = vec![0x10, 0x00, 0xC4, 0x00, 0x00, 0x00, 0x0B];
        assert_eq!(decode_i_am(&apdu), None);
    }

    /// ReadProperty-ComplexACK 解析：AI:1 present-value = 42.5（real，0x422A0000）
    #[test]
    fn read_ack_golden() {
        let apdu = vec![
            0x02, 0x09, 0x0C, 0xC4, 0x00, 0x00, 0x00, 0x01, 0x19, 0x55, 0x3E, 0x44, 0x42, 0x2A,
            0x00, 0x00, 0x3F,
        ];
        let ack = decode_read_property_ack(&apdu).unwrap().unwrap();
        assert_eq!(ack.invoke_id, 9);
        assert_eq!(ack.value, BacnetValue::Real(42.5));
    }

    #[test]
    fn error_decode() {
        assert_eq!(
            decode_error(&[0x14, 0x01, 0x0C, 0x01, 0x1F]),
            Some("bacnet error: class=1 code=31".into())
        );
        assert_eq!(decode_error(&[0x02, 0x01, 0x0C]), None);
    }
}
