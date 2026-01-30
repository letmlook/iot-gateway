@echo off
REM 使用机器码生成 license.dat，输出到 data/license.dat
cd /d "%~dp0.."
python scripts\gen_license.py --machine-id ed7ee4e17be180620a10907a2481deae79bfe84f24af69b2ef5cb3105fa8d668 -o data\license.dat
if %ERRORLEVEL% neq 0 (
  echo 若提示找不到 python，请先安装 Python 并执行: pip install cryptography
  pause
  exit /b %ERRORLEVEL%
)
echo.
echo 已生成 data\license.dat，重启网关即可生效。
pause
