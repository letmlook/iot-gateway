//! S7 地址语法解析（纯逻辑，零网络依赖）。
//!
//! 地址格式（对齐 snap7-client 的区域标识）：
//! - 位：`DBX b.x` / `Mx.x` / `Ix.x` / `Qx.x`
//! - 字节：`DBB b` / `MB b` / `IB b` / `QB b`
//! - 字：`DBW w` / `MW w` / `IW w` / `QW w`
//! - 双字：`DBD d` / `MD d` / `ID d` / `QD d`
//! - 字符串：`DBb.Spos.len`（DB 号.起始字节.最大长度）
//! - 变长字节：`DBb.DBBn.N`（DB 号.起始字节.N 长度）
//!
//! `#F` 后缀 → float32；`#D` 后缀 → float64；`#B` → 字节序交换（16bit）
//! `#L` → 大端（默认），与 modbus-tcp 风格保持一致。

/// 解析后的 S7 地址
#[derive(Debug, Clone)]
pub struct ParsedS7Address {
    /// 数据块号（仅 DataBlock 区域）
    pub db_number: Option<u16>,
    /// 起始字节偏移
    pub byte_offset: u32,
    /// 位偏移（0-7），为 None 时按字/双字/字节访问
    pub bit_offset: Option<u8>,
    /// 数据长度（字节）；字符串/bytes 时由地址本身决定
    pub count: u32,
    /// 字节序修饰符
    pub endian: S7Endian,
    /// 是否为字符串（特殊处理）
    #[allow(dead_code)]
    pub is_string: bool,
}

/// 字节序修饰（仅对多字节类型有意义）
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum S7Endian {
    /// 大端（默认，S7 网络序）
    #[default]
    Big,
    /// 小端（部分设备使用）
    Little,
    /// 交换字节序（DBW#B 等场景）
    Swapped,
}

/// 纯解析：把地址字符串转为 ParsedS7Address
pub fn parse_address_full(addr: &str) -> Option<ParsedS7Address> {
    let addr = addr.trim();
    if addr.is_empty() {
        return None;
    }

    // 先处理 # 后缀（字节序/类型修饰符）
    let mut endian = S7Endian::default();
    let mut addr_part = addr;
    if let Some(excl) = addr.find('#') {
        let (pre, suffix) = addr.split_at(excl);
        addr_part = pre;
        let suffix = suffix.trim_start_matches('#').to_uppercase();
        match suffix.as_str() {
            "F" => {} // float32 hint，不影响解析
            "D" => {} // float64 hint，不影响解析
            "B" => endian = S7Endian::Swapped,
            "L" => endian = S7Endian::Big,
            "LE" => endian = S7Endian::Little,
            _ => {}
        }
    }

    let parts: Vec<&str> = addr_part.split('.').map(|s| s.trim()).collect();
    if parts.is_empty() {
        return None;
    }

    let first = parts[0].to_uppercase();

    // ---- Merker: M0.0 / MB10 / MW12 / MD16 / MX0 ----
    if first.starts_with("MB")
        || first.starts_with("MW")
        || first.starts_with("MD")
        || first.starts_with("MX")
    {
        // Two-letter prefix: MB/MW/MD/MX followed by number
        let prefix_len = 2;
        let num_str = &first[prefix_len..];
        let byte: u32 = num_str.parse().ok()?;
        return Some(ParsedS7Address {
            db_number: None,
            byte_offset: byte,
            bit_offset: None,
            count: if first.starts_with("MD") {
                4
            } else if first.starts_with("MW") {
                2
            } else {
                1
            },
            endian,
            is_string: false,
        });
    }
    if first.starts_with('M') {
        return parse_merker(&first, &parts[1..], endian);
    }

    // ---- 输入: I0.0 / IB2 / IW4 / ID6 ----
    if first.starts_with("IB") || first.starts_with("IW") || first.starts_with("ID") {
        let prefix_len = 2;
        let num_str = &first[prefix_len..];
        let byte: u32 = num_str.parse().ok()?;
        return Some(ParsedS7Address {
            db_number: None,
            byte_offset: byte,
            bit_offset: None,
            count: if first.starts_with("ID") {
                4
            } else if first.starts_with("IW") {
                2
            } else {
                1
            },
            endian,
            is_string: false,
        });
    }
    if first.starts_with('I') || first == "E" {
        return parse_simple_area("I", &first, &parts[1..], endian);
    }

    // ---- 输出: Q0.0 / QB2 / QW4 / QD6 ----
    if first.starts_with("QB") || first.starts_with("QW") || first.starts_with("QD") {
        let prefix_len = 2;
        let num_str = &first[prefix_len..];
        let byte: u32 = num_str.parse().ok()?;
        return Some(ParsedS7Address {
            db_number: None,
            byte_offset: byte,
            bit_offset: None,
            count: if first.starts_with("QD") {
                4
            } else if first.starts_with("QW") {
                2
            } else {
                1
            },
            endian,
            is_string: false,
        });
    }
    if first.starts_with('Q') || first == "A" {
        return parse_simple_area("Q", &first, &parts[1..], endian);
    }

    // ---- 数据块: DB1.DBX0.1 / DB1.DBW10 / DB1.DBD12 / DB1.S20.32 ----
    if first.starts_with("DB") {
        return parse_data_block(&first, &parts[1..], endian);
    }

    // ---- 计数器: C0 ----
    if first.starts_with('C') {
        return parse_counter_or_timer(&first, &parts[1..], endian);
    }

    // ---- 定时器: T0 ----
    if first.starts_with('T') && !first.starts_with("TO") {
        return parse_counter_or_timer(&first, &parts[1..], endian);
    }

    None
}

fn parse_merker(first: &str, rest: &[&str], endian: S7Endian) -> Option<ParsedS7Address> {
    let byte_str = first.trim_start_matches('M');
    let byte: u32 = byte_str.parse().ok()?;
    parse_area_common("M", byte, rest, endian)
}

fn parse_simple_area(
    _area: &str,
    first: &str,
    rest: &[&str],
    endian: S7Endian,
) -> Option<ParsedS7Address> {
    let byte_str = first.trim_start_matches(['I', 'i', 'E', 'Q', 'A', 'q', 'a']);
    let byte: u32 = byte_str.parse().ok()?;
    parse_area_common(_area, byte, rest, endian)
}

fn parse_data_block(first: &str, rest: &[&str], endian: S7Endian) -> Option<ParsedS7Address> {
    // first = "DB1" / "db1"
    let db_str = first[2..].trim_start_matches(['D', 'd', 'B', 'b']);
    let db_number: u16 = db_str.parse().ok()?;

    if rest.is_empty() {
        return None;
    }

    let second = rest[0].to_uppercase();

    // DBX b.x — 位访问
    if let Some(rest_str) = second.strip_prefix("DBX") {
        let byte_str = rest_str.trim_start_matches(['D', 'd', 'B', 'b', 'X', 'x']);
        let byte: u32 = byte_str.parse().ok()?;
        let bit = if let Some(bit_part) = rest.get(1) {
            bit_part.parse().ok().unwrap_or(0)
        } else {
            0
        };
        if bit > 7 {
            return None;
        }
        return Some(ParsedS7Address {
            db_number: Some(db_number),
            byte_offset: byte,
            bit_offset: Some(bit as u8),
            count: 1,
            endian,
            is_string: false,
        });
    }

    // DBB b — 字节访问
    if let Some(rest_str) = second.strip_prefix("DBB") {
        let byte_str = rest_str.trim_start_matches(['D', 'd', 'B', 'b', 'X', 'x']);
        let byte: u32 = byte_str.parse().ok()?;
        return Some(ParsedS7Address {
            db_number: Some(db_number),
            byte_offset: byte,
            bit_offset: None,
            count: 1,
            endian,
            is_string: false,
        });
    }

    // DBW w — 字访问
    if let Some(rest_str) = second.strip_prefix("DBW") {
        let byte_str = rest_str.trim_start_matches(['D', 'd', 'B', 'b', 'W', 'w']);
        let byte: u32 = byte_str.parse().ok()?;
        return Some(ParsedS7Address {
            db_number: Some(db_number),
            byte_offset: byte,
            bit_offset: None,
            count: 2,
            endian,
            is_string: false,
        });
    }

    // DBD d — 双字访问
    if let Some(rest_str) = second.strip_prefix("DBD") {
        let byte_str = rest_str.trim_start_matches(['D', 'd', 'B', 'b', 'D', 'd']);
        let byte: u32 = byte_str.parse().ok()?;
        return Some(ParsedS7Address {
            db_number: Some(db_number),
            byte_offset: byte,
            bit_offset: None,
            count: 4,
            endian,
            is_string: false,
        });
    }

    // DB1.S20.32 — 字符串：S + 起始字节 + 最大长度
    if let Some(rest_str) = second.strip_prefix('S') {
        let start_byte: u32 = rest_str.parse().ok()?;
        let len: u32 = {
            let l = rest.get(1)?;
            l.parse().ok()?
        };
        return Some(ParsedS7Address {
            db_number: Some(db_number),
            byte_offset: start_byte,
            bit_offset: None,
            count: len,
            endian,
            is_string: true,
        });
    }

    None
}

fn parse_area_common(
    _area: &str,
    byte: u32,
    rest: &[&str],
    endian: S7Endian,
) -> Option<ParsedS7Address> {
    if rest.is_empty() {
        // MB10 / IB2 等：单字节
        return Some(ParsedS7Address {
            db_number: None,
            byte_offset: byte,
            bit_offset: None,
            count: 1,
            endian,
            is_string: false,
        });
    }

    let second = rest[0].to_uppercase();
    // 如果第二个部分是纯数字 → 位访问
    if let Ok(bit) = second.parse::<u8>() {
        if bit <= 7 {
            return Some(ParsedS7Address {
                db_number: None,
                byte_offset: byte,
                bit_offset: Some(bit),
                count: 1,
                endian,
                is_string: false,
            });
        }
    }

    None
}

fn parse_counter_or_timer(
    first: &str,
    _rest: &[&str],
    endian: S7Endian,
) -> Option<ParsedS7Address> {
    let num_str = first.trim_start_matches(['C', 'c', 'T', 't']);
    let _num: u32 = num_str.parse().ok()?;
    Some(ParsedS7Address {
        db_number: None,
        byte_offset: 0,
        bit_offset: None,
        count: 1,
        endian,
        is_string: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_dbx_bit() {
        let p = parse_address_full("DB2.DBX0.1").unwrap();
        assert_eq!(p.db_number, Some(2));
        assert_eq!(p.byte_offset, 0);
        assert_eq!(p.bit_offset, Some(1));
    }

    #[test]
    fn test_parse_dbw_word() {
        let p = parse_address_full("DB2.DBW10").unwrap();
        assert_eq!(p.db_number, Some(2));
        assert_eq!(p.byte_offset, 10);
        assert_eq!(p.count, 2);
    }

    #[test]
    fn test_parse_dbd_dword() {
        let p = parse_address_full("DB2.DBD12").unwrap();
        assert_eq!(p.db_number, Some(2));
        assert_eq!(p.byte_offset, 12);
        assert_eq!(p.count, 4);
    }

    #[test]
    fn test_parse_string() {
        let p = parse_address_full("DB1.S20.32").unwrap();
        assert_eq!(p.db_number, Some(1));
        assert_eq!(p.byte_offset, 20);
        assert_eq!(p.count, 32);
        assert!(p.is_string);
    }

    #[test]
    fn test_parse_merker_bit() {
        let p = parse_address_full("M0.0").unwrap();
        assert_eq!(p.byte_offset, 0);
        assert_eq!(p.bit_offset, Some(0));
    }

    #[test]
    fn test_parse_merker_byte() {
        let p = parse_address_full("MB10").unwrap();
        assert_eq!(p.byte_offset, 10);
        assert_eq!(p.count, 1);
    }

    #[test]
    fn test_parse_merker_word() {
        let p = parse_address_full("MW12").unwrap();
        assert_eq!(p.byte_offset, 12);
        assert_eq!(p.count, 2);
    }

    #[test]
    fn test_parse_merker_dword() {
        let p = parse_address_full("MD16#F").unwrap();
        assert_eq!(p.byte_offset, 16);
        assert_eq!(p.count, 4);
    }

    #[test]
    fn test_parse_input_bit() {
        let p = parse_address_full("I0.0").unwrap();
        assert_eq!(p.byte_offset, 0);
        assert_eq!(p.bit_offset, Some(0));
    }

    #[test]
    fn test_parse_input_word() {
        let p = parse_address_full("IW4").unwrap();
        assert_eq!(p.byte_offset, 4);
        assert_eq!(p.count, 2);
    }

    #[test]
    fn test_parse_output_word() {
        let p = parse_address_full("QW4").unwrap();
        assert_eq!(p.byte_offset, 4);
        assert_eq!(p.count, 2);
    }

    #[test]
    fn test_parse_dbb_with_swap() {
        let p = parse_address_full("DB2.DBW10#B").unwrap();
        assert_eq!(p.endian, S7Endian::Swapped);
    }

    #[test]
    fn test_invalid_address() {
        assert!(parse_address_full("").is_none());
        assert!(parse_address_full("NOTAVALID").is_none());
        assert!(parse_address_full("DB2.INVALID").is_none());
    }
}
