# #707 城市并行性能对照（第二切片：harness 入口验证与带暖机 pilot）

后续诊断设计见[源码成本分析与短窗口诊断草案](short-profile-plan.md)。
正式 r1 后续状态见 [D1/D2 与诊断证据索引](formal-diagnostics-index.md)；
下文“未执行 D/E”专指原 pilot 切片，不表示截至今日未进行任何正式运行。
2026-09-21 双窗口采集完成后的分析见 [L3 离线诊断](l3-findings.md)，包含采集
边界、线程活动、函数热点及替换命令的新候选。
第一轮实际插桩、前缀与生命周期结果见[分层短测结果](short-profile-results.md)。
当前方向见[激进优化与结果验收计划](performance-first-next-plan.md)：直接研究
入口队列、局部许可和多频更新，以实测性能与交通结果验收；不继承旧版严格对拍
或跨 worker 逐拍一致要求。下文 pilot/正式窗口流程是既有实验记录，不构成新模型门槛。
多 profile 的新增输入、覆盖检查及局部时窗实验边界见[多 gap profile 夹具](multi-gap-fixtures.md)。
下一阶段的代码原型、速度与交通结果见[入口候选与局部时窗结果](entry-frontier-results.md)。
后续 [P3 / Waiting 工作范围实验](scope-results.md)已完成：100k 平衡复测均值
52.750 → 44.038ms；新增共享目标多读者与混合转向夹具，并记录城市长等待问题。
预检、收尾实验见[上一轮结果](finalize-results.md)。最新的
[边界变化与四拍控制复用结果](boundary-results.md)确认 B1 的约 5.44% 增量收益，
B2 因负收益淘汰，并定位了红灯停止线 ETA=0 阻挡绿灯的样本。
设计背景见[Motion 边界变化计划](motion-boundary-next-plan.md)。
最新 [B1 暖机后 WPR 诊断](b1-wpr-findings.md)确认主线程持续满载，五个独立观测
入口占目标进程 CPU 样本 46.26%；Core 重点仍是运动约束查询与车辆状态迁移。
后续 [H1 完整观测去重](harness-dedup-results.md)已完成：100k ABBA 的 observation
均值下降 13.61%、iteration 下降 9.29%，观测内容与频率保持，Core 没有明确改善。
后续 [C1 门控规则复用](constraint-reuse-results.md)完成诊断、审计与 ABBA/BAAB：
门控重复查询约 88.44%，但候选没有稳定净收益，已淘汰；继续保留 B1 + H1。
后续 [Motion 四段成本](motion-phase-results.md)已量完：约束查询是最大段，
非前车约束占其中多数；跟车求解和下一状态都更小。
后续 [屏障跳过](motion-barrier-results.md)已保留：够不着的冲突、等待和信号
停止查询不再每拍重做，100k Core 均值少约 1.1–1.3 ms，短窗输出与 H1 一致。
后续 [停车 binding 稠密下标](motion-input-results.md)已保留：Core 均值再少
0.629 ms，observation 少约 2.8 ms。后续
[路线本地长度、限速和 profile](motion-input-local-results.md)已淘汰：
同一二进制上的 Core 差值落在运行间极差里。后续
[非前车约束拆分](motion-constraint-split-results.md)已量完：冲突停止是最大块，
没有桶通过计时开销的两倍线。后续
[收窄冲突停止](motion-conflict-narrow-results.md)已淘汰：同一二进制上的
Core 差值小于 direct 的运行间极差。后续
[运动以外的 Core](core-outside-motion-results.md)已量完：冲突准备约
13.3–13.8 ms，大于运动循环。后续
[frontier 按 hop 记住](frontier-replay-results.md)已保留：frontier 少
1.165 ms，Core 少 1.445 ms。后续
[远车滑行](motion-glide-results.md)已淘汰：运动循环少 1.831 ms，但出现
12 对重叠。后续
[运动按路线边成批](motion-batch-results.md)已淘汰：排序把准备阶段增加约
12 ms，分发没有变快。后续
[按车辆下标放运动热行](motion-hot-results.md)已淘汰：分发少 0.373 ms，
消费把这段时间拿了回去。后续
[按车道边成批](motion-lane-batch-results.md)已淘汰：分发慢 2.159 ms。
P5 的布局尝试结束。16 ms 目标仍未达到。

> 测量基线：`4de40e045398e4b010b2aa36522afc02a4094c4d`（main，#731 合并后）。
> 证据根：`E:/projects/laneflow-evidence/issue-707/4de40e04/`（checkout 外、只新增不覆盖）。
> 测量 checkout：`E:/projects/worktrees/707-measure/laneflow`（detached @ 4de40e04，全程干净）。
> 原 pilot 切片范围：WP A/B/C；后续 D1/D2 已完成 r1，完整 D/E 尚未完成。
> 本研究文档分支未改 Runtime 算法/调度/阈值/P4/SIMD/数据布局或 Harness 代码；
> 诊断插桩在独立 checkout，身份与边界见分层短测结果。

## 可复现命令

环境（所有运行统一设置）：

```text
set LANEFLOW_HARDWARE_ROLE=amd-ryzen9-9955hx-16c-32t-64gb-hyperv
set LANEFLOW_POWER_ROLE=win11-balanced-381b4222
```

构建（仅测量 worktree，benchmark 前完成，之后零编译）：

```text
cargo +1.98.0 build -p laneflow-urban-harness --release --locked
cargo +1.98.0 build -p laneflow-urban-generator --release --locked
```

制品（现有目录无 10k/100k 城市制品通过摘要校验，用冻结入口重建）：

```text
laneflow-urban-generator.exe --config examples/config/cn-urban.toml --scale 10k  --output <evidence>/inputs/urban-10k
laneflow-urban-generator.exe --config examples/config/cn-urban.toml --scale 100k --output <evidence>/inputs/urban-100k
```

smoke（WP B，每档）：

```text
laneflow-urban-harness.exe plan  <artifacts> <plans>/<scale>-smoke.toml  --case MIXED-PEAK --probe-ticks 256
laneflow-urban-harness.exe run   <artifacts> <plans>/<scale>-smoke.toml  <smoke>/<scale>-w1-r1 --workers 1
laneflow-urban-harness.exe run   <artifacts> <plans>/<scale>-smoke.toml  <smoke>/<scale>-w4-r1 --workers 4
laneflow-urban-harness.exe compare <smoke>/<scale>-w1-r1 <smoke>/<scale>-w4-r1 <comparisons>/<scale>-smoke-w1-w4.json
```

pilot（WP C，每档）：

```text
laneflow-urban-harness.exe plan  <artifacts> <plans>/<scale>-pilot.toml --case MIXED-PEAK --probe-warm-up 1024 --probe-ticks 4096
laneflow-urban-harness.exe run   <artifacts> <plans>/<scale>-pilot.toml <pilot>/<scale>-w1-r1 --workers 1
laneflow-urban-harness.exe run   <artifacts> <plans>/<scale>-pilot.toml <pilot>/<scale>-w4-r1 --workers 4
laneflow-urban-harness.exe compare <pilot>/<scale>-w1-r1 <pilot>/<scale>-w4-r1 <comparisons>/<scale>-pilot-w1-w4.json
```

## 已完成 / 待完成

- [x] WP A 目录与身份冻结（worktree、证据根、构建、manifest 素材、环境变量、制品）
- [x] WP B 入口验证（10k/100k smoke 双工 + compare，逐臂核查）
- [x] WP C 带暖机 pilot（10k/100k × w1/w4 + compare）
- [x] 基线清单核查（P1/Pp 实测；B-P2/R0/R1 登记不可比原因）
- [ ] WP D 正式观察窗（r1/r2/r3 交错，每臂同 worker 三轮）——D1/D2 r1 已完成；r2/r3 与三轮聚合待完成
- [ ] WP E 城市级结论与预算判定——r1 两档均未达原预算；完整四基线、稳定 Active、资源成本及认证仍待完成

## 进入正式窗口（WP D）的判断

1. smoke/pilot 两组 `probe-match` 且无来源/配置不一致（本切片已满足，见 pilot-report.md）。
2. 计划用正式 performance 窗口生成 `--performance` 计划并跑同 worker 三轮聚合
   （`compare <a> <b> <c>`）；跨臂 1w/4w 只做语义对拍，不做计时结论。
3. 预算判定只用正式 `measurements.toml` 的观察窗样本；诊断分位数
   （diagnostics.json，含暖机）不得表述为正式 Core p95。

## 已知偏差（WP A 核对记录）

- **制品为本次重建**：仓库现有目录（examples/data、research/issue-543-*、
  target/tmp）无可通过摘要校验的 10k/100k 城市制品，按冻结入口重建。
- **plans.json 冻结摘要不匹配（已核查，结论见 input-diff.md；未改
  golden）**：对重建制品生成的 MIXED-PEAK correctness 计划与
  `fixtures/v3/plans.json` 的 `10k-mixed-peak`/`100k-mixed-peak`
  **字节数完全一致**（4057834 / 40845093），SHA-256 不一致。WP C.1
  逐字段核查定位：两档制品与冻结制品**仅两个构建内存统计字段不同**
  （`shared_*_retained_bytes`），network_revision 与全部内容文件摘要
  一致；唯一计划差异是内嵌 `manifest_digest`（其输入为整张 manifest
  的 SHA-256），差异数字同为 7 位十进制故计划字节数不变。属非交通
  元数据漂移，Runtime 消费字段等价——**接受重建制品**，golden 原样。
