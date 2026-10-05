@echo off
REM 构建所有插件 .dll 并复制到 plugins/
REM 包含 sim/mqtt/modbus-tcp/modbus-rtu/opcua/virb/http/influxdb/tdengine
REM Kafka 需要 CMake + librdkafka：自动跳过（不阻塞其他插件）
REM 调用 PowerShell 脚本执行实际构建

powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0build-plugins.ps1"
exit /b %ERRORLEVEL%
