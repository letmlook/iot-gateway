# 自研 IoT 网关：架构与 Neuron 对标

> 南向设备、北向应用均通过插件实现；后端 Rust，前端 Vue + JavaScript。

---

## 一、整体架构

```
                    ┌─────────────────────────────────────────────────────┐
                    │                    Gateway Core                      │
                    │  ┌─────────────┐    Bus (broadcast)   ┌────────────┐ │
                    │  │ Store       │◄────────────────────►│ Manager    │ │
                    │  │ nodes       │    GroupData         │ routing    │ │
                    │  │ groups/tags │                      │ plugins    │ │
                    │  └─────────────┘                      └────────────┘ │
                    └───────────────┬──────────────────────────────┬───────┘
                                    │                              │
         ┌──────────────────────────┼──────────────────────────────┼──────────────────────────┐
         │                          │                              │                          │
         ▼                          ▼                              ▼                          ▼
  ┌─────────────┐            ┌─────────────┐               ┌─────────────┐             ┌─────────────┐
  │ South       │   poll     │ South       │               │ North       │   subscribe │ North       │
  │ Plugin      │◄───────────│ Adapter     │   GroupData   │ Adapter     │◄────────────│ Plugin      │
  │ (sim, …)    │   group    │ (task)      │──────────────►│ (task)      │   filter    │ (mqtt, …)   │
  └─────────────┘            └─────────────┘               └─────────────┘             └─────────────┘
         │                          │                              │                          │
         ▼                          ▼                              ▼                          ▼
    设备 / 协议                轮询采集                        总线订阅                    云 / MQTT
```

- **Core**：Store（节点/组/标签）、Bus（GroupData 广播）、Manager（插件注册、节点启停、路由）。
- **南向**：SouthPlugin 实现协议，按 Group 轮询采集；Adapter 以 tokio 任务运行轮询循环，结果写入 Bus。
- **北向**：NorthPlugin 订阅南向 Group，接收 GroupData，转发到 MQTT/HTTP 等；Adapter 订阅 Bus，按订阅表过滤后调用 `on_group_data`。

---

## 二、与 Neuron 对标

| 能力 | Neuron | 本网关 |
|------|--------|--------|
| 南向插件 | 动态库 .so + C SDK | **.so 动态加载**（cdylib + libloading）或静态链接；trait `SouthPlugin` |
| 北向插件 | 同上 | 同上，trait `NorthPlugin` |
| 消息总线 | NNG 星型 | tokio `broadcast` 广播 GroupData |
| 节点 | 适配器 + 插件实例 | `Node` = 配置 + 插件名；每个节点对应轮询/消费任务 |
| Tag / Group | 点位、分组、轮询间隔 | `Tag`、`Group`，`Group.interval_ms` 控制轮询 |
| 统一数据类型 | Neuron 统一类型 → JSON | `DataValue` 枚举 → JSON |
| 北向订阅 | 订阅表路由 GroupData | `SubscriptionTable`：北向 node → [(south_node, group)] |
| REST API | 节点/组/标签/插件管理 | `/api/health`、`/api/nodes`、groups、tags、subscriptions、plugins 等 |
| Web UI | 内置控制台 | Vue3 + JS 管理台，列表/创建/启停、插件 version |
| 持久化 | 配置与状态落盘 | **SQLite** `data/data.db`，事务写入、启动加载、变更保存；支持从旧版 `data.json` 迁移 |
| 热插拔 | 动态加载 .so | **支持**：`plugins/` 下 .so 启动时加载；内置插件（无 plugins 时） |

---

## 三、目录与 crate 结构

```
iot-gateway/
├── Cargo.toml                 # workspace
├── data/                      # 数据目录（可配置），含 data.db（SQLite）持久化
├── crates/
│   ├── gateway-sdk/           # 插件 SDK：trait、Tag/Group/DataValue、消息类型
│   ├── gateway-core/          # 核心：Bus、Store、Manager、Node、persist
│   ├── gateway-server/        # HTTP API、静态资源、main、config、state
│   ├── gateway-plugin-sim/    # 南向示例：模拟设备
│   └── gateway-plugin-mqtt/   # 北向示例：MQTT（当前占位日志）
├── web/                       # Vue3 + Vite 前端
│   ├── src/
│   │   ├── App.vue
│   │   ├── api.js             # /api 封装
│   │   └── main.js
│   └── dist/                  # 构建产出，网关服务于此
└── docs/
    ├── IoT网关调研报告.md
    └── 架构与Neuron对标.md
```

---

## 四、插件开发

### 南向（SouthPlugin）

- `meta()`：name、kind=South、description、version。
- `open(node_id, config)`：连接设备，初始化状态。
- `close(node_id)`：断开、清理。
- `poll_group(node_id, group_id, tags)`：采集该组 Tag，返回 `(TagId, DataValue)[]`。
- `list_groups` / `list_tags`：供 Core 同步默认组/标签（如 sim 的 default 组）。

### 北向（NorthPlugin）

- `meta()`：name、kind=North、description、version。
- `open` / `close`：连接 MQTT Broker 等。
- `set_subscriptions(node_id, [(south_node_id, group_id), …])`：设置订阅。
- `on_group_data(node_id, data)`：Core 推送 GroupData 时调用。

### 新增插件

1. 在 `crates/` 下新增 `gateway-plugin-xxx`，依赖 `gateway-sdk`。
2. 实现 `SouthPlugin` 或 `NorthPlugin`，导出实例。
3. 在 `gateway-server` 的 `main` 中 `register_south` / `register_north`，并加入 workspace。

---

## 五、API 速览

| 方法 | 路径 | 说明 |
|------|------|------|
| GET | /api/health | 健康检查（status、nodes_count、nodes_running、plugins_south/north） |
| GET | /api/metrics | Prometheus 格式指标 |
| GET | /api/version | 版本信息（version、build_date、revision，对标 Neuron） |
| GET | /api/export | 导出当前快照 JSON |
| GET | /api/plugins/south | 南向插件列表 (name, description, version) |
| GET | /api/plugins/south/:name/config_schema | 南向插件配置 Schema |
| GET | /api/plugins/south/:name/tag_schema | 南向插件点位 Schema |
| GET | /api/plugins/north | 北向插件列表 (name, description, version) |
| GET | /api/plugins/north/:name/config_schema | 北向插件配置 Schema（与南向对称） |
| GET | /api/nodes | 节点列表 |
| POST | /api/nodes | 创建节点（name, kind, plugin_name, config） |
| GET | /api/nodes/:id | 节点详情 |
| PUT | /api/nodes/:id | 更新节点（body: name?，对标 Neuron Update node） |
| DELETE | /api/nodes/:id | 删除节点 |
| POST | /api/nodes/:id/start | 启动节点 |
| POST | /api/nodes/:id/stop | 停止节点 |
| GET | /api/nodes/:id/groups | 组列表 |
| GET | /api/nodes/:id/groups/:gid | 单个组详情 |
| POST | /api/nodes/:id/groups | 添加组 |
| PUT | /api/nodes/:id/groups/:gid | 更新组（body: name?, interval_ms?, description?） |
| DELETE | /api/nodes/:id/groups/:gid | 删除组 |
| GET | /api/nodes/:id/tags | 标签列表 |
| GET | /api/nodes/:id/tags/:tid | 单个标签详情（校验属于该节点） |
| POST | /api/nodes/:id/tags | 添加标签 |
| PUT | /api/nodes/:id/tags/:tid | 更新标签（body: name?, address?, attr?, data_type?, description?） |
| DELETE | /api/nodes/:id/tags/:tid | 删除标签 |
| GET | /api/nodes/:id/subscriptions | 北向订阅列表 |
| PUT | /api/nodes/:id/subscriptions | 设置北向订阅 |
| GET | /api/nodes/:id/setting | 获取节点插件配置（仅返回 { config }，对标 Neuron GET setting） |
| PUT | /api/nodes/:id/setting | 修改节点插件配置（对标 Neuron setting） |
| POST | /api/nodes/:id/read_tags | 南向按需读 Tag（对标 Neuron read_tag） |
| POST | /api/nodes/:id/write_tags | 南向写 Tag（对标 Neuron write_tag） |

---

## 六、配置与持久化

### 配置（环境变量）

| 变量 | 默认 | 说明 |
|------|------|------|
| `GATEWAY_PORT` | 3000 | HTTP 监听端口 |
| `GATEWAY_DATA_DIR` | data | 数据目录，持久化 `data.db`（SQLite） |
| `GATEWAY_STATIC_DIR` | web/dist | 前端静态资源根 |
| `GATEWAY_PLUGINS_DIR` | plugins | 插件 .so 目录（对标 Neuron plugins） |
| `GATEWAY_CONFIG` | （无） | 配置文件路径；缺省时尝试 `config/gateway.json` |
| `GATEWAY_DISABLE_AUTH` | 0 | 设为 1 或 true 关闭 API 认证 |
| `GATEWAY_TOKEN` | （无） | API Bearer Token；设置后除 /api/health、/api/metrics、/api/version 外需带 Authorization |
| `RUST_LOG` | info,tower_http=debug | 日志 filter |

### 持久化（对标 Neuron 配置与状态落盘，SQLite）

- **SQLite**：节点、组、标签、北向订阅写入 `data/data.db`；启动时加载，任意变更后自动保存。
- **Schema**：`meta`（version）、`nodes`、`groups`、`tags`、`subscriptions` 表；版本号便于后续迁移。
- **原子写入**：save 在单事务内替换全量数据，崩溃不产生半写。
- **迁移**：若 `data.db` 不存在且存在旧版 `data.json`，启动时自动从 JSON 加载并写入 DB，再按 DB 加载。
- **错误**：`persist_load` / `persist_save` 返回 `Result<..., PersistError>`（Io、Sqlite、Json、VersionUnsupported、Validation）；启动时加载失败会打 warn 并以空配置继续。
- **兼容**：`persist_load_json` 可从 `data.json` 读取（用于迁移或导出）。

### 节点状态

- 启停时更新 `Node.state`：`running` / `stopped`，并参与持久化。

---

## 七、运行与构建

```bash
# 后端
cargo build --release
./target/release/gateway   # 默认 0.0.0.0:3000

# 可选：覆盖配置
GATEWAY_PORT=8080 GATEWAY_DATA_DIR=./mydata ./target/release/gateway

# 前端
cd web && npm install && npm run build   # 产出 web/dist
# 开发时：npm run dev（Vite 代理 /api -> :3000）
```

网关会服务 `/api` 与静态资源 `web/dist`（含 SPA fallback）。

---

## 八、.so 插件（对标 Neuron）

1. **构建 .so**：  
   `cargo build -p gateway-plugin-sim -p gateway-plugin-mqtt --features ffi`  
   产出：`target/debug/libgateway_plugin_sim.so`、`libgateway_plugin_mqtt.so`。
2. **部署**：将 .so 放入 `plugins/`（或 `GATEWAY_PLUGINS_DIR` 所指目录）。网关启动时扫描 `*.so` 并加载；若目录不存在则使用内置 sim/mqtt。
3. **约定**：南向导出 `gateway_south_plugin_*`，北向导出 `gateway_north_plugin_*`；共享 `gateway_plugin_free_string`（由 gateway-sdk 提供）。详见 `gateway-sdk/ffi` 与各插件 `ffi` 模块。

## 九、后续可扩展

1. **MQTT 北向**：在 `gateway-plugin-mqtt` 中接入 `rumqttc`，按 topic 发布 GroupData。
2. **更多南向**：Modbus、OPC UA 等，按 `SouthPlugin` 实现即可；可同时提供 .so（`--features ffi`）与静态链接。
3. **JWT / 认证**：API 鉴权，对齐 Neuron 的认证能力。
