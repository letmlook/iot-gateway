//! SQLite store for flows and flow_nodes.
//!
//! Uses `Arc<tokio::sync::Mutex<Connection>>` to allow thread-safe access from async handlers.

use chrono::{DateTime, Utc};
use gateway_flow::{Flow, FlowBinding, FlowEdge, FlowFailurePolicy, FlowNode, FlowStatus};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::Mutex;
use uuid::Uuid;

/// Snapshot of a flow version for history tracking.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowSnapshot {
    pub version: i64,
    pub name: String,
    pub description: Option<String>,
    pub status: String,
    pub nodes: Vec<FlowNode>,
    pub edges: Vec<FlowEdge>,
    pub saved_at: String,
}

/// Thread-safe wrapper around SQLite connection for Flow persistence.
#[derive(Clone)]
pub struct FlowStore {
    conn: Arc<Mutex<Connection>>,
    pub db_path: std::path::PathBuf,
}

impl FlowStore {
    pub fn new(db_path: &std::path::Path) -> Result<Self, rusqlite::Error> {
        let conn = Connection::open(db_path)?;
        let store = Self {
            conn: Arc::new(Mutex::new(conn)),
            db_path: db_path.to_path_buf(),
        };
        store.init()?;
        Ok(store)
    }

    pub fn init(&self) -> Result<(), rusqlite::Error> {
        // Use try_lock since we're in a sync context
        // This should succeed immediately as no other thread has accessed the mutex yet
        let conn = self
            .conn
            .try_lock()
            .expect("FlowStore.init(): mutex should be available immediately");
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS flows (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                description TEXT,
                status TEXT NOT NULL DEFAULT 'draft',
                version INTEGER NOT NULL DEFAULT 1,
                version_history TEXT NOT NULL DEFAULT '[]',
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            
            CREATE TABLE IF NOT EXISTS flow_nodes (
                id TEXT PRIMARY KEY,
                flow_id TEXT NOT NULL,
                name TEXT NOT NULL,
                kind TEXT NOT NULL,
                operator_name TEXT,
                config TEXT NOT NULL DEFAULT '{}',
                input_ports TEXT NOT NULL DEFAULT '[]',
                output_ports TEXT NOT NULL DEFAULT '[]',
                position_x REAL DEFAULT 0,
                position_y REAL DEFAULT 0,
                FOREIGN KEY (flow_id) REFERENCES flows(id) ON DELETE CASCADE
            );
            
            CREATE TABLE IF NOT EXISTS flow_edges (
                id TEXT PRIMARY KEY,
                flow_id TEXT NOT NULL,
                source_node_id TEXT NOT NULL,
                source_port TEXT NOT NULL,
                target_node_id TEXT NOT NULL,
                target_port TEXT NOT NULL,
                FOREIGN KEY (flow_id) REFERENCES flows(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS flow_bindings (
                flow_id TEXT NOT NULL,
                south_node_id TEXT NOT NULL,
                group_id TEXT NOT NULL,
                enabled INTEGER NOT NULL DEFAULT 1,
                failure_policy TEXT NOT NULL DEFAULT 'fail_open',
                PRIMARY KEY (flow_id, south_node_id, group_id),
                FOREIGN KEY (flow_id) REFERENCES flows(id) ON DELETE CASCADE
            );

            CREATE INDEX IF NOT EXISTS idx_flow_nodes_flow_id ON flow_nodes(flow_id);
            CREATE INDEX IF NOT EXISTS idx_flow_edges_flow_id ON flow_edges(flow_id);
            CREATE INDEX IF NOT EXISTS idx_flow_bindings_flow_id ON flow_bindings(flow_id);
            CREATE INDEX IF NOT EXISTS idx_flow_bindings_source ON flow_bindings(south_node_id, group_id);
            ",
        )?;

        // Idempotent migration: add position columns if missing (pre-0ea5d11 DBs).
        Self::migrate_position_columns(&conn)?;

        Ok(())
    }

    /// Add `position_x` / `position_y` to `flow_nodes` if they don't exist yet.
    fn migrate_position_columns(conn: &Connection) -> Result<(), rusqlite::Error> {
        let mut stmt = conn.prepare("PRAGMA table_info(flow_nodes)")?;
        let existing: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<Result<_, _>>()?;

        if !existing.iter().any(|c| c == "position_x") {
            conn.execute("ALTER TABLE flow_nodes ADD COLUMN position_x REAL DEFAULT 0", [])?;
        }
        if !existing.iter().any(|c| c == "position_y") {
            conn.execute("ALTER TABLE flow_nodes ADD COLUMN position_y REAL DEFAULT 0", [])?;
        }
        Ok(())
    }

    // ---------- Flow CRUD ----------

    pub async fn create_flow(&self, flow: &Flow) -> Result<(), FlowStoreError> {
        let conn = self.conn.lock().await;
        conn.execute(
            "INSERT INTO flows (id, name, description, status, version, version_history, created_at, updated_at) 
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                flow.id.to_string(),
                &flow.name,
                &flow.description,
                serde_json::to_string(&flow.status).unwrap_or_default(),
                flow.version,
                "[]",
                flow.created_at.to_rfc3339(),
                flow.updated_at.to_rfc3339(),
            ],
        )
        .map_err(FlowStoreError::Rusqlite)?;

        for node in &flow.nodes {
            Self::upsert_node_sync(&conn, flow.id, node)?;
        }
        for edge in &flow.edges {
            Self::upsert_edge_sync(&conn, flow.id, edge)?;
        }
        Self::replace_bindings_sync(&conn, flow.id, &flow.bindings)?;
        Ok(())
    }

    pub async fn get_flow(&self, id: Uuid) -> Result<Option<Flow>, FlowStoreError> {
        let conn = self.conn.lock().await;
        let mut stmt = conn
            .prepare(
                "SELECT id, name, description, status, version, version_history, created_at, updated_at FROM flows WHERE id = ?1",
            )
            .map_err(FlowStoreError::Rusqlite)?;

        let mut rows = stmt
            .query(params![id.to_string()])
            .map_err(FlowStoreError::Rusqlite)?;
        if let Some(row) = rows.next().map_err(FlowStoreError::Rusqlite)? {
            let flow = self.row_to_flow_sync(&conn, row)?;
            Ok(Some(flow))
        } else {
            Ok(None)
        }
    }

    pub async fn list_flows(&self) -> Result<Vec<Flow>, FlowStoreError> {
        let conn = self.conn.lock().await;
        let mut stmt = conn
            .prepare(
                "SELECT id, name, description, status, version, version_history, created_at, updated_at FROM flows ORDER BY updated_at DESC",
            )
            .map_err(FlowStoreError::Rusqlite)?;

        let mut flows = Vec::new();
        let mut rows = stmt.query([]).map_err(FlowStoreError::Rusqlite)?;
        while let Some(row) = rows.next().map_err(FlowStoreError::Rusqlite)? {
            flows.push(self.row_to_flow_sync(&conn, row)?);
        }
        Ok(flows)
    }

    pub async fn flow_count(&self) -> usize {
        let conn = self.conn.lock().await;
        conn.query_row("SELECT COUNT(*) FROM flows", [], |row| row.get::<_, i64>(0))
            .map(|c| c as usize)
            .unwrap_or(0)
    }

    pub async fn update_flow(&self, flow: &Flow) -> Result<(), FlowStoreError> {
        // Save snapshot before updating (ignore errors if flow doesn't exist yet)
        let _ = self.save_flow_snapshot(&flow.id).await;

        let conn = self.conn.lock().await;
        conn.execute(
            "UPDATE flows SET name=?2, description=?3, status=?4, version=?5, updated_at=?6 WHERE id=?1",
            params![
                flow.id.to_string(),
                &flow.name,
                &flow.description,
                serde_json::to_string(&flow.status).unwrap_or_default(),
                flow.version,
                Utc::now().to_rfc3339(),
            ],
        )
        .map_err(FlowStoreError::Rusqlite)?;

        // Delete and re-insert nodes and edges
        conn.execute(
            "DELETE FROM flow_nodes WHERE flow_id=?1",
            params![flow.id.to_string()],
        )
        .map_err(FlowStoreError::Rusqlite)?;
        conn.execute(
            "DELETE FROM flow_edges WHERE flow_id=?1",
            params![flow.id.to_string()],
        )
        .map_err(FlowStoreError::Rusqlite)?;

        for node in &flow.nodes {
            Self::upsert_node_sync(&conn, flow.id, node)?;
        }
        for edge in &flow.edges {
            Self::upsert_edge_sync(&conn, flow.id, edge)?;
        }
        Self::replace_bindings_sync(&conn, flow.id, &flow.bindings)?;
        Ok(())
    }

    pub async fn delete_flow(&self, id: Uuid) -> Result<(), FlowStoreError> {
        let conn = self.conn.lock().await;
        conn.execute("DELETE FROM flows WHERE id=?1", params![id.to_string()])
            .map_err(FlowStoreError::Rusqlite)?;
        Ok(())
    }

    pub async fn get_flow_by_name(&self, name: &str) -> Result<Option<Flow>, FlowStoreError> {
        let conn = self.conn.lock().await;
        let mut stmt = conn
            .prepare(
                "SELECT id, name, description, status, version, version_history, created_at, updated_at FROM flows WHERE name = ?1 LIMIT 1",
            )
            .map_err(FlowStoreError::Rusqlite)?;

        let mut rows = stmt
            .query(params![name])
            .map_err(FlowStoreError::Rusqlite)?;
        if let Some(row) = rows.next().map_err(FlowStoreError::Rusqlite)? {
            let flow = self.row_to_flow_sync(&conn, row)?;
            Ok(Some(flow))
        } else {
            Ok(None)
        }
    }

    pub async fn list_bindings(&self, flow_id: Uuid) -> Result<Vec<FlowBinding>, FlowStoreError> {
        let conn = self.conn.lock().await;
        self.load_bindings_sync(&conn, flow_id)
    }

    pub async fn replace_bindings(
        &self,
        flow_id: Uuid,
        bindings: &[FlowBinding],
    ) -> Result<(), FlowStoreError> {
        let conn = self.conn.lock().await;
        Self::replace_bindings_sync(&conn, flow_id, bindings)
    }

    pub async fn enabled_binding_for_source(
        &self,
        south_node_id: gateway_sdk::NodeId,
        group_id: gateway_sdk::GroupId,
    ) -> Result<Option<FlowBinding>, FlowStoreError> {
        let conn = self.conn.lock().await;
        let mut stmt = conn
            .prepare(
                "SELECT flow_id, south_node_id, group_id, enabled, failure_policy FROM flow_bindings
                 WHERE south_node_id=?1 AND group_id=?2 AND enabled=1 LIMIT 1",
            )
            .map_err(FlowStoreError::Rusqlite)?;
        let mut rows = stmt
            .query(params![south_node_id.0.to_string(), group_id.0.to_string()])
            .map_err(FlowStoreError::Rusqlite)?;
        if let Some(row) = rows.next().map_err(FlowStoreError::Rusqlite)? {
            Ok(Some(Self::row_to_binding(row)?))
        } else {
            Ok(None)
        }
    }

    pub async fn update_flow_status(
        &self,
        id: Uuid,
        status: FlowStatus,
    ) -> Result<(), FlowStoreError> {
        let conn = self.conn.lock().await;
        conn.execute(
            "UPDATE flows SET status=?2, updated_at=?3 WHERE id=?1",
            params![
                id.to_string(),
                serde_json::to_string(&status).unwrap_or_default(),
                Utc::now().to_rfc3339()
            ],
        )
        .map_err(FlowStoreError::Rusqlite)?;
        Ok(())
    }

    /// Save a version snapshot of the flow before major updates.
    pub async fn save_flow_snapshot(&self, id: &Uuid) -> Result<(), FlowStoreError> {
        let flow = self
            .get_flow(*id)
            .await?
            .ok_or_else(|| FlowStoreError::Rusqlite(rusqlite::Error::QueryReturnedNoRows))?;

        let snapshot = FlowSnapshot {
            version: flow.version,
            name: flow.name.clone(),
            description: flow.description.clone(),
            status: serde_json::to_string(&flow.status).unwrap_or_default(),
            nodes: flow.nodes.clone(),
            edges: flow.edges.clone(),
            saved_at: Utc::now().to_rfc3339(),
        };

        let mut history: Vec<FlowSnapshot> = self.get_flow_version_history_raw(id).await?;

        // Keep only last 10 snapshots
        if history.len() >= 10 {
            history.remove(0);
        }
        history.push(snapshot);

        let history_json = serde_json::to_string(&history).map_err(FlowStoreError::Json)?;

        let conn = self.conn.lock().await;
        conn.execute(
            "UPDATE flows SET version_history=?1 WHERE id=?2",
            params![history_json, id.to_string()],
        )
        .map_err(FlowStoreError::Rusqlite)?;
        Ok(())
    }

    /// Get version history for a flow (deserialized).
    pub async fn get_flow_version_history(
        &self,
        id: &Uuid,
    ) -> Result<Vec<FlowSnapshot>, FlowStoreError> {
        self.get_flow_version_history_raw(id).await
    }

    /// Internal: get version history raw JSON and parse.
    async fn get_flow_version_history_raw(
        &self,
        id: &Uuid,
    ) -> Result<Vec<FlowSnapshot>, FlowStoreError> {
        let conn = self.conn.lock().await;
        let mut stmt = conn
            .prepare("SELECT version_history FROM flows WHERE id=?1")
            .map_err(FlowStoreError::Rusqlite)?;

        let history_str: String = stmt
            .query_row(params![id.to_string()], |row| row.get(0))
            .map_err(FlowStoreError::Rusqlite)?;

        let history: Vec<FlowSnapshot> = serde_json::from_str(&history_str).unwrap_or_default();
        Ok(history)
    }

    /// Restore a flow from a snapshot definition.
    pub async fn restore_from_snapshot(
        &self,
        id: &Uuid,
        snapshot: &FlowSnapshot,
    ) -> Result<Flow, FlowStoreError> {
        let conn = self.conn.lock().await;

        // Update flow metadata
        conn.execute(
            "UPDATE flows SET name=?2, description=?3, status=?4, version=?5, updated_at=?6 WHERE id=?1",
            params![
                id.to_string(),
                &snapshot.name,
                &snapshot.description,
                &snapshot.status,
                snapshot.version,
                Utc::now().to_rfc3339(),
            ],
        )
        .map_err(FlowStoreError::Rusqlite)?;

        // Delete existing nodes and edges
        conn.execute(
            "DELETE FROM flow_nodes WHERE flow_id=?1",
            params![id.to_string()],
        )
        .map_err(FlowStoreError::Rusqlite)?;
        conn.execute(
            "DELETE FROM flow_edges WHERE flow_id=?1",
            params![id.to_string()],
        )
        .map_err(FlowStoreError::Rusqlite)?;

        // Re-insert nodes and edges from snapshot
        for node in &snapshot.nodes {
            Self::upsert_node_sync(&conn, *id, node)?;
        }
        for edge in &snapshot.edges {
            Self::upsert_edge_sync(&conn, *id, edge)?;
        }

        drop(conn);
        self.get_flow(*id)
            .await?
            .ok_or_else(|| FlowStoreError::Rusqlite(rusqlite::Error::QueryReturnedNoRows))
    }

    fn replace_bindings_sync(
        conn: &Connection,
        flow_id: Uuid,
        bindings: &[FlowBinding],
    ) -> Result<(), FlowStoreError> {
        conn.execute(
            "DELETE FROM flow_bindings WHERE flow_id=?1",
            params![flow_id.to_string()],
        )
        .map_err(FlowStoreError::Rusqlite)?;

        for binding in bindings {
            conn.execute(
                "INSERT INTO flow_bindings (flow_id, south_node_id, group_id, enabled, failure_policy)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    binding.flow_id.to_string(),
                    binding.south_node_id.0.to_string(),
                    binding.group_id.0.to_string(),
                    if binding.enabled { 1_i64 } else { 0_i64 },
                    serde_json::to_string(&binding.failure_policy)
                        .unwrap_or_else(|_| "\"fail_open\"".to_string())
                        .trim_matches('"')
                        .to_string(),
                ],
            )
            .map_err(FlowStoreError::Rusqlite)?;
        }
        Ok(())
    }

    fn row_to_binding(row: &rusqlite::Row) -> Result<FlowBinding, FlowStoreError> {
        let flow_id_str: String = row.get(0).map_err(FlowStoreError::Rusqlite)?;
        let south_node_id_str: String = row.get(1).map_err(FlowStoreError::Rusqlite)?;
        let group_id_str: String = row.get(2).map_err(FlowStoreError::Rusqlite)?;
        let enabled: i64 = row.get(3).map_err(FlowStoreError::Rusqlite)?;
        let failure_policy_str: String = row.get(4).map_err(FlowStoreError::Rusqlite)?;
        let failure_policy =
            serde_json::from_str::<FlowFailurePolicy>(&format!("\"{}\"", failure_policy_str))
                .unwrap_or_default();

        Ok(FlowBinding {
            flow_id: Uuid::parse_str(&flow_id_str).unwrap_or_else(|_| Uuid::new_v4()),
            south_node_id: gateway_sdk::NodeId(
                Uuid::parse_str(&south_node_id_str).unwrap_or_else(|_| Uuid::new_v4()),
            ),
            group_id: gateway_sdk::GroupId(
                Uuid::parse_str(&group_id_str).unwrap_or_else(|_| Uuid::new_v4()),
            ),
            enabled: enabled != 0,
            failure_policy,
        })
    }

    fn load_bindings_sync(
        &self,
        conn: &Connection,
        flow_id: Uuid,
    ) -> Result<Vec<FlowBinding>, FlowStoreError> {
        let mut stmt = conn
            .prepare(
                "SELECT flow_id, south_node_id, group_id, enabled, failure_policy FROM flow_bindings WHERE flow_id=?1",
            )
            .map_err(FlowStoreError::Rusqlite)?;
        let mut rows = stmt
            .query(params![flow_id.to_string()])
            .map_err(FlowStoreError::Rusqlite)?;
        let mut bindings = Vec::new();
        while let Some(row) = rows.next().map_err(FlowStoreError::Rusqlite)? {
            bindings.push(Self::row_to_binding(row)?);
        }
        Ok(bindings)
    }

    fn upsert_node_sync(
        conn: &Connection,
        flow_id: Uuid,
        node: &FlowNode,
    ) -> Result<(), FlowStoreError> {
        let position_x = node.position.as_ref().map(|p| p.x).unwrap_or(0.0);
        let position_y = node.position.as_ref().map(|p| p.y).unwrap_or(0.0);
        conn.execute(
            "INSERT INTO flow_nodes (id, flow_id, name, kind, operator_name, config, input_ports, output_ports, position_x, position_y)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(id) DO UPDATE SET
                name=?3, kind=?4, operator_name=?5, config=?6, input_ports=?7, output_ports=?8, position_x=?9, position_y=?10",
            params![
                node.id.to_string(),
                flow_id.to_string(),
                &node.name,
                serde_json::to_string(&node.kind).unwrap_or_default(),
                &node.operator_name,
                serde_json::to_string(&node.config).unwrap_or_default(),
                serde_json::to_string(&node.input_ports).unwrap_or_default(),
                serde_json::to_string(&node.output_ports).unwrap_or_default(),
                position_x,
                position_y,
            ],
        )
        .map_err(FlowStoreError::Rusqlite)?;
        Ok(())
    }

    fn upsert_edge_sync(
        conn: &Connection,
        flow_id: Uuid,
        edge: &FlowEdge,
    ) -> Result<(), FlowStoreError> {
        conn.execute(
            "INSERT INTO flow_edges (id, flow_id, source_node_id, source_port, target_node_id, target_port)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(id) DO UPDATE SET
                source_node_id=?3, source_port=?4, target_node_id=?5, target_port=?6",
            params![
                Uuid::new_v4().to_string(),
                flow_id.to_string(),
                edge.source_node_id.to_string(),
                &edge.source_port,
                edge.target_node_id.to_string(),
                &edge.target_port,
            ],
        )
        .map_err(FlowStoreError::Rusqlite)?;
        Ok(())
    }

    fn row_to_flow_sync(
        &self,
        conn: &Connection,
        row: &rusqlite::Row,
    ) -> Result<Flow, FlowStoreError> {
        let id_str: String = row.get(0).map_err(FlowStoreError::Rusqlite)?;
        let id = Uuid::parse_str(&id_str).unwrap_or_else(|_| Uuid::new_v4());
        let name: String = row.get(1).map_err(FlowStoreError::Rusqlite)?;
        let description: Option<String> = row.get(2).map_err(FlowStoreError::Rusqlite)?;
        let status_str: String = row.get(3).map_err(FlowStoreError::Rusqlite)?;
        let version: i64 = row.get(4).map_err(FlowStoreError::Rusqlite)?;
        // version_history at index 5 - read but not used in Flow struct
        let _version_history: String = row.get(5).map_err(FlowStoreError::Rusqlite)?;
        let created_at_str: String = row.get(6).map_err(FlowStoreError::Rusqlite)?;
        let updated_at_str: String = row.get(7).map_err(FlowStoreError::Rusqlite)?;

        let status: FlowStatus = serde_json::from_str(&status_str).unwrap_or(FlowStatus::Draft);
        let created_at = DateTime::parse_from_rfc3339(&created_at_str)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now());
        let updated_at = DateTime::parse_from_rfc3339(&updated_at_str)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now());

        // Load nodes and edges
        let nodes = self.load_nodes_sync(conn, id)?;
        let edges = self.load_edges_sync(conn, id)?;
        let bindings = self.load_bindings_sync(conn, id)?;

        Ok(Flow {
            id,
            name,
            description,
            status,
            nodes,
            edges,
            bindings,
            version,
            created_at,
            updated_at,
        })
    }

    fn load_nodes_sync(
        &self,
        conn: &Connection,
        flow_id: Uuid,
    ) -> Result<Vec<FlowNode>, FlowStoreError> {
        let mut stmt = conn
            .prepare(
                "SELECT id, name, kind, operator_name, config, input_ports, output_ports, position_x, position_y FROM flow_nodes WHERE flow_id=?1",
            )
            .map_err(FlowStoreError::Rusqlite)?;
        let mut nodes = Vec::new();
        let mut rows = stmt
            .query(params![flow_id.to_string()])
            .map_err(FlowStoreError::Rusqlite)?;
        while let Some(row) = rows.next().map_err(FlowStoreError::Rusqlite)? {
            let id_str: String = row.get(0).map_err(FlowStoreError::Rusqlite)?;
            let name: String = row.get(1).map_err(FlowStoreError::Rusqlite)?;
            let kind_str: String = row.get(2).map_err(FlowStoreError::Rusqlite)?;
            let operator_name: Option<String> = row.get(3).map_err(FlowStoreError::Rusqlite)?;
            let config_str: String = row.get(4).map_err(FlowStoreError::Rusqlite)?;
            let input_ports_str: String = row.get(5).map_err(FlowStoreError::Rusqlite)?;
            let output_ports_str: String = row.get(6).map_err(FlowStoreError::Rusqlite)?;
            let position_x: f64 = row.get(7).map_err(FlowStoreError::Rusqlite)?;
            let position_y: f64 = row.get(8).map_err(FlowStoreError::Rusqlite)?;

            let id = Uuid::parse_str(&id_str).unwrap_or_else(|_| Uuid::new_v4());
            let kind: gateway_flow::NodeKind =
                serde_json::from_str(&kind_str).unwrap_or(gateway_flow::NodeKind::South);
            let config: gateway_sdk::PluginConfig =
                serde_json::from_str(&config_str).unwrap_or_default();
            let input_ports: Vec<gateway_flow::Port> =
                serde_json::from_str(&input_ports_str).unwrap_or_default();
            let output_ports: Vec<gateway_flow::Port> =
                serde_json::from_str(&output_ports_str).unwrap_or_default();

            nodes.push(FlowNode {
                id,
                name,
                kind,
                operator_name,
                config,
                input_ports,
                output_ports,
                position: Some(gateway_flow::node::NodePosition {
                    x: position_x,
                    y: position_y,
                }),
            });
        }
        Ok(nodes)
    }

    fn load_edges_sync(
        &self,
        conn: &Connection,
        flow_id: Uuid,
    ) -> Result<Vec<FlowEdge>, FlowStoreError> {
        let mut stmt = conn
            .prepare(
                "SELECT id, source_node_id, source_port, target_node_id, target_port FROM flow_edges WHERE flow_id=?1",
            )
            .map_err(FlowStoreError::Rusqlite)?;
        let mut edges = Vec::new();
        let mut rows = stmt
            .query(params![flow_id.to_string()])
            .map_err(FlowStoreError::Rusqlite)?;
        while let Some(row) = rows.next().map_err(FlowStoreError::Rusqlite)? {
            let edge_id_str: String = row.get(0).map_err(FlowStoreError::Rusqlite)?;
            let source_id_str: String = row.get(1).map_err(FlowStoreError::Rusqlite)?;
            let source_port: String = row.get(2).map_err(FlowStoreError::Rusqlite)?;
            let target_id_str: String = row.get(3).map_err(FlowStoreError::Rusqlite)?;
            let target_port: String = row.get(4).map_err(FlowStoreError::Rusqlite)?;

            edges.push(FlowEdge {
                source_node_id: Uuid::parse_str(&source_id_str).unwrap_or_else(|_| Uuid::new_v4()),
                source_port,
                target_node_id: Uuid::parse_str(&target_id_str).unwrap_or_else(|_| Uuid::new_v4()),
                target_port,
            });
        }
        Ok(edges)
    }
}

/// Errors from FlowStore operations.
#[derive(Debug)]
pub enum FlowStoreError {
    Rusqlite(rusqlite::Error),
    Json(serde_json::Error),
}

impl std::fmt::Display for FlowStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FlowStoreError::Rusqlite(e) => write!(f, "FlowStore: {}", e),
            FlowStoreError::Json(e) => write!(f, "FlowStore: {}", e),
        }
    }
}

impl std::error::Error for FlowStoreError {}

impl From<rusqlite::Error> for FlowStoreError {
    fn from(e: rusqlite::Error) -> Self {
        FlowStoreError::Rusqlite(e)
    }
}

impl From<serde_json::Error> for FlowStoreError {
    fn from(e: serde_json::Error) -> Self {
        FlowStoreError::Json(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gateway_flow::{node::NodePosition, Flow, FlowNode, FlowStatus, NodeKind};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_db_path(name: &str) -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("iot-gateway-{name}-{nanos}.db"))
    }

    #[test]
    fn flow_store_adds_position_columns_to_existing_nodes_table() {
        let db_path = test_db_path("flow-position-migration");
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE flows (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                description TEXT,
                status TEXT NOT NULL DEFAULT 'draft',
                version INTEGER NOT NULL DEFAULT 1,
                version_history TEXT NOT NULL DEFAULT '[]',
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE flow_nodes (
                id TEXT PRIMARY KEY,
                flow_id TEXT NOT NULL,
                name TEXT NOT NULL,
                kind TEXT NOT NULL,
                operator_name TEXT,
                config TEXT NOT NULL DEFAULT '{}',
                input_ports TEXT NOT NULL DEFAULT '[]',
                output_ports TEXT NOT NULL DEFAULT '[]',
                FOREIGN KEY (flow_id) REFERENCES flows(id) ON DELETE CASCADE
            );

            CREATE TABLE flow_edges (
                id TEXT PRIMARY KEY,
                flow_id TEXT NOT NULL,
                source_node_id TEXT NOT NULL,
                source_port TEXT NOT NULL,
                target_node_id TEXT NOT NULL,
                target_port TEXT NOT NULL,
                FOREIGN KEY (flow_id) REFERENCES flows(id) ON DELETE CASCADE
            );

            CREATE TABLE flow_bindings (
                flow_id TEXT NOT NULL,
                south_node_id TEXT NOT NULL,
                group_id TEXT NOT NULL,
                enabled INTEGER NOT NULL DEFAULT 1,
                failure_policy TEXT NOT NULL DEFAULT 'fail_open',
                PRIMARY KEY (flow_id, south_node_id, group_id),
                FOREIGN KEY (flow_id) REFERENCES flows(id) ON DELETE CASCADE
            );",
        )
        .unwrap();
        drop(conn);

        let _store = FlowStore::new(&db_path).unwrap();

        let conn = rusqlite::Connection::open(&db_path).unwrap();
        let mut stmt = conn.prepare("PRAGMA table_info(flow_nodes)").unwrap();
        let columns: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert!(columns.iter().any(|column| column == "position_x"));
        assert!(columns.iter().any(|column| column == "position_y"));

        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn flow_store_round_trips_node_position() {
        let db_path = test_db_path("flow-position");
        let store = FlowStore::new(&db_path).unwrap();
        let mut flow = Flow::new("position-flow");
        flow.status = FlowStatus::Draft;
        flow.nodes.push(FlowNode {
            id: Uuid::new_v4(),
            name: "operator".to_string(),
            kind: NodeKind::Operator,
            operator_name: Some("range".to_string()),
            config: gateway_sdk::PluginConfig::new(),
            input_ports: vec![],
            output_ports: vec![],
            position: Some(NodePosition { x: 123.0, y: 456.0 }),
        });

        store.create_flow(&flow).await.unwrap();
        let loaded = store.get_flow(flow.id).await.unwrap().unwrap();
        let position = loaded.nodes[0].position.as_ref().unwrap();
        assert_eq!(position.x, 123.0);
        assert_eq!(position.y, 456.0);

        let _ = std::fs::remove_file(db_path);
    }
}
