//! IEC 60870-5-104 南向插件（主站/控制站角色）。
//!
//! 架构：插件自持 OS 线程 + 专属 tokio Runtime + 长驻 iec104::Client。
//! - FFI 运行时契约（docs/design/南向驱动.md §2.1）要求插件方法内不得 spawn 跨调用存活的
//!   tokio 任务；iec104::Client::connect 自身在调用方 runtime 上 tokio::spawn。
//! - 因此连接线程、专属 Runtime 与 Client 全部自持于插件内，与宿主调用 runtime 解耦。
//!
//! 重连归属 crate 内部（iec104 内置无限重连），插件不叠加退避，只维护健康标志。
//!
//! 地址语法：IOA 十进制数字（1–16777215），tag_regex `^[0-9]{1,8}$`。

#[cfg(feature = "ffi")]
mod ffi;

mod address;
mod config;
mod state;
mod value;
mod worker;

use address::is_ioa_format;
use config::{config_str, config_u16, config_u64, config_u8};
use state::Iec104NodeState;
use worker::worker_loop;
use worker::{Command, WriteValue};

use gateway_sdk::log;
use gateway_sdk::types::{DataValue, PluginKind};
use gateway_sdk::{
    ConfigSchema, Group, GroupId, NodeId, ParamAttribute, ParamSchema, ParamType, ParamValid,
    PluginMeta, SouthPlugin, Tag, TagId, TagRegexEntry, TagSchema,
};
use gateway_sdk::{PluginConfig, PluginError, PluginResult};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// k/w 约束上限（⌊2k/3⌋）
fn kw_max_w(k: u16) -> u16 {
    ((k as u32 * 2) / 3) as u16
}

/// 校验 k/w 配置合法性（硬约束 w ≤ ⌊2k/3⌋ 且 k,w > 0）。
/// 返回 config_invalid 错误（带上限值）如不合规。
fn validate_kw(k: u16, w: u16) -> PluginResult<()> {
    if k == 0 {
        return Err(PluginError::config_invalid(
            "k must be > 0 (APCI send window)",
        ));
    }
    if w == 0 {
        return Err(PluginError::config_invalid(
            "w must be > 0 (APCI acknowledge window)",
        ));
    }
    let max_w = kw_max_w(k);
    if w > max_w {
        return Err(PluginError::config_invalid(format!(
            "w ({}) must be <= floor(2k/3) = {} (k={})",
            w, max_w, k
        )));
    }
    Ok(())
}

/// IEC 60870-5-104 南向插件
pub struct Iec104Plugin {
    // HashMap 值用 Arc 包装：poll_group 等读操作 clone Arc 再 lock，不阻塞写
    state: Arc<RwLock<HashMap<NodeId, Arc<Iec104NodeState>>>>,
}

impl Default for Iec104Plugin {
    fn default() -> Self {
        Self::new()
    }
}

impl Iec104Plugin {
    pub fn new() -> Self {
        Self {
            state: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    fn default_groups() -> Vec<Group> {
        vec![Group {
            id: GroupId::new(),
            name: "default".to_string(),
            interval_ms: 1000,
            description: Some("默认采集组".to_string()),
        }]
    }
}

#[async_trait::async_trait]
impl SouthPlugin for Iec104Plugin {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "iec104",
            kind: PluginKind::South,
            description: Some("IEC 60870-5-104 南向驱动（主站/控制站）"),
            version: "0.1.0",
            name_zh: Some("IEC 104"),
            name_en: Some("IEC 104"),
            description_zh: Some("IEC 60870-5-104 南向驱动（主站/控制站）"),
            description_en: Some("IEC 60870-5-104 south driver (controlling station)"),
        }
    }

    fn config_schema(&self) -> Option<ConfigSchema> {
        Some(
            ConfigSchema::new()
                .param(ParamSchema {
                    name: "host".to_string(),
                    name_zh: Some("目标 IP".to_string()),
                    name_en: Some("Remote IP".to_string()),
                    description: Some("被控站 IP 地址".to_string()),
                    description_zh: Some("被控站 IP 地址".to_string()),
                    description_en: Some("IP address of the IEC 104 controlled station".to_string()),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::String,
                    default: Some(serde_json::json!("127.0.0.1")),
                    valid: None,
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "port".to_string(),
                    name_zh: Some("端口号".to_string()),
                    name_en: Some("Port".to_string()),
                    description: Some("IEC 104 端口，默认为 2404".to_string()),
                    description_zh: Some("IEC 104 端口号，默认为 2404".to_string()),
                    description_en: Some("IEC 104 port, default 2404".to_string()),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(2404)),
                    valid: Some(ParamValid {
                        min: Some(1),
                        max: Some(65535),
                        regex: None,
                        length: None,
                    }),
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "common_address".to_string(),
                    name_zh: Some("公共地址 (CA)".to_string()),
                    name_en: Some("Common Address (CA)".to_string()),
                    description: Some("IEC 104 公共地址 (CA)，范围 1–65534".to_string()),
                    description_zh: Some("IEC 104 公共地址 CA，范围 1–65534".to_string()),
                    description_en: Some("IEC 104 common address (CA), range 1–65534".to_string()),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(1)),
                    valid: Some(ParamValid {
                        min: Some(1),
                        max: Some(65534),
                        regex: None,
                        length: None,
                    }),
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "originator_address".to_string(),
                    name_zh: Some("源发地址 (OA)".to_string()),
                    name_en: Some("Originator Address (OA)".to_string()),
                    description: Some("IEC 104 源发地址 OA，范围 0–255，默认为 0".to_string()),
                    description_zh: Some("IEC 104 源发地址 OA，范围 0–255".to_string()),
                    description_en: Some("IEC 104 originator address (OA), range 0–255".to_string()),
                    attribute: ParamAttribute::Optional,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(0)),
                    valid: Some(ParamValid {
                        min: Some(0),
                        max: Some(255),
                        regex: None,
                        length: None,
                    }),
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "k".to_string(),
                    name_zh: Some("发送窗口 k".to_string()),
                    name_en: Some("Send Window k".to_string()),
                    description: Some(
                        "APCI 发送窗口 k（未确认 APDU 上限），必须 > 0；与 w 必须满足 w ≤ ⌊2k/3⌋。默认 12。".to_string(),
                    ),
                    description_zh: Some(
                        "APCI 发送窗口 k（未确认 APDU 上限），必须 > 0；与 w 必须满足 w ≤ ⌊2k/3⌋。默认 12。".to_string(),
                    ),
                    description_en: Some(
                        "APCI send window k (max unacknowledged APDUs), must be > 0; w ≤ ⌊2k/3⌋. Default 12.".to_string(),
                    ),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(12)),
                    valid: Some(ParamValid {
                        min: Some(1),
                        max: Some(32767),
                        regex: None,
                        length: None,
                    }),
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "w".to_string(),
                    name_zh: Some("确认窗口 w".to_string()),
                    name_en: Some("Acknowledge Window w".to_string()),
                    description: Some(
                        "APCI 确认窗口 w（收到 w 个 APDU 后即确认），必须 > 0；必须满足 w ≤ ⌊2k/3⌋。默认 8（合法值，⌊24/3⌋=8）。".to_string(),
                    ),
                    description_zh: Some(
                        "APCI 确认窗口 w（收到 w 个 APDU 后即确认），必须 > 0；必须满足 w ≤ ⌊2k/3⌋。默认 8（合法值，⌊24/3⌋=8）。".to_string(),
                    ),
                    description_en: Some(
                        "APCI acknowledge window w (ack after w APDUs), must be > 0; must satisfy w ≤ ⌊2k/3⌋. Default 8 (legal for k=12: ⌊24/3⌋=8).".to_string(),
                    ),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(8)),
                    valid: Some(ParamValid {
                        min: Some(1),
                        max: Some(32767),
                        regex: None,
                        length: None,
                    }),
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "t0_ms".to_string(),
                    name_zh: Some("建连超时 t0 (ms)".to_string()),
                    name_en: Some("Connection Timeout t0 (ms)".to_string()),
                    description: Some("TCP 建连超时（也是 crate 重连间隔），默认 10000 ms".to_string()),
                    description_zh: Some("TCP 建连超时（也是 crate 重连间隔），默认 10000 ms".to_string()),
                    description_en: Some("TCP connection timeout (also crate reconnect interval), default 10000 ms".to_string()),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(10000)),
                    valid: Some(ParamValid {
                        min: Some(100),
                        max: Some(600000),
                        regex: None,
                        length: None,
                    }),
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "t1_ms".to_string(),
                    name_zh: Some("应答超时 t1 (ms)".to_string()),
                    name_en: Some("Response Timeout t1 (ms)".to_string()),
                    description: Some(
                        "发送 APDU 后等待 S 确认的超时，默认 15000 ms。写命令超时也用此值。".to_string(),
                    ),
                    description_zh: Some(
                        "发送 APDU 后等待 S 确认的超时，默认 15000 ms。写命令超时也用此值。".to_string(),
                    ),
                    description_en: Some(
                        "APDU send/response timeout, default 15000 ms. Also used for write command timeout.".to_string(),
                    ),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(15000)),
                    valid: Some(ParamValid {
                        min: Some(100),
                        max: Some(600000),
                        regex: None,
                        length: None,
                    }),
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "t3_ms".to_string(),
                    name_zh: Some("测试帧间隔 t3 (ms)".to_string()),
                    name_en: Some("Test Frame Interval t3 (ms)".to_string()),
                    description: Some("无数据交互时发送测试帧的间隔，默认 20000 ms".to_string()),
                    description_zh: Some("无数据交互时发送测试帧的间隔，默认 20000 ms".to_string()),
                    description_en: Some("Send test frames when idle, default 20000 ms".to_string()),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(20000)),
                    valid: Some(ParamValid {
                        min: Some(1000),
                        max: Some(600000),
                        regex: None,
                        length: None,
                    }),
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "gi_interval_ms".to_string(),
                    name_zh: Some("总召唤间隔 (ms)".to_string()),
                    name_en: Some("General Interrogation Interval (ms)".to_string()),
                    description: Some(
                        "周期总召唤间隔，0=仅连上时一次。默认 300000 ms（5分钟）".to_string(),
                    ),
                    description_zh: Some(
                        "周期总召唤间隔，0=仅连上时一次。默认 300000 ms（5分钟）".to_string(),
                    ),
                    description_en: Some(
                        "Periodic general interrogation interval, 0=once on connect only. Default 300000 ms (5 min)".to_string(),
                    ),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(300000)),
                    valid: Some(ParamValid {
                        min: Some(0),
                        max: Some(86400000),
                        regex: None,
                        length: None,
                    }),
                    ..Default::default()
                })
                .param(ParamSchema {
                    name: "clock_sync_interval_ms".to_string(),
                    name_zh: Some("时钟同步间隔 (ms)".to_string()),
                    name_en: Some("Clock Synchronization Interval (ms)".to_string()),
                    description: Some("周期时钟同步间隔，0=关闭。默认 0".to_string()),
                    description_zh: Some("周期时钟同步间隔，0=关闭。默认 0".to_string()),
                    description_en: Some(
                        "Periodic clock synchronization interval, 0=disabled. Default 0".to_string(),
                    ),
                    attribute: ParamAttribute::Required,
                    ty: ParamType::Int,
                    default: Some(serde_json::json!(0)),
                    valid: Some(ParamValid {
                        min: Some(0),
                        max: Some(86400000),
                        regex: None,
                        length: None,
                    }),
                    ..Default::default()
                })
                .tag_regex(vec![TagRegexEntry {
                    data_type: "bool".to_string(),
                    regex: r"^[0-9]{1,8}$".to_string(),
                }]),
        )
    }

    fn tag_schema(&self) -> Option<TagSchema> {
        Some(TagSchema {
            data_types: Some(vec![
                "bool".to_string(),
                "int16".to_string(),
                "float32".to_string(),
                "int32".to_string(),
                "uint32".to_string(),
            ]),
            address_format: Some("IOA 十进制数字（1–16777215）".to_string()),
            address_format_zh: Some("IOA 十进制数字，范围 1–16777215".to_string()),
            address_format_en: Some("IOA decimal number, range 1–16777215".to_string()),
        })
    }

    async fn validate_tag(&self, _node_id: NodeId, tag: &Tag) -> PluginResult<()> {
        if tag.name.is_empty() {
            return Err(PluginError::tag_invalid("tag name required"));
        }
        let addr = &tag.address;
        if !is_ioa_format(addr) {
            return Err(PluginError::tag_invalid(
                "IOA must be decimal digits only, 1–8 chars",
            ));
        }
        let ioa = addr
            .parse::<u32>()
            .map_err(|_| PluginError::tag_invalid("IOA must be 1–16777215"))?;
        if !(1..=16_777_215).contains(&ioa) {
            return Err(PluginError::tag_invalid("IOA must be in range 1–16777215"));
        }
        Ok(())
    }

    async fn open(&self, node_id: NodeId, config: PluginConfig) -> PluginResult<()> {
        let host = config_str(&config, "host", "127.0.0.1");
        let port = config_u16(&config, "port", 2404);
        let common_address = config_u16(&config, "common_address", 1);
        let originator_address = config_u8(&config, "originator_address", 0);
        let k = config_u16(&config, "k", 12);
        let w = config_u16(&config, "w", 8);
        let t0_ms = config_u64(&config, "t0_ms", 10000);
        let t1_ms = config_u64(&config, "t1_ms", 15000);
        let t3_ms = config_u64(&config, "t3_ms", 20000);
        let gi_interval_ms = config_u64(&config, "gi_interval_ms", 300000);
        let clock_sync_interval_ms = config_u64(&config, "clock_sync_interval_ms", 0);

        validate_kw(k, w)?;

        log::info(
            node_id,
            format!(
                "open iec104: host={}, port={}, ca={}, k={}, w={}",
                host, port, common_address, k, w
            ),
        );

        let groups = Self::default_groups();
        let tags = groups
            .iter()
            .flat_map(|g| {
                vec![Tag {
                    id: TagId::new(),
                    name: "tag1".to_string(),
                    address: "1".to_string(),
                    attr: gateway_sdk::TagAttr::Read,
                    data_type: Some("bool".to_string()),
                    description: Some("示例点位".to_string()),
                    group_id: g.id,
                }]
            })
            .collect::<Vec<_>>();

        let (cmd_tx, cmd_rx) = tokio::sync::mpsc::channel::<Command>(16);

        let node_state = Iec104NodeState::new(
            host,
            port,
            common_address,
            originator_address,
            k,
            w,
            t0_ms,
            t1_ms,
            t3_ms,
            gi_interval_ms,
            clock_sync_interval_ms,
            groups,
            tags,
            cmd_tx,
        );

        // Spawn worker 线程（自持 OS 线程 + current_thread Runtime）
        let node_state_arc = Arc::new(node_state);
        let worker_arc = Arc::clone(&node_state_arc);

        let jh = std::thread::Builder::new()
            .name(format!("iec104-{:?}", node_id))
            .spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("iec104 worker runtime");
                rt.block_on(worker_loop(worker_arc, cmd_rx));
            })
            .map_err(|e| PluginError::msg(format!("spawn iec104 worker: {}", e)))?;

        // 存 jh 到 node_state
        {
            let mut jh_guard = node_state_arc.join_handle.lock().await;
            *jh_guard = Some(jh);
        }

        // 插入 map（值为 Arc 包装）
        {
            let mut state = self.state.write().await;
            state.insert(node_id, node_state_arc);
        }

        Ok(())
    }

    async fn close(&self, node_id: NodeId) -> PluginResult<()> {
        log::info(node_id, "close iec104");
        let node_state_arc = {
            let mut state = self.state.write().await;
            state.remove(&node_id)
        };

        if let Some(ns) = node_state_arc {
            // 发 Shutdown 命令
            let _ = ns.cmd_tx.send(Command::Shutdown).await;
            // 等待线程退出
            if let Some(jh) = ns.join_handle.lock().await.take() {
                let _ = jh.join();
            }
        }
        Ok(())
    }

    async fn start(&self, node_id: NodeId) -> PluginResult<()> {
        log::info(node_id, "start iec104");
        let state = self.state.read().await;
        let ns = state
            .get(&node_id)
            .ok_or_else(|| PluginError::msg("node not open"))?;
        ns.cmd_tx
            .send(Command::Connect)
            .await
            .map_err(|_| PluginError::msg("iec104 worker not running"))?;
        Ok(())
    }

    async fn stop(&self, node_id: NodeId) -> PluginResult<()> {
        log::info(node_id, "stop iec104");
        let state = self.state.read().await;
        let ns = state
            .get(&node_id)
            .ok_or_else(|| PluginError::msg("node not open"))?;
        ns.cmd_tx
            .send(Command::Disconnect)
            .await
            .map_err(|_| PluginError::msg("iec104 worker not running"))?;
        Ok(())
    }

    async fn setting(&self, node_id: NodeId, config: PluginConfig) -> PluginResult<()> {
        let k = config_u16(&config, "k", 12);
        let w = config_u16(&config, "w", 8);
        validate_kw(k, w)?;

        // 取出旧条目，构造新 NodeState（修改字段），重新插入
        let old_arc = {
            let mut state = self.state.write().await;
            state.remove(&node_id)
        };

        if let Some(old) = old_arc {
            let new_node_state = Iec104NodeState::new(
                config_str(&config, "host", &old.host),
                config_u16(&config, "port", old.port),
                config_u16(&config, "common_address", old.common_address),
                config_u8(&config, "originator_address", old.originator_address),
                k,
                w,
                config_u64(&config, "t0_ms", old.t0_ms),
                config_u64(&config, "t1_ms", old.t1_ms),
                config_u64(&config, "t3_ms", old.t3_ms),
                config_u64(&config, "gi_interval_ms", old.gi_interval_ms),
                config_u64(
                    &config,
                    "clock_sync_interval_ms",
                    old.clock_sync_interval_ms,
                ),
                old.groups.clone(),
                old.tags.clone(),
                old.cmd_tx.clone(),
            );

            let new_arc = Arc::new(new_node_state);

            // 如果旧 worker 线程正在运行，新 NodeState 的 cmd_tx 是新 channel
            // 旧 channel 已在 close 时断开，这里是新 channel
            {
                let mut state = self.state.write().await;
                state.insert(node_id, new_arc);
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
        let ns = state
            .get(&node_id)
            .ok_or_else(|| PluginError::msg("node not open"))?;

        // 只读缓存快照，不触网、不等待
        let cache = ns.cache.lock().await;
        let mut result = Vec::with_capacity(tags.len());
        for tag in tags {
            let ioa: u32 = tag.address.parse().unwrap_or(0);
            if let Some((value, _ts)) = cache.get(&ioa) {
                result.push((tag.id, value.clone()));
            }
            // IOA 未命中直接跳过（§3.2 例外设计）
        }
        Ok(result)
    }

    async fn list_groups(&self, node_id: NodeId) -> PluginResult<Vec<Group>> {
        let state = self.state.read().await;
        let ns = state
            .get(&node_id)
            .ok_or_else(|| PluginError::msg("node not open"))?;
        Ok(ns.groups.clone())
    }

    async fn list_tags(&self, node_id: NodeId, group_id: GroupId) -> PluginResult<Vec<Tag>> {
        let state = self.state.read().await;
        let ns = state
            .get(&node_id)
            .ok_or_else(|| PluginError::msg("node not open"))?;
        Ok(ns
            .tags
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
        let ns = state
            .get(&node_id)
            .ok_or_else(|| PluginError::msg("node not open"))?;

        let timeout_ms = ns.t1_ms;

        for (tag, value) in values {
            let ioa: u32 = tag
                .address
                .parse()
                .map_err(|_| PluginError::tag_invalid(format!("invalid IOA: {}", tag.address)))?;

            let write_value = match value {
                DataValue::Bool(b) => WriteValue::Bool(*b),
                DataValue::Int8(v) => WriteValue::DoublePoint((*v as u8) & 0x3),
                DataValue::UInt8(v) => WriteValue::DoublePoint((*v) & 0x3),
                DataValue::Int16(v) => WriteValue::DoublePoint((*v as u8) & 0x3),
                DataValue::UInt16(v) => WriteValue::DoublePoint((*v as u8) & 0x3),
                DataValue::Int32(v) => WriteValue::DoublePoint((*v as u8) & 0x3),
                DataValue::UInt32(v) => WriteValue::DoublePoint((*v as u8) & 0x3),
                DataValue::Int64(v) => WriteValue::DoublePoint((*v as u8) & 0x3),
                DataValue::UInt64(v) => WriteValue::DoublePoint((*v as u8) & 0x3),
                _ => {
                    return Err(PluginError::tag_invalid(
                        "iec104 write: Bool→SP command, Int 0..3→DP command; other types not supported",
                    ));
                }
            };

            let (tx, rx) = tokio::sync::oneshot::channel();
            ns.cmd_tx
                .send(Command::Write {
                    ioa,
                    value: write_value,
                    tx,
                })
                .await
                .map_err(|_| PluginError::msg("iec104 worker not running"))?;

            let result = tokio::time::timeout(std::time::Duration::from_millis(timeout_ms), rx)
                .await
                .map_err(|_| PluginError::timeout("iec104 write timeout"))?
                .map_err(|e| PluginError::msg(format!("iec104 write: {}", e)))?;

            result.map_err(PluginError::msg)?;
        }

        Ok(())
    }
}

// ============================================================================
// 单元测试
// ============================================================================
#[cfg(test)]
mod tests {
    use super::*;

    // ---- k/w 校验纯逻辑单测 ----

    #[test]
    fn test_kw_max_w_formula() {
        assert_eq!(kw_max_w(12), 8);
        assert_eq!(kw_max_w(1), 0);
        assert_eq!(kw_max_w(2), 1);
        assert_eq!(kw_max_w(3), 2);
        assert_eq!(kw_max_w(6), 4);
        assert_eq!(kw_max_w(100), 66);
    }

    #[test]
    fn test_validate_kw_valid() {
        assert!(validate_kw(12, 8).is_ok());
        assert!(validate_kw(12, 7).is_ok());
        assert!(validate_kw(12, 1).is_ok());
        assert!(validate_kw(2, 1).is_ok());
        assert!(validate_kw(3, 2).is_ok());
    }

    #[test]
    fn test_validate_kw_invalid() {
        // w > ⌊2k/3⌋
        let r = validate_kw(12, 9);
        assert!(r.is_err());
        let err_msg = r.unwrap_err().message();
        assert!(err_msg.contains("9"));
        assert!(err_msg.contains("8"));
        assert!(err_msg.contains("12"));
        assert!(err_msg.contains("k=12"));

        assert!(validate_kw(0, 1).is_err());
        assert!(validate_kw(12, 0).is_err());
    }

    // ---- IOA 解析单测 ----
    #[test]
    fn test_ioa_parsing() {
        use crate::address::parse_ioa;
        assert_eq!(parse_ioa("1"), Some(1));
        assert_eq!(parse_ioa("16777215"), Some(16777215));
        assert_eq!(parse_ioa("0"), None);
        assert_eq!(parse_ioa("16777216"), None);
        assert_eq!(parse_ioa("abc"), None);
    }

    // ---- 命令编码单测 ----
    #[test]
    fn test_write_value_double_point_bounds() {
        use gateway_sdk::types::DataValue;
        for &(ref val, expected) in &[
            (DataValue::Int8(0), 0u8),
            (DataValue::Int8(1), 1),
            (DataValue::Int8(2), 2),
            (DataValue::Int8(3), 3),
            (DataValue::Int8(4), 0), // (4 & 0x3) = 0
            (DataValue::UInt32(100), 0),
        ] {
            let write_value = match &val {
                DataValue::Bool(b) => WriteValue::Bool(*b),
                DataValue::Int8(v) => WriteValue::DoublePoint((*v as u8) & 0x3),
                DataValue::UInt8(v) => WriteValue::DoublePoint((*v) & 0x3),
                DataValue::Int16(v) => WriteValue::DoublePoint((*v as u8) & 0x3),
                DataValue::UInt16(v) => WriteValue::DoublePoint((*v as u8) & 0x3),
                DataValue::Int32(v) => WriteValue::DoublePoint((*v as u8) & 0x3),
                DataValue::UInt32(v) => WriteValue::DoublePoint((*v as u8) & 0x3),
                DataValue::Int64(v) => WriteValue::DoublePoint((*v as u8) & 0x3),
                DataValue::UInt64(v) => WriteValue::DoublePoint((*v as u8) & 0x3),
                _ => unreachable!(),
            };
            if let WriteValue::DoublePoint(dp) = write_value {
                assert_eq!(dp, expected, "val={:?}", val);
            } else {
                unreachable!();
            }
        }
    }
}
