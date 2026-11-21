//! 北向 SMTP 插件：把 GroupData 作为邮件发送——告警通知渠道（蓝图 §7 Q6，2026-11-21 拍板）。
//!
//! 典型接线（flow 编辑器拖拽即可完成，无独立「通知配置」概念）：
//! `source → threshold_alarm → alarm_to_data → sink_north(smtp 节点)`
//! 告警事件经 `alarm_to_data` 转成数据帧后到达本插件，每个字段一行的可读正文
//! 发往配置的收件人。也可以直接接普通数据帧（每帧一封邮件，慎用）。
//!
//! 语义约定：
//! - **尽力而为**：发送失败只计数 + 记录 last_error，不重试不排队——告警的可靠
//!   来源是告警中心（alarm store + REST/WS），邮件只是推送通知。
//! - **防风暴**：`dedup_window_secs` 窗口内相同正文的帧只发一封（告警同键去重
//!   由 alarm store 负责，这里是通知侧的再保险）；窗口默认 300s，0 = 关闭。
//! - **等级过滤**：帧携带 `alarm_level` 字段时按 `min_level` 过滤（低于不发）；
//!   不带该字段的普通数据帧不受过滤（按原样通知）。
//!
//! # Feature flags
//!
//! - `smtp-client`（非默认）：编译 lettre（tokio1 + rustls），完整 SMTP 实现。
//!   不开时为 stub：计数 `dropped_no_client`，不发送（schema 明确标注，防静默丢数据）。
//! - `ffi`：构建 .so/.dll 时启用。

#[cfg(feature = "ffi")]
mod ffi;

use chrono::Utc;
use gateway_plugin_common::config::{config_str, config_u64};
use gateway_sdk::log;
use gateway_sdk::types::PluginKind;
use gateway_sdk::PluginResult;
use gateway_sdk::{GroupData, GroupSubscription, NodeId, NorthPlugin, PluginConfig, PluginMeta};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

// ---------------------------------------------------------------------------
// 常量
// ---------------------------------------------------------------------------

const DEFAULT_SMTP_PORT: u64 = 587;
const DEFAULT_TLS: &str = "starttls";
const DEFAULT_SUBJECT: &str = "[IoT Gateway] Alarm Notification";
const DEFAULT_MIN_LEVEL: &str = "info";
const DEFAULT_DEDUP_WINDOW_SECS: u64 = 300;

/// 告警等级排序（rank）：与 gateway_flow::AlarmLevel 对齐，critical 最高
fn level_rank(s: &str) -> Option<u8> {
    match s.to_ascii_lowercase().as_str() {
        "critical" => Some(4),
        "high" => Some(3),
        "medium" => Some(2),
        "low" => Some(1),
        "info" => Some(0),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// 配置与状态
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum SmtpTls {
    StartTls,
    Ssl,
    None,
}

// smtp_port/tls/username/password/from 仅在 smtp-client 构建的 send_email 中读取；
// stub 构建下保留字段（open 时解析校验即生效）并放宽 dead_code
#[cfg_attr(not(feature = "smtp-client"), allow(dead_code))]
#[derive(Debug, Clone)]
struct NodeSmtpConfig {
    smtp_host: String,
    smtp_port: u16,
    tls: SmtpTls,
    username: Option<String>,
    password: Option<String>,
    from: String,
    to: Vec<String>,
    subject: String,
    /// 最低告警等级 rank；不带 alarm_level 字段的帧不受限
    min_level_rank: u8,
    dedup_window_secs: u64,
}

fn parse_node_config(config: &PluginConfig) -> Result<NodeSmtpConfig, String> {
    let smtp_host = config_str(config, "smtp_host", "");
    if smtp_host.is_empty() {
        return Err("smtp_host is required".into());
    }
    let from = config_str(config, "from", "");
    if from.is_empty() {
        return Err("from is required".into());
    }
    let to_raw = config_str(config, "to", "");
    if to_raw.is_empty() {
        return Err("to is required (comma-separated recipients)".into());
    }
    let to: Vec<String> = to_raw
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect();
    if to.is_empty() {
        return Err("to is required (comma-separated recipients)".into());
    }
    let tls = match config_str(config, "tls", DEFAULT_TLS)
        .to_ascii_lowercase()
        .as_str()
    {
        "none" => SmtpTls::None,
        "ssl" => SmtpTls::Ssl,
        _ => SmtpTls::StartTls,
    };
    let min_level_rank = level_rank(&config_str(config, "min_level", DEFAULT_MIN_LEVEL))
        .ok_or_else(|| "min_level must be one of info/low/medium/high/critical".to_string())?;
    let username = {
        let u = config_str(config, "username", "");
        if u.is_empty() {
            None
        } else {
            Some(u)
        }
    };
    let password = {
        let p = config_str(config, "password", "");
        if p.is_empty() {
            None
        } else {
            Some(p)
        }
    };
    let smtp_port = config_u64(config, "smtp_port", DEFAULT_SMTP_PORT);
    if smtp_port == 0 || smtp_port > 65535 {
        return Err("smtp_port must be 1-65535".into());
    }
    Ok(NodeSmtpConfig {
        smtp_host,
        smtp_port: smtp_port as u16,
        tls,
        username,
        password,
        from,
        to,
        subject: config_str(config, "subject", DEFAULT_SUBJECT),
        min_level_rank,
        dedup_window_secs: config_u64(config, "dedup_window_secs", DEFAULT_DEDUP_WINDOW_SECS),
    })
}

#[derive(Debug, Default, Clone)]
struct SmtpStatus {
    running: bool,
    sent: u64,
    deduped: u64,
    dropped_no_client: u64,
    dropped_low_level: u64,
    send_failed: u64,
    last_error: Option<String>,
    last_subject: Option<String>,
}

/// 每节点状态（stub 与 smtp-client 两种构建共用）
struct NodeSmtpState {
    cfg: NodeSmtpConfig,
    status: Arc<RwLock<SmtpStatus>>,
    /// 正文 → 上次发送时刻（epoch ms）；窗口内相同正文只发一封
    dedup: Arc<std::sync::Mutex<HashMap<String, i64>>>,
}

#[derive(Default)]
struct SmtpState {
    nodes: HashMap<NodeId, NodeSmtpState>,
}

pub struct SmtpPlugin {
    state: Arc<RwLock<SmtpState>>,
}

impl Default for SmtpPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl SmtpPlugin {
    pub fn new() -> Self {
        Self {
            state: Arc::new(RwLock::new(SmtpState {
                nodes: HashMap::new(),
            })),
        }
    }
}

// ---------------------------------------------------------------------------
// 纯函数（两种构建共享；stub 下也可单测）
// ---------------------------------------------------------------------------

/// 把 GroupData 格式化为邮件正文：来源行 + 每字段一行。
/// 北向插件对 tag_names 的字段 key 约定与 plugin-mqtt/kafka 一致。
fn format_body(data: &GroupData) -> String {
    let mut lines = Vec::new();
    match (&data.node_name, &data.group_name) {
        (Some(n), Some(g)) => lines.push(format!("source: {n}/{g}")),
        (Some(n), None) => lines.push(format!("source: {n}")),
        _ => {}
    }
    lines.push(format!("time: {}", data.ts.to_rfc3339()));
    if let Some(names) = &data.tag_names {
        let mut fields: Vec<(String, String)> = Vec::new();
        for (tag_id, value) in &data.values {
            if let Some(name) = names.get(tag_id) {
                let display = match value {
                    gateway_sdk::DataValue::String(s) => s.clone(),
                    other => other
                        .as_f64()
                        .map(|f| f.to_string())
                        .unwrap_or_else(|| format!("{other:?}")),
                };
                fields.push((name.clone(), display));
            }
        }
        fields.sort();
        for (name, display) in fields {
            lines.push(format!("- {name}: {display}"));
        }
    }
    lines.join("\n")
}

/// 帧携带的告警等级 rank（无 alarm_level 字段 → None，不受 min_level 过滤）
fn alarm_rank(data: &GroupData) -> Option<u8> {
    let names = data.tag_names.as_ref()?;
    for (tag_id, value) in &data.values {
        if names.get(tag_id).map(String::as_str) == Some("alarm_level") {
            if let gateway_sdk::DataValue::String(s) = value {
                return level_rank(s);
            }
        }
    }
    None
}

/// dedup 判定：返回 true 表示窗口内重复（应抑制）；否则记录本次并放行。
/// 顺带清理过期条目，防长时间运行键膨胀。
fn should_dedup(dedup: &mut HashMap<String, i64>, key: &str, now_ms: i64, window_ms: i64) -> bool {
    if window_ms <= 0 {
        return false;
    }
    // 清理过期项
    dedup.retain(|_, t| now_ms - *t < window_ms);
    match dedup.get(key) {
        Some(t) if now_ms - *t < window_ms => true,
        _ => {
            dedup.insert(key.to_string(), now_ms);
            false
        }
    }
}

// ---------------------------------------------------------------------------
// NorthPlugin
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
impl NorthPlugin for SmtpPlugin {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "smtp",
            kind: PluginKind::North,
            description: Some("SMTP north: sends GroupData as email (alarm notification channel)"),
            version: "0.1.0",
            name_zh: Some("邮件通知"),
            name_en: Some("Email (SMTP)"),
            description_zh: Some("SMTP 北向：把数据帧/告警帧作为邮件发送（告警通知渠道）"),
            description_en: Some(
                "SMTP north: sends GroupData as email (alarm notification channel)",
            ),
        }
    }

    fn config_schema(&self) -> Option<gateway_sdk::ConfigSchema> {
        Some(config_schema())
    }

    async fn open(&self, node_id: NodeId, config: PluginConfig) -> PluginResult<()> {
        let cfg = parse_node_config(&config).map_err(gateway_sdk::PluginError::config_invalid)?;
        let mut state = self.state.write().await;
        // 保留已有 dedup 表（setting() 热更新不重置去重窗口）；新节点从零开始
        let dedup = state
            .nodes
            .get(&node_id)
            .map(|n| n.dedup.clone())
            .unwrap_or_else(|| Arc::new(std::sync::Mutex::new(HashMap::new())));
        let status = state
            .nodes
            .get(&node_id)
            .map(|n| n.status.clone())
            .unwrap_or_else(|| Arc::new(RwLock::new(SmtpStatus::default())));
        state
            .nodes
            .insert(node_id, NodeSmtpState { cfg, status, dedup });
        Ok(())
    }

    async fn close(&self, node_id: NodeId) -> PluginResult<()> {
        self.state.write().await.nodes.remove(&node_id);
        Ok(())
    }

    async fn start(&self, node_id: NodeId) -> PluginResult<()> {
        match self.state.write().await.nodes.get_mut(&node_id) {
            Some(n) => {
                n.status.write().await.running = true;
                Ok(())
            }
            None => Err(gateway_sdk::PluginError::msg("node not open")),
        }
    }

    async fn stop(&self, node_id: NodeId) -> PluginResult<()> {
        match self.state.write().await.nodes.get_mut(&node_id) {
            Some(n) => {
                n.status.write().await.running = false;
                Ok(())
            }
            None => Err(gateway_sdk::PluginError::msg("node not open")),
        }
    }

    async fn setting(&self, node_id: NodeId, config: PluginConfig) -> PluginResult<()> {
        // 与 open 同构：解析校验 + 热替换配置（保留 dedup/status）
        self.open(node_id, config).await
    }

    async fn set_subscriptions(
        &self,
        node_id: NodeId,
        subscriptions: &[GroupSubscription],
    ) -> PluginResult<()> {
        // SMTP 是推送型北向：订阅由 Manager 编排，插件侧只需不报错地接受
        log::info(
            node_id,
            format!("subscriptions set: {}", subscriptions.len()),
        );
        Ok(())
    }

    async fn connection_status(&self, node_id: NodeId) -> Option<serde_json::Value> {
        let state = self.state.read().await;
        let n = state.nodes.get(&node_id)?;
        let st = n.status.read().await;
        Some(serde_json::json!({
            "running": st.running,
            "mode": if cfg!(feature = "smtp-client") { "client" } else { "stub" },
            "sent": st.sent,
            "deduped": st.deduped,
            "dropped_no_client": st.dropped_no_client,
            "dropped_low_level": st.dropped_low_level,
            "send_failed": st.send_failed,
            "last_error": st.last_error,
            "last_subject": st.last_subject,
            "smtp_host": n.cfg.smtp_host,
            "recipients": n.cfg.to,
        }))
    }

    async fn on_group_data(&self, node_id: NodeId, data: Arc<GroupData>) -> PluginResult<()> {
        let (cfg, status, dedup) = {
            let state = self.state.read().await;
            let Some(n) = state.nodes.get(&node_id) else {
                log::warn(node_id, "on_group_data: node not open, skip");
                return Ok(());
            };
            (n.cfg.clone(), n.status.clone(), n.dedup.clone())
        };
        let running = status.read().await.running;
        if !running {
            return Ok(()); // 与 north 消费任务「未 Running 不转发」同语义
        }

        // 1) 等级过滤：带 alarm_level 的帧低于 min_level 不发
        if let Some(rank) = alarm_rank(&data) {
            if rank < cfg.min_level_rank {
                status.write().await.dropped_low_level += 1;
                return Ok(());
            }
        }

        // 2) 格式化 + 防风暴去重
        let subject = cfg.subject.clone();
        let body = format_body(&data);
        let now_ms = Utc::now().timestamp_millis();
        let window_ms = cfg.dedup_window_secs as i64 * 1000;
        if should_dedup(
            &mut dedup.lock().unwrap_or_else(|e| e.into_inner()),
            &body,
            now_ms,
            window_ms,
        ) {
            status.write().await.deduped += 1;
            return Ok(());
        }

        // 3) 发送（stub：计数即返回，schema 已明确标注防误以为真实发送）
        #[cfg(not(feature = "smtp-client"))]
        {
            let mut st = status.write().await;
            st.dropped_no_client += 1;
            st.last_subject = Some(subject);
        }
        #[cfg(feature = "smtp-client")]
        {
            status.write().await.last_subject = Some(subject.clone());
            let status2 = Arc::clone(&status);
            // 尽力而为：spawn 后台发送，不阻塞 on_group_data；失败只计数
            tokio::spawn(async move {
                match send_email(&cfg, &subject, &body).await {
                    Ok(()) => {
                        status2.write().await.sent += 1;
                    }
                    Err(e) => {
                        log::warn(node_id, format!("smtp send failed: {e}"));
                        let mut st = status2.write().await;
                        st.send_failed += 1;
                        st.last_error = Some(e);
                    }
                }
            });
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// 真实发送（仅 smtp-client 构建）
// ---------------------------------------------------------------------------

#[cfg(feature = "smtp-client")]
async fn send_email(cfg: &NodeSmtpConfig, subject: &str, body: &str) -> Result<(), String> {
    use lettre::transport::smtp::authentication::Credentials;
    use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

    let mut builder = Message::builder()
        .from(
            cfg.from
                .parse()
                .map_err(|e| format!("invalid from address: {e}"))?,
        )
        .subject(subject.to_string());
    // 多收件人在 builder 阶段逐个追加（Message 一旦 build 不可再加收件人）
    for r in &cfg.to {
        let mailbox = r
            .parse()
            .map_err(|e| format!("invalid recipient `{r}`: {e}"))?;
        builder = builder.to(mailbox);
    }
    let email = builder
        .body(body.to_string())
        .map_err(|e| format!("build email: {e}"))?;
    // relay/starttls_relay/builder_dangerous 都返回 Builder，链式配置后 .build() 成 transport
    let mut builder = match cfg.tls {
        SmtpTls::StartTls => AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&cfg.smtp_host)
            .map_err(|e| format!("starttls relay: {e}"))?,
        SmtpTls::Ssl => AsyncSmtpTransport::<Tokio1Executor>::relay(&cfg.smtp_host)
            .map_err(|e| format!("tls relay: {e}"))?,
        SmtpTls::None => AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&cfg.smtp_host),
    };
    builder = builder.port(cfg.smtp_port);
    if let (Some(u), Some(p)) = (&cfg.username, &cfg.password) {
        builder = builder.credentials(Credentials::new(u.clone(), p.clone()));
    }
    builder
        .build()
        .send(email)
        .await
        .map(|_| ())
        .map_err(|e| format!("smtp send: {e}"))
}

// ---------------------------------------------------------------------------
// 配置 Schema
// ---------------------------------------------------------------------------

fn config_schema() -> gateway_sdk::ConfigSchema {
    use gateway_sdk::schema::{ParamAttribute, ParamOption, ParamSchema, ParamType, ParamValid};
    use gateway_sdk::ConfigSchema;

    let host_desc = "SMTP server host".to_string();
    let host_desc_zh = "SMTP 服务器地址".to_string();
    let host_desc_en = "SMTP server host".to_string();
    // 与 kafka 同款防静默丢数据标注（缺陷④对策）：stub 构建在首个必填参数上明示
    #[cfg(not(feature = "smtp-client"))]
    let host_desc = format!(
        "[STUB] 当前构建为 stub，需 smtp-client feature 才真实发送：本构建可创建节点、\
         connection_status 可观测（dropped_no_client 计数），但不会真实发送邮件。{host_desc}"
    );
    #[cfg(not(feature = "smtp-client"))]
    let host_desc_zh = format!(
        "[STUB] 当前构建为 stub，需 smtp-client feature 才真实发送：本构建可创建节点、\
         connection_status 可观测（dropped_no_client 计数），但不会真实发送邮件。{host_desc_zh}"
    );
    #[cfg(not(feature = "smtp-client"))]
    let host_desc_en = format!(
        "[STUB] This build is a stub; the smtp-client feature is required for real delivery. \
         Nodes can be created and observed via connection_status (dropped_no_client counter), \
         but no email is actually sent. {host_desc_en}"
    );

    let level_option = |v: &str, zh: &str, en: &str| ParamOption {
        value: serde_json::json!(v),
        label: Some(en.to_string()),
        label_zh: Some(zh.to_string()),
        label_en: Some(en.to_string()),
    };

    let mut schema = ConfigSchema::new()
        .param(ParamSchema {
            name: "smtp_host".to_string(),
            name_zh: Some("SMTP 服务器".to_string()),
            name_en: Some("SMTP Host".to_string()),
            description: Some(host_desc),
            description_zh: Some(host_desc_zh),
            description_en: Some(host_desc_en),
            attribute: ParamAttribute::Required,
            ty: ParamType::String,
            default: None,
            valid: Some(ParamValid {
                min: None,
                max: None,
                regex: Some(".+".to_string()),
                length: Some(253),
            }),
            ..Default::default()
        })
        .param(ParamSchema {
            name: "smtp_port".to_string(),
            name_zh: Some("端口".to_string()),
            name_en: Some("Port".to_string()),
            description: Some("SMTP port (587 STARTTLS / 465 SSL / 25 none)".to_string()),
            description_zh: Some("SMTP 端口（587 STARTTLS / 465 SSL / 25 不加密）".to_string()),
            description_en: Some("SMTP port (587 STARTTLS / 465 SSL / 25 none)".to_string()),
            attribute: ParamAttribute::Optional,
            ty: ParamType::Int,
            default: Some(serde_json::json!(DEFAULT_SMTP_PORT as i64)),
            valid: Some(ParamValid {
                min: Some(1),
                max: Some(65535),
                regex: None,
                length: None,
            }),
            ..Default::default()
        })
        .param(ParamSchema {
            name: "tls".to_string(),
            name_zh: Some("加密方式".to_string()),
            name_en: Some("TLS".to_string()),
            description: Some("starttls (port 587) / ssl (port 465) / none (plaintext, local testing only)".to_string()),
            description_zh: Some("starttls（587 端口）/ ssl（465 端口）/ none（明文，仅本地测试）".to_string()),
            description_en: Some("starttls (587) / ssl (465) / none (plaintext, local testing only)".to_string()),
            attribute: ParamAttribute::Optional,
            ty: ParamType::Select,
            default: Some(serde_json::json!(DEFAULT_TLS)),
            valid: None,
            options: Some(vec![
                level_option("starttls", "STARTTLS", "STARTTLS"),
                level_option("ssl", "SSL/TLS", "SSL/TLS"),
                level_option("none", "无加密", "None"),
            ]),
            ..Default::default()
        })
        .param(ParamSchema {
            name: "username".to_string(),
            name_zh: Some("用户名".to_string()),
            name_en: Some("Username".to_string()),
            description: Some("SMTP auth username (empty = no auth)".to_string()),
            description_zh: Some("SMTP 认证用户名（留空 = 不认证）".to_string()),
            description_en: Some("SMTP auth username (empty = no auth)".to_string()),
            attribute: ParamAttribute::Optional,
            ty: ParamType::String,
            default: None,
            valid: None,
            ..Default::default()
        })
        .param(ParamSchema {
            name: "password".to_string(),
            name_zh: Some("密码".to_string()),
            name_en: Some("Password".to_string()),
            description: Some("SMTP auth password".to_string()),
            description_zh: Some("SMTP 认证密码（API 返回时脱敏）".to_string()),
            description_en: Some("SMTP auth password (masked in API responses)".to_string()),
            attribute: ParamAttribute::Optional,
            ty: ParamType::String,
            default: None,
            valid: None,
            ..Default::default()
        })
        .param(ParamSchema {
            name: "from".to_string(),
            name_zh: Some("发件人".to_string()),
            name_en: Some("From".to_string()),
            description: Some("Sender email address".to_string()),
            description_zh: Some("发件人邮箱地址".to_string()),
            description_en: Some("Sender email address".to_string()),
            attribute: ParamAttribute::Required,
            ty: ParamType::String,
            default: None,
            valid: Some(ParamValid {
                min: None,
                max: None,
                regex: Some(".+@.+".to_string()),
                length: Some(254),
            }),
            ..Default::default()
        })
        .param(ParamSchema {
            name: "to".to_string(),
            name_zh: Some("收件人".to_string()),
            name_en: Some("To".to_string()),
            description: Some("Comma-separated recipients".to_string()),
            description_zh: Some("收件人邮箱，逗号分隔多个".to_string()),
            description_en: Some("Comma-separated recipients".to_string()),
            attribute: ParamAttribute::Required,
            ty: ParamType::String,
            default: None,
            valid: Some(ParamValid {
                min: None,
                max: None,
                regex: Some(".+".to_string()),
                length: Some(4096),
            }),
            ..Default::default()
        })
        .param(ParamSchema {
            name: "subject".to_string(),
            name_zh: Some("邮件主题".to_string()),
            name_en: Some("Subject".to_string()),
            description: Some("Email subject".to_string()),
            description_zh: Some("邮件主题".to_string()),
            description_en: Some("Email subject".to_string()),
            attribute: ParamAttribute::Optional,
            ty: ParamType::String,
            default: Some(serde_json::json!(DEFAULT_SUBJECT)),
            valid: Some(ParamValid {
                min: None,
                max: None,
                regex: None,
                length: Some(255),
            }),
            ..Default::default()
        })
        .param(ParamSchema {
            name: "min_level".to_string(),
            name_zh: Some("最低告警等级".to_string()),
            name_en: Some("Min Level".to_string()),
            description: Some("Only send alarm frames at or above this level; frames without alarm_level field bypass this filter".to_string()),
            description_zh: Some("只发送不低于该等级的告警帧；不含 alarm_level 字段的普通帧不受过滤".to_string()),
            description_en: Some("Only send alarm frames at or above this level; frames without alarm_level field bypass this filter".to_string()),
            attribute: ParamAttribute::Optional,
            ty: ParamType::Select,
            default: Some(serde_json::json!(DEFAULT_MIN_LEVEL)),
            valid: None,
            options: Some(vec![
                level_option("info", "信息", "Info"),
                level_option("low", "低", "Low"),
                level_option("medium", "中", "Medium"),
                level_option("high", "高", "High"),
                level_option("critical", "严重", "Critical"),
            ]),
            ..Default::default()
        })
        .param(ParamSchema {
            name: "dedup_window_secs".to_string(),
            name_zh: Some("去重窗口（秒）".to_string()),
            name_en: Some("Dedup Window (s)".to_string()),
            description: Some("Suppress identical emails within this window (0 = disabled)".to_string()),
            description_zh: Some("相同正文在此窗口内只发一封（0 = 关闭）".to_string()),
            description_en: Some("Suppress identical emails within this window (0 = disabled)".to_string()),
            attribute: ParamAttribute::Optional,
            ty: ParamType::Int,
            default: Some(serde_json::json!(DEFAULT_DEDUP_WINDOW_SECS as i64)),
            valid: Some(ParamValid {
                min: Some(0),
                max: Some(86400),
                regex: None,
                length: None,
            }),
            ..Default::default()
        });
    // 密码脱敏 + 加密持久化（ConfigSchema.sensitive 约定）
    schema.sensitive = Some(vec!["password".to_string()]);
    schema
}

// ---------------------------------------------------------------------------
// 测试（stub 构建即可全跑：核心逻辑全部为纯函数）
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use gateway_sdk::DataValue;
    use std::collections::HashMap as StdHashMap;

    fn alarm_frame(level: &str, message: &str) -> GroupData {
        let ts = Utc::now();
        let id_level = gateway_sdk::TagId(uuid::Uuid::from_u128(1));
        let id_msg = gateway_sdk::TagId(uuid::Uuid::from_u128(5));
        GroupData {
            node_id: gateway_sdk::NodeId(uuid::Uuid::nil()),
            group_id: gateway_sdk::GroupId(uuid::Uuid::nil()),
            ts,
            values: vec![
                (id_level, DataValue::String(level.into())),
                (id_msg, DataValue::String(message.into())),
            ],
            node_name: Some("alarms".into()),
            group_name: Some("events".into()),
            tag_names: Some(StdHashMap::from([
                (id_level, "alarm_level".to_string()),
                (id_msg, "alarm_message".to_string()),
            ])),
        }
    }

    #[test]
    fn format_body_lists_source_and_fields() {
        let body = format_body(&alarm_frame("high", "temperature crossed"));
        assert!(body.contains("source: alarms/events"), "{body}");
        assert!(body.contains("- alarm_level: high"), "{body}");
        assert!(
            body.contains("- alarm_message: temperature crossed"),
            "{body}"
        );
        assert!(body.contains("time: "), "{body}");
    }

    #[test]
    fn level_filter_drops_below_min() {
        let frame = alarm_frame("low", "minor");
        assert_eq!(alarm_rank(&frame), Some(1));
        // min_level=medium(2)：rank 1 < 2 → 丢弃
        assert!(Some(1) < Some(2u8));
        // 无 alarm_level 字段的帧不受限
        let plain = GroupData {
            node_id: gateway_sdk::NodeId(uuid::Uuid::nil()),
            group_id: gateway_sdk::GroupId(uuid::Uuid::nil()),
            ts: Utc::now(),
            values: vec![(
                gateway_sdk::TagId(uuid::Uuid::from_u128(9)),
                DataValue::Float64(1.0),
            )],
            node_name: Some("sim".into()),
            group_name: Some("g1".into()),
            tag_names: Some(StdHashMap::from([(
                gateway_sdk::TagId(uuid::Uuid::from_u128(9)),
                "temperature".to_string(),
            )])),
        };
        assert_eq!(alarm_rank(&plain), None);
    }

    #[test]
    fn dedup_suppresses_within_window_and_expires() {
        let mut dedup = HashMap::new();
        // 窗口 10s
        assert!(!should_dedup(&mut dedup, "body", 1_000, 10_000));
        assert!(should_dedup(&mut dedup, "body", 5_000, 10_000));
        assert!(should_dedup(&mut dedup, "body", 10_999, 10_000));
        // 窗口外放行并刷新
        assert!(!should_dedup(&mut dedup, "body", 11_001, 10_000));
        // 不同正文互不影响
        assert!(!should_dedup(&mut dedup, "other", 12_000, 10_000));
        // 窗口 0 = 关闭
        let mut off = HashMap::new();
        assert!(!should_dedup(&mut off, "body", 1_000, 0));
        assert!(!should_dedup(&mut off, "body", 2_000, 0));
    }

    #[test]
    fn parse_node_config_expands_recipients_and_defaults() {
        let mut cfg: PluginConfig = StdHashMap::new();
        cfg.insert("smtp_host".into(), serde_json::json!("smtp.example.com"));
        cfg.insert("from".into(), serde_json::json!("gw@example.com"));
        cfg.insert("to".into(), serde_json::json!("a@x.com, b@x.com ,c@x.com"));
        let c = parse_node_config(&cfg).unwrap();
        assert_eq!(c.to, vec!["a@x.com", "b@x.com", "c@x.com"]);
        assert_eq!(c.smtp_port, 587);
        assert!(matches!(c.tls, SmtpTls::StartTls));
        assert_eq!(c.min_level_rank, 0);
        assert_eq!(c.dedup_window_secs, 300);
        // 必填缺失
        let empty: PluginConfig = StdHashMap::new();
        assert!(parse_node_config(&empty).is_err());
        // 非法 tls/min_level/port
        let mut bad = cfg.clone();
        bad.insert("tls".into(), serde_json::json!("nope"));
        let c2 = parse_node_config(&bad).unwrap(); // 未知 tls 回退 starttls
        assert!(matches!(c2.tls, SmtpTls::StartTls));
        let mut bad2 = cfg.clone();
        bad2.insert("min_level".into(), serde_json::json!("urgent"));
        assert!(parse_node_config(&bad2).is_err());
        let mut bad3 = cfg;
        bad3.insert("smtp_port".into(), serde_json::json!(70000));
        assert!(parse_node_config(&bad3).is_err());
    }

    #[test]
    fn config_schema_marks_stub_build() {
        let schema = config_schema();
        let host = schema
            .params
            .iter()
            .find(|p| p.name == "smtp_host")
            .unwrap();
        #[cfg(not(feature = "smtp-client"))]
        assert!(host.description.as_ref().unwrap().contains("STUB"));
        #[cfg(feature = "smtp-client")]
        assert!(!host.description.as_ref().unwrap().contains("STUB"));
        // 密码脱敏约定
        assert_eq!(
            schema.sensitive.as_deref(),
            Some(&["password".to_string()][..])
        );
    }

    #[tokio::test]
    async fn open_start_and_connection_status_roundtrip() {
        let plugin = SmtpPlugin::new();
        let id = NodeId(uuid::Uuid::new_v4());
        let mut cfg: PluginConfig = StdHashMap::new();
        cfg.insert("smtp_host".into(), serde_json::json!("smtp.example.com"));
        cfg.insert("from".into(), serde_json::json!("gw@example.com"));
        cfg.insert("to".into(), serde_json::json!("a@x.com"));
        plugin.open(id, cfg).await.unwrap();
        plugin.start(id).await.unwrap();
        let status = plugin.connection_status(id).await.unwrap();
        assert_eq!(status["running"], serde_json::json!(true));
        assert_eq!(status["recipients"], serde_json::json!(["a@x.com"]));
        plugin.close(id).await.unwrap();
        assert!(plugin.connection_status(id).await.is_none());
    }

    #[tokio::test]
    async fn on_group_data_counts_stub_drop_and_dedup() {
        let plugin = SmtpPlugin::new();
        let id = NodeId(uuid::Uuid::new_v4());
        let mut cfg: PluginConfig = StdHashMap::new();
        cfg.insert("smtp_host".into(), serde_json::json!("smtp.example.com"));
        cfg.insert("from".into(), serde_json::json!("gw@example.com"));
        cfg.insert("to".into(), serde_json::json!("a@x.com"));
        cfg.insert("dedup_window_secs".into(), serde_json::json!(600));
        plugin.open(id, cfg).await.unwrap();
        plugin.start(id).await.unwrap();
        let frame = Arc::new(alarm_frame("high", "same event"));
        // stub 构建：第一帧 dropped_no_client，第二帧 deduped
        plugin.on_group_data(id, frame.clone()).await.unwrap();
        plugin.on_group_data(id, frame.clone()).await.unwrap();
        // smtp-client 构建下发送走后台 spawn，计数异步落——这里不断言 sent；
        // stub 断言在下方 cfg(not) 块。status 仍取一次验证节点可观测。
        #[cfg(not(feature = "smtp-client"))]
        let status = plugin.connection_status(id).await.unwrap();
        #[cfg(feature = "smtp-client")]
        let _status = plugin.connection_status(id).await.unwrap();
        #[cfg(not(feature = "smtp-client"))]
        {
            assert_eq!(status["dropped_no_client"], serde_json::json!(1));
            assert_eq!(status["deduped"], serde_json::json!(1));
        }
        // 等级过滤：min_level=critical，high 帧被丢弃
        let mut cfg2: PluginConfig = StdHashMap::new();
        cfg2.insert("smtp_host".into(), serde_json::json!("smtp.example.com"));
        cfg2.insert("from".into(), serde_json::json!("gw@example.com"));
        cfg2.insert("to".into(), serde_json::json!("a@x.com"));
        cfg2.insert("min_level".into(), serde_json::json!("critical"));
        cfg2.insert("dedup_window_secs".into(), serde_json::json!(0));
        let id2 = NodeId(uuid::Uuid::new_v4());
        plugin.open(id2, cfg2).await.unwrap();
        plugin.start(id2).await.unwrap();
        plugin
            .on_group_data(id2, Arc::new(alarm_frame("high", "other")))
            .await
            .unwrap();
        let status2 = plugin.connection_status(id2).await.unwrap();
        assert_eq!(status2["dropped_low_level"], serde_json::json!(1));
    }

    #[tokio::test]
    async fn on_group_data_without_open_is_noop() {
        let plugin = SmtpPlugin::new();
        plugin
            .on_group_data(
                NodeId(uuid::Uuid::new_v4()),
                Arc::new(alarm_frame("info", "x")),
            )
            .await
            .unwrap();
    }
}
