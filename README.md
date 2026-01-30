# IoT 网关

自研设备数据采集网关：南向设备、北向应用均通过插件实现。  
后端 **Rust**，前端 **Vue 3 + JavaScript**。

## 快速开始

```bash
# 后端
cargo build --release
./target/release/gateway   # http://0.0.0.0:3000

# 可选：环境变量
# GATEWAY_PORT=8080  GATEWAY_DATA_DIR=./data  RUST_LOG=info

# 编译并自动生成 license（与当前机器一致的机器码，输出到 data/license.dat）
# Windows: .\scripts\build_with_license.ps1 [--release]
# Linux/mac: ./scripts/build_with_license.sh [--release]

# 前端（构建后由网关服务 web/dist）
cd web && npm install && npm run build
```

开发时可选：`cd web && npm run dev`（Vite 将 `/api` 代理到 `:3000`）。

## 配置与持久化

- **配置**：`GATEWAY_PORT`、`GATEWAY_DATA_DIR`、`GATEWAY_STATIC_DIR`、`RUST_LOG`（见 [架构说明](docs/架构与Neuron对标.md)）。
- **持久化**：节点 / 组 / 标签 / 订阅写入 `data/data.json`，启动加载、变更自动保存。

## 项目结构

- `gateway/gateway-sdk`：插件 trait、Tag/Group/DataValue、消息类型
- `gateway/gateway-core`：消息总线、Store、Manager、持久化、节点启停与路由
- `gateway/gateway-server`：REST API、静态资源、Config、AppState、main
- `gateway/gateway-plugins/`：南/北向插件（plugin-sim、plugin-mqtt、plugin-modbus-tcp、plugin-modbus-rtu、plugin-opcua）
- `web/`：Vue3 + Vite 管理台

## 文档

- [IoT 网关调研报告](docs/IoT网关调研报告.md)
- [架构说明](docs/架构与Neuron对标.md)
- [SDK 说明](docs/SDK与Neuron对标.md)

## License

MIT
