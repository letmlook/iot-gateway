# 性能基线

本目录保存 `gateway-bench` 的历史基线，用于回答「这次改动到底快了没有」。

## 怎么跑

```bash
./scripts/bench.sh                 # 完整规模 → docs/bench/baseline-<日期>.md
./scripts/bench.sh --quick         # 小规模（约 1/10），用于提交前快速回归
cargo run --release -p gateway-bench            # 只看结果，不落盘
```

CI 中由 `.github/workflows/bench-nightly.yml` 每晚跑一次并存为构建产物。

## 怎么读

| 指标 | 关注点 |
|---|---|
| 总线发布 / 端到端 | 分区后投递成本应与订阅者数量无关；端到端数值应远高于现场所需（单组 1s 周期 × 千级组 ≈ 10³ msg/s） |
| 总线背压丢弃 | **不是缺陷指标**：它证明 `Lagged` 语义生效。生产要看 `gateway_north_lagged_by_node` 告警 |
| 持久化保存 / 加载 | 保存是热路径（每次配置变更触发），加载只在启动时发生 |
| Modbus 合并规划 | 「请求/周期」才是现场最关心的数字：1000 点位从 1000 次往返降到个位数 |
| FFI 边界 JSON 往返 | 量化「JSON over C ABI」的固有成本，用于判断高频点位是否需要更紧凑的编码 |

## 注意

- 绝对值随机器、后台负载、CPU 频率策略波动明显（同一机器重复运行差异可达 2 倍）。
  因此**只用于同机同负载的前后对比**，跨机器比较没有意义；建议各跑 3 次取中位数。
- 压测程序是 release 构建；debug 构建的数字没有参考价值。
