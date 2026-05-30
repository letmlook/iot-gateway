# FlowEditor Real Graph Model Design

Date: 2026-05-30

## Goal

Make FlowEditor save and load a real graph model instead of a partially visual draft. This increment focuses on persistence and model consistency: node positions, operator config, ports, edges, draft save behavior, and preserving existing flow bindings.

## Scope

In scope:

- Add persisted node position to the backend Flow model.
- Store and load node positions through the existing `flow_nodes.position_x` and `flow_nodes.position_y` columns.
- Restore VueFlow node positions when opening a saved flow.
- Save VueFlow node positions when saving a flow.
- Store operator configuration in the backend-recognized `FlowNode.config` field.
- Stop relying on `operator_config` as a backend field.
- Generate valid default ports for south, operator, and north flow nodes.
- Allow incomplete draft flows to be saved.
- Keep deploy, preview, and runtime validation strict.
- Preserve existing `FlowBinding` entries when saving from FlowEditor.

Out of scope:

- VueFlow custom node/handle UI.
- Flow binding editor panel.
- Real DAG runtime execution semantics.
- User-editable preview JSON input.
- Dynamic operator schema/default endpoint.
- Major FlowEditor visual redesign.

## Current Problems

### Node positions are not persisted

`FlowEditor.vue` restores every loaded node to:

```js
position: { x: 100, y: 100 }
```

The backend store already has `position_x` and `position_y` columns, but current code writes `0.0` and does not read them back into the flow JSON.

### Operator config is saved to the wrong field

The frontend UI edits `operatorConfig` and saves it as `operator_config`. The backend `FlowNode` model has `config`, not `operator_config`. Flow runtime reads operator settings from `node.config`, so user-edited operator config can be ignored by preview, deploy, and runtime execution.

### Ports are incomplete for graph validation

Flow save currently gives ports only to operator nodes. South and north nodes have empty ports, so common edges such as `south:out -> operator:in -> north:in` can fail backend port validation.

### Draft save is too strict

`update_flow()` validates the full DAG. This makes normal editing awkward because an incomplete draft graph can fail save before the user finishes connecting nodes.

### Bindings can be lost

Flow bindings already connect a flow to real south node/group runtime sources. FlowEditor must preserve existing bindings when saving graph changes, even though this increment does not add binding UI.

## Backend Design

### 1. Add `NodePosition`

In `gateway-flow/src/node.rs`, add:

```rust
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

The field is optional for compatibility with old JSON and non-UI-created flows.

### 2. Persist position in FlowStore

`gateway-server/src/flow/store.rs` already has `position_x` and `position_y` columns.

On save:

- If `node.position` exists, write its `x` and `y`.
- If `node.position` is missing, write `0.0` and `0.0`.

On load:

- Select `position_x` and `position_y`.
- Return `position: Some(NodePosition { x, y })`.

This makes REST Flow JSON include:

```json
{
  "position": { "x": 320, "y": 160 }
}
```

### 3. Draft save validation

Add a helper such as `validate_draft_flow(flow: &Flow)` in flow handlers or flow model code.

For draft flows, validate only:

- `name.trim()` is not empty.
- For update, path id equals body id.
- Every edge references source and target node ids that exist in `flow.nodes`.

Do not require for draft save:

- at least one south and one north node;
- port type matching;
- required inputs connected;
- complete deployable topology.

Keep strict validation for:

- deploy;
- preview runtime creation;
- runtime execution.

This preserves product safety while allowing normal editor drafts.

## Frontend Design

### 1. Load real positions

When loading a flow, map backend position into VueFlow:

```js
position: n.position || { x: 100, y: 100 }
```

If backend data is old and lacks `position`, use the existing fallback.

### 2. Load operator config from `config`

For loaded operator nodes:

```js
data: {
  name: n.name,
  kind: n.kind,
  operatorName: n.operator_name,
  config: n.config || {},
  operatorConfig: n.config || {},
  subscriptions: []
}
```

`operatorConfig` remains an internal UI binding object, but it is initialized from the backend `config` field.

### 3. Save backend-recognized config

When saving nodes:

```js
config: n.data.kind === 'operator'
  ? (n.data.operatorConfig || {})
  : (n.data.config || {})
```

Do not send `operator_config` in the saved node JSON.

### 4. Generate default ports

Save ports by node kind:

```text
south:
  input_ports: []
  output_ports: [{ id: "out", name: "输出", port_type: "data", required: false }]

operator:
  input_ports: [{ id: "in", name: "输入", port_type: "data", required: true }]
  output_ports: [{ id: "out", name: "输出", port_type: "data", required: false }]

north:
  input_ports: [{ id: "in", name: "输入", port_type: "data", required: true }]
  output_ports: []
```

This supports the common visual graph:

```text
south:out -> operator:in -> north:in
```

### 5. Save positions

Each saved FlowNode includes:

```js
position: {
  x: n.position?.x || 0,
  y: n.position?.y || 0
}
```

### 6. Preserve bindings

When loading a flow:

```js
currentFlowBindings.value = flow.bindings || []
```

When saving:

```js
bindings: currentFlowBindings.value || []
```

FlowEditor will not edit bindings in this increment. It only avoids dropping them.

## Validation Behavior

### Draft save

Draft save is lenient enough to persist incomplete graphs.

Examples that should save:

- one operator node only;
- south and operator nodes with no north yet;
- nodes present but not fully connected.

Examples that should still fail:

- empty flow name;
- edge source id does not exist;
- edge target id does not exist;
- update path id differs from body id.

### Deploy / preview / runtime

Strict validation remains unchanged for executable operations.

Incomplete graphs should fail deploy/preview with clear validation errors. Complete graphs should continue to deploy/preview/run.

## Testing Plan

### Backend tests

Add tests for:

1. `FlowNode` position serialization and deserialization.
2. Deserializing old `FlowNode` JSON without `position`.
3. `FlowStore` save/load position round-trip.
4. Draft update/save accepts incomplete draft graphs.
5. Draft save rejects edges that reference missing node ids.
6. Deploy validation remains strict for incomplete graphs.

### Frontend verification

Run:

```bash
cd web && npm run build
```

Manual/browser checks:

1. Open FlowEditor.
2. Add south/operator/north nodes.
3. Move nodes to distinct positions.
4. Save.
5. Reopen the flow.
6. Confirm node positions are restored.
7. Configure an operator.
8. Save and reopen.
9. Confirm operator form is initialized from `config`.
10. Confirm saved node JSON does not include `operator_config`.
11. Confirm edges use `source_port` and `target_port` values that exist on saved node ports.
12. Confirm existing flow bindings remain after saving.

### Regression checks

Run:

```bash
cargo test --workspace
E2E_MQTT_HOST=127.0.0.1 E2E_MQTT_PORT=1883 PYTHON_BIN=e2e/full_chain/.venv/bin/python ./scripts/e2e_full_chain.sh
```

## Acceptance Criteria

- Flow JSON supports optional node `position`.
- FlowStore persists and loads positions.
- FlowEditor restores node positions from backend Flow JSON.
- FlowEditor saves node positions.
- FlowEditor saves operator settings into `config`, not `operator_config`.
- FlowEditor generates default ports for south/operator/north nodes.
- Draft save accepts incomplete but internally consistent graphs.
- Deploy/preview still reject non-executable graphs.
- FlowEditor save preserves existing `bindings`.
- `cargo test --workspace` passes.
- Frontend build passes.
- Full-chain E2E still passes.
