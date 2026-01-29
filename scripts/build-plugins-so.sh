#!/bin/bash
# 构建 sim/mqtt 插件 .so 并复制到 plugins/

set -e
cd "$(dirname "$0")/.."
mkdir -p plugins
cargo build -p gateway-plugin-sim -p gateway-plugin-mqtt --features ffi
cp target/debug/libgateway_plugin_sim.so target/debug/libgateway_plugin_mqtt.so plugins/
echo "plugins: $(ls plugins/*.so 2>/dev/null || true)"
