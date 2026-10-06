//! 持久化：节点/组/标签/订阅落盘。
//!
//! - **SQLite 存储**：配置与状态写入 `data/data.db`，单文件、支持事务与并发读。
//! - **版本与迁移**：`meta` 表记录 schema 版本，便于后续升级。
//! - **原子写入**：save 在单事务内替换全量数据，崩溃不产生半写。
//!
//! ## 使用
//! - 启动时 `load(path)`，path 为数据库文件（如 `data/data.db`）；文件不存在返回 `Ok(None)`。
//! - 变更后 `save(path, &snapshot)`；建议由上层（如 API 层）统一触发保存。

use crate::node::Node;
use crate::store::Store;
use gateway_sdk::types::{GroupId, NodeKind, NodeState, TagAttr, TagId};
use gateway_sdk::{Group, GroupSubscription, NodeId, Tag};
use rusqlite::{params, Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::info;

/// 当前快照/schema 版本。大于此版本需升级程序。
pub const SNAPSHOT_VERSION: u32 = 2;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS meta (version INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS nodes (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  kind TEXT NOT NULL,
  plugin_name TEXT NOT NULL,
  config TEXT NOT NULL,
  state TEXT NOT NULL,
  tenant_id TEXT NOT NULL DEFAULT 'default'
);
CREATE TABLE IF NOT EXISTS groups (
  node_id TEXT NOT NULL,
  group_id TEXT NOT NULL,
  name TEXT NOT NULL,
  interval_ms INTEGER NOT NULL,
  description TEXT,
  PRIMARY KEY (node_id, group_id)
);
CREATE TABLE IF NOT EXISTS tags (
  tag_id TEXT PRIMARY KEY,
  node_id TEXT NOT NULL,
  group_id TEXT NOT NULL,
  name TEXT NOT NULL,
  address TEXT NOT NULL,
  attr TEXT NOT NULL,
  data_type TEXT,
  description TEXT
);
CREATE TABLE IF NOT EXISTS subscriptions (
  north_node_id TEXT NOT NULL,
  south_node_id TEXT NOT NULL,
  group_id TEXT NOT NULL,
  PRIMARY KEY (north_node_id, south_node_id, group_id)
);
CREATE TABLE IF NOT EXISTS rules (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  enabled INTEGER NOT NULL,
  south_node_id TEXT NOT NULL,
  group_id TEXT NOT NULL,
  tag_name TEXT NOT NULL,
  op TEXT NOT NULL,
  threshold REAL NOT NULL,
  for_ms INTEGER NOT NULL,
  clear_ms INTEGER NOT NULL,
  action TEXT NOT NULL,
  tenant_id TEXT NOT NULL DEFAULT 'default'
);
CREATE TABLE IF NOT EXISTS policies (
  south_node_id TEXT NOT NULL,
  group_id      TEXT NOT NULL,
  config        TEXT NOT NULL,
  PRIMARY KEY (south_node_id, group_id)
);
"#;

/// 持久化错误
#[derive(Debug, thiserror::Error)]
pub enum PersistError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("unsupported snapshot version: got {0}, max supported {1}")]
    VersionUnsupported(u32, u32),
    #[error("validation failed: {0}")]
    Validation(String),
    #[error("database recovery failed: {0}")]
    Recovery(String),
}

/// 持久化快照（节点/组/标签/北向订阅）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    #[serde(default)]
    pub version: u32,
    pub nodes: Vec<Node>,
    pub groups: Vec<(NodeId, Group)>,
    pub tags: Vec<(NodeId, Tag)>,
    pub subscriptions: Vec<(NodeId, Vec<GroupSubscription>)>,
    /// 规则。带 `#[serde(default)]`：旧版本库/快照没有该字段也能正常加载
    #[serde(default)]
    pub rules: Vec<crate::rules::Rule>,
    /// 组数据策略。带 `#[serde(default)]`：旧版本库/快照没有该字段也能正常加载
    #[serde(default)]
    pub policies: Vec<crate::filters::GroupPolicy>,
}

impl Default for Snapshot {
    fn default() -> Self {
        Self {
            version: SNAPSHOT_VERSION,
            nodes: Vec::new(),
            groups: Vec::new(),
            tags: Vec::new(),
            subscriptions: Vec::new(),
            rules: Vec::new(),
            policies: Vec::new(),
        }
    }
}

impl Snapshot {
    /// 校验快照：版本、节点 ID 唯一性、tenant_id 非空等
    pub fn validate(&self) -> Result<(), PersistError> {
        if self.version > SNAPSHOT_VERSION {
            return Err(PersistError::VersionUnsupported(
                self.version,
                SNAPSHOT_VERSION,
            ));
        }
        let mut ids = std::collections::HashSet::new();
        for n in &self.nodes {
            if !ids.insert(n.id()) {
                return Err(PersistError::Validation(format!(
                    "duplicate node id: {:?}",
                    n.id()
                )));
            }
            if n.config.tenant_id.is_empty() {
                return Err(PersistError::Validation(format!(
                    "node {:?} has empty tenant_id",
                    n.id()
                )));
            }
        }
        let mut rule_ids = std::collections::HashSet::new();
        for r in &self.rules {
            if !rule_ids.insert(r.id.as_str()) {
                return Err(PersistError::Validation(format!(
                    "duplicate rule id: {}",
                    r.id
                )));
            }
            if r.tenant_id.is_empty() {
                return Err(PersistError::Validation(format!(
                    "rule {} has empty tenant_id",
                    r.id
                )));
            }
        }
        Ok(())
    }
}

/// 从 Manager 提供的数据构建快照
pub fn build_snapshot(
    nodes: Vec<Node>,
    groups: Vec<(NodeId, Group)>,
    tags: Vec<(NodeId, Tag)>,
    subscriptions: Vec<(NodeId, Vec<GroupSubscription>)>,
    rules: Vec<crate::rules::Rule>,
    policies: Vec<crate::filters::GroupPolicy>,
) -> Snapshot {
    Snapshot {
        version: SNAPSHOT_VERSION,
        nodes,
        groups,
        tags,
        subscriptions,
        rules,
        policies,
    }
}

fn ensure_schema(conn: &Connection) -> Result<(), rusqlite::Error> {
    apply_pragmas(conn)?;
    conn.execute_batch(SCHEMA)?;

    // 读取当前版本
    let version: Option<u32> = conn
        .query_row("SELECT version FROM meta LIMIT 1", [], |r| r.get(0))
        .ok();

    if version.is_none() {
        // 全新库：直接插入版本号（CREATE TABLE IF NOT EXISTS 幂等）
        conn.execute("INSERT INTO meta (version) VALUES (?1)", [SNAPSHOT_VERSION])?;
        return Ok(());
    }

    let v = version.unwrap();

    // v1 → v2 迁移：nodes 和 rules 表加 tenant_id 列
    if v < 2 {
        // 迁移 nodes 表
        let nodes_has_tenant: bool = conn
            .query_row("PRAGMA table_info(nodes)", [], |r| {
                let col: String = r.get(1)?;
                Ok(col == "tenant_id")
            })
            .unwrap_or(false);
        if !nodes_has_tenant {
            conn.execute(
                "ALTER TABLE nodes ADD COLUMN tenant_id TEXT NOT NULL DEFAULT 'default'",
                [],
            )?;
        }

        // 迁移 rules 表
        let rules_has_tenant: bool = conn
            .query_row("PRAGMA table_info(rules)", [], |r| {
                let col: String = r.get(1)?;
                Ok(col == "tenant_id")
            })
            .unwrap_or(false);
        if !rules_has_tenant {
            conn.execute(
                "ALTER TABLE rules ADD COLUMN tenant_id TEXT NOT NULL DEFAULT 'default'",
                [],
            )?;
        }

        // 按 source 节点为存量规则补章（孤儿规则保持 default）
        conn.execute(
            r#"UPDATE rules SET tenant_id = (
                SELECT n.tenant_id FROM nodes n WHERE n.id = rules.south_node_id
               ) WHERE EXISTS (SELECT 1 FROM nodes n WHERE n.id = rules.south_node_id)"#,
            [],
        )?;

        // 更新版本号
        conn.execute("UPDATE meta SET version = 2", [])?;
    }

    Ok(())
}

/// SQLite 运行参数：
/// - `WAL`：读写并发（API 写不再阻塞读），显著降低 SQLITE_BUSY
/// - `synchronous=NORMAL`：WAL 下进程崩溃不损坏库，仅在断电时可能丢最后若干提交
/// - `busy_timeout`：并发写等待而不是立刻返回 SQLITE_BUSY
/// - `foreign_keys`：保持引用完整性约束
fn apply_pragmas(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         PRAGMA busy_timeout = 5000;
         PRAGMA foreign_keys = ON;",
    )
}

/// 删除表中已不在快照里的行（`keys` 为空表示清空该表）
fn delete_missing(
    tx: &rusqlite::Transaction<'_>,
    table: &str,
    key_expr: &str,
    keys: &[String],
) -> Result<(), PersistError> {
    if keys.is_empty() {
        tx.execute(&format!("DELETE FROM {}", table), [])?;
        return Ok(());
    }
    // 用「临时键表 + 求差集」代替把 N 个占位符拼进 `NOT IN (...)`。
    //
    // 原实现在 5000 个点位时会产生约 10 KB 的 SQL 文本，每次保存都要重新解析与规划；
    // 临时表方案把成本降到一次批量插入 + 一次带索引的连接，且不受参数个数上限影响。
    tx.execute_batch(
        "DROP TABLE IF EXISTS temp._gw_keep;
         CREATE TEMP TABLE _gw_keep (k TEXT PRIMARY KEY);",
    )?;
    {
        let mut stmt = tx.prepare_cached("INSERT OR IGNORE INTO temp._gw_keep (k) VALUES (?1)")?;
        for k in keys {
            stmt.execute(params![k])?;
        }
    }
    tx.execute(
        &format!(
            "DELETE FROM {} WHERE {} NOT IN (SELECT k FROM temp._gw_keep)",
            table, key_expr
        ),
        [],
    )?;
    Ok(())
}

fn node_kind_to_str(k: NodeKind) -> &'static str {
    match k {
        NodeKind::South => "south",
        NodeKind::North => "north",
    }
}

fn str_to_node_kind(s: &str) -> Result<NodeKind, PersistError> {
    match s {
        "south" => Ok(NodeKind::South),
        "north" => Ok(NodeKind::North),
        _ => Err(PersistError::Validation(format!(
            "unknown node kind: {}",
            s
        ))),
    }
}

fn node_state_to_str(s: NodeState) -> &'static str {
    match s {
        NodeState::Stopped => "stopped",
        NodeState::Running => "running",
        NodeState::Error => "error",
    }
}

fn str_to_node_state(s: &str) -> Result<NodeState, PersistError> {
    match s {
        "stopped" => Ok(NodeState::Stopped),
        "running" => Ok(NodeState::Running),
        "error" => Ok(NodeState::Error),
        _ => Err(PersistError::Validation(format!(
            "unknown node state: {}",
            s
        ))),
    }
}

fn tag_attr_to_str(a: TagAttr) -> &'static str {
    match a {
        TagAttr::Read => "read",
        TagAttr::Write => "write",
        TagAttr::ReadWrite => "readwrite",
    }
}

fn str_to_tag_attr(s: &str) -> Result<TagAttr, PersistError> {
    match s {
        "read" => Ok(TagAttr::Read),
        "write" => Ok(TagAttr::Write),
        "readwrite" => Ok(TagAttr::ReadWrite),
        _ => Err(PersistError::Validation(format!("unknown tag attr: {}", s))),
    }
}

fn parse_node_id(s: &str) -> Result<NodeId, PersistError> {
    uuid::Uuid::parse_str(s)
        .map(NodeId)
        .map_err(|e| PersistError::Validation(format!("invalid node_id uuid: {}", e)))
}

fn parse_group_id(s: &str) -> Result<GroupId, PersistError> {
    uuid::Uuid::parse_str(s)
        .map(GroupId)
        .map_err(|e| PersistError::Validation(format!("invalid group_id uuid: {}", e)))
}

fn parse_tag_id(s: &str) -> Result<TagId, PersistError> {
    uuid::Uuid::parse_str(s)
        .map(TagId)
        .map_err(|e| PersistError::Validation(format!("invalid tag_id uuid: {}", e)))
}

fn load_from_db(conn: &Connection) -> Result<Snapshot, PersistError> {
    let version: u32 = conn.query_row("SELECT version FROM meta LIMIT 1", [], |r| r.get(0))?;
    if version > SNAPSHOT_VERSION {
        return Err(PersistError::VersionUnsupported(version, SNAPSHOT_VERSION));
    }

    let mut nodes = Vec::new();
    let mut stmt =
        conn.prepare("SELECT id, name, kind, plugin_name, config, state, tenant_id FROM nodes")?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, String>(6)?,
        ))
    })?;
    for row in rows {
        let (id, name, kind, plugin_name, config, state, tenant_id) = row?;
        let id = parse_node_id(&id)?;
        let kind = str_to_node_kind(&kind)?;
        let state = str_to_node_state(&state)?;
        let config: gateway_sdk::PluginConfig = serde_json::from_str(&config)
            .map_err(|e| PersistError::Validation(format!("node config json: {}", e)))?;
        nodes.push(Node {
            config: crate::node::NodeConfig {
                id,
                name,
                kind,
                plugin_name,
                config,
                tenant_id,
            },
            state,
        });
    }

    let mut groups = Vec::new();
    let mut stmt =
        conn.prepare("SELECT node_id, group_id, name, interval_ms, description FROM groups")?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, i64>(3)?,
            row.get::<_, Option<String>>(4)?,
        ))
    })?;
    for row in rows {
        let (node_id, group_id, name, interval_ms, description) = row?;
        let node_id = parse_node_id(&node_id)?;
        let group_id = parse_group_id(&group_id)?;
        groups.push((
            node_id,
            Group {
                id: group_id,
                name,
                interval_ms: interval_ms as u64,
                description,
            },
        ));
    }

    let mut tags = Vec::new();
    let mut stmt = conn.prepare(
        "SELECT tag_id, node_id, group_id, name, address, attr, data_type, description FROM tags",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, Option<String>>(6)?,
            row.get::<_, Option<String>>(7)?,
        ))
    })?;
    for row in rows {
        let (tag_id, node_id, group_id, name, address, attr, data_type, description) = row?;
        let tag_id = parse_tag_id(&tag_id)?;
        let node_id = parse_node_id(&node_id)?;
        let group_id = parse_group_id(&group_id)?;
        let attr = str_to_tag_attr(&attr)?;
        tags.push((
            node_id,
            Tag {
                id: tag_id,
                name,
                address,
                attr,
                data_type,
                description,
                group_id,
            },
        ));
    }

    let mut sub_map: std::collections::HashMap<NodeId, Vec<GroupSubscription>> =
        std::collections::HashMap::new();
    let mut stmt =
        conn.prepare("SELECT north_node_id, south_node_id, group_id FROM subscriptions")?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    for row in rows {
        let (north, south, gid) = row?;
        let north = parse_node_id(&north)?;
        let south = parse_node_id(&south)?;
        let gid = parse_group_id(&gid)?;
        sub_map.entry(north).or_default().push(GroupSubscription {
            south_node_id: south,
            group_id: gid,
        });
    }
    let subscriptions = sub_map.into_iter().collect();

    // 规则：action 以 JSON 存储，其余字段分列（便于人工查库）
    let mut rules = Vec::new();
    let mut stmt = conn.prepare(
        "SELECT id, name, enabled, south_node_id, group_id, tag_name, op, threshold, for_ms, clear_ms, action, tenant_id FROM rules",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, String>(6)?,
            row.get::<_, f64>(7)?,
            row.get::<_, i64>(8)?,
            row.get::<_, i64>(9)?,
            row.get::<_, String>(10)?,
            row.get::<_, String>(11)?,
        ))
    })?;
    for row in rows {
        let (
            id,
            name,
            enabled,
            south,
            gid,
            tag_name,
            op,
            threshold,
            for_ms,
            clear_ms,
            action,
            tenant_id,
        ) = row?;
        let op = serde_json::from_str::<crate::rules::CompareOp>(&format!("\"{}\"", op))
            .map_err(|e| PersistError::Validation(format!("rule {}: bad op: {}", id, e)))?;
        let action: crate::rules::RuleAction = serde_json::from_str(&action)
            .map_err(|e| PersistError::Validation(format!("rule {}: bad action: {}", id, e)))?;
        rules.push(crate::rules::Rule {
            id,
            name,
            enabled: enabled != 0,
            source: crate::rules::RuleSource {
                south_node_id: parse_node_id(&south)?,
                group_id: parse_group_id(&gid)?,
                tag_name,
            },
            condition: crate::rules::RuleCondition { op, threshold },
            for_ms: for_ms.max(0) as u64,
            clear_ms: clear_ms.max(0) as u64,
            action,
            tenant_id,
        });
    }

    // 策略
    let mut policies = Vec::new();
    let mut stmt = conn.prepare("SELECT south_node_id, group_id, config FROM policies")?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    for row in rows {
        let (south_id, gid, config) = row?;
        let south_node_id = parse_node_id(&south_id)?;
        let group_id = parse_group_id(&gid)?;
        let policy: crate::filters::GroupPolicy = serde_json::from_str(&config).map_err(|e| {
            PersistError::Validation(format!(
                "policy ({},{}): bad config JSON: {}",
                south_id, gid, e
            ))
        })?;
        // 覆盖路径参数（外部传入的 south_node_id/group_id 优先级更高）
        let mut policy = policy;
        policy.south_node_id = south_node_id;
        policy.group_id = group_id;
        policies.push(policy);
    }

    Ok(Snapshot {
        version,
        nodes,
        groups,
        tags,
        subscriptions,
        rules,
        policies,
    })
}

// ---------- 敏感配置落盘加密 ----------
/// 加密值前缀：用于识别「已加密」并在无密钥时避免误解密
const ENC_PREFIX: &str = "enc:v1:";
/// AES-256-GCM nonce 长度
const NONCE_LEN: usize = 12;

fn secret_key(secret: &str) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(secret.as_bytes());
    let out = h.finalize();
    let mut key = [0u8; 32];
    key.copy_from_slice(&out);
    key
}

/// AES-256-GCM 加密字符串，返回 `enc:v1:<base64(nonce|ciphertext)>`；失败时返回 None（由调用方决定降级为明文）
fn encrypt_secret(secret: &str, plain: &str) -> Option<String> {
    use aes_gcm::aead::{Aead, KeyInit};
    use aes_gcm::{Aes256Gcm, Nonce};
    let cipher = Aes256Gcm::new_from_slice(&secret_key(secret)).ok()?;
    let mut nonce_bytes = [0u8; NONCE_LEN];
    use rand::RngCore;
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let ct = cipher
        .encrypt(Nonce::from_slice(&nonce_bytes), plain.as_bytes())
        .ok()?;
    let mut blob = nonce_bytes.to_vec();
    blob.extend_from_slice(&ct);
    use base64::Engine;
    Some(format!(
        "{}{}",
        ENC_PREFIX,
        base64::engine::general_purpose::STANDARD.encode(blob)
    ))
}

/// 解密 `enc:v1:...`；前缀不匹配或解密失败返回 None（保留原值，避免丢配置）
fn decrypt_secret(secret: &str, value: &str) -> Option<String> {
    use aes_gcm::aead::{Aead, KeyInit};
    use aes_gcm::{Aes256Gcm, Nonce};
    use base64::Engine;
    let b64 = value.strip_prefix(ENC_PREFIX)?;
    let blob = base64::engine::general_purpose::STANDARD.decode(b64).ok()?;
    if blob.len() <= NONCE_LEN {
        return None;
    }
    let (nonce_bytes, ct) = blob.split_at(NONCE_LEN);
    let cipher = Aes256Gcm::new_from_slice(&secret_key(secret)).ok()?;
    let pt = cipher.decrypt(Nonce::from_slice(nonce_bytes), ct).ok()?;
    String::from_utf8(pt).ok()
}

/// 对快照中所有节点的敏感配置项加密（幂等：已是加密值则跳过）
fn encrypt_snapshot(s: &Snapshot, secret: &str) -> Snapshot {
    let mut out = s.clone();
    for n in &mut out.nodes {
        for (k, v) in n.config.config.iter_mut() {
            if !gateway_sdk::schema::is_sensitive_key(k) {
                continue;
            }
            if let Some(s) = v.as_str() {
                if s.starts_with(ENC_PREFIX) || s.is_empty() {
                    continue;
                }
                if let Some(enc) = encrypt_secret(secret, s) {
                    *v = serde_json::Value::String(enc);
                }
            }
        }
    }
    out
}

/// 解密快照中的敏感配置项；无密钥或解密失败时保留密文（仅告警，不丢数据）
fn decrypt_snapshot(s: &mut Snapshot, secret: Option<&str>) {
    let Some(secret) = secret else { return };
    for n in &mut s.nodes {
        for (k, v) in n.config.config.iter_mut() {
            let Some(s) = v.as_str() else { continue };
            if !s.starts_with(ENC_PREFIX) {
                continue;
            }
            match decrypt_secret(secret, s) {
                Some(plain) => *v = serde_json::Value::String(plain),
                None => tracing::warn!(
                    node = %n.config.name,
                    key = %k,
                    "sensitive config could not be decrypted (wrong or changed GATEWAY_SECRET_KEY?)"
                ),
            }
        }
    }
}

fn save_to_db(conn: &Connection, s: &Snapshot) -> Result<(), PersistError> {
    s.validate()?;
    let tx = conn.unchecked_transaction()?;
    tx.execute("UPDATE meta SET version = ?1", [s.version])?;

    // 增量写入：UPSERT 只影响真正变化的行，配合末尾的「删除差集」保持全量语义。
    // 相比原来的「DELETE 全表 + 全量 INSERT」，变更 1 个点位不再重写整库。
    //
    // 每条 UPSERT 都通过 `prepare_cached` 执行：否则 SQLite 每行都要重新解析/编译一次语句，
    // 5000 点位的配置下这部分就是保存耗时的主体（性能基线实测约 83 ms/次）。
    {
        let mut stmt = tx.prepare_cached(
            "INSERT INTO nodes (id, name, kind, plugin_name, config, state, tenant_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                kind = excluded.kind,
                plugin_name = excluded.plugin_name,
                config = excluded.config,
                state = excluded.state,
                tenant_id = excluded.tenant_id",
        )?;
        for n in &s.nodes {
            stmt.execute(params![
                n.config.id.0.to_string(),
                n.config.name,
                node_kind_to_str(n.config.kind),
                n.config.plugin_name,
                serde_json::to_string(&n.config.config)?,
                node_state_to_str(n.state),
                n.config.tenant_id,
            ])?;
        }
    }
    let node_keys: Vec<String> = s.nodes.iter().map(|n| n.config.id.0.to_string()).collect();
    delete_missing(&tx, "nodes", "id", &node_keys)?;

    {
        let mut stmt = tx.prepare_cached(
            "INSERT INTO groups (node_id, group_id, name, interval_ms, description) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(node_id, group_id) DO UPDATE SET
                name = excluded.name,
                interval_ms = excluded.interval_ms,
                description = excluded.description",
        )?;
        for (nid, g) in &s.groups {
            stmt.execute(params![
                nid.0.to_string(),
                g.id.0.to_string(),
                g.name,
                g.interval_ms as i64,
                g.description,
            ])?;
        }
    }
    let group_keys: Vec<String> = s
        .groups
        .iter()
        .map(|(nid, g)| format!("{}|{}", nid.0, g.id.0))
        .collect();
    delete_missing(&tx, "groups", "node_id || '|' || group_id", &group_keys)?;

    {
        let mut stmt = tx.prepare_cached(
            "INSERT INTO tags (tag_id, node_id, group_id, name, address, attr, data_type, description) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(tag_id) DO UPDATE SET
                node_id = excluded.node_id,
                group_id = excluded.group_id,
                name = excluded.name,
                address = excluded.address,
                attr = excluded.attr,
                data_type = excluded.data_type,
                description = excluded.description",
        )?;
        for (nid, t) in &s.tags {
            stmt.execute(params![
                t.id.0.to_string(),
                nid.0.to_string(),
                t.group_id.0.to_string(),
                t.name,
                t.address,
                tag_attr_to_str(t.attr),
                t.data_type,
                t.description,
            ])?;
        }
    }
    let tag_keys: Vec<String> = s.tags.iter().map(|(_, t)| t.id.0.to_string()).collect();
    delete_missing(&tx, "tags", "tag_id", &tag_keys)?;

    {
        let mut stmt = tx.prepare_cached(
            "INSERT INTO subscriptions (north_node_id, south_node_id, group_id) VALUES (?1, ?2, ?3)
             ON CONFLICT(north_node_id, south_node_id, group_id) DO NOTHING",
        )?;
        for (north_id, subs) in &s.subscriptions {
            for sub in subs {
                stmt.execute(params![
                    north_id.0.to_string(),
                    sub.south_node_id.0.to_string(),
                    sub.group_id.0.to_string(),
                ])?;
            }
        }
    }
    let sub_keys: Vec<String> = s
        .subscriptions
        .iter()
        .flat_map(|(nid, subs)| {
            subs.iter()
                .map(move |sub| format!("{}|{}|{}", nid.0, sub.south_node_id.0, sub.group_id.0))
        })
        .collect();
    delete_missing(
        &tx,
        "subscriptions",
        "north_node_id || '|' || south_node_id || '|' || group_id",
        &sub_keys,
    )?;

    {
        let mut stmt = tx.prepare_cached(
            "INSERT INTO rules (id, name, enabled, south_node_id, group_id, tag_name, op, threshold, for_ms, clear_ms, action, tenant_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
             ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                enabled = excluded.enabled,
                south_node_id = excluded.south_node_id,
                group_id = excluded.group_id,
                tag_name = excluded.tag_name,
                op = excluded.op,
                threshold = excluded.threshold,
                for_ms = excluded.for_ms,
                clear_ms = excluded.clear_ms,
                action = excluded.action,
                tenant_id = excluded.tenant_id",
        )?;
        for r in &s.rules {
            stmt.execute(params![
                r.id,
                r.name,
                if r.enabled { 1i64 } else { 0i64 },
                r.source.south_node_id.0.to_string(),
                r.source.group_id.0.to_string(),
                r.source.tag_name,
                r.condition.op.as_str(),
                r.condition.threshold,
                r.for_ms as i64,
                r.clear_ms as i64,
                serde_json::to_string(&r.action)?,
                r.tenant_id,
            ])?;
        }
    }
    let rule_keys: Vec<String> = s.rules.iter().map(|r| r.id.clone()).collect();
    delete_missing(&tx, "rules", "id", &rule_keys)?;

    // 策略（south_node_id || '|' || group_id 作为复合主键的 key）
    {
        let mut stmt = tx.prepare_cached(
            "INSERT INTO policies (south_node_id, group_id, config) VALUES (?1, ?2, ?3)
             ON CONFLICT(south_node_id, group_id) DO UPDATE SET config = excluded.config",
        )?;
        for p in &s.policies {
            stmt.execute(params![
                p.south_node_id.0.to_string(),
                p.group_id.0.to_string(),
                serde_json::to_string(p)?,
            ])?;
        }
    }
    let policy_keys: Vec<String> = s
        .policies
        .iter()
        .map(|p| format!("{}|{}", p.south_node_id.0, p.group_id.0))
        .collect();
    delete_missing(
        &tx,
        "policies",
        "south_node_id || '|' || group_id",
        &policy_keys,
    )?;

    tx.commit()?;
    Ok(())
}

/// 从 SQLite 数据库加载快照，并对敏感配置解密。`secret` 为 `None` 时不解密（保留密文）。
pub async fn load_secret(
    path: &Path,
    secret: Option<&str>,
) -> Result<Option<Snapshot>, PersistError> {
    if !path.exists() {
        return Ok(None);
    }
    let path_buf = path.to_path_buf();
    let path_log = path_buf.display().to_string();
    let snap = tokio::task::spawn_blocking(move || {
        let conn = Connection::open(&path_buf)?;
        ensure_schema(&conn)?;
        load_from_db(&conn)
    })
    .await
    .map_err(|e| PersistError::Io(std::io::Error::other(e)))?;
    let mut s = snap?;
    decrypt_snapshot(&mut s, secret);
    info!(path = %path_log, version = s.version, "persist loaded");
    Ok(Some(s))
}

/// 将快照写入 SQLite（可选地先对敏感配置加密）。
pub async fn save_secret(
    path: &Path,
    s: &Snapshot,
    secret: Option<&str>,
) -> Result<(), PersistError> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let path_buf = path.to_path_buf();
    let path_log = path_buf.display().to_string();
    let snap = match secret {
        Some(k) => encrypt_snapshot(s, k),
        None => s.clone(),
    };
    tokio::task::spawn_blocking(move || {
        let conn = Connection::open_with_flags(
            &path_buf,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
        )?;
        ensure_schema(&conn)?;
        // 备份改为节流执行（默认每小时一次）且使用一致性快照，
        // 不再「每次保存都整文件 copy」造成 O(库大小) 开销。
        if let Err(e) = maybe_backup(&conn, &path_buf) {
            tracing::warn!(path = %path_buf.display(), "backup before save failed: {}", e);
        }
        save_to_db(&conn, &snap)
    })
    .await
    .map_err(|e| PersistError::Io(std::io::Error::other(e)))??;
    info!(path = %path_log, version = s.version, encrypted = secret.is_some(), "persist saved");
    Ok(())
}

/// 备份节流间隔（秒）：距上次备份不足该时长则跳过
const BACKUP_INTERVAL_SECS: u64 = 3600;
static LAST_BACKUP_SECS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn now_epoch_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 生成一致性备份 `{db}.bak`。`VACUUM INTO` 产出事务一致的快照，
/// 比直接 copy 正在写入的数据库文件更可靠。
fn maybe_backup(conn: &Connection, path: &Path) -> Result<(), PersistError> {
    use std::sync::atomic::Ordering;
    if !path.exists() {
        return Ok(());
    }
    let now = now_epoch_secs();
    let last = LAST_BACKUP_SECS.load(Ordering::Relaxed);
    if last != 0 && now.saturating_sub(last) < BACKUP_INTERVAL_SECS {
        return Ok(());
    }
    // 只有一个任务能拿到这次备份机会
    if LAST_BACKUP_SECS
        .compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed)
        .is_err()
    {
        return Ok(());
    }
    let backup_path = path
        .parent()
        .map(|p| p.join(path.file_name().unwrap().to_string_lossy().to_string() + ".bak"))
        .unwrap_or_else(|| PathBuf::from(path.to_string_lossy().to_string() + ".bak"));
    // VACUUM INTO 要求目标文件不存在
    let _ = std::fs::remove_file(&backup_path);
    conn.execute(
        "VACUUM INTO ?1",
        [backup_path.to_string_lossy().to_string()],
    )?;
    info!(path = %backup_path.display(), "database snapshot created");
    Ok(())
}

// ---------- 共享连接（Db）：data.db 的进程内单长连接 ----------

/// 启动完整性检查模式（`GATEWAY_DB_INTEGRITY=full|quick|off`，默认 `full`）。
/// `data.db` 是配置库（点位规模千级、库体积 MB 级以下），全量检查只在启动时执行一次，
/// 耗时毫秒级；`quick` 与 `off` 供超大库或特殊场景降级。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntegrityMode {
    /// 逐行全量校验（默认）
    Full,
    /// 只查结构完整性
    Quick,
    /// 整体跳过检查（对坏文件不检查、不恢复；后续加载按既有错误路径失败）
    Off,
}

impl IntegrityMode {
    /// 解析配置字符串；无法识别的值按 `full` 处理（安全默认）
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "quick" => IntegrityMode::Quick,
            "off" => IntegrityMode::Off,
            _ => IntegrityMode::Full,
        }
    }
}

/// data.db 的进程内共享连接：单连接 + 互斥，全部 SQLite 访问经 [`Db::with`] 串行执行。
///
/// `rusqlite::Connection` 是 `Send` 非 `Sync`，`Mutex` 包裹后满足跨任务共享；
/// 所有调用方应运行在 `spawn_blocking` 里（持锁阻塞的是 blocking 线程池而非 tokio worker）。
/// 写负载已被去抖合并为 ≥300ms 一次、users 操作为低频短事务，单连接串行不会成为瓶颈。
pub struct Db {
    path: PathBuf,
    conn: std::sync::Mutex<Connection>,
}

impl Db {
    /// 打开（或修复后打开）库文件：integrity_check 门禁 → PRAGMA → ensure_schema，各只做一次。
    ///
    /// 顺序与错误分类是硬性要求（SQLite 默认 `busy_timeout=0`，若把检查放在 PRAGMA 之前，
    /// 启动时残留 `-wal` 恢复的短暂锁竞争、另一实例运行/备份、Windows 上杀毒软件占用等
    /// 都会误触发破坏性恢复）：
    /// 1. `PRAGMA busy_timeout = 5000` 先于检查设置；
    /// 2. `PRAGMA integrity_check|quick_check`：Busy/Locked 退避重试后仍失败 → 告警跳过继续启动；
    ///    NotADatabase 或「检查成功执行但结果 ≠ ok」→ 恢复序列；其他错误记录后跳过检查继续启动；
    /// 3. 检查通过（或被跳过）后才执行 journal_mode/synchronous/foreign_keys 与 ensure_schema。
    pub fn open(path: &Path, integrity: IntegrityMode) -> Result<Arc<Db>, PersistError> {
        Self::open_inner(path, integrity, 0)
    }

    fn open_inner(
        path: &Path,
        integrity: IntegrityMode,
        depth: u32,
    ) -> Result<Arc<Db>, PersistError> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let conn = Connection::open(path)?;
        // 硬性顺序第 1 步：busy_timeout 先于检查设置
        conn.busy_timeout(std::time::Duration::from_millis(5000))?;
        // 硬性顺序第 2 步：完整性检查 + 按错误码分类
        let mut busy_retries = 0usize;
        loop {
            match run_integrity_check_once(&conn, integrity) {
                Ok(()) => break,
                Err(CheckProblem::Busy(e)) if busy_retries < BUSY_RETRY_DELAYS_MS.len() => {
                    let delay_ms = BUSY_RETRY_DELAYS_MS[busy_retries];
                    busy_retries += 1;
                    tracing::warn!(
                        path = %path.display(),
                        retry = busy_retries,
                        delay_ms,
                        "integrity check busy, retrying: {}",
                        e
                    );
                    std::thread::sleep(std::time::Duration::from_millis(delay_ms));
                }
                Err(CheckProblem::Busy(e)) => {
                    // 常见于另一实例在运行/备份：绝不进入恢复序列，正常继续启动
                    tracing::warn!(
                        path = %path.display(),
                        "integrity check skipped (busy): {}; continuing startup, file untouched",
                        e
                    );
                    break;
                }
                Err(CheckProblem::NotADatabase(e)) => {
                    // 破坏性恢复仅由「检查成功执行但结果 ≠ ok」与 NotADatabase 两类信号触发
                    drop(conn);
                    let reason = format!("file is not a database: {}", e);
                    return Self::recover(path, integrity, depth, &reason);
                }
                Err(CheckProblem::Failed(first)) => {
                    drop(conn);
                    return Self::recover(path, integrity, depth, &first);
                }
                Err(CheckProblem::Other(e)) => {
                    // 磁盘 I/O 错误、文件被第三方软件占用等：记录后跳过检查继续启动，绝不改名/覆盖/删除
                    tracing::error!(
                        path = %path.display(),
                        "integrity check errored (non-corruption), skipping check and continuing: {}",
                        e
                    );
                    break;
                }
            }
        }
        // 硬性顺序第 3 步：检查通过（或被跳过）后才配置 PRAGMA 并建 schema
        ensure_schema(&conn)?;
        tracing::info!(path = %path.display(), "database connection ready (single shared connection)");
        Ok(Arc::new(Db {
            path: path.to_path_buf(),
            conn: std::sync::Mutex::new(conn),
        }))
    }

    /// 库文件路径（供备份命名与日志使用）
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 在共享连接上执行一段同步 SQLite 操作（内部 spawn_blocking 由调用方负责）。
    /// 锁中毒视为可恢复（panic 时 SQLite 在下次 reset 自动回滚未完成事务），取回内层数据继续。
    pub fn with<T>(
        &self,
        f: impl FnOnce(&Connection) -> Result<T, PersistError>,
    ) -> Result<T, PersistError> {
        let conn = self.lock();
        f(&conn)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// 在共享连接上加载快照，并对敏感配置解密。`secret` 为 `None` 时不解密（保留密文）。
    /// schema 与 PRAGMA 已在 `open` 时完成，这里只做读取。
    pub fn load_snapshot(&self, secret: Option<&str>) -> Result<Option<Snapshot>, PersistError> {
        let s = self.with(|conn| {
            let mut s = load_from_db(conn)?;
            decrypt_snapshot(&mut s, secret);
            Ok(s)
        })?;
        info!(path = %self.path.display(), version = s.version, "persist loaded");
        Ok(Some(s))
    }

    /// 将快照写入共享连接（可选地先对敏感配置加密）。
    /// 备份节流（`maybe_backup`）与单事务 UPSERT/差集删除语义保持不变。
    pub fn save_snapshot(&self, secret: Option<&str>, s: &Snapshot) -> Result<(), PersistError> {
        let snap = match secret {
            Some(k) => encrypt_snapshot(s, k),
            None => s.clone(),
        };
        self.with(|conn| {
            if let Err(e) = maybe_backup(conn, &self.path) {
                tracing::warn!(path = %self.path.display(), "backup before save failed: {}", e);
            }
            save_to_db(conn, &snap)
        })?;
        info!(
            path = %self.path.display(),
            version = s.version,
            encrypted = secret.is_some(),
            "persist saved"
        );
        Ok(())
    }

    /// 优雅退出时调用：`PRAGMA optimize`（幂等、廉价，维护统计信息）
    pub fn optimize(&self) -> Result<(), PersistError> {
        self.with(|conn| Ok(conn.execute_batch("PRAGMA optimize;")?))
    }

    /// 恢复序列：仅由「检查成功执行但结果 ≠ ok」与 `NotADatabase` 触发。
    ///
    /// 1. 把坏库（连同 `-wal`/`-shm`）隔离为 `.corrupt-<unix秒>` 留证；改名失败（如 Windows
    ///    句柄被占用）→ 停止恢复、文件不动、告警后由上层走既有兜底继续启动，不做删除与循环重试；
    /// 2. 若 `{db}.bak` 存在则复制回 `data.db` 并重新走同一套检查；复制失败同样「不动文件、告警继续」；
    /// 3. 备份不存在或恢复后仍未通过：当前 data.db 再次隔离留证后，删除并按空库重建（配置丢失）。
    fn recover(
        path: &Path,
        integrity: IntegrityMode,
        depth: u32,
        reason: &str,
    ) -> Result<Arc<Db>, PersistError> {
        tracing::error!(
            path = %path.display(),
            reason,
            "database integrity check FAILED; starting recovery"
        );
        if depth >= RECOVERY_MAX_DEPTH {
            return Err(PersistError::Recovery(format!(
                "recovery depth limit reached at {}",
                path.display()
            )));
        }
        // 1. 隔离坏库留证
        match isolate_corrupt(path) {
            Ok(corrupt) => tracing::warn!(
                path = %path.display(),
                corrupt = %corrupt.display(),
                "corrupt database isolated for forensics"
            ),
            Err(e) => {
                tracing::error!(
                    path = %path.display(),
                    "recovery aborted: cannot move corrupt database, FILE LEFT UNTOUCHED; \
                     manual intervention required: {}",
                    e
                );
                return Err(PersistError::Recovery(format!(
                    "cannot isolate corrupt database {}: {}",
                    path.display(),
                    e
                )));
            }
        }
        // 2. 从 .bak 恢复（VACUUM INTO 产出的事务一致快照）
        let bak = backup_path_for(path);
        if bak.exists() {
            match std::fs::copy(&bak, path) {
                Err(e) => {
                    tracing::error!(
                        backup = %bak.display(),
                        "restore from backup failed, no file deleted and no retry; \
                         continuing startup with existing fallbacks: {}",
                        e
                    );
                    return Err(PersistError::Recovery(format!(
                        "cannot restore backup {}: {}",
                        bak.display(),
                        e
                    )));
                }
                Ok(_) => match Self::open_inner(path, integrity, depth + 1) {
                    Ok(db) => {
                        tracing::warn!(
                            path = %path.display(),
                            backup = %bak.display(),
                            "recovered database from backup"
                        );
                        return Ok(db);
                    }
                    Err(e) => tracing::error!(
                        path = %path.display(),
                        "restored backup still failed integrity checks: {}; rebuilding empty database",
                        e
                    ),
                },
            }
        }
        // 3. 空库重建：坏库（含恢复失败的副本）此时均已隔离为 .corrupt-* 留证
        if path.exists() {
            match isolate_corrupt(path) {
                Ok(corrupt) => {
                    tracing::warn!(corrupt = %corrupt.display(), "isolated failed restore")
                }
                Err(e) => {
                    tracing::error!(
                        path = %path.display(),
                        "recovery aborted before rebuild, FILE LEFT UNTOUCHED: {}",
                        e
                    );
                    return Err(PersistError::Recovery(format!(
                        "cannot isolate failed restore {}: {}",
                        path.display(),
                        e
                    )));
                }
            }
        }
        tracing::error!(
            path = %path.display(),
            "starting with an EMPTY database; previous configuration is lost \
             (corrupt files kept as .corrupt-*)"
        );
        Self::open_inner(path, integrity, depth + 1)
    }
}

/// Busy/Locked 退避重试间隔（毫秒）：仍失败则告警跳过检查继续启动
const BUSY_RETRY_DELAYS_MS: [u64; 3] = [200, 500, 1000];
/// 恢复序列最大递归深度（恢复 bak 失败 → 空库重建，最多两层）
const RECOVERY_MAX_DEPTH: u32 = 2;

/// `{db}.bak` 路径（与 `maybe_backup` 保持一致）
fn backup_path_for(path: &Path) -> PathBuf {
    path.parent()
        .map(|p| {
            p.join(
                path.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string()
                    + ".bak",
            )
        })
        .unwrap_or_else(|| PathBuf::from(path.to_string_lossy().to_string() + ".bak"))
}

/// 把坏库（连同 `-wal`/`-shm`）改名为 `.corrupt-<unix秒>` 留证，不直接删除。
/// 任何一步失败都返回 Err，由调用方停止恢复（Windows 上句柄被占用时 rename 会失败）。
fn isolate_corrupt(path: &Path) -> std::io::Result<PathBuf> {
    let ts = now_epoch_secs();
    let mut corrupt = PathBuf::from(format!("{}.corrupt-{}", path.display(), ts));
    while corrupt.exists() {
        // 同一秒内多次恢复时避免覆盖留证文件
        corrupt = PathBuf::from(format!(
            "{}.corrupt-{}-{}",
            path.display(),
            ts,
            uuid::Uuid::new_v4().simple()
        ));
    }
    std::fs::rename(path, &corrupt)?;
    for suffix in ["-wal", "-shm"] {
        let side = PathBuf::from(format!("{}{}", path.display(), suffix));
        if side.exists() {
            let side_corrupt =
                PathBuf::from(format!("{}{}.corrupt-{}", path.display(), suffix, ts));
            std::fs::rename(&side, &side_corrupt)?;
        }
    }
    Ok(corrupt)
}

/// integrity_check / quick_check 的一次检查结果分类。
/// 实现者必须按错误码分类，不得把一切 `Err` 当损坏。
enum CheckProblem {
    /// `DatabaseBusy`/`DatabaseLocked`（含 -wal 恢复竞争、另一实例运行/备份）
    Busy(rusqlite::Error),
    /// 检查成功执行但首行结果 ≠ ok（多行结果 = 多条错误，同样进入恢复序列）
    Failed(String),
    /// `NotADatabase`：文件内容损坏的典型信号
    NotADatabase(rusqlite::Error),
    /// 其他错误（磁盘 I/O、文件被第三方软件占用等）
    Other(rusqlite::Error),
}

fn classify_check_err(e: rusqlite::Error) -> CheckProblem {
    match &e {
        rusqlite::Error::SqliteFailure(ffi, _) => match ffi.code {
            rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked => {
                CheckProblem::Busy(e)
            }
            rusqlite::ErrorCode::NotADatabase => CheckProblem::NotADatabase(e),
            _ => CheckProblem::Other(e),
        },
        _ => CheckProblem::Other(e),
    }
}

/// 执行一次完整性检查；`IntegrityMode::Off` 直接通过（不打开检查语句）。
fn run_integrity_check_once(
    conn: &Connection,
    integrity: IntegrityMode,
) -> Result<(), CheckProblem> {
    let sql = match integrity {
        IntegrityMode::Off => return Ok(()),
        IntegrityMode::Full => "PRAGMA integrity_check",
        IntegrityMode::Quick => "PRAGMA quick_check",
    };
    let mut stmt = conn.prepare(sql).map_err(classify_check_err)?;
    let mut rows = stmt.query([]).map_err(classify_check_err)?;
    let first: Option<String> = match rows.next() {
        Ok(row) => match row {
            Some(row) => Some(row.get(0).map_err(classify_check_err)?),
            None => None,
        },
        Err(e) => return Err(classify_check_err(e)),
    };
    match first {
        Some(ref s) if s.eq_ignore_ascii_case("ok") => Ok(()),
        Some(s) => Err(CheckProblem::Failed(s)),
        None => Err(CheckProblem::Failed("no result rows".into())),
    }
}

/// 快照中是否存在仍以明文保存的敏感配置项（用于首次启用加密时回写）
pub fn has_plaintext_secrets(s: &Snapshot) -> bool {
    s.nodes.iter().any(|n| {
        n.config.config.iter().any(|(k, v)| {
            gateway_sdk::schema::is_sensitive_key(k)
                && v.as_str()
                    .map(|x| !x.is_empty() && !x.starts_with(ENC_PREFIX))
                    .unwrap_or(false)
        })
    })
}

/// 不加密的保存（保持向后兼容）
pub async fn save(path: &Path, s: &Snapshot) -> Result<(), PersistError> {
    save_secret(path, s, None).await
}

/// 不解密的加载（保持向后兼容）
pub async fn load(path: &Path) -> Result<Option<Snapshot>, PersistError> {
    load_secret(path, None).await
}

/// 与 `save` 相同；保留接口兼容，备份由上层或定期拷贝 db 文件实现。
pub async fn save_with_backup(
    path: &Path,
    s: &Snapshot,
    _backup: bool,
) -> Result<(), PersistError> {
    save(path, s).await
}

/// 从旧版 JSON 文件加载快照（用于迁移到 SQLite）。文件不存在或为空返回 `Ok(None)`。
pub async fn load_json(path: &Path) -> Result<Option<Snapshot>, PersistError> {
    let data = match tokio::fs::read_to_string(path).await {
        Ok(d) => d,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let data = data.trim();
    if data.is_empty() {
        return Ok(None);
    }
    let mut s: Snapshot = serde_json::from_str(data).map_err(PersistError::Json)?;
    if s.version == 0 {
        s.version = SNAPSHOT_VERSION;
    }
    s.validate()?;
    info!(path = %path.display(), version = s.version, "persist loaded from json");
    Ok(Some(s))
}

/// 将快照应用至 Store（清空后填入）。订阅表由 Manager 在 `apply_snapshot` 中另行写入。
pub fn apply_to_store(store: &Store, s: &Snapshot) {
    store.clear();
    for n in &s.nodes {
        store.node_insert(n.clone());
    }
    for (nid, g) in &s.groups {
        store.group_insert(*nid, g.clone());
    }
    for (nid, t) in &s.tags {
        store.tag_insert(*nid, t.clone());
    }
    for r in &s.rules {
        store.rule_insert(r.clone());
    }
    for p in &s.policies {
        store.policy_insert(p.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::Node;
    use gateway_sdk::types::NodeKind;

    /// 串行化所有会触碰 maybe_backup 全局节流状态（LAST_BACKUP_SECS）的测试：
    /// 该计数器是进程级全局，并行测试中的 save 会在彼此的「重置 -> CAS」窗口内
    /// 抢走备份机会，导致节流断言偶发失败。异步测试需要跨 .await 持有守卫，用 tokio Mutex。
    static BACKUP_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    fn temp_db(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("gw-test-{}-{}.db", name, uuid::Uuid::new_v4()));
        let _ = std::fs::remove_file(&p);
        let _ = std::fs::remove_file(format!("{}.bak", p.display()));
        p
    }

    fn node_with(name: &str, config: serde_json::Value) -> Node {
        let mut n = Node::new(
            name,
            NodeKind::South,
            "sim",
            serde_json::from_value(config).unwrap(),
        );
        n.state = NodeState::Running;
        n
    }

    fn snapshot_with_mqtt_password() -> Snapshot {
        let mut nodes = Vec::new();
        nodes.push(node_with(
            "mqtt-app",
            serde_json::json!({ "host": "127.0.0.1", "port": 1883, "password": "p@ssw0rd" }),
        ));
        Snapshot {
            version: SNAPSHOT_VERSION,
            nodes,
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn snapshot_roundtrip_preserves_nodes() {
        let _backup_guard = BACKUP_TEST_LOCK.lock().await;
        let path = temp_db("roundtrip");
        let snap = snapshot_with_mqtt_password();
        let node_id = snap.nodes[0].config.id;
        save(&path, &snap).await.expect("save failed");

        let loaded = load(&path)
            .await
            .expect("load failed")
            .expect("no snapshot");
        assert_eq!(loaded.nodes.len(), 1);
        assert_eq!(loaded.nodes[0].config.id, node_id);
        assert_eq!(loaded.nodes[0].config.name, "mqtt-app");
        assert_eq!(loaded.nodes[0].state, NodeState::Running);
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn secret_is_encrypted_at_rest_and_decrypted_on_load() {
        let _backup_guard = BACKUP_TEST_LOCK.lock().await;
        let path = temp_db("enc");
        let snap = snapshot_with_mqtt_password();
        save_secret(&path, &snap, Some("unit-test-secret"))
            .await
            .expect("save failed");

        // 数据库中不应出现明文口令
        let conn = Connection::open(&path).unwrap();
        let cfg: String = conn
            .query_row("SELECT config FROM nodes LIMIT 1", [], |r| r.get(0))
            .unwrap();
        assert!(
            !cfg.contains("p@ssw0rd"),
            "plaintext password leaked into db"
        );
        assert!(cfg.contains(ENC_PREFIX), "value was not encrypted");
        drop(conn);

        // 正确密钥可读回明文
        let loaded = load_secret(&path, Some("unit-test-secret"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            loaded.nodes[0].config.config.get("password").unwrap(),
            "p@ssw0rd"
        );
        // 主机等非敏感字段保持明文可读
        assert_eq!(
            loaded.nodes[0].config.config.get("host").unwrap(),
            "127.0.0.1"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn wrong_secret_keeps_ciphertext_without_panic() {
        let _backup_guard = BACKUP_TEST_LOCK.lock().await;
        let path = temp_db("wrongkey");
        let snap = snapshot_with_mqtt_password();
        save_secret(&path, &snap, Some("right-secret"))
            .await
            .unwrap();

        let loaded = load_secret(&path, Some("wrong-secret"))
            .await
            .unwrap()
            .unwrap();
        // 解密失败时保留密文（不丢配置），而非崩溃或返回空
        let v = loaded.nodes[0]
            .config
            .config
            .get("password")
            .unwrap()
            .as_str()
            .unwrap();
        assert!(v.starts_with(ENC_PREFIX));

        // 无密钥加载同样不丢数据
        let plain_load = load_secret(&path, None).await.unwrap().unwrap();
        assert!(plain_load.nodes[0]
            .config
            .config
            .get("password")
            .unwrap()
            .as_str()
            .unwrap()
            .starts_with(ENC_PREFIX));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn plaintext_secrets_detection() {
        let snap = snapshot_with_mqtt_password();
        assert!(has_plaintext_secrets(&snap));

        // 无口令的节点集合不应触发
        let mut clean = snapshot_with_mqtt_password();
        clean.nodes[0].config.config.remove("password");
        assert!(!has_plaintext_secrets(&clean));

        // 已加密的口令不应重复触发回写
        let mut encrypted = snapshot_with_mqtt_password();
        let enc = encrypt_secret("k", "p@ssw0rd").expect("encrypt failed");
        encrypted.nodes[0]
            .config
            .config
            .insert("password".to_string(), serde_json::Value::String(enc));
        assert!(!has_plaintext_secrets(&encrypted));
    }

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let enc = encrypt_secret("secret", "hello").unwrap();
        assert!(enc.starts_with(ENC_PREFIX));
        assert_ne!(enc, format!("{}hello", ENC_PREFIX));
        assert_eq!(decrypt_secret("secret", &enc).unwrap(), "hello");
        assert!(decrypt_secret("other-secret", &enc).is_none());
        assert!(decrypt_secret("secret", "not-encrypted").is_none());
    }

    #[test]
    fn nodes_are_restored_into_store() {
        let snap = snapshot_with_mqtt_password();
        let id = snap.nodes[0].config.id;
        let store = Store::new();
        apply_to_store(&store, &snap);
        assert_eq!(store.nodes_list().len(), 1);
        assert_eq!(store.nodes_list()[0].config.id, id);
    }

    fn cleanup_db(path: &Path) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{}", path.display(), suffix));
        }
    }

    #[tokio::test]
    async fn wal_mode_is_enabled() {
        let _backup_guard = BACKUP_TEST_LOCK.lock().await;
        let path = temp_db("wal");
        save(&path, &snapshot_with_mqtt_password())
            .await
            .expect("save");
        let conn = Connection::open(&path).unwrap();
        let mode: String = conn
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            mode.to_lowercase(),
            "wal",
            "WAL should be enabled for concurrent access"
        );
        let busy: i64 = conn
            .query_row("PRAGMA busy_timeout", [], |r| r.get(0))
            .unwrap();
        assert_eq!(busy, 5000, "busy_timeout should be configured");
        drop(conn);
        cleanup_db(&path);
    }

    #[tokio::test]
    async fn incremental_save_updates_and_removes_rows() {
        let _backup_guard = BACKUP_TEST_LOCK.lock().await;
        let path = temp_db("upsert");
        let mut snap = snapshot_with_mqtt_password();
        let keep_id = snap.nodes[0].config.id;
        let extra = node_with("sim-2", serde_json::json!({ "poll_base_ms": 1000 }));
        let drop_id = extra.config.id;
        snap.nodes.push(extra);
        save(&path, &snap).await.expect("first save");

        // 同时验证「更新」与「删除」：重命名保留节点，移除另一个节点
        snap.nodes.retain(|n| n.config.id != drop_id);
        snap.nodes[0].config.name = "renamed".to_string();
        save(&path, &snap).await.expect("second save");

        let loaded = load(&path).await.unwrap().unwrap();
        assert_eq!(
            loaded.nodes.len(),
            1,
            "removed node must not survive an incremental save"
        );
        assert_eq!(loaded.nodes[0].config.id, keep_id);
        assert_eq!(
            loaded.nodes[0].config.name, "renamed",
            "upsert should apply updates"
        );

        // 重复保存同一快照不应产生重复行
        save(&path, &snap).await.expect("third save");
        let again = load(&path).await.unwrap().unwrap();
        assert_eq!(again.nodes.len(), 1);
        cleanup_db(&path);
    }

    #[tokio::test]
    async fn throttled_backup_creates_consistent_snapshot() {
        let _backup_guard = BACKUP_TEST_LOCK.lock().await;
        let path = temp_db("bak");
        save(&path, &snapshot_with_mqtt_password())
            .await
            .expect("save");
        // 重置节流计时，确保本次一定执行备份
        super::LAST_BACKUP_SECS.store(0, std::sync::atomic::Ordering::Relaxed);

        let conn = Connection::open(&path).unwrap();
        maybe_backup(&conn, &path).expect("backup should succeed");
        let bak = PathBuf::from(format!("{}.bak", path.display()));
        assert!(bak.exists(), "throttled backup file should exist");

        // 备份本身必须是可独立打开、数据完整的一致性快照
        let bak_conn = Connection::open(&bak).unwrap();
        let rows: i64 = bak_conn
            .query_row("SELECT COUNT(1) FROM nodes", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 1, "snapshot should contain the saved node");

        // 节流：紧接着再次调用不应重复备份（时间戳保持不变）
        super::LAST_BACKUP_SECS.store(now_epoch_secs(), std::sync::atomic::Ordering::Relaxed);
        let before = std::fs::metadata(&bak).unwrap().modified().unwrap();
        maybe_backup(&conn, &path).expect("second backup call");
        let after = std::fs::metadata(&bak).unwrap().modified().unwrap();
        assert_eq!(
            before, after,
            "backup should be throttled within the interval"
        );

        drop(bak_conn);
        drop(conn);
        cleanup_db(&path);
        let _ = std::fs::remove_file(&bak);
    }

    // ---------- 共享连接 Db ----------

    fn cleanup_all(path: &Path) {
        for suffix in ["", "-wal", "-shm", ".bak"] {
            let _ = std::fs::remove_file(format!("{}{}", path.display(), suffix));
        }
        // 隔离留证文件（.corrupt-*）
        let prefix = format!("{}.", path.display());
        if let Ok(entries) = std::fs::read_dir(path.parent().unwrap_or(Path::new("."))) {
            for e in entries.flatten() {
                let p = e.path();
                if p.to_string_lossy().starts_with(&prefix)
                    && p.to_string_lossy().contains(".corrupt-")
                {
                    let _ = std::fs::remove_file(&p);
                }
            }
        }
    }

    fn corrupt_artifacts(path: &Path) -> Vec<PathBuf> {
        let prefix = format!("{}.", path.display());
        std::fs::read_dir(path.parent().unwrap_or(Path::new(".")))
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                let s = p.to_string_lossy();
                s.starts_with(&prefix) && s.contains(".corrupt-")
            })
            .collect()
    }

    /// 破坏文件头（抹掉 "SQLite format 3\0" 魔数）→ 典型的 NotADatabase 信号
    fn corrupt_file_header(path: &Path) {
        use std::io::{Seek, SeekFrom, Write};
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .open(path)
            .expect("open db for corruption");
        f.seek(SeekFrom::Start(0)).unwrap();
        f.write_all(&[0u8; 32]).unwrap();
    }

    #[tokio::test]
    async fn db_open_is_idempotent_and_pragmas_apply() {
        let path = temp_db("db-open");
        let db = Db::open(&path, IntegrityMode::Full).expect("first open");
        // 第二次 open 同一文件不得报错（schema 只建一次、WAL 重复设置无害）
        let db2 = Db::open(&path, IntegrityMode::Full).expect("second open");
        drop(db2);
        db.with(|conn| {
            let mode: String = conn.query_row("PRAGMA journal_mode", [], |r| r.get(0))?;
            assert_eq!(mode.to_lowercase(), "wal");
            let busy: i64 = conn.query_row("PRAGMA busy_timeout", [], |r| r.get(0))?;
            assert_eq!(busy, 5000, "busy_timeout should be configured");
            let version: i64 =
                conn.query_row("SELECT version FROM meta LIMIT 1", [], |r| r.get(0))?;
            assert_eq!(version, SNAPSHOT_VERSION as i64);
            Ok(())
        })
        .expect("pragma checks");
        drop(db);
        cleanup_all(&path);
    }

    #[tokio::test]
    async fn db_snapshot_roundtrip_with_secret() {
        let _backup_guard = BACKUP_TEST_LOCK.lock().await;
        let path = temp_db("db-roundtrip");
        let db = Db::open(&path, IntegrityMode::Off).expect("open");
        let snap = snapshot_with_mqtt_password();
        let node_id = snap.nodes[0].config.id;
        db.save_snapshot(Some("unit-test-secret"), &snap)
            .expect("save");
        // 库中不落明文
        db.with(|conn| {
            let cfg: String =
                conn.query_row("SELECT config FROM nodes LIMIT 1", [], |r| r.get(0))?;
            assert!(!cfg.contains("p@ssw0rd"));
            assert!(cfg.contains(ENC_PREFIX));
            Ok(())
        })
        .expect("ciphertext check");
        let loaded = db
            .load_snapshot(Some("unit-test-secret"))
            .expect("load")
            .expect("some snapshot");
        assert_eq!(loaded.nodes.len(), 1);
        assert_eq!(loaded.nodes[0].config.id, node_id);
        assert_eq!(
            loaded.nodes[0].config.config.get("password").unwrap(),
            "p@ssw0rd"
        );
        drop(db);
        cleanup_all(&path);
    }

    /// 破损检测：文件头被破坏（NotADatabase）→ 原文件隔离为 .corrupt-* → 从 .bak 自动恢复
    #[tokio::test]
    async fn integrity_recovery_restores_from_bak() {
        let _backup_guard = BACKUP_TEST_LOCK.lock().await;
        let path = temp_db("db-bak");
        save(&path, &snapshot_with_mqtt_password())
            .await
            .expect("seed save");
        // 直接 VACUUM INTO 造备份：绕开 LAST_BACKUP_SECS 全局节流，避免与其他并行测试互相干扰
        let bak = PathBuf::from(format!("{}.bak", path.display()));
        {
            let conn = Connection::open(&path).unwrap();
            let _ = std::fs::remove_file(&bak);
            conn.execute("VACUUM INTO ?1", [bak.to_string_lossy().to_string()])
                .unwrap();
        }
        corrupt_file_header(&path);
        let db = Db::open(&path, IntegrityMode::Full).expect("open must recover from backup");
        let loaded = db
            .load_snapshot(None)
            .expect("load after recovery")
            .unwrap();
        assert_eq!(loaded.nodes.len(), 1, "data must be restored from .bak");
        assert_eq!(loaded.nodes[0].config.name, "mqtt-app");
        assert_eq!(
            corrupt_artifacts(&path).len(),
            1,
            "corrupt file kept for forensics"
        );
        drop(db);
        cleanup_all(&path);
    }

    /// 破损且无备份：坏库隔离留证后按空库启动（不 panic、不阻塞启动）
    #[tokio::test]
    async fn integrity_recovery_without_bak_starts_empty() {
        let _backup_guard = BACKUP_TEST_LOCK.lock().await;
        let path = temp_db("db-nobak");
        save(&path, &snapshot_with_mqtt_password())
            .await
            .expect("seed save");
        corrupt_file_header(&path);
        let db = Db::open(&path, IntegrityMode::Full).expect("open must rebuild empty");
        let loaded = db.load_snapshot(None).expect("load empty").unwrap();
        assert!(loaded.nodes.is_empty(), "rebuilt database must be empty");
        assert_eq!(
            corrupt_artifacts(&path).len(),
            1,
            "corrupt file kept for forensics"
        );
        drop(db);
        cleanup_all(&path);
    }

    /// 检查「成功执行但结果 ≠ ok」（非 NotADatabase 信号）同样触发恢复序列
    #[tokio::test]
    async fn integrity_failed_result_triggers_recovery() {
        let _backup_guard = BACKUP_TEST_LOCK.lock().await;
        let path = temp_db("db-page");
        save(&path, &snapshot_with_mqtt_password())
            .await
            .expect("seed save");
        // 破坏第 2 页内容（魔数完好，integrity_check 报告页级错误 → 首行 ≠ ok）
        {
            use std::io::{Seek, SeekFrom, Write};
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .open(&path)
                .expect("open db for corruption");
            f.seek(SeekFrom::Start(4096)).unwrap();
            f.write_all(&[0xFFu8; 1024]).unwrap();
        }
        let db = Db::open(&path, IntegrityMode::Full).expect("open must recover");
        let loaded = db.load_snapshot(None).expect("load").unwrap();
        assert!(
            loaded.nodes.is_empty(),
            "no .bak present: rebuilt database must be empty"
        );
        assert_eq!(
            corrupt_artifacts(&path).len(),
            1,
            "recovery must isolate the damaged file"
        );
        drop(db);
        cleanup_all(&path);
    }

    /// Off 模式：不做检查、不做恢复——坏文件原样保留（无 .corrupt-*），
    /// 打开按既有错误路径失败（PRAGMA/schema 触碰文件时报 NotADatabase），
    /// 由上层「加载失败 → 空配置启动」兜底，绝不静默重建。
    #[tokio::test]
    async fn integrity_off_skips_check_and_keeps_file() {
        let _backup_guard = BACKUP_TEST_LOCK.lock().await;
        let path = temp_db("db-off");
        save(&path, &snapshot_with_mqtt_password())
            .await
            .expect("seed save");
        corrupt_file_header(&path);
        assert!(
            Db::open(&path, IntegrityMode::Off).is_err(),
            "off mode must not check/recover: corrupt file fails at open via existing error path"
        );
        assert!(
            corrupt_artifacts(&path).is_empty(),
            "off mode must not touch the file"
        );
        cleanup_all(&path);
    }

    /// 错误码分类：Busy/Locked → 退避重试（仍失败跳过检查）；NotADatabase → 恢复；
    /// 其他 → 跳过检查继续启动。破坏性恢复只允许由前两类之外的明确信号触发。
    #[test]
    fn check_error_classification() {
        use rusqlite::ffi;
        fn sqlite_err(raw: i32) -> rusqlite::Error {
            rusqlite::Error::SqliteFailure(ffi::Error::new(raw), Some("db".into()))
        }
        assert!(matches!(
            classify_check_err(sqlite_err(ffi::SQLITE_BUSY)),
            CheckProblem::Busy(_)
        ));
        assert!(matches!(
            classify_check_err(sqlite_err(ffi::SQLITE_LOCKED)),
            CheckProblem::Busy(_)
        ));
        assert!(matches!(
            classify_check_err(sqlite_err(ffi::SQLITE_NOTADB)),
            CheckProblem::NotADatabase(_)
        ));
        // 磁盘 I/O 类（SQLITE_IOERR）与其他非 SqliteFailure 错误都不得触发恢复
        assert!(matches!(
            classify_check_err(sqlite_err(ffi::SQLITE_IOERR)),
            CheckProblem::Other(_)
        ));
        assert!(matches!(
            classify_check_err(rusqlite::Error::InvalidQuery),
            CheckProblem::Other(_)
        ));
    }

    /// 短暂锁竞争（另一实例运行/备份残留的写锁）不得误触发破坏性恢复：
    /// busy_timeout=5000 先于检查设置，检查会等待锁释放后正常通过，文件不动。
    #[tokio::test(flavor = "multi_thread")]
    async fn busy_lock_waits_out_transient_contention() {
        let path = temp_db("db-busy");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch("CREATE TABLE t (x)").unwrap();
        }
        let holder = {
            let path = path.clone();
            std::thread::spawn(move || {
                let conn = Connection::open(&path).unwrap();
                conn.execute_batch("BEGIN EXCLUSIVE").unwrap();
                std::thread::sleep(std::time::Duration::from_millis(300));
                conn.execute_batch("COMMIT").unwrap();
            })
        };
        std::thread::sleep(std::time::Duration::from_millis(50));
        let db = Db::open(&path, IntegrityMode::Full)
            .expect("transient lock contention must wait out, not recover");
        holder.join().unwrap();
        assert!(
            corrupt_artifacts(&path).is_empty(),
            "transient contention must not trigger recovery"
        );
        drop(db);
        cleanup_all(&path);
    }

    /// 共享连接串行正确性：多任务并发混合读写快照与草稿表（模拟 users 共库短事务），
    /// 最终数据一致——快照整体等于某一次保存的完整状态，计数器恰好等于全部增量之和。
    #[tokio::test(flavor = "multi_thread")]
    async fn concurrent_tasks_serialize_on_shared_connection() {
        let _backup_guard = BACKUP_TEST_LOCK.lock().await;
        let path = temp_db("db-conc");
        let db = Db::open(&path, IntegrityMode::Off).expect("open");
        db.with(|conn| {
            conn.execute_batch("CREATE TABLE IF NOT EXISTS t_ops (n INTEGER NOT NULL)")?;
            conn.execute("INSERT INTO t_ops (n) VALUES (0)", [])?;
            Ok(())
        })
        .expect("seed scratch table");

        const TASKS: usize = 4;
        const ITERS: usize = 5;
        let mut handles = Vec::new();
        for t in 0..TASKS {
            let db = Arc::clone(&db);
            handles.push(tokio::task::spawn_blocking(move || {
                for _ in 0..ITERS {
                    // 快照保存：每个任务维护自己的节点集（最终数量互不相同，便于识别）
                    let mut snap = Snapshot::default();
                    for k in 0..=t {
                        snap.nodes.push(node_with(
                            &format!("node-t{t}-{k}"),
                            serde_json::json!({ "poll_base_ms": 1000 }),
                        ));
                    }
                    db.save_snapshot(None, &snap).expect("save");
                    // 共库短事务写（模拟 users 操作）
                    db.with(|conn| {
                        conn.execute("UPDATE t_ops SET n = n + 1", [])?;
                        Ok(())
                    })
                    .expect("op");
                }
            }));
        }
        for h in handles {
            h.await.expect("task join");
        }
        let loaded = db.load_snapshot(None).expect("final load").unwrap();
        assert!(
            (1..=TASKS).contains(&loaded.nodes.len()),
            "loaded snapshot must be one complete saved state, got {} nodes",
            loaded.nodes.len()
        );
        let t = loaded.nodes.len() - 1;
        for k in 0..loaded.nodes.len() {
            assert_eq!(
                loaded.nodes[k].config.name,
                format!("node-t{t}-{k}"),
                "no torn state: node names must match exactly one save"
            );
        }
        let ops: i64 = db
            .with(|conn| Ok(conn.query_row("SELECT n FROM t_ops", [], |r| r.get(0))?))
            .expect("ops count");
        assert_eq!(
            ops,
            (TASKS * ITERS) as i64,
            "every short transaction must land"
        );
        drop(db);
        cleanup_all(&path);
    }

    /// v1 → v2 迁移：既有库加 tenant_id 列后，新列默认为 'default'，规则按 source 节点补章
    #[tokio::test]
    async fn v1_to_v2_migration_adds_tenant_id_columns() {
        let _backup_guard = BACKUP_TEST_LOCK.lock().await;
        let path = temp_db("v1-mig");

        // 用 v1 schema 手工造库（不含 tenant_id 列）
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE meta (version INTEGER NOT NULL);
                 INSERT INTO meta VALUES (1);
                 CREATE TABLE nodes (id TEXT PRIMARY KEY, name TEXT NOT NULL, kind TEXT NOT NULL,
                   plugin_name TEXT NOT NULL, config TEXT NOT NULL, state TEXT NOT NULL);
                 CREATE TABLE rules (id TEXT PRIMARY KEY, name TEXT NOT NULL, enabled INTEGER NOT NULL,
                   south_node_id TEXT NOT NULL, group_id TEXT NOT NULL, tag_name TEXT NOT NULL,
                   op TEXT NOT NULL, threshold REAL NOT NULL, for_ms INTEGER NOT NULL,
                   clear_ms INTEGER NOT NULL, action TEXT NOT NULL);
                 CREATE TABLE groups (node_id TEXT NOT NULL, group_id TEXT NOT NULL,
                   name TEXT NOT NULL, interval_ms INTEGER NOT NULL, description TEXT,
                   PRIMARY KEY (node_id, group_id));",
            )
            .unwrap();
            // 插入一个节点和一条规则（用真实 UUID 格式）
            let node_uuid = "550e8400-e29b-41d4-a716-446655440001";
            let group_uuid = "550e8400-e29b-41d4-a716-446655440002";
            conn.execute(
                &format!(
                    "INSERT INTO nodes VALUES ('{}', 'node-a', 'south', 'sim', '{{}}', 'running')",
                    node_uuid
                ),
                [],
            )
            .unwrap();
            conn.execute(
                &format!("INSERT INTO rules VALUES ('rid1', 'rule-1', 1, '{}', '{}', 'temp', 'gt', 30.0, 0, 0, '{{\"type\":\"log\",\"message\":\"hi\"}}')", node_uuid, group_uuid),
                [],
            )
            .unwrap();
        }

        // 走 ensure_schema 迁移路径
        let db = Db::open(&path, IntegrityMode::Off).expect("open migrated db");
        let loaded = db
            .load_snapshot(None)
            .expect("load snapshot")
            .expect("some");

        // 节点和规则都带了 tenant_id
        assert_eq!(loaded.nodes.len(), 1);
        assert_eq!(loaded.nodes[0].config.tenant_id, "default");
        assert_eq!(loaded.rules.len(), 1);
        assert_eq!(loaded.rules[0].tenant_id, "default");

        // meta 版本升到 2
        let version: i64 = db
            .with(|conn| Ok(conn.query_row("SELECT version FROM meta LIMIT 1", [], |r| r.get(0))?))
            .expect("get version");
        assert_eq!(version, 2);

        drop(db);
        cleanup_all(&path);
    }

    /// v2 快照 roundtrip：两个域的节点/规则域不串
    #[tokio::test]
    async fn snapshot_roundtrip_preserves_tenant_isolation() {
        let _backup_guard = BACKUP_TEST_LOCK.lock().await;
        let path = temp_db("tenant-iso");

        let make_node = |name: &str, tenant: &str| -> Node {
            let mut n = node_with(name, serde_json::json!({}));
            n.config.tenant_id = tenant.to_string();
            n.state = NodeState::Running;
            n
        };

        let snap = Snapshot {
            version: SNAPSHOT_VERSION,
            nodes: vec![
                make_node("node-a", "tenant-a"),
                make_node("node-b", "tenant-b"),
            ],
            rules: vec![crate::rules::Rule {
                id: "r1".to_string(),
                name: "rule-a".to_string(),
                enabled: true,
                source: crate::rules::RuleSource {
                    south_node_id: NodeId::new(),
                    group_id: GroupId::new(),
                    tag_name: "temp".to_string(),
                },
                condition: crate::rules::RuleCondition {
                    op: crate::rules::CompareOp::Gt,
                    threshold: 30.0,
                },
                for_ms: 0,
                clear_ms: 0,
                action: crate::rules::RuleAction::Log {
                    message: "hi".to_string(),
                },
                tenant_id: "tenant-a".to_string(),
            }],
            ..Default::default()
        };

        save(&path, &snap).await.expect("save");
        let loaded = load(&path).await.expect("load").expect("some");

        assert_eq!(loaded.nodes.len(), 2);
        let t1 = loaded
            .nodes
            .iter()
            .find(|n| n.config.name == "node-a")
            .unwrap();
        let t2 = loaded
            .nodes
            .iter()
            .find(|n| n.config.name == "node-b")
            .unwrap();
        assert_eq!(t1.config.tenant_id, "tenant-a");
        assert_eq!(t2.config.tenant_id, "tenant-b");
        assert_eq!(loaded.rules[0].tenant_id, "tenant-a");

        cleanup_all(&path);
    }
}
