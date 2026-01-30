#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
自动生成与后端一致的机器码并生成 license 到 data 目录。
逻辑与 gateway-server 的 license/hardware.rs 完全一致：先构建并运行 get_machine_id 示例得到机器码，再调用 gen_license.py 生成加密的 license.dat。

用法：
  1. 在项目根目录执行：python scripts/gen_license_auto.py
  2. 编译时自动生成：运行 scripts/build_with_license.ps1（Windows）或 scripts/build_with_license.sh（Linux/mac），先编译再生成 license
  3. 或设置 GEN_LICENSE=1 后 cargo build -p gateway-server，由 build.rs 在编译时调用本脚本（需已能构建通过）

默认生成的功能为 all_plugins（允许所有插件）。依赖：Rust 工具链（cargo）、Python 及 cryptography（pip install cryptography），scripts/license_private.pem。
"""

import os
import subprocess
import sys
from pathlib import Path


def project_root() -> Path:
    return Path(__file__).resolve().parent.parent


def main() -> int:
    root = project_root()
    os.chdir(root)

    # 1) 构建 get_machine_id 示例（与后端 hardware.rs 相同逻辑）
    print("Building get_machine_id example...")
    r = subprocess.run(
        ["cargo", "build", "--example", "get_machine_id", "-p", "gateway-server"],
        cwd=root,
        capture_output=True,
        text=True,
    )
    if r.returncode != 0:
        print("Build failed:", r.stderr or r.stdout, file=sys.stderr)
        return 1

    # 2) 运行示例获取机器码
    exe = root / "target" / "debug" / "examples" / "get_machine_id"
    if sys.platform == "win32":
        exe = exe.with_suffix(".exe")
    if not exe.is_file():
        print("Binary not found:", exe, file=sys.stderr)
        return 1

    out = subprocess.run([str(exe)], cwd=root, capture_output=True, text=True)
    machine_id = (out.stdout or out.stderr or "").strip()
    if not machine_id:
        print("get_machine_id produced no output", file=sys.stderr)
        return 1
    print("Machine ID:", machine_id[:16] + "...")

    # 3) 输出目录
    data_dir = root / "data"
    data_dir.mkdir(parents=True, exist_ok=True)
    license_path = data_dir / "license.dat"

    # 4) 调用 gen_license.py 生成加密 license
    script_dir = root / "scripts"
    gen_license = script_dir / "gen_license.py"
    if not gen_license.is_file():
        print("gen_license.py not found:", gen_license, file=sys.stderr)
        return 1

    r2 = subprocess.run(
        [
            sys.executable,
            str(gen_license),
            "--machine-id",
            machine_id,
            "--features",
            "all_plugins",
            "-o",
            str(license_path),
        ],
        cwd=root,
    )
    if r2.returncode != 0:
        print("gen_license.py failed (e.g. missing cryptography or license_private.pem)", file=sys.stderr)
        return 1

    print("License written to:", license_path)
    return 0


if __name__ == "__main__":
    sys.exit(main())
