//! 持久化：节点/组/标签/订阅落盘，对标 Neuron 配置与状态管理。
//!
//! ## 对标 Neuron
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
    conn.execute_batch(SCHEMA)?;
    let mut stmt = conn.prepare("SELECT version FROM meta LIMIT 1")?;
    let has_version = stmt.exists([])?;
    drop(stmt);
    if !has_version {
        conn.execute("INSERT INTO meta (version) VALUES (?1)", [SNAPSHOT_VERSION])?;
    }
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

fn save_to_db(conn: &Connection, s: &Snapshot) -> Result<(), PersistError> {
    s.validate()?;
    let tx = conn.unchecked_transaction()?;
    tx.execute("DELETE FROM nodes", [])?;
    tx.execute("DELETE FROM groups", [])?;
    tx.execute("DELETE FROM tags", [])?;
    tx.execute("DELETE FROM subscriptions", [])?;
    tx.execute("UPDATE meta SET version = ?1", [s.version])?;

    for n in &s.nodes {
        tx.execute(
            "INSERT INTO nodes (id, name, kind, plugin_name, config, state) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
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
    for (nid, g) in &s.groups {
        tx.execute(
            "INSERT INTO groups (node_id, group_id, name, interval_ms, description) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                nid.0.to_string(),
                g.id.0.to_string(),
                g.name,
                g.interval_ms as i64,
                g.description,
            ],
        )?;
    }
    for (nid, t) in &s.tags {
        tx.execute(
            "INSERT INTO tags (tag_id, node_id, group_id, name, address, attr, data_type, description) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
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
    for (north_id, subs) in &s.subscriptions {
        for sub in subs {
            tx.execute(
                "INSERT INTO subscriptions (north_node_id, south_node_id, group_id) VALUES (?1, ?2, ?3)",
                params![
                    north_id.0.to_string(),
                    sub.south_node_id.0.to_string(),
                    sub.group_id.0.to_string(),
                ],
            )?;
        }
    }
    tx.commit()?;
    Ok(())
}

/// 从 SQLite 数据库加载快照。path 为 db 文件；文件不存在返回 `Ok(None)`。
pub async fn load(path: &Path) -> Result<Option<Snapshot>, PersistError> {
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
    .map_err(|e| PersistError::Io(std::io::Error::new(std::io::ErrorKind::Other, e)))?;
    let s = snap?;
    info!(path = %path_log, version = s.version, "persist loaded");
    Ok(Some(s))
}

/// 将快照写入 SQLite。path 为 db 文件；不存在则创建。若文件已存在则先备份为 {path}.bak。
pub async fn save(path: &Path, s: &Snapshot) -> Result<(), PersistError> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    if path.exists() {
        let backup_path = path
            .parent()
            .map(|p| p.join(path.file_name().unwrap().to_string_lossy().to_string() + ".bak"))
            .unwrap_or_else(|| PathBuf::from(path.to_string_lossy().to_string() + ".bak"));
        if let Err(e) = tokio::fs::copy(path, &backup_path).await {
            tracing::warn!(path = ?backup_path, "backup before save failed: {}", e);
        }
    }
    let path_buf = path.to_path_buf();
    let path_log = path_buf.display().to_string();
    let snap = s.clone();
    tokio::task::spawn_blocking(move || {
        let conn = Connection::open_with_flags(
            &path_buf,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
        )?;
        ensure_schema(&conn)?;
        save_to_db(&conn, &snap)
    })
    .await
    .map_err(|e| PersistError::Io(std::io::Error::new(std::io::ErrorKind::Other, e)))??;
    info!(path = %path_log, version = s.version, "persist saved");
    Ok(())
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
