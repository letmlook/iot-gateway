# IoT 网关

自研设备数据采集网关：南向设备、北向应用均通过插件实现。  
后端 **Rust**，前端 **Vue 3 + JavaScript**。架构对标 NeuronEX，支持 SQLite 持久化、REST API、Web 管理台、.so 插件热加载。

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

## 配置与持久化

- **配置**：支持 `config/gateway.json`（或 `GATEWAY_CONFIG` 指定路径）及环境变量；优先级：默认 → 配置文件 → 环境变量。常用：`GATEWAY_PORT`、`GATEWAY_DATA_DIR`、`GATEWAY_STATIC_DIR`、`GATEWAY_PLUGINS_DIR`、`GATEWAY_TOKEN`（API 认证）、`GATEWAY_DISABLE_AUTH=1`（关闭认证）、`RUST_LOG`。
- **持久化**：节点 / 组 / 标签 / 订阅写入 **SQLite** `data/data.db`，启动加载、变更自动保存；无 DB 时若有旧版 `data.json` 会自动迁移。支持 `GET /api/export` 导出快照、保存前自动备份 `data.db.bak`。

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

- [架构说明](docs/架构与Neuron对标.md)
- [SDK 与 Neuron 对标](docs/SDK与Neuron对标.md)
- [功能总结与待完善列表](docs/功能总结与待完善列表.md)
- [南向北向业务逻辑](docs/南向北向业务逻辑梳理.md)

## License

MIT
