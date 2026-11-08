//! 流程编排持久化：data.db 附加表 `flows`（设计 docs/design/流程编排-v2.md §2.2）。
//!
//! 附加表策略：`CREATE TABLE IF NOT EXISTS`，**不参与主快照的版本化**——旧版本加载器
//! 容忍多出的表，因此不递增 meta.version；备份/恢复语义与快照表的一致性由后续批次的
//! 快照导出/导入对齐。操作经 `persist::Db::with` 在共享连接上串行执行（与 users 同法）。

use gateway_core::{Db, PersistError};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;
use uuid::Uuid;

use crate::api::scope::TenantScope;

const FLOWS_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS flows (
  id TEXT PRIMARY KEY,
  tenant_id TEXT NOT NULL,
  name TEXT NOT NULL,
  description TEXT,
  status TEXT NOT NULL DEFAULT 'draft',
  version INTEGER NOT NULL DEFAULT 0,
  graph_json TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  UNIQUE(tenant_id, name)
);
CREATE INDEX IF NOT EXISTS idx_flows_tenant ON flows(tenant_id, updated_at DESC);
"#;

/// 一条流程记录。`graph_json` 只含图部分（nodes/edges，含编辑器 position）；
/// 流程元数据（id/name/status/version）为显式列。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowRow {
    pub id: Uuid,
    pub tenant_id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// draft | published
    pub status: String,
    pub version: i64,
    pub graph_json: serde_json::Value,
    /// epoch ms
    pub created_at: i64,
    pub updated_at: i64,
}

impl FlowRow {
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<FlowRow> {
        Ok(FlowRow {
            id: Uuid::parse_str(&row.get::<_, String>(0)?).map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, e.into())
            })?,
            tenant_id: row.get(1)?,
            name: row.get(2)?,
            description: row.get(3)?,
            status: row.get(4)?,
            version: row.get(5)?,
            graph_json: serde_json::from_str(&row.get::<_, String>(6)?).map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(6, rusqlite::types::Type::Text, e.into())
            })?,
            created_at: row.get(7)?,
            updated_at: row.get(8)?,
        })
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 流程存储。`db` 与配置快照/users 共用同一进程内长连接。
pub struct FlowStore {
    db: Arc<Db>,
}

impl FlowStore {
    /// 打开并确保 schema（幂等）。
    pub fn open(db: Arc<Db>) -> Result<Self, PersistError> {
        db.with(|conn| conn.execute_batch(FLOWS_SCHEMA).map_err(PersistError::from))?;
        Ok(Self { db })
    }

    /// 内存库降级路径（AppState 在 `db=None` 的测试/降级场景使用，保证 API 始终可用）。
    pub fn open_in_memory() -> Self {
        let db = Db::open(Path::new(":memory:"), gateway_core::IntegrityMode::Off)
            .expect("open in-memory db");
        Self::open(db).expect("ensure flows schema (in-memory)")
    }

    /// AppState 组装入口：有共享连接就复用（确保 schema），失败或缺失则退化为内存库。
    pub fn open_best_effort(db: Option<Arc<Db>>) -> Arc<Self> {
        match db {
            Some(d) => match Self::open(d) {
                Ok(s) => Arc::new(s),
                Err(e) => {
                    tracing::warn!("flows schema ensure failed, using in-memory flows db: {e}");
                    Arc::new(Self::open_in_memory())
                }
            },
            None => Arc::new(Self::open_in_memory()),
        }
    }

    pub fn insert(&self, row: &FlowRow) -> Result<(), PersistError> {
        self.db.with(|conn| {
            conn.execute(
                "INSERT INTO flows (id, tenant_id, name, description, status, version, graph_json, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    row.id.to_string(),
                    row.tenant_id,
                    row.name,
                    row.description,
                    row.status,
                    row.version,
                    row.graph_json.to_string(),
                    row.created_at,
                    row.updated_at,
                ],
            )?;
            Ok(())
        })
    }

    /// 整行更新（不含 status/version——部署生命周期走 `set_status`）。返回 false = 不存在。
    pub fn update(&self, row: &FlowRow) -> Result<bool, PersistError> {
        self.db.with(|conn| {
            let n = conn.execute(
                "UPDATE flows SET name = ?2, description = ?3, graph_json = ?4, updated_at = ?5 WHERE id = ?1",
                params![
                    row.id.to_string(),
                    row.name,
                    row.description,
                    row.graph_json.to_string(),
                    now_ms(),
                ],
            )?;
            Ok(n > 0)
        })
    }

    /// 删除（租户过滤：跨租户删除视为不存在）。返回 false = 不存在。
    pub fn delete(&self, id: Uuid, tenant: &TenantScope) -> Result<bool, PersistError> {
        self.db.with(|conn| {
            let n = conn.execute(
                "DELETE FROM flows WHERE id = ?1 AND (?2 = '*' OR tenant_id = ?2)",
                params![id.to_string(), tenant_filter(tenant)],
            )?;
            Ok(n > 0)
        })
    }

    pub fn get(&self, id: Uuid, tenant: &TenantScope) -> Result<Option<FlowRow>, PersistError> {
        self.db.with(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, tenant_id, name, description, status, version, graph_json, created_at, updated_at
                 FROM flows WHERE id = ?1 AND (?2 = '*' OR tenant_id = ?2)",
            )?;
            let mut rows = stmt.query_map(params![id.to_string(), tenant_filter(tenant)], FlowRow::from_row)?;
            match rows.next() {
                Some(r) => Ok(Some(r?)),
                None => Ok(None),
            }
        })
    }

    pub fn list(&self, tenant: &TenantScope) -> Result<Vec<FlowRow>, PersistError> {
        self.db.with(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, tenant_id, name, description, status, version, graph_json, created_at, updated_at
                 FROM flows WHERE (?1 = '*' OR tenant_id = ?1) ORDER BY updated_at DESC",
            )?;
            let rows = stmt.query_map(params![tenant_filter(tenant)], FlowRow::from_row)?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r?);
            }
            Ok(out)
        })
    }

    /// 部署生命周期：status 切换；publish 时 version+1。返回 None = 不存在。
    pub fn set_status(
        &self,
        id: Uuid,
        tenant: &TenantScope,
        status: &str,
        bump_version: bool,
    ) -> Result<Option<FlowRow>, PersistError> {
        self.db.with(|conn| {
            let n = conn.execute(
                "UPDATE flows SET status = ?3, version = version + ?4, updated_at = ?5
                 WHERE id = ?1 AND (?2 = '*' OR tenant_id = ?2)",
                params![id.to_string(), tenant_filter(tenant), status, i64::from(bump_version), now_ms()],
            )?;
            if n == 0 {
                return Ok(None);
            }
            let mut stmt = conn.prepare(
                "SELECT id, tenant_id, name, description, status, version, graph_json, created_at, updated_at
                 FROM flows WHERE id = ?1",
            )?;
            let mut rows = stmt.query_map(params![id.to_string()], FlowRow::from_row)?;
            match rows.next() {
                Some(r) => Ok(Some(r?)),
                None => Ok(None),
            }
        })
    }
}

/// TenantScope → SQL 过滤值（与 history_tenant_filter 同约定：All → '*'）
fn tenant_filter(tenant: &TenantScope) -> String {
    match tenant {
        TenantScope::All => "*".to_string(),
        TenantScope::One(t) => t.clone(),
    }
}
