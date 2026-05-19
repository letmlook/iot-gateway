# IoT 网关破坏性重构 — Phase 2 实施计划

> **分支**: `redesign/v1-flow-orchestration`
>
> **目标**: 协议插件补全 + 高级算子，实现工业场景全覆盖
>
> **前置依赖**: Phase 1 完成（Flow Orchestrator 核心框架 ✅）

---

## 一、范围与优先级

### 南向插件（S05-S26，新增 22 个）

| 优先级 | ID | 插件名 | 协议 | 工业场景 |
|--------|----|-------|------|---------|
| P0 | S05 | bacnet | BACnet/IP | 楼宇自控 HVAC |
| P0 | S06 | s7 | Siemens S7 (1200/1500/300/400) | 离散制造 PLC |
| P1 | S08 | ethernet-ip | EtherNet/IP (AB PLC) | 北美工业自动化 |
| P1 | S09 | mitsubishi-mc | 三菱 MC 协议 | 日本 PLC |
| P1 | S18 | dlt645 | DL/T645（电能表） | 电力监控 |
| P2 | S07 | iec61850 | IEC 61850 (MMS/GOOSE) | 变电站自动化 |
| P2 | S10 | omron-fins | Omron FINS | 日本 PLC |
| P3 | S11-S17 | hollysys/deltadvp/yokogawa/abb-dcs/honeywell-pks/emerson-deltav/siemens-pcs7 | 各厂商 DCS | 大型过程控制 |
| P3 | S19-S26 | gb28181/knx/enocean/mbus/obix/ethercat/profinet/userprog | 视频/楼宇/无线/现场总线 | 专业细分场景 |

### 北向插件（N02-N14，新增 12 个）

| 优先级 | ID | 插件名 | 协议 | 用途 |
|--------|----|-------|------|------|
| P0 | N02 | http | HTTP POST/GET | 任意 REST API 云端 |
| P0 | N07 | kafka | Apache Kafka | 企业消息中间件 |
| P1 | N08 | influxdb | InfluxDB | 时序数据库 |
| P1 | N09 | tdengine | TDengine | 国产时序数据库 |
| P2 | N03 | websocket | WebSocket Server | 前端实时推送 |
| P2 | N04 | sparkplugb | Sparkplug B | MQTT 工业互联标准 |
| P2 | N10 | timescaledb | TimescaleDB | PostgreSQL 时序扩展 |
| P3 | N05 | opcua-server | OPC UA Server | 允许第三方采集 |
| P3 | N06 | grpc | gRPC | 高性能微服务 |
| P3 | N11-N13 | postgresql/mysql/oracle | SQL 数据库 | 企业数据持久化 |
| P3 | N14 | modbus-tcp-north | Modbus TCP (主站) | 反向控制设备 |

### 高级算子（O06-O28，新增 23 个）

| 优先级 | ID | 插件名 | 复杂度 | 说明 |
|--------|----|-------|--------|------|
| P0 | O15 | alarm | ⭐⭐⭐ | 告警规则（阈值/变化率/状态） |
| P1 | O09 | json-path | ⭐ | JSON 路径提取 |
| P1 | O25 | deadband | ⭐ | 死区过滤 |
| P1 | O26 | formula | ⭐⭐ | 公式解析求值 |
| P2 | O06 | delay | ⭐ | 延迟注入 |
| P2 | O11 | splitter | ⭐ | 批量消息拆分 |
| P2 | O12 | merger | ⭐⭐ | 多源数据合并 |
| P2 | O14 | throttle | ⭐ | 限流/采样 |
| P2 | O17 | switch | ⭐ | 多分支条件匹配 |
| P2 | O18 | clamp | ⭐ | 值域限幅 |
| P2 | O19 | round | ⭐ | 数值取整 |
| P2 | O20 | change | ⭐ | 字段替换/删除 |
| P2 | O21 | range | ⭐ | 线性变换 |
| P3 | O07 | script-js | ⭐⭐ | JavaScript 脚本（需 mujs 沙箱） |
| P3 | O08 | script-python | ⭐⭐ | Python 脚本（需 PyO3 沙箱） |
| P3 | O10 | xml-path | ⭐⭐ | XML 路径提取 |
| P3 | O13 | cache | ⭐⭐ | 缓存（in-memory） |
| P3 | O16 | batch | ⭐ | 批量分组 |
| P3 | O22 | csv-parser | ⭐⭐ | CSV 编码/解码 |
| P3 | O23 | binary | ⭐⭐ | 二进制位操作 |
| P3 | O24 | time-window | ⭐⭐⭐ | 时间窗口（独立算子） |
| P3 | O27 | string-ops | ⭐ | 字符串操作 |
| P3 | O28 | timestamp | ⭐ | 时间戳操作 |

---

## 二、实施策略

### 插件开发模式

参考现有 `plugin-modbus-tcp` 的结构：
```
plugin-xxx/
├── src/
│   ├── lib.rs          # 插件入口，定义 FFI 符号
│   ├── ffi.rs          # gateway_sdk 兼容层
│   ├── config.rs       # 配置结构体（JSON Schema）
│   ├── address.rs      # 地址模型（支持多类型标签地址）
│   ├── state.rs        # 运行时状态
│   └── value.rs        # 数据值转换
├── Cargo.toml
└── build.rs (可选)
```

### 通用基础设施

Phase 2 启动前，先扩展 `gateway-sdk` 支持南向/北向插件通用能力：

1. **SouthAddress 泛化** — 当前 `ModbusAddress`，未来还有 BACnetAddress、S7Address...
2. **插件发现机制** — 扫描 `gateway-plugins/` 下的 `.so` 文件自动注册
3. **统一配置 Schema** — 各类插件的配置 JSON Schema 规范化

---

## 三、任务分解

### Stage A: 基础设施（开工前必做）

#### Task A1: gateway-sdk 南向地址模型泛化

**目标**: 将 `ModbusAddress` 泛化为 `SouthAddress` enum，支持多协议地址解析

**Files**:
- Modify: `gateway-sdk/src/types.rs` — `SouthAddress` enum（BACnet/S7/Ethernet-IP...）

#### Task A2: 插件自动发现机制

**目标**: gateway-server 启动时扫描 `gateway-plugins/` 目录自动加载插件

**Files**:
- Modify: `gateway-core/src/loader.rs` — 新增 `discover_plugins()` 方法
- Modify: `gateway-server/src/main.rs` — 启动时调用

---

### Stage B: 南向插件（按优先级）

#### Task B1: BACnet/IP (S05)

**依赖**: A1
**目标**: 实现 BACnet/IP 协议采集插件，支持：
- Who-Is / I-Am 设备发现
- ReadProperty / WriteProperty 服务
- 订阅 Change-of-Value（COV）
- BACnet 专用地址模型（BACnetAddress: device_id, object_type, instance）

**参考**: `bacnet` Rust crate (https://crates.io/crates/bacnet)

#### Task B2: Siemens S7 (S06)

**依赖**: A1
**目标**: 实现 S7 协议采集插件，支持：
- ISO-TCP 连接（RPi/ISO-on-TCP）
- S7 300/400/1200/1500 CPU
- DB/M/E/A/PA 地址区读写
- 变量列表订阅（Polling）

**参考**: `s7` Rust crate 或 snap7 C bindings

#### Task B3: Ethernet/IP (S08)

**依赖**: A1
**目标**: 实现 EtherNet/IP 采集插件，支持：
- CIP 消息族
- Unconnected/Session Based 通信
- Tag Read/Write
- Explicit Messaging

**参考**: `ethernet-ip` Rust crate

#### Task B4: DL/T645 (S18)

**依赖**: 无
**目标**: 实现 DL/T645 电能表协议，支持：
- 规约帧解析（Data=报文）
- 多功能电能表数据项
- 串口通信（RS485）

#### Task B5: IEC61850 / Omron FINS / Mitsubishi MC / Hollysys

**后续批次**，每批 2-3 个协议

---

### Stage C: 北向插件（按优先级）

#### Task C1: HTTP (N02)

**依赖**: 无
**目标**: 实现 HTTP 北向插件，支持：
- HTTP POST/GET/PUT/DELETE
- 模板化 URL（支持 `${tag}` 占位符）
- 自定义 Header（Authorization: Bearer xxx）
- JSON Body 模板化
- 认证（Basic / Bearer Token / API Key）

#### Task C2: Kafka (N07)

**依赖**: 无
**目标**: 实现 Kafka 北向插件，支持：
- rdkafka 客户端
- SASL/SSL 认证
- 主题模板（`${tags.location}`）
- Key-Value 消息（Key = device_id）
- 批量发送优化

#### Task C3: InfluxDB (N08)

**依赖**: 无
**目标**: 实现 InfluxDB 北向插件，支持：
- InfluxDB Line Protocol
- HTTP API (POST /write)
- Measurement + TagSet + FieldSet 模型
- 批量写入优化（攒批）

#### Task C4: TDengine (N09)

**依赖**: 无
**目标**: 实现 TDengine 北向插件，支持：
- REST API / taosc 客户端
- 超级表 + 子表模型
- 标签（Tags）和数据字段

#### Task C5: WebSocket / SparkplugB / TimescaleDB / OPC-UA-Server / gRPC

**后续批次**

---

### Stage D: 高级算子（按优先级）

#### Task D1: alarm 算子 (O15) — 最高优先级

**目标**: 告警规则引擎，支持：
- **阈值告警**: `value > high` / `value < low`
- **变化率告警**: `rate_of_change > threshold`
- **状态变化告警**: `value == alarm_state`
- **告警级别**: Critical / High / Medium / Low / Info
- **告警状态**: Active / Acknowledged / Cleared
- **事件输出**: 触发时向下游传递告警数据（包含 original_data）

**Files**:
- Create: `gateway-flow/src/operators/alarm.rs`

**技术实现**:
```rust
pub struct AlarmOperator {
    rules: Vec<AlarmRule>,
    // 告警状态缓存（用于变化率/确认状态）
    states: HashMap<String, AlarmState>,
}

pub enum AlarmRule {
    Threshold { tag: String, op: CompareOp, value: f64, level: AlarmLevel },
    RateOfChange { tag: String, threshold: f64, window_secs: u64 },
    StateChange { tag: String, from: String, to: String, level: AlarmLevel },
}

#[derive(Clone, Serialize, Deserialize)]
pub struct AlarmEvent {
    pub rule_id: String,
    pub level: AlarmLevel,
    pub message: String,
    pub original_data: PipelineData,
    pub timestamp: i64,
}
```

#### Task D2: json-path 算子 (O09)

**依赖**: Phase 1 aggregate（已有基础）
**目标**: 从 JSON 数据中按路径提取字段
- 使用 `jsonpath-rust` crate
- 支持提取单个值或多值
- 支持结果重命名映射

#### Task D3: deadband 算子 (O25)

**目标**: 死区过滤，只在变化量超过阈值时通过
```rust
pub struct DeadbandOperator {
    deadband: f64,       // 死区阈值
    last_values: HashMap<String, f64>,  // 上次值
}
```
- 首次数据直接通过
- 后续数据：`|new - last| > deadband` 时才通过

#### Task D4: formula 算子 (O26)

**目标**: 公式解析求值，支持数学表达式
- 使用 `rhai` 或 `formicai/evalexpr`
- 公式示例: `(temperature - 32) * 5/9`, `pressure * 1.01325`
- 输入变量为当前 PipelineData 中的 tag 值

#### Task D5-D9: 轻量级算子批量实现

**目标**: 一次性实现 5 个轻量算子

| 算子 | 文件 | 核心逻辑 |
|------|------|---------|
| delay (O06) | `gateway-flow/src/operators/delay.rs` | `tokio::time::sleep()` 延迟 |
| clamp (O18) | `gateway-flow/src/operators/clamp.rs` | `value.max(min).min(max)` |
| round (O19) | `gateway-flow/src/operators/round.rs` | `precision` 位小数 |
| change (O20) | `gateway-flow/src/operators/change.rs` | 字段替换/删除 |
| range (O21) | `gateway-flow/src/operators/range.rs` | 线性映射 `output = (input - in_min) / (in_max - in_min) * (out_max - out_min) + out_min` |

#### Task D10-D14: 中等复杂度算子

| 算子 | 文件 | 核心逻辑 |
|------|------|---------|
| splitter (O11) | `gateway-flow/src/operators/splitter.rs` | 批量拆分为单条 |
| merger (O12) | `gateway-flow/src/operators/merger.rs` | 多输入汇聚为批量 |
| throttle (O14) | `gateway-flow/src/operators/throttle.rs` | 时间窗口内最多 N 条 |
| switch (O17) | `gateway-flow/src/operators/switch.rs` | 多分支条件（比 router 更强大） |
| string-ops (O27) | `gateway-flow/src/operators/string_ops.rs` | trim/substring/regex/replace |

#### Task D15: cache 算子 (O13)

**目标**: 缓存（读上一条值 / 时间窗口）
```rust
pub struct CacheOperator {
    mode: CacheMode,  // LastValue | TimeWindow
    last_values: HashMap<String, (PipelineData, i64)>,
    window_secs: u64,
}
```

#### Task D16: batch / csv-parser / binary

| 算子 | 文件 | 核心逻辑 |
|------|------|---------|
| batch (O16) | `gateway-flow/src/operators/batch.rs` | 按条数或时间攒批 |
| csv-parser (O22) | `gateway-flow/src/operators/csv_parser.rs` | CSV 编码/解码 |
| binary (O23) | `gateway-flow/src/operators/binary.rs` | 字节位操作（bit extraction/packing） |

#### Task D17: timestamp 算子 (O28)

**目标**: 时间戳操作
- `now()` → 当前时间戳
- `format(ts, fmt)` → 格式化
- `parse(str, fmt)` → 解析为时间戳
- `unix_to_datetime(ts)` / `datetime_to_unix(dt)`

#### Task D18: xml-path 算子 (O10)

**依赖**: `quick-xml` 或 `xmltree`
**目标**: XPath 路径提取（参考 json-path 模式）

#### Task D19: script-js 算子 (O07) — 沙箱安全重点

**目标**: JavaScript 脚本自定义处理
- 使用 `mujs` 引擎
- 沙箱限制：禁用 eval/Function.constructor/require/import
- 只暴露 math/json/date/string/number API
- 超时控制（100ms 脚本执行上限）
- 示例脚本：
  ```javascript
  // 对 temperature 字段做摄氏转华氏
  let fahrenheit = (data.temperature * 9/5) + 32;
  data.temp_f = fahrenheit;
  return data;
  ```

#### Task D20: script-python 算子 (O08) — 沙箱安全重点

**目标**: Python 脚本自定义处理
- 使用 `PyO3`
- 沙箱限制：禁用 os/sys/subprocess/socket/import
- 只暴露 math/json/datetime/collections/statistics
- 超时控制（100ms）
- 示例脚本：
  ```python
  import math
  # 摄氏转华氏
  data["temp_f"] = data["temperature"] * 9/5 + 32
  return data
  ```

#### Task D21: time-window 算子 (O24) — 最高复杂度

**目标**: 独立时间窗口算子（超越 aggregate 的完整窗口机制）
- Tumbling Window（滚动窗口）
- Sliding Window（滑动窗口）
- Count Window（计数窗口）
- Session Window（会话窗口，基于空闲超时）
- Watermark 机制（处理乱序数据）
- 支持窗口内: sum / avg / min / max / count / first / last

**Files**:
- Create: `gateway-flow/src/operators/time_window.rs`

---

## 四、并行实施建议

Phase 2 任务可以**四路并行**执行：

| Track | 内容 | 主要工作 |
|-------|------|---------|
| **Track 1** | 基础设施 | gateway-sdk 地址泛化 + 插件发现 |
| **Track 2** | 南向插件 | BACnet → S7 → DL/T645 → Ethernet-IP |
| **Track 3** | 北向插件 | HTTP → Kafka → InfluxDB → TDengine |
| **Track 4** | 高级算子 | alarm → json-path/deadband/formula → 轻量算子 → script-js |

---

## 五、Git 提交规范

每完成一个插件/算子单独提交：
```
feat(plugin-bacnet): add BACnet/IP south plugin with device discovery and COV subscription
feat(plugin-http): add HTTP north plugin with bearer token auth and JSON body templates
feat(operator-alarm): add alarm rule engine with threshold/rate-of-change/state-change support
```

---

## 六、验收标准

- [ ] `cargo build --workspace` 编译通过
- [ ] 每个插件可独立编译（`cargo build -p plugin-xxx`）
- [ ] alarm 算子可响应 `curl` 触发测试（模拟数据）
- [ ] `npm run build` 前端构建通过
- [ ] Web UI 插件下拉菜单能显示新增插件名称
