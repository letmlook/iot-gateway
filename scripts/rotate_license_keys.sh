#!/usr/bin/env bash
# License 密钥对轮换脚本。
#
# 背景：仓库曾把签名私钥 scripts/license_private.pem 一并提交，任何人都能签出任意机器码 /
# 任意功能的 license.dat，授权体系形同虚设。该私钥已从 Git 索引移除，但因已泄露，
# **必须轮换**：生成新密钥对，并把新公钥写入 gateway-server/src/license/license.rs 的
# BUILTIN_PUBLIC_KEY_PEM，之后重新签发所有客户的 license.dat。
#
# 用法：
#   ./scripts/rotate_license_keys.sh            # 生成到 scripts/ 下（私钥已被 .gitignore 忽略）
#   ./scripts/rotate_license_keys.sh --force    # 覆盖已存在的密钥文件
#
# 依赖：openssl

set -euo pipefail

cd "$(dirname "$0")/.."

PRIVATE_KEY="scripts/license_private.pem"
PUBLIC_KEY="scripts/license_public.pem"
LICENSE_RS="gateway/gateway-server/src/license/license.rs"

if [[ ! -f "$PRIVATE_KEY" ]] || [[ "${1:-}" == "--force" ]]; then
  openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:2048 -out "$PRIVATE_KEY"
  openssl rsa -in "$PRIVATE_KEY" -pubout -out "$PUBLIC_KEY"
  echo "已生成新密钥对：$PRIVATE_KEY / $PUBLIC_KEY"
else
  echo "私钥已存在，跳过生成（使用 --force 覆盖）。"
  [[ -f "$PUBLIC_KEY" ]] || openssl rsa -in "$PRIVATE_KEY" -pubout -out "$PUBLIC_KEY"
fi

chmod 600 "$PRIVATE_KEY"
echo
echo "=== 请将下面这段公钥写入 $LICENSE_RS 的 BUILTIN_PUBLIC_KEY_PEM ==="
echo
cat "$PUBLIC_KEY"
echo
echo "=== 轮换后必须做的事 ==="
echo "1) 用新私钥为所有客户重新签发 license.dat：python3 scripts/gen_license.py ..."
echo "2) 旧公钥签发的所有 license 立即失效，请提前通知客户"
echo "3) 确认私钥不会被提交：git status --short scripts/"
