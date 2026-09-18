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
| 4       | 738900         | 737200–747100   | 1233600        |
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
| b99a282e w4 → 新实现 w4   | 657300   | 738900     | +12.4% |
| b99a282e w8 → 新实现 w8   | 652400   | 811100     | +24.3% |
| b99a282e w16 → 新实现 w16 | 743300   | 1024900    | +37.8% |

读法（如实，R6）：

- **组织成本提取**（b99a w1 → 新 w1）：+0.7%——收窄表述：当前三轮
  对照中单 worker 差异较小，**尚不能区分**本次组织变化与测量波动。
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
  合并；墙钟未再重测——收敛的组织收益/成本在本小工作量场景预计不可测出
  （参照 w1 对照 +0.7% 的方差内结论），重测随 #707 城市级认证一并进行。

## R3-3c 真 Conflict 夹具分配证据（2026-09-19，lib 子进程测量）

夹具：conflict_scale 1_200 车同路线（非空 cells/downstream 候选，
`visited_passages` = 48,000/8 拍窗），worker=4，三相位每拍自然跨阈值分发；
测量在 lib 测试的独占子进程内进行（`LFRT_CONFLICT_ALLOC_EVIDENCE` 门控，
复用既有 INSTRUMENTED_SYSTEM 全局计数分配器）。

| 窗                                     | 拍数 | 分配实测 | 节点预算     | reallocations | 断言                    |
| -------------------------------------- | ---- | -------- | ------------ | ------------- | ----------------------- |
| A（含首触）                            | 8    | 97       | 96 + 8 松弛  | 0             | ≤ 预算 + 松弛           |
| B（紧接 A）                            | 8    | 97       | —            | 0             | ≤ A（窗口间不增长）     |
| C（长窗）                              | 24   | 292      | 288 + 8 松弛 | 0             | 亚线性（非逐拍/逐候选） |
| 失败清理（注入首错 → 重试 → 4 稳态拍） | 6    | 68       | 72           | 0             | ≤ 预算，清理无泄漏      |

实测形态：每拍 12 次分配 = 3 相位 × 4 worker 的 Rayon scope 任务节点
（执行配置 §3 豁免），稳态 LaneFlow 自有分配为零（段暂存跨拍复用生效）；
注（W1-A 后更新）：报告全部变体（含 None/Staged/Failed）现均随行
scratch，发现位 0 在 None ↔ Computed 间交替时上拍容量由 into_scratch
统一回收——旧表述「上拍 None 报告无容量可回收」作废。每 ~8 拍的 +1
已定位为非暂存链来源（F3b 冷池首触与工作集形状变化，W1 前后同值），
如实登记为有界首触，归因留待 #707（dhat 剖析），非每候选每拍分配。

## W1/W2 后短对照（2026-09-19，当前 head 重跑）

测量条件：本轮机器无并发负载（此前各轮测量期间有验证链并发，绝对值
不可跨轮直比）；同一场景同一 example，正常 release 非插桩。

| workers | p50 三轮（ns）           | 中位   |
| ------- | ------------------------ | ------ |
| 1       | 330500 / 330400 / 329700 | 330400 |
| 4       | 312300 / 283600 / 308900 | 308900 |

w4 相对 w1 = −6.5%，在 round 间方差内（与上轮「多 worker 回退」的
观察相反——不同修订、运行条件和批次之间的绝对值差异较大，目前尚未
分离代码变化与环境影响；净收益仍未证明）。
b99a282e 基线对照为 R3 前中间修订测量，未在当前 head 重测（需第二
检出全量 release 构建，预算外）。

## 未测项与原因

| 项                                                                    | 状态   | 原因                                                                                                                 |
| --------------------------------------------------------------------- | ------ | -------------------------------------------------------------------------------------------------------------------- |
| P3-only / P5-only 消融                                                | 未测   | E1 开关为 cfg(test) 私有旋钮，release example 不可见；需把开关提升为执行配置或构建 test-profile 墙钟设施，独立工作项 |
| 各阶段实际工作/最长块/参与线程/回退/暂存字节/串行部分（release 形态） | 未测   | performance_profile 阶段计时为 cfg(test) 插桩；release 非插桩阶段计时出口随上一条立项                                |
| p99/max                                                               | 未登记 | 现有 example 只输出 p50/p95；扩展输出为独立工作项                                                                    |
