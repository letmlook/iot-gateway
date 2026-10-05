# 设计文档：API 现代化与实时推送（api-v1-openapi / ws-pinia）

> 状态：设计定稿（已按评审意见修订一轮：Swagger UI 鉴权方式、WS 白名单条目、心跳阈值单一化、行号勘正），待实施。
> 对应功能项：`api-v1-openapi`（/api/v1 前缀、列表分页、utoipa OpenAPI 与前端 TS 类型生成）、
> `ws-pinia`（WebSocket 实时推送 + Pinia 状态管理 + WS 客户端）。
> 对应 `docs/功能实现清单.md` 第八、九节的三条 🚧 条目（API 版本化/分页/OpenAPI、Pinia+WebSocket、OpenAPI 生成 TS 类型）。
> 本文自包含：实现者只读本文即可开工，无需追溯设计过程。所有行号以撰写时（2026-10-05）工作区代码为准。

---

## 一、目标与范围

### 1.1 目标

| 项 | id | 交付内容 |
|---|---|---|
| API 现代化 | `api-v1-openapi` | ① 所有现有 REST 接口在 `/api/v1` 前缀下提供规范别名，旧路径 `/api/*` 行为完全不变；② 无界集合端点分页（统一响应信封）；③ utoipa 生成 OpenAPI 文档 + Swagger UI + 独立导出命令；④ 前端由 OpenAPI 自动生成 TS 类型（`.d.ts`），以 JSDoc + `checkJs` 消费，CI 防契约漂移 |
| 实时推送 | `ws-pinia` | ① 后端 WebSocket 端点 `/api/v1/ws`：实时值推送（总线旁路）、节点状态快照周期推送、首消息鉴权；② 前端引入 Pinia，把 App.vue 的 provide/inject 全局数据收敛为 store；③ WS 客户端（自动重连、订阅过滤、断连回退轮询），DataMonitor 等页面接入 |

### 1.2 明确不在本设计范围（不做）

- **多租户**：维持形态 A（单租户），见 `docs/多租户决策方案.md`，本设计不引入 tenant 维度。
- **axum 0.7 → 0.8 升级**：工作区锁定 axum 0.7.9（`Cargo.lock`:112-114），升级是独立的破坏性变更，本设计所有选型以 axum 0.7 为前提。
- **前端整体迁移 TypeScript**：`web/src` 保持 JavaScript 运行时，只做「生成 `.d.ts` + JSDoc 类型标注 + 局部 `checkJs`」，理由见 3.3.5。
- **JWT / Cookie 会话**：沿用现有 Bearer token（localStorage）体系。
- **历史数据、数据流 per-tag 统计的分页**：`/history/series` 已有自己的 from/to/bucket 参数（`gateway-server/src/history.rs`），`/data-flow` 的 per_tag 列表属诊断页低频接口，均不纳入本次分页。
- **北向节点状态事件的实时总线**：gateway-core 目前没有「节点状态变更」事件源（见 3.4.4），本设计用服务端周期快照代替，不为此改造 core。

---

## 二、现状与衔接点

### 2.1 后端（gateway-server）

| 现状 | 位置 | 对本设计的影响 |
|---|---|---|
| 路由表集中在 `api::router()`，路径不带前缀，由 `main.rs` `nest("/api", ...)` 挂载 | `gateway/gateway-server/src/api/mod.rs:203-320`、`gateway/gateway-server/src/main.rs:289-293` | v1 别名在同一个 router 内以 `v1/...` 路径注册，见 3.1.2 |
| 认证+授权合并中间件；路径白名单（health/metrics/version/license/status/auth/login）；`required_role()` 按「剥掉 `/api` 前缀后的路径 + 方法」映射角色 | `api/mod.rs:104-201`、`api/mod.rs:51-89`；嵌套路由下 URI 可能带或不带 `/api` 前缀的说明在 `api/mod.rs:117-124` | **引入 `/api/v1` 必须先修路径归一化**，否则 `v1/users` 会绕过 `users` 的 Admin 规则（详见 3.1.3 与风险 R1） |
| 错误体 `{ code, message }` | `gateway/gateway-server/src/api/error.rs:20-63` | v1 错误体保持同形状，纳入 OpenAPI 组件 `Error` |
| 列表接口现状：`list_nodes` 裸数组（`handlers.rs:1003`）、`list_groups` 裸数组（`handlers.rs:1214`）、`list_tags` 裸数组（`handlers.rs:1320`）、`list_rules` 裸数组（`handlers.rs:2018`）、`list_users` 已有 `{ users: [...] }` 信封（`handlers.rs:172-189`） | `gateway/gateway-server/src/api/handlers.rs` | 分页只发生在 v1 别名上，旧路径不动（3.2） |
| **集合无稳定顺序**：`Store::nodes_list()` 直接迭代 `DashMap` | `gateway/gateway-core/src/store.rs:9-10, 40-42`（经 `manager.rs:306-308` 暴露） | 分页前必须显式排序，否则翻页会重复/漏项（3.2.3） |
| 总线旁路订阅现成可用：`Bus::subscribe_all() -> (tap_id, Receiver<Arc<GroupData>>)`；`HistoryRecorder` 已用同模式（订阅→select 循环→Lagged 计数→退出时 `unsubscribe_all`） | `gateway/gateway-core/src/bus.rs:79-98`；`gateway/gateway-server/src/history.rs:145-172` | WS 推送直接复用该模式，每连接一个 tap（3.4.5） |
| 实时值缓存：`Manager::last_values(nid) -> HashMap<TagId, LastValue>`，现有 `GET /nodes/:id/values`（`handlers.rs:2241-2272`）只读缓存不打设备 | `gateway/gateway-core/src/manager.rs:164-172` | WS `values` 帧与该 REST 响应字段语义对齐 |
| WebSocket 能力已就位：axum 0.7.9 已启用 `ws` feature | `gateway/gateway-server/Cargo.toml:18`（`features = ["json", "macros", "multipart", "tokio", "ws"]`） | 后端 WS **零新增运行时依赖** |
| CORS 默认拒绝跨域；`GATEWAY_ALLOWED_ORIGINS` 显式放行 | `main.rs:259-287` | CORS 不约束 WebSocket 握手，WS 需自查 Origin（3.4.3） |
| 优雅退出已有 SIGINT/SIGTERM 处理 | `main.rs:319-361` | WS 连接需在停机时被通知关闭（3.4.7） |
| 配置为 env > `config/gateway.json` > 默认值；默认端口 **3000** | `gateway/gateway-server/src/config.rs:145-147` | 注意：根目录 `CLAUDE.md` 写「默认 4000」与代码不符，实现时以 `config.rs` 的 3000 为准；前端 vite 代理默认 3000 一致（`web/vite.config.js:4`） |
| Prometheus 文本指标手工拼装 | `handlers.rs:641`（`metrics` 函数体） | 新增 WS 指标在此追加（3.4.8） |

### 2.2 前端（web/）

| 现状 | 位置 | 对本设计的影响 |
|---|---|---|
| 纯 JS（无 TS、无状态库），依赖：vue 3.5.24 / element-plus 2.13.1 / vue-router 4.6.4 / vue-i18n 11.2.8 | `web/package.json` | Pinia 4 与 vue 3.5 兼容（peer 已核实，见 3.5.1） |
| API 层单文件，`BASE = '/api'`，统一 401/403 处理与取消支持 | `web/src/api.js:1, 70-94` | 切到 v1 只需改 `BASE`；分页信封在调用点适配 |
| 全局数据靠 `App.vue` `provide('southPlugins'/'northPlugins'/'nodes'/'health')`，仅 `onMounted` 拉一次，**无任何周期刷新** | `web/src/App.vue:17-26, 84-113` | 改造为 Pinia store（3.5.2），顺带补齐「订阅推送后自动刷新」 |
| provide 的消费方：`CreateNode.vue:17-18`、`Dashboard.vue:15-16`、`NorthApps.vue:15`、`SouthDevices.vue:15`（共 4 个视图，`inject`） | `web/src/views/*` | 改动清单逐文件列出（3.6.2） |
| DataMonitor 以 1–10s 定时轮询 `nodeValues`（页面隐藏时暂停），`formatValue` 已兼容 `{type, value}` 形状的 DataValue | `web/src/views/DataMonitor.vue:121-135, 162-169, 171+` | WS 推送与此形状一致即可无缝替换刷新源 |
| vite dev 代理只代理 `/api`，未开 `ws` | `web/vite.config.js:30-35` | WS 经 dev 代理必须加 `ws: true`（3.5.4） |
| 登录态存 localStorage，`api.js` 统一清理并跳登录 | `web/src/api.js:47-61` | WS 首消息鉴权复用同一 token（3.4.3） |

### 2.3 依赖核实结果（本设计撰写时实际执行 `cargo info` / `npm view` / 干跑解析所得）

| 依赖 | 版本（已核实存在） | 结论 |
|---|---|---|
| axum | 0.7.9（已在 `Cargo.lock`） | `ws` feature 已启用，WS 无需新增依赖 |
| utoipa | **5.5.0**（`cargo info utoipa@5.5.0` 实测；6.0.0 已发布但过新） | 采用 5.5；features：`macros`（默认）、`axum_extras`、`chrono`、`uuid`（均实测存在） |
| utoipa-swagger-ui | **8.1.0**（8.x 最新；9.0.2 起要求 axum ^0.8，docs.rs 实测 8.0.3 依赖 axum ^0.7、utoipa ^5） | 采用 8.1，features `axum` + `vendored`；在临时工程中 `cargo tree` 实测与 axum 0.7.9、utoipa 5.5.0 单版本共存，`vendored` 解析到 `utoipa-swagger-ui-vendored 0.1.2`（离线构建，无运行时/构建期下载） |
| pinia | **4.0.3**（`npm view pinia version` 实测 latest） | peer 要求 `vue ^3.5.11`、`typescript >=5.6.0`，与仓库 vue 3.5.24+ 兼容 |
| openapi-typescript | **7.13.0** | 前端类型生成 CLI |
| typescript | **5.9.3**（5.x 线最新；7.0.2 已存在但为全新原生编译器线） | 仅作 devDependency 供 `checkJs`，锁 5.x 避免工具链噪音 |
| tokio-tungstenite | 0.2x（**未逐版本核实**） | 仅作为 gateway-server 的 **dev-dependency** 用于 WS 集成测试；实现时以能连 axum 0.7 ws 的最新 0.2x 为准 |

---

## 三、方案

### 3.1 `/api/v1` 前缀与旧路径兼容策略（api-v1-openapi ①）

#### 3.1.1 选型：单挂载 + v1 别名路由表（已定，不采用双 nest）

**采用**：保持 `main.rs` 现状 `nest("/api", api::router(...))` 不变；`api::router()` 内部改为「路由表」构建——每个路由声明一次，注册两次：

```
/nodes            → list_nodes        （旧行为：裸数组）
/v1/nodes         → list_nodes_v1     （新行为：分页信封 + DTO）
/nodes/:id/start  → start_node        （旧行为）
/v1/nodes/:id/start → start_node      （同 handler，逐字转发）
```

实现为一个小工具函数（伪码）：

```rust
fn route2(routes: Routes, path: &str, mr: MethodRouter<AppState>) -> Routes {
    routes.route(path, mr.clone()).route(format!("v1{}", path), mr)
}
```

5 个分页端点不用 `route2`，改用 `route(path, legacy_mr).route(v1_path, v1_mr)` 双 handler（见 3.2）；`/v1/ws`、`/v1/openapi.json`、`/v1/docs` 只注册 v1 版本。

**否决「`.nest("/api", r.clone()).nest("/api/v1", r)` 双挂载」的理由**：
1. 同一 router 双挂载依赖 axum 嵌套匹配的顺序/回退语义，`/api` nest 会先吃掉 `/api/v1/*` 前缀并在子路由 404，行为跨版本易变；
2. 分页端点新旧响应形状不同，同一 handler 无法同时表达两种信封，双挂载反而要引入 per-request 分支；
3. `serve_static_or_index` 的 `/api/*` 404 JSON 分支（`api/mod.rs:334-344`）对 `v1` 前缀天然生效，无需改动。

#### 3.1.2 兼容承诺

- 旧路径 `/api/*` 的**路径、方法、请求体、响应体逐字节不变**（含裸数组、`{ users: [...] }` 等），前端与外部集成方零破坏。
- `/api/v1/*` 与 `/api/*` 是同一 handler 的两扇门：鉴权、RBAC、限流、`x-request-id`、错误码全部一致。
- 旧路径**长期保留**（无 deprecation 时间表）：网关是自托管单体，双注册的维护成本为零（同一路由表）。`docs/架构说明.md` 的 API 速览表（第五节）补充「canonical 路径为 /api/v1」说明，由实施者随本次改动更新文档。

#### 3.1.3 路径归一化修复（必须最先做，独立成步）

现状 `trim_start_matches("/api/")` 链（`api/mod.rs:53-57` 与 `117-124` 两处各自实现）会把 `/api/v1/users` 归一成 `v1/users`，导致：
- `required_role()` 中 `p == "users"` 不命中 → 用户管理降级为普通 Viewer 可读（**越权**）；
- `auth_middleware` 白名单中 `path == "health"` 不命中 → `/api/v1/health` 变成 401（探针失效，fail-closed）。

修复：抽出一个统一函数并让两处共用：

```rust
/// 归一化 API 相对路径：接受 /api/v1/users、/api/users、v1/users、users 四种输入，统一返回 "users"
fn normalize_api_path(raw: &str) -> &str {
    let p = raw.trim_start_matches("/api/v1/")
              .trim_start_matches("/api/v1")
              .trim_start_matches("/api/")
              .trim_start_matches("/api")
              .trim_start_matches('/')
              .trim_end_matches('/');
    p.strip_prefix("v1/").unwrap_or(p)   // 兜底：裸 "v1/xxx" 输入
}
```

配套单测矩阵（纯逻辑，无网络）：对 `["/api/v1/users", "/api/users", "v1/users", "users"]` 四种输入，`required_role(GET, …) == Admin`、`required_role(POST, /api/v1/rules) == Admin`、白名单 `["/api/v1/health", "/api/health", "v1/health", "health"]` 全部放行。**这一步不依赖其他任何改动，先行合入。**

#### 3.1.4 文档与探针端点

`/api/v1/health`、`/api/v1/version`、`/api/v1/metrics` 均为别名注册；k8s/compose 探针可继续用 `/api/health`。

---

### 3.2 列表分页（api-v1-openapi ②）

#### 3.2.1 信封形状（v1 专用，全局唯一）

```jsonc
// GET /api/v1/nodes?page=2&page_size=50&kind=south&q=plc
{
  "items": [ /* NodeDto... */ ],
  "total": 137,        // 过滤后的总条数（非当前页）
  "page": 2,           // 1-based，回显规范化后的值
  "page_size": 50      // 回显 clamp 后的值
}
```

- 字段命名 snake_case，与现有 API（如 `node_values` 的 `tag_id`）一致。
- 不加 `has_next`（前端由 `page * page_size < total` 推导），不为凑字段加 `links`。

#### 3.2.2 查询参数

| 参数 | 类型 | 默认 | 约束 |
|---|---|---|---|
| `page` | int | 1 | `>= 1`，否则 400（`bad_request`） |
| `page_size` | int | 50 | clamp 到 `[1, 1000]`（超界取边界，不报错，响应回显实际值） |
| `q` | string | — | name 子串过滤（大小写不敏感），仅 nodes/groups/tags 提供 |
| `kind` | `south\|north` | — | 仅 nodes |
| `group_id` | uuid | — | 仅 tags（替代前端自行按组过滤） |

解析用 `axum::extract::Query<PageParams>`（serde `#[serde(default)]`），`Query` 提取失败时 axum 自带 400。

#### 3.2.3 排序（关键决策）

`DashMap` 迭代无序（`store.rs:40-42`），分页必须先排序。**v1 列表统一按 `name` 升序、同 name 按 `id` 字符串升序**（`Vec::sort_by`，内存集合排序开销可忽略）。排序只作用于 v1 分页 handler，旧路径维持现状（行为不变承诺）。

#### 3.2.4 纳入分页的端点（v1 别名）

| v1 端点 | 过滤参数 | 说明 |
|---|---|---|
| `GET /api/v1/nodes` | `page/page_size/kind/q` | 含 `connection_status` 与脱敏 config（沿用 `list_nodes` 逻辑） |
| `GET /api/v1/nodes/:id/groups` | `page/page_size/q` | 南向节点专属（保留 `ensure_node_south`） |
| `GET /api/v1/nodes/:id/tags` | `page/page_size/group_id/q` | 现实现是跨组 flat_map（`handlers.rs:1320-1333`），分页前先过滤后排序 |
| `GET /api/v1/rules` | `page/page_size` | 规则数量可增长（阈值规则常态配置） |
| `GET /api/v1/users` | `page/page_size` | 响应从 `{users:[...]}` 变为标准信封（v1 内不用旧信封） |

**不做分页**（保持全量返回，v1 别名仅是路径别名）：
- `/plugins/south`、`/plugins/north`（数量 = 已加载插件，个位数）；
- `/nodes/:id/subscriptions`（订阅表按定义有界且 UI 需要整表编辑）；
- `/nodes/:id/values`（监控页一次性消费整表；改进方向是后续加 `group_id` 过滤参数，不在本次）；
- `/history/*`（自带 from/to/bucket）、`/data-flow`（诊断页，低频）。

#### 3.2.5 实现

新增 `handlers_page.rs`（或并入 handlers.rs，由实施者定）：`PageParams`、`Page<T>`（serde 泛型）与 5 个 `*_v1` handler。handler 复用现有取数逻辑（`manager.nodes_list()` 等），追加 filter → sort → window → 计数。旧 handler 一行不改。

---

### 3.3 utoipa OpenAPI 与前端类型生成（api-v1-openapi ③④）

#### 3.3.1 DTO 层（不污染 SDK/Core）

**在 `gateway-sdk` / `gateway-core` 上不加 utoipa 依赖**（插件生态不应为文档买单）。在 gateway-server 新增 `api/dto.rs`，显式定义响应 DTO + `From` 映射：

| DTO | 来源 | 说明 |
|---|---|---|
| `NodeDto` | `Node`（`gateway-core/src/node.rs:20-26`）+ `connection_status` 字段 | 字段：`id: String, name, kind, plugin_name, state, config: Object, connection_status: Option<Object>` |
| `GroupDto` | `Group`（`gateway-sdk/src/types.rs:240-246`） | `id, name, interval_ms, description` |
| `TagDto` | `Tag`（`types.rs:211-222`） | `id, group_id, name, address, attr, data_type, description` |
| `RuleDto` | `RuleView`（`gateway-core/src/rules.rs:184-188`） | 展开 `rule`（`id,name,enabled,source,condition,for_ms,clear_ms,action`）+ `runtime: {fired,last_value,fire_count}` |
| `UserDto` | `handlers.rs:172-189` 现有 JSON 形状 | `id, username, role, created_at, updated_at` |
| `PluginDto` | `handlers.rs:869-913` 现有形状 | `name, description, version, name_zh, name_en, description_zh, description_en, licensed, is_free` |
| `ValueDto` | `node_values` 的元素（`handlers.rs:2252-2265`） | `tag_id, name, group_id, value: DataValueJson, ts, available` |
| `Error` | `ApiErrorBody`（`error.rs:20-24`） | 全局错误响应组件 |
| `Page<T>` | 3.2.1 | utoipa 泛型 schema，实例化为 `PageNode`/`PageGroup`/`PageTag`/`PageRule`/`PageUser` |
| WS 帧 DTO | 3.4.4 | 也定义在这里（`ToSchema`），保证 OpenAPI 与 WS 帧单测共用同一类型 |

`DataValue`、`connection_status`、插件 `config` 是动态 JSON（serde 形状见 `gateway-sdk/src/types.rs:16-32`，DataValue 为 `{"type":"Int32","value":5}` 式 tag/content 枚举）：DTO 中声明为 `#[schema(value_type = Object)]` 的强类型包装结构 `DataValueJson`，**如实标注为 object 而不是假装精确**。核心域（节点/组/点位/规则/用户/分页信封）是精确类型，这已覆盖前端主要消费面。

#### 3.3.2 ApiDoc 与导出

- `api/openapi.rs`：`#[derive(OpenApi)]` 的 `ApiDoc`，`paths(...)` 收集全部 v1 路径注解，`components(schemas(...))` 收集 DTO。
- 所有 v1 handler 加 `#[utoipa::path(...)]` 注解（旧 handler 不加）。`value_type = Object` 的请求/响应体（如 `/v1/logs/config`、`/v1/system/config`、`/v1/backup`）如实标注 object；这两个接口不新建 DTO（系统级低频接口，标注诚实即可）。
- **导出命令**：新增 bin `gateway-server/src/bin/export-openapi.rs`，`cargo run -p gateway-server --bin export-openapi` 把 `ApiDoc::openapi()` JSON 打到 stdout。供本地脚本与 CI 使用，不起服务即可生成。
- **Swagger UI**：`utoipa-swagger-ui 8.1`（features `axum`, `vendored`）挂载 `/v1/docs` + `/v1/openapi.json`，受 `enable_docs` 配置开关控制（3.3.4），**默认关闭**。
- **鉴权决策（已定）：`enable_docs=true` 时这两个路径加入 `auth_middleware` 白名单，置于认证之外**。理由：浏览器顶级导航与 vendored Swagger UI 内嵌 JS 发起的 `openapi.json` 请求都无法携带 `Authorization` 头——与本设计 3.4.2 论证 WS 不用 URL 传 token 是同一原理；若放在认证中间件之内，`/api/v1/docs` 在任何浏览器里都是 401，Swagger UI「浏览器调试」的唯一用途归零（形同虚设）。已否决的替代方案：① Swagger UI `requestInterceptor` 注入 Bearer——需要把 token 暴露进页面全局配置，复杂度高于收益；② 仅保留 `export-openapi` 命令、不提供 UI——丢弃交付物的一半价值。暴露面收敛靠两点：开关默认关闭 + openapi.json 只描述端点结构、不含任何业务数据，与风险表 R6 自洽。
- 挂载仍发生在 `api::router` 内（按 `state.config.enable_docs` 条件注册路由），但白名单放行使其实际不经过认证；实现若发现 `SwaggerUi::into_axum_router()` 与 `Router<AppState>` 状态类型不合，用 `.with_state(())` 转换后 merge。`required_role` 无需为 docs 增加条目（白名单路径在解析身份之前直接放行，与 health 同机制）。

#### 3.3.3 前端类型生成与防漂移

- 生成物：`web/src/types/api.d.ts`（openapi-typescript 7 输出 `components['schemas'][...]` 命名空间）。
- npm scripts（`web/package.json`）：

```jsonc
"gen:openapi": "cargo run -q -p gateway-server --bin export-openapi > ../openapi.json",
"gen:api": "npm run gen:openapi && openapi-typescript ../openapi.json -o src/types/api.d.ts",
"typecheck": "tsc -p jsconfig.json"
```

- `web/jsconfig.json`：`{ "compilerOptions": { "checkJs": true, "noEmit": true, "target": "ES2022", "module": "ESNext", "moduleResolution": "Bundler", "strict": false }, "include": ["src/api.js", "src/stores/**/*.js", "src/ws/**/*.js", "src/types/**/*.d.ts"] }`——**只对 API 层/store/WS 客户端开 checkJs**，17 个视图暂不纳入（避免一次性淹没在类型报错里）。
- `api.js` 渐进标注示例：

```js
/** @returns {Promise<import('./types/api').components['schemas']['PageNode']>} */
nodes: (params) => req('GET', `/v1/nodes${qs(params)}`),
```

- **CI 防漂移**（新增 job 步骤）：`cargo run -q -p gateway-server --bin export-openapi > openapi.json && cd web && npm run gen:api && git diff --exit-code -- src/types/api.d.ts`。后端改了契约而未重新生成 → CI 红。

#### 3.3.4 新增配置项（api-v1-openapi）

| 配置 | env / gateway.json | 默认 | 说明 |
|---|---|---|---|
| `enable_docs` | `GATEWAY_ENABLE_DOCS` | `false` | 开启 `/api/v1/docs` 与 `/api/v1/openapi.json`（跟随 config.rs 既有 env+json 模式）。开启时这两个路径（归一化后 `docs` / `openapi.json`）进入 `auth_middleware` 白名单，置于认证之外——见 3.3.2 的鉴权决策 |

#### 3.3.5 为什么前端不做 TS 迁移

仓库前端为纯 JS + 17 个视图，整体迁移是数倍工作量且与本两项无关。生成 `.d.ts` + JSDoc + checkJs 能获得同一份类型收益（编辑器提示 + CI 校验），运行时零改动，后续真要迁移 TS 时 `api.d.ts` 原样可用。

---

### 3.4 WebSocket 实时推送（ws-pinia ①，后端）

#### 3.4.1 端点

`GET /api/v1/ws`（仅 v1；无旧别名）。`api::router` 注册时从 `auth_middleware` 白名单中放行（原因见 3.4.2：握手请求无 `Authorization` 头，必被 401），**白名单条目写归一化后的路径 `"ws"`**——按 3.1.3 的 `normalize_api_path`，`/api/v1/ws`、`v1/ws`、`/api/ws` 都归一为 `"ws"`，白名单比较的是归一化结果（现状 `api/mod.rs:125-133` 即归一化后逐串比较），写成 `v1/ws` 会永远匹配不上、握手必然 401。`required_role` 对 WS 豁免（WS 鉴权在协议内完成）；`/api/ws` 虽被放行但无路由注册，落进 `serve_static_or_index` 的 `/api/*` 404 JSON 分支，无暴露面。

#### 3.4.2 鉴权（关键决策：首消息认证，不用 URL 传 token）

浏览器 `new WebSocket()` **不能携带 Authorization 头**；`?token=` 会进入 `TraceLayer` 访问日志（`main.rs:291`）与代理日志，造成凭据泄露。因此：

1. 握手阶段：仅校验 Origin（3.4.3）+ 连接数上限（3.4.6）+ HTTP 中间件白名单放行（白名单条目为归一化路径 `"ws"`，见 3.1.3 与 3.4.1；无 Authorization 头也会被 `auth_middleware` 401，故必须放行）。
2. 升级成功后 **10 秒内**客户端必须发送 `{"type":"auth","token":"<Bearer token>"}`；超时或首条消息非 auth → 关闭码 `4001 unauthorized`。
3. token 校验复用 HTTP 中间件同一段逻辑：静态 `GATEWAY_TOKEN` → Admin；用户 token → `user_store.role_of(&t)`（`api/mod.rs:155`）。把这段逻辑从 `auth_middleware` 中提为 `fn resolve_bearer(state, token) -> Option<AuthContext>` 供两处调用，避免两份实现漂移。
4. WS 是只读推送（不下发任何写操作），鉴权要求 **Viewer+**；`disable_auth=true` 时跳过 2-4 直接放行（与 HTTP 行为一致）。
5. 鉴权失败连接立即关闭；不做失败重试计数（每连接至多一次机会）。

#### 3.4.3 Origin 校验（CSWSH 防护）

握手时若请求带 `Origin` 头（浏览器必带）且非同源（与 `Host` 比较）且不在 `config.allowed_origins` → 403 拒绝握手。无 `Origin` 的非浏览器客户端放行（依赖 token 鉴权兜底）。CORS 配置不覆盖 WS（`main.rs:259-287`），故必须单独做。

#### 3.4.4 消息协议（v1-ws，帧格式全局唯一定义处）

传输：JSON 文本帧。信封：

```jsonc
// 服务器 → 客户端（除 pong 外均携带 ts，RFC3339 毫秒）
{ "type": "values", "ts": "2026-10-05T03:21:07.123Z", "data": { ... } }
// 客户端 → 服务器
{ "type": "subscribe", "topics": ["values","nodes"], "node_ids": ["<uuid>"], "group_ids": ["<uuid>"] }
```

| 方向 | type | data 形状 | 触发时机 |
|---|---|---|---|
| C→S | `auth` | `{ token: string }` | 连接后 10s 内必发，且必须是首条 |
| C→S | `subscribe` | `{ topics?: ["values"\|"nodes"], node_ids?: uuid[], group_ids?: uuid[] }` | 订阅/改订阅（覆盖式）；`node_ids/group_ids` 为空 = 全部 |
| C→S | `unsubscribe` | `{ topics: [...] }` | 停止某 topic |
| C→S | `ping` | `{}` | 应用层心跳 |
| S→C | `hello` | `{ version, build_date, features: ["values","nodes"] }` | auth 成功后立即发 |
| S→C | `values` | `{ node_id, node_name, group_id, group_name, values: [{ tag_id, tag_name, value: {"type":…,"value":…}, }] }` | 每条 GroupData（`gateway-sdk/src/messages.rs:8-24`），按订阅过滤后转发 |
| S→C | `nodes` | `{ nodes: [{ id, name, kind, plugin_name, state, connection_status? }] }` | 服务端每 `ws_snapshot_interval_ms`（默认 5000）对全量客户端推送一次快照 |
| S→C | `pong` | `{}` | 响应 ping |
| S→C | `error` | `{ code, message }` | 协议错误（未知 type、非法 JSON、越权 topics） |
| S→C | — | — | 关闭码：`4001` 未认证/认证失败、`4002` 服务端停机、`1008` Origin 拒绝（握手层 403） |

**`values.value` 的编码与 REST 完全一致**：`DataValue` 的 serde 形状 `{"type":"Int32","value":5}`（`gateway-sdk/src/types.rs:16-32`），`DataMonitor.formatValue` 已兼容该形状（`DataMonitor.vue:171-180`），前端零适配。

**为什么没有节点状态事件帧**：gateway-core 没有状态变更事件源（状态是 `Store` 里的一个字段，`store.rs:44-48`）。为其加事件机制要动 core 与全部启停路径，超出本项范围；周期快照（默认 5s）对「仪表盘状态刷新」足够，且实现者可把 `nodes` 推送间隔配成 0 关闭。此为**如实的能力边界**，不是推送延迟承诺。

#### 3.4.5 数据源与任务结构

新增 `gateway-server/src/ws.rs`，模式对标 `HistoryRecorder` 的旁路订阅（`history.rs:145-172`）：

```
每个 WS 连接 = 1 个 tokio task：
  tap = manager.bus().subscribe_all()          // 独立 tap，bus.rs:82
  loop select:
    tap_rx.recv()  → Ok(GroupData)：按本连接过滤器过滤 → 序列化 values 帧 → out_tx.send（有界 256）
                     Err(Lagged(n))：累加 dropped_lagged，继续（不关连接）
    out_rx.recv()  → ws.send(Text(frame))；通道满时丢帧并 ws_dropped_frames+1（推送是监控数据，可丢；绝不反压采集链路）
    心跳 interval   → 60s 无任何入站消息 → 关闭（1000）   // 唯一取值，见 3.4.6：客户端 25s 一跳，60s = 2×间隔+抖动余量
    shutdown watch → 关闭（4002）
    客户端消息      → auth 前只收 auth；subscribe 更新过滤器
连接结束 → bus.unsubscribe_all(tap_id)
```

- 过滤器：`HashSet<(NodeId, GroupId)>`（按 subscribe 消息重建）；topic 关闭则跳过对应分支。
- 序列化在连接 task 内完成（`tag_names` 映射已在 `GroupData` 中，`gateway-sdk/src/messages.rs:21-23`，无需反查 store；`node_name/group_name` 同理）。GroupData 是 `Arc` 广播，克隆廉价。
- 不做集中 Hub：每连接独立 tap 即天然扇出，不引入共享背压点。

#### 3.4.6 连接与资源保护

| 保护 | 值 | 说明 |
|---|---|---|
| 并发连接上限 | `ws_max_clients`，默认 **64** | 满时握手直接 503/关闭；防 FD 耗尽 |
| 出站通道 | mpsc 有界 **256** 帧 | 满则丢帧 + `gateway_ws_frames_dropped_total` 计数 |
| 首消息超时 | **10s** | 见 3.4.2 |
| 应用层心跳 | 客户端 `ping` ≤25s 一次（3.5.3 取 25s）；服务端 **60s** 无入站 → 关闭（全文档唯一取值，3.4.5 伪码同步） | 穿透 NAT/代理空闲回收 |

#### 3.4.7 优雅停机

`AppState` 增加 `ws_shutdown: tokio::sync::watch::Sender<bool>`（或等价 Notify）；`shutdown_signal()`（`main.rs:319`）停机序列中先广播关闭（发 `4002` Close 帧再断），再执行现有节点停止与 flush。WS 不阻塞停机：所有连接 task 在 watch 触发后立即退出。

#### 3.4.8 可观测

`/api/metrics`（`handlers.rs:641` 起的手工文本）追加：

- `gateway_ws_clients`（gauge，当前连接数）
- `gateway_ws_frames_sent_total`（counter）
- `gateway_ws_frames_dropped_total`（counter，含通道满与 Lagged）

新增配置项（ws-pinia）：

| 配置 | env / gateway.json | 默认 |
|---|---|---|
| `ws_max_clients` | `GATEWAY_WS_MAX_CLIENTS` | `64` |
| `ws_snapshot_interval_ms` | `GATEWAY_WS_SNAPSHOT_INTERVAL_MS` | `5000`（0 = 关闭 nodes 周期推送） |

---

### 3.5 Pinia + WS 客户端（ws-pinia ②③，前端）

#### 3.5.1 依赖与初始化

- `npm i pinia@^4.0.3`；`npm i -D typescript@~5.9.3 openapi-typescript@^7.13.0`（typescript 仅类型检查用）。
- `web/src/main.js`：`app.use(createPinia())`（在 `app.use(router)` 之前）。

#### 3.5.2 三个 store（`web/src/stores/`）

| store | 状态 | 行为 | 替代现状 |
|---|---|---|---|
| `auth.js` | `token, username, authenticated` | `login/logout/clearAuth`（逻辑自 `api.js:97-132` 迁入，api.js 保留纯传输函数）；401 统一跳转逻辑保留在 api.js | localStorage 散落读写 |
| `gateway.js` | `nodes, southPlugins, northPlugins, health, lastRefresh` | `loadInit()`（自 `App.vue:84-101` 迁入）；`applyNodesSnapshot(list)`（被 WS `nodes` 帧调用）；getter `runningCount/totalCount/southCount/northCount` | `App.vue` provide（`App.vue:17-26`） |
| `monitor.js` | `tagValues: { [tag_id]: value }, lastPushAt, wsState` | `applyValuesFrame(frame)`、`setConnected(bool)`、`reset(nodeId)` | DataMonitor 本地 `tagValues`（`DataMonitor.vue:121-135`） |

WS 接入后：`gateway.nodes` 由 5s 快照帧自动维护（登录后任何页面都新鲜），`monitor.tagValues` 由 `values` 帧维护。

#### 3.5.3 WS 客户端（`web/src/ws/gatewayWs.js`）

单例类，职责：

- 连接 `ws(s)://{location.host}/api/v1/ws`（生产同源；dev 走 vite 代理）。
- 首条消息发 `auth`（token 取自 `auth` store / localStorage），10s 内等 `hello`，失败按重连处理。
- **重连退避**：1s 起指数退避至 30s 封顶，成功后重置；`visibilitychange` 恢复可见时若已断则立即重连。
- 自动重发 `subscribe`（记住最后订阅：`topics + node_ids + group_ids`）。
- 入站分派：`values → monitor.applyValuesFrame`；`nodes → gateway.applyNodesSnapshot`；`pong/error` 记日志。
- 暴露 `state: 'connecting'|'open'|'closed'|'error'`（ref）供 UI 显示实时链路状态徽标。
- 应用层 `ping` 每 25s。

#### 3.5.4 vite dev 代理

`web/vite.config.js` 代理增加 `ws: true`（`/api` 条目内），否则 dev 模式 WS 握手不过代理：

```js
proxy: { '/api': { target: `http://127.0.0.1:${BACKEND_PORT}`, changeOrigin: true, ws: true } }
```

#### 3.5.5 DataMonitor 接入与回退（关键决策）

- `readValues()` 轮询逻辑**保留不删**；新增规则：WS 处于 `open` 且订阅了当前节点时，跳过定时器（WS 为准）；WS 断开/出错时自动恢复现有 `setInterval` 轮询。即「WS 主、轮询备」，网络环境不支持 WS（老代理、企业网关）时页面行为与今天完全一致。
- 订阅过滤随 `selectedNode` 变化重发 subscribe；切换节点时 `monitor.reset(nodeId)` 清空旧值避免串台。
- 手动「立即读取设备」（`readTags`）行为不变。

#### 3.5.6 其他页面

- `App.vue` 删除 4 个 provide，改用 `storeToRefs(gatewayStore)`；`runningCount/totalCount` 改为 getter。
- `SidebarNav` / `StatusBar` 拿的 props 不变（App.vue 继续传，数据源换成 store），组件零改动。
- SouthDevices / NorthApps / Dashboard / CreateNode：`inject(...)` 各两行替换为 `storeToRefs`。列表页（SouthDevices/NorthApps）因 `nodes` 快照自动刷新，无需各写轮询。
- `DataFlowMetrics.vue`、`SystemInfo.vue` 等维持 HTTP 轮询（低频诊断页，明确不在范围）。

---

### 3.6 改动清单

#### 3.6.1 后端（gateway-server）

| 文件 | 改动 |
|---|---|
| `Cargo.toml` | +`utoipa = { version = "5.5", features = ["axum_extras", "chrono", "uuid"] }`；+`utoipa-swagger-ui = { version = "8.1", features = ["axum", "vendored"] }`；dev-deps +`tokio-tungstenite`（版本实现时核实）、`tower` 已有 |
| `src/api/mod.rs` | `normalize_api_path()` 统一归一化并替换两处 trim 链（3.1.3）；路由表化 + v1 别名注册；白名单加入归一化路径 `ws`（3.4.1）与条件条目 `docs`/`openapi.json`（仅 `enable_docs=true` 时放行，3.3.2）；挂 Swagger UI（受 `enable_docs`） |
| `src/api/error.rs` | `ApiErrorBody` 加 `#[derive(utoipa::ToSchema)]`（纯派生，无行为变化） |
| `src/api/dto.rs`（新） | 3.3.1 全部 DTO + From 映射 + `Page<T>` |
| `src/api/openapi.rs`（新） | `ApiDoc` 聚合 |
| `src/api/handlers_page.rs`（新） | `PageParams` + 5 个 `*_v1` 分页 handler |
| `src/api/handlers.rs` | v1 handler 加 `#[utoipa::path]` 注解；`resolve_bearer()` 提取（自 auth_middleware）；metrics 追加 WS 指标 |
| `src/ws.rs`（新） | 3.4.4–3.4.7 全部：协议类型（serde + ToSchema）、连接 task、过滤器、心跳、停机 watch、连接计数 |
| `src/state.rs` | +`ws_shutdown` watch、WS 连接计数器 |
| `src/config.rs` | +`enable_docs` / `ws_max_clients` / `ws_snapshot_interval_ms`（env + gateway.json 双通道，仿现有字段） |
| `src/main.rs` | 启动 WS 相关（watch 通道创建）；停机序列先关 WS |
| `src/bin/export-openapi.rs`（新） | stdout 打印 openapi JSON |
| `docs/架构说明.md` | API 速览表补 v1 说明与 WS 端点（实施者随代码更新） |

#### 3.6.2 前端（web/）

| 文件 | 改动 |
|---|---|
| `package.json` | +pinia、-D typescript、-D openapi-typescript；scripts +`gen:api`/`typecheck` |
| `jsconfig.json`（新） | 3.3.3 |
| `src/main.js` | `app.use(createPinia())` |
| `src/api.js` | `BASE = '/api/v1'`；`nodes/groups/tags/rules/users` 适配分页信封（解 `items`）；auth 相关纯函数保留 |
| `src/stores/auth.js`、`gateway.js`、`monitor.js`（新） | 3.5.2 |
| `src/ws/gatewayWs.js`（新） | 3.5.3 |
| `src/App.vue` | 删 provide → store；`loadInitData` → `gatewayStore.loadInit()`；WS 客户端在主布局挂载后 connect |
| `src/views/CreateNode.vue:17-18`、`Dashboard.vue:15-16`、`NorthApps.vue:15`、`SouthDevices.vue:15` | `inject` → `storeToRefs(gatewayStore)` |
| `src/views/DataMonitor.vue` | 3.5.5：WS 订阅 + 轮询回退 + 链路状态徽标 |
| `src/types/api.d.ts`（生成物，提交入库） | `npm run gen:api` 产物 |
| `vite.config.js` | 代理 `ws: true` |
| `src/locales/zh.js` / `en.js` | WS 状态徽标、分页控件文案 |

---

## 四、测试策略

### 4.1 纯逻辑单测（CI 可跑，无需真实设备/服务）

`cargo test -p gateway-server`（沿用现有 oneshot 打 Router 的测试风格，`api/mod.rs:432-449`）：

1. **normalize 矩阵**：3.1.3 的四输入等价断言（含 `/api/v1/users` 必须仍是 Admin、`/api/v1/health` 必须放行）——这是防越权的关键测试。
2. **v1 别名等价性**：抽 3 个代表端点（`/v1/nodes/:id/start`、`/v1/plugins/south`、`/v1/health`）断言与旧路径状态码/角色一致。
3. **分页**：`PageParams` 边界（page=0 → 400；page_size=0 → clamp 1；>1000 → clamp）；构造 7 个节点断言 `page=2&page_size=3` 的 items 精确集合与 `total=7`；断言排序稳定（name 同名按 id）；`kind=q` 过滤正确。
4. **DTO 序列化**：NodeDto 等对现有 JSON 形状做 golden 对比（确保 From 映射不丢字段，尤其 `connection_status` 与脱敏 config）。
5. **openapi 导出**：`export-openapi` 输出可被 serde_json 解析且 `paths` 非空、含 `/v1/nodes`。
6. **WS 协议纯逻辑**：帧 serde golden（每个 type 一条样例 JSON 字符串断言，防字段改名）；过滤器（node/group 集合语义）；auth 状态机（10s 超时、首条非 auth 拒绝）——超时用 tokio pause 时间测试。
7. **WS 通道背压**：out 通道塞满后继续 send 不阻塞、dropped 计数增长。

WS **连接级**集成测试（真实 TCP listener + `tokio-tungstenite` dev-dep：握手→auth→收到 sim 节点 values 帧→停机收到 4002）依赖真实网络栈，需在 ephemeral 端口起真实 listener。落位：**新建 `gateway-server/tests/` 目录写 `tests/ws_e2e.rs`**——该 crate 目前没有 tests/ 目录（现有测试均为 `src` 内 `#[cfg(test)]` 模块，如 `api/mod.rs:395` 起），集成测试的组织方式参照 **`gateway-core/tests/`**（同仓库另一个 crate，现有 `ffi_fault_isolation.rs` / `node_setting.rs` / `poll_scheduler.rs` / `process_isolation.rs` / `rule_engine.rs` 五个文件），不能照抄成本 crate 内部路径。

### 4.2 真机 / 真实服务验证（本地起网关执行）

- `cargo run -p gateway-server --bin gateway` + 浏览器登录管理台：Swagger UI 开启后访问 `/api/v1/docs`，逐条调试 v1 接口。
- `npm run gen:api` 全流程；改一个后端字段验证 CI 漂移检查变红。
- WS 端到端：创建 sim 南向节点并启动（interval 500ms），DataMonitor 选中节点，确认 values 秒级推送（非 1s 轮询节奏）；杀后端验证前端自动重连与轮询回退无缝切换；开两个浏览器标签互不串值（过滤正确）。
- 分页真机验证：导入数百点位（`tags/batch`）后翻页顺序与 total 正确。
- RBAC 真机验证：Viewer 登录访问 `/api/v1/users` 必须 403（3.1.3 回归）。

### 4.3 明确不验证

- 不做全量契约模糊测试（schemathesis 类）；不为 WS 做压测基线（`gateway-bench` 不扩展；64 连接上限远低于风险区）。
- 不在无设备环境验证 modbus/opcua 相关 v1 接口语义——它们只是路径别名，行为与旧路径同源。

---

## 五、风险与回退

| # | 风险 | 影响 | 缓解 | 回退 |
|---|---|---|---|---|
| R1 | v1 别名使 `required_role`/白名单匹配失效导致越权（`v1/users` 降为 Viewer 可读） | 高（安全） | 3.1.3 独立先行合入 + 四输入等价矩阵单测；review 必看该测试 | 该步独立 PR，revert 即回到现状 |
| R2 | DashMap 无序 + 分页 = 翻页重复/漏项 | 中 | 3.2.3 强制排序；单测断言精确窗口 | 分页仅存在于 v1 新 handler，旧路径从未受影响 |
| R3 | utoipa 宏派生与现有 serde 属性冲突（flatten、transparent Uuid） | 中（编译期） | DTO 层隔离（SDK/Core 不派生），flatten 的 `Node`/`RuleView` 在 DTO 中显式展开字段；失败面只在 dto.rs | 删除 `#[utoipa::path]` 注解与 ApiDoc 即回退，运行时代码不受影响 |
| R4 | WS 高频帧（多节点×高频率组）压垮浏览器 | 中 | 3.4.6 有界通道丢帧 + 客户端默认只订阅当前页面所需节点；`ws_snapshot_interval_ms=0` 可关快照 | 前端不 connect 即回到纯轮询；后端无客户端时 WS 零开销 |
| R5 | WS 中间件白名单放行后鉴权疏漏 | 高（安全） | 白名单只放行归一化路径 `ws`（`/api/ws` 无路由注册，404 兜底）；握手仍有 Origin 检查 + 连接上限；协议层 token 校验复用 `resolve_bearer` 单一实现并单测；关闭码 4001 | 删除该白名单条目即恢复 fail-closed（WS 功能随之不可用） |
| R6 | Swagger UI / openapi.json 暴露端点拓扑（开启后置于认证之外，匿名可读） | 低 | `enable_docs` 默认 false；开关语义已写入 3.3.2/3.3.4：openapi.json 只含端点结构、不含业务数据，生产不开 | 无需回退（默认关；开启即已知情接受暴露面） |
| R7 | Pinia 迁移破坏 17 视图中的隐式依赖 | 中 | provide 消费方仅 4 个视图（已 grep 确认）；store 与 provide 数据形状一一对应；view 改动均为两行 inject→storeToRefs | store 层独立 PR；回退保留 App.vue provide 版本 |
| R8 | dev 代理 WS 不通导致「WS 坏了」误报 | 低 | vite `ws: true` 写入本设计；WS 客户端有轮询回退兜底（3.5.5） | — |
| R9 | 契约漂移（后端改字段、前端 d.ts 过期） | 中 | CI `git diff --exit-code` 门禁（3.3.3）；WS 帧由 golden 单测锁定 | — |

**总体回退策略**：两项都是「增量路由 + 新增文件」形态。`api-v1-openapi` 回退 = 删 v1 别名注册与文档相关文件（旧路径从未变更）；`ws-pinia` 后端回退 = 摘除 `/v1/ws` 注册（前端自动回退轮询），前端回退 = store PR 单独 revert。

---

## 六、分步实施顺序

按依赖关系排序；每步可独立合入并通过现有 CI（fmt/clippy/test 全强制）。

| 步 | 内容 | 对应 id | 前置 |
|---|---|---|---|
| S1 | `normalize_api_path()` 统一路径归一化 + 等价矩阵单测（3.1.3）；`resolve_bearer()` 提取 | api-v1-openapi | 无 |
| S2 | `api/dto.rs` DTO + `Page<T>` + 5 个 v1 分页 handler + 排序/过滤/clamp 单测（3.2） | api-v1-openapi | S1 |
| S3 | 路由表化 + 全量 v1 别名注册 + 别名等价性单测；前端 `BASE='/api/v1'` + api.js 分页信封适配（3.1/3.6） | api-v1-openapi | S2 |
| S4 | utoipa 注解 + `ApiDoc` + `export-openapi` bin + Swagger UI（`enable_docs`）+ openapi 单测（3.3） | api-v1-openapi | S2 |
| S5 | 前端 `gen:api`/`jsconfig`/`typecheck`/CI 漂移门禁 + `api.d.ts` 入库（3.3.3） | api-v1-openapi | S4 |
| S6 | `ws.rs` 协议类型 + 连接 task + 鉴权/Origin/上限/心跳/停机 + metrics + 纯逻辑单测与 WS e2e 集成测试（3.4） | ws-pinia | S1（resolve_bearer） |
| S7 | 前端 pinia 引入 + auth/gateway store + App.vue 与 4 视图去 provide（3.5.1/3.5.2/3.5.6） | ws-pinia | 无（可与 S6 并行） |
| S8 | `gatewayWs.js` + vite `ws:true` + monitor store + DataMonitor WS 接入与轮询回退（3.5.3–3.5.5） | ws-pinia | S6、S7 |
| S9 | 真机联调（4.2 清单）+ 更新 `docs/架构说明.md` API 速览、`docs/功能实现清单.md` 两条目置 ✅ | api-v1-openapi / ws-pinia | S5、S8 |

> 排期提示：S1→S5 为 `api-v1-openapi` 主线；S7 可与 S1–S6 并行；S8 是唯一同时依赖两条线的合流点。多租户（形态 B/C）若未来启动，`normalize_api_path` 是注入 tenant 过滤的天然收口点（对应 `docs/多租户决策方案.md` B3）。
