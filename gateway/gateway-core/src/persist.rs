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
use gateway_sdk::{Group, GroupSubscription, NodeId, Tag};
use gateway_sdk::types::{GroupId, NodeKind, NodeState, TagAttr, TagId};
use rusqlite::{params, Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tracing::info;

/// 当前快照/schema 版本。大于此版本需升级程序。
pub const SNAPSHOT_VERSION: u32 = 1;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS meta (version INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS nodes (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  kind TEXT NOT NULL,
  plugin_name TEXT NOT NULL,
  config TEXT NOT NULL,
  state TEXT NOT NULL
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
}

impl Default for Snapshot {
    fn default() -> Self {
        Self {
            version: SNAPSHOT_VERSION,
            nodes: Vec::new(),
            groups: Vec::new(),
            tags: Vec::new(),
            subscriptions: Vec::new(),
        }
    }
}

impl Snapshot {
    /// 校验快照：版本、节点 ID 唯一性等
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
) -> Snapshot {
    Snapshot {
        version: SNAPSHOT_VERSION,
        nodes,
        groups,
        tags,
        subscriptions,
    }
}

fn ensure_schema(conn: &Connection) -> Result<(), rusqlite::Error> {
    apply_pragmas(conn)?;
    conn.execute_batch(SCHEMA)?;
    let mut stmt = conn.prepare("SELECT version FROM meta LIMIT 1")?;
    let has_version = stmt.exists([])?;
    drop(stmt);
    if !has_version {
        conn.execute("INSERT INTO meta (version) VALUES (?1)", [SNAPSHOT_VERSION])?;
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
    let placeholders = vec!["?"; keys.len()].join(", ");
    let sql = format!(
        "DELETE FROM {} WHERE {} NOT IN ({})",
        table, key_expr, placeholders
    );
    let params: Vec<&dyn rusqlite::ToSql> =
        keys.iter().map(|k| k as &dyn rusqlite::ToSql).collect();
    tx.execute(&sql, params.as_slice())?;
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
        _ => Err(PersistError::Validation(format!("unknown node kind: {}", s))),
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
        _ => Err(PersistError::Validation(format!("unknown node state: {}", s))),
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
    let mut stmt = conn.prepare(
        "SELECT id, name, kind, plugin_name, config, state FROM nodes",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
        ))
    })?;
    for row in rows {
        let (id, name, kind, plugin_name, config, state) = row?;
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
            },
            state,
        });
    }

    let mut groups = Vec::new();
    let mut stmt = conn.prepare(
        "SELECT node_id, group_id, name, interval_ms, description FROM groups",
    )?;
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
    let mut stmt = conn.prepare(
        "SELECT north_node_id, south_node_id, group_id FROM subscriptions",
    )?;
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
        sub_map
            .entry(north)
            .or_default()
            .push(GroupSubscription {
                south_node_id: south,
                group_id: gid,
            });
    }
    let subscriptions = sub_map.into_iter().collect();

    Ok(Snapshot {
        version,
        nodes,
        groups,
        tags,
        subscriptions,
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
    let pt = cipher
        .decrypt(Nonce::from_slice(nonce_bytes), ct)
        .ok()?;
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

    // 增量写入：UPSERT 只影响真正变化的行，配合末尾的“删除差集”保持全量语义。
    // 相比原来的「DELETE 全表 + 全量 INSERT」，变更 1 个点位不再重写整库。
    for n in &s.nodes {
        tx.execute(
            "INSERT INTO nodes (id, name, kind, plugin_name, config, state) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                kind = excluded.kind,
                plugin_name = excluded.plugin_name,
                config = excluded.config,
                state = excluded.state",
            params![
                n.config.id.0.to_string(),
                n.config.name,
                node_kind_to_str(n.config.kind),
                n.config.plugin_name,
                serde_json::to_string(&n.config.config)?,
                node_state_to_str(n.state),
            ],
        )?;
    }
    let node_keys: Vec<String> = s.nodes.iter().map(|n| n.config.id.0.to_string()).collect();
    delete_missing(&tx, "nodes", "id", &node_keys)?;

    for (nid, g) in &s.groups {
        tx.execute(
            "INSERT INTO groups (node_id, group_id, name, interval_ms, description) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(node_id, group_id) DO UPDATE SET
                name = excluded.name,
                interval_ms = excluded.interval_ms,
                description = excluded.description",
            params![
                nid.0.to_string(),
                g.id.0.to_string(),
                g.name,
                g.interval_ms as i64,
                g.description,
            ],
        )?;
    }
    let group_keys: Vec<String> = s
        .groups
        .iter()
        .map(|(nid, g)| format!("{}|{}", nid.0, g.id.0))
        .collect();
    delete_missing(&tx, "groups", "node_id || '|' || group_id", &group_keys)?;

    for (nid, t) in &s.tags {
        tx.execute(
            "INSERT INTO tags (tag_id, node_id, group_id, name, address, attr, data_type, description) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(tag_id) DO UPDATE SET
                node_id = excluded.node_id,
                group_id = excluded.group_id,
                name = excluded.name,
                address = excluded.address,
                attr = excluded.attr,
                data_type = excluded.data_type,
                description = excluded.description",
            params![
                t.id.0.to_string(),
                nid.0.to_string(),
                t.group_id.0.to_string(),
                t.name,
                t.address,
                tag_attr_to_str(t.attr),
                t.data_type,
                t.description,
            ],
        )?;
    }
    let tag_keys: Vec<String> = s.tags.iter().map(|(_, t)| t.id.0.to_string()).collect();
    delete_missing(&tx, "tags", "tag_id", &tag_keys)?;

    for (north_id, subs) in &s.subscriptions {
        for sub in subs {
            tx.execute(
                "INSERT INTO subscriptions (north_node_id, south_node_id, group_id) VALUES (?1, ?2, ?3)
                 ON CONFLICT(north_node_id, south_node_id, group_id) DO NOTHING",
                params![
                    north_id.0.to_string(),
                    sub.south_node_id.0.to_string(),
                    sub.group_id.0.to_string(),
                ],
            )?;
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
    conn.execute("VACUUM INTO ?1", [backup_path.to_string_lossy().to_string()])?;
    info!(path = %backup_path.display(), "database snapshot created");
    Ok(())
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::Node;
    use gateway_sdk::types::NodeKind;

    fn temp_db(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("gw-test-{}-{}.db", name, uuid::Uuid::new_v4()));
        let _ = std::fs::remove_file(&p);
        let _ = std::fs::remove_file(format!("{}.bak", p.display()));
        p
    }

    fn node_with(name: &str, config: serde_json::Value) -> Node {
        let mut n = Node::new(name, NodeKind::South, "sim", serde_json::from_value(config).unwrap());
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
        let path = temp_db("roundtrip");
        let snap = snapshot_with_mqtt_password();
        let node_id = snap.nodes[0].config.id;
        save(&path, &snap).await.expect("save failed");

        let loaded = load(&path).await.expect("load failed").expect("no snapshot");
        assert_eq!(loaded.nodes.len(), 1);
        assert_eq!(loaded.nodes[0].config.id, node_id);
        assert_eq!(loaded.nodes[0].config.name, "mqtt-app");
        assert_eq!(loaded.nodes[0].state, NodeState::Running);
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn secret_is_encrypted_at_rest_and_decrypted_on_load() {
        let path = temp_db("enc");
        let snap = snapshot_with_mqtt_password();
        save_secret(&path, &snap, Some("unit-test-secret")).await.expect("save failed");

        // 数据库中不应出现明文口令
        let conn = Connection::open(&path).unwrap();
        let cfg: String = conn
            .query_row("SELECT config FROM nodes LIMIT 1", [], |r| r.get(0))
            .unwrap();
        assert!(!cfg.contains("p@ssw0rd"), "plaintext password leaked into db");
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
        let path = temp_db("wrongkey");
        let snap = snapshot_with_mqtt_password();
        save_secret(&path, &snap, Some("right-secret")).await.unwrap();

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
        let path = temp_db("wal");
        save(&path, &snapshot_with_mqtt_password()).await.expect("save");
        let conn = Connection::open(&path).unwrap();
        let mode: String = conn
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap();
        assert_eq!(mode.to_lowercase(), "wal", "WAL should be enabled for concurrent access");
        let busy: i64 = conn.query_row("PRAGMA busy_timeout", [], |r| r.get(0)).unwrap();
        assert_eq!(busy, 5000, "busy_timeout should be configured");
        drop(conn);
        cleanup_db(&path);
    }

    #[tokio::test]
    async fn incremental_save_updates_and_removes_rows() {
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
        assert_eq!(loaded.nodes.len(), 1, "removed node must not survive an incremental save");
        assert_eq!(loaded.nodes[0].config.id, keep_id);
        assert_eq!(loaded.nodes[0].config.name, "renamed", "upsert should apply updates");

        // 重复保存同一快照不应产生重复行
        save(&path, &snap).await.expect("third save");
        let again = load(&path).await.unwrap().unwrap();
        assert_eq!(again.nodes.len(), 1);
        cleanup_db(&path);
    }

    #[tokio::test]
    async fn throttled_backup_creates_consistent_snapshot() {
        let path = temp_db("bak");
        save(&path, &snapshot_with_mqtt_password()).await.expect("save");
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
        assert_eq!(before, after, "backup should be throttled within the interval");

        drop(bak_conn);
        drop(conn);
        cleanup_db(&path);
        let _ = std::fs::remove_file(&bak);
    }
}
