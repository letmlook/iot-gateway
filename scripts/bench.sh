#!/usr/bin/env bash
# 运行性能基线压测，结果写入 docs/bench/
#
#   ./scripts/bench.sh              # 完整规模
#   ./scripts/bench.sh --quick      # 小规模，用于本地快速回归
#
# 说明：数值受机器与后台负载影响，横向比较请在**同一台机器、同一负载**下各跑 3 次取中位数。
set -euo pipefail
cd "$(dirname "$0")/.."

mkdir -p docs/bench
OUT="docs/bench/baseline-$(date +%F).md"

echo "==> 构建 release 压测程序"
cargo build --release -p gateway-bench

echo "==> 运行压测，输出 ${OUT}"
cargo run --release -p gateway-bench -- "$@" --out "${OUT}"

echo "==> 完成：${OUT}"
