# 设计文档：数据面聚合扩容与累计量（第一批）

> 批次路线：本批为「规则算法补齐」三批中的**第一批（数据面 / filters.rs）**。
> 第二批（流算子：rate_of_change、去重、断流告警）与第三批（表达式：hysteresis、多条件组合、rhai 脚本专项）各自另立 spec。
> 本文自包含：实现者只需读本文与文中引用的源码文件。所有路径相对仓库根目录。

---

## 一、目标与范围

数据面（`gateway/gateway-core/src/filters.rs`）新增三类算法能力：

1. **窗口聚合扩容**：`WindowAgg.kinds` 白名单从 6 种扩到 **14 种**——新增 `sum / stddev / median / p50 / p90 / p95 / p99 / delta`；
2. **totalizer 累计量**：`GroupPolicy` 新增可选字段，跨窗口持续累计表计读数（流量/电度），支持周期重置、读数翻转、节拍发射；
3. **分位数计算性能**：median 与四个分位共用单次排序。

**动机**：市面采集网关（AWS IoT SiteWise、eKuiper 等）普遍提供 sum/stddev/分位聚合与累计量计算；本项目缺口调研（2026-11-22）确认上述为高频工业场景。

**明确不做（本批）**：
- **增量计算状态机**（eKuiper 式 incremental calculation）：只有 sum/count/avg 能真正 O(1) 增量，stddev 需二阶统计量、分位数需有序结构，收益不抵复杂度。窗口样本量上界 = `window_ms / 组 interval_ms`（典型 ≤ 数百），全量重算成本可忽略；gateway-bench 证明窗口聚合是热点后再立专项。
- 会话窗口 / 计数窗口 / 事件时间时轴（第二批语义更合适）。
- 虚拟点位与真实点位重名的冲突仲裁（现状即不处理，非本批引入）。

---

## 二、窗口聚合扩容语义（对外承诺，测试据此编写）

窗口样本集 S = 窗口内按时间升序的 `(ts, v)` 序列（现有滑窗实现，`window_ms` 内）。非空窗口上：

| kind | 定义 | 样本不足时 |
|---|---|---|
| `avg/min/max/count/first/last` | 现状不变 | 现状不变 |
| `sum` | Σv | 空窗不输出（复用现有 `aggregate() → None` 机制） |
| `stddev` | **样本标准差**，n−1 分母 | n < 2 不输出该虚拟点位 |
| `median` | 中位数；偶数样本取中间两值平均 | n ≥ 1 总有输出 |
| `p50/p90/p95/p99` | 线性插值法：`rank = p·(n−1)`，`⌊rank⌋` 与 `⌈rank⌉` 两样本线性插值（与 numpy 默认 `linear` 一致） | n ≥ 1 总有输出；n = 1 输出该样本值 |
| `delta` | 窗口**首尾差**：`last.v − first.v` | 单样本输出 **0**（语义：该窗无变化） |

- **虚拟点位命名**：沿用 `{tag_name}_{agg}`（如 `flow_counter_p95`、`flow_counter_delta`）。
- **p50 与 median 数值恒等**，两者都保留：配置习惯不同的人群各取所需。
- **kinds 全集**（校验白名单顺序）：`avg, min, max, count, first, last, sum, stddev, median, p50, p90, p95, p99, delta`。
- 聚合输出不经过死区（现状承诺，保持）。
- 运行态不落盘、重启清零（现状承诺，保持）。

---

## 三、totalizer 累计量

### 3.1 配置（`GroupPolicy` 新增可选字段）

```rust
/// 表计累计量配置；None 或空数组 = 未启用
#[serde(default)]
pub totalizer: Option<Vec<TotalizerTag>>,

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TotalizerTag {
    /// 表读数点位名（必须组内存在，校验去重）
    pub tag_name: String,
    /// 累计重置周期（毫秒）；0 = 永不重置
    #[serde(default)]
    pub reset_period_ms: u64,
    /// 表满量程读数（如电表 9999.9）；配置后读数高位回绕按翻转补差
    #[serde(default)]
    pub rollover: Option<f64>,
    /// 输出节拍（毫秒）；0 = 仅 total 变化时输出；> 0 时周期心跳强制输出
    #[serde(default)]
    pub emit_ms: u64,
}
```

- 与 `window.aggregates` **相互独立**：可只配其一。
- 输出虚拟点位 `{tag_name}_total`，走 `synthetic` 通道（与窗口聚合同通道，不经过死区）。

### 3.2 状态机（运行态不落盘；每 tag 维护）

状态：`total: f64`、`last_counter: Option<f64>`、`period_start_ms: u64`、`last_emitted_total: Option<f64>`。

每次 `apply()`（输入永远是**原始值**，与窗口聚合同口径）：

1. **首见读数**：`last_counter = new`，不累计，不输出。
2. **读数上升（new ≥ last_counter）**：`total += new − last_counter`。
3. **读数回绕（new < last_counter）**：
   - 配置了 `rollover` 且 `last_counter` 高于 `rollover` 一半 → 视为翻转：`total += (rollover − last_counter) + new`；
   - 未配置 `rollover`（或 last_counter 低于 rollover 一半）→ 视为**换表/复位**：`total` 保留不变（不累计负增量），基线 `last_counter` 重置为新读数，`tracing::warn!` 记录。
4. **周期重置**（`reset_period_ms > 0` 且 `ts_ms − period_start_ms ≥ reset_period_ms`）：**先发射本周期 total，再 `total = 0` 并同批发射 0**（两条 synthetic：旧值 = 周期关闭，0 = 新周期开启；下游以 0 为新周期起点），`period_start_ms = ts_ms`。归零后 `last_emitted_total = 0`。
5. **发射**：`last_emitted_total` 为 None（首见后的首次推进）**视为变化**，发射一次初值确立基线；此后 `total ≠ last_emitted_total` 时输出 `{tag}_total` 并更新 `last_emitted_total`；`emit_ms > 0` 且距上次输出 ≥ emit_ms 时周期强制输出。

---

## 四、实现要点

- `aggregate()` 扩展：接受样本快照 `&[(u64, f64)]`，新增 8 个 match 分支；median/p50/p90/p95/p99 共用**同一次排序**（快照排序一次 O(n log n)，四个分位查表插值，禁止各自排序）。
- `FilterState::GroupFilterState` 新增 `totalizers: RwLock<HashMap<TagId, TotalizerState>>`；`synthetic_ids` 缓存机制复用（`{tag}_total` 同样走名称→虚拟 TagId 缓存）。
- `FilterOutcome` 不变（totalizer 输出并入 `synthetic` + `synthetic_names`）。
- `forget_group / forget_node / clear_all` 语义不变（连同 totalizer 状态一起清除，现状实现已整组删除，天然覆盖）。
- `numeric_f64` 口径复用：非数值点位不进 totalizer（校验时对非数值声明类型给出告警性提示，不拒绝——点位 data_type 可为 None）。

---

## 五、校验（`validate_policy` 扩展）

- kinds 白名单扩为 14 种（§二全集），未知 kind 错误信息列出现有合法值。
- `totalizer` 每项：
  - `tag_name` 非空；同组内 `totalizer` 数组**去重拒绝**（错误信息含点位名，风格与现有 tags 去重一致）；
  - `rollover` 必须有限正数（`> 0.0 && is_finite`）；
  - `reset_period_ms / emit_ms` 为 u64 天然合法，无下限校验。
- 点位 `data_type` 声明为非数值类型时不拒绝（点位可能后改），运行期按 `numeric_f64` 口径跳过并 warn。
- `totalizer` 的 tag_name **不要求**在组内已存在（点位可能后建），与 `window` 对点位不预校验的现状一致。

---

## 六、测试计划（每功能随批带测试，P9 教训）

沿用文件头「对外承诺 + 确定性 property test（xorshift 固定种子）」模式：

**单元**：
1. 8 种新聚合与手算对照：sum；stddev 与公式对照（n=2、n=5）；median 奇/偶样本；p90/p95 插值精确值（含 rank 落在两样本之间的构造样本）；p50 == median；delta 首尾差；delta 单样本 = 0；stddev n=1 不输出。
2. totalizer：单调累计；翻转补差（配置 rollover）；未配 rollover 值下降重计 + warn；周期重置「先发后清」（重置边界上一次输出是旧 total）；仅变化发射（连续不变不输出）；emit_ms 心跳强制输出；首见读数不输出。
3. 校验：14 种白名单接受、非法 kind 拒绝；totalizer 去重拒绝、rollover 非法拒绝。

**Property**：
- 非空窗口发射时 `synthetic.len() == 启用的 kinds 数`（现有不变量推广到 14 种）；
- passthrough 恒等回归；
- totalizer 守恒：`total` 对任意读数序列只增不减，周期重置边界除外；翻转补差后的 total 与「逐段展开翻转的真实用量」一致。

**回归**：现有 6 种聚合、死区、心跳、passthrough、forget 全部测试保持通过。

**前端**：
- `web/src/views/DataMonitor.vue` 策略表单：kinds 多选扩 8 项（分位与 delta 分组展示）；totalizer 配置段（点位选择 + reset_period_ms + rollover + emit_ms）；
- `web/src/locales/zh.js / en.js` 同步；
- `types/api.d.ts` GroupPolicy 类型同步。

**API / 文档**：
- 零新增端点（policy PUT/GET 沿用）；`gateway/gateway-server/src/api/openapi.rs` 的 GroupPolicy schema 同步；`docs/架构说明.md` 数据面一节补一段。

---

## 七、批次路线图（备忘，另立 spec）

| 批次 | 内容 | 主要落点 |
|---|---|---|
| 第二批（流算子） | `rate_of_change` 算子（速度窗口语义专项设计）、`deduplicate` 去重算子、**断流告警**（组采集超时未上报 → Alarm 事件，复用 alarm_to_data 通道） | `gateway-flow/src/operators.rs`、engine |
| 第三批（表达式） | 告警回差 hysteresis（触发/恢复阈值分离）、多点位 AND/OR 条件组合、rhai 脚本算子（沙箱专项——历史教训 P7：`59e88c6` 加固后砍除，需先定沙箱边界） | `rules.rs`、`operators.rs` |
