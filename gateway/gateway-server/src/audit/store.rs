//! SQLite-backed audit event store.

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    pub id: Uuid,
    pub actor: String,
    pub action: String,
    pub resource_type: String,
    pub resource_id: Option<String>,
    pub request_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub detail: serde_json::Value,
}

#[derive(Clone)]
pub struct AuditStore {
    conn: Arc<Mutex<Connection>>,
    pub db_path: PathBuf,
}

impl AuditStore {
    pub fn new(db_path: &Path) -> Result<Self, rusqlite::Error> {
        let conn = Connection::open(db_path)?;
        let store = Self {
            conn: Arc::new(Mutex::new(conn)),
            db_path: db_path.to_path_buf(),
        };
        store.init()?;
        Ok(store)
    }

    fn init(&self) -> Result<(), rusqlite::Error> {
        let conn = self
            .conn
            .try_lock()
            .expect("AuditStore.init(): mutex should be available immediately");
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS audit_events (
                id TEXT PRIMARY KEY,
                actor TEXT NOT NULL,
                action TEXT NOT NULL,
                resource_type TEXT NOT NULL,
                resource_id TEXT,
                request_id TEXT,
                created_at TEXT NOT NULL,
                detail TEXT NOT NULL DEFAULT '{}'
            );
            CREATE INDEX IF NOT EXISTS idx_audit_events_created_at ON audit_events(created_at);
            CREATE INDEX IF NOT EXISTS idx_audit_events_action ON audit_events(action);
            CREATE INDEX IF NOT EXISTS idx_audit_events_resource ON audit_events(resource_type, resource_id);",
        )?;
        Ok(())
    }

    pub async fn record(
        &self,
        actor: impl Into<String>,
        action: impl Into<String>,
        resource_type: impl Into<String>,
        resource_id: Option<String>,
        request_id: Option<String>,
        detail: serde_json::Value,
    ) -> Result<AuditEvent, String> {
        let event = AuditEvent {
            id: Uuid::new_v4(),
            actor: actor.into(),
            action: action.into(),
            resource_type: resource_type.into(),
            resource_id,
            request_id,
            created_at: Utc::now(),
            detail,
        };
        let conn = self.conn.lock().await;
        conn.execute(
            "INSERT INTO audit_events (id, actor, action, resource_type, resource_id, request_id, created_at, detail)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                event.id.to_string(),
                &event.actor,
                &event.action,
                &event.resource_type,
                &event.resource_id,
                &event.request_id,
                event.created_at.to_rfc3339(),
                event.detail.to_string(),
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(event)
    }

    pub async fn list(&self) -> Result<Vec<AuditEvent>, String> {
        let conn = self.conn.lock().await;
        let mut stmt = conn
            .prepare(
                "SELECT id, actor, action, resource_type, resource_id, request_id, created_at, detail
                 FROM audit_events ORDER BY created_at DESC LIMIT 500",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| Self::row_to_event(row))
            .map_err(|e| e.to_string())?;
        let mut events = Vec::new();
        for row in rows {
            events.push(row.map_err(|e| e.to_string())?);
        }
        Ok(events)
    }

    fn row_to_event(row: &rusqlite::Row) -> rusqlite::Result<AuditEvent> {
        let id_str: String = row.get(0)?;
        let created_at_str: String = row.get(6)?;
        let detail_str: String = row.get(7)?;
        Ok(AuditEvent {
            id: Uuid::parse_str(&id_str).unwrap_or_else(|_| Uuid::new_v4()),
            actor: row.get(1)?,
            action: row.get(2)?,
            resource_type: row.get(3)?,
            resource_id: row.get(4)?,
            request_id: row.get(5)?,
            created_at: chrono::DateTime::parse_from_rfc3339(&created_at_str)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
            detail: serde_json::from_str(&detail_str).unwrap_or_else(|_| serde_json::json!({})),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn audit_store_persists_events() {
        let path = std::env::temp_dir().join(format!("audit-store-{}.db", Uuid::new_v4()));
        let store = AuditStore::new(&path).unwrap();
        store
            .record(
                "admin",
                "alarm.ack",
                "alarm_event",
                Some("a1".to_string()),
                None,
                serde_json::json!({"ok": true}),
            )
            .await
            .unwrap();
        let events = store.list().await.unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].actor, "admin");
        assert_eq!(events[0].action, "alarm.ack");
        let _ = std::fs::remove_file(path);
    }
}
