//! IEC 60870-5-104 地址解析：IOA（Information Object Address）。
//!
//! 地址语法：十进制数字字符串，范围 1–16777215。

/// IOA 上限（24 位无符号）
#[allow(dead_code)]
pub const IOA_MAX: u32 = 16_777_215;
/// IOA 下限
#[allow(dead_code)]
pub const IOA_MIN: u32 = 1;

/// 解析 IOA 地址字符串（库内使用，单元测试也引用）。
#[allow(dead_code)]
///
/// 返回 `Some(ioa)` 如果解析成功且在有效范围 [1, 16777215] 内，
/// 否则返回 `None`。
#[cfg(feature = "iec104-client")]
pub fn parse_ioa(addr: &str) -> Option<u32> {
    let ioa: u32 = addr.parse().ok()?;
    if (IOA_MIN..=IOA_MAX).contains(&ioa) {
        Some(ioa)
    } else {
        None
    }
}

/// 校验地址字符串是否符合 IOA 正则 `^[0-9]{1,8}$`。
/// 不校验数值范围（由解析器在校验后检查）。
pub fn is_ioa_format(addr: &str) -> bool {
    !addr.is_empty() && addr.len() <= 8 && addr.chars().all(|c| c.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_ioa_valid() {
        assert_eq!(parse_ioa("1"), Some(1));
        assert_eq!(parse_ioa("100"), Some(100));
        assert_eq!(parse_ioa("16777215"), Some(16777215));
        assert_eq!(parse_ioa("1000000"), Some(1000000));
    }

    #[test]
    fn test_parse_ioa_invalid() {
        // 越界
        assert_eq!(parse_ioa("0"), None);
        assert_eq!(parse_ioa("16777216"), None);
        assert_eq!(parse_ioa("99999999"), None);
        // 非数字
        assert_eq!(parse_ioa("abc"), None);
        assert_eq!(parse_ioa("1a"), None);
        assert_eq!(parse_ioa(""), None);
        assert_eq!(parse_ioa(" "), None);
        assert_eq!(parse_ioa("-1"), None);
    }

    #[test]
    fn test_is_ioa_format() {
        assert!(is_ioa_format("1"));
        assert!(is_ioa_format("12345678"));
        assert!(is_ioa_format("0"));
        // 超过8位
        assert!(!is_ioa_format("123456789"));
        // 非数字
        assert!(!is_ioa_format("abc"));
        assert!(!is_ioa_format(""));
        assert!(!is_ioa_format("12a"));
    }
}
