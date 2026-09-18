# #706 增量 E 成本对照证据（审阅者 §9.4）

> 状态：**部分完成**（2026-09-19）。本文件先登记方法论与已完成测量；
> 未完成项逐条标注「未测」与原因，不以推测充数。

## 测量口径

- 正常 release 构建、非插桩（无 cfg(test) 诊断计数、无统计探针）；
  与插桩诊断（cfg(test) 块级记录、路径计数）分开跑、分开记。
- 场景：`examples/preview_parallel_wall_clock` 的 multi-gate 1_024 活动车
  （补员保持稳态，`multi_gate_scene::replenish`），warmup 24 拍 +
  测量 128 拍 × 3 rounds，逐拍整步墙钟取 p50/p95。
- 机器：9955HX 16C/32T；测量期间无其他编译负载。

## 已完成：新实现 w1 vs w2/w4/w8/w16（并行机制净效应）

`cargo run --release -p laneflow-runtime --example preview_parallel_wall_clock`

原始输出（整步 ns，scene=multi-gate-1024）：

```text
preview-wall scene=multi-gate-1024 workers=1  round=0 whole_p50_ns=1004900 whole_p95_ns=1712600
preview-wall scene=multi-gate-1024 workers=1  round=1 whole_p50_ns=952400  whole_p95_ns=1819600
preview-wall scene=multi-gate-1024 workers=1  round=2 whole_p50_ns=768000  whole_p95_ns=1316200
preview-wall scene=multi-gate-1024 workers=2  round=0 whole_p50_ns=1150900 whole_p95_ns=1852400
preview-wall scene=multi-gate-1024 workers=2  round=1 whole_p50_ns=1232100 whole_p95_ns=1935900
preview-wall scene=multi-gate-1024 workers=2  round=2 whole_p50_ns=1079000 whole_p95_ns=1767400
preview-wall scene=multi-gate-1024 workers=4  round=0 whole_p50_ns=1006600 whole_p95_ns=1649800
preview-wall scene=multi-gate-1024 workers=4  round=1 whole_p50_ns=1017900 whole_p95_ns=1735000
preview-wall scene=multi-gate-1024 workers=4  round=2 whole_p50_ns=1157300 whole_p95_ns=1972900
preview-wall scene=multi-gate-1024 workers=8  round=0 whole_p50_ns=1096900 whole_p95_ns=1762300
preview-wall scene=multi-gate-1024 workers=8  round=1 whole_p50_ns=1061600 whole_p95_ns=1884700
preview-wall scene=multi-gate-1024 workers=8  round=2 whole_p50_ns=1160900 whole_p95_ns=1952600
preview-wall scene=multi-gate-1024 workers=16 round=0 whole_p50_ns=1788500 whole_p95_ns=2665800
preview-wall scene=multi-gate-1024 workers=16 round=1 whole_p50_ns=1580000 whole_p95_ns=2504400
preview-wall scene=multi-gate-1024 workers=16 round=2 whole_p50_ns=1739300 whole_p95_ns=2678000
```

| workers | p50 中位（ns） | p50 区间        | p95 中位（ns） | p95 区间        |
| ------- | -------------- | --------------- | -------------- | --------------- |
| 1       | 952400         | 768000–1004900  | 1712600        | 1316200–1819600 |
| 2       | 1150900        | 1079000–1232100 | 1852400        | 1767400–1935900 |
| 4       | 1017900        | 1006600–1157300 | 1735000        | 1649800–1972900 |
| 8       | 1096900        | 1061600–1160900 | 1762300        | 1762300–1952600 |
| 16      | 1739300        | 1580000–1788500 | 2665800        | 2504400–2678000 |

结论（multi-gate-1024，阈边界场景）：w2–w8 的整步墙钟与 w1 在噪声带内
持平（p50 中位 +7%～+21%，round 间方差同量级），w16 明显回退
（+63%～+82%，超订：16 worker × 三相 × 块 setup 超出本场景每拍实际
并行工作）。本场景逐车工作极小（空区间 Waiting 候选 + 直线运动），
分发固定成本（输入发现、槽位、完整 join × 3）抵消并行收益——与「阈值
保持保守、城市级认证归 #707」的登记一致；本数据不构成生产性能通过。

读法要点：

- 1_024 活动车等于三个相位的生产分发阈值（P2/P3/P5 均保守 1_024），
  ≥ 阈值的拍自然走真实分发，无 cfg(test) 强制入口——本组即「正常
  release 非插桩」下的并行机制净效应证据。
- 该场景每拍三个分发阶段（P2 预览 + P3 候选 + P5 运动）全部在
  阈值之上分发；P4 acquire、rebuild frontier、串行尾保持协调器串行。

## 未测项与原因

| 对照组                                                                    | 状态 | 原因                                                                                                                                 |
| ------------------------------------------------------------------------- | ---- | ------------------------------------------------------------------------------------------------------------------------------------ |
| b99a282e 单 worker vs 新实现单 worker（组织成本提取）                     | 未测 | 需第二检出的 release 构建（基线提交在 main），本次 2 小时预算不足以完成两次全量 release 构建 + 稳定测量；建议增量 F 或 #707 前置时补 |
| b99a282e w4 vs 新实现 w4（相对仅 P2 并行的整步改善）                      | 未测 | 同上                                                                                                                                 |
| P3-only / P5-only 消融（E1 开关）                                         | 未测 | E1 开关为 cfg(test) 私有旋钮，release example 不可见；消融需把开关提升为执行配置项或构建 test-profile 墙钟设施，属独立工作项         |
| 各阶段实际工作/最长块/参与线程/回退次数/暂存字节/串行部分（release 形态） | 未测 | 现有阶段计时（performance_profile）为 cfg(test) 插桩；release 形态需要非插桩阶段计时出口，随上一条一起立项                           |

## 阈值登记（三个保守初值）

- `WAITING_PREVIEW_DISPATCH_MIN_ACTIVE`（P2）、
  `CONFLICT_DISPATCH_MIN_ACTIVE`（P3）、`MOTION_DISPATCH_MIN_ACTIVE`
  （P5）三个独立常量均为保守初值 1_024（#706 全部增量一致）。
- 依据已测数据：multi-gate-1024 场景在阈值边界（等于 1_024）每拍三相
  全分发、输出与融合参照逐拍一致（`preview_at_threshold_equivalence`
  1_280 车集成证据 + lib 强制分发对拍）。
- 结论：**保持保守，不在本增量调整**。精确校准需要城市级规模数据
  （#707 认证管线），且审阅者 §9.4 明确「登记即可，不追求调优」。
