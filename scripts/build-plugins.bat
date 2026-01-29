@echo off
REM 构建 sim/mqtt 插件 .dll 并复制到 plugins/
REM 调用 PowerShell 脚本执行实际构建

powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0build-plugins.ps1"
exit /b %ERRORLEVEL%
