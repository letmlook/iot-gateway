# IoT 网关

自研设备数据采集网关：南向设备、北向应用均通过插件实现。  
后端 **Rust**，前端 **Vue 3 + JavaScript**。支持 SQLite 持久化、REST API、Web 管理台、插件静态/动态加载。

## 快速开始

```bash
# 后端（workspace 根目录）
cargo build --release
./target/release/gateway   # 默认 http://0.0.0.0:3000

# 可选环境变量
# GATEWAY_PORT=8080  GATEWAY_DATA_DIR=./data  GATEWAY_PLUGINS_DIR=./plugins  RUST_LOG=info

# 带 license 构建（机器码与当前一致，输出到 data/license.dat）
# Windows: .\scripts\build_with_license.ps1 [--release]
# Linux/mac: ./scripts/build_with_license.sh [--release]

# 前端（构建后由网关服务 web/dist）
cd web && npm install && npm run build
```

开发时可选：`cd web && npm run dev`（Vite 将 `/api` 代理到 `:3000`）。

## 测试与 CI

```bash
cargo test --workspace          # 单元测试 + server 集成测试（23 个）
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings   # CI 强制门禁
cargo fmt --all -- --check      # 存量格式问题，暂为提示
```

- 单元测试：`gateway-sdk`（Schema 校验、敏感字段识别与脱敏）、`gateway-core`（快照往返、敏感配置落盘加密与解密）
- 集成测试：`gateway-server` 用 `tower::ServiceExt::oneshot` 直接打 axum Router，覆盖认证 401/200、`/data-flow` 需鉴权、静态资源路径穿越防护、`/api/*` 未知路径返回 404 JSON、RBAC 角色授权（viewer 不可写值/管用户/恢复备份）
- CI：`.github/workflows/ci.yml`（backend check+test 强制 / clippy -D warnings 强制 / fmt 提示 / 前端构建）

## 配置与持久化

- **配置**：支持 `config/gateway.json`（或 `GATEWAY_CONFIG` 指定路径）及环境变量；优先级：默认 → 配置文件 → 环境变量。
- **持久化**：节点 / 组 / 标签 / 订阅写入 **SQLite** `data/data.db`，启动加载、变更自动保存；无 DB 时若有旧版 `data.json` 会自动迁移。支持 `/api/backup` 备份恢复、保存前自动备份 `data.db.bak`。

## 安全基线

部署前请确认以下各项（对应本仓库的安全默认值）：

| 项 | 默认行为 | 说明 |
|---|---|---|
| API 认证 | **开启**（`disable_auth: false`） | 关闭时所有 `/api` 匿名可访问，包含写值、恢复备份、用户管理；关闭会在启动日志打印 WARN |
| 初始管理员 | **随机口令** | 首次初始化生成 20 位随机口令，写入 `data/.admin_initial_password`（0600）并打印到日志，首次登录后应立即改密并删除该文件 |
| 登录限流 | 连续失败 5 次锁定 5 分钟 | 防暴力破解 |
| 会话失效 | 改密 / 删除用户 / 登出 token 立即失效 | 新增 `POST /api/auth/logout` |
| CORS | **默认拒绝所有跨域来源** | 需跨域时用 `GATEWAY_ALLOWED_ORIGINS`（逗号分隔）显式配置白名单 |
| 静态资源 | 拒绝 `..` 穿越与绝对路径 | 未匹配的 `/api/*` 返回 404 JSON，不再回退 index.html |
| 备份密钥 | 未设置时**随机生成** | 仓库中不再有硬编码默认密钥；跨实例恢复必须显式设置 `GATEWAY_BACKUP_SECRET` |
| License 私钥 | **禁止入库** | `scripts/*.pem` 已加入 `.gitignore` 并从 Git 索引移除 |
| 角色授权 | **默认开启** | 读 Viewer+ / 写与写值 Operator+ / 用户·备份恢复·授权 Admin；`GATEWAY_ENFORCE_ROLES=0` 可临时关闭 |

> ⚠️ 历史遗留：仓库曾提交过签名私钥 `scripts/license_private.pem`。该私钥已从版本控制移除但**视为已泄露**，
> 请执行 `./scripts/rotate_license_keys.sh` 轮换密钥对，替换 `license.rs` 中的公钥并重新签发所有 `license.dat`。

### 环境变量（安全相关）

| 变量 | 默认 | 说明 |
|---|---|---|
| `GATEWAY_DISABLE_AUTH` | 未设置（开启认证） | 设为 `1`/`true` 关闭认证，仅限本地调试 |
| `GATEWAY_TOKEN` | 无 | 静态 Bearer Token；比较采用恒定时间算法 |
| `GATEWAY_ALLOWED_ORIGINS` | 空 | 允许的跨域来源，逗号分隔 |
| `GATEWAY_BACKUP_SECRET` | 无（随机） | 备份加密密钥，跨实例恢复时必填 |
| `GATEWAY_BIND` | `0.0.0.0` | 监听地址，建议内网部署时绑定内网网卡 |

## 项目结构

| 目录 | 说明 |
|------|------|
| `gateway/gateway-sdk` | 插件 trait（SouthPlugin / NorthPlugin）、Tag/Group/DataValue、消息类型、FFI 约定 |
| `gateway/gateway-core` | 消息总线 Bus、Store、Manager、持久化、节点启停与路由、数据流统计 |
| `gateway/gateway-server` | REST API、静态资源、Config、license、用户与认证、main |
| `gateway/gateway-plugins/` | 南/北向插件（见下表） |
| `web/` | Vue3 + Vite 管理台（概览、南/北向节点、组/标签/订阅、数据监控、写值、插件、系统、导出等） |

### 插件

| 插件 | 类型 | 说明 |
|------|------|------|
| plugin-sim | 南向 | 模拟设备，默认组/标签 |
| plugin-modbus-tcp | 南向 | Modbus TCP |
| plugin-modbus-rtu | 南向 | Modbus RTU |
| plugin-opcua | 南向 | OPC UA（可选 opcua-client feature） |
| plugin-virb | 南向 | 联能 YE6235D/YE6235D2 振动采集，UDP |
| plugin-mqtt | 北向 | MQTT：QoS 0/1/2、主题模板、TLS、离线缓存与补发、多种上报格式（values_format/tags_format/ecp_format/group_data/raw_data） |

插件可按需编译为 .so 放入 `GATEWAY_PLUGINS_DIR`，启动时扫描加载；未提供 .so 时使用内置静态链接。

## 文档

- [架构说明](docs/架构说明.md)
- [SDK 与插件开发](docs/SDK与插件开发.md)
- [功能实现清单](docs/功能实现清单.md)（已实现 / 规划中 / 不做，以代码为准）
- [功能总结与待完善列表](docs/功能总结与待完善列表.md)
- [南向北向业务逻辑](docs/南向北向业务逻辑梳理.md)
- [部署与发布](docs/部署与发布.md)（构建产物、环境变量、容器部署、授权与升级）
- [性能基线](docs/bench/README.md)（`gateway-bench` 与历史基线）
- [设计评审与优化方案](docs/review/优化方案总览-2026-09-26.md)

## License

MIT
