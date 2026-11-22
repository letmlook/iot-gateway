# 设计文档：流算子（rate_of_change / deduplicate / 断流告警）第二批

> 批次路线：本批为「规则算法补齐」三批中的**第二批（流算子 / gateway-flow）**。
> 第一批（数据面 filters.rs）已完成合并；第三批（表达式：hysteresis、多条件组合、rhai 沙箱专项）另立 spec。
> 本文自包含：实现者只需读本文与文中引用的源码文件。所有路径相对仓库根目录。

---

## 一、目标与范围

`gateway/gateway-flow` 新增两个算子，并为流程引擎补上"无数据也能触发"的检测能力：

1. **rate_of_change 算子**：点位变化速率（单位/秒）超限告警——触发型 + 相邻拍单差分；
2. **deduplicate 算子**：整帧去重——与上一帧完全相同的数据帧不再向下游转发；
3. **断流告警**：source 声明的南向组超过配置时长未上报数据 → Alarm 事件，走既有告警通道（sink_alarm 落库 / alarm_to_data 北向通知），数据恢复自动 resolve。

**动机**：缺口调研（2026-11-22）确认变化率告警与断流检测是采集网关高频需求；去重复用 filters.rs on_change 的 flow 侧场景（同一数据要同时走"原值转发"与"告警"两条分支时，on_change 在 filters 层做不了 flow 分支粒度）。

**明确不做（本批）**：
- rate 的窗口差分（已确认单差分；窗口语义 filters 层已有）；
- dedup 的 tag 白名单与单点位裁剪（已确认整帧）；
- 断流告警的人工确认流（已确认自动恢复）；
- 恢复事件的下游通知（resolve 只改告警状态，告警中心可见；不做"恢复帧"注入图）。

---

## 二、rate_of_change 算子语义（对外承诺，测试据此编写）

- **配置**（`RateOfChangeConfig`）：
  - `tag_name: String`——监视点位名（非空）；
  - `max_rate: f64`——速率上限（**正值**，单位 = 值/秒）；validate 要求有限且 `> 0.0`；
  - `level: String`——告警等级（`AlarmLevel::parse` 合法值）；
  - `message: Option<String>`——自定义消息；缺省自动生成。
- **端口**：`in → alarm | normal`（threshold_alarm 同构）。
- **求值**（每次帧到达）：
  1. 取 `value_by_name(gd, tag_name)` 并 `as_f64`；缺值或非数值 → **走 normal 端**（缺测不误触发，route 同口径），**且不更新速率状态**；
  2. 首见读数（该 node+tag 无 prev 状态）→ 记录 `(v, ts)`，走 normal 端，不输出告警；
  3. 速率 `rate = (v − v_prev) / ((ts − ts_prev) / 1000.0)`，时间取 `GroupData.ts` 的毫秒；`ts == ts_prev`（同时间戳）→ 视速率 0（防除零）；
  4. `rate.abs() > max_rate` → alarm 端：`AlarmEvent { level, tag, value: Some(v), threshold: Some(max_rate), message }`，message 缺省格式 `"{}={} rate={:.3}/s crossed max {}/s"`（tag、v、rate、max_rate 顺序）；否则 normal 端；
  5. **每次拿到有效数值都更新 prev 状态**（无论是否触发）。
- **状态**：`FlowState` 新增 `last_rate: HashMap<Uuid, HashMap<String, (f64, u64)>>`——外键 = flow 节点 id，内键 = tag_name，值 = (prev_v, prev_ts_ms)。运行态不落盘、重启清零（与 last_values 同语义）。

---

## 三、deduplicate 算子语义

- **配置**：`{}`——无字段（空 object）。
- **端口**：`in → out`（单端口）；重复帧**无输出**，下游节点静默跳过（engine 既有语义：上游端口无输出则 continue）。
- **求值**：
  1. 首帧（该节点无快照）→ 转发完整帧并记录快照；
  2. 后续帧与快照**按位相等**：`values` 长度相同且逐位 `(TagId, DataValue)` 相等（DataValue 按值比较，含类型；若 sdk 未 derive PartialEq 则手写比较函数）→ **抑制**（无输出）；
  3. 任一差异（任一位的值/类型/TagId 不同，或长度不同）→ 转发完整帧并更新快照。
- **已知边界（不特判）**：含 NaN 的帧永不与快照相等（NaN ≠ NaN），每帧转发——与逐值比较语义一致。
- **状态**：`FlowState` 新增 `last_frames: HashMap<Uuid, Vec<(TagId, DataValue)>>`——键 = flow 节点 id，值 = 上一帧 `gd.values` 快照。

---

## 四、断流告警（source watchdog + 引擎注入）

### 4.1 配置（`SourceConfig` 扩展）

```rust
pub struct SourceConfig {
    pub south_node_id: Uuid,
    pub group_id: Uuid,
    /// 断流检测超时（毫秒）；0 或缺省 = 不检测
    #[serde(default)]
    pub stale_timeout_ms: u64,
    /// 断流告警等级；缺省 "high"；stale_timeout_ms > 0 时必须是 AlarmLevel 合法值
    #[serde(default = "default_stale_level")]
    pub stale_level: String,
}
```

### 4.2 检测循环（`gateway-server/src/flow_runtime.rs`）

- 消费循环改 `tokio::select!`：
  - 分支一：`rx.recv()` 收到帧 → 正常 `evaluate` + 分发；随后若存在 active 断流告警 → `alarm_store.resolve(id, tenant)` 并清除记录；刷新 `deadline = now + stale_timeout_ms`；
  - 分支二：`sleep_until(deadline)` 到期 → 构造断流 `AlarmEvent` → 注入图（§4.3）分发；**`deadline` 顺延一个 `stale_timeout_ms`**（持续断流周期性 re-raise；AlarmStore 同键 active 去重只累计 occurrences，不产生新条目）。
- runtime 记住最近一次断流 raise 返回的 alarm id（`Option<Uuid>`，任务局部变量），用于自动 resolve。
- **断流 AlarmEvent 构造**：`level = stale_level`；`tag = 组名`（runtime 经 Manager store 按 `(south_node_id, group_id)` 查组名，查不到则用 group_id 字符串）；`value = None`；`threshold = Some(stale_timeout_ms as f64)`；`message` 缺省格式 `"组 {组名} 超过 {秒} 秒未上报数据"`；`ts = Utc::now()`。
- `stale_timeout_ms = 0`（未配置）时保持现有 `rx.recv().await` 直收循环（不引入 select 开销）——实现可统一为 select 分支二永不到期（deadline = far future），以实现简洁为准。

### 4.3 告警进图（`engine::evaluate` 注入参数）

`evaluate` 签名扩展：

```rust
pub fn evaluate(
    plan: &ExecutionPlan,
    flow: &Flow,
    input: &GroupData,
    state: &mut FlowState,
    inject: Option<&AlarmEvent>,   // 新增；None = 普通数据帧
) -> Result<Vec<SinkDelivery>, FlowError>
```

注入模式（`inject = Some(ev)`）下各节点的输入语义：

| 节点 | 注入模式输入 |
|---|---|
| `alarm_to_data` 算子 | `FlowPayload::Alarm(ev.clone())` → 产出 `("out", Data)` 沿图正常传播（下游 sink_north 推送） |
| `sink_alarm` sink | 直接收 `Alarm(ev)` → 正常进入 deliveries |
| 其他算子（threshold/route/state_change/dedup/rate） | 无输入 → 静默跳过（缺测不误触发） |
| Source | 不输出 |

- 纯函数性保持（crate 无 tokio/无 I/O 不变）；下游传播复用现有分层循环，`flow_runtime` 的 deliveries 分发代码零改动（`SinkTarget::Alarm + FlowPayload::Alarm` 路径已有）。
- runtime 调用点：数据帧 `inject = None`；断流超时 `inject = Some(&ev)`。
- 图中既无 `sink_alarm` 也无 `alarm_to_data` 时，断流告警无处投递（deliveries 为空）——publish 门禁校验（§五）拒绝此类配置。

---

## 五、校验

- `validate_config`：
  - `rate_of_change`：tag_name 非空；max_rate 有限且 > 0；level 合法（`AlarmLevel::parse`）。
  - `deduplicate`：空 object 通过。
- `validate_source_config` 扩展：`stale_timeout_ms > 0` 时 `stale_level` 必须是 AlarmLevel 合法值；`stale_timeout_ms = 0` 时 `stale_level` 不校验。
- **publish 门禁**（server 接线层）：source 配置了 `stale_timeout_ms > 0` 的流程，图中必须存在至少一个 `sink_alarm` 或 `alarm_to_data` 节点——否则拒绝发布（错误信息说明断流告警无投递出口）。
- `PortSpec::operator`：`rate_of_change` → `alarm/normal`；`deduplicate` → `out`。
- `OPERATOR_NAMES` 6 → 8（追加 `rate_of_change`、`deduplicate`）。

---

## 六、测试计划（每功能随批带测试）

沿用现有模式：纯函数单测 + 确定性断言（gateway-flow tests 内联在模块下）。

**单元（gateway-flow）**：
1. rate_of_change：首见不触发；上升超速触发（构造 Δv/Δt 精确值）；下降超速触发（`|rate|`）；正常速率不触发；缺值走 normal 且**不更新状态**（下一帧恢复有效值时按更早的 prev 计算）；同 ts 速率 0；message 缺省格式。
2. deduplicate：首帧转发；完全相同帧抑制；任一点位值变化转发；点位长度变化转发；NaN 帧每帧转发（边界）。
3. inject 语义：alarm_to_data 节点收到 Alarm → 产出 Data 且下游 sink_north 收到；sink_alarm 收到 Alarm；threshold_alarm 节点静默；普通调用（inject=None）行为与现状完全一致（回归）。
4. 校验：rate config 各非法分支拒绝；dedup 空 config 通过；source stale_level 非法拒绝。

**runtime（gateway-server）**：
- deadline / 断流事件构造提取为可测纯函数并单测（组名回退 group_id、message 格式、re-raise deadline 计算）。
- select 循环本身不写时间驱动测试（tokio time pause 收益不抵脆弱性；逻辑已提取到纯函数）。

**回归**：现有 flow 测试全部保持通过（evaluate 签名变更后全部调用点同步 `inject = None`）。

**前端**：
- 流程编辑器从 `/flows/operators` 发现算子（既有机制，前端不硬编码算子清单——踩坑 P6 对策）；计划阶段核实 per-operator 配置表单是否硬编码，若硬编码则补 rate_of_change / deduplicate 表单与 source 面板 stale 字段（zh/en i18n 同步）。

**文档**：`docs/架构说明.md` 流程算子清单补段；`docs/功能实现清单.md` 更新。

---

## 七、批次路线图（备忘，另立 spec）

| 批次 | 内容 | 主要落点 |
|---|---|---|
| 第三批（表达式） | 告警回差 hysteresis（触发/恢复阈值分离）、多点位 AND/OR 条件组合、rhai 脚本算子（沙箱专项——历史教训 P7：`59e88c6` 加固后砍除，需先定沙箱边界） | `rules.rs`、`operators.rs` |
