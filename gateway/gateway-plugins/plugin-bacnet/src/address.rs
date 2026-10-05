//! BACnet 地址解析：object-type:instance[:property]。
//!
//! 地址语法（default feature 关闭时也可编译、可单测）：
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
//!
//! 缩写表以 bacnet-rs ObjectType 枚举为准。

use crate::BacnetObjectType;

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

    /// 转为 bacnet-rs 的 ObjectType
    #[cfg(feature = "bacnet-client")]
    pub fn to_object_type(&self) -> bacnet_rs::object::ObjectType {
        use bacnet_rs::object::ObjectType as BOT;
        match self {
            Self::AnalogInput => BOT::AnalogInput,
            Self::AnalogOutput => BOT::AnalogOutput,
            Self::AnalogValue => BOT::AnalogValue,
            Self::BinaryInput => BOT::BinaryInput,
            Self::BinaryOutput => BOT::BinaryOutput,
            Self::BinaryValue => BOT::BinaryValue,
            Self::MultiStateInput => BOT::MultiStateInput,
            Self::MultiStateOutput => BOT::MultiStateOutput,
            Self::MultiStateValue => BOT::MultiStateValue,
            Self::BitstringValue => BOT::BitstringValue,
            Self::CharacterstringValue => BOT::CharacterstringValue,
            Self::IntegerValue => BOT::IntegerValue,
            Self::LargeAnalogValue => BOT::LargeAnalogValue,
            Self::OctetstringValue => BOT::OctetstringValue,
            Self::PositiveIntegerValue => BOT::PositiveIntegerValue,
        }
    }

    /// 该对象类型对应的默认 data_type
    pub fn default_data_type(&self) -> &'static str {
        match self {
            Self::AnalogInput | Self::AnalogOutput | Self::AnalogValue => "float64",
            Self::BinaryInput | Self::BinaryOutput | Self::BinaryValue => "bool",
            Self::MultiStateInput | Self::MultiStateOutput | Self::MultiStateValue => "int32",
            Self::BitstringValue => "bytes",
            Self::CharacterstringValue => "string",
            Self::IntegerValue => "int32",
            Self::LargeAnalogValue => "float64",
            Self::OctetstringValue => "bytes",
            Self::PositiveIntegerValue => "uint64",
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
pub fn parse_address(addr: &str) -> Option<ParsedBacnetAddress> {
    parse_address_full(addr.trim())
}

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
        let p = parse_address("ai:1").unwrap();
        assert!(matches!(p.object_type, BacnetObjectType::AnalogInput));
        assert_eq!(p.instance, 1);
        assert_eq!(p.property, "present-value");
    }

    #[test]
    fn test_ao_with_property() {
        let p = parse_address("ao:5:units").unwrap();
        assert!(matches!(p.object_type, BacnetObjectType::AnalogOutput));
        assert_eq!(p.instance, 5);
        assert_eq!(p.property, "units");
    }

    #[test]
    fn test_av() {
        let p = parse_address("av:2").unwrap();
        assert!(matches!(p.object_type, BacnetObjectType::AnalogValue));
    }

    #[test]
    fn test_binary() {
        let p = parse_address("bi:10").unwrap();
        assert!(matches!(p.object_type, BacnetObjectType::BinaryInput));
        assert_eq!(p.instance, 10);
    }

    #[test]
    fn test_bo() {
        let p = parse_address("bo:3").unwrap();
        assert!(matches!(p.object_type, BacnetObjectType::BinaryOutput));
    }

    #[test]
    fn test_bv() {
        let p = parse_address("bv:7").unwrap();
        assert!(matches!(p.object_type, BacnetObjectType::BinaryValue));
    }

    #[test]
    fn test_multistate() {
        let p = parse_address("msi:1").unwrap();
        assert!(matches!(p.object_type, BacnetObjectType::MultiStateInput));
        assert_eq!(p.instance, 1);

        let p = parse_address("mso:2").unwrap();
        assert!(matches!(p.object_type, BacnetObjectType::MultiStateOutput));

        let p = parse_address("msv:3").unwrap();
        assert!(matches!(p.object_type, BacnetObjectType::MultiStateValue));
    }

    #[test]
    fn test_bsv() {
        let p = parse_address("bsv:1").unwrap();
        assert!(matches!(p.object_type, BacnetObjectType::BitstringValue));
        assert_eq!(p.instance, 1);
    }

    #[test]
    fn test_csv() {
        let p = parse_address("csv:1").unwrap();
        assert!(matches!(
            p.object_type,
            BacnetObjectType::CharacterstringValue
        ));
    }

    #[test]
    fn test_iv() {
        let p = parse_address("iv:42").unwrap();
        assert!(matches!(p.object_type, BacnetObjectType::IntegerValue));
        assert_eq!(p.instance, 42);
    }

    #[test]
    fn test_lav() {
        let p = parse_address("lav:1").unwrap();
        assert!(matches!(p.object_type, BacnetObjectType::LargeAnalogValue));
    }

    #[test]
    fn test_osv() {
        let p = parse_address("osv:1").unwrap();
        assert!(matches!(p.object_type, BacnetObjectType::OctetstringValue));
    }

    #[test]
    fn test_piv() {
        let p = parse_address("piv:1").unwrap();
        assert!(matches!(
            p.object_type,
            BacnetObjectType::PositiveIntegerValue
        ));
    }

    #[test]
    fn test_full_name() {
        let p = parse_address("analog-input:5").unwrap();
        assert!(matches!(p.object_type, BacnetObjectType::AnalogInput));
        assert_eq!(p.instance, 5);
    }

    #[test]
    fn test_invalid_empty() {
        assert!(parse_address("").is_none());
        assert!(parse_address("   ").is_none());
    }

    #[test]
    fn test_invalid_type() {
        assert!(parse_address("xyz:1").is_none());
    }

    #[test]
    fn test_invalid_instance() {
        assert!(parse_address("ai:abc").is_none());
        assert!(parse_address("ai:").is_none());
    }

    #[test]
    fn test_invalid_negative() {
        assert!(parse_address("ai:-1").is_none());
    }

    #[test]
    fn test_case_insensitive() {
        let p = parse_address("AI:1").unwrap();
        assert!(matches!(p.object_type, BacnetObjectType::AnalogInput));
        let p = parse_address("Ao:5:Units").unwrap();
        assert!(matches!(p.object_type, BacnetObjectType::AnalogOutput));
        assert_eq!(p.property, "units");
    }
}
