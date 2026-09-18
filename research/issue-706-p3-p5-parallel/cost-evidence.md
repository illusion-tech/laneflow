# #706 增量 E 成本对照证据（审阅者 §9.4，R6 修正版）

> 状态：两轮测量完成（返工前 + R1/R2/R4 修复后），含 b99a282e 有界对照。
> 未测项逐条标注；正常 release 非插桩，与 cfg(test) 诊断分开跑、分开记。

## 测量口径

- 正常 release 构建、非插桩（无 cfg(test) 诊断计数、无统计探针）。
- 场景：`examples/preview_parallel_wall_clock` 的 multi-gate 1_024 活动车
  （补员保持稳态），warmup 24 拍 + 测量 128 拍 × 3 rounds，逐拍整步墙钟
  p50/p95；worker ∈ {1,2,4,8,16}。
- 基线：同一 example 的 `b99a282e` 版本（#717 主干，仅 P2 预览分发；
  P3/P5 为 #706 新增）在独立 worktree 同机测量。
- 机器：9955HX 16C/32T；两组测量串行执行、无并发编译负载。

## 新实现（R1/R2/R4 修复后重测）原始输出

```text
preview-wall scene=multi-gate-1024 workers=1  round=0 whole_p50_ns=760600 whole_p95_ns=1282800
preview-wall scene=multi-gate-1024 workers=1  round=1 whole_p50_ns=767800 whole_p95_ns=1388000
preview-wall scene=multi-gate-1024 workers=1  round=2 whole_p50_ns=751000 whole_p95_ns=1226900
preview-wall scene=multi-gate-1024 workers=2  round=0 whole_p50_ns=825800 whole_p95_ns=1305400
preview-wall scene=multi-gate-1024 workers=2  round=1 whole_p50_ns=812200 whole_p95_ns=1287500
preview-wall scene=multi-gate-1024 workers=2  round=2 whole_p50_ns=844500 whole_p95_ns=1305000
preview-wall scene=multi-gate-1024 workers=4  round=0 whole_p50_ns=737200 whole_p95_ns=1211700
preview-wall scene=multi-gate-1024 workers=4  round=1 whole_p50_ns=747100 whole_p95_ns=1278400
preview-wall scene=multi-gate-1024 workers=4  round=2 whole_p50_ns=738900 whole_p95_ns=1233600
preview-wall scene=multi-gate-1024 workers=8  round=0 whole_p50_ns=811100 whole_p95_ns=1579000
preview-wall scene=multi-gate-1024 workers=8  round=1 whole_p50_ns=864700 whole_p95_ns=1277100
preview-wall scene=multi-gate-1024 workers=8  round=2 whole_p50_ns=799600 whole_p95_ns=1257800
preview-wall scene=multi-gate-1024 workers=16 round=0 whole_p50_ns=1000400 whole_p95_ns=1667900
preview-wall scene=multi-gate-1024 workers=16 round=1 whole_p50_ns=1108400 whole_p95_ns=1633400
preview-wall scene=multi-gate-1024 workers=16 round=2 whole_p50_ns=1024900 whole_p95_ns=1552900
```

| workers | p50 中位（ns） | p50 区间        | p95 中位（ns） |
| ------- | -------------- | --------------- | -------------- |
| 1       | 760600         | 751000–767800   | 1282800        |
| 2       | 825800         | 812200–844500   | 1305000        |
| 4       | 747100         | 737200–747100   | 1233600        |
| 8       | 811100         | 799600–864700   | 1277100        |
| 16      | 1024900        | 1000400–1108400 | 1633400        |

## b99a282e 基线（仅 P2 分发）原始输出

```text
preview-wall scene=multi-gate-1024 workers=1  round=0 whole_p50_ns=751400 whole_p95_ns=1692700
preview-wall scene=multi-gate-1024 workers=1  round=1 whole_p50_ns=755100 whole_p95_ns=1262500
preview-wall scene=multi-gate-1024 workers=1  round=2 whole_p50_ns=786700 whole_p95_ns=1306900
preview-wall scene=multi-gate-1024 workers=2  round=0 whole_p50_ns=706300 whole_p95_ns=1290700
preview-wall scene=multi-gate-1024 workers=2  round=1 whole_p50_ns=751900 whole_p95_ns=1322700
preview-wall scene=multi-gate-1024 workers=2  round=2 whole_p50_ns=740500 whole_p95_ns=1201100
preview-wall scene=multi-gate-1024 workers=4  round=0 whole_p50_ns=657300 whole_p95_ns=1069300
preview-wall scene=multi-gate-1024 workers=4  round=1 whole_p50_ns=633500 whole_p95_ns=1021700
preview-wall scene=multi-gate-1024 workers=4  round=2 whole_p50_ns=665400 whole_p95_ns=1807700
preview-wall scene=multi-gate-1024 workers=8  round=0 whole_p50_ns=675400 whole_p95_ns=1108500
preview-wall scene=multi-gate-1024 workers=8  round=1 whole_p50_ns=652400 whole_p95_ns=1148600
preview-wall scene=multi-gate-1024 workers=8  round=2 whole_p50_ns=648400 whole_p95_ns=1108100
preview-wall scene=multi-gate-1024 workers=16 round=0 whole_p50_ns=775400 whole_p95_ns=1361200
preview-wall scene=multi-gate-1024 workers=16 round=1 whole_p50_ns=743300 whole_p95_ns=1305000
preview-wall scene=multi-gate-1024 workers=16 round=2 whole_p50_ns=720700 whole_p95_ns=1274700
```

| workers | p50 中位（ns） | p50 区间      | p95 中位（ns） |
| ------- | -------------- | ------------- | -------------- |
| 1       | 755100         | 751400–786700 | 1306900        |
| 2       | 740500         | 706300–751900 | 1290700        |
| 4       | 657300         | 633500–665400 | 1069300        |
| 8       | 652400         | 648400–675400 | 1108500        |
| 16      | 743300         | 720700–775400 | 1305000        |

## 有界对照（R6）

| 对照组                    | 基线 p50 | 新实现 p50 | Δ      |
| ------------------------- | -------- | ---------- | ------ |
| b99a282e w1 → 新实现 w1   | 755100   | 760600     | +0.7%  |
| b99a282e w2 → 新实现 w2   | 740500   | 825800     | +11.5% |
| b99a282e w4 → 新实现 w4   | 657300   | 747100     | +13.7% |
| b99a282e w8 → 新实现 w8   | 652400   | 811100     | +24.3% |
| b99a282e w16 → 新实现 w16 | 743300   | 1024900    | +37.8% |

读法（如实，R6）：

- **组织成本提取**（b99a w1 → 新 w1）：+0.7%，在 round 间方差内——
  P3/P5 原语提取与分发装配的串行组织成本在该场景不可测出。
- **相对仅 P2 并行的整步改善**（b99a 多 worker → 新多 worker）：观察到
  +11.5%～+37.8% 的整步回退——小工作量场景（空区间 Waiting 候选 +
  直线运动）下 P3/P5 两个新分发阶段的固定成本（输入发现、槽位、
  完整 join）大于其并行收益。**尚无净收益证据**；显著性与成因未分离
  （round 间方差同量级、未做配对检验），归因撤回：三阶段顺序执行、
  各自完整 join，并非 48 worker 同时运行；w16 的 +37.8% 机理未定位。
- 本数据不构成生产性能通过；城市级规模认证归 #707。

## 阈值登记（三个保守初值，R6 口径修正）

- `WAITING_PREVIEW_DISPATCH_MIN_ACTIVE`（P2）、
  `CONFLICT_DISPATCH_MIN_ACTIVE`（P3）、`MOTION_DISPATCH_MIN_ACTIVE`
  （P5）三个独立常量保持保守初值 1_024。
- 「Active ≥ 1_024」不等于「P3 本拍跨阈值」：P3 的分发工作集跳过持有
  旧 reservation 的车辆（发现阶段排除）；生产路径证据按各相位自己的
  条件说明（P2 = Active 投影，P3 = Active 且非旧 reservation，P5 =
  Active 投影）。阈边界对照（等于 1_024 每拍三相全分发、逐拍与融合
  一致）由 `preview_at_threshold_equivalence` 1_280 车集成证据与 lib
  强制分发对拍覆盖。
- R1/R2/R4 修复后已重测（本表）。R3 收敛（共用领域段 + 段暂存复用）已
  合并且 71 项冲突测试全绿，但墙钟未再重测——收敛的组织收益/成本在
  本小工作量场景预计不可测出（参照 w1 对照 +0.7% 的方差内结论），
  重测随 #707 城市级认证一并进行。

## 未测项与原因

| 项                                                                    | 状态   | 原因                                                                                                                 |
| --------------------------------------------------------------------- | ------ | -------------------------------------------------------------------------------------------------------------------- |
| P3-only / P5-only 消融                                                | 未测   | E1 开关为 cfg(test) 私有旋钮，release example 不可见；需把开关提升为执行配置或构建 test-profile 墙钟设施，独立工作项 |
| 各阶段实际工作/最长块/参与线程/回退/暂存字节/串行部分（release 形态） | 未测   | performance_profile 阶段计时为 cfg(test) 插桩；release 非插桩阶段计时出口随上一条立项                                |
| p99/max                                                               | 未登记 | 现有 example 只输出 p50/p95；扩展输出为独立工作项                                                                    |
