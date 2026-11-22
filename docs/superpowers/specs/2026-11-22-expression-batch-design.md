# 设计文档：表达式批（hysteresis 回差 / 多条件组合 / rhai expr_route）第三批

> 批次路线：本批为「规则算法补齐」三批中的**第三批（表达式）**。
> 第一批（数据面 filters.rs）、第二批（流算子 gateway-flow）已完成合并。
> 本文自包含：实现者只需读本文与文中引用的源码文件。所有路径相对仓库根目录。

---

## 一、目标与范围

1. **hysteresis 回差**（`gateway/gateway-core/src/rules.rs`）：规则可选配**独立恢复条件**——触发与恢复阈值分离，防临界抖动；
2. **多条件组合**（rules.rs）：触发条件从单点位扩展为「主条件 + 追加条件」扁平列表，all/any 逻辑；
3. **rhai expr_route 算子**（`gateway/gateway-flow`）：布尔表达式对组内点位求值路由（route 的表达式版），**沙箱边界专项定死**（P7 教训：`59e88c6` 时代 python 子进程算子加固后砍除；rhai 为纯 Rust 嵌入式脚本，无子进程/无 IO 面）。

**明确不做（本批）**：
- 表达式树（AND/OR 任意嵌套）——扁平列表 + all/any 已覆盖；
- 对称回差值（threshold ∓ hysteresis 自动推导）——独立恢复条件语义更显式；
- rhai expr_transform（返回 map 变换数据帧）——输出 schema 设计另立专项；
- 规则管理前端 UI（规则本就无界面，API 层使用）；
- python/子进程脚本算子（P7 教训，永不复活）。

---

## 二、hysteresis 独立恢复条件（对外承诺，测试据此编写）

### 2.1 配置

```rust
/// 独立恢复条件（hysteresis）。None = 现状语义（触发条件不满足 + clear_ms 去抖）。
#[serde(default)]
pub clear_condition: Option<ClearCondition>,

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClearCondition {
    /// 恢复条件监测的点位；None = 主点位（source.tag_name）
    #[serde(default)]
    pub tag_name: Option<String>,
    pub op: CompareOp,
    pub threshold: f64,
}
```

### 2.2 状态机语义（在现有 `advance()` 骨架上，仅恢复分支变化）

设 `triggered = logic([主条件] + extra_conditions)`（§三）：

1. **triggered 为真**：现状不变——清 `clear_since_ms`，`since_ms` 计时，持续 ≥ `for_ms` 且未 fired → 触发（fired=true）。
2. **fired 且 triggered 为假**（恢复判定）：
   - **配了 `clear_condition`**：`cleared = clear_condition.eval(values)`（点位缺位 → false）。`cleared` 为真 → `clear_since_ms` 计时，持续 ≥ `clear_ms` → **重新武装**（fired=false、since/clear_since 清零）；`cleared` 为假 → **保持 fired 且 `clear_since_ms = None`**（要求恢复条件**持续**满足，中途反弹重新计时）。
   - **未配 `clear_condition`**：现状不变——进入分支即视为恢复计时，持续 ≥ `clear_ms` 重新武装。
3. **未 fired 且 triggered 为假**：现状不变——重置 since/clear_since。

**不变量（测试钉住）**：临界带（触发条件已不满足、恢复条件未满足）期间规则**保持 fired**，动作不重复执行也不提前重新武装；恢复条件中途由真变假时 clear 计时归零重计。

---

## 三、多条件组合

### 3.1 配置

```rust
/// 追加触发条件（与主条件组成条件组，按 logic 求值）；空 = 现状单条件
#[serde(default)]
pub extra_conditions: Vec<TagCondition>,

/// 条件组逻辑；单条件时 all/any 等价（serde lowercase：all | any）
#[serde(default)]
pub logic: Logic,

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TagCondition {
    pub tag_name: String,
    pub op: CompareOp,
    pub threshold: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Logic { All, Any }
```

- 主条件 = 现有 `condition` 字段（监测 `source.tag_name`）；`extra_conditions` 为追加项；**旧规则 JSON（无新字段）经 serde default 零迁移兼容**。
- **触发判定**：`logic` 作用于 `[主条件, ...extra_conditions]` 全体——All = 全部满足，Any = 任一满足。

### 3.2 缺位口径

- 主点位缺位（本次采集无该点位）：**整条规则不推进状态**（现状 continue，不变）；
- 主点位在、extra 点位缺位：该 extra 条件判 **false**（All 下整体不满足，Any 下看其余项）。

### 3.3 与 hysteresis 的组合

正交可组合：多条件触发 + 独立单条恢复条件（恢复条件监测点位由其 `tag_name` 指定，缺省主点位）。

---

## 四、rhai expr_route 算子（沙箱专项）

### 4.1 配置与端口

- 配置：`ExprRouteConfig { expression: String }`（expression 非空）。
- 端口：`in → true | false`。
- `OPERATOR_NAMES` 8 → 9（追加 `expr_route`）；`PortSpec::operator` 加 true/false 双端口分支。

### 4.2 数据暴露

- rhai 变量 `tags`：`rhai::Map`，键 = tag_name，值按 DataValue 映射——
  Float32/Float64/Int8..Int64/UInt8..UInt64 → `f64`；`Bool` → `bool`；`String` → `String`；其余（Bytes 等）→ `()`（null）。
- **不注册任何自定义函数**（纯语言子集，最小沙箱面）。示例：`tags.temp > 80 && tags.humi < 30`。

### 4.3 沙箱边界（定死，实现与审查据此核对）

```rust
let mut engine = rhai::Engine::new();      // 默认无文件/无目录/无网络/无系统访问
engine.set_max_operations(10_000);         // 防死循环（超限 → 求值错误）
engine.set_max_call_levels(16);            // 防深递归
engine.set_max_expr_depths(32, 32);        // 防深嵌套表达式
engine.set_max_string_size(4_096);         // 防内存爆炸
engine.set_max_array_size(64);
```

### 4.4 求值口径（缺测不误触发）

- 结果 `Ok(bool)`：true → true 端；false → false 端；
- 求值错误（编译期错误已在 publish 拦截，运行期错误如对缺位点位的访问、类型不匹配、**超操作数**）或返回值非 bool：**走 false 端**；
- 日志：同节点同 AST **首次**错误 `warn!`，后续同类错误 `debug!`（防高频采集日志爆炸），求值成功后重置——实现为 AST 缓存条目带 `warned: bool` 标志。

**实施修正（终审实证）**：rhai 1.27 对 map 缺失属性的点访问得 UNIT，`tags.ghost > 10` 这类比较**静默判 false**（返回 Ok(false)——非求值错误、无日志）；上文「首错 warn 后续 debug」仅覆盖真运行期错误（如对 UNIT 做算术运算、类型不匹配、超操作数）。点位拼写错误因此无任何观测信号，观测增强列入 backlog。

### 4.5 AST 缓存

`FlowState` 新增 `script_asts: HashMap<Uuid, CachedScript>`（键 = flow 节点 id）：

```rust
struct CachedScript {
    expression: String,   // 缓存 key：config 的 expression 字符串
    ast: rhai::AST,
    warned: bool,         // §4.4 首次错误 warn 标志
}
```

- 求值时 config.expression 与缓存 `expression` 一致 → 复用 AST；否则重新编译并替换；
- flow republish = 任务重建（FlowState 默认）→ 缓存自然失效；运行态不落盘语义不变。

### 4.6 发布门禁（P7 教训前移）

`validate_config("expr_route", ...)`：非空校验 + **编译校验**（`engine.compile(expression)`，只编译不执行），编译失败拒绝发布，错误信息含 rhai 诊断。运行期语义错误不在编译期拦截（依赖缺位数据等，按 §4.4 口径处理）。

### 4.7 依赖

workspace 根 `Cargo.toml` 加 `rhai`（最新 1.x 稳定版；具体版本号在计划阶段核实锁定）。纯 Rust、无 unsafe 依赖，二进制体积影响可接受。

**实施修正（Task 2 审查裁决）**：依赖实际落在 `gateway/gateway-flow/Cargo.toml` 直接声明（本仓库无 `[workspace.dependencies]` 集中段，非 `workspace = true` 方式）；且实际启用了 `sync` feature——`Engine`/`AST` 需跨 `tokio::spawn`（Send bound）；`sync` 仅将内部 `Rc` 换为 `Arc`，沙箱语义与边界不变，上文「不开任何 feature」以本条为准。

---

## 五、校验

**Rule::validate 扩展**：
- `extra_conditions` 每项：tag_name 非空、threshold 有限；**数量上限 8**；
- `clear_condition`：tag_name 若有须非空、threshold 有限；
- 现有校验全部保留。

**expr_route**：§4.6（非空 + 编译试运行）。

**publish 门禁**：无新增规则（expr_route 走既有算子校验通道）。

---

## 六、测试计划（每功能随批带测试）

**单元（gateway-core rules）**：
1. hysteresis 全时序：>80 触发（for_ms=0）；80~75 临界带保持 fired 不重武装不重复 fire；<75 持续 clear_ms 重新武装；恢复条件中途反弹（75→76→75）clear 计时归零；恢复条件指定非主点位。
2. 多条件：All 两条全满足触发、任一不满足不触发；Any 任一满足触发；extra 点位缺位判 false（All 整体 false / Any 看其余）；主点位缺位整条不推进（runtime 无变化）。
3. 组合：多条件触发 + clear_condition 恢复。
4. 兼容：旧 JSON（仅 condition 字段）反序列化 → extra 空、logic=all、clear_condition=None，advance 行为与现状一致（回归）。
5. 校验：extra_conditions 超 8 项拒绝；任一 threshold 非有限拒绝（含 clear_condition）；clear_condition.tag_name 空串拒绝。

**单元（gateway-flow expr_route）**：
6. true/false 路由（`tags.temp > 50`）；缺位 tag 访问走 false；非 bool 返回（如 `tags.temp`）走 false；语法错误（`tags.temp >`）validate 拒绝发布；无限循环 `while true {} ` 超操作数走 false（沙箱生效证明）；表达式变更后 AST 重编译（新表达式生效）。
7. 首错 warn 后续 debug 不便断言（tracing 断言成本高）——以实现正确性审查为准，不写日志断言。

**回归**：现有 rules 测试（rules.rs 全部）、flow 测试、全量 workspace。

**前端**：FlowEditor expr_route 表单（expression 多行文本）+ zh/en i18n；palette 自动发现。

**文档**：`docs/架构说明.md`（规则一节补 hysteresis/多条件、流程算子清单补 expr_route + 沙箱边界）；`docs/功能实现清单.md` 更新。

---

## 七、路线图备忘

三批全部完成后，「规则算法补齐」专题收官。后续 backlog（终审与各批审查遗留）：
- M-2：stale=None 365d deadline 不 re-arm（一行）
- M-3：超大 stale_timeout_ms 溢出 panic 钳制
- M-5：flow 算子 message 空串视同 None 统一
- rate NaN 盲一拍文档句；窗口聚合不拦非有限值（第一批 M5，backlog）
