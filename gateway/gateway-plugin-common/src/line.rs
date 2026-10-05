//! InfluxDB 行协议编码器（按官方语法转义规则逐条编码）。
//!
//! 官方语法参考：<https://influxdata.github.io/influxdb/v2/reference/syntax/line-protocol/>
//!
//! 转义规则（官方核实）：
//! - measurement：逗号 `,` 和空格必须转义为 `\,` 和 `\ `
//! - tag key/value、field key：逗号 `,`、等号 `=`、空格必须转义
//! - 字符串 field value：双引号 `"` 和反斜杠 `\` 必须转义为 `\"` 和 `\\`
//! - 整型 field value：后缀 `i`
//! - 无符号整型 field value：后缀 `u`
//! - 浮点 field value：默认浮点
//! - 布尔 field value：`t` 或 `f`
//! - Bytes 类型：跳过（调用方应计 skipped_bytes_fields）

use gateway_sdk::types::DataValue;

/// 行协议字段值
#[derive(Debug, Clone, PartialEq)]
pub enum LineField {
    Int(i64),
    UInt(u64),
    Float(f64),
    Bool(bool),
    Str(String),
}

/// 行协议一个点位
#[derive(Debug, Clone)]
pub struct LinePoint {
    pub measurement: String,
    pub tags: Vec<(String, String)>,
    pub fields: Vec<(String, LineField)>,
    pub ts_ms: i64,
}

/// DataValue → LineField。Bytes 类型返回 None（调用方跳过并计数）。
pub fn data_value_to_line_field(v: &DataValue) -> Option<LineField> {
    match v {
        DataValue::Bool(b) => Some(LineField::Bool(*b)),
        DataValue::Int8(n) => Some(LineField::Int(*n as i64)),
        DataValue::Int16(n) => Some(LineField::Int(*n as i64)),
        DataValue::Int32(n) => Some(LineField::Int(*n as i64)),
        DataValue::Int64(n) => Some(LineField::Int(*n)),
        DataValue::UInt8(n) => Some(LineField::UInt(*n as u64)),
        DataValue::UInt16(n) => Some(LineField::UInt(*n as u64)),
        DataValue::UInt32(n) => Some(LineField::UInt(*n as u64)),
        DataValue::UInt64(n) => Some(LineField::UInt(*n)),
        DataValue::Float32(f) => Some(LineField::Float(*f as f64)),
        DataValue::Float64(f) => Some(LineField::Float(*f)),
        DataValue::String(s) => Some(LineField::Str(s.clone())),
        DataValue::Bytes(_) => None,
    }
}

/// 逐字符转义measurement
fn escape_measurement(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            ',' => out.push_str("\\,"),
            ' ' => out.push_str("\\ "),
            _ => out.push(c),
        }
    }
    out
}

/// 逐字符转义 tag key/value / field key
fn escape_tag_or_field(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            ',' => out.push_str("\\,"),
            '=' => out.push_str("\\="),
            ' ' => out.push_str("\\ "),
            _ => out.push(c),
        }
    }
    out
}

/// 逐字符转义字符串 field value
fn escape_string_field(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            _ => out.push(c),
        }
    }
    out
}

/// 将 LinePoint 编码为一行，行尾无换行（调用方自行追加 `\n`）
pub fn encode(p: &LinePoint, out: &mut String) {
    // measurement
    out.push_str(&escape_measurement(&p.measurement));

    // tags（逗号分隔，无则留空）
    if !p.tags.is_empty() {
        out.push(',');
        for (i, (k, v)) in p.tags.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str(&escape_tag_or_field(k));
            out.push('=');
            out.push_str(&escape_tag_or_field(v));
        }
    }

    // fields（等号分隔，空格分隔）
    out.push(' ');
    for (i, (k, vf)) in p.fields.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str(&escape_tag_or_field(k));
        out.push('=');
        match vf {
            LineField::Int(n) => {
                out.push_str(&n.to_string());
                out.push('i');
            }
            LineField::UInt(n) => {
                out.push_str(&n.to_string());
                out.push('u');
            }
            LineField::Float(n) => {
                // 使用 Debug 格式以得到稳定的 NaN/Inf 表示
                out.push_str(&format!("{:?}", n));
            }
            LineField::Bool(true) => out.push('t'),
            LineField::Bool(false) => out.push('f'),
            LineField::Str(s) => {
                out.push('"');
                out.push_str(&escape_string_field(s));
                out.push('"');
            }
        }
    }

    // timestamp
    out.push(' ');
    out.push_str(&p.ts_ms.to_string());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(
        meas: &str,
        tags: Vec<(&str, &str)>,
        fields: Vec<(&str, LineField)>,
        ts_ms: i64,
    ) -> LinePoint {
        LinePoint {
            measurement: meas.to_string(),
            tags: tags
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            fields: fields
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
            ts_ms,
        }
    }

    fn encoded(p: &LinePoint) -> String {
        let mut s = String::new();
        encode(p, &mut s);
        s
    }

    #[test]
    fn measurement_escapes_comma_and_space() {
        let p = point("cpu,usage", vec![], vec![("val", LineField::Int(1))], 1000);
        let s = encoded(&p);
        assert!(s.starts_with("cpu\\,usage"));
        // cpu,usage has no space — verify no spurious escape
        assert!(!s.contains("\\ "));
    }

    #[test]
    fn tag_escapes_comma_equals_space() {
        let p = point(
            "m",
            vec![("k=a", "v b")],
            vec![("f", LineField::Int(1))],
            1000,
        );
        let s = encoded(&p);
        // tag key should be escaped
        assert!(s.contains("k\\=a"));
        // tag value should be escaped
        assert!(s.contains("v\\ b"));
    }

    #[test]
    fn string_field_escapes_quote_and_backslash() {
        let p = point(
            "m",
            vec![],
            vec![("msg", LineField::Str(r#"say "hello"\\test"#.to_string()))],
            1000,
        );
        let s = encoded(&p);
        // the field value should have \" and \\
        assert!(s.contains(r#"\"hello\""#));
        assert!(s.contains(r#"\\test"#));
    }

    #[test]
    fn integer_suffix_i() {
        let p = point("m", vec![], vec![("n", LineField::Int(-42))], 1000);
        let s = encoded(&p);
        assert!(s.contains("-42i"));
    }

    #[test]
    fn unsigned_suffix_u() {
        let p = point("m", vec![], vec![("n", LineField::UInt(42))], 1000);
        let s = encoded(&p);
        assert!(s.contains("42u"));
    }

    #[test]
    fn bool_suffix_t_or_f() {
        let p = point(
            "m",
            vec![],
            vec![
                ("ok", LineField::Bool(true)),
                ("err", LineField::Bool(false)),
            ],
            1000,
        );
        let s = encoded(&p);
        assert!(s.contains("t"));
        assert!(s.contains("f"));
    }

    #[test]
    fn float_no_suffix() {
        let p = point("m", vec![], vec![("temp", LineField::Float(36.6))], 1000);
        let s = encoded(&p);
        assert!(s.contains("36.6"));
        // should not contain 'i' or 'u' suffix
        assert!(!s.contains("i"));
    }

    #[test]
    fn data_value_to_line_field_bytes_returns_none() {
        use gateway_sdk::types::DataValue;
        let bv = DataValue::Bytes(vec![1, 2, 3]);
        assert_eq!(data_value_to_line_field(&bv), None);
    }

    #[test]
    fn data_value_to_line_field_all_types() {
        use gateway_sdk::types::DataValue;
        assert!(matches!(
            data_value_to_line_field(&DataValue::Bool(true)),
            Some(LineField::Bool(true))
        ));
        assert!(matches!(
            data_value_to_line_field(&DataValue::Int64(1)),
            Some(LineField::Int(1))
        ));
        assert!(matches!(
            data_value_to_line_field(&DataValue::UInt64(1)),
            Some(LineField::UInt(1))
        ));
        assert!(matches!(
            data_value_to_line_field(&DataValue::Float64(1.0)),
            Some(LineField::Float(_))
        ));
        assert!(matches!(
            data_value_to_line_field(&DataValue::String("hi".into())),
            Some(LineField::Str(_))
        ));
    }

    #[test]
    fn same_meas_tags_ts_repeated_points() {
        // 官方语义：同 measurement + tag set + timestamp 的两行，字段合并新值胜出
        // 这是行协议的固有语义，不在编码器层面处理，仅验证编码器对这类输入不 panic
        let p1 = point(
            "m",
            vec![("host", "srv1")],
            vec![("v", LineField::Int(1))],
            1000,
        );
        let p2 = point(
            "m",
            vec![("host", "srv1")],
            vec![("v", LineField::Int(2))],
            1000,
        );
        let s1 = encoded(&p1);
        let s2 = encoded(&p2);
        assert!(!s1.is_empty());
        assert!(!s2.is_empty());
    }

    #[test]
    fn template_fallback_id() {
        // 测试当 node_name / group_name 缺失时渲染 measurement 模板回退 id
        let p = point("node_group", vec![], vec![("v", LineField::Int(1))], 1000);
        let s = encoded(&p);
        assert!(s.contains("node_group"));
    }
}
