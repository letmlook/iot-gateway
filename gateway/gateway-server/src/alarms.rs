//! 告警事件存储：data.db 附加表 `alarm_events`（设计 docs/design/流程编排-v2.md §2.2/§4）。
//!
//! 去重语义：同 (tenant, flow, node, tag, level) 的 `active` 事件存在时，新命中只前移
//! `last_ts`、刷新值并累加 `occurrences`，不建新行（redesign AlarmStore 的
//! duplicate-suppression 语义保留）。生命周期：active → acknowledged → resolved。

use gateway_core::{Db, PersistError};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;
use uuid::Uuid;

use crate::api::scope::TenantScope;

const ALARMS_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS alarm_events (
  id TEXT PRIMARY KEY,
  tenant_id TEXT NOT NULL,
  flow_id TEXT NOT NULL,
  node_name TEXT NOT NULL,
  level TEXT NOT NULL,
  state TEXT NOT NULL DEFAULT 'active',
  tag TEXT NOT NULL,
  value REAL,
  threshold REAL,
  message TEXT NOT NULL,
  occurrences INTEGER NOT NULL DEFAULT 1,
  first_ts INTEGER NOT NULL,
  last_ts INTEGER NOT NULL,
  acked_by TEXT,
  acked_at INTEGER,
  resolved_at INTEGER
);
CREATE INDEX IF NOT EXISTS idx_alarm_open ON alarm_events(tenant_id, state, first_ts DESC);
"#;

/// 一条告警事件记录（API 序列化形状）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlarmRecord {
    pub id: Uuid,
    pub tenant_id: String,
    pub flow_id: String,
    pub node_name: String,
    pub level: String,
    /// active | acknowledged | resolved
    pub state: String,
    pub tag: String,
    pub value: Option<f64>,
    pub threshold: Option<f64>,
    pub message: String,
    pub occurrences: i64,
    pub first_ts: i64,
    pub last_ts: i64,
    pub acked_by: Option<String>,
    pub acked_at: Option<i64>,
    pub resolved_at: Option<i64>,
}

fn record_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AlarmRecord> {
    let parse_uuid = |v: Result<String, rusqlite::Error>, col| {
        Uuid::parse_str(&v?).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(col, rusqlite::types::Type::Text, e.into())
        })
    };
    Ok(AlarmRecord {
        id: parse_uuid(row.get(0), 0)?,
        tenant_id: row.get(1)?,
        flow_id: row.get(2)?,
        node_name: row.get(3)?,
        level: row.get(4)?,
        state: row.get(5)?,
        tag: row.get(6)?,
        value: row.get(7)?,
        threshold: row.get(8)?,
        message: row.get(9)?,
        occurrences: row.get(10)?,
        first_ts: row.get(11)?,
        last_ts: row.get(12)?,
        acked_by: row.get(13)?,
        acked_at: row.get(14)?,
        resolved_at: row.get(15)?,
    })
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn tenant_filter(tenant: &TenantScope) -> String {
    match tenant {
        TenantScope::All => "*".to_string(),
        TenantScope::One(t) => t.clone(),
    }
}

pub struct AlarmStore {
    db: Arc<Db>,
}

impl AlarmStore {
    pub fn open(db: Arc<Db>) -> Result<Self, PersistError> {
        db.with(|conn| {
            conn.execute_batch(ALARMS_SCHEMA)
                .map_err(PersistError::from)
        })?;
        Ok(Self { db })
    }

    /// AppState 组装入口（db=None 的测试/降级场景退化为内存库）。
    pub fn open_best_effort(db: Option<Arc<Db>>) -> Arc<Self> {
        match db {
            Some(d) => match Self::open(d) {
                Ok(s) => Arc::new(s),
                Err(e) => {
                    tracing::warn!("alarms schema ensure failed, using in-memory db: {e}");
                    Arc::new(Self::in_memory())
                }
            },
            None => Arc::new(Self::in_memory()),
        }
    }

    fn in_memory() -> Self {
        let db = Db::open(Path::new(":memory:"), gateway_core::IntegrityMode::Off)
            .expect("open in-memory db");
        Self::open(db).expect("ensure alarms schema (in-memory)")
    }

    /// 写入告警（同键 active 去重前移）。`node_name` 为流程内 sink 节点名。
    pub fn raise(
        &self,
        tenant_id: &str,
        flow_id: &Uuid,
        node_name: &str,
        ev: &gateway_flow::AlarmEvent,
    ) -> Result<Uuid, PersistError> {
        let ts = ev.ts.timestamp_millis();
        self.db.with(|conn| {
            let bumped = conn.execute(
                "UPDATE alarm_events SET last_ts = ?1, value = ?2, threshold = ?3, message = ?4,
                 occurrences = occurrences + 1
                 WHERE tenant_id = ?5 AND flow_id = ?6 AND node_name = ?7 AND tag = ?8
                   AND level = ?9 AND state = 'active'",
                params![
                    ts,
                    ev.value,
                    ev.threshold,
                    ev.message,
                    tenant_id,
                    flow_id.to_string(),
                    node_name,
                    ev.tag,
                    ev.level.as_str(),
                ],
            )?;
            if bumped > 0 {
                let id: String = conn.query_row(
                    "SELECT id FROM alarm_events WHERE tenant_id = ?1 AND flow_id = ?2
                       AND node_name = ?3 AND tag = ?4 AND level = ?5 AND state = 'active'",
                    params![
                        tenant_id,
                        flow_id.to_string(),
                        node_name,
                        ev.tag,
                        ev.level.as_str()
                    ],
                    |r| r.get(0),
                )?;
                return Uuid::parse_str(&id)
                    .map_err(|e| PersistError::Validation(format!("bad alarm id: {e}")));
            }
            let id = Uuid::new_v4();
            conn.execute(
                "INSERT INTO alarm_events
                   (id, tenant_id, flow_id, node_name, level, state, tag, value, threshold,
                    message, occurrences, first_ts, last_ts)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'active', ?6, ?7, ?8, ?9, 1, ?10, ?10)",
                params![
                    id.to_string(),
                    tenant_id,
                    flow_id.to_string(),
                    node_name,
                    ev.level.as_str(),
                    ev.tag,
                    ev.value,
                    ev.threshold,
                    ev.message,
                    ts,
                ],
            )?;
            Ok(id)
        })
    }

    /// 列表（租户过滤；可选 state/level 过滤；按 last_ts 倒序）。
    pub fn list(
        &self,
        tenant: &TenantScope,
        state_filter: Option<&str>,
        level: Option<&str>,
        limit: i64,
    ) -> Result<Vec<AlarmRecord>, PersistError> {
        self.db.with(|conn| {
            let mut sql = String::from(
                "SELECT id, tenant_id, flow_id, node_name, level, state, tag, value, threshold,
                        message, occurrences, first_ts, last_ts, acked_by, acked_at, resolved_at
                 FROM alarm_events WHERE (?1 = '*' OR tenant_id = ?1)",
            );
            let mut args: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(tenant_filter(tenant))];
            if let Some(s) = state_filter {
                sql.push_str(&format!(" AND state = ?{}", args.len() + 1));
                args.push(Box::new(s.to_string()));
            }
            if let Some(l) = level {
                sql.push_str(&format!(" AND level = ?{}", args.len() + 1));
                args.push(Box::new(l.to_string()));
            }
            sql.push_str(&format!(" ORDER BY last_ts DESC LIMIT ?{}", args.len() + 1));
            args.push(Box::new(limit.clamp(1, 1000)));
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(rusqlite::params_from_iter(args.iter()), record_from_row)?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r?);
            }
            Ok(out)
        })
    }

    /// active → acknowledged。非 active 状态返回 Err 语义由调用方转 400；不存在返回 Ok(false)。
    pub fn ack(&self, id: Uuid, tenant: &TenantScope, by: &str) -> Result<bool, PersistError> {
        self.db.with(|conn| {
            let n = conn.execute(
                "UPDATE alarm_events SET state = 'acknowledged', acked_by = ?3, acked_at = ?4
                 WHERE id = ?1 AND (?2 = '*' OR tenant_id = ?2) AND state = 'active'",
                params![id.to_string(), tenant_filter(tenant), by, now_ms()],
            )?;
            Ok(n > 0)
        })
    }

    /// active|acknowledged → resolved（终态）。
    pub fn resolve(&self, id: Uuid, tenant: &TenantScope) -> Result<bool, PersistError> {
        self.db.with(|conn| {
            let n = conn.execute(
                "UPDATE alarm_events SET state = 'resolved', resolved_at = ?3
                 WHERE id = ?1 AND (?2 = '*' OR tenant_id = ?2) AND state IN ('active', 'acknowledged')",
                params![id.to_string(), tenant_filter(tenant), now_ms()],
            )?;
            Ok(n > 0)
        })
    }
}
