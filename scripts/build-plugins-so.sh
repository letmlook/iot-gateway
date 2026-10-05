#!/bin/bash
# 构建北向/南向插件 .so 并复制到 plugins/
# Kafka 需要 librdkafka + CMake，单独构建：cargo build -p plugin-kafka --features "ffi,kafka-client"
set -e
cd "$(dirname "$0")/.."
mkdir -p plugins
cargo build -p plugin-sim -p plugin-mqtt -p plugin-modbus-tcp -p plugin-modbus-rtu -p plugin-opcua -p plugin-virb -p plugin-http -p plugin-influxdb -p plugin-tdengine --features ffi
cp target/debug/libplugin_sim.so target/debug/libplugin_mqtt.so \
   target/debug/libplugin_modbus_tcp.so target/debug/libplugin_modbus_rtu.so \
   target/debug/libplugin_opcua.so target/debug/libplugin_virb.so \
   target/debug/libplugin_http.so target/debug/libplugin_influxdb.so \
   target/debug/libplugin_tdengine.so plugins/
# Kafka 单独构建（可能失败）
cargo build -p plugin-kafka --features "ffi,kafka-client" 2>/dev/null && \
   cp target/debug/libplugin_kafka.so plugins/ || \
   echo "WARN: kafka plugin build failed (requires CMake + librdkafka), skipping"
echo "plugins: $(ls plugins/*.so 2>/dev/null || true)"
