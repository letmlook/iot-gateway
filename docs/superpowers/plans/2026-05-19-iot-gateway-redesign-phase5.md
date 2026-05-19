# IoT 网关破坏性重构 — Phase 5 实施计划
> WebSocket 实时可视化 + Flow 运行态监控

**创建时间：** 2026-05-19
**设计文档：** `2026-05-19-iot-gateway-redesign.md` § Phase 5

---

## 目标

Flow 运行数据实时推送，前端 WebSocket 订阅。

```
gateway-server              Web 前端
     │                           │
  FlowRuntime ──────────────────┼── WebSocket /ws/flows/:id/live
     │                           │   {"type": "node_data", "node_id": "...", "tags": {...}}
     └───────────────────────────┘
```

---

## Task A: WebSocket 后端实现

### A1. `gateway-server/src/websocket.rs` — WebSocket 模块

创建 `gateway-server/src/websocket/mod.rs`：

```rust
use axum::{
    extract::{ws::{WebSocket, WebSocketUpgrade}, Path, State},
    response::IntoResponse,
    routing::get,
    Router,
};
use futures::{SinkExt, StreamExt};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::{broadcast, RwLock};
use serde::{Deserialize, Serialize};

// ── 事件类型 ────────────────────────────────────────
#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WsEvent {
    NodeData {
        flow_id: String,
        node_id: String,
        timestamp: i64,
        tags: HashMap<String, serde_json::Value>,
    },
    NodeStatus {
        flow_id: String,
        node_id: String,
        status: String,   // "running" | "error" | "stopped"
        message: Option<String>,
    },
    FlowStatus {
        flow_id: String,
        status: String,   // "running" | "paused" | "stopped"
        nodes_total: usize,
        nodes_running: usize,
    },
    Alarm {
        flow_id: String,
        alarm_id: String,
        level: String,    // "info" | "warn" | "critical"
        message: String,
        timestamp: i64,
    },
}

// ── 订阅中心（broadcast channel）────────────────────
#[derive(Default)]
pub struct WsHub {
    // flow_id -> broadcast::Sender<WsEvent>
    subscriptions: Arc<RwLock<HashMap<String, broadcast::Sender<WsEvent>>>>,
}

impl WsHub {
    pub fn subscribe(&self, flow_id: &str) -> broadcast::Receiver<WsEvent> {
        let guard = self.subscriptions.blocking_read();
        if let Some(tx) = guard.get(flow_id) {
            return tx.subscribe();
        }
        // 不存在则创建新的 channel
        drop(guard);
        let mut subs = self.subscriptions.write().blockingu();
        let (tx, rx) = broadcast::channel(1024);
        subs.insert(flow_id.to_string(), tx.clone());
        rx
    }

    pub fn broadcast(&self, flow_id: &str, event: WsEvent) {
        let guard = self.subscriptions.read().blockingu();
        if let Some(tx) = guard.get(flow_id) {
            let _ = tx.send(event);
        }
    }
}

// ── WebSocket Handler ───────────────────────────────
async fn ws_handler(
    ws: WebSocketUpgrade,
    Path(flow_id): Path<String>,
    State(hub): State<Arc<WsHub>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| websocket_socket(socket, flow_id, hub))
}

async fn websocket_socket(socket: WebSocket, flow_id: String, hub: Arc<WsHub>) {
    let (mut sender, mut receiver) = socket.split();
    let mut rx = hub.subscribe(&flow_id);

    // 发送欢迎消息（当前快照）
    let summary = hub.get_flow_summary(&flow_id).await;
    if let Ok(json) = serde_json::to_string(&summary) {
        let _ = sender.send(futures::sink::drain().feed(json.into()).await);
    }

    loop {
        tokio::select! {
            // 后端推送到前端
            result = rx.recv() => {
                match result {
                    Ok(event) => {
                        let json = serde_json::to_string(&event).unwrap();
                        if sender.send(axum::extract::ws::Message::Text(json).into()).await.is_err() {
                            break; // 客户端断开
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => { /* 跳过旧消息 */ }
                    Err(_) => break,
                }
            }
            // 前端消息（ping/pong 或取消订阅）
            msg = receiver.next() => {
                match msg {
                    Some(Ok(axum::extract::ws::Message::Ping(data))) => {
                        let _ = sender.send(axum::extract::ws::Message::Pong(data).into()).await;
                    }
                    Some(Ok(axum::extract::ws::Message::Text(text))) => {
                        // 解析前端请求
                        if let Ok(req) = serde_json::from_str::<WsClientMsg>(&text) {
                            match req.action.as_str() {
                                "unsubscribe" => { break; }
                                _ => {}
                            }
                        }
                    }
                    Some(Ok(axum::extract::ws::Message::Close(_))) | None => break,
                    _ => {}
                }
            }
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "action")]
enum WsClientMsg {
    #[serde(rename = "unsubscribe")]
    Unsubscribe,
}
```

### A2. WebSocket 路由注册

`gateway-server/src/api/mod.rs` 新增：

```rust
.route("/ws/flows/:id/live", get(websocket::ws_handler))
```

`gateway-server/src/main.rs` 新增：

```rust
mod websocket;
// ...
let hub = Arc::new(websocket::WsHub::default());
// 通过 State 传入 Router
```

### A3. FlowRuntime 集成 — 实时数据上报

修改 `gateway-server/src/flow/mod.rs` 中 `FlowRuntime::run` 循环：

```rust
// 在每个节点处理完成后的循环内：
let event = WsEvent::NodeData {
    flow_id: self.flow.id.clone(),
    node_id: node_id.clone(),
    timestamp: chrono::Utc::now().timestamp_millis(),
    tags: node_output_tags,
};
self.hub.broadcast(&self.flow.id, event);
```

并在节点状态变化时发送 `NodeStatus` 事件，Flow 启动/停止时发送 `FlowStatus` 事件。

### A4. `GET /flows/:id/live/summary` 实现

返回当前 Flow 运行快照：

```json
{
  "flow_id": "flow-001",
  "status": "running",
  "nodes": [
    {
      "node_id": "node-1",
      "status": "running",
      "last_tags": {"temperature": 25.6, "humidity": 60.0},
      "last_update": 1234567890123
    }
  ],
  "start_time": 1234567890000,
  "messages_total": 12345
}
```

---

## Task B: 前端实现

### B1. `web/src/views/LiveMonitor.vue` — 实时监控视图

新建 `web/src/views/LiveMonitor.vue`：

**功能：**
- 左侧：Flow 节点树（状态指示灯）
- 中间：ECharts 实时曲线图（最多 5 条 tag 同时显示）
- 右侧：节点详情面板 + 告警事件列表
- 底部：消息速率仪表盘（msg/s）

**ECharts 曲线图：**
```javascript
// 每秒刷新，接入 WebSocket node_data 事件
const option = {
  xAxis: { type: 'time', name: '时间' },
  yAxis: { type: 'value', name: 'Tag 值' },
  series: [
    {
      name: 'temperature',
      type: 'line',
      smooth: true,
      data: [], // [{time, value}, ...]
      sampling: 'lttb', // 降采样
    }
  ]
};
```

### B2. FlowEditor.vue 嵌入 Live 按钮

在 FlowEditor.vue 顶部工具栏新增按钮：

```html
<el-button type="primary" @click="openLiveMonitor">
  <el-icon><VideoPlay /></el-icon> 实时监控
</el-button>
```

点击后在新 tab 或 drawer 中打开 LiveMonitor 视图，并自动 WebSocket 连接到当前 flow。

### B3. WebSocket 客户端 Hook — `web/src/composables/useFlowWebSocket.js`

```javascript
export function useFlowWebSocket(flowId) {
  const data = ref({});
  let ws = null;

  function connect() {
    ws = new WebSocket(`ws://${location.host}/api/ws/flows/${flowId}/live`);
    ws.onmessage = (e) => {
      const event = JSON.parse(e.data);
      switch (event.type) {
        case 'node_data':
          data.value[event.node_id] = event.tags;
          break;
        case 'node_status':
          // 更新节点状态
          break;
        case 'alarm':
          // 推入告警列表
          break;
      }
    };
    ws.onclose = () => { /* 自动重连 */ };
  }

  function disconnect() { ws?.close(); }

  return { data, connect, disconnect };
}
```

---

## Task C: 告警事件前端

### C1. 告警列表组件 — `web/src/components/AlarmEventList.vue`

- 实时接收 `WsEvent::Alarm` 事件
- 支持按级别过滤（info/warn/critical）
- 支持分页和导出 CSV
- 新告警高亮闪烁效果

### C2. 节点状态徽章

在 FlowEditor.vue 节点树中，根据 `node_status` 事件：

| status | 颜色 | 图标 |
|--------|------|------|
| running | 🟢 绿色 | `CircleCheck` |
| error | 🔴 红色 | `CircleClose` |
| stopped | ⚪ 灰色 | `Remove` |

---

## Task D: 性能优化

### D1. 采样降频

- WebSocket 广播频率上限：100 msg/s per flow
- 超出时自动降采样（时间窗口均匀抽取）
- ECharts 数据点上限：500 点/曲线（超出则滚动丢弃旧数据）

### D2. 连接管理

- 页面不可见时暂停 ECharts 渲染（`document.visibilityState`）
- WebSocket 心跳：30s ping/pong，无响应则 5s 后重连
- 多 tab 同 Flow 共享一个 WebSocket 连接（BroadcastChannel）

---

## 任务清单

| Task | 内容 | 依赖 | 复杂度 |
|------|------|------|--------|
| A1 | `websocket/mod.rs` — WebSocket Hub + Handler | — | 中 |
| A2 | WebSocket 路由注册 | A1 | 低 |
| A3 | FlowRuntime 集成实时数据上报 | A1 | 中 |
| A4 | `GET /flows/:id/live/summary` 实现 | A1 | 低 |
| B1 | `LiveMonitor.vue` — 实时监控视图 | A1-A4 | 高 |
| B2 | FlowEditor 嵌入 Live 按钮 | B1 | 低 |
| B3 | `useFlowWebSocket.js` — WebSocket Hook | A1 | 中 |
| C1 | `AlarmEventList.vue` — 告警列表组件 | B3 | 中 |
| C2 | 节点状态徽章（FlowEditor） | B3 | 低 |
| D1 | 采样降频 + ECharts 滚动 | B1 | 中 |
| D2 | 连接管理与多 Tab 共享 | B3 | 中 |

---

## 交付物

1. `gateway-server/src/websocket/mod.rs` — WebSocket 核心
2. `gateway-server/src/flow/mod.rs` — 集成实时数据上报
3. `gateway-server/src/api/mod.rs` — 新增 `/ws/flows/:id/live` + `/flows/:id/live/summary`
4. `web/src/views/LiveMonitor.vue` — 实时监控视图
5. `web/src/composables/useFlowWebSocket.js` — WebSocket Hook
6. `web/src/components/AlarmEventList.vue` — 告警列表组件
7. `web/src/views/FlowEditor.vue` — 新增 Live 按钮

---

## 验收标准

- [ ] WebSocket 连接建立成功，`ws://localhost:8080/api/ws/flows/:id/live` 返回 101
- [ ] 节点发送数据后，前端 1s 内收到 `node_data` 事件并更新曲线
- [ ] ECharts 曲线图正常渲染，数据滚动更新
- [ ] 告警事件触发后，AlarmEventList 实时展示
- [ ] Flow 停止时，`FlowStatus` 事件正确推送，前端显示 STOPPED
- [ ] 多 Tab 打开同一 Flow 监控，不创建多个 WebSocket 连接
- [ ] `cargo build --workspace` 编译通过
- [ ] `npm run build` 前端构建通过
