# Gateway SDK 说明

> 南/北向插件 SDK 详细说明。

---

## 一、SDK 模块概览

| 模块 | 说明 |
|------|------|
| `error` | `PluginError`、`PluginErrorCode`、`PluginResult` |
| `types` | `DataValue`、`DataType`、`Tag`、`Group`、`NodeId` 等 |
| `schema` | `ConfigSchema`、`ParamSchema`、`TagRegexEntry`、`TagSchema` |
| `messages` | `GroupData`、`GroupSubscription`、`TagRead`、`TagWrite` |
| `plugin` | `SouthPlugin`、`NorthPlugin`、`PluginMeta` |

---

## 二、生命周期（open/close/init/uninit/start/stop/setting）

### 南向 SouthPlugin

| 接口 | 时机 |
|------|------|
| `open(node_id, config)` | 创建 node 时首先调用 |
| `close(node_id)` | 删除 node 时最后调用 |
| `init(node_id)` | open 之后 |
| `uninit(node_id)` | 删除 node 时首先调用 |
| `start(node_id)` | 用户点击「启动」 |
| `stop(node_id)` | 用户点击「停止」 |
| `setting(node_id, config)` | 用户修改插件配置 |

### 北向 NorthPlugin

同 `open` / `close` / `init` / `uninit` / `start` / `stop` / `setting`，语义类似；无 `validate_tag`、`poll_group`、`write_tags`。

---

## 三、南向采集与读写

| 接口 | 说明 |
|------|------|
| `poll_group(node_id, group_id, tags)` | 按 Group 定时采集，返回 `(TagId, DataValue)[]` |
| `write_tags(node_id, values)` | 写 Tag，默认 `NotSupported` |
| `validate_tag(node_id, tag)` | 添加/更新 tag 时校验，默认通过 |
| `list_groups` / `list_tags` | 默认 groups/tags（如 sim） |

---

## 四、统一数据类型 DataValue / DataType

- **DataValue**：`Bool`、`Int8`～`Int64`、`UInt8`～`UInt64`、`Float32`/`Float64`、`String`、`Bytes`。  
  提供 `as_f64`、`as_i64`、`as_u64`、`as_bool`、`as_string`、`data_type()`。
- **DataType**：`DataType::Float64` 等，`as_str()`、`Display`。  
  用于 Schema、校验、UI。

---

## 五、ConfigSchema / TagSchema

### ConfigSchema

- `params: Vec<ParamSchema>`：配置参数列表。
- `tag_regex: Option<Vec<TagRegexEntry>>`：按数据类型配置地址正则。
- `validate_address(data_type, address) -> bool`：用 `tag_regex` 校验地址。

### ParamSchema

- `name`、`description`、`attribute`(required/optional)、`type`(int/string/bool)、`default`、`valid`(min/max/regex/length)。

### TagSchema

- `data_types`、`address_format`：用于 UI 与校验提示。

### API

- `GET /api/plugins/south/:name/config_schema`
- `GET /api/plugins/south/:name/tag_schema`

---

## 六、PluginError / PluginErrorCode

| Code | 含义 |
|------|------|
| `Unknown` | 通用 |
| `ConfigInvalid` | 配置无效 |
| `TagInvalid` | 点位校验失败 |
| `ConnectionFailed` | 连接失败 |
| `Timeout` | 超时 |
| `NotSupported` | 操作不支持（如写） |
| `Io` | IO 错误 |
| `ValidationFailed` | 校验失败 |

构造：`PluginError::msg(s)`、`PluginError::tag_invalid(s)`、`PluginError::config_invalid(s)` 等。

---

## 七、消息与总线

- **GroupData**：`node_id`、`group_id`、`ts`、`values: (TagId, DataValue)[]`。南向发布，北向按订阅消费。
- **GroupSubscription**：`(south_node_id, group_id)`。北向订阅表。
- **TagRead** / **TagWrite**：读/写请求结构，预留 API 与扩展。

---

## 八、插件开发步骤

1. 新建 crate，依赖 `gateway-sdk`。
2. 实现 `SouthPlugin` 或 `NorthPlugin`（含 `meta`、`open`/`close`，及 `init`/`uninit`/`start`/`stop`/`setting` 按需覆盖）。
3. 南向可选：`config_schema`、`tag_schema`、`validate_tag`；实现 `poll_group`，按需 `write_tags`。
4. 北向：`set_subscriptions`、`on_group_data`。
5. 在 `gateway-server` 的 `main` 中 `register_south` / `register_north`，并加入 workspace。

---

## 九、实现要点

| 项目 | 本 SDK |
|------|--------|
| 实现语言 | Rust，静态链接 crate 或 .so |
| 模块导出 | `PluginMeta` + trait 实现 |
| 配置 Schema | `config_schema()` 返回 `ConfigSchema` |
| 生命周期 | open→init→start / stop→uninit→close，`async` |
| 校验 | `validate_tag` + 可选 `ConfigSchema::validate_address` |
