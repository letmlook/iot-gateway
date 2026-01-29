# 构建 sim/mqtt/modbus/opcua 插件 .dll 并复制到 plugins/
# Windows 下使用此脚本（Linux/macOS 使用 build-plugins-so.sh）
# OPC UA 完整功能需：cargo build -p plugin-opcua --features "ffi,opcua-client"

$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot\..

if (-not (Test-Path plugins)) {
    New-Item -ItemType Directory -Path plugins | Out-Null
}

cargo build -p plugin-sim -p plugin-mqtt -p plugin-modbus-tcp -p plugin-modbus-rtu -p plugin-opcua --features ffi

Copy-Item target\debug\plugin_sim.dll plugins\
Copy-Item target\debug\plugin_mqtt.dll plugins\
Copy-Item target\debug\plugin_modbus_tcp.dll plugins\
Copy-Item target\debug\plugin_modbus_rtu.dll plugins\
Copy-Item target\debug\plugin_opcua.dll plugins\

Write-Host "plugins: $((Get-ChildItem plugins\*.dll -ErrorAction SilentlyContinue | ForEach-Object { $_.FullName }) -join ', ')"
Get-ChildItem plugins\*.dll -ErrorAction SilentlyContinue
