#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
离线授权文件生成脚本（模拟后端签名过程）。
完全本地运行，不发起任何网络请求。
生成的授权文件使用 AES-256-GCM 加密，防止用户直接查看内容。

用法：
  1. 首次生成密钥对（二选一）：
     - Rust： cargo run --example gen_license_keys -p gateway-server
       会生成 scripts/license_private.pem 并打印公钥，将公钥粘贴到 gateway-server/src/license/license.rs 的 BUILTIN_PUBLIC_KEY_PEM。
     - Python： python gen_license.py --gen-keys
       同上，并将输出的公钥粘贴到 license.rs。
  2. 生成授权文件：
     python gen_license.py --machine-id <机器码> --expiry 2026-12-31 --features "premium_export,batch_process,pro_tool"
     python gen_license.py --machine-id <机器码> --expiry 2026-12-31 --features "all_plugins" --max-tags 1000
     或从管道读取机器码： echo <机器码> | python gen_license.py --expiry 2026-12-31 --features "pro_tool"
     机器码可从客户端 GET /api/license/machine-id 获取。
  3. 将输出文件保存为数据目录下的 license.dat，重启网关即可生效。

依赖： pip install -r requirements.txt  或  pip install cryptography
"""

import argparse
import base64
import json
import os
import sys
from pathlib import Path

# AES-256-GCM 加密密钥（32 字节）- 必须与 Rust 后端 license.rs 中的密钥一致
LICENSE_AES_KEY = b"IoTGateway@2024!SecretKey#Lic_32"

def _ensure_cryptography():
    try:
        from cryptography.hazmat.primitives import hashes, serialization
        from cryptography.hazmat.primitives.asymmetric import padding, rsa
        from cryptography.hazmat.primitives.ciphers.aead import AESGCM
        return hashes, serialization, padding, rsa, AESGCM
    except ImportError:
        print("请安装依赖: pip install cryptography", file=sys.stderr)
        sys.exit(1)


def default_key_paths():
    script_dir = Path(__file__).resolve().parent
    return script_dir / "license_private.pem", script_dir / "license_public.pem"


def gen_keys(private_path, public_path):
    hashes, serialization, padding, rsa, _ = _ensure_cryptography()
    key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
    pem_private = key.private_bytes(
        encoding=serialization.Encoding.PEM,
        format=serialization.PrivateFormat.PKCS8,
        encryption_algorithm=serialization.NoEncryption(),
    )
    pem_public = key.public_key().public_bytes(
        encoding=serialization.Encoding.PEM,
        format=serialization.PublicFormat.SubjectPublicKeyInfo,
    )
    with open(private_path, "wb") as f:
        f.write(pem_private)
    with open(public_path, "wb") as f:
        f.write(pem_public)
    print("密钥已保存:", private_path, public_path)
    print("\n请将以下公钥粘贴到 gateway-server/src/license/license.rs 的 BUILTIN_PUBLIC_KEY_PEM：\n")
    print(pem_public.decode("utf-8"))


def encrypt_license(data: bytes) -> bytes:
    """使用 AES-256-GCM 加密授权文件内容
    返回格式：nonce(12字节) + ciphertext
    """
    _, _, _, _, AESGCM = _ensure_cryptography()
    aesgcm = AESGCM(LICENSE_AES_KEY)
    nonce = os.urandom(12)  # GCM 推荐使用 12 字节 nonce
    ciphertext = aesgcm.encrypt(nonce, data, None)
    return nonce + ciphertext


def sign_license(private_path, machine_id: str, expiry_date: str, features: list, max_tags: int = None):
    """生成加密的授权文件（二进制格式）"""
    hashes, serialization, padding, rsa, _ = _ensure_cryptography()
    with open(private_path, "rb") as f:
        key = serialization.load_pem_private_key(f.read(), password=None)
    payload = {
        "machineId": machine_id,
        "expiryDate": expiry_date,
        "features": features,
    }
    # 仅当设置了 max_tags 且大于 0 时添加到 payload
    if max_tags is not None and max_tags > 0:
        payload["maxTags"] = max_tags
    payload_bytes = json.dumps(payload, separators=(",", ":")).encode("utf-8")
    signature = key.sign(payload_bytes, padding.PKCS1v15(), hashes.SHA256())
    license_obj = {
        "payload_b64": base64.standard_b64encode(payload_bytes).decode("ascii"),
        "signature_b64": base64.standard_b64encode(signature).decode("ascii"),
    }
    # 将 JSON 加密
    json_bytes = json.dumps(license_obj, separators=(",", ":")).encode("utf-8")
    encrypted = encrypt_license(json_bytes)
    return encrypted


def main():
    parser = argparse.ArgumentParser(description="离线授权文件生成（RSA 签名 + AES 加密）")
    parser.add_argument("--gen-keys", action="store_true", help="生成 RSA 密钥对并输出公钥")
    parser.add_argument("--private-key", default=None, help="私钥 PEM 文件路径（默认 scripts/license_private.pem）")
    parser.add_argument("--machine-id", default=None, help="机器码（与客户端 GET /api/license/machine-id 一致）")
    parser.add_argument("--expiry", default="2030-12-31", help="到期日期 YYYY-MM-DD")
    parser.add_argument("--features", default="premium_export,batch_process,pro_tool", help="功能列表，逗号分隔")
    parser.add_argument("--max-tags", type=int, default=None, help="最大点位数限制（不设置或0表示无限制）")
    parser.add_argument("-o", "--output", default=None, help="输出 license.dat 路径（必须指定，加密后为二进制文件）")
    args = parser.parse_args()

    priv_path, pub_path = default_key_paths()
    if args.gen_keys:
        gen_keys(priv_path, pub_path)
        return

    private_path = args.private_key or str(priv_path)
    if not os.path.isfile(private_path):
        print("未找到私钥文件:", private_path, file=sys.stderr)
        print("请先运行: python gen_license.py --gen-keys", file=sys.stderr)
        sys.exit(1)

    machine_id = args.machine_id
    if not machine_id and not sys.stdin.isatty():
        machine_id = sys.stdin.read().strip()
    if not machine_id:
        print("请提供 --machine-id 或从标准输入传入机器码", file=sys.stderr)
        sys.exit(1)

    features = [s.strip() for s in args.features.split(",") if s.strip()]
    encrypted_data = sign_license(private_path, machine_id, args.expiry, features, args.max_tags)
    
    if args.output:
        with open(args.output, "wb") as f:
            f.write(encrypted_data)
        print("已写入加密授权文件:", args.output, file=sys.stderr)
        print(f"  - 机器码: {machine_id[:16]}...", file=sys.stderr)
        print(f"  - 到期日期: {args.expiry}", file=sys.stderr)
        print(f"  - 功能: {', '.join(features)}", file=sys.stderr)
        if args.max_tags:
            print(f"  - 最大点位数: {args.max_tags}", file=sys.stderr)
        print(f"  - 文件大小: {len(encrypted_data)} 字节", file=sys.stderr)
    else:
        print("错误: 加密授权文件为二进制格式，必须使用 -o 参数指定输出路径", file=sys.stderr)
        print("示例: python gen_license.py --machine-id <机器码> -o license.dat", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
