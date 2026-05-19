# IoT 网关破坏性重构 — Phase 3 & 4 实施计划

> **分支**: `redesign/v1-flow-orchestration`
>
> **目标**: Phase 3 完成剩余算子 + Flow 完整前端；Phase 4 实现生产化
>
> **前置依赖**: Phase 1（核心框架 ✅）+ Phase 2（协议插件 ✅）

---

## 一、Phase 3 定位调整说明

原始设计 Phase 3 = "高级算子（4-6周）"，但 Phase 2 Track 4 已实现 **16 个高级算子**，剩余未完成：

| 算子 | 状态 | 备注 |
|------|------|------|
| script-js | ✅ 已完成 | rhai JS 兼容模式 |
| alarm | ✅ 已完成 | 阈值/变化率/状态变化 |
| json-path | ✅ 已完成 | JSON 字段提取 |
| xml-path | ⬜ 未完成 | XML 解析（剩余唯一未完成高级算子） |
| throttle | ✅ 已完成 |  |
| cache | ✅ 已完成 |  |
| batch | ✅ 已完成 |  |
| script-python | ⬜ 未完成 | Python 沙箱（需要 pyoxidizer 或 inline-python） |

**Phase 3 重新定义**：剩余算子（xml-path / script-python）+ Flow 前端完善（节点面板 / 属性配置 / 部署管理 / 实时监控）+ South/North 节点配置界面

---

## 二、Phase 3 任务详情

### Track A: 剩余算子

#### Task A1: xml-path 算子（O35）

XML 路径算子，从 XML 字符串中提取字段。使用 quick-xml + xpath-rs。

配置字段：
- source_field: 输入 XML 字段名，默认 "xml"
- expressions: Vec<(output_field, xpath_expression)>
- namespaces: HashMap<prefix, namespace_uri>

#### Task A2: script-python 算子（O36）

Python 脚本算子，使用 inline-python（wasmer2 沙箱）执行 Python 代码。

配置字段：
- script: Python 代码片段
- input_fields: 注入的输入变量
- output_fields: 提取的输出变量
- timeout_ms: 执行超时，默认 1000ms

注意：inline-python 依赖 wasmer2，编译较慢。如集成困难，降级为 subprocess 模式。

---

### Track B: Flow 前端完善

#### Task B1: 节点选择面板（Node Palette）

在 FlowEditor.vue 左侧新增可折叠节点面板，展示所有可用 South/Operator/North 节点。

- South 节点：图标 🔌，颜色 #409EFF（蓝色）
- Operator 算子：图标 ⚙️，颜色 #67C23A（绿色）
- North 节点：图标 📤，颜色 #E6A23C（橙色）

从 /plugins 和 /flows/operators API 加载插件列表，支持拖拽到画布。

#### Task B2: 属性配置面板（Properties Panel）

选中节点后右侧滑出配置面板，使用 ElementPlus 动态表单。

- 调用 plugin.config_schema() 获取 JSON Schema
- 动态生成 ElementPlus 表单
- 支持 South / Operator / North 三类节点

#### Task B3: 部署管理（Deploy Panel）

FlowList 页面新增部署状态管理：

- 状态列：draft / validated / running / paused / failed
- 操作：deploy / pause / stop / delete
- 生命周期状态机实现

API：POST /flows/:id/deploy | /pause | /stop

#### Task B4: 运行时数据预览（Data Preview）

FlowEditor 中新增可折叠底部面板，实时显示节点输入/输出数据。

Phase 3 用轮询实现，WebSocket 在 Phase 4 预留。

---

### Track C: South/North 节点配置 UI

#### Task C1: South 点位管理

属性面板中新增"点位管理"Tab：

- 调用 plugin.list_groups() 获取分组
- 调用 plugin.list_tags() 获取点位
- 支持新增/编辑/删除 Tag
- 调用 plugin.validate_tag() 验证

#### Task C2: North 订阅管理

North 节点配置面板中新增"订阅管理"Tab：

- 列出当前 Flow 中的 South/Operator 节点
- 勾选需要订阅的节点
- 调用 plugin.set_subscriptions() 保存

---

## 三、Phase 4 任务详情

### Task D1: 多实例高可用

- Flow 实例级别锁（PostgreSQL advisory lock）
- 同一 Flow 只允许一个实例运行
- 健康检查端点 GET /health
- 实例注册表

### Task D2: Flow 版本管理

- Flow 变更历史记录，支持回滚
- version_history JSON 数组保存快照
- API: GET /flows/:id/versions, POST /flows/:id/rollback/:v

### Task D3: Flow 热重载

- POST /flows/:id/reload 重新加载 Flow 定义
- FlowExecutor 动态更新节点配置
- South/North 插件 reconnect

### Task D4: 监控与告警

- GET /metrics Prometheus 格式端点
- 指标：flow_running_total, flow_nodes_processed_total, south_plugin_read_ms 等
- 用户可配置告警规则

### Task D5: 配置导入/导出

- GET /flows/export?ids=id1,id2
- POST /flows/import

### Task D6: 备份与恢复

- POST /admin/backup, GET /admin/backup, POST /admin/restore
- SQLite VACUUM INTO 备份
- 保留最近 7 个备份

### Task D7: 性能压测

- gateway-bench binary
- Mock South/North 插件
- 输出: messages/sec, p50/p95/p99 延迟

---

## 四、Phase 3 & 4 实施顺序

Phase 3:
S1 → B1（节点面板）→ B2（属性面板）→ C1（South点位）→ C2（North订阅）
S2 → B3（部署管理）→ B4（数据预览）
S3 → A1（xml-path）
S4 → A2（script-python）

Phase 4:
D3（热重载）→ D4（监控）→ D2（版本管理）→ D1（多实例）→ D5（导入导出）→ D6（备份恢复）→ D7（压测）

---

## 五、验收标准

### Phase 3 验收
- [ ] xml-path 算子编译通过
- [ ] script-python 算子编译通过（或 stub 模式）
- [ ] FlowEditor 可拖拽节点到画布
- [ ] 属性面板支持 South/Operator/North 三类节点配置
- [ ] Flow 可部署/暂停/停止
- [ ] South 节点点位管理 UI 完成
- [ ] North 节点订阅管理 UI 完成
- [ ] npm run build 前端编译通过
- [ ] cargo build --workspace 全量编译通过

### Phase 4 验收
- [ ] /metrics Prometheus 端点返回正确格式
- [ ] Flow 版本历史可查看和回滚
- [ ] POST /flows/:id/reload 热重载生效
- [ ] 配置导入/导出正常工作
- [ ] SQLite 备份/恢复功能正常
- [ ] gateway-bench 压测工具可运行
- [ ] 多实例部署文档完成
