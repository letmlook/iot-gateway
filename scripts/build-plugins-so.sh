#!/bin/bash
# 构建 sim/mqtt/modbus-tcp/modbus-rtu/opcua 插件 .so 并复制到 plugins/
# OPC UA 完整功能需：cargo build -p plugin-opcua --features "ffi,opcua-client"

set -e
cd "$(dirname "$0")/.."
mkdir -p plugins
cargo build -p plugin-sim -p plugin-mqtt -p plugin-modbus-tcp -p plugin-modbus-rtu -p plugin-opcua --features ffi
cp target/debug/libplugin_sim.so target/debug/libplugin_mqtt.so target/debug/libplugin_modbus_tcp.so target/debug/libplugin_modbus_rtu.so target/debug/libplugin_opcua.so plugins/
echo "plugins: $(ls plugins/*.so 2>/dev/null || true)"
