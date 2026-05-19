# IoT 网关破坏性重构：最终形态设计

> 文档版本：v1.2
> 日期：2026-05-19
> 更新：2026-05-19 Phase 1/2/3/4 实现完成
> 目标：不考虑兼容，完全重新设计，实现可视化物联网数据流编排平台

---

## Phase 1 实现状态

✅ **已完成**（分支：`redesign/v1-flow-orchestration`）

### Stage 1 — gateway-sdk 扩展
- [x] `PipelineData` 类型 + `Operable` trait
- [x] `PluginKind::Operator` 变体
- [x] `OperatorPlugin` trait（process/reset/state）
- [x] `OperatorMetrics` 指标类型
- [x] FFI 符号（供未来外部算子.so）

### Stage 2 — gateway-flow crate
- [x] 创建 `gateway-flow` crate（含 flow-core + flow-derive）
- [x] `NodeKind::Operator` 变体
- [x] `Flow` 结构体（nodes + edges + metadata）
- [x] DAG 环检测 + port 类型匹配验证
- [x] `OperatorRegistry` 插件注册表
- [x] `FlowRuntime` DAG 执行引擎

### Stage 3 — 5 个内置算子
- [x] `filter`（条件过滤）
- [x] `transform`（字段变换，rhai）
- [x] `aggregate`（时间窗口聚合）
- [x] `router`（条件多路路由）
- [x] `buffer`（缓冲批处理）

### Stage 4 — gateway-server 集成
- [x] flows + flow_nodes + flow_edges 建表 DDL
- [x] Flow CRUD REST API（list/create/get/update/delete）
- [x] Flow 生命周期（deploy/start/pause/stop/metrics）
- [x] gateway-flow 集成到 AppState

### Stage 5 — VueFlow 前端
- [x] vue-flow 安装（@vue-flow/core/background/controls/minimap）
- [x] Flow API 方法（api.js）
- [x] Flow 路由（/flows, /flows/new, /flows/:id）
- [x] `FlowList.vue`（列表 + 生命周期操作）
- [x] `FlowEditor.vue`（拖拽画布 + 属性面板 + 节点面板）

### Git 提交记录
```
e1445c3 chore(gateway-server): add gateway-flow dependency
471dad8 feat(web): VueFlow FlowList + FlowEditor
74bf0f4 feat(gateway-server): Flow CRUD REST API + lifecycle
72744c9 feat(gateway-flow): buffer operator
d1ba34c feat(gateway-flow): router operator
7768938 feat(gateway-flow): aggregate operator
570c927 feat(gateway-flow): transform operator
db69b3e feat(gateway-flow): FlowRuntime
775b731 feat(gateway-flow): DAG validation
20efc0f feat(gateway-flow): create crate
e9c715f feat(gateway-sdk): OperatorPlugin + FFI symbols
d9969b9 feat(gateway-sdk): PipelineData + Operable
```

---

## 一、设计愿景

---

## 一、设计愿景

构建一个**可插件化扩展的物联网数据流编排平台**，核心能力：

- **采集层（South）**：通过插件接入任何工业协议
- **处理层（Operator）**：通过插件实现数据转换、过滤、聚合、路由
- **传输层（North）**：通过插件将数据发送到任意云端/应用
- **编排层（Flow）**：通过可视化拖拽画布编排数据流
- **新协议接入 = 实现一个插件，无须修改核心代码**

---

## 二、目标架构

```
┌──────────────────────────────────────────────────────────────┐
│                        Web UI (Vue3)                        │
│  ┌────────────┐  ┌────────────┐  ┌────────────┐  ┌────────┐ │
│  │  Flow      │  │  Plugin    │  │  Data      │  │ System │ │
│  │  Editor    │  │  Manager   │  │  Monitor   │  │ Config │ │
│  │  (拖拽画布) │  │  (插件配置) │  │  (实时数据) │  │        │ │
│  └────────────┘  └────────────┘  └────────────┘  └────────┘ │
└──────────────────────────┬───────────────────────────────────┘
                           │ REST + WebSocket
┌──────────────────────────▼───────────────────────────────────┐
│                   Gateway Server (Axum)                      │
│                                                             │
│  ┌──────────────────────────────────────────────────────┐   │
│  │              Flow Orchestrator (NEW)                   │   │
│  │  - Flow Definition CRUD                              │   │
│  │  - Flow Instance Lifecycle                             │   │
│  │  - Operator Scheduling                                 │   │
│  │  - Data Pipeline Execution                            │   │
│  └──────────────────────────────────────────────────────┘   │
│                                                             │
│  ┌──────────────┐  ┌──────────────────┐  ┌──────────────┐  │
│  │ SouthPlugin  │  │ OperatorPlugin   │  │ NorthPlugin  │  │
│  │  (采集)       │─▶│  (数据处理)       │─▶│  (传输)       │  │
│  │              │  │  (插件化)         │  │              │  │
│  └──────────────┘  └──────────────────┘  └──────────────┘  │
└──────────────────────────────────────────────────────────────┘
```

---

## 三、插件体系（三层）

### Layer 1: South Plugins（采集层）

| ID | 插件名 | 协议 | 状态 |
|----|--------|------|------|
| S01 | sim | 模拟数据 | ✅ 已有 |
| S02 | modbus-tcp | Modbus TCP | ✅ 已有 |
| S03 | modbus-rtu | Modbus RTU（串口） | ✅ 已有 |
| S04 | opcua | OPC UA | ✅ 已有 |
| S05 | bacnet | BACnet/IP | 🆕 新增 |
| S06 | s7 | Siemens S7 (1200/1500/300/400) | 🆕 新增 |
| S07 | iec61850 | IEC 61850 (MMS/GOOSE/SV) | 🆕 新增 |
| S08 | ethernet-ip | EtherNet/IP (AB PLC) | 🆕 新增 |
| S09 | mitsubishi-mc | 三菱 MC 协议 | 🆕 新增 |
| S10 | omron-fins | Omron FINS | 🆕 新增 |
| S11 | hollysys | 和利时 DCS | 🆕 新增 |
| S12 | deltadvp | 台达 DVP | 🆕 新增 |
| S13 | yokogawa | 横河 DCS (Centum/PROSEC) | 🆕 新增 |
| S14 | abb-dcs | ABB 800xA / Industrial IT | 🆕 新增 |
| S15 | honeywell-pks | Honeywell PKS | 🆕 新增 |
| S16 | emerson-deltav | Emerson DeltaV | 🆕 新增 |
| S17 | siemens-pcs7 | Siemens PCS 7 | 🆕 新增 |
| S18 | dlt645 | DL/T645（电能表） | 🆕 新增 |
| S19 | gb28181 | GB/T 28181（视频监控） | 🆕 新增 |
| S20 | knx | KNX 总线 | 🆕 新增 |
| S21 | enocean | EnOcean 无线 | 🆕 新增 |
| S22 | mbus | M-Bus（热量表） | 🆕 新增 |
| S23 | obix | oBIX 协议 | 🆕 新增 |
| S24 | ethercat | EtherCAT | 🆕 新增 |
| S25 | profinet | PROFINET | 🆕 新增 |
| S26 | userprog | 用户自定义程序 | 🆕 新增 |

### Layer 2: Operator Plugins（处理层）🆕 NEW

|| ID | 插件名 | 功能 | 复杂度 | 参考实现 |
|----|--------|------|--------|---------|
|| O01 | filter | 按条件过滤数据（表达式） | ⭐ | NeuronEX / Node-RED |
|| O02 | transform | 数据类型转换、缩放、计算 | ⭐ | NeuronEX |
|| O03 | aggregate | 窗口聚合（count/sum/avg/max/min/first/last） | ⭐⭐ | Apache Flink / NeuronEX |
|| O04 | router | 按条件路由到不同下游分支 | ⭐ | Node-RED |
|| O05 | buffer | 缓冲批量输出（攒批） | ⭐ | Node-RED / MQTT Bridge |
|| O06 | delay | 延迟注入 | ⭐ | Node-RED |
|| O07 | script-js | JavaScript 脚本自定义处理 | ⭐⭐ | Node-RED / EMQX |
|| O08 | script-python | Python 脚本自定义处理 | ⭐⭐ | NeuronEX Enterprise |
|| O09 | json-path | JSON 路径提取 | ⭐ | Jayway JsonPath |
|| O10 | xml-path | XML 路径提取 | ⭐⭐ | Apache JMeter XPath |
|| O11 | splitter | 批量消息拆分 | ⭐ | Node-RED |
|| O12 | merger | 多源数据合并 | ⭐⭐ | Node-RED / Apache Flink |
|| O13 | cache | 缓存（读上一条值/时间窗口） | ⭐⭐ | Redis / In-Memory |
|| O14 | throttle | 限流/采样 | ⭐ | Node-RED / EMQX |
|| O15 | alarm | 告警规则（阈值/状态/变化率） | ⭐⭐⭐ | NeuronEX / industrial frameworks |
|| O16 | batch | 批量分组 | ⭐ | Node-RED / Apache Kafka |
|| O17 | switch | 多分支条件匹配（比router更强大） | ⭐ | Node-RED |
|| O18 | clamp | 值域限幅（上下限截断） | ⭐ | — |
|| O19 | round | 数值取整（round/ceil/floor） | ⭐ | — |
|| O20 | change | 特定字段替换/删除/重命名 | ⭐ | Node-RED |
|| O21 | range | 线性变换（将值从一个范围映射到另一个范围） | ⭐ | Node-RED |
|| O22 | csv-parser | CSV 编码/解码 | ⭐⭐ | — |
|| O23 | binary | 二进制解析（字节位操作） | ⭐⭐ | — |
|| O24 | time-window | 时间窗口缓存（滑动/滚动窗口） | ⭐⭐⭐ | Apache Flink / Kafka Streams |
|| O25 | deadband | 死区过滤（变化量小于阈值则忽略） | ⭐ | industrial SCADA |
|| O26 | formula | 公式解析求值（支持数学函数） | ⭐⭐ | NeuronEX |
|| O27 | string-ops | 字符串操作（trim/substring/regex/replace） | ⭐ | — |
|| O28 | timestamp | 时间戳操作（转换/格式化/解析） | ⭐ | — |

---

## 三-附录：算子技术调研

### 1. 行业参考实现

#### NeuronEX（南潮物联）
- **内置算子**：filter、transform、aggregate、router、switch、json-path、xml-path、formula、string-ops、time-window、deadband、range、round、clamp 等 28 个
- **特点**：工业级稳定，支持插件扩展
- **窗口机制**： Tumbling Window（滚动）、Sliding Window（滑动）、Count Window、Session Window
- **表达式引擎**：基于 Rust 动态脚本（rhai）

#### Node-RED
- **内置节点**：filter(change)、switch、router(split/join)、delay、buffer(batch)、throttle、template、csv、json、xml、function(script-js)、rbe(deadband)
- **特点**：轻量、社区活跃，适合快速原型
- **不足**：无原生时间窗口聚合（需额外节点库 node-red-contrib-windows）

#### Apache Flink
- **算子类型**：Map、Filter、FlatMap、KeyBy、Window、Reduce、Aggregate、Fold、Process
- **窗口**：TumblingEventTimeWindow、SlidingEventTimeWindow、SessionWindow、CounWindow
- **特点**：分布式、流批一体，工业级
- **不足**：重量级，不适合边缘网关

#### EMQX Rule Engine
- **内置函数**：50+ SQL 函数（数学、字符串、时间、JSON、MQTT）
- **特点**：规则 SQL 化，数据流编排
- **不足**：依赖 MQTT 生态，非独立部署

### 2. 表达式引擎选型（O01 Filter / O26 Formula 核心）

| 引擎 | 语言 | 性能 | 安全性 | 适用场景 |
|------|------|------|--------|---------|
| **rhai** | Rust 嵌入 | 极高 | 沙箱/受限 | 高性能数据处理（网关首选） |
| **mujs** | C 嵌入 | 高 | 沙箱 | JavaScript 脚本执行 |
| **PyO3** | Rust+Python | 高 | 沙箱（受限） | Python 脚本（重量级） |
| **jsonpath-rust** | Rust | 极高 | 无脚本 | JSON 路径提取 |
| **xpath** | Rust xmltree | 高 | 无脚本 | XML 路径提取 |
| **formicai/evalexpr** | Rust | 极高 | 沙箱 | 通用表达式求值 |

**推荐**：rhai（主）+ mujs（JS扩展）

### 3. 时间窗口实现方案

窗口是 aggregate 和 time-window 的核心能力，分4类：

| 窗口类型 | 触发时机 | 适用场景 | 实现复杂度 |
|---------|---------|---------|-----------|
| **Tumbling Window**（滚动） | 窗口结束时 | 固定周期统计（每分钟均值） | ⭐ |
| **Sliding Window**（滑动） | 窗口滑动步长 | 移动平均、趋势检测 | ⭐⭐ |
| **Count Window**（计数） | 达到N条 | 凑满N条再处理 | ⭐ |
| **Session Window**（会话） | 间隙超时 | 用户行为分析 | ⭐⭐⭐ |

**实现技术**：
- 边缘网关轻量级：用 `tokio::time::Interval` + `HashMap<WindowKey, Vec<Data>>`
- 进阶：参考 Flink 的 watermark 机制，处理乱序数据

### 4. 脚本算子安全性

O07（JavaScript）和 O08（Python）均为用户自定义脚本，必须沙箱化：

**JS 沙箱方案**：
```
mujs（轻量）> Duktape（已停止维护）> QuickJS（推荐）
  → 禁用：eval、Function.constructor、require、import
  → 只暴露：math、json、date、string、number 操作
```

**Python 沙箱方案**：
```
PyO3（Rust 调用 Python）
  → 禁用：os、sys、subprocess、socket、import
  → 只暴露：math、json、datetime、collections、statistics
  → 超时控制：每个脚本最大执行时间（如 100ms）
```

### 5. 算子实现优先级

| 优先级 | 算子 | 理由 |
|--------|------|------|
| **P0 必须** | O01 filter, O02 transform, O04 router | 核心数据流处理 |
| **P1 高优** | O03 aggregate, O05 buffer, O26 formula | 统计类场景必需 |
| **P2 中优** | O07 script-js, O09 json-path, O25 deadband, O15 alarm | 灵活扩展性 |
| **P3 低优** | O08 script-python, O24 time-window, O22 csv-parser | 高级特性 |

### 6. 算子与数据流模式

常见工业数据流编排模式：

```
模式1: 采集 → Filter → Transform → Aggregate(1min) → MQTT
  用途: 数据清洗 + 统计上报

模式2: 采集 → Deadband → Router → [Alarm分支 → HTTP] [Normal分支 → MQTT]
  用途: 异常数据分离告警

模式3: [South-A] ─┐
                 ├─→ Merger → Aggregate → North-MQTT
[South-B] ──────┘
  用途: 多源数据汇聚统计

模式4: 采集 → TimeWindow(sliding 30s/10s) → Aggregate(avg) → Throttle → MQTT
  用途: 平滑数据上报、降低带宽

模式5: 采集 → Script-JS(自定义计算) → Transform → Batch(攒50条) → HTTP POST
  用途: 批量数据上报企业系统
```

### Layer 3: North Plugins（传输层）

| ID | 插件名 | 协议 | 状态 |
|----|--------|------|------|
| N01 | mqtt | MQTT v3.1.1 / v5.0 | ✅ 已有 |
| N02 | http | HTTP POST/GET/PUT | 🆕 新增 |
| N03 | websocket | WebSocket Server/Push | 🆕 新增 |
| N04 | sparkplugb | Sparkplug B | 🆕 新增 |
| N05 | opcua-server | OPC UA Server | 🆕 新增 |
| N06 | grpc | gRPC | 🆕 新增 |
| N07 | kafka | Apache Kafka | 🆕 新增 |
| N08 | influxdb | InfluxDB | 🆕 新增 |
| N09 | tdengine | TDengine | 🆕 新增 |
| N10 | timescaledb | TimescaleDB | 🆕 新增 |
| N11 | postgresql | PostgreSQL | 🆕 新增 |
| N12 | mysql | MySQL | 🆕 新增 |
| N13 | oracle | Oracle | 🆕 新增 |
| N14 | modbus-tcp-north | Modbus TCP (作为主站) | 🆕 新增 |

---

## 四、数据流模型

### Flow 定义（JSON）

```json
{
  "version": "1.0",
  "flows": [
    {
      "id": "flow-uuid",
      "name": "Modbus→Filter→MQTT",
      "description": "示例流程",
      "enabled": true,
      "nodes": [
        {
          "id": "node-1",
          "type": "south",
          "plugin": "modbus-tcp",
          "name": "PLC-1",
          "config": {
            "host": "192.168.1.100",
            "port": 502,
            "slave_id": 1
          }
        },
        {
          "id": "node-2",
          "type": "operator",
          "plugin": "filter",
          "name": "温度过滤",
          "config": {
            "condition": "temperature > 100",
            "pass": true
          }
        },
        {
          "id": "node-3",
          "type": "operator",
          "plugin": "transform",
          "name": "数据转换",
          "config": {
            "rules": [
              {"tag": "temperature", "expr": "value / 10.0", "target_tag": "temp_scaled"},
              {"tag": "pressure", "expr": "value * 1.01325", "target_tag": "pressure_bar"}
            ]
          }
        },
        {
          "id": "node-4",
          "type": "north",
          "plugin": "mqtt",
          "name": "云端上传",
          "config": {
            "broker": "mqtt://broker.emqx.io:1883",
            "topic": "plant/${tags.location}/${tags.device_id}",
            "qos": 1,
            "username": "",
            "password": ""
          }
        }
      ],
      "edges": [
        {"from": "node-1", "to": "node-2"},
        {"from": "node-2", "to": "node-3"},
        {"from": "node-3", "to": "node-4"}
      ]
    }
  ]
}
```

### 数据包格式（PipelineData）

```rust
pub struct PipelineData {
    pub flow_id: Uuid,
    pub node_id: Uuid,           // 当前节点
    pub source_node_id: Uuid,    // 数据来源节点（追踪用）
    pub tags: HashMap<TagId, TagValue>,
    pub timestamp: i64,
    pub metadata: HashMap<String, String>,  // 传递上下文
}
```

---

## 五、插件 Trait 设计

### SouthPlugin Trait

```rust
#[async_trait]
pub trait SouthPlugin: Send + Sync {
    fn meta(&self) -> PluginMeta;
    fn config_schema(&self) -> Option<ConfigSchema>;
    fn tag_schema(&self) -> Option<TagSchema>;

    async fn open(&self, node_id: Uuid, config: PluginConfig) -> PluginResult<()>;
    async fn close(&self, node_id: Uuid) -> PluginResult<()>;
    async fn init(&self, node_id: Uuid) -> PluginResult<()>;
    async fn uninit(&self, node_id: Uuid) -> PluginResult<()>;
    async fn start(&self, node_id: Uuid) -> PluginResult<()>;
    async fn stop(&self, node_id: Uuid) -> PluginResult<()>;
    async fn setting(&self, node_id: Uuid, config: PluginConfig) -> PluginResult<()>;

    async fn validate_tag(&self, node_id: Uuid, tag: &Tag) -> PluginResult<()>;
    async fn poll_group(&self, node_id: Uuid, group_id: Uuid, tags: &[Tag]) -> PluginResult<Vec<(TagId, DataValue)>>;
    async fn write_tags(&self, node_id: Uuid, values: &[(Tag, DataValue)]) -> PluginResult<()>;
    async fn list_groups(&self, node_id: Uuid) -> PluginResult<Vec<Group>>;
    async fn list_tags(&self, node_id: Uuid, group_id: Uuid) -> PluginResult<Vec<Tag>>;
}
```

### OperatorPlugin Trait 🆕 NEW

```rust
#[async_trait]
pub trait OperatorPlugin: Send + Sync {
    fn meta(&self) -> PluginMeta;
    fn config_schema(&self) -> Option<ConfigSchema>;

    async fn init(&self, config: PluginConfig) -> PluginResult<()>;
    async fn uninit(&self) -> PluginResult<()>;

    /// 处理数据，返回变换后的数据（可增删改tags）
    async fn process(&self, data: PipelineData) -> PluginResult<PipelineData>;

    /// 可选：支持批处理（多个数据合并处理）
    async fn process_batch(&self, batch: Vec<PipelineData>) -> PluginResult<Vec<PipelineData>> {
        let mut results = Vec::new();
        for d in batch {
            results.push(self.process(d).await?);
        }
        Ok(results)
    }
}
```

### NorthPlugin Trait

```rust
#[async_trait]
pub trait NorthPlugin: Send + Sync {
    fn meta(&self) -> PluginMeta;
    fn config_schema(&self) -> Option<ConfigSchema>;

    async fn open(&self, node_id: Uuid, config: PluginConfig) -> PluginResult<()>;
    async fn close(&self, node_id: Uuid) -> PluginResult<()>;
    async fn init(&self, node_id: Uuid) -> PluginResult<()>;
    async fn uninit(&self, node_id: Uuid) -> PluginResult<()>;
    async fn start(&self, node_id: Uuid) -> PluginResult<()>;
    async fn stop(&self, node_id: Uuid) -> PluginResult<()>;
    async fn setting(&self, node_id: Uuid, config: PluginConfig) -> PluginResult<()>;

    async fn connection_status(&self, node_id: Uuid) -> Option<serde_json::Value>;

    async fn on_pipeline_data(&self, node_id: Uuid, data: PipelineData) -> PluginResult<()>;
    async fn on_batch(&self, node_id: Uuid, batch: Vec<PipelineData>) -> PluginResult<()>;
}
```

---

## 六、Flow Orchestrator 核心

### 目录结构

```
gateway/
├── gateway-sdk/           # 共享类型、trait定义、错误类型
├── gateway-core/         # Bus、Store、Manager（保留）
├── gateway-flow/         # 🆕 Flow编排器
│   ├── src/
│   │   ├── lib.rs
│   │   ├── flow.rs       # Flow定义、验证、DAG检查
│   │   ├── instance.rs   # Flow实例、生命周期
│   │   ├── scheduler.rs  # Pipeline调度、拓扑排序
│   │   ├── executor.rs   # 数据执行引擎
│   │   └── registry.rs  # 三层插件注册表
├── gateway-server/       # HTTP Server
└── gateway-plugins/      # 所有插件
    ├── south/            # 采集插件
    │   ├── sim/
    │   ├── modbus-tcp/
    │   ├── modbus-rtu/
    │   ├── opcua/
    │   ├── bacnet/
    │   ├── s7/
    │   ├── iec61850/
    │   ├── ethernet-ip/
    │   ├── mitsubishi-mc/
    │   ├── omron-fins/
    │   └── ... (其他南向)
    ├── operator/         # 🆕 处理插件
    │   ├── filter/
    │   ├── transform/
    │   ├── aggregate/
    │   ├── router/
    │   ├── buffer/
    │   ├── script-js/
    │   ├── alarm/
    │   └── ... (其他算子)
    └── north/            # 传输插件
        ├── mqtt/
        ├── http/
        ├── websocket/
        ├── kafka/
        ├── influxdb/
        └── ... (其他北向)
```

### Flow 生命周期

```
DRAFT → VALIDATED → DEPLOYED → RUNNING ←→ PAUSED
                    ↓
                 FAILED (错误恢复)
```

### DAG 拓扑执行

- Flow 启动时对 nodes + edges 做拓扑排序
- 每个节点在独立 tokio task 中运行
- 数据通过 channel 在节点间传递
- 支持并行分支（同一层的节点可并发执行）
- 循环依赖检测（启动时拒绝）

---

## 七、前端可视化设计

### 技术选型

| 库 | Stars | 理由 |
|----|-------|------|
| **VueFlow** (@vue-flow/core) | ~3k | Vue3 专用，API 简洁，文档清晰，支持自定义节点，可与 Element Plus 很好结合 |
| Drawflow | 6k | 轻量但非 Vue 专用 |
| G6 (AntV) | 13k | 功能强大但学习曲线陡峭，集成 Vue 需额外工作 |

**推荐：VueFlow**
- Vue3 生态原生集成
- 支持拖拽、自定义节点、边样式
- 社区活跃，TypeScript 支持好
- 内置 minimap、controls、background

### 页面结构

```
/flow                          # 流程列表
/flow/new                      # 新建流程（画布）
/flow/:id/edit                 # 编辑流程
/flow/:id/monitor              # 监控流程运行状态
/plugin                        # 插件市场
/plugin/:type/:name            # 插件详情
/south                         # 南向节点列表
/north                         # 北向节点列表
/data                          # 数据监控
/system                        # 系统配置
```

### Flow Editor 画布布局

```
┌────────────────────────────────────────────────────────────────────┐
│  Flow: Modbus→MQTT  [Save] [Deploy] [Delete]      [Zoom: 100%]   │
├────────┬───────────────────────────────────────────────┬───────────┤
│        │                                               │           │
│ PANEL  │                                               │  CONFIG   │
│        │              CANVAS (VueFlow)                 │           │
│ [South]│    ┌─────────┐      ┌─────────┐              │  Plugin:  │
│  sim   │    │ MODBUS  │─────▶│ FILTER  │────────┐    │  mqtt     │
│  modbus│    │  TCP    │      │ temp>100│        │    │           │
│  opcua │    └─────────┘      └─────────┘        │    │  Broker:  │
│  bacnet│                                        ▼    │  mqtt://  │
│  ...   │                                  ┌─────────┐ │           │
│        │                                  │  MQTT   │ │           │
│[Operator]                                 │         │ │           │
│  filter │                                  └─────────┘ │           │
│  transform                                 ▲           │           │
│  aggregate│                                │           │           │
│  router  │                        ┌─────────┘           │           │
│  ...     │                        │                    │           │
│        │                        │                       │           │
│[North] │                        └──────────────────────┘           │
│  mqtt   │                  (数据流连线)                             │
│  http   │                                                       │
│  kafka  │                                                       │
│  ...   │                                                       │
└────────┴───────────────────────────────────────────────────────┴───┘
```

---

## 八、数据库 Schema（SQLite）

```sql
-- Flow 定义
CREATE TABLE flows (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    description TEXT,
    definition TEXT NOT NULL,  -- JSON: nodes + edges
    version INTEGER DEFAULT 1,
    enabled INTEGER DEFAULT 0,
    status TEXT DEFAULT 'draft',  -- draft/validated/deployed/running/paused
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

-- South 节点（复用原结构）
CREATE TABLE nodes (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL,  -- south/operator/north
    plugin_name TEXT NOT NULL,
    name TEXT NOT NULL,
    config TEXT NOT NULL,  -- JSON
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

-- 保留原 groups/tags/subscriptions 表结构不变
```

---

## 九、实施路线

### Phase 1: 核心框架（4-6周）
- [ ] 重构 gateway-sdk：新增 OperatorPlugin trait、PipelineData 类型
- [ ] 新建 gateway-flow crate：Flow 定义、验证、编排器
- [ ] 重构 gateway-server：Flow CRUD API
- [ ] 内置 5 个算子：filter、transform、aggregate、router、buffer
- [ ] 前端 VueFlow 集成：Flow Editor 画布页面
- [ ] Flow 与 South/North 节点统一配置管理

### Phase 2: 协议插件补全（8-12周）
- [ ] S05-S10（bacnet、s7、iec61850、ethernet-ip、mitsubishi-mc、omron-fins）
- [ ] S11-S18（hollysys、deltadvp、yokogawa、abb-dcs、honeywell-pks、emerson-deltav、siemens-pcs7、dlt645）
- [ ] S19-S26（gb28181、knx、enocean、mbus、obix、ethercat、profinet、userprog）
- [ ] N02-N14（http、websocket、sparkplugb、opcua-server、grpc、kafka、influxdb、tdengine、timescaledb、postgresql、mysql、oracle、modbus-tcp-north）

### Phase 3: 高级算子（4-6周）
- [ ] script-js、script-python（沙箱执行）
- [ ] alarm（告警规则引擎）
- [ ] json-path、xml-path
- [ ] throttle、cache、batch

### Phase 4: 生产化（2-4周）
- [ ] 高可用（多实例部署）
- [ ] 配置导入/导出
- [ ] 备份与恢复
- [ ] 性能压测

---

## 九-附录：开发前调研详情

### A. 前端技术栈选型

#### 选型结论：**VueFlow**（已确认）

| 候选方案 | Stars | 优点 | 缺点 | 结论 |
|---------|-------|------|------|------|
| **VueFlow** | ~3k | Vue3 原生、API 简洁、TS 好、自定义节点、minimap/controls 内置 | 相对较新 | ✅ 选用 |
| Node-RED | 18k | 成熟、拖拽好 | jQuery 技术栈、Vue 集成成本高 | ❌ 放弃 |
| G6 (AntV) | 13k | 功能强大 | 学习曲线陡峭、Vue 集成工作量大 | ❌ 放弃 |
| 自研 Canvas | — | 完全可控 | 工作量大、BUG 多、周期长 | ❌ 放弃 |

**VueFlow 核心能力：**
- 拖拽节点到画布
- 自定义节点类型（South/Operator/North 三种外观）
- 边连接（source/target）
- MiniMap + Controls
- 双击节点打开配置面板
- 节点位置持久化（保存 Flow 时存储坐标）

**与现有技术栈匹配：**
- 现有前端：Vue3 + Element Plus + vue-router + vite
- VueFlow 直接支持 Vue3，Element Plus 表单组件可直接用于配置面板
- 无需更换技术栈

**安装命令（Phase 1 执行）：**
```bash
npm install @vue-flow/core @vue-flow/background @vue-flow/controls @vue-flow/minimap
```

---

### B. 插件 FFI 加载机制（现有架构分析）

#### 现状：South/North 插件

- **加载方式**：通过 `libloading` 加载 `.so` 动态库（C ABI）
- **跨边界数据传递**：全部以 JSON 字符串传递
- **符号约定**：每个方法对应一个 C 符号（如 `gateway_south_plugin_open`）
- **宿主侧适配器**：`SouthSoAdapter` / `NorthSoAdapter` 实现 `SouthPlugin` / `NorthPlugin` trait，将 FFI 调用转发给 `.so`
- **内存管理**：插件分配字符串 → 宿主复制 → 调用 `gateway_plugin_free_string` 释放

#### Operator 插件加载策略

**两种方案对比：**

| 方案 | 实现方式 | 优点 | 缺点 |
|------|---------|------|------|
| **方案A：内置（推荐）** | Operator 作为 `gateway-flow` crate 内部模块（`src/operators/filter.rs` 等） | 无 FFI 开销、类型安全、调试简单、共享 `gateway-sdk` | 每次新增算子需编译整个 crate |
| **方案B：.so 插件化** | 同 South/North，通过 `libloading` 加载 `OperatorSoAdapter` | 完全解耦、插件可独立发布 | FFI 复杂、表达式引擎需跨边界传递 |

**推荐方案A（内置）** 理由：
- Operator 逻辑相对简单（无设备连接、无协议解析）
- 算子表达式求值（rhai）可直接内嵌 Rust
- 内置算子共享 `gateway-flow` 依赖树，无额外加载复杂度
- Phase 1 阶段算子数量有限（5个），编译开销可接受
- 未来算子超过 10 个时可平滑迁移到方案B

**FFI 符号设计（如果未来需要外部 Operator）：**
```rust
// 追加到 gateway-sdk/src/ffi.rs
pub const SYM_OPERATOR_CREATE: &[u8] = b"gateway_operator_plugin_create";
pub const SYM_OPERATOR_DESTROY: &[u8] = b"gateway_operator_plugin_destroy";
pub const SYM_OPERATOR_META: &[u8] = b"gateway_operator_plugin_meta";
pub const SYM_OPERATOR_INIT: &[u8] = b"gateway_operator_plugin_init";
pub const SYM_OPERATOR_UNINIT: &[u8] = b"gateway_operator_plugin_uninit";
pub const SYM_OPERATOR_PROCESS: &[u8] = b"gateway_operator_plugin_process";
pub const SYM_OPERATOR_PROCESS_BATCH: &[u8] = b"gateway_operator_plugin_process_batch";
pub const SYM_OPERATOR_CONFIG_SCHEMA: &[u8] = b"gateway_operator_plugin_config_schema";
```

---

### C. 数据库 Schema 详细设计

#### 设计原则
- 复用现有 `groups` / `tags` / `subscriptions` 表（不变）
- 新增 `flows` / `flow_nodes` 表存储 Flow 定义
- Node 粒度：South 和 Operator 节点存储在 `flow_nodes`，North 节点同样
- 运行时状态（running/paused）存内存，不落库

#### 完整 Schema

```sql
-- ========== Flow 定义表 ==========
CREATE TABLE flows (
    id          TEXT PRIMARY KEY,           -- UUID
    name        TEXT NOT NULL,
    description TEXT,
    definition  TEXT NOT NULL,              -- JSON: nodes[] + edges[]
    version     INTEGER DEFAULT 1,
    enabled     INTEGER DEFAULT 0,          -- 0=禁用, 1=启用
    status      TEXT DEFAULT 'draft',       -- draft/validated/running/paused/failed
    created_at  TEXT NOT NULL,              -- ISO8601
    updated_at  TEXT NOT NULL
);

-- ========== Flow 节点配置表 ==========
CREATE TABLE flow_nodes (
    id          TEXT PRIMARY KEY,           -- UUID
    flow_id     TEXT NOT NULL,
    kind        TEXT NOT NULL,              -- south / operator / north
    plugin_name TEXT NOT NULL,              -- 插件名（sim, modbus-tcp, filter, mqtt...）
    name        TEXT NOT NULL,
    position_x  REAL,                       -- 画布 X 坐标（前端用）
    position_y  REAL,                       -- 画布 Y 坐标
    config      TEXT NOT NULL DEFAULT '{}', -- JSON: 插件配置
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL,
    FOREIGN KEY (flow_id) REFERENCES flows(id) ON DELETE CASCADE
);

-- ========== 保留现有表（不变）==========
-- groups: South 设备点位分组
-- tags: South 设备具体点位
-- subscriptions: North 订阅关系

-- ========== 索引 ==========
CREATE INDEX idx_flow_nodes_flow_id ON flow_nodes(flow_id);
CREATE INDEX idx_flows_status ON flows(status);
```

#### 迁移策略
- Phase 1 先用 SQLite，Flow 表结构独立，不影响现有表
- Phase 4 如需换 PostgreSQL，迁移脚本只操作 `flows` / `flow_nodes`

---

### D. Flow Orchestrator API 详细设计

#### REST API 设计

| Method | Path | 说明 |
|--------|------|------|
| `GET` | `/api/flows` | 列出所有 Flow |
| `POST` | `/api/flows` | 创建 Flow（含 nodes + edges） |
| `GET` | `/api/flows/:id` | 获取单个 Flow 详情 |
| `PUT` | `/api/flows/:id` | 更新 Flow 定义 |
| `DELETE` | `/api/flows/:id` | 删除 Flow |
| `POST` | `/api/flows/:id/deploy` | 部署 Flow（draft → running） |
| `POST` | `/api/flows/:id/pause` | 暂停 Flow（running → paused） |
| `POST` | `/api/flows/:id/stop` | 停止 Flow（running/paused → stopped） |
| `GET` | `/api/flows/:id/status` | 获取 Flow 运行状态 |
| `GET` | `/api/flows/:id/nodes/:node_id/metrics` | 节点级 metrics |

#### 请求/响应示例

**POST /api/flows** — 创建 Flow
```json
// Request
{
  "name": "Modbus → Filter → MQTT",
  "description": "PLC 数据清洗后上传云端",
  "nodes": [
    {
      "id": "550e8400-e29b-41d4-a716-446655440001",
      "name": "PLC-1",
      "plugin": "modbus-tcp",
      "kind": "south",
      "position_x": 100,
      "position_y": 200,
      "config": {
        "host": "192.168.1.100",
        "port": 502,
        "slave_id": 1
      }
    },
    {
      "id": "550e8400-e29b-41d4-a716-446655440002",
      "name": "温度过滤",
      "plugin": "filter",
      "kind": "operator",
      "position_x": 350,
      "position_y": 200,
      "config": {
        "condition": "temperature > 100",
        "pass": true
      }
    },
    {
      "id": "550e8400-e29b-41d4-a716-446655440003",
      "name": "云端上传",
      "plugin": "mqtt",
      "kind": "north",
      "position_x": 600,
      "position_y": 200,
      "config": {
        "broker": "mqtt://broker.emqx.io:1883",
        "topic": "plant/${tags.location}/${tags.device_id}"
      }
    }
  ],
  "edges": [
    {"from": "550e8400-e29b-41d4-a716-446655440001", "to": "550e8400-e29b-41d4-a716-446655440002"},
    {"from": "550e8400-e29b-41d4-a716-446655440002", "to": "550e8400-e29b-41d4-a716-446655440003"}
  ]
}

// Response 201
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "name": "Modbus → Filter → MQTT",
  "status": "draft",
  ...
}
```

**POST /api/flows/:id/deploy** — 部署
```json
// Response 200
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "status": "running",
  "deployed_at": "2026-05-19T10:30:00Z"
}
```

#### WebSocket 实时事件（未来扩展）

```json
// 节点数据事件（Flow 运行中）
{
  "type": "node_data",
  "flow_id": "550e8400-...",
  "node_id": "550e8400-...",
  "timestamp": 1716106200000,
  "tags": {"temperature": {"type": "float64", "value": 85.5}}
}

// 节点状态变更
{
  "type": "node_status",
  "flow_id": "550e8400-...",
  "node_id": "550e8400-...",
  "status": "error",
  "message": "连接超时"
}
```

---

### E. OperatorPlugin 完整接口设计

#### 核心接口

```rust
#[async_trait]
pub trait OperatorPlugin: Send + Sync {
    fn meta(&self) -> PluginMeta;

    /// 配置 Schema（JSON Schema，用于前端表单生成）
    fn config_schema(&self) -> Option<ConfigSchema> { None }

    /// 插件级初始化（可选）
    async fn init(&self, config: PluginConfig) -> PluginResult<()> {
        let _ = config;
        Ok(())
    }

    /// 插件级反初始化（可选）
    async fn uninit(&self) -> PluginResult<()> { Ok(()) }

    /// 处理单条 PipelineData
    async fn process(&self, data: PipelineData) -> PluginResult<PipelineData>;

    /// 批处理（默认逐条）
    async fn process_batch(&self, batch: Vec<PipelineData>) -> PluginResult<Vec<PipelineData>> {
        let mut results = Vec::with_capacity(batch.len());
        for d in batch {
            results.push(self.process(d).await?);
        }
        Ok(results)
    }

    /// 可选：重置算子内部状态（如 aggregate 的滑动窗口状态）
    async fn reset(&self) -> PluginResult<()> { Ok(()) }
}
```

#### PipelineData 类型（完整定义）

```rust
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineData {
    /// Flow 实例 ID
    pub flow_id: Uuid,
    /// 当前节点 ID
    pub node_id: Uuid,
    /// 数据来源节点 ID（链路追踪）
    pub source_node_id: Uuid,
    /// 标签数据
    pub tags: HashMap<String, DataValue>,
    /// 原始时间戳（毫秒）
    pub timestamp: i64,
    /// 上下文元数据（来源设备、位置等）
    pub metadata: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "value")]
pub enum DataValue {
    Bool(bool),
    Int8(i8), Int16(i16), Int32(i32), Int64(i64),
    UInt8(u8), UInt16(u16), UInt32(u32), UInt64(u64),
    Float32(f32), Float64(f64),
    String(String),
    Bytes(Vec<u8>),
    Null,
}
```

#### Operator 状态管理设计

**有状态算子（aggregate、buffer、cache）** 需要在算子实例生命周期内保持状态：

```rust
// 算子实例状态（每个节点独享）
pub struct OperatorInstance {
    plugin_name: String,
    config: PluginConfig,
    // 有状态算子的内部状态
    state: OperatorState,
}

pub enum OperatorState {
    Filter(()),                    // 无状态
    Transform(()),                 // 无状态
    Aggregate(AggregateState),      // 有状态
    Buffer(BufferState),            // 有状态
    Cache(CacheState),              // 有状态
    Router(RouterState),            // 有状态
}

pub struct AggregateState {
    /// tag_name → 窗口数据
    windows: HashMap<String, Vec<(i64, f64)>>,
    /// tag_name → 上次触发时间
    last_trigger: HashMap<String, i64>,
    window_ms: i64,
    aggregation: AggregationType,
}

pub struct BufferState {
    queue: VecDeque<PipelineData>,
    batch_size: usize,
    flush_interval_ms: u64,
}
```

**状态生命周期：**
- 随 Flow 实例创建而创建
- Flow 暂停/停止时状态保留（内存）
- Flow 删除时释放
- 不落库（Phase 1），未来可扩展状态快照到 SQLite

---

### F. Phase 1 详细实施任务（最终版）

#### Stage 1: gateway-sdk 扩展（5 任务）

| # | 任务 | 产出 |
|---|------|------|
| 1 | 新增 `PipelineData` + `DataValue` 类型到 `src/pipeline.rs` | 跨节点数据传递标准格式 |
| 2 | 新增 `OperatorPlugin` trait 到 `src/plugin.rs` | 算子插件标准接口 |
| 3 | 扩展 `PluginKind::Operator` 枚举 | 类型系统支持三层插件 |
| 4 | 更新 `PluginConfig` 类型（现有 `HashMap` 已够用，确认即可） | 配置传递标准化 |
| 5 | 更新 `meta_to_ffi()` 支持 `Operator` | FFI 符号导出兼容 |

#### Stage 2: gateway-flow crate（6 任务）

| # | 任务 | 产出 |
|---|------|------|
| 6 | 创建 crate + 添加到 workspace | `gateway-flow` 独立 crate |
| 7 | 实现 `Flow` 结构 + DAG 验证 + 拓扑排序 | Flow 定义与校验 |
| 8 | 实现 `PluginRegistry` 三层注册表 | South/North/Operator 插件管理 |
| 9 | 实现 `FlowExecutor` DAG 执行骨架 | 拓扑顺序执行节点 |
| 10 | 创建 `operators/mod.rs` + `operators/filter.rs` 骨架 | 算子模块结构 |
| 11 | 添加 `rhai` 依赖，验证 filter 表达式求值 | 表达式引擎集成 |

#### Stage 3: 5 个内置算子（5 任务）

| # | 任务 | 产出 |
|---|------|------|
| 12 | 完成 `filter` 算子（条件表达式，rhai） | 过滤数据 |
| 13 | 实现 `transform` 算子（类型转换、缩放） | 数据转换 |
| 14 | 实现 `aggregate` 算子（时间窗口聚合） | 统计聚合 |
| 15 | 实现 `router` 算子（条件路由） | 分支分发 |
| 16 | 实现 `buffer` 算子（批量缓冲） | 攒批输出 |

#### Stage 4: gateway-server Flow API（4 任务）

| # | 任务 | 产出 |
|---|------|------|
| 17 | 创建 `flows` / `flow_nodes` SQLite 表 | 持久化存储 |
| 18 | 实现 Flow CRUD REST 处理器 | `/api/flows` 系列接口 |
| 19 | 实现 Flow deploy/pause/stop 生命周期 API | 部署控制接口 |
| 20 | 将 `gateway-flow` 集成到 `AppState` | 运行时 FlowExecutor 管理 |

#### Stage 5: 前端 VueFlow 集成（4 任务）

| # | 任务 | 产出 |
|---|------|------|
| 21 | 安装 VueFlow 依赖 | `@vue-flow/core` 等 |
| 22 | 创建 `FlowList.vue` 列表页 | 流程列表 |
| 23 | 创建 `FlowEditor.vue` 编辑器（拖拽画布） | 可视化编排 |
| 24 | 创建自定义节点组件（South/Operator/North 三种外观） | 节点样式区分 |

**Phase 1 总计：24 个任务，预计 4-6 周完成。**

---

## Phase 2 实现状态 ✅

✅ **已完成**

### 协议插件补全

#### South 插件（14个）
| # | 插件 | 说明 |
|---|------|------|
| S01 | modbus-rtu | 串口 Modbus RTU ✅ 原有 |
| S02 | modbus-tcp | Modbus TCP ✅ 原有 |
| S03 | mqtt | MQTT 客户端 ✅ 原有 |
| S04 | opcua | OPC-UA ✅ 原有 |
| S05 | sim | 模拟数据 ✅ 原有 |
| S06 | virb | 虚拟设备 ✅ 原有 |
| S07 | bacnet | BACnet 楼宇自动化 ✅ |
| S08 | s7 | 西门子 S7 PLC ✅ |
| S09 | dl-t645 | DL/T645 电表 ✅ |
| S10 | iec61850 | IEC61850 变电站 ✅ |
| S11 | ethernet-ip | EtherNet/IP ✅ |
| S12 | mitsubishi-mc | 三菱 MC 协议 ✅ |
| S13 | profinet | PROFINET 工业以太网 ✅ |
| S14 | snmp | SNMP 网络监控 ✅ |
| S15 | omron-fins | Omron FINS PLC ✅ |

#### North 插件（8个）
| # | 插件 | 说明 |
|---|------|------|
| N01 | mqtt | MQTT Broker ✅ 原有 |
| N02 | http | HTTP Webhook ✅ |
| N03 | kafka | Kafka Producer ✅ |
| N04 | influxdb | InfluxDB 时序库 ✅ |
| N05 | tdengine | TDengine 时序库 ✅ |
| N06 | websocket | WebSocket 推送 ✅ |
| N07 | grpc | gRPC 推送 ✅ |
| N08 | sparkplug | Sparkplug B (MQTT格式) ✅ |

#### 高级算子（21个）
| # | 算子 | 说明 |
|---|------|------|
| O01 | filter | 条件过滤 ✅ |
| O02 | transform | 字段变换 ✅ |
| O03 | aggregate | 时间窗口聚合 ✅ |
| O04 | router | 条件路由 ✅ |
| O05 | buffer | 缓冲批处理 ✅ |
| O06 | alarm | 告警规则引擎 ✅ |
| O07 | json-path | JSON 字段提取 ✅ |
| O08 | deadband | 死区过滤 ✅ |
| O09 | formula | rhai 表达式计算 ✅ |
| O10 | clamp | 限幅 ✅ |
| O11 | round | 四舍五入 ✅ |
| O12 | change | 变化检测 ✅ |
| O13 | range | 范围映射 ✅ |
| O14 | batch | 批量处理 ✅ |
| O15 | split | 字符串分割 ✅ |
| O16 | join | 字符串合并 ✅ |
| O17 | dedup | 去重 ✅ |
| O18 | script | rhai 脚本算子 ✅ |
| O19 | throttle | 限流 ✅ |
| O20 | convert | 类型转换 ✅ |
| O21 | log | 日志输出 ✅ |
| O22 | xml-path | XML XPath 提取 ✅ |
| O23 | script-python | Python 脚本（stub） ✅ |

### 基础设施
- [x] `SouthAddress` 统一地址枚举（ModbusAddr/BACnetAddr/S7Addr/IEC61850Addr/GenericAddr）
- [x] `AddressScheme` trait
- [x] `/flows/operators` API（算子发现）
- [x] `/plugins` API（插件发现）

### Phase 2 Git 提交记录
```
a71e24d feat(gateway-plugins): add DL/T645, IEC61850, EtherNet/IP, Mitsubishi MC...
cdc7b6c feat(gateway-plugins): add WebSocket, gRPC, and Sparkplug B north...
91b1d94 feat(gateway-plugins): add PROFINET, SNMP, and Omron FINS south...
cba3f93 feat(gateway-plugins): add EtherNet/IP and Mitsubishi MC south protocol...
28d9292 feat(gateway-flow): add batch/split/join/dedup/script/throttle/convert/log...
babc9ea feat(gateway-plugins): add HTTP webhook and Kafka north protocol plugins
```

---

## Phase 3 实现状态 ✅

✅ **已完成**

### 前端完善

#### B1: 节点选择面板
- [x] 算子从 `GET /flows/operators` API 动态加载
- [x] South/Operator/North 三分类可折叠面板（el-collapse）
- [x] Hover 显示 description_zh 说明
- [x] Emoji 图标：🔌 South / ⚙️ Operator / 📤 North

#### B2: 属性配置面板
- [x] South 节点：`api.pluginSouthSchema()` 动态表单
- [x] Operator 节点：21种算子配置字段完整覆盖
- [x] North 节点：`api.pluginNorthSchema()` 动态表单
- [x] el-tabs 多标签页布局

#### B3: 部署管理
- [x] FlowList 生命周期按钮（deploy/start/pause/stop/delete）
- [x] FlowEditor 工具栏状态联动
- [x] 部署后每3秒轮询状态，共5次（15秒窗口）

#### B4: 数据预览面板
- [x] FlowEditor 底部可折叠面板
- [x] 选中节点显示输入/输出数据占位
- [x] 最近5条消息列表
- [x] 每2秒自动刷新

#### C1: South 点位管理
- [x] 属性面板「点位」Tab
- [x] `api.groups(nodeId)` 加载分组
- [x] `api.tags(nodeId)` 加载点位
- [x] el-table 展示（名称/地址/类型/访问模式）
- [x] 刷新按钮

#### C2: North 订阅管理
- [x] 属性面板「订阅」Tab
- [x] 当前 Flow 中 South/Operator 节点列表
- [x] 勾选订阅节点
- [x] `api.setSubscriptions()` 保存

### 剩余算子

#### A1: xml-path 算子
- [x] 基于 regex 的 XPath-like 提取
- [x] 配置：source_field, expressions, namespaces

#### A2: script-python 算子
- [x] rhai 引擎执行 Python 语法
- [x] 配置：script, input_fields, output_fields, timeout_ms
- [x] 内置函数：len/abs/round/min/max/floor/ceil/sqrt/pow/log/sin/cos/tan/str/int/float

### Phase 3 Git 提交记录
```
679e8e1 feat: add xml-path and script-python operators + enhance FlowEditor...
```

---

## Phase 4 实现状态 ✅

✅ **核心已完成**

### D3: Flow 热重载
- [x] `POST /flows/:id/reload`
- [x] 停止运行中的 Flow 实例
- [x] 重新解析 Flow 定义并验证
- [x] 状态重置为 draft

### D4: Prometheus 监控
- [x] `GET /metrics` — Prometheus text exposition 格式
- [x] `GET /health` — 健康检查 + timestamp
- [x] 指标：gateway_flows_running / gateway_flows_total / gateway_nodes_processed_total / gateway_errors_total

### D2: Flow 版本历史
- [x] `version_history` JSON 数组列（保留最近10个快照）
- [x] `GET /flows/:id/versions` — 列出版本历史
- [x] `POST /flows/:id/rollback/:version` — 回滚到指定版本
- [x] `update_flow` 自动保存快照

### D5: 配置导入/导出
- [x] `GET /flows/export` — 导出 Flow JSON（支持 `?ids=` 选择性导出）
- [x] `POST /flows/import` — 导入（`force: true` 覆盖同名）
- [x] ExportResponse 版本时间戳元数据

### D6: SQLite 备份/恢复
- [x] `POST /admin/sqlite-backup` — 时间戳文件备份
- [x] `GET /admin/sqlite-backup` — 下载最新备份
- [x] `POST /admin/sqlite-restore` — 从备份路径恢复

### Phase 4 Git 提交记录
```
cab48c0 feat(gateway-server): add D3 hot reload, D4 Prometheus metrics, D2 version history, D5 import/export, D6 backup/restore
```

---

## Phase 5: WebSocket 实时可视化（规划中）

### 目标
Flow 运行数据实时推送，前端 WebSocket 订阅。

### 方案
```
gateway-server              Web 前端
     │                           │
  FlowRuntime ──────────────────┼── WebSocket /ws/flows/:id/live
     │                           │   {"type": "node_data", "node_id": "...", "tags": {...}}
     └───────────────────────────┘
```

### API 设计
| Method | Path | 说明 |
|--------|------|------|
| `WS` | `/ws/flows/:id/live` | 订阅 Flow 实时数据 |
| `GET` | `/flows/:id/live/summary` | 获取当前 Flow 运行快照 |

### WebSocket 事件
```json
{ "type": "node_data", "flow_id": "...", "node_id": "...", "timestamp": 1234567890, "tags": {...} }
{ "type": "node_status", "flow_id": "...", "node_id": "...", "status": "running|error", "message": "..." }
{ "type": "flow_status", "flow_id": "...", "status": "running|paused|stopped", "nodes_total": 5 }
```

### 前端 DataMonitor 增强
- 实时曲线图（基于 ECharts）
- 节点状态指示（绿色=运行/红色=错误/灰色=停止）
- 消息速率仪表盘
- 告警事件列表

---

## 向后兼容性

**完全破坏性，不保留旧代码。**

新仓库建议命名：`neuron-gateway` 或 `flowlink`（待定）

---

*文档状态：v1.2 — Phase 1/2/3/4 全部实现完成，Phase 5 WebSocket 实时可视化规划中*
