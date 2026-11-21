#!/usr/bin/env python3
"""nightly bench 基线对比（T4，蓝图 M3）。

用法：
    python3 scripts/bench_compare.py <base.json> <cur.json> [--threshold 0.5]

对每个同名指标比较 cur / base 比值；数值恶化超过阈值（默认 0.5 = 变慢 50%）
时告警。指标改善（比值 < 1）不告警。非数值指标与只在单侧出现的指标跳过
（单侧缺失常因新增/移除基准项）。

输出（供 workflow 消费）：
  - 逐指标对比表到 stdout
  - GITHUB_OUTPUT：alert=true/false 与告警摘要
退出码恒为 0（告警与否由 alert 输出驱动后续 step，夜间任务不因此变红——
基线对比是「提示」性质，且 runner 负载噪声大，避免误报打断）。
"""
import argparse
import json
import os
import sys

# 单机重复运行差异可达 2 倍（docs/bench/README 说明），runner 同规格波动小些；
# 阈值默认 0.5（恶化 50%）在「能发现真实回退」与「不被噪声误报」之间取平衡。
DEFAULT_THRESHOLD = 0.5


def load(path: str) -> dict:
    with open(path, encoding="utf-8") as f:
        return json.load(f)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("base")
    ap.add_argument("cur")
    ap.add_argument("--threshold", type=float, default=DEFAULT_THRESHOLD)
    args = ap.parse_args()

    base = {m["metric"]: m for m in load(args.base)["metrics"]}
    cur = {m["metric"]: m for m in load(args.cur)["metrics"]}

    rows = []
    alerts = []
    for name in sorted(set(base) | set(cur)):
        b, c = base.get(name), cur.get(name)
        if b is None:
            rows.append(f"  + {name}: 新增基准（无基线可比）")
            continue
        if c is None:
            rows.append(f"  - {name}: 本次未跑（基线 {b['value']} {b['unit']}）")
            continue
        # 只比较数值型（同为 int/float）；时间类指标值越小越好、吞吐类越大越好——
        # 统一按「数值增大 = 恶化」处理：吞吐 msg/s 增大是改善会误报，故按 unit 分流
        if not all(isinstance(m["value"], (int, float)) for m in (b, c)):
            rows.append(f"  = {name}: 非数值，跳过")
            continue
        bv, cv = float(b["value"]), float(c["value"])
        if bv <= 0:
            rows.append(f"  = {name}: 基线为 0，跳过")
            continue
        ratio = cv / bv
        # 吞吐/速率类（值越大越好）用倒数比较：越大 → ratio<1；越慢/越大越坏的指标保持原序
        bigger_is_better = any(
            k in b["unit"].lower() for k in ("msg/s", "ops/s", "/s")
        )
        degrade_ratio = 1.0 / ratio if bigger_is_better else ratio
        mark = "✓"
        if degrade_ratio > 1 + args.threshold:
            mark = "✗"
            alerts.append(
                f"{name}: {cv:g} {c['unit']} vs 基线 {bv:g} {b['unit']} "
                f"（恶化 {degrade_ratio:.2f}x，阈值 {1 + args.threshold:.2f}x）"
            )
            rows.append(f"  {mark} {name}: {cv:g} {c['unit']} vs {bv:g} {b['unit']} → 恶化 {degrade_ratio:.2f}x")
        else:
            rows.append(f"  {mark} {name}: {cv:g} {c['unit']} vs {bv:g} {b['unit']} → {ratio:.2f}x")

    print(f"bench 对比（阈值：恶化 > {1 + args.threshold:.2f}x）")
    print("\n".join(rows))
    if alerts:
        print(f"\n告警 {len(alerts)} 项：")
        print("\n".join(f"  - {a}" for a in alerts))
    else:
        print("\n全部指标在阈值内。")

    out = os.environ.get("GITHUB_OUTPUT")
    if out:
        with open(out, "a", encoding="utf-8") as f:
            f.write(f"alert={'true' if alerts else 'false'}\n")
            f.write(f"summary={'; '.join(alerts) if alerts else 'ok'}\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
