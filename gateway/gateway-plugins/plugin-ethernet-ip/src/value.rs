//! CIP 数据类型与 DataValue 互转。
//!
//! 本模块为桩实现：仅返回占位值，不执行真实协议解析。

use gateway_sdk::types::DataValue;

/// CIP 标签数据类型标签（用于日志/调试）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CipDataType {
    Bool,
    Sint,
    Int,
    Dint,
    Lint,
    Usint,
    Uint,
    Udint,
    Ulint,
    Real,
    Lreal,
    String,
    Byte,
    Word,
    Dword,
    Unknown,
}

impl CipDataType {
    /// 从数据类型字符串获取 CIP 类型
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "bool" => CipDataType::Bool,
            "sint" => CipDataType::Sint,
            "int" => CipDataType::Int,
            "dint" => CipDataType::Dint,
            "lint" => CipDataType::Lint,
            "usint" => CipDataType::Usint,
            "uint" => CipDataType::Uint,
            "udint" => CipDataType::Udint,
            "ulint" => CipDataType::Ulint,
            "real" => CipDataType::Real,
            "lreal" => CipDataType::Lreal,
            "string" => CipDataType::String,
            "byte" => CipDataType::Byte,
            "word" => CipDataType::Word,
            "dword" => CipDataType::Dword,
            _ => CipDataType::Unknown,
        }
    }

    /// 返回占位 DataValue（用于无真实协议栈时的桩实现）
    pub fn placeholder_value(&self) -> DataValue {
        match self {
            CipDataType::Bool => DataValue::Bool(false),
            CipDataType::Sint | CipDataType::Int | CipDataType::Lint => DataValue::Int64(0),
            CipDataType::Usint
            | CipDataType::Uint
            | CipDataType::Ulint
            | CipDataType::Byte
            | CipDataType::Word
            | CipDataType::Dword => DataValue::UInt64(0),
            CipDataType::Dint | CipDataType::Udint => DataValue::Int32(0),
            CipDataType::Real => DataValue::Float32(0.0),
            CipDataType::Lreal => DataValue::Float64(0.0),
            CipDataType::String => DataValue::String(String::new()),
            CipDataType::Unknown => DataValue::UInt64(0),
        }
    }
}

/// CIP 字节序列（用于批量数据类型）
#[allow(dead_code)]
pub struct CipBytes {
    pub bytes: Vec<u8>,
}

#[allow(dead_code)]
impl CipBytes {
    pub fn new() -> Self {
        Self { bytes: Vec::new() }
    }

    pub fn from_value(value: &DataValue) -> Self {
        let mut b = Vec::new();
        match value {
            DataValue::Bool(v) => {
                b.push(if *v { 1 } else { 0 });
            }
            DataValue::Int8(v) => b.push(*v as u8),
            DataValue::UInt8(v) => b.push(*v),
            DataValue::Int16(v) => b.extend_from_slice(&v.to_le_bytes()),
            DataValue::UInt16(v) => b.extend_from_slice(&v.to_le_bytes()),
            DataValue::Int32(v) => b.extend_from_slice(&v.to_le_bytes()),
            DataValue::UInt32(v) => b.extend_from_slice(&v.to_le_bytes()),
            DataValue::Int64(v) => b.extend_from_slice(&v.to_le_bytes()),
            DataValue::UInt64(v) => b.extend_from_slice(&v.to_le_bytes()),
            DataValue::Float32(v) => b.extend_from_slice(&v.to_bits().to_le_bytes()),
            DataValue::Float64(v) => b.extend_from_slice(&v.to_bits().to_le_bytes()),
            DataValue::String(s) => b.extend_from_slice(s.as_bytes()),
            DataValue::Bytes(bs) => b.extend_from_slice(bs),
        }
        Self { bytes: b }
    }
}

impl Default for CipBytes {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cip_data_type_from_str() {
        assert_eq!(CipDataType::from_str("bool"), CipDataType::Bool);
        assert_eq!(CipDataType::from_str("INT"), CipDataType::Int);
        assert_eq!(CipDataType::from_str("Real"), CipDataType::Real);
        assert_eq!(CipDataType::from_str("unknown"), CipDataType::Unknown);
    }

    #[test]
    fn test_placeholder_value() {
        assert_eq!(
            CipDataType::Bool.placeholder_value(),
            DataValue::Bool(false)
        );
        assert_eq!(CipDataType::Int.placeholder_value(), DataValue::Int64(0));
        assert_eq!(CipDataType::Uint.placeholder_value(), DataValue::UInt64(0));
        assert_eq!(
            CipDataType::Real.placeholder_value(),
            DataValue::Float32(0.0)
        );
        assert_eq!(
            CipDataType::String.placeholder_value(),
            DataValue::String(String::new())
        );
    }

    #[test]
    fn test_cip_bytes_from_value() {
        let cb = CipBytes::from_value(&DataValue::Int32(0x12345678));
        assert_eq!(cb.bytes.len(), 4);

        let cb = CipBytes::from_value(&DataValue::String("AB".to_string()));
        assert_eq!(cb.bytes, b"AB");
    }
}
