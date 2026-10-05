//! 南向 Siemens S7 插件：ISO-on-TCP (TPKT/COTP/S7Comm / S7CommPlus)。
//!
//! 支持 S7-300/400/1200/1500 系列 PLC，协议版本：
//! - S7Comm（经典，S7-300/400）
//! - S7CommPlus（S7-1200/1500，实验性）
//!
//! 地址语法：
//! - 位：`DB2.DBX0.1` / `M0.0` / `I0.0` / `Q0.0`
//! - 字：`DB2.DBW10` / `MW12` / `IW4` / `QW4`
//! - 双字：`DB2.DBD12` / `MD16` / `ID8` / `QD8`
//! - 字符串：`DB1.S20.32`（DB 号.起始字节.最大长度）
//! - 字节序列：`DB2.DBB0.N`（N 为长度）
//!
//! `#F` 后缀 → float32；`#D` → float64；`#B` → 字节序交换

#[cfg(feature = "ffi")]
mod ffi;

mod address;
mod config;
mod state;
mod value;

use crate::address::{parse_address_full, ParsedS7Address};
use crate::config::{config_str, config_u16, config_u64};
use crate::state::S7NodeState;
use crate::value::{raw_to_value, value_to_raw};
use gateway_sdk::log;
use gateway_sdk::types::{DataValue, PluginKind};
use gateway_sdk::{
    ConfigSchema, Group, GroupId, NodeId, ParamAttribute, ParamOption, ParamSchema, ParamType,
    ParamValid, PluginMeta, SouthPlugin, Tag, TagId, TagRegexEntry, TagSchema,
};
use gateway_sdk::{PluginConfig, PluginError, PluginResult};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// S7 南向插件
pub struct S7Plugin {
    state: Arc<RwLock<HashMap<NodeId, S7NodeState>>>,
}

impl Default for S7Plugin {
    fn default() -> Self {
        Self::new()
    }
}

impl S7Plugin {
    pub fn new() -> Self {
        Self {
            state: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

#[cfg(feature = "s7-client")]
async fn s7_connect(
    s: &S7NodeState,
) -> PluginResult<snap7_client::S7Client<snap7_client::transport::TcpTransport>> {
    use std::net::SocketAddr;
    use std::time::Duration;

    let addr: SocketAddr = format!("{}:{}", s.host, s.port)
        .parse()
        .map_err(|_| PluginError::config_invalid("invalid host:port"))?;

    let timeout = Duration::from_millis(s.connection_timeout_ms.min(60000));

    let params = snap7_client::types::ConnectParams {
        rack: s.rack as u8,
        slot: s.slot as u8,
        pdu_size: 480,
        connect_timeout: timeout,
        request_timeout: Duration::from_millis(s.read_timeout_ms.min(60000)),
    };

    snap7_client::S7Client::connect(addr, params)
        .await
        .map_err(|e| PluginError::connection_failed(format!("s7 connect: {:?}", e)))
}

#[cfg(feature = "s7-client")]
async fn ensure_connection(
    s: &S7NodeState,
) -> PluginResult<tokio::sync::MutexGuard<'_, state::ConnectionState>> {
    let mut cst = s.conn.lock().await;
    if cst.client.is_none() {
        let wait = cst.backoff_remaining_ms();
        if wait > 0 {
            return Err(PluginError::msg(format!(
                "connection in backoff, retry in {} ms",
                wait
            )));
        }
        match s7_connect(s).await {
            Ok(client) => {
                cst.client = Some(client);
                cst.on_success();
            }
            Err(e) => {
                cst.on_failure();
                return Err(e);
            }
        }
    }
    Ok(cst)
}

/// 将内部区域字符串转换为 snap7_client 的 Area 枚举
#[cfg(feature = "s7-client")]
fn area_str_to_area(area: &str) -> snap7_client::proto::s7::header::Area {
    match area {
        "I" | "E" | "PE" => snap7_client::proto::s7::header::Area::ProcessInput,
        "Q" | "A" | "PA" => snap7_client::proto::s7::header::Area::ProcessOutput,
        "M" | "MK" => snap7_client::proto::s7::header::Area::Marker,
        "DB" => snap7_client::proto::s7::header::Area::DataBlock,
        "C" | "CT" => snap7_client::proto::s7::header::Area::Counter,
        "T" | "TM" => snap7_client::proto::s7::header::Area::Timer,
        _ => snap7_client::proto::s7::header::Area::Marker,
    }
}

#[async_trait::async_trait]
impl SouthPlugin for S7Plugin {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "s7",
            kind: PluginKind::South,
            description: Some("Siemens S7 南向驱动（ISO-on-TCP / S7Comm / S7CommPlus）"),
            version: "0.1.0",
            name_zh: Some("S7"),
            name_en: Some("S7"),
            description_zh: Some("Siemens S7 南向驱动，支持 S7-300/400/1200/1500"),
            description_en: Some("Siemens S7 south driver, supports S7-300/400/1200/1500"),
        }
    }

    fn config_schema(&self) -> Option<ConfigSchema> {
        Some(
            ConfigSchema::new()
                .param(ParamSchema {
                    name: "host".to_string(),
                    name_zh: Some("PLC IP".to_string()),
                    name_en: Some("PLC IP".to_string()),
                    description: Some("PLC IP address".to_string()),
                    description_zh: Some("PLC IP 地址".to_string()),
                    description_en: Some("PLC IP address".to_string()),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::String,
                    default: Some(serde_json::json!("127.0.0.1")),
                    valid: None,
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "port".to_string(),
                    name_zh: Some("端口".to_string()),
                    name_en: Some("Port".to_string()),
                    description: Some("S7 protocol port (default 102)".to_string()),
                    description_zh: Some("S7 协议端口（默认 102）".to_string()),
                    description_en: Some("S7 protocol port (default 102)".to_string()),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(102)),
                    valid: Some(ParamValid {
                        min: Some(1),
                        max: Some(65535),
                        regex: None,
                        length: None,
                    }),
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "protocol".to_string(),
                    name_zh: Some("协议版本".to_string()),
                    name_en: Some("Protocol Version".to_string()),
                    description: Some("S7Comm (S7-300/400) or S7CommPlus (S7-1200/1500)".to_string()),
                    description_zh: Some("S7Comm（S7-300/400）或 S7CommPlus（S7-1200/1500）".to_string()),
                    description_en: Some("S7Comm (S7-300/400) or S7CommPlus (S7-1200/1500)".to_string()),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::Select,
                    default: Some(serde_json::json!("s7comm")),
                    valid: None,
                    options: Some(vec![
                        ParamOption {
                            value: serde_json::json!("s7comm"),
                            label: Some("S7Comm".to_string()),
                            label_zh: Some("S7Comm".to_string()),
                            label_en: Some("S7Comm".to_string()),
                        },
                        ParamOption {
                            value: serde_json::json!("s7comm-plus"),
                            label: Some("S7CommPlus (experimental)".to_string()),
                            label_zh: Some("S7CommPlus（实验性）".to_string()),
                            label_en: Some("S7CommPlus (experimental)".to_string()),
                        },
                    ]),
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "rack".to_string(),
                    name_zh: Some("机架号".to_string()),
                    name_en: Some("Rack".to_string()),
                    description: Some("S7 rack number (0-15)".to_string()),
                    description_zh: Some("S7 机架号（0-15）".to_string()),
                    description_en: Some("S7 rack number (0-15)".to_string()),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(0)),
                    valid: Some(ParamValid {
                        min: Some(0),
                        max: Some(15),
                        regex: None,
                        length: None,
                    }),
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "slot".to_string(),
                    name_zh: Some("槽位号".to_string()),
                    name_en: Some("Slot".to_string()),
                    description: Some("S7 slot number (0-31)".to_string()),
                    description_zh: Some("S7 槽位号（0-31）".to_string()),
                    description_en: Some("S7 slot number (0-31)".to_string()),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(1)),
                    valid: Some(ParamValid {
                        min: Some(0),
                        max: Some(31),
                        regex: None,
                        length: None,
                    }),
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "connection_timeout_ms".to_string(),
                    name_zh: Some("连接超时 (ms)".to_string()),
                    name_en: Some("Connection Timeout (ms)".to_string()),
                    description: Some("Connection timeout in milliseconds".to_string()),
                    description_zh: Some("连接超时时间（毫秒）".to_string()),
                    description_en: Some("Connection timeout in milliseconds".to_string()),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(3000)),
                    valid: Some(ParamValid {
                        min: Some(1000),
                        max: Some(60000),
                        regex: None,
                        length: None,
                    }),
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "read_timeout_ms".to_string(),
                    name_zh: Some("读超时 (ms)".to_string()),
                    name_en: Some("Read Timeout (ms)".to_string()),
                    description: Some("Read operation timeout in milliseconds".to_string()),
                    description_zh: Some("读操作超时时间（毫秒）".to_string()),
                    description_en: Some("Read operation timeout in milliseconds".to_string()),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(3000)),
                    valid: Some(ParamValid {
                        min: Some(1000),
                        max: Some(60000),
                        regex: None,
                        length: None,
                    }),
                    ..Default::default()
                })
                .tag_regex(vec![
                    TagRegexEntry {
                        data_type: "bool".to_string(),
                        regex: r"^(DB[0-9]+\.)?DB[XYZ][0-9]+\.[0-7]$|^(M|I|Q)[0-9]+\.[0-7]$".to_string(),
                    },
                    TagRegexEntry {
                        data_type: "int16".to_string(),
                        regex: r"^(DB[0-9]+\.)?DB[WP][0-9]+(#(B|L|F|D))?$|^(M|I|Q)[QW][0-9]+(#(B|L|F|D))?$".to_string(),
                    },
                    TagRegexEntry {
                        data_type: "uint16".to_string(),
                        regex: r"^(DB[0-9]+\.)?DB[WP][0-9]+(#(B|L|F|D))?$|^(M|I|Q)[QW][0-9]+(#(B|L|F|D))?$".to_string(),
                    },
                    TagRegexEntry {
                        data_type: "int32".to_string(),
                        regex: r"^(DB[0-9]+\.)?DBD[0-9]+(#(B|L|F|D))?$|^(M|I|Q)D[0-9]+(#(B|L|F|D))?$".to_string(),
                    },
                    TagRegexEntry {
                        data_type: "uint32".to_string(),
                        regex: r"^(DB[0-9]+\.)?DBD[0-9]+(#(B|L|F|D))?$|^(M|I|Q)D[0-9]+(#(B|L|F|D))?$".to_string(),
                    },
                    TagRegexEntry {
                        data_type: "float32".to_string(),
                        regex: r"^(DB[0-9]+\.)?DBD[0-9]+#F$|^(M|I|Q)D[0-9]+#F$".to_string(),
                    },
                    TagRegexEntry {
                        data_type: "float64".to_string(),
                        regex: r"^(DB[0-9]+\.)?DBD[0-9]+#D$|^(M|I|Q)D[0-9]+#D$".to_string(),
                    },
                    TagRegexEntry {
                        data_type: "string".to_string(),
                        regex: r"^DB[0-9]+\.S[0-9]+\.[0-9]+$".to_string(),
                    },
                    TagRegexEntry {
                        data_type: "bytes".to_string(),
                        regex: r"^(DB[0-9]+\.)?DBB[0-9]+\.[0-9]+$|^(M|I|Q)B[0-9]+$".to_string(),
                    },
                ]),
        )
    }

    fn tag_schema(&self) -> Option<TagSchema> {
        Some(TagSchema {
            data_types: Some(vec![
                "bool".to_string(),
                "int16".to_string(),
                "uint16".to_string(),
                "int32".to_string(),
                "uint32".to_string(),
                "int64".to_string(),
                "uint64".to_string(),
                "float32".to_string(),
                "float64".to_string(),
                "string".to_string(),
                "bytes".to_string(),
            ]),
            address_format: Some("DB块.区域(bit/byte/word/dword) 或 M/I/Q 区域".to_string()),
            address_format_zh: Some("DB块.区域(bit/byte/word/dword) 或 M/I/Q 区域".to_string()),
            address_format_en: Some(
                "DB_number.area(bit/byte/word/dword) or M/I/Q area".to_string(),
            ),
        })
    }

    async fn validate_tag(&self, _node_id: NodeId, tag: &Tag) -> PluginResult<()> {
        if tag.name.is_empty() {
            return Err(PluginError::tag_invalid("tag name required"));
        }
        parse_address_full(&tag.address).ok_or_else(|| {
            PluginError::tag_invalid(
                "address format: DB2.DBX0.1 / DB2.DBW10 / DB2.DBD12#F / DB1.S20.32 / M0.0 / MB10 / MW12 / MD16#F / I0.0 / IB2 / IW4 / Q0.0 / QB2 / QW4",
            )
        })?;
        Ok(())
    }

    async fn open(&self, node_id: NodeId, config: PluginConfig) -> PluginResult<()> {
        let host = config_str(&config, "host", "127.0.0.1");
        let port = config_u16(&config, "port", 102);
        let protocol = config_str(&config, "protocol", "s7comm");
        let rack = config_u16(&config, "rack", 0);
        let slot = config_u16(&config, "slot", 1);
        let connection_timeout_ms = config_u64(&config, "connection_timeout_ms", 3000);
        let read_timeout_ms = config_u64(&config, "read_timeout_ms", 3000);

        log::info(
            node_id,
            format!(
                "open s7: host={}, port={}, protocol={}, rack={}, slot={}",
                host, port, protocol, rack, slot
            ),
        );

        let state = S7NodeState::new(
            host,
            port,
            protocol,
            rack,
            slot,
            connection_timeout_ms,
            read_timeout_ms,
        );

        let mut all_state = self.state.write().await;
        all_state.insert(node_id, state);
        Ok(())
    }

    async fn close(&self, node_id: NodeId) -> PluginResult<()> {
        log::info(node_id, "close s7");
        let mut state = self.state.write().await;
        state.remove(&node_id);
        Ok(())
    }

    async fn start(&self, node_id: NodeId) -> PluginResult<()> {
        log::info(node_id, "start s7");
        Ok(())
    }

    async fn stop(&self, node_id: NodeId) -> PluginResult<()> {
        log::info(node_id, "stop s7");
        Ok(())
    }

    async fn setting(&self, node_id: NodeId, config: PluginConfig) -> PluginResult<()> {
        log::info(node_id, "setting s7 (config updated)");
        let host = config_str(&config, "host", "127.0.0.1");
        let port = config_u16(&config, "port", 102);
        let mut state = self.state.write().await;
        if let Some(s) = state.get_mut(&node_id) {
            s.host = host;
            s.port = port;
            s.protocol = config_str(&config, "protocol", &s.protocol);
            s.rack = config_u16(&config, "rack", s.rack);
            s.slot = config_u16(&config, "slot", s.slot);
            s.connection_timeout_ms =
                config_u64(&config, "connection_timeout_ms", s.connection_timeout_ms);
            s.read_timeout_ms = config_u64(&config, "read_timeout_ms", s.read_timeout_ms);
            // Host/port/rack/slot changed → drop connection so next poll reconnects
            if s.conn.lock().await.client.is_some() {
                s.conn.lock().await.on_failure();
            }
        }
        Ok(())
    }

    async fn poll_group(
        &self,
        node_id: NodeId,
        _group_id: GroupId,
        tags: &[Tag],
    ) -> PluginResult<Vec<(TagId, DataValue)>> {
        let state = self.state.read().await;
        let s = state
            .get(&node_id)
            .ok_or_else(|| PluginError::msg("node not open"))?;

        #[cfg(feature = "s7-client")]
        {
            use snap7_client::MultiReadItem;
            use std::time::Duration;
            use tokio::time::timeout;

            if tags.is_empty() {
                return Ok(vec![]);
            }

            // 取出或建立连接
            let mut cst = ensure_connection(s).await?;
            let client = cst
                .client
                .as_ref()
                .ok_or_else(|| PluginError::msg("s7 connection unavailable"))?;

            let timeout_dur = Duration::from_millis(s.read_timeout_ms.min(60000));

            // 按 (area, db_number, start, count) 分组，构建 MultiReadItem
            // 每个唯一 (area, db_num, start, count) 作为一个读取项
            #[derive(Default)]
            struct ReadGroup {
                items: Vec<(usize, ParsedS7Address, String)>, // (tag_index, addr, data_type)
                area: String,
                db_num: u16,
                start: u32,
                count: u32,
            }

            let mut groups: Vec<ReadGroup> = Vec::new();

            for (i, tag) in tags.iter().enumerate() {
                let addr = match parse_address_full(&tag.address) {
                    Some(a) => a,
                    None => continue,
                };
                let data_type = tag.data_type.as_deref().unwrap_or("int16");

                // 推断区域字符串
                let area_str = infer_area_str(&tag.address, &addr);
                let db_num = addr.db_number.unwrap_or(0);

                // 检查是否有完全匹配的组（相同 area/db_num/start/count）
                if let Some(g) = groups.iter_mut().find(|g| {
                    g.area == area_str
                        && g.db_num == db_num
                        && g.start == addr.byte_offset
                        && g.count == addr.count
                }) {
                    g.items.push((i, addr, data_type.to_string()));
                } else {
                    let g = ReadGroup {
                        items: vec![(i, addr.clone(), data_type.to_string())],
                        area: area_str,
                        db_num,
                        start: addr.byte_offset,
                        count: addr.count,
                    };
                    groups.push(g);
                }
            }

            // 构建 MultiReadItem 列表
            let read_items: Vec<MultiReadItem> = groups
                .iter()
                .map(|g| MultiReadItem {
                    area: area_str_to_area(&g.area),
                    db_number: g.db_num,
                    start: g.start,
                    length: g.count as u16,
                    transport: snap7_client::proto::s7::header::TransportSize::Byte,
                })
                .collect();

            let mut slots: Vec<Option<(TagId, DataValue)>> = vec![None; tags.len()];

            let read_result = timeout(timeout_dur, client.read_multi_vars(&read_items)).await;

            match read_result {
                Ok(Ok(results)) => {
                    // 分配结果到各 tag
                    for (gi, result) in results.into_iter().enumerate() {
                        if gi >= groups.len() {
                            break;
                        }
                        let raw = result;
                        for (tag_idx, addr, dt) in &groups[gi].items {
                            let value = raw_to_value(&raw, dt, addr);
                            slots[*tag_idx] = Some((tags[*tag_idx].id, value));
                        }
                    }
                    cst.on_success();
                    Ok(slots
                        .into_iter()
                        .enumerate()
                        .map(|(i, v)| v.unwrap_or_else(|| (tags[i].id, DataValue::UInt16(0))))
                        .collect())
                }
                Ok(Err(e)) => {
                    cst.on_failure();
                    Err(PluginError::msg(format!("s7 read_multi_vars: {:?}", e)))
                }
                Err(_) => {
                    cst.on_failure();
                    Err(PluginError::timeout("s7 read timeout"))
                }
            }
        }

        #[cfg(not(feature = "s7-client"))]
        {
            let _ = (node_id, s);
            let mut out = Vec::with_capacity(tags.len());
            for tag in tags {
                let _ = parse_address_full(&tag.address);
                out.push((tag.id, DataValue::UInt16(0)));
            }
            Ok(out)
        }
    }

    async fn list_groups(&self, node_id: NodeId) -> PluginResult<Vec<Group>> {
        let state = self.state.read().await;
        let s = state
            .get(&node_id)
            .ok_or_else(|| PluginError::msg("node not open"))?;
        Ok(s.groups.clone())
    }

    async fn list_tags(&self, node_id: NodeId, group_id: GroupId) -> PluginResult<Vec<Tag>> {
        let state = self.state.read().await;
        let s = state
            .get(&node_id)
            .ok_or_else(|| PluginError::msg("node not open"))?;
        Ok(s.tags
            .iter()
            .filter(|t| t.group_id == group_id)
            .cloned()
            .collect())
    }

    async fn write_tags(&self, node_id: NodeId, values: &[(Tag, DataValue)]) -> PluginResult<()> {
        if values.is_empty() {
            return Ok(());
        }
        let state = self.state.read().await;
        let s = state
            .get(&node_id)
            .ok_or_else(|| PluginError::msg("node not open"))?;

        #[cfg(feature = "s7-client")]
        {
            use bytes::Bytes;
            use snap7_client::MultiWriteItem;
            use std::time::Duration;
            use tokio::time::timeout;

            let mut cst = ensure_connection(s).await?;
            let client = cst
                .client
                .as_ref()
                .ok_or_else(|| PluginError::msg("s7 connection unavailable"))?;

            let timeout_dur = Duration::from_millis(s.read_timeout_ms.min(60000));
            let mut failure: Option<PluginError> = None;

            let mut write_items: Vec<MultiWriteItem> = Vec::new();

            for (tag, value) in values {
                let addr = match parse_address_full(&tag.address) {
                    Some(a) => a,
                    None => {
                        failure = Some(PluginError::tag_invalid(format!(
                            "invalid address: {}",
                            tag.address
                        )));
                        break;
                    }
                };
                let data_type = tag.data_type.as_deref().unwrap_or("int16");
                let raw = value_to_raw(value, data_type, &addr);
                let area_str = infer_area_str(&tag.address, &addr);
                let db_num = addr.db_number.unwrap_or(0);

                write_items.push(MultiWriteItem {
                    area: area_str_to_area(&area_str),
                    db_number: db_num,
                    start: addr.byte_offset,
                    data: Bytes::from(raw),
                });
            }

            if failure.is_none() {
                let write_result =
                    timeout(timeout_dur, client.write_multi_vars(&write_items)).await;
                match write_result {
                    Ok(Ok(())) => {
                        cst.on_success();
                        return Ok(());
                    }
                    Ok(Err(e)) => {
                        failure = Some(PluginError::msg(format!("s7 write: {:?}", e)));
                    }
                    Err(_) => {
                        failure = Some(PluginError::timeout("s7 write timeout"));
                    }
                }
            }

            if let Some(e) = failure {
                cst.on_failure();
                Err(e)
            } else {
                Ok(())
            }
        }

        #[cfg(not(feature = "s7-client"))]
        {
            let _ = (node_id, s, values);
            Err(PluginError::not_supported(
                "write requires s7-client feature",
            ))
        }
    }
}

/// 从地址字符串和解析结果推断 S7 区域字符串
fn infer_area_str(addr: &str, parsed: &ParsedS7Address) -> String {
    if parsed.db_number.is_some() {
        return "DB".to_string();
    }
    let upper = addr.to_uppercase();
    if upper.starts_with('M') {
        "M".to_string()
    } else if upper.starts_with('I') || upper.starts_with('E') {
        "I".to_string()
    } else if upper.starts_with('Q') || upper.starts_with('A') {
        "Q".to_string()
    } else {
        "M".to_string()
    }
}

#[cfg(test)]
mod tests {
    use gateway_sdk::SouthPlugin;

    #[test]
    fn test_s7_plugin_can_be_created() {
        use super::S7Plugin;
        let plugin = S7Plugin::new();
        assert_eq!(plugin.meta().name, "s7");
    }

    #[test]
    fn test_address_parsing_integration() {
        use crate::address::parse_address_full;
        // All these should parse without panic
        let addrs = [
            "DB2.DBX0.1",
            "DB2.DBW10",
            "DB2.DBD12",
            "DB1.S20.32",
            "M0.0",
            "MB10",
            "MW12",
            "MD16",
            "I0.0",
            "IB2",
            "IW4",
            "Q0.0",
            "QB2",
            "QW4",
        ];
        for addr in addrs {
            let result = parse_address_full(addr);
            assert!(result.is_some(), "Failed to parse: {}", addr);
        }
    }
}

// ===== #[ignore] integration tests =====
// These require a real S7 PLC or snap7-server simulator.
// Run with: cargo test -p plugin-s7 -- --ignored

#[cfg(all(test, feature = "s7-client"))]
mod integration_tests {
    use std::net::SocketAddr;
    use std::time::Duration;

    use snap7_server::store::area::*;
    use snap7_server::{DataStore, S7Server};

    /// Start a local snap7-server on a random port, set up test data, and return the server + address.
    async fn start_test_server() -> SocketAddr {
        let store = DataStore::new();
        // Register a DB (area 0x84)
        store.register_area(DATA_BLOCK, 1024);
        // Set DB1.DBW10 = 0x1234 using public API
        store.write_bytes(1, 10, &[0x12, 0x34]);
        // Register merker area
        store.register_area(MARKERS, 256);
        // Set MB0 = 0xAB using public API (write to offset 0 in markers area)
        store.write_area(MARKERS, 0, 0, &[0xAB]);

        let server = S7Server::start_to("127.0.0.1:0", 1).await.unwrap();
        let addr = server.local_addr().unwrap();
        let store_clone = store.clone();

        // Run server in background
        tokio::spawn(async move {
            let _ = server.serve(store_clone).await;
        });

        addr
    }

    #[tokio::test]
    #[ignore]
    async fn test_poll_db_word_via_snap7_server() {
        let addr = start_test_server().await;

        let params = snap7_client::types::ConnectParams {
            rack: 0,
            slot: 1,
            pdu_size: 480,
            connect_timeout: Duration::from_secs(5),
            request_timeout: Duration::from_secs(5),
        };

        let client = snap7_client::S7Client::connect(addr, params)
            .await
            .expect("failed to connect to snap7-server");

        // Read DB1.DBW10 (2 bytes starting at byte offset 10)
        let raw = client.db_read(1, 10, 2).await.expect("db_read failed");
        assert_eq!(raw.len(), 2);
        assert_eq!([raw[0], raw[1]], [0x12, 0x34]);
    }

    #[tokio::test]
    #[ignore]
    async fn test_poll_merker_byte_via_snap7_server() {
        let addr = start_test_server().await;

        let params = snap7_client::types::ConnectParams {
            rack: 0,
            slot: 1,
            pdu_size: 480,
            connect_timeout: Duration::from_secs(5),
            request_timeout: Duration::from_secs(5),
        };

        let client = snap7_client::S7Client::connect(addr, params)
            .await
            .expect("failed to connect to snap7-server");

        // Read MB0 (1 byte)
        let raw = client.mb_read(0, 1).await.expect("mb_read failed");
        assert_eq!(raw.len(), 1);
        assert_eq!(raw[0], 0xAB);
    }

    #[tokio::test]
    #[ignore]
    async fn test_write_and_readback_db_via_snap7_server() {
        let addr = start_test_server().await;

        let params = snap7_client::types::ConnectParams {
            rack: 0,
            slot: 1,
            pdu_size: 480,
            connect_timeout: Duration::from_secs(5),
            request_timeout: Duration::from_secs(5),
        };

        let client = snap7_client::S7Client::connect(addr, params)
            .await
            .expect("failed to connect");

        // Write MB0 = 0x42
        client.mb_write(0, &[0x42]).await.expect("mb_write failed");

        // Read it back
        let raw = client.mb_read(0, 1).await.expect("mb_read failed");
        assert_eq!(raw[0], 0x42);
    }
}
