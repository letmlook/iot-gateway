#!/usr/bin/env bash
# 编译网关并自动生成 license（与后端一致的机器码逻辑，输出到 data/license.dat）
# 用法：在项目根目录执行 ./scripts/build_with_license.sh [--release]

set -e
cd "$(dirname "$0")/.."

RELEASE=""
for a in "$@"; do
  [ "$a" = "--release" ] && RELEASE="--release"
done

if [ -n "$RELEASE" ]; then
  cargo build --release -p gateway-server
else
  cargo build -p gateway-server
fi

python3 scripts/gen_license_auto.py || python scripts/gen_license_auto.py || {
  echo "gen_license_auto failed (pip install cryptography? scripts/license_private.pem?)" >&2
  exit 1
}
echo "License written to data/license.dat"
