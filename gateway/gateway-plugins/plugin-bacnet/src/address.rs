//! BACnet 地址解析：object-type:instance[:property]。
//!
//! 地址语法（纯逻辑、可单测）：
//!   ai:1          → AnalogInput instance 1，present-value
//!   ao:5:units    → AnalogOutput instance 5，units 属性
//!   av:2          → AnalogValue instance 2
//!   bi:1 / bo:1 / bv:1  → BinaryInput/Output/Value
//!   msi:1 / mso:1 / msv:1  → MultiStateInput/Output/Value
//!   bsv:1         → BitstringValue（bytes）
//!   csv:1         → CharacterstringValue（string）
//!   iv:1          → IntegerValue（int32）
//!   lav:1          → LargeAnalogValue（float64）
//!   osv:1         → OctetstringValue（bytes）
//!   piv:1         → PositiveIntegerValue（uint64）

/// BACnet 对象类型缩写 → ObjectType
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BacnetObjectType {
    AnalogInput,
    AnalogOutput,
    AnalogValue,
    BinaryInput,
    BinaryOutput,
    BinaryValue,
    MultiStateInput,
    MultiStateOutput,
    MultiStateValue,
    BitstringValue,
    CharacterstringValue,
    IntegerValue,
    LargeAnalogValue,
    OctetstringValue,
    PositiveIntegerValue,
}

impl BacnetObjectType {
    /// 根据缩写字符串返回 ObjectType，None 表示不支持该缩写
    pub fn from_abbr(abbr: &str) -> Option<Self> {
        match abbr.to_lowercase().as_str() {
            "ai" => Some(Self::AnalogInput),
            "ao" => Some(Self::AnalogOutput),
            "av" => Some(Self::AnalogValue),
            "bi" => Some(Self::BinaryInput),
            "bo" => Some(Self::BinaryOutput),
            "bv" => Some(Self::BinaryValue),
            "msi" => Some(Self::MultiStateInput),
            "mso" => Some(Self::MultiStateOutput),
            "msv" => Some(Self::MultiStateValue),
            "bsv" => Some(Self::BitstringValue),
            "csv" => Some(Self::CharacterstringValue),
            "iv" => Some(Self::IntegerValue),
            "lav" => Some(Self::LargeAnalogValue),
            "osv" => Some(Self::OctetstringValue),
            "piv" => Some(Self::PositiveIntegerValue),
            // 全名
            "analog-input" => Some(Self::AnalogInput),
            "analog-output" => Some(Self::AnalogOutput),
            "analog-value" => Some(Self::AnalogValue),
            "binary-input" => Some(Self::BinaryInput),
            "binary-output" => Some(Self::BinaryOutput),
            "binary-value" => Some(Self::BinaryValue),
            "multistate-input" => Some(Self::MultiStateInput),
            "multistate-output" => Some(Self::MultiStateOutput),
            "multistate-value" => Some(Self::MultiStateValue),
            "bitstring-value" => Some(Self::BitstringValue),
            "characterstring-value" => Some(Self::CharacterstringValue),
            "integer-value" => Some(Self::IntegerValue),
            "large-analog-value" => Some(Self::LargeAnalogValue),
            "octetstring-value" => Some(Self::OctetstringValue),
            "positive-integer-value" => Some(Self::PositiveIntegerValue),
            _ => None,
        }
    }
}

/// 解析后的 BACnet 地址
#[derive(Debug, Clone)]
pub struct ParsedBacnetAddress {
    /// 对象类型
    pub object_type: BacnetObjectType,
    /// 实例号
    pub instance: u32,
    /// 属性标识符（默认 present-value）
    pub property: String,
}

impl ParsedBacnetAddress {
    pub fn new(object_type: BacnetObjectType, instance: u32) -> Self {
        Self {
            object_type,
            instance,
            property: "present-value".to_string(),
        }
    }

    pub fn with_property(object_type: BacnetObjectType, instance: u32, property: &str) -> Self {
        Self {
            object_type,
            instance,
            property: property.to_string(),
        }
    }
}

/// 解析 BACnet 地址字符串。
/// 格式：object-type:instance[:property]
/// object-type 可以是缩写（ai/ao/av/bi/bo/bv/msi/mso/msv/bsv/csv/iv/lav/osv/piv）或全名。
pub fn parse_address_full(addr: &str) -> Option<ParsedBacnetAddress> {
    let addr = addr.trim();
    if addr.is_empty() {
        return None;
    }

    // 找到第一个 ':'
    let colon_pos = addr.find(':')?;
    let type_str = &addr[..colon_pos];
    let rest = &addr[colon_pos + 1..];

    let object_type = BacnetObjectType::from_abbr(type_str)?;

    // 找到第二个 ':'（属性分隔符）
    if let Some(prop_pos) = rest.find(':') {
        let instance_str = &rest[..prop_pos];
        let instance: u32 = instance_str.parse().ok()?;
        let property = rest[prop_pos + 1..].to_lowercase();
        if property.is_empty() {
            return None;
        }
        Some(ParsedBacnetAddress::with_property(
            object_type,
            instance,
            &property,
        ))
    } else {
        // 无属性：默认 present-value
        let instance: u32 = rest.parse().ok()?;
        Some(ParsedBacnetAddress::new(object_type, instance))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ai() {
        let p = parse_address_full("ai:1").unwrap();
        assert!(matches!(p.object_type, BacnetObjectType::AnalogInput));
        assert_eq!(p.instance, 1);
        assert_eq!(p.property, "present-value");
    }

    #[test]
    fn test_ao_with_property() {
        let p = parse_address_full("ao:5:units").unwrap();
        assert!(matches!(p.object_type, BacnetObjectType::AnalogOutput));
        assert_eq!(p.instance, 5);
        assert_eq!(p.property, "units");
    }

    #[test]
    fn test_av() {
        let p = parse_address_full("av:2").unwrap();
        assert!(matches!(p.object_type, BacnetObjectType::AnalogValue));
    }

    #[test]
    fn test_binary() {
        let p = parse_address_full("bi:10").unwrap();
        assert!(matches!(p.object_type, BacnetObjectType::BinaryInput));
        assert_eq!(p.instance, 10);
    }

    #[test]
    fn test_bo() {
        let p = parse_address_full("bo:3").unwrap();
        assert!(matches!(p.object_type, BacnetObjectType::BinaryOutput));
    }

    #[test]
    fn test_bv() {
        let p = parse_address_full("bv:7").unwrap();
        assert!(matches!(p.object_type, BacnetObjectType::BinaryValue));
    }

    #[test]
    fn test_multistate() {
        let p = parse_address_full("msi:1").unwrap();
        assert!(matches!(p.object_type, BacnetObjectType::MultiStateInput));
        assert_eq!(p.instance, 1);

        let p = parse_address_full("mso:2").unwrap();
        assert!(matches!(p.object_type, BacnetObjectType::MultiStateOutput));

        let p = parse_address_full("msv:3").unwrap();
        assert!(matches!(p.object_type, BacnetObjectType::MultiStateValue));
    }

    #[test]
    fn test_bsv() {
        let p = parse_address_full("bsv:1").unwrap();
        assert!(matches!(p.object_type, BacnetObjectType::BitstringValue));
        assert_eq!(p.instance, 1);
    }

    #[test]
    fn test_csv() {
        let p = parse_address_full("csv:1").unwrap();
        assert!(matches!(
            p.object_type,
            BacnetObjectType::CharacterstringValue
        ));
    }

    #[test]
    fn test_iv() {
        let p = parse_address_full("iv:42").unwrap();
        assert!(matches!(p.object_type, BacnetObjectType::IntegerValue));
        assert_eq!(p.instance, 42);
    }

    #[test]
    fn test_lav() {
        let p = parse_address_full("lav:1").unwrap();
        assert!(matches!(p.object_type, BacnetObjectType::LargeAnalogValue));
    }

    #[test]
    fn test_osv() {
        let p = parse_address_full("osv:1").unwrap();
        assert!(matches!(p.object_type, BacnetObjectType::OctetstringValue));
    }

    #[test]
    fn test_piv() {
        let p = parse_address_full("piv:1").unwrap();
        assert!(matches!(
            p.object_type,
            BacnetObjectType::PositiveIntegerValue
        ));
    }

    #[test]
    fn test_full_name() {
        let p = parse_address_full("analog-input:5").unwrap();
        assert!(matches!(p.object_type, BacnetObjectType::AnalogInput));
        assert_eq!(p.instance, 5);
    }

    #[test]
    fn test_invalid_empty() {
        assert!(parse_address_full("").is_none());
        assert!(parse_address_full("   ").is_none());
    }

    #[test]
    fn test_invalid_type() {
        assert!(parse_address_full("xyz:1").is_none());
    }

    #[test]
    fn test_invalid_instance() {
        assert!(parse_address_full("ai:abc").is_none());
        assert!(parse_address_full("ai:").is_none());
    }

    #[test]
    fn test_invalid_negative() {
        assert!(parse_address_full("ai:-1").is_none());
    }

    #[test]
    fn test_case_insensitive() {
        let p = parse_address_full("AI:1").unwrap();
        assert!(matches!(p.object_type, BacnetObjectType::AnalogInput));
        let p = parse_address_full("Ao:5:Units").unwrap();
        assert!(matches!(p.object_type, BacnetObjectType::AnalogOutput));
        assert_eq!(p.property, "units");
    }
}
