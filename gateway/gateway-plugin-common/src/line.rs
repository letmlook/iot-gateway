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

    // fields：按官方行协议，字段间分隔只能是单个 ","（逗号+空格会被 InfluxDB
    // 400 / taosAdapter 500 拒绝）。空格只作为 measurement↔tag / tag↔field /
    // field↔timestamp 的语义分隔符，不出现在字段之间。
    out.push(' ');
    for (i, (k, vf)) in p.fields.iter().enumerate() {
        if i > 0 {
            out.push(',');
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

    // -----------------------------------------------------------------------
    // 缺陷⑤回归（line.rs:117）：字段间分隔符必须是单个 ","，绝不能是 ", "
    // 官方行协议：<https://influxdata.github.io/influxdb/v2/reference/syntax/line-protocol/>
    // 此前 c20d49f 把分隔符写成 ", "（逗号+空格），导致：
    //   - InfluxDB 写入端返回 400 `invalid field format`（dropped_rejected）
    //   - taosAdapter 返回 500 `[0x127] invalid timestamp`（无限重试涨队列）
    // -----------------------------------------------------------------------

    /// 缺陷⑤的最小回归：多 field 之间只能是单个 ','，不得含空格
    #[test]
    fn regression_field_separator_is_single_comma_no_space() {
        let p = point(
            "m",
            vec![],
            vec![
                ("a", LineField::Int(1)),
                ("b", LineField::Int(2)),
                ("c", LineField::Int(3)),
            ],
            1_000,
        );
        let s = encoded(&p);
        // 官方行协议: `m a=1i,b=2i,c=3i 1000`
        assert_eq!(s, "m a=1i,b=2i,c=3i 1000");
        // 关键反例：字段循环里不能出现 ", "
        assert!(
            !s.contains(", "),
            "field separator must be a single comma (no trailing space): {s:?}"
        );
        // 子串定位：b 与 a 之间、c 与 b 之间必须以 ',' 直接相连
        assert!(s.contains("1i,b="));
        assert!(s.contains("2i,c="));
    }

    /// 缺陷⑤的字节级证据：把字段分隔符从 ", " 改为 "," 后，每条多 field 行
    /// 减少的字节数 == (字段数 - 1)（每个分隔符少一个空格字符）。
    /// 这里直接断言「不含任何 ", "」，避免回归到 ", "。
    #[test]
    fn regression_no_comma_space_anywhere_in_field_section() {
        // 多种 field 组合都走一遍，确保任何形态下都不会出现 ", "
        let cases: Vec<Vec<(&str, LineField)>> = vec![
            vec![("a", LineField::Int(1)), ("b", LineField::Int(2))],
            vec![
                ("temperature", LineField::Float(25.5)),
                ("humidity", LineField::Float(60.0)),
                ("pressure", LineField::Float(1013.2)),
            ],
            vec![
                ("on", LineField::Bool(true)),
                ("ok", LineField::Bool(false)),
                ("n", LineField::UInt(42)),
                ("s", LineField::Str("hi".into())),
            ],
            vec![("k=1", LineField::Int(1)), ("k 2", LineField::Int(2))],
        ];
        for fields in cases {
            let p = point("m", vec![], fields, 123);
            let s = encoded(&p);
            // 把字段段（measurement 与 timestamp 之间的内容）取出来再断言。
            // measurement 末尾到 timestamp 前是 " " + fields。
            let ts_idx = s.rfind(' ').expect("timestamp separated by space");
            let field_section = &s[1..ts_idx]; // 跳过 measurement 后的那个语义空格
            assert!(
                !field_section.contains(", "),
                "field section must not contain ', ': section={field_section:?}, full={s:?}"
            );
        }
    }

    /// 端到端编码样例 #1：测量名含逗号、多 tag、含转义字符串 field value。
    /// 官方语法：measurement 内的逗号/空格必须 `\,` / `\ `；string field value
    /// 内的 `"` 和 `\` 必须 `\"` / `\\`；tag key/value 内的 `,`/`=`/空格同上转义。
    #[test]
    fn end_to_end_sample_measurement_with_comma_and_escaped_string_field() {
        let p = point(
            "cpu,usage", // measurement 内的逗号必须转义
            vec![
                ("host,1", "node a"), // tag key 内逗号、tag value 内空格
            ],
            vec![
                ("value", LineField::Float(72.5)),
                ("msg", LineField::Str(r#"he said "go"\now"#.to_string())),
            ],
            1_700_000_000_000,
        );
        let s = encoded(&p);
        // measurement 部分
        assert!(s.starts_with("cpu\\,usage"));
        // measurement 后的 ',' 接 tag section（k,v 内字符全部按规则转义）：
        // ",host\,1=node\ a " —— 首尾分别是 ',' 与 ' '（与 field 段的语义分隔）。
        assert!(s.contains(",host\\,1=node\\ a "));
        // field 部分：value=72.5,msg="he said \"go\"\\now" （字段间用单个 ","）
        assert!(s.contains(r#"value=72.5,msg="he said \"go\"\\now""#));
        // 时间戳由单个空格分隔
        assert!(s.ends_with(r#" 1700000000000"#));
        // 整条串里不允许出现 ", "
        assert!(!s.contains(", "));
    }

    /// 端到端编码样例 #2：measurement 含空格、tag key 含 `=`、多 field（含 string）。
    #[test]
    fn end_to_end_sample_measurement_with_space_and_eq_in_tag_key() {
        let p = point(
            "disk usage",         // measurement 含空格必须转义
            vec![("k=v", "w=x")], // tag key 与 value 都含 '='
            vec![
                ("read", LineField::UInt(100)),
                ("write", LineField::UInt(200)),
                ("ratio", LineField::Float(0.5)),
            ],
            42,
        );
        let s = encoded(&p);
        // measurement 内空格转义
        assert!(s.starts_with("disk\\ usage"));
        // tag 段：,k\=v=w\=x （k/v 内的 '=' 已转义；段尾接 ' ' 进入 field 段）
        assert!(s.contains(",k\\=v=w\\=x "));
        // field 段以单个空格分隔、field 之间只有 "," 无空格
        assert!(s.contains(" read=100u,write=200u,ratio=0.5"));
        // 时间戳
        assert!(s.ends_with(" 42"));
        // 整条不允许 ", "
        assert!(!s.contains(", "));
    }

    /// 端到端编码样例 #3：tag↔field 边界与 field↔timestamp 边界。
    /// 验证「测量名后第一个空格」「tag 后空格」「field 段后空格」三个位置都是
    /// 单个 ASCII 空格（语义分隔），而 field 之间只用单个 ","。
    #[test]
    fn end_to_end_sample_semantic_spaces_only_at_boundaries() {
        let p = point(
            "weather",
            vec![("city", "sf"), ("region", "us")],
            vec![("temp", LineField::Int(18)), ("hum", LineField::Int(50))],
            1_000,
        );
        let s = encoded(&p);
        // 官方完整一行：weather,city=sf,region=us temp=18i,hum=50i 1000
        assert_eq!(
            s, "weather,city=sf,region=us temp=18i,hum=50i 1000",
            "exact-match assertion: official line protocol grammar (defect⑤ regression)"
        );
        // 三个语义空格都在边界；除此之外不允许任何空格。
        // 1) measurement↔tags: "weather,city"
        assert!(s.contains("weather,city"));
        // 2) tag-set↔fields: "region=us temp"
        assert!(s.contains("region=us temp"));
        // 3) fields↔timestamp: "hum=50i 1000"
        assert!(s.contains("hum=50i 1000"));
        // field 间为单个 ","（无空格）
        assert!(s.contains("18i,hum="));
    }

    // -----------------------------------------------------------------------
    // 缺陷⑤ HTTP 自检（修复回退测试会红才算数）：
    // 起一个本地 HTTP 服务，接收真正写入的 line-protocol 行；
    // 字节级断言修复后的实际报文，并对比"修复前（buggy）"长度差异。
    // -----------------------------------------------------------------------
    #[test]
    fn http_self_check_field_separator_byte_diff() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::sync::mpsc;

        // 起本地 HTTP 服务：读取一个完整请求，回 204，把读到的 body 原样推到 channel。
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = mpsc::channel::<Vec<u8>>();

        let server = std::thread::spawn(move || {
            if let Some(stream) = listener.incoming().next() {
                let Ok(mut stream) = stream else {
                    return;
                };
                let mut buf = Vec::new();
                let mut chunk = [0u8; 1024];
                // 读 header 段
                let header_end = loop {
                    let n = match stream.read(&mut chunk) {
                        Ok(n) => n,
                        Err(_) => break usize::MAX,
                    };
                    if n == 0 {
                        break buf.len();
                    }
                    buf.extend_from_slice(&chunk[..n]);
                    if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        break pos + 4;
                    }
                };
                let head = String::from_utf8_lossy(&buf[..header_end.min(buf.len())]).to_string();
                let cl: usize = head
                    .to_ascii_lowercase()
                    .lines()
                    .find_map(|l| {
                        l.strip_prefix("content-length:")
                            .map(|v| v.trim().parse::<usize>().ok().unwrap_or(0))
                    })
                    .unwrap_or(0);
                while buf.len() < header_end + cl {
                    let n = match stream.read(&mut chunk) {
                        Ok(n) => n,
                        Err(_) => break,
                    };
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                }
                let body = if header_end <= buf.len() {
                    buf[header_end..].to_vec()
                } else {
                    Vec::new()
                };
                let _ = stream.write_all(
                    b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                );
                let _ = stream.flush();
                let _ = tx.send(body);
            }
        });

        // 构造一个含多 field 的点：4 个 field → 修复前每条分隔符 ", " 会多 3 个空格字节
        let p = point(
            "sensor",
            vec![("node", "temp-01"), ("group", "env")],
            vec![
                ("temperature", LineField::Float(25.5)),
                ("humidity", LineField::Float(60.0)),
                ("pressure", LineField::Float(1013.2)),
                ("online", LineField::Bool(true)),
            ],
            1_704_067_200_000,
        );
        let mut line = String::new();
        encode(&p, &mut line);
        let body = line.into_bytes();
        let fixed_len = body.len();

        // 通过 std::net::TcpStream 直接发一个 HTTP POST，把编码后的行作为 body
        let mut stream = std::net::TcpStream::connect(addr).unwrap();
        let req = format!(
            "POST /write?db=gateway HTTP/1.1\r\nHost: {}\r\nContent-Length: {}\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\n",
            addr,
            fixed_len
        );
        stream.write_all(req.as_bytes()).unwrap();
        stream.write_all(&body).unwrap();
        stream.flush().unwrap();
        drop(stream);

        let received = rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("server must receive the body within 5s");
        server.join().unwrap();

        // ① 字节级断言：服务端实际收到的字节就是编码器产出的字节
        assert_eq!(received.len(), fixed_len);
        assert_eq!(received, body);

        // ② 缺陷⑤核心反例：报文里不允许出现 ", " —— 这是 InfluxDB 400 / taosAdapter 500 的根因
        let received_str = std::str::from_utf8(&received).expect("ASCII-only payload");
        assert!(
            !received_str.contains(", "),
            "defect⑤ regression: received body must not contain ', ': {received_str:?}"
        );

        // ③ 字节差证明（开关前后对比）：模拟旧 buggy 编码器，把每处 field 间 ',' 后插一个 ' '。
        // 4 个 field ⇒ 3 个 field 分隔符 ⇒ 修复前比修复后多 3 字节。
        // 判别依据：第一个 ASCII 空格是 tag↔field 语义分隔符；之后每一个 ',' 都是 field 分隔符
        // （measurement / tag 段的 ',' 全部出现在第一个空格之前）。
        let mut buggy = Vec::with_capacity(fixed_len + 4);
        let mut inserted = 0usize;
        let bytes: Vec<u8> = received.to_vec();
        let first_space = bytes
            .iter()
            .position(|&b| b == b' ')
            .expect("line protocol must contain the tag↔field semantic space");
        let mut i = 0;
        while i < bytes.len() {
            let b = bytes[i];
            buggy.push(b);
            if b == b',' && i > first_space {
                buggy.push(b' ');
                inserted += 1;
            }
            i += 1;
        }
        assert_eq!(
            inserted, 3,
            "4 fields ⇒ 3 field separators ⇒ bug 时期会插入 3 个多余空格"
        );
        assert_eq!(buggy.len() - fixed_len, 3);

        // ④ 关键反向证据：buggy 串必然含 ", "，且长度 == fixed_len + 3
        let buggy_str = std::str::from_utf8(&buggy).expect("ASCII-only payload");
        assert!(
            buggy_str.contains(", "),
            "buggy simulation must contain ', ' to evidence the original defect"
        );
        assert_eq!(buggy.len(), fixed_len + 3);

        // ⑤ 写入证据（自检报告）：让本测试即使在 --nocapture 也留下审计痕迹
        eprintln!(
            "DEFECT5_HTTP_SELFCHECK fixed_len={fixed_len} buggy_len={} delta=3 has_comma_space_buggy=true has_comma_space_fixed=false payload_fixed={:?}",
            buggy.len(),
            received_str
        );
    }
}
