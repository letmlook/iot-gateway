//! Modbus RTU 地址解析：0x!addr / 1x!addr / 3x!addr / 4x!addr 或 1!400001[.BIT][#ENDIAN]。

/// Modbus 区域
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ModbusArea {
    Coil,
    DiscreteInput,
    InputRegister,
    HoldingRegister,
}

/// 字节序：#L/#B(16bit) #LL/#LB/#BL/#BB(32/64bit)
#[derive(Clone, Copy, Default)]
pub struct Endianness {
    /// 16bit: false=#L(1,2) true=#B(2,1)
    pub swap16: bool,
    /// 32/64bit: 0=LL 1=LB 2=BL 3=BB
    pub order32: u8,
}

/// 解析后的地址（支持 4x!addr 与 1!400001[.BIT][#ENDIAN]）
#[derive(Clone)]
pub struct ParsedAddress {
    pub area: ModbusArea,
    pub start: u16,
    pub count: u16,
    pub bit_index: Option<u8>,
    pub endian: Endianness,
}

pub fn parse_address(addr: &str) -> Option<(ModbusArea, u16, u16)> {
    parse_address_full(addr, 1).map(|p| (p.area, p.start, p.count))
}

/// 完整解析：支持 4x!100、4x!100!2 与 1!400001、1!400001.4、1!400001#LB
pub fn parse_address_full(addr: &str, start_address: u8) -> Option<ParsedAddress> {
    let addr = addr.trim();
    let mut endian = Endianness::default();
    let mut bit_index: Option<u8> = None;
    let addr_no_endian = if let Some(excl) = addr.find('#') {
        let (addr_part, endian_part) = addr.split_at(excl);
        let endian_str = endian_part.trim_start_matches('#').to_uppercase();
        match endian_str.as_str() {
            "B" => endian.swap16 = true,
            "L" | "LL" => {}
            "LB" => endian.order32 = 1,
            "BL" => endian.order32 = 2,
            "BB" => endian.order32 = 3,
            _ => {}
        }
        addr_part.trim()
    } else {
        addr
    };
    let (area, start, count) = if let Some(dot) = addr_no_endian.find('.') {
        let (base, rest) = addr_no_endian.split_at(dot);
        let rest = rest.trim_start_matches('.');
        let rest_upper = rest.to_uppercase();
        let is_string_len = rest_upper.ends_with('H') || rest_upper.ends_with('L') || rest_upper.ends_with('D') || rest_upper.ends_with('E');
        let len_str: &str = if is_string_len { rest_upper.trim_end_matches(|c: char| c == 'H' || c == 'L' || c == 'D' || c == 'E') } else { &rest_upper };
        if !is_string_len && rest.len() == 1 {
            if let Ok(b) = rest.parse::<u8>() {
                if b <= 15 {
                    bit_index = Some(b);
                }
            }
            parse_address_core(base, start_address)?
        } else if let Ok(len) = len_str.parse::<usize>() {
            let (a, s, _) = parse_address_core(base, start_address)?;
            let regs = (len + 1) / 2;
            return Some(ParsedAddress { area: a, start: s, count: regs as u16, bit_index: None, endian });
        } else {
            parse_address_core(base, start_address)?
        }
    } else {
        parse_address_core(addr_no_endian, start_address)?
    };
    Some(ParsedAddress { area, start, count, bit_index, endian })
}

fn parse_address_core(addr: &str, start_address: u8) -> Option<(ModbusArea, u16, u16)> {
    let parts: Vec<&str> = addr.split('!').map(|s| s.trim()).collect();
    if parts.is_empty() {
        return None;
    }
    let first = parts[0].to_lowercase();
    if first == "0x" || first == "0" || first == "1x" || first == "1" || first == "3x" || first == "3" || first == "4x" || first == "4" {
        let area = match first.as_str() {
            "0x" | "0" => ModbusArea::Coil,
            "1x" | "1" => ModbusArea::DiscreteInput,
            "3x" | "3" => ModbusArea::InputRegister,
            _ => ModbusArea::HoldingRegister,
        };
        let start: u16 = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
        let count: u16 = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(1);
        return Some((area, start, count.max(1)));
    }
    let _slave: u8 = parts[0].parse().ok().filter(|&s| s <= 247)?;
    let addr_num: u32 = parts.get(1).and_then(|s| s.parse().ok())?;
    let (area, base) = if addr_num >= 400001 && addr_num <= 465536 {
        (ModbusArea::HoldingRegister, 400000u32)
    } else if addr_num >= 300001 && addr_num <= 365536 {
        (ModbusArea::InputRegister, 300000)
    } else if addr_num >= 100001 && addr_num <= 165536 {
        (ModbusArea::DiscreteInput, 100000)
    } else if addr_num >= 1 && addr_num <= 65536 {
        (ModbusArea::Coil, 0)
    } else {
        return None;
    };
    let start = (addr_num.saturating_sub(base).saturating_sub(start_address as u32)) as u16;
    Some((area, start, 1u16))
}
