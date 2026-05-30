# FlowEditor Real Graph Model Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make FlowEditor save and load a real, reloadable flow graph with persisted positions, real operator config, valid default ports, lenient draft saves, and preserved bindings.

**Architecture:** Extend the shared `gateway-flow` model with optional UI position metadata, wire existing SQLite position columns into `FlowStore`, relax save-time validation only for draft flows, and update FlowEditor serialization/deserialization to use backend-recognized fields. Keep deploy, preview, and runtime validation strict.

**Tech Stack:** Rust (`gateway-flow`, `gateway-server`, rusqlite, serde), Vue 3 + VueFlow + Vite.

---

## File Structure

Modify:

- `gateway/gateway-flow/src/node.rs` — add `NodePosition` and optional `FlowNode.position`.
- `gateway/gateway-flow/src/flow.rs` — add model tests for position compatibility.
- `gateway/gateway-server/src/flow/store.rs` — persist and load node positions; add store round-trip test.
- `gateway/gateway-server/src/flow/handlers.rs` — add draft validation helper and use it for draft create/update.
- `web/src/views/FlowEditor.vue` — load/save positions, map operator UI config to `config`, generate default ports, preserve bindings.

Do not modify:

- runtime DAG execution semantics;
- Flow binding UI;
- VueFlow custom node handle UI;
- `web/src/api.js`.

---

### Task 1: Add Backend Node Position Model

**Files:**
- Modify: `gateway/gateway-flow/src/node.rs`
- Modify: `gateway/gateway-flow/src/flow.rs`

- [ ] **Step 1: Add failing model tests**

In `gateway/gateway-flow/src/flow.rs`, inside the existing `#[cfg(test)] mod tests`, add imports and two tests:

```rust
use crate::node::{FlowNode, NodeKind, NodePosition};
use serde_json::json;

#[test]
fn flow_node_position_round_trips_json() {
    let node = FlowNode {
        id: Uuid::new_v4(),
        name: "range".to_string(),
        kind: NodeKind::Operator,
        operator_name: Some("range".to_string()),
        config: gateway_sdk::PluginConfig::new(),
        input_ports: vec![],
        output_ports: vec![],
        position: Some(NodePosition { x: 320.0, y: 160.0 }),
    };

    let json = serde_json::to_value(&node).unwrap();
    assert_eq!(json["position"]["x"], 320.0);
    assert_eq!(json["position"]["y"], 160.0);

    let decoded: FlowNode = serde_json::from_value(json).unwrap();
    let position = decoded.position.unwrap();
    assert_eq!(position.x, 320.0);
    assert_eq!(position.y, 160.0);
}

#[test]
fn flow_node_without_position_deserializes_for_legacy_json() {
    let json = json!({
        "id": Uuid::new_v4(),
        "name": "legacy",
        "kind": "operator",
        "operator_name": "range",
        "config": {},
        "input_ports": [],
        "output_ports": []
    });

    let decoded: FlowNode = serde_json::from_value(json).unwrap();
    assert!(decoded.position.is_none());
}
```

- [ ] **Step 2: Run the failing tests**

Run:

```bash
cargo test -p gateway-flow flow::tests::flow_node_position_round_trips_json flow::tests::flow_node_without_position_deserializes_for_legacy_json
```

Expected: compile failure because `NodePosition` and `FlowNode.position` do not exist.

- [ ] **Step 3: Implement position model**

In `gateway/gateway-flow/src/node.rs`, after `Port`, add:

```rust
/// UI position for rendering a node in graph editors.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodePosition {
    pub x: f64,
    pub y: f64,
}
```

Extend `FlowNode`:

```rust
    #[serde(default)]
    pub position: Option<NodePosition>,
```

Final struct:

```rust
pub struct FlowNode {
    pub id: Uuid,
    pub name: String,
    pub kind: NodeKind,
    pub operator_name: Option<String>,
    pub config: gateway_sdk::PluginConfig,
    pub input_ports: Vec<Port>,
    pub output_ports: Vec<Port>,
    #[serde(default)]
    pub position: Option<NodePosition>,
}
```

- [ ] **Step 4: Run tests**

Run:

```bash
cargo test -p gateway-flow flow::tests::flow_node_position_round_trips_json flow::tests::flow_node_without_position_deserializes_for_legacy_json
```

Expected: both tests pass.

- [ ] **Step 5: Commit**

```bash
git add gateway/gateway-flow/src/node.rs gateway/gateway-flow/src/flow.rs
git commit -m "feat(flow): add node position metadata" -m "Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 2: Persist FlowNode Positions in FlowStore

**Files:**
- Modify: `gateway/gateway-server/src/flow/store.rs`

- [ ] **Step 1: Add failing store round-trip test**

Append this `#[cfg(test)]` module to `gateway/gateway-server/src/flow/store.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use gateway_flow::{Flow, FlowNode, FlowStatus, NodeKind, NodePosition};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_db_path(name: &str) -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("iot-gateway-{name}-{nanos}.db"))
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
```

- [ ] **Step 2: Run failing test**

Run:

```bash
cargo test -p gateway-server flow::store::tests::flow_store_round_trips_node_position
```

Expected: test fails because store loads position as missing or writes `0.0`.

- [ ] **Step 3: Persist position on save**

In `upsert_node_sync`, before `conn.execute`, add:

```rust
        let position_x = node.position.as_ref().map(|p| p.x).unwrap_or(0.0);
        let position_y = node.position.as_ref().map(|p| p.y).unwrap_or(0.0);
```

Replace params entries:

```rust
                0.0f64,
                0.0f64,
```

with:

```rust
                position_x,
                position_y,
```

- [ ] **Step 4: Load position from DB**

Change the query in `load_nodes_sync` from:

```rust
"SELECT id, name, kind, operator_name, config, input_ports, output_ports FROM flow_nodes WHERE flow_id=?1"
```

to:

```rust
"SELECT id, name, kind, operator_name, config, input_ports, output_ports, position_x, position_y FROM flow_nodes WHERE flow_id=?1"
```

After reading `output_ports_str`, add:

```rust
            let position_x: f64 = row.get(7).map_err(FlowStoreError::Rusqlite)?;
            let position_y: f64 = row.get(8).map_err(FlowStoreError::Rusqlite)?;
```

When constructing `FlowNode`, add:

```rust
                position: Some(gateway_flow::NodePosition { x: position_x, y: position_y }),
```

- [ ] **Step 5: Run tests**

Run:

```bash
cargo test -p gateway-server flow::store::tests::flow_store_round_trips_node_position
cargo test -p gateway-server
```

Expected: both pass.

- [ ] **Step 6: Commit**

```bash
git add gateway/gateway-server/src/flow/store.rs
git commit -m "feat(flow): persist node positions" -m "Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 3: Add Lenient Draft Flow Save Validation

**Files:**
- Modify: `gateway/gateway-server/src/flow/handlers.rs`

- [ ] **Step 1: Add draft validation unit tests**

At the end of `gateway/gateway-server/src/flow/handlers.rs`, add:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use gateway_flow::{Flow, FlowEdge, FlowNode, FlowStatus, NodeKind};

    fn draft_flow_with_one_node() -> Flow {
        let mut flow = Flow::new("draft");
        flow.status = FlowStatus::Draft;
        flow.nodes.push(FlowNode {
            id: Uuid::new_v4(),
            name: "operator".to_string(),
            kind: NodeKind::Operator,
            operator_name: Some("range".to_string()),
            config: gateway_sdk::PluginConfig::new(),
            input_ports: vec![],
            output_ports: vec![],
            position: None,
        });
        flow
    }

    #[test]
    fn draft_validation_allows_incomplete_graph() {
        let flow = draft_flow_with_one_node();
        validate_draft_flow(&flow).unwrap();
    }

    #[test]
    fn draft_validation_rejects_empty_name() {
        let mut flow = draft_flow_with_one_node();
        flow.name = "  ".to_string();
        let err = validate_draft_flow(&flow).unwrap_err();
        assert!(err.contains("flow name cannot be empty"));
    }

    #[test]
    fn draft_validation_rejects_edges_referencing_missing_nodes() {
        let mut flow = draft_flow_with_one_node();
        flow.edges.push(FlowEdge {
            source_node_id: flow.nodes[0].id,
            source_port: "out".to_string(),
            target_node_id: Uuid::new_v4(),
            target_port: "in".to_string(),
        });
        let err = validate_draft_flow(&flow).unwrap_err();
        assert!(err.contains("edge references unknown target node"));
    }
}
```

- [ ] **Step 2: Run failing tests**

Run:

```bash
cargo test -p gateway-server flow::handlers::tests::draft_validation_allows_incomplete_graph flow::handlers::tests::draft_validation_rejects_empty_name flow::handlers::tests::draft_validation_rejects_edges_referencing_missing_nodes
```

Expected: compile failure because `validate_draft_flow` does not exist.

- [ ] **Step 3: Implement draft validation helper**

Near the top of `gateway/gateway-server/src/flow/handlers.rs`, after `impl From<FlowStoreError> for ApiError`, add:

```rust
fn validate_draft_flow(flow: &Flow) -> Result<(), String> {
    if flow.name.trim().is_empty() {
        return Err("flow name cannot be empty".to_string());
    }

    let node_ids: std::collections::HashSet<Uuid> = flow.nodes.iter().map(|node| node.id).collect();
    for edge in &flow.edges {
        if !node_ids.contains(&edge.source_node_id) {
            return Err(format!("edge references unknown source node {}", edge.source_node_id));
        }
        if !node_ids.contains(&edge.target_node_id) {
            return Err(format!("edge references unknown target node {}", edge.target_node_id));
        }
    }

    Ok(())
}

fn validate_flow_for_save(flow: &Flow) -> Result<(), String> {
    if flow.status == gateway_flow::FlowStatus::Draft {
        validate_draft_flow(flow)
    } else {
        flow.validate().map_err(|e| e.to_string())
    }
}
```

- [ ] **Step 4: Use save validation in create/update**

In `create_flow`, before `state.flow_store.create_flow(&flow).await?;`, add:

```rust
    validate_flow_for_save(&flow).map_err(ApiError::bad_request)?;
```

In `update_flow`, replace:

```rust
    flow.validate()
        .map_err(|e| ApiError::bad_request(e.to_string()))?;
```

with:

```rust
    validate_flow_for_save(&flow).map_err(ApiError::bad_request)?;
```

- [ ] **Step 5: Run tests**

Run:

```bash
cargo test -p gateway-server flow::handlers::tests::draft_validation_allows_incomplete_graph flow::handlers::tests::draft_validation_rejects_empty_name flow::handlers::tests::draft_validation_rejects_edges_referencing_missing_nodes
cargo test -p gateway-server
```

Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add gateway/gateway-server/src/flow/handlers.rs
git commit -m "fix(flow): allow incomplete draft saves" -m "Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 4: Save and Load Real Graph Data in FlowEditor

**Files:**
- Modify: `web/src/views/FlowEditor.vue`

- [ ] **Step 1: Verify current frontend mismatches**

Run:

```bash
rg -n "position: \{ x: 100, y: 100 \}|operator_config|input_ports: n\.data\.kind === 'operator'|bindings:" web/src/views/FlowEditor.vue
```

Expected output includes fixed load position, `operator_config`, operator-only ports, and no saved `bindings` in the save body.

- [ ] **Step 2: Add port helper functions**

After `addRecentPreviewMessage`, add:

```js
function dataPort(id, name, required) {
  return { id, name, port_type: 'data', required }
}

function buildInputPorts(node) {
  if (node.data.kind === 'operator') return [dataPort('in', '输入', true)]
  if (node.data.kind === 'north') return [dataPort('in', '输入', true)]
  return []
}

function buildOutputPorts(node) {
  if (node.data.kind === 'south') return [dataPort('out', '输出', false)]
  if (node.data.kind === 'operator') return [dataPort('out', '输出', false)]
  return []
}

function nodePosition(node) {
  return {
    x: node.position?.x || 0,
    y: node.position?.y || 0
  }
}
```

- [ ] **Step 3: Track current flow bindings**

Near preview refs, add:

```js
const currentFlowBindings = ref([])
```

When loading a flow, after `flowStatus.value = flow.status`, add:

```js
      currentFlowBindings.value = flow.bindings || []
```

- [ ] **Step 4: Load persisted node positions and config**

In the flow load node mapping, replace:

```js
        position: { x: 100, y: 100 },
        data: {
          name: n.name,
          kind: n.kind,
          operatorName: n.operator_name,
          config: n.config || {},
          operatorConfig: n.operator_config || {},
          subscriptions: []
        }
```

with:

```js
        position: n.position || { x: 100, y: 100 },
        data: {
          name: n.name,
          kind: n.kind,
          operatorName: n.operator_name,
          config: n.config || {},
          operatorConfig: n.config || {},
          subscriptions: []
        }
```

- [ ] **Step 5: Save real config, ports, positions, and bindings**

In `handleSave`, replace each saved node object:

```js
      config: n.data.config || {},
      operator_config: n.data.operatorConfig || {},
      input_ports: n.data.kind === 'operator' ? [{ id: 'in', name: '输入', port_type: 'data', required: true }] : [],
      output_ports: n.data.kind === 'operator' ? [{ id: 'out', name: '输出', port_type: 'data', required: false }] : [],
```

with:

```js
      config: n.data.kind === 'operator'
        ? (n.data.operatorConfig || {})
        : (n.data.config || {}),
      input_ports: buildInputPorts(n),
      output_ports: buildOutputPorts(n),
      position: nodePosition(n),
```

In the save body, after `edges: flowEdges,`, add:

```js
      bindings: currentFlowBindings.value || [],
```

- [ ] **Step 6: Verify unwanted `operator_config` is gone**

Run:

```bash
rg -n "operator_config" web/src/views/FlowEditor.vue
```

Expected: no output.

- [ ] **Step 7: Run frontend build**

Run:

```bash
cd web && npm run build
```

Expected: build succeeds.

- [ ] **Step 8: Commit**

```bash
git add web/src/views/FlowEditor.vue
git commit -m "fix(web): save real FlowEditor graph data" -m "Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 5: Final Verification

**Files:**
- Validate only; no file edits expected.

- [ ] **Step 1: Frontend build**

```bash
cd web && npm run build
```

Expected: build succeeds.

- [ ] **Step 2: Rust workspace tests**

```bash
cargo test --workspace
```

Expected: all tests pass.

- [ ] **Step 3: Full-chain E2E**

If local E2E venv is missing:

```bash
python3 -m venv e2e/full_chain/.venv
e2e/full_chain/.venv/bin/python -m pip install -r e2e/full_chain/requirements.txt
```

Run:

```bash
E2E_MQTT_HOST=127.0.0.1 E2E_MQTT_PORT=1883 PYTHON_BIN=e2e/full_chain/.venv/bin/python ./scripts/e2e_full_chain.sh
```

Expected:

```text
[result] full-chain E2E passed
```

- [ ] **Step 4: Git status**

```bash
git status --short
```

Expected: clean working tree.

---

## Self-Review Against Spec

Spec coverage:

- Node position model: Task 1.
- Store position persistence: Task 2.
- Draft save leniency: Task 3.
- Frontend position load/save: Task 4.
- Operator config stored in `config`: Task 4.
- Default ports for south/operator/north: Task 4.
- Binding preservation: Task 4.
- Deploy/preview strictness: Task 3 keeps strict validation for non-draft saves and deploy/preview runtime paths still call `flow.validate()` or `FlowRuntime::new()`.
- Regression verification: Task 5.

Type consistency:

- `NodePosition` is referenced as `gateway_flow::NodePosition` from server store tests and load code.
- `FlowNode.position` is `Option<NodePosition>` and uses `#[serde(default)]` for legacy JSON.
- Frontend position shape `{ x, y }` matches backend JSON.
- `FlowNode.config` receives operator UI config; no `operator_config` remains in saved JSON.
- Ports use backend `Port` JSON shape: `{ id, name, port_type, required }`.
