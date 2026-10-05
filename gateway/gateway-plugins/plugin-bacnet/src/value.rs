//! BACnet APDU 值 ↔ DataValue 转换。

use gateway_sdk::types::DataValue;

use crate::address::BacnetObjectType;

/// BACnet PropertyValue → DataValue
/// 注意：这是纯逻辑转换，不触网
pub fn bacnet_property_to_datavalue(
    object_type: &BacnetObjectType,
    value: &bacnet_rs::property::PropertyValue,
) -> DataValue {
    use bacnet_rs::property::PropertyValue;

    match value {
        PropertyValue::Boolean(b) => DataValue::Bool(*b),
        PropertyValue::UnsignedInteger(v) => {
            // 判断范围决定类型
            if *v <= u8::MAX as u64 {
                DataValue::UInt8(*v as u8)
            } else if *v <= u16::MAX as u64 {
                DataValue::UInt16(*v as u16)
            } else if *v <= u32::MAX as u64 {
                DataValue::UInt32(*v as u32)
            } else {
                DataValue::UInt64(*v)
            }
        }
        PropertyValue::SignedInteger(v) => {
            if *v >= i8::MIN as i64 && *v <= i8::MAX as i64 {
                DataValue::Int8(*v as i8)
            } else if *v >= i16::MIN as i64 && *v <= i16::MAX as i64 {
                DataValue::Int16(*v as i16)
            } else if *v >= i32::MIN as i64 && *v <= i32::MAX as i64 {
                DataValue::Int32(*v as i32)
            } else {
                DataValue::Int64(*v)
            }
        }
        PropertyValue::Real(v) => DataValue::Float32(*v),
        PropertyValue::Double(v) => DataValue::Float64(*v),
        PropertyValue::CharacterString(s) => DataValue::String(s.clone()),
        PropertyValue::OctetString(b) => DataValue::Bytes(b.clone()),
        PropertyValue::BitString(bits) => DataValue::Bytes(bits.clone()),
        PropertyValue::Enumerated(v) => {
            // 枚举通常用 int32 表示
            DataValue::Int32(*v as i32)
        }
        PropertyValue::Date(_) => {
            // 日期无法直接映射，用 string 表示
            DataValue::String("date".to_string())
        }
        PropertyValue::Time(_) => DataValue::String("time".to_string()),
        PropertyValue::ObjectIdentifier(_) => DataValue::String("object-identifier".to_string()),
        _ => DataValue::String(format!("{:?}", value)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_rs::property::PropertyValue;

    #[test]
    fn test_bool_true() {
        let pv = PropertyValue::Boolean(true);
        let dv = bacnet_property_to_datavalue(&BacnetObjectType::BinaryInput, &pv);
        assert_eq!(dv, DataValue::Bool(true));
    }

    #[test]
    fn test_bool_false() {
        let pv = PropertyValue::Boolean(false);
        let dv = bacnet_property_to_datavalue(&BacnetObjectType::BinaryInput, &pv);
        assert_eq!(dv, DataValue::Bool(false));
    }

    #[test]
    fn test_unsigned_small() {
        let pv = PropertyValue::UnsignedInteger(100);
        let dv = bacnet_property_to_datavalue(&BacnetObjectType::AnalogInput, &pv);
        assert_eq!(dv, DataValue::UInt8(100));
    }

    #[test]
    fn test_unsigned_16() {
        let pv = PropertyValue::UnsignedInteger(70000);
        let dv = bacnet_property_to_datavalue(&BacnetObjectType::AnalogInput, &pv);
        assert_eq!(dv, DataValue::UInt16(70000));
    }

    #[test]
    fn test_real() {
        let pv = PropertyValue::Real(3.14);
        let dv = bacnet_property_to_datavalue(&BacnetObjectType::AnalogInput, &pv);
        assert_eq!(dv, DataValue::Float32(3.14));
    }

    #[test]
    fn test_double() {
        let pv = PropertyValue::Double(2.71828);
        let dv = bacnet_property_to_datavalue(&BacnetObjectType::AnalogInput, &pv);
        assert_eq!(dv, DataValue::Float64(2.71828));
    }

    #[test]
    fn test_characterstring() {
        let pv = PropertyValue::CharacterString("hello".to_string());
        let dv = bacnet_property_to_datavalue(&BacnetObjectType::CharacterstringValue, &pv);
        assert_eq!(dv, DataValue::String("hello".to_string()));
    }

    #[test]
    fn test_octetstring() {
        let pv = PropertyValue::OctetString(vec![0xDE, 0xAD, 0xBE, 0xEF]);
        let dv = bacnet_property_to_datavalue(&BacnetObjectType::OctetstringValue, &pv);
        assert_eq!(dv, DataValue::Bytes(vec![0xDE, 0xAD, 0xBE, 0xEF]));
    }

    #[test]
    fn test_signed_integer() {
        let pv = PropertyValue::SignedInteger(-42);
        let dv = bacnet_property_to_datavalue(&BacnetObjectType::IntegerValue, &pv);
        assert_eq!(dv, DataValue::Int32(-42));
    }
}
