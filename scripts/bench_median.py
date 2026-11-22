#!/usr/bin/env python3
"""合并多次 gateway-bench --json 输出，按指标取中位数（docs/bench/README 口径的自动化）。

用法：
    python3 scripts/bench_median.py run1.json run2.json run3.json > latest.json

规则：
  - 同名指标：数值取中位数（消单次运行的机器噪声）；unit 取首个报告的；
  - 非数值指标（纯文本值）无法取中位：全部一致则保留该值，否则保留首个并在
    stderr 提示（对比脚本对非数值行本来就跳过）；
  - 指标集取并集（某次缺失的指标按有值的那几次计算）；
  - 元数据（os/arch/rustc/date）取最后一个报告（代表本批最近一次运行）。
"""
import json
import statistics
import sys
from collections import Counter


def main() -> int:
    if len(sys.argv) < 2:
        print("usage: bench_median.py <run1.json> [run2.json ...]", file=sys.stderr)
        return 1
    reports = []
    for p in sys.argv[1:]:
        with open(p, encoding="utf-8") as f:
            reports.append(json.load(f))

    # 按指标名聚合各次取值
    gathered: dict[str, list] = {}
    units: dict[str, str] = {}
    for rep in reports:
        for m in rep["metrics"]:
            gathered.setdefault(m["metric"], []).append(m["value"])
            units.setdefault(m["metric"], m["unit"])

    metrics = []
    for name in sorted(gathered):
        values = gathered[name]
        numeric = [v for v in values if isinstance(v, (int, float))]
        if numeric:
            # 数值取中位数；混入的字符串形态值丢弃（同指标应同型，属于异常）
            med = statistics.median(numeric)
            if len(numeric) != len(values):
                print(f"WARN: {name} 混有非数值样本，仅按 {len(numeric)} 个数值取中位", file=sys.stderr)
            value = med
        else:
            # 全为字符串：一致则保留，不一致取第一个（对比脚本会跳过非数值行）
            top, n = Counter(values).most_common(1)[0]
            if n < len(values):
                print(f"WARN: {name} 非数值且取值不一致，保留首个：{values[0]!r}", file=sys.stderr)
            value = values[0] if n < len(values) else top
        metrics.append({"metric": name, "value": value, "unit": units[name]})

    out = dict(reports[-1])  # 元数据取最后一批
    out["metrics"] = metrics
    json.dump(out, sys.stdout, ensure_ascii=False, indent=2)
    sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
