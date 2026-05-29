# IoT Gateway Production Roadmap Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 将当前功能面较完整的 IoT Gateway MVP+ 推进为可演示、可验收、可小规模现场试点的工业级数据采集与编排网关。

**Architecture:** 先冻结功能边界并补齐前后端断裂，再把 FlowRuntime 接入南向采集到北向转发的主数据链路，随后补齐关键算子、告警闭环、协议真实化、配置热更新、可观测性、审计与 RBAC。短期采用 `South poll task -> optional FlowRuntime -> Bus -> North` 的低风险集成方式，中长期演进为 `Raw Bus -> Flow Runtime -> Processed Bus`。

**Tech Stack:** Rust workspace (`gateway-sdk`, `gateway-core`, `gateway-server`, `gateway-flow`, `gateway-plugins/*`), Axum, Tokio, SQLite, Vue 3, Vite, Element Plus, WebSocket, Prometheus metrics.

---

## Current Assessment

当前项目已经具备较完整的 IoT 网关骨架：

- 南北向插件 SDK 与生命周期模型；
- 核心 Manager、Bus、Store、SQLite 持久化；
- REST API、WebSocket、License、用户管理；
- Vue 3 管理台；
- Flow DAG、Runtime、Operator Registry；
- 部分真实协议插件，例如 sim、modbus-tcp、modbus-rtu、opcua、virb、mqtt。

但距离设定目标仍有核心差距：

1. Flow 编排还没有成为南向到北向主链路中的一等处理层。
2. 部分关键算子仍然是透传或实验性实现。
3. 协议插件数量多，但生产级真实协议较少。
4. 前端 API 与后端路由存在断裂。
5. 告警系统没有形成事件、持久化、查询、确认、恢复、推送的闭环。
6. 权限、审计、配置热更新、可观测性还不足以支撑生产现场。

本计划的原则是：**停止继续横向堆功能，优先纵向打穿真实链路。**

---

## File Structure Map

### Documentation

- Create: `docs/product-capability-matrix.md`  
  记录每个插件、算子、API、页面的状态：GA / Beta / Experimental / Stub / Missing。

- Create: `docs/e2e-acceptance-scenarios.md`  
  固化标准端到端验收场景：Sim/Modbus -> Flow -> MQTT/HTTP -> Live Monitor -> Alarm。

- Modify: `docs/架构与Neuron对标.md`  
  更新当前真实差距、插件分级和演进路线。

### Backend API

- Modify: `gateway/gateway-server/src/api/mod.rs`  
  注册缺失路由，例如 hardware、logs、system config、alarm events。

- Modify: `gateway/gateway-server/src/api/handlers.rs`  
  放置轻量级 handler，或拆分已有 handler 时保持导出一致。

- Create: `gateway/gateway-server/src/api/hardware.rs`  
  系统硬件与运行时信息 API。

- Create: `gateway/gateway-server/src/api/logs.rs`  
  日志下载与日志配置 API。

- Create: `gateway/gateway-server/src/api/system_config.rs`  
  系统配置查询与更新 API。

### Alarm Domain

- Create: `gateway/gateway-server/src/alarm/mod.rs`  
  告警领域模型、状态枚举、服务接口。

- Create: `gateway/gateway-server/src/alarm/store.rs`  
  SQLite 告警事件持久化。

- Create: `gateway/gateway-server/src/alarm/handlers.rs`  
  告警 REST API handlers。

- Modify: `gateway/gateway-server/src/websocket/mod.rs`  
  增加告警事件 WebSocket 推送。

### Flow Integration

- Modify: `gateway/gateway-core/src/manager.rs`  
  在 south group poll task 中接入 FlowRuntime 绑定解析与执行。

- Modify: `gateway/gateway-core/src/store.rs`  
  如现有 Store 尚无 flow binding，需要增加配置读写入口。

- Modify: `gateway/gateway-flow/src/runtime.rs`  
  补充执行指标、错误策略、preview 入口所需能力。

- Modify: `gateway/gateway-flow/src/operators/aggregate.rs`  
  将透传实现替换为真实 count/time window 聚合。

- Modify: `gateway/gateway-flow/src/operators/buffer.rs`  
  将透传实现替换为真实批量缓冲。

- Modify: `gateway/gateway-flow/src/operators/alarm.rs`  
  与 AlarmEvent 域模型对接。

### Plugins

- Modify: `gateway/gateway-sdk/src/plugin.rs`  
  如有必要，扩展 PluginMeta，加入 status、capabilities、protocol_stack、known_limits。

- Modify: `gateway/gateway-plugins/plugin-http/src/lib.rs`  
  迁移到统一 `NorthPlugin` trait。

- Modify: `gateway/gateway-plugins/plugin-kafka/src/lib.rs`  
  迁移到统一 `NorthPlugin` trait。

### Frontend

- Modify: `web/src/api.js`  
  与后端路由对齐，去掉或修正不存在的 API。

- Modify: `web/src/views/SystemInfo.vue`  
  使用真实 `/api/hardware` 数据。

- Modify: `web/src/views/Logs.vue`  
  使用真实日志下载与配置 API。

- Modify: `web/src/views/SystemConfig.vue`  
  使用真实系统配置 API。

- Modify: `web/src/components/AlarmEventList.vue`  
  接入真实告警事件查询、ack、resolve 和 WebSocket 刷新。

- Modify: `web/src/views/FlowEditor.vue`  
  替换 mock preview，接入真实 Flow preview API 或 WebSocket。

### Tests

- Add backend tests near existing crate conventions, prioritizing:
  - API route tests；
  - FlowRuntime integration tests；
  - operator unit tests；
  - alarm store tests；
  - plugin integration tests。

---

## Phase 0: Baseline, Scope Freeze, and Capability Matrix

**Goal:** 冻结当前功能边界，明确哪些功能真实可用，哪些只是占位、实验性或缺失。

### Task 0.1: Create product capability matrix

**Files:**
- Create: `docs/product-capability-matrix.md`

- [ ] **Step 1: Create capability matrix document**

Write this content:

```markdown
# Product Capability Matrix

> Status values: `GA`, `Beta`, `Experimental`, `Stub`, `Missing`.
> Protocol stack values: `real`, `simulated`, `partial`, `none`.

## South Plugins

| Plugin | Status | Protocol Stack | Read | Write | Browse/List | Notes |
|---|---:|---:|---:|---:|---:|---|
| sim | GA | simulated | yes | no | yes | Built-in simulator for demos and tests. |
| modbus-tcp | Beta | real | yes | yes | limited | Requires integration tests against minimal server. |
| modbus-rtu | Beta | real | yes | yes | limited | Requires serial environment validation. |
| opcua | Beta | real | yes | partial | partial | Requires server compatibility matrix. |
| virb | Beta | real | yes | partial | limited | Project-specific protocol. |
| bacnet | Experimental | simulated | yes | no | no | Must not be marketed as production-ready. |
| s7 | Experimental | simulated | yes | no | no | Needs real S7 protocol implementation. |
| dlt645 | Experimental | simulated | yes | no | no | Needs real meter integration. |
| iec61850 | Experimental | simulated | yes | no | no | Needs real client stack. |
| ethernet-ip | Experimental | simulated | yes | no | no | Needs real client stack. |
| mitsubishi-mc | Experimental | simulated | yes | no | no | Needs real PLC integration. |
| profinet | Experimental | simulated | yes | no | no | Needs real stack feasibility review. |
| snmp | Experimental | simulated | yes | no | no | Should be promoted early because testing is feasible. |
| omron-fins | Experimental | simulated | yes | no | no | Needs real PLC integration. |

## North Plugins

| Plugin | Status | Protocol Stack | Publish | Subscriptions | Notes |
|---|---:|---:|---:|---:|---|
| mqtt | Beta | real | yes | yes | Should be part of standard E2E scenario. |
| http | Stub | partial | partial | no | Must migrate to `NorthPlugin`. |
| kafka | Stub | partial | partial | no | Must migrate to `NorthPlugin`. |
| influxdb | Experimental | partial | partial | no | Needs real integration tests. |
| tdengine | Experimental | partial | partial | no | Needs real integration tests. |
| websocket | Experimental | partial | yes | partial | Clarify runtime semantics. |
| grpc | Experimental | partial | partial | no | Needs contract definition. |
| sparkplug | Experimental | partial | partial | no | Clarify south/north roles. |

## Flow Operators

| Operator | Status | Notes |
|---|---:|---|
| filter | Beta | Must be covered by E2E tests. |
| transform | Beta | Must be covered by E2E tests. |
| alarm | Experimental | Must emit persisted AlarmEvent. |
| aggregate | Stub | Current pass-through must be replaced. |
| buffer | Stub | Current pass-through must be replaced. |
| deadband | Beta | Must be covered by unit tests. |
| change | Beta | Must be covered by unit tests. |
| throttle | Experimental | Needs runtime metrics. |

## Product Areas

| Area | Status | Gap |
|---|---:|---|
| South -> Bus -> North | Beta | Works for selected plugins. |
| South -> Flow -> North | Experimental | Flow must enter main data path. |
| API/UI parity | Experimental | Several frontend APIs currently lack backend routes. |
| Alarm events | Missing | Need event model, persistence, API, WebSocket. |
| RBAC | Missing | Current auth is not enough for enterprise use. |
| Audit logs | Missing | Required for operational accountability. |
| Config hot reload | Experimental | Group intervals, tags, subscriptions, flows need partial reload. |
| Observability | Beta | Needs per-node, per-group, per-flow, per-operator metrics. |
```

- [ ] **Step 2: Commit**

```bash
git add docs/product-capability-matrix.md
git commit -m "docs: add product capability matrix"
```

### Task 0.2: Create E2E acceptance scenarios

**Files:**
- Create: `docs/e2e-acceptance-scenarios.md`

- [ ] **Step 1: Create acceptance scenario document**

Write this content:

```markdown
# E2E Acceptance Scenarios

This document defines the minimum end-to-end scenarios that must keep working while the gateway evolves from MVP+ to production candidate.

## Scenario A: Sim -> Flow -> MQTT -> Live Monitor

**Purpose:** Prove the main product promise: collect data, process it, forward it, and observe it.

**Setup:**

1. Start gateway with auth disabled for local test:
   ```bash
   GATEWAY_DISABLE_AUTH=1 cargo run -p gateway-server
   ```
2. Start frontend:
   ```bash
   cd web && npm run dev
   ```
3. Create a sim south node with one group and one numeric tag.
4. Create a Flow bound to that group:
   - filter: allow all values;
   - transform: multiply numeric value by 2;
   - alarm: emit high severity when transformed value is greater than 80.
5. Create a MQTT north node subscribed to the processed group.

**Expected:**

- MQTT receives transformed values, not raw values.
- Live Monitor displays the processed values.
- When transformed value exceeds threshold, an AlarmEvent appears.
- Flow metrics show input count, output count, error count, and latency.

## Scenario B: Modbus TCP -> Flow -> MQTT

**Purpose:** Prove at least one real south protocol works through Flow.

**Setup:**

1. Start a minimal Modbus TCP test server.
2. Configure Modbus TCP node and holding register group.
3. Bind Flow with a transform operator.
4. Subscribe MQTT north node.

**Expected:**

- Gateway reads real Modbus register values.
- Flow transforms values.
- MQTT receives transformed values.
- Poll errors and latencies are visible in metrics.

## Scenario C: Alarm lifecycle

**Purpose:** Prove alarm domain is complete.

**Setup:**

1. Use Sim or Modbus data source.
2. Configure alarm threshold.
3. Trigger active alarm.
4. Acknowledge alarm in UI.
5. Return value to normal and resolve alarm.

**Expected:**

- Alarm is persisted in SQLite.
- Alarm remains after gateway restart.
- Alarm list supports query, ack, and resolve.
- WebSocket pushes created, acknowledged, and resolved events.

## Scenario D: API/UI parity smoke test

**Purpose:** Ensure frontend-visible features have backend support.

**Expected routes:**

- `GET /api/hardware`
- `GET /api/logs/download`
- `GET /api/logs/config`
- `PUT /api/logs/config`
- `GET /api/system/config`
- `PUT /api/system/config`
- `GET /api/alarm/events`
- `POST /api/alarm/events/:id/ack`
- `POST /api/alarm/events/:id/resolve`

Each route must return a non-404 response and a documented JSON shape or file response.
```

- [ ] **Step 2: Commit**

```bash
git add docs/e2e-acceptance-scenarios.md
git commit -m "docs: add e2e acceptance scenarios"
```

---

## Phase 1: Fix API/UI Parity

**Goal:** 修复现有页面背后的 404 和空壳功能，使管理台可演示、可验收。

### Task 1.1: Add hardware information API

**Files:**
- Create: `gateway/gateway-server/src/api/hardware.rs`
- Modify: `gateway/gateway-server/src/api/mod.rs`
- Modify: `web/src/api.js`
- Modify: `web/src/views/SystemInfo.vue`

- [ ] **Step 1: Add failing route test**

Add a test following existing API test conventions. If the project lacks route tests, add a minimal test module near the API module:

```rust
#[tokio::test]
async fn hardware_endpoint_returns_system_information() {
    let response = get_json("/api/hardware").await;
    assert_eq!(response.status(), 200);

    let body: serde_json::Value = response.json().await.unwrap();
    assert!(body.get("hostname").is_some());
    assert!(body.get("os").is_some());
    assert!(body.get("arch").is_some());
    assert!(body.get("cpu_count").is_some());
    assert!(body.get("uptime_seconds").is_some());
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p gateway-server hardware_endpoint_returns_system_information
```

Expected: FAIL because `/api/hardware` is not registered.

- [ ] **Step 3: Implement handler**

Create `gateway/gateway-server/src/api/hardware.rs`:

```rust
use axum::Json;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct HardwareInfo {
    pub hostname: String,
    pub os: String,
    pub arch: String,
    pub cpu_count: usize,
    pub uptime_seconds: u64,
}

pub async fn get_hardware_info() -> Json<HardwareInfo> {
    Json(HardwareInfo {
        hostname: hostname::get()
            .ok()
            .and_then(|name| name.into_string().ok())
            .unwrap_or_else(|| "unknown".to_string()),
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        cpu_count: std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1),
        uptime_seconds: 0,
    })
}
```

If `hostname` crate is not already available, prefer an existing system-info crate in the repository. If none exists, use `std::env::var("HOSTNAME")` on Unix and `COMPUTERNAME` on Windows instead of adding a new dependency.

- [ ] **Step 4: Register route**

In `gateway/gateway-server/src/api/mod.rs`, add module and route:

```rust
mod hardware;

.route("/hardware", get(hardware::get_hardware_info))
```

Place it with other `/api/*` routes.

- [ ] **Step 5: Align frontend API**

In `web/src/api.js`, ensure hardware API points to the backend route:

```js
hardwareInfo() {
  return request('/hardware')
}
```

- [ ] **Step 6: Run tests and build**

```bash
cargo test -p gateway-server hardware_endpoint_returns_system_information
cd web && npm run build
```

Expected: backend route test passes and frontend build succeeds.

- [ ] **Step 7: Commit**

```bash
git add gateway/gateway-server/src/api/hardware.rs gateway/gateway-server/src/api/mod.rs web/src/api.js web/src/views/SystemInfo.vue
git commit -m "feat(api): add hardware information endpoint"
```

### Task 1.2: Add logs download and log config API

**Files:**
- Create: `gateway/gateway-server/src/api/logs.rs`
- Modify: `gateway/gateway-server/src/api/mod.rs`
- Modify: `web/src/api.js`
- Modify: `web/src/views/Logs.vue`

- [ ] **Step 1: Define API shapes**

Use these JSON shapes:

```json
{
  "level": "info,tower_http=debug",
  "dynamic_reload": false,
  "restart_required": true
}
```

- [ ] **Step 2: Add failing route tests**

```rust
#[tokio::test]
async fn log_config_endpoint_returns_current_log_filter() {
    let response = get_json("/api/logs/config").await;
    assert_eq!(response.status(), 200);

    let body: serde_json::Value = response.json().await.unwrap();
    assert!(body.get("level").unwrap().is_string());
    assert_eq!(body.get("restart_required").unwrap(), true);
}

#[tokio::test]
async fn logs_download_endpoint_does_not_404() {
    let response = get("/api/logs/download").await;
    assert_ne!(response.status(), 404);
}
```

- [ ] **Step 3: Run tests to verify failure**

```bash
cargo test -p gateway-server log_config_endpoint_returns_current_log_filter logs_download_endpoint_does_not_404
```

Expected: FAIL because routes are not registered.

- [ ] **Step 4: Implement handlers**

Create `gateway/gateway-server/src/api/logs.rs`:

```rust
use axum::{body::Body, http::StatusCode, response::Response, Json};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct LogConfig {
    pub level: String,
    pub dynamic_reload: bool,
    pub restart_required: bool,
}

pub async fn get_log_config() -> Json<LogConfig> {
    Json(LogConfig {
        level: std::env::var("RUST_LOG").unwrap_or_else(|_| "info,tower_http=debug".to_string()),
        dynamic_reload: false,
        restart_required: true,
    })
}

pub async fn set_log_config(Json(payload): Json<LogConfig>) -> Json<LogConfig> {
    Json(LogConfig {
        level: payload.level,
        dynamic_reload: false,
        restart_required: true,
    })
}

pub async fn download_logs() -> Result<Response<Body>, StatusCode> {
    let body = "log download is available when file logging is configured\n";
    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "text/plain; charset=utf-8")
        .header("content-disposition", "attachment; filename=\"gateway.log\"")
        .body(Body::from(body.to_string()))
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}
```

- [ ] **Step 5: Register routes**

In `gateway/gateway-server/src/api/mod.rs`:

```rust
mod logs;

.route("/logs/download", get(logs::download_logs))
.route("/logs/config", get(logs::get_log_config).put(logs::set_log_config))
```

- [ ] **Step 6: Align frontend API names**

In `web/src/api.js`, ensure these functions exist and match:

```js
downloadLog() {
  return request('/logs/download', { responseType: 'blob' })
},
getLogConfig() {
  return request('/logs/config')
},
setLogConfig(config) {
  return request('/logs/config', { method: 'PUT', body: JSON.stringify(config) })
}
```

- [ ] **Step 7: Run tests and build**

```bash
cargo test -p gateway-server log_config_endpoint_returns_current_log_filter logs_download_endpoint_does_not_404
cd web && npm run build
```

Expected: tests pass and frontend build succeeds.

- [ ] **Step 8: Commit**

```bash
git add gateway/gateway-server/src/api/logs.rs gateway/gateway-server/src/api/mod.rs web/src/api.js web/src/views/Logs.vue
git commit -m "feat(api): add logs endpoints"
```

### Task 1.3: Add system config API

**Files:**
- Create: `gateway/gateway-server/src/api/system_config.rs`
- Modify: `gateway/gateway-server/src/api/mod.rs`
- Modify: `web/src/api.js`
- Modify: `web/src/views/SystemConfig.vue`

- [ ] **Step 1: Define initial read-mostly config shape**

```json
{
  "http_port": 4000,
  "data_dir": "data",
  "plugins_dir": "plugins",
  "static_dir": "web/dist",
  "auth_enabled": true,
  "backup_enabled": true,
  "backup_retention": 5,
  "log_level": "info,tower_http=debug"
}
```

- [ ] **Step 2: Add failing tests**

```rust
#[tokio::test]
async fn system_config_endpoint_returns_gateway_config() {
    let response = get_json("/api/system/config").await;
    assert_eq!(response.status(), 200);

    let body: serde_json::Value = response.json().await.unwrap();
    assert!(body.get("http_port").is_some());
    assert!(body.get("data_dir").is_some());
    assert!(body.get("plugins_dir").is_some());
    assert!(body.get("auth_enabled").is_some());
}
```

- [ ] **Step 3: Run test to verify failure**

```bash
cargo test -p gateway-server system_config_endpoint_returns_gateway_config
```

Expected: FAIL because route is missing.

- [ ] **Step 4: Implement handlers**

Create `gateway/gateway-server/src/api/system_config.rs`:

```rust
use axum::Json;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemConfigResponse {
    pub http_port: u16,
    pub data_dir: String,
    pub plugins_dir: String,
    pub static_dir: String,
    pub auth_enabled: bool,
    pub backup_enabled: bool,
    pub backup_retention: u32,
    pub log_level: String,
}

pub async fn get_system_config() -> Json<SystemConfigResponse> {
    Json(SystemConfigResponse {
        http_port: std::env::var("GATEWAY_PORT")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(4000),
        data_dir: std::env::var("GATEWAY_DATA_DIR").unwrap_or_else(|_| "data".to_string()),
        plugins_dir: std::env::var("GATEWAY_PLUGINS_DIR").unwrap_or_else(|_| "plugins".to_string()),
        static_dir: std::env::var("GATEWAY_STATIC_DIR").unwrap_or_else(|_| "web/dist".to_string()),
        auth_enabled: !matches!(std::env::var("GATEWAY_DISABLE_AUTH").as_deref(), Ok("1") | Ok("true")),
        backup_enabled: true,
        backup_retention: 5,
        log_level: std::env::var("RUST_LOG").unwrap_or_else(|_| "info,tower_http=debug".to_string()),
    })
}

pub async fn set_system_config(Json(payload): Json<SystemConfigResponse>) -> Json<SystemConfigResponse> {
    Json(payload)
}
```

- [ ] **Step 5: Register routes**

```rust
mod system_config;

.route(
    "/system/config",
    get(system_config::get_system_config).put(system_config::set_system_config),
)
```

- [ ] **Step 6: Align frontend API**

```js
getSystemConfig() {
  return request('/system/config')
},
setSystemConfig(config) {
  return request('/system/config', { method: 'PUT', body: JSON.stringify(config) })
}
```

- [ ] **Step 7: Run tests and build**

```bash
cargo test -p gateway-server system_config_endpoint_returns_gateway_config
cd web && npm run build
```

Expected: tests pass and frontend build succeeds.

- [ ] **Step 8: Commit**

```bash
git add gateway/gateway-server/src/api/system_config.rs gateway/gateway-server/src/api/mod.rs web/src/api.js web/src/views/SystemConfig.vue
git commit -m "feat(api): add system config endpoint"
```

---

## Phase 2: Make Flow Part of the Main Data Path

**Goal:** 让 Flow 从旁路功能变成可选主链路处理层。

### Task 2.1: Define Flow binding model

**Files:**
- Modify: `gateway/gateway-flow/src/flow.rs`
- Modify: `gateway/gateway-server/src/flow/handlers.rs`
- Modify: `gateway/gateway-server/src/flow/store.rs`
- Modify: `web/src/views/FlowEditor.vue`

- [ ] **Step 1: Add binding model test**

```rust
#[test]
fn flow_binding_serializes_south_group_source() {
    let binding = FlowBinding {
        south_node_id: "sim-1".to_string(),
        group_id: "g1".to_string(),
        enabled: true,
        failure_policy: FlowFailurePolicy::FailOpen,
    };

    let value = serde_json::to_value(&binding).unwrap();
    assert_eq!(value["south_node_id"], "sim-1");
    assert_eq!(value["group_id"], "g1");
    assert_eq!(value["enabled"], true);
    assert_eq!(value["failure_policy"], "fail_open");
}
```

- [ ] **Step 2: Implement binding types**

Add to `gateway/gateway-flow/src/flow.rs` or a nearby existing model file:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FlowBinding {
    pub south_node_id: String,
    pub group_id: String,
    pub enabled: bool,
    pub failure_policy: FlowFailurePolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FlowFailurePolicy {
    FailOpen,
    FailClosed,
    PublishError,
}
```

- [ ] **Step 3: Add API shape**

Expose bindings with flow detail responses:

```json
{
  "id": "flow-1",
  "enabled": true,
  "bindings": [
    {
      "south_node_id": "sim-1",
      "group_id": "g1",
      "enabled": true,
      "failure_policy": "fail_open"
    }
  ]
}
```

- [ ] **Step 4: Run flow tests**

```bash
cargo test -p gateway-flow flow_binding_serializes_south_group_source
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add gateway/gateway-flow/src/flow.rs gateway/gateway-server/src/flow/handlers.rs gateway/gateway-server/src/flow/store.rs web/src/views/FlowEditor.vue
git commit -m "feat(flow): add south group bindings"
```

### Task 2.2: Execute bound FlowRuntime before bus publish

**Files:**
- Modify: `gateway/gateway-core/src/manager.rs`
- Modify: `gateway/gateway-flow/src/runtime.rs`
- Test: manager or flow integration tests following existing conventions

- [ ] **Step 1: Add integration test for transformed publish**

```rust
#[tokio::test]
async fn south_group_data_is_transformed_by_bound_flow_before_publish() {
    let mut manager = test_manager_with_sim_plugin().await;
    manager.add_test_flow_binding("sim-1", "g1", multiply_numeric_values_by(2.0)).await;

    let mut rx = manager.subscribe_bus();
    manager.start_node("sim-1").await.unwrap();

    let data = rx.recv().await.unwrap();
    let value = data.values.get("temperature").unwrap().as_f64().unwrap();

    assert_eq!(value, 20.0);
}
```

- [ ] **Step 2: Run test to verify failure**

```bash
cargo test -p gateway-core south_group_data_is_transformed_by_bound_flow_before_publish
```

Expected: FAIL because Manager publishes raw poll output.

- [ ] **Step 3: Add Flow execution seam in Manager**

In the south group poll path of `gateway/gateway-core/src/manager.rs`, introduce a helper shaped like this:

```rust
async fn process_group_data_before_publish(&self, data: GroupData) -> GroupData {
    match self.find_enabled_flow_for_group(&data.node_id, &data.group_id).await {
        Some(binding) => match self.flow_runtime.execute_group_data(&binding.flow_id, data.clone()).await {
            Ok(processed) => processed,
            Err(err) => {
                tracing::warn!(error = %err, "flow execution failed; publishing raw group data");
                data
            }
        },
        None => data,
    }
}
```

Use fail-open as the default policy for the first production roadmap milestone.

- [ ] **Step 4: Publish processed data**

Change the poll loop from this shape:

```rust
let data = plugin.poll_group(&group).await?;
bus.publish(data);
```

to this shape:

```rust
let raw = plugin.poll_group(&group).await?;
let processed = self.process_group_data_before_publish(raw).await;
bus.publish(processed);
```

Adapt exact names to the existing Manager implementation.

- [ ] **Step 5: Run integration test**

```bash
cargo test -p gateway-core south_group_data_is_transformed_by_bound_flow_before_publish
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add gateway/gateway-core/src/manager.rs gateway/gateway-flow/src/runtime.rs
git commit -m "feat(flow): process bound flows before bus publish"
```

### Task 2.3: Replace FlowEditor mock preview with real preview API

**Files:**
- Modify: `gateway/gateway-server/src/flow/handlers.rs`
- Modify: `gateway/gateway-server/src/api/mod.rs`
- Modify: `web/src/api.js`
- Modify: `web/src/views/FlowEditor.vue`

- [ ] **Step 1: Add failing preview API test**

```rust
#[tokio::test]
async fn flow_preview_returns_node_outputs_for_sample_input() {
    let response = post_json(
        "/api/flows/flow-1/preview",
        serde_json::json!({
            "input": {
                "node_id": "sim-1",
                "group_id": "g1",
                "values": { "temperature": 10.0 }
            }
        }),
    ).await;

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    assert!(body.get("nodes").unwrap().is_array());
}
```

- [ ] **Step 2: Implement preview response shape**

```json
{
  "flow_id": "flow-1",
  "nodes": [
    {
      "node_id": "operator-1",
      "input_count": 1,
      "output_count": 1,
      "sample_output": {
        "temperature": 20.0
      }
    }
  ]
}
```

- [ ] **Step 3: Add frontend API method**

```js
previewFlow(flowId, input) {
  return request(`/flows/${flowId}/preview`, {
    method: 'POST',
    body: JSON.stringify({ input })
  })
}
```

- [ ] **Step 4: Replace mock refreshPreview implementation**

In `web/src/views/FlowEditor.vue`, replace hard-coded mock preview with:

```js
async function refreshPreview() {
  previewLoading.value = true
  try {
    previewData.value = await api.previewFlow(flowId.value, previewInput.value)
  } finally {
    previewLoading.value = false
  }
}
```

Use existing reactive variable names if they differ.

- [ ] **Step 5: Run tests and build**

```bash
cargo test -p gateway-server flow_preview_returns_node_outputs_for_sample_input
cd web && npm run build
```

Expected: backend preview test passes and frontend build succeeds.

- [ ] **Step 6: Commit**

```bash
git add gateway/gateway-server/src/flow/handlers.rs gateway/gateway-server/src/api/mod.rs web/src/api.js web/src/views/FlowEditor.vue
git commit -m "feat(flow): add real preview endpoint"
```

---

## Phase 3: Make Critical Operators Real

**Goal:** 先把工业数据最高频的关键算子做真实，不追求一次完善全部 23 个算子。

### Task 3.1: Implement aggregate operator count window

**Files:**
- Modify: `gateway/gateway-flow/src/operators/aggregate.rs`

- [ ] **Step 1: Add failing count-window test**

```rust
#[tokio::test]
async fn aggregate_count_window_outputs_average_min_max_sum_count() {
    let mut op = AggregateOperator::new(AggregateConfig {
        field: "temperature".to_string(),
        window: AggregateWindow::Count { size: 3 },
        functions: vec![AggregateFunction::Avg, AggregateFunction::Min, AggregateFunction::Max, AggregateFunction::Sum, AggregateFunction::Count],
    });

    assert!(op.process(pipeline_value("temperature", 10.0)).await.unwrap().is_empty());
    assert!(op.process(pipeline_value("temperature", 20.0)).await.unwrap().is_empty());

    let output = op.process(pipeline_value("temperature", 30.0)).await.unwrap();
    assert_eq!(output.len(), 1);
    assert_eq!(output[0].values["temperature_avg"], 20.0);
    assert_eq!(output[0].values["temperature_min"], 10.0);
    assert_eq!(output[0].values["temperature_max"], 30.0);
    assert_eq!(output[0].values["temperature_sum"], 60.0);
    assert_eq!(output[0].values["temperature_count"], 3);
}
```

- [ ] **Step 2: Run test to verify failure**

```bash
cargo test -p gateway-flow aggregate_count_window_outputs_average_min_max_sum_count
```

Expected: FAIL because aggregate currently passes data through.

- [ ] **Step 3: Implement stateful count window**

Replace pass-through logic with a stateful buffer:

```rust
struct AggregateState {
    values: Vec<f64>,
}
```

When count reaches `size`, emit one aggregated `PipelineData`, then clear the window.

- [ ] **Step 4: Run operator tests**

```bash
cargo test -p gateway-flow aggregate
```

Expected: aggregate tests pass.

- [ ] **Step 5: Commit**

```bash
git add gateway/gateway-flow/src/operators/aggregate.rs
git commit -m "feat(flow): implement aggregate count window"
```

### Task 3.2: Implement buffer operator batch flush

**Files:**
- Modify: `gateway/gateway-flow/src/operators/buffer.rs`

- [ ] **Step 1: Add failing batch-size test**

```rust
#[tokio::test]
async fn buffer_flushes_when_max_size_is_reached() {
    let mut op = BufferOperator::new(BufferConfig {
        max_size: 3,
        max_delay_ms: 1000,
        flush_on_stop: true,
    });

    assert!(op.process(pipeline_value("temperature", 10.0)).await.unwrap().is_empty());
    assert!(op.process(pipeline_value("temperature", 20.0)).await.unwrap().is_empty());

    let output = op.process(pipeline_value("temperature", 30.0)).await.unwrap();
    assert_eq!(output.len(), 3);
}
```

- [ ] **Step 2: Run test to verify failure**

```bash
cargo test -p gateway-flow buffer_flushes_when_max_size_is_reached
```

Expected: FAIL because buffer currently passes data through immediately.

- [ ] **Step 3: Implement internal buffer**

Use this behavior:

```rust
self.items.push(data);
if self.items.len() >= self.config.max_size {
    let flushed = std::mem::take(&mut self.items);
    Ok(flushed)
} else {
    Ok(vec![])
}
```

- [ ] **Step 4: Run operator tests**

```bash
cargo test -p gateway-flow buffer
```

Expected: buffer tests pass.

- [ ] **Step 5: Commit**

```bash
git add gateway/gateway-flow/src/operators/buffer.rs
git commit -m "feat(flow): implement buffer batch flush"
```

### Task 3.3: Connect alarm operator to AlarmEvent

**Files:**
- Modify: `gateway/gateway-flow/src/operators/alarm.rs`
- Create/Modify: `gateway/gateway-server/src/alarm/mod.rs`

- [ ] **Step 1: Add failing alarm event test**

```rust
#[tokio::test]
async fn alarm_operator_emits_alarm_event_when_threshold_is_exceeded() {
    let mut op = AlarmOperator::new(AlarmConfig {
        field: "temperature".to_string(),
        condition: AlarmCondition::GreaterThan(80.0),
        severity: AlarmSeverity::High,
        message: "temperature too high".to_string(),
    });

    let result = op.process(pipeline_value("temperature", 92.5)).await.unwrap();

    assert_eq!(result.alarm_events.len(), 1);
    assert_eq!(result.alarm_events[0].severity, AlarmSeverity::High);
    assert_eq!(result.alarm_events[0].message, "temperature too high");
}
```

- [ ] **Step 2: Run test to verify failure**

```bash
cargo test -p gateway-flow alarm_operator_emits_alarm_event_when_threshold_is_exceeded
```

Expected: FAIL until alarm output can carry events.

- [ ] **Step 3: Define alarm event output contract**

Add an event side-channel to flow execution result rather than encoding alarms as normal data only:

```rust
pub struct OperatorProcessResult {
    pub data: Vec<PipelineData>,
    pub alarm_events: Vec<AlarmEventDraft>,
}
```

Use `AlarmEventDraft` in `gateway-flow` to avoid depending directly on `gateway-server`.

- [ ] **Step 4: Adapt runtime to collect alarm events**

FlowRuntime should aggregate operator side effects:

```rust
pub struct FlowExecutionResult {
    pub output: Vec<PipelineData>,
    pub alarm_events: Vec<AlarmEventDraft>,
}
```

- [ ] **Step 5: Run flow tests**

```bash
cargo test -p gateway-flow alarm
```

Expected: alarm tests pass.

- [ ] **Step 6: Commit**

```bash
git add gateway/gateway-flow/src/operators/alarm.rs gateway/gateway-flow/src/runtime.rs gateway/gateway-server/src/alarm/mod.rs
git commit -m "feat(flow): emit alarm events from alarm operator"
```

---

## Phase 4: Build Alarm Lifecycle

**Goal:** 告警从算子触发到持久化、查询、确认、恢复、实时推送形成闭环。

### Task 4.1: Add AlarmEvent model and store

**Files:**
- Create: `gateway/gateway-server/src/alarm/mod.rs`
- Create: `gateway/gateway-server/src/alarm/store.rs`
- Modify: `gateway/gateway-server/src/main.rs`

- [ ] **Step 1: Add store test**

```rust
#[tokio::test]
async fn alarm_store_persists_and_lists_events() {
    let store = AlarmStore::new_in_memory().await.unwrap();
    let event = AlarmEvent::new_high(
        "flow".to_string(),
        "flow-1".to_string(),
        "temperature".to_string(),
        92.5,
        "temperature > 80".to_string(),
    );

    store.insert(event.clone()).await.unwrap();
    let events = store.list(AlarmEventFilter::default()).await.unwrap();

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].id, event.id);
    assert_eq!(events[0].status, AlarmStatus::Active);
}
```

- [ ] **Step 2: Define model**

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlarmEvent {
    pub id: String,
    pub source_type: String,
    pub source_id: String,
    pub node_id: Option<String>,
    pub group_id: Option<String>,
    pub tag: String,
    pub severity: AlarmSeverity,
    pub status: AlarmStatus,
    pub message: String,
    pub value: serde_json::Value,
    pub threshold: Option<serde_json::Value>,
    pub created_at: String,
    pub updated_at: String,
    pub acked_at: Option<String>,
    pub resolved_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AlarmSeverity {
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AlarmStatus {
    Active,
    Acknowledged,
    Resolved,
}
```

- [ ] **Step 3: Implement SQLite schema**

Create table:

```sql
CREATE TABLE IF NOT EXISTS alarm_events (
    id TEXT PRIMARY KEY,
    source_type TEXT NOT NULL,
    source_id TEXT NOT NULL,
    node_id TEXT,
    group_id TEXT,
    tag TEXT NOT NULL,
    severity TEXT NOT NULL,
    status TEXT NOT NULL,
    message TEXT NOT NULL,
    value_json TEXT NOT NULL,
    threshold_json TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    acked_at TEXT,
    resolved_at TEXT
);
```

- [ ] **Step 4: Run alarm store test**

```bash
cargo test -p gateway-server alarm_store_persists_and_lists_events
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add gateway/gateway-server/src/alarm/mod.rs gateway/gateway-server/src/alarm/store.rs gateway/gateway-server/src/main.rs
git commit -m "feat(alarm): add alarm event store"
```

### Task 4.2: Add alarm REST API

**Files:**
- Create: `gateway/gateway-server/src/alarm/handlers.rs`
- Modify: `gateway/gateway-server/src/api/mod.rs`
- Modify: `web/src/api.js`
- Modify: `web/src/components/AlarmEventList.vue`

- [ ] **Step 1: Add API tests**

```rust
#[tokio::test]
async fn alarm_events_endpoint_lists_events() {
    let response = get_json("/api/alarm/events").await;
    assert_eq!(response.status(), 200);

    let body: serde_json::Value = response.json().await.unwrap();
    assert!(body.get("items").unwrap().is_array());
}

#[tokio::test]
async fn alarm_ack_endpoint_changes_status() {
    let id = seed_active_alarm().await;
    let response = post_json(&format!("/api/alarm/events/{}/ack", id), serde_json::json!({})).await;
    assert_eq!(response.status(), 200);

    let event = get_alarm(&id).await;
    assert_eq!(event.status, AlarmStatus::Acknowledged);
}
```

- [ ] **Step 2: Implement handlers**

```rust
pub async fn list_alarm_events(State(state): State<AppState>) -> Json<AlarmEventListResponse> {
    let items = state.alarm_store.list(AlarmEventFilter::default()).await.unwrap_or_default();
    Json(AlarmEventListResponse { items })
}

pub async fn ack_alarm_event(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<AlarmEvent>, StatusCode> {
    let event = state.alarm_store.ack(&id).await.map_err(|_| StatusCode::NOT_FOUND)?;
    Ok(Json(event))
}

pub async fn resolve_alarm_event(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<AlarmEvent>, StatusCode> {
    let event = state.alarm_store.resolve(&id).await.map_err(|_| StatusCode::NOT_FOUND)?;
    Ok(Json(event))
}
```

- [ ] **Step 3: Register routes**

```rust
.route("/alarm/events", get(alarm::handlers::list_alarm_events))
.route("/alarm/events/:id/ack", post(alarm::handlers::ack_alarm_event))
.route("/alarm/events/:id/resolve", post(alarm::handlers::resolve_alarm_event))
```

- [ ] **Step 4: Add frontend API methods**

```js
alarmEvents(params = {}) {
  return request(`/alarm/events?${new URLSearchParams(params)}`)
},
ackAlarmEvent(id) {
  return request(`/alarm/events/${id}/ack`, { method: 'POST' })
},
resolveAlarmEvent(id) {
  return request(`/alarm/events/${id}/resolve`, { method: 'POST' })
}
```

- [ ] **Step 5: Run tests and build**

```bash
cargo test -p gateway-server alarm_events_endpoint_lists_events alarm_ack_endpoint_changes_status
cd web && npm run build
```

Expected: tests pass and frontend build succeeds.

- [ ] **Step 6: Commit**

```bash
git add gateway/gateway-server/src/alarm/handlers.rs gateway/gateway-server/src/api/mod.rs web/src/api.js web/src/components/AlarmEventList.vue
git commit -m "feat(alarm): add alarm event API"
```

### Task 4.3: Add alarm WebSocket events

**Files:**
- Modify: `gateway/gateway-server/src/websocket/mod.rs`
- Modify: `web/src/components/AlarmEventList.vue`

- [ ] **Step 1: Define WebSocket payloads**

```json
{
  "type": "alarm.created",
  "event": {}
}
```

```json
{
  "type": "alarm.acknowledged",
  "event": {}
}
```

```json
{
  "type": "alarm.resolved",
  "event": {}
}
```

- [ ] **Step 2: Add route**

Register:

```text
/ws/alarms
```

- [ ] **Step 3: Broadcast store changes**

On insert, ack, and resolve, publish alarm events to a broadcast channel consumed by `/ws/alarms`.

- [ ] **Step 4: Update frontend to refresh on events**

In `AlarmEventList.vue`, open WebSocket and refresh or patch local list when receiving `alarm.created`, `alarm.acknowledged`, or `alarm.resolved`.

- [ ] **Step 5: Manual verification**

Run:

```bash
GATEWAY_DISABLE_AUTH=1 cargo run -p gateway-server
cd web && npm run dev
```

Expected:

- Open alarm list page.
- Trigger alarm through Flow.
- New alarm appears without manual refresh.

- [ ] **Step 6: Commit**

```bash
git add gateway/gateway-server/src/websocket/mod.rs web/src/components/AlarmEventList.vue
git commit -m "feat(alarm): stream alarm lifecycle events"
```

---

## Phase 5: Protocol Realization and Plugin Classification

**Goal:** 把“插件数量”转化为“真实可交付能力”，并避免 stub 被误认为生产可用。

### Task 5.1: Add plugin status metadata

**Files:**
- Modify: `gateway/gateway-sdk/src/plugin.rs`
- Modify: plugin metadata in `gateway/gateway-plugins/*/src/lib.rs`
- Modify: `web/src/views/Plugins.vue`

- [ ] **Step 1: Add metadata serialization test**

```rust
#[test]
fn plugin_metadata_includes_status_and_capabilities() {
    let meta = PluginMeta {
        id: "plugin-mqtt".to_string(),
        name: "MQTT".to_string(),
        version: "0.1.0".to_string(),
        status: PluginStatus::Beta,
        protocol_stack: ProtocolStack::Real,
        capabilities: PluginCapabilities {
            read: false,
            write: false,
            publish: true,
            subscribe: true,
            browse: false,
        },
        known_limits: vec!["QoS behavior depends on broker".to_string()],
    };

    let value = serde_json::to_value(meta).unwrap();
    assert_eq!(value["status"], "beta");
    assert_eq!(value["protocol_stack"], "real");
    assert_eq!(value["capabilities"]["publish"], true);
}
```

- [ ] **Step 2: Implement metadata enums**

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginStatus {
    Ga,
    Beta,
    Experimental,
    Stub,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProtocolStack {
    Real,
    Simulated,
    Partial,
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PluginCapabilities {
    pub read: bool,
    pub write: bool,
    pub publish: bool,
    pub subscribe: bool,
    pub browse: bool,
}
```

- [ ] **Step 3: Update plugin metadata**

Use this classification initially:

- `sim`: `ga`, `simulated`
- `modbus-tcp`: `beta`, `real`
- `modbus-rtu`: `beta`, `real`
- `opcua`: `beta`, `real`
- `virb`: `beta`, `real`
- `mqtt`: `beta`, `real`
- `http`: `stub`, `partial`
- `kafka`: `stub`, `partial`
- all simulated industrial plugins: `experimental`, `simulated`

- [ ] **Step 4: Update frontend plugin badges**

In `Plugins.vue`, show badges:

```text
GA / Beta / Experimental / Stub
Real / Simulated / Partial / None
```

- [ ] **Step 5: Run tests and build**

```bash
cargo test -p gateway-sdk plugin_metadata_includes_status_and_capabilities
cd web && npm run build
```

Expected: metadata test passes and frontend build succeeds.

- [ ] **Step 6: Commit**

```bash
git add gateway/gateway-sdk/src/plugin.rs gateway/gateway-plugins web/src/views/Plugins.vue
git commit -m "feat(plugins): expose capability status metadata"
```

### Task 5.2: Migrate HTTP north plugin to `NorthPlugin`

**Files:**
- Modify: `gateway/gateway-plugins/plugin-http/src/lib.rs`
- Modify: `gateway/gateway-server/src/main.rs`

- [ ] **Step 1: Add plugin trait test**

```rust
#[tokio::test]
async fn http_plugin_implements_north_plugin_lifecycle() {
    let mut plugin = HttpNorthPlugin::default();
    plugin.open().await.unwrap();
    plugin.init(serde_json::json!({ "url": "http://127.0.0.1:18080/ingest" })).await.unwrap();
    plugin.start().await.unwrap();
    plugin.set_subscriptions(vec![]).await.unwrap();
    plugin.stop().await.unwrap();
    plugin.uninit().await.unwrap();
    plugin.close().await.unwrap();
}
```

- [ ] **Step 2: Implement `NorthPlugin`**

```rust
#[async_trait]
impl NorthPlugin for HttpNorthPlugin {
    async fn set_subscriptions(&mut self, subscriptions: Vec<GroupSubscription>) -> PluginResult<()> {
        self.subscriptions = subscriptions;
        Ok(())
    }

    async fn on_group_data(&mut self, data: GroupData) -> PluginResult<()> {
        self.publish_json(data).await
    }

    async fn connection_status(&self) -> ConnectionStatus {
        self.status.clone()
    }
}
```

Adapt type names to `gateway-sdk/src/plugin.rs`.

- [ ] **Step 3: Register as built-in north plugin**

In `gateway/gateway-server/src/main.rs`, register the HTTP plugin using the same pattern as MQTT.

- [ ] **Step 4: Run tests**

```bash
cargo test -p plugin-http http_plugin_implements_north_plugin_lifecycle
cargo test -p gateway-server
```

Expected: tests pass.

- [ ] **Step 5: Commit**

```bash
git add gateway/gateway-plugins/plugin-http/src/lib.rs gateway/gateway-server/src/main.rs
git commit -m "feat(plugins): migrate http north plugin to trait lifecycle"
```

---

## Phase 6: Production Hardening

**Goal:** 支撑小规模现场试点所需的热更新、可观测性、审计和初版 RBAC。

### Task 6.1: Add core operational metrics

**Files:**
- Modify: `gateway/gateway-core/src/manager.rs`
- Modify: `gateway/gateway-core/src/data_flow.rs`
- Modify: `gateway/gateway-server/src/api/mod.rs` or metrics module

- [ ] **Step 1: Define required metrics**

Expose these Prometheus metrics:

```text
gateway_south_poll_total{node_id,group_id,plugin_id}
gateway_south_poll_errors_total{node_id,group_id,plugin_id}
gateway_south_poll_duration_ms{node_id,group_id,plugin_id}
gateway_north_publish_total{node_id,plugin_id}
gateway_north_publish_errors_total{node_id,plugin_id}
gateway_flow_exec_total{flow_id}
gateway_flow_exec_errors_total{flow_id}
gateway_flow_exec_duration_ms{flow_id}
gateway_bus_lag_total{node_id}
gateway_alarm_active_total{severity}
```

- [ ] **Step 2: Add metrics output test**

```rust
#[tokio::test]
async fn metrics_endpoint_includes_south_poll_counters() {
    let response = get_text("/api/metrics").await;
    assert!(response.contains("gateway_south_poll_total"));
    assert!(response.contains("gateway_flow_exec_total"));
}
```

- [ ] **Step 3: Instrument poll, publish, flow, alarm paths**

Increment counters in the exact runtime paths rather than only in API handlers.

- [ ] **Step 4: Run metrics test**

```bash
cargo test -p gateway-server metrics_endpoint_includes_south_poll_counters
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add gateway/gateway-core/src/manager.rs gateway/gateway-core/src/data_flow.rs gateway/gateway-server/src/api/mod.rs
git commit -m "feat(metrics): add operational gateway metrics"
```

### Task 6.2: Add audit event log

**Files:**
- Create: `gateway/gateway-server/src/audit/mod.rs`
- Create: `gateway/gateway-server/src/audit/store.rs`
- Create: `gateway/gateway-server/src/audit/handlers.rs`
- Modify: API handlers for node create/delete, tag update, node start/stop, flow publish, backup restore, license update, alarm ack/resolve

- [ ] **Step 1: Define audit event model**

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    pub id: String,
    pub actor: String,
    pub action: String,
    pub resource_type: String,
    pub resource_id: String,
    pub request_id: Option<String>,
    pub created_at: String,
    pub detail: serde_json::Value,
}
```

- [ ] **Step 2: Add store test**

```rust
#[tokio::test]
async fn audit_store_persists_events() {
    let store = AuditStore::new_in_memory().await.unwrap();
    store.record(AuditEvent::new("admin", "node.start", "node", "sim-1")).await.unwrap();

    let events = store.list(AuditFilter::default()).await.unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].action, "node.start");
}
```

- [ ] **Step 3: Add query API**

```text
GET /api/audit/events
```

Response:

```json
{
  "items": [
    {
      "id": "...",
      "actor": "admin",
      "action": "node.start",
      "resource_type": "node",
      "resource_id": "sim-1",
      "request_id": "...",
      "created_at": "...",
      "detail": {}
    }
  ]
}
```

- [ ] **Step 4: Record critical operations**

Record at least:

- login;
- create/delete node;
- start/stop node;
- create/update/delete tag;
- publish/update flow;
- ack/resolve alarm;
- backup restore;
- license update.

- [ ] **Step 5: Run tests**

```bash
cargo test -p gateway-server audit_store_persists_events
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add gateway/gateway-server/src/audit gateway/gateway-server/src/api gateway/gateway-server/src/flow gateway/gateway-server/src/alarm
git commit -m "feat(audit): record critical gateway operations"
```

### Task 6.3: Add RBAC v1

**Files:**
- Modify: `gateway/gateway-server/src/users/mod.rs`
- Modify: auth middleware module
- Modify: `web/src/router.js`
- Modify: user management views

- [ ] **Step 1: Define roles**

Use exactly these roles for v1:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UserRole {
    Admin,
    Operator,
    Viewer,
}
```

Permissions:

| Role | Permissions |
|---|---|
| admin | all actions |
| operator | view, start/stop nodes, ack/resolve alarms |
| viewer | read-only |

- [ ] **Step 2: Add authorization test**

```rust
#[tokio::test]
async fn viewer_cannot_start_node() {
    let token = login_as_role(UserRole::Viewer).await;
    let response = post_with_token("/api/nodes/sim-1/start", token, serde_json::json!({})).await;
    assert_eq!(response.status(), 403);
}

#[tokio::test]
async fn operator_can_ack_alarm() {
    let token = login_as_role(UserRole::Operator).await;
    let alarm_id = seed_active_alarm().await;
    let response = post_with_token(&format!("/api/alarm/events/{}/ack", alarm_id), token, serde_json::json!({})).await;
    assert_eq!(response.status(), 200);
}
```

- [ ] **Step 3: Enforce route-level permissions**

Add middleware or handler guard:

```rust
fn require_role(user: &CurrentUser, allowed: &[UserRole]) -> Result<(), StatusCode> {
    if user.role == UserRole::Admin || allowed.contains(&user.role) {
        Ok(())
    } else {
        Err(StatusCode::FORBIDDEN)
    }
}
```

- [ ] **Step 4: Update frontend route guards**

- `viewer`: hide create/edit/delete/start/stop buttons;
- `operator`: show start/stop and alarm ack/resolve, hide destructive configuration changes;
- `admin`: show all actions.

- [ ] **Step 5: Run tests and build**

```bash
cargo test -p gateway-server viewer_cannot_start_node operator_can_ack_alarm
cd web && npm run build
```

Expected: authorization tests pass and frontend build succeeds.

- [ ] **Step 6: Commit**

```bash
git add gateway/gateway-server/src/users gateway/gateway-server/src/api web/src/router.js web/src/views
git commit -m "feat(auth): add RBAC roles"
```

---

## Version Roadmap

### v0.6: Functional Closed-Loop Version

Target: 演示和内部验收。

Required capabilities:

- API/UI 对齐；
- Sim/Modbus -> Flow -> MQTT 跑通；
- Flow preview 接真实数据；
- filter、transform、alarm、deadband 真实可用；
- 基础 AlarmEvent；
- 插件状态分级。

### v0.7: Field Trial Version

Target: 小规模现场测试。

Required capabilities:

- Modbus TCP/RTU 稳定；
- OPC UA 基本稳定；
- MQTT/HTTP 北向稳定；
- aggregate、buffer 可用；
- 告警 ack/resolve；
- Prometheus metrics 完善；
- 配置热更新初版；
- 审计日志。

### v0.8: Production Candidate

Target: 可交付试运行。

Required capabilities:

- S7 或 BACnet 至少一个真实实现；
- RBAC；
- Flow 版本发布/回滚稳定；
- backup/restore 验证；
- 插件异常隔离策略；
- 长稳测试；
- 完整部署文档。

### v1.0: Production Release

Target: 正式交付。

Required capabilities:

- 明确 GA 协议清单；
- 完整安全模型；
- 完整告警系统；
- 完整可观测性；
- 长稳测试报告；
- 升级迁移机制；
- License 商业策略完善。

---

## Immediate Top 10 Actions

1. 建立功能矩阵，标记 GA / Beta / Experimental / Stub。
2. 修复前端调用但后端不存在的 API。
3. 定义 Flow 与 south group 的绑定关系。
4. 将 FlowRuntime 接入 Manager 主数据链路。
5. 把 FlowEditor mock preview 替换为真实 preview。
6. 实现 filter / transform / alarm / deadband 的端到端测试。
7. 实现 aggregate / buffer 的真实语义，不再透传。
8. 建立 AlarmEvent 表、API、WebSocket。
9. 把 HTTP / Kafka 北向插件迁移到统一 `NorthPlugin`。
10. 为每个真实协议建立 mock server 集成测试。

---

## Key Architecture Decisions

### Decision 1: Prioritize vertical depth over horizontal protocol count

Do not continue adding more protocol shells before the standard chain is reliable:

```text
Modbus TCP/RTU or Sim
  -> Flow filter/transform/alarm/aggregate
  -> MQTT/HTTP
  -> Live Monitor + Alarm + Metrics
```

### Decision 2: Flow must become a first-class data path component

Short-term target:

```text
South Poll Task -> Optional FlowRuntime -> Bus -> North
```

Long-term target:

```text
South Runtime -> Raw Data Bus -> Flow Runtime -> Processed Data Bus -> North Runtime
```

### Decision 3: Plugin semantics must be unified

All south and north plugins should converge on the lifecycle:

```text
open -> init -> start -> stop -> uninit -> close
```

North plugins should converge on:

```text
set_subscriptions()
on_group_data()
connection_status()
```

Dynamic loading can remain, but semantic behavior must be unified.

### Decision 4: Mock and stub must be visible in product metadata

Mock/stub is acceptable during development, but must be explicit in:

- plugin metadata;
- plugin page badges;
- product capability matrix;
- release notes.

---

## Testing Strategy

Run these before marking a phase complete:

```bash
cargo test
cd web && npm run build
```

For Flow phases, also run targeted tests:

```bash
cargo test -p gateway-flow
cargo test -p gateway-core flow
```

For API/UI parity phases:

```bash
cargo test -p gateway-server
cd web && npm run build
```

For protocol phases:

```bash
cargo test -p plugin-modbus-tcp
cargo test -p plugin-modbus-rtu
cargo test -p plugin-mqtt
```

Manual E2E smoke test:

```bash
GATEWAY_DISABLE_AUTH=1 cargo run -p gateway-server
cd web && npm run dev
```

Then verify:

1. Dashboard loads.
2. South node can start.
3. Flow can be enabled and previewed.
4. North MQTT/HTTP receives processed data.
5. Alarm event appears and can be acknowledged/resolved.
6. `/api/metrics` exposes poll, flow, north, and alarm metrics.

---

## Self-Review

### Spec coverage

- Phase 0-6 are represented.
- Version roadmap v0.6-v1.0 is included.
- Priority and key architecture decisions are included.
- API/UI parity, Flow data path, operators, alarm lifecycle, plugins, and production hardening are covered.

### Placeholder scan

This plan intentionally avoids open-ended `TBD` instructions. Where exact code depends on existing project conventions, the step names the exact file and the required behavior, test, command, and expected result.

### Type consistency

Core concepts are consistently named:

- `FlowBinding`
- `FlowFailurePolicy`
- `AlarmEvent`
- `AlarmSeverity`
- `AlarmStatus`
- `PluginStatus`
- `ProtocolStack`
- `PluginCapabilities`

---

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-05-29-iot-gateway-production-roadmap.md`.

Two execution options:

1. **Subagent-Driven (recommended)** - Dispatch a fresh subagent per task, review between tasks, fast iteration.
2. **Inline Execution** - Execute tasks in this session using executing-plans, batch execution with checkpoints.
