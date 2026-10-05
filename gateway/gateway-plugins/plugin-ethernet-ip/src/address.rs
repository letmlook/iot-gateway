//! EtherNet/IP CIP 地址解析。
//!
//! 地址格式（纯逻辑，无真实 CIP 栈）：
//! - `Class/Instance/Attribute`  例如 `0x67/1/3`
//! - `Class/Instance`           例如 `100/1`
//! - 简写标签名                 例如 `Tag1`
//!
//! 本模块为桩实现，解析结果仅供校验格式正确性，不涉及网络通信。

/// CIP 地址解析结果
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ParsedAddress {
    /// CIP 类别 ID（16 位）
    pub class_id: u32,
    /// 实例 ID（16 位）
    pub instance_id: u32,
    /// 属性 ID（默认为 0 表示整个实例）
    pub attribute_id: u32,
}

/// 解析 CIP 地址字符串。
///
/// 接受格式：
/// - `class/instance/attribute`  (全部十进制或 0x 前缀十六进制)
/// - `class/instance`            (attribute 默认为 0)
/// - 纯标签名                    (返回 None)
///
/// 详见单元测试。
pub fn parse_address(addr: &str) -> Option<ParsedAddress> {
    let addr = addr.trim();
    if addr.is_empty() {
        return None;
    }

    // 标签名格式：含字母但不含 `/` 且不以 0x 开头（如 "Tag1"）
    // 含 `/` 的地址即使是 hex 格式（如 "0x67/1/3"）也应继续解析
    let is_tag_name = |s: &str| -> bool {
        s.chars().any(|c| c.is_ascii_alphabetic())
            && !s.contains('/')
            && !s.starts_with("0x")
            && !s.starts_with("0X")
    };
    if is_tag_name(addr) {
        return None;
    }

    let parts: Vec<&str> = addr.split('/').map(|s| s.trim()).collect();
    if parts.len() < 2 || parts.len() > 3 {
        return None;
    }

    let parse_u32 = |s: &str| -> Option<u32> {
        if s.starts_with("0x") || s.starts_with("0X") {
            u32::from_str_radix(&s[2..], 16).ok()
        } else {
            s.parse().ok()
        }
    };

    let class_id = parse_u32(parts[0])?;
    let instance_id = parse_u32(parts[1])?;
    let attribute_id = if parts.len() == 3 {
        parse_u32(parts[2])?
    } else {
        0
    };

    Some(ParsedAddress {
        class_id,
        instance_id,
        attribute_id,
    })
}

/// 校验地址是否为有效的 CIP 标签名（字母开头，可含数字下划线）
pub fn is_valid_tag_name(addr: &str) -> bool {
    let addr = addr.trim();
    if addr.is_empty() {
        return false;
    }
    let mut chars = addr.chars();
    let first = chars.next().unwrap();
    if !first.is_ascii_alphabetic() && first != '_' {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_address_full() {
        let p = parse_address("100/1/3").unwrap();
        assert_eq!(p.class_id, 100);
        assert_eq!(p.instance_id, 1);
        assert_eq!(p.attribute_id, 3);
    }

    #[test]
    fn test_parse_address_hex() {
        let p = parse_address("0x67/1/3").unwrap();
        assert_eq!(p.class_id, 0x67);
        assert_eq!(p.instance_id, 1);
        assert_eq!(p.attribute_id, 3);
    }

    #[test]
    fn test_parse_address_no_attribute() {
        let p = parse_address("100/1").unwrap();
        assert_eq!(p.class_id, 100);
        assert_eq!(p.instance_id, 1);
        assert_eq!(p.attribute_id, 0);
    }

    #[test]
    fn test_parse_address_tag_name() {
        assert!(parse_address("Tag1").is_none());
        assert!(parse_address("My_Tag").is_none());
        assert!(parse_address("Motor1_Speed").is_none());
    }

    #[test]
    fn test_parse_address_invalid() {
        assert!(parse_address("").is_none());
        assert!(parse_address("100").is_none());
        assert!(parse_address("100/").is_none());
        assert!(parse_address("/100/1").is_none());
        assert!(parse_address("abc/def/ghi").is_none());
        assert!(parse_address("1/2/3/4").is_none());
    }

    #[test]
    fn test_is_valid_tag_name() {
        assert!(is_valid_tag_name("Tag1"));
        assert!(is_valid_tag_name("My_Tag"));
        assert!(is_valid_tag_name("_Leading"));
        assert!(is_valid_tag_name("Motor_Speed_123"));
        assert!(!is_valid_tag_name(""));
        assert!(!is_valid_tag_name("123Tag"));
        assert!(!is_valid_tag_name("Tag-1"));
        assert!(!is_valid_tag_name("Tag.1"));
    }

    #[test]
    fn test_parse_address_edge_cases() {
        // 最大值
        let p = parse_address("0xFFFFFFFF/0xFFFFFFFF/0xFFFFFFFF").unwrap();
        assert_eq!(p.class_id, u32::MAX);
        assert_eq!(p.instance_id, u32::MAX);
        assert_eq!(p.attribute_id, u32::MAX);

        // 全小写十六进制
        let p = parse_address("0xab/0xcd/0xef").unwrap();
        assert_eq!(p.class_id, 0xab);
        assert_eq!(p.instance_id, 0xcd);
        assert_eq!(p.attribute_id, 0xef);

        // 混合大小写十六进制
        let p = parse_address("0XaB/0XCd/0XeF").unwrap();
        assert_eq!(p.class_id, 0xab);
    }
}
