//! SQLite-backed alarm event store.

use chrono::{DateTime, Utc};
use gateway_flow::operators::alarm::AlarmEventDraft;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AlarmSeverity {
    Low,
    Medium,
    High,
    Critical,
    Info,
}

impl AlarmSeverity {
    pub fn as_str(self) -> &'static str {
        match self {
            AlarmSeverity::Low => "low",
            AlarmSeverity::Medium => "medium",
            AlarmSeverity::High => "high",
            AlarmSeverity::Critical => "critical",
            AlarmSeverity::Info => "info",
        }
    }

    fn from_str(value: &str) -> Self {
        match value.to_ascii_lowercase().as_str() {
            "critical" => AlarmSeverity::Critical,
            "high" | "major" => AlarmSeverity::High,
            "medium" | "minor" => AlarmSeverity::Medium,
            "low" => AlarmSeverity::Low,
            _ => AlarmSeverity::Info,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AlarmStatus {
    Active,
    Acknowledged,
    Resolved,
}

impl AlarmStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            AlarmStatus::Active => "active",
            AlarmStatus::Acknowledged => "acknowledged",
            AlarmStatus::Resolved => "resolved",
        }
    }

    fn from_str(value: &str) -> Self {
        match value.to_ascii_lowercase().as_str() {
            "acknowledged" => AlarmStatus::Acknowledged,
            "resolved" => AlarmStatus::Resolved,
            _ => AlarmStatus::Active,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlarmEvent {
    pub id: Uuid,
    pub source_type: String,
    pub source_id: Option<String>,
    pub node_id: String,
    pub tag: String,
    pub severity: AlarmSeverity,
    /// Compatibility alias for existing frontend filters.
    pub level: String,
    pub status: AlarmStatus,
    pub message: String,
    pub value: Option<f64>,
    pub threshold: Option<String>,
    pub rule_id: Option<String>,
    pub created_at: DateTime<Utc>,
    /// Compatibility alias for existing frontend timestamp display.
    pub timestamp: DateTime<Utc>,
    pub acknowledged_at: Option<DateTime<Utc>>,
    pub resolved_at: Option<DateTime<Utc>>,
}

#[derive(Clone)]
pub struct AlarmStore {
    conn: Arc<Mutex<Connection>>,
    pub db_path: PathBuf,
}

impl AlarmStore {
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
            .expect("AlarmStore.init(): mutex should be available immediately");
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS alarm_events (
                id TEXT PRIMARY KEY,
                source_type TEXT NOT NULL,
                source_id TEXT,
                node_id TEXT NOT NULL,
                tag TEXT NOT NULL,
                severity TEXT NOT NULL,
                status TEXT NOT NULL,
                message TEXT NOT NULL,
                value REAL,
                threshold TEXT,
                rule_id TEXT,
                created_at TEXT NOT NULL,
                acknowledged_at TEXT,
                resolved_at TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_alarm_events_status ON alarm_events(status);
            CREATE INDEX IF NOT EXISTS idx_alarm_events_severity ON alarm_events(severity);
            CREATE INDEX IF NOT EXISTS idx_alarm_events_created_at ON alarm_events(created_at);",
        )?;
        Ok(())
    }

    pub async fn create_from_draft(&self, draft: AlarmEventDraft) -> Result<AlarmEvent, String> {
        let event = AlarmEvent {
            id: Uuid::new_v4(),
            source_type: draft.source_type,
            source_id: draft.source_id,
            node_id: draft.node_id,
            tag: draft.tag,
            severity: AlarmSeverity::from_str(&format!("{:?}", draft.severity)),
            level: AlarmSeverity::from_str(&format!("{:?}", draft.severity))
                .as_str()
                .to_string(),
            status: AlarmStatus::Active,
            message: draft.message,
            value: Some(draft.value),
            threshold: Some(draft.threshold),
            rule_id: Some(draft.rule_id),
            created_at: draft.created_at,
            timestamp: draft.created_at,
            acknowledged_at: None,
            resolved_at: None,
        };
        self.insert(&event).await?;
        Ok(event)
    }

    pub async fn insert(&self, event: &AlarmEvent) -> Result<(), String> {
        let conn = self.conn.lock().await;
        conn.execute(
            "INSERT INTO alarm_events (
                id, source_type, source_id, node_id, tag, severity, status, message,
                value, threshold, rule_id, created_at, acknowledged_at, resolved_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                event.id.to_string(),
                &event.source_type,
                &event.source_id,
                &event.node_id,
                &event.tag,
                event.severity.as_str(),
                event.status.as_str(),
                &event.message,
                event.value,
                &event.threshold,
                &event.rule_id,
                event.created_at.to_rfc3339(),
                event.acknowledged_at.map(|ts| ts.to_rfc3339()),
                event.resolved_at.map(|ts| ts.to_rfc3339()),
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn list(&self) -> Result<Vec<AlarmEvent>, String> {
        let conn = self.conn.lock().await;
        let mut stmt = conn
            .prepare(
                "SELECT id, source_type, source_id, node_id, tag, severity, status, message,
                        value, threshold, rule_id, created_at, acknowledged_at, resolved_at
                 FROM alarm_events
                 ORDER BY created_at DESC
                 LIMIT 500",
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

    pub async fn acknowledge(&self, id: Uuid) -> Result<AlarmEvent, String> {
        let now = Utc::now();
        {
            let conn = self.conn.lock().await;
            let changed = conn
                .execute(
                    "UPDATE alarm_events SET status=?2, acknowledged_at=?3 WHERE id=?1",
                    params![
                        id.to_string(),
                        AlarmStatus::Acknowledged.as_str(),
                        now.to_rfc3339()
                    ],
                )
                .map_err(|e| e.to_string())?;
            if changed == 0 {
                return Err("alarm event not found".to_string());
            }
        }
        self.get(id)
            .await?
            .ok_or_else(|| "alarm event not found".to_string())
    }

    pub async fn resolve(&self, id: Uuid) -> Result<AlarmEvent, String> {
        let now = Utc::now();
        {
            let conn = self.conn.lock().await;
            let changed = conn
                .execute(
                    "UPDATE alarm_events SET status=?2, resolved_at=?3 WHERE id=?1",
                    params![
                        id.to_string(),
                        AlarmStatus::Resolved.as_str(),
                        now.to_rfc3339()
                    ],
                )
                .map_err(|e| e.to_string())?;
            if changed == 0 {
                return Err("alarm event not found".to_string());
            }
        }
        self.get(id)
            .await?
            .ok_or_else(|| "alarm event not found".to_string())
    }

    async fn get(&self, id: Uuid) -> Result<Option<AlarmEvent>, String> {
        let conn = self.conn.lock().await;
        let mut stmt = conn
            .prepare(
                "SELECT id, source_type, source_id, node_id, tag, severity, status, message,
                        value, threshold, rule_id, created_at, acknowledged_at, resolved_at
                 FROM alarm_events WHERE id=?1",
            )
            .map_err(|e| e.to_string())?;
        match stmt.query_row(params![id.to_string()], |row| Self::row_to_event(row)) {
            Ok(event) => Ok(Some(event)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }

    fn row_to_event(row: &rusqlite::Row) -> rusqlite::Result<AlarmEvent> {
        let id_str: String = row.get(0)?;
        let severity_str: String = row.get(5)?;
        let status_str: String = row.get(6)?;
        let created_at_str: String = row.get(11)?;
        let acknowledged_at_str: Option<String> = row.get(12)?;
        let resolved_at_str: Option<String> = row.get(13)?;
        let severity = AlarmSeverity::from_str(&severity_str);
        let created_at = parse_time(&created_at_str);
        Ok(AlarmEvent {
            id: Uuid::parse_str(&id_str).unwrap_or_else(|_| Uuid::new_v4()),
            source_type: row.get(1)?,
            source_id: row.get(2)?,
            node_id: row.get(3)?,
            tag: row.get(4)?,
            severity,
            level: severity.as_str().to_string(),
            status: AlarmStatus::from_str(&status_str),
            message: row.get(7)?,
            value: row.get(8)?,
            threshold: row.get(9)?,
            rule_id: row.get(10)?,
            created_at,
            timestamp: created_at,
            acknowledged_at: acknowledged_at_str.as_deref().map(parse_time),
            resolved_at: resolved_at_str.as_deref().map(parse_time),
        })
    }
}

fn parse_time(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn alarm_store_persists_and_lists_events() {
        let dir = std::env::temp_dir().join(format!("alarm-store-{}.db", Uuid::new_v4()));
        let store = AlarmStore::new(&dir).unwrap();
        let event = AlarmEvent {
            id: Uuid::new_v4(),
            source_type: "flow".to_string(),
            source_id: None,
            node_id: "node-1".to_string(),
            tag: "temperature".to_string(),
            severity: AlarmSeverity::High,
            level: "high".to_string(),
            status: AlarmStatus::Active,
            message: "temperature too high".to_string(),
            value: Some(92.5),
            threshold: Some("92.5 > 80".to_string()),
            rule_id: Some("temp_high".to_string()),
            created_at: Utc::now(),
            timestamp: Utc::now(),
            acknowledged_at: None,
            resolved_at: None,
        };

        store.insert(&event).await.unwrap();
        let events = store.list().await.unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].status, AlarmStatus::Active);
        assert_eq!(events[0].severity, AlarmSeverity::High);

        let acked = store.acknowledge(event.id).await.unwrap();
        assert_eq!(acked.status, AlarmStatus::Acknowledged);
        assert!(acked.acknowledged_at.is_some());

        let resolved = store.resolve(event.id).await.unwrap();
        assert_eq!(resolved.status, AlarmStatus::Resolved);
        assert!(resolved.resolved_at.is_some());

        let _ = std::fs::remove_file(dir);
    }
}
