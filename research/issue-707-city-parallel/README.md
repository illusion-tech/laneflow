# #707 城市并行性能对照（第二切片：harness 入口验证与带暖机 pilot）

> 测量基线：`4de40e045398e4b010b2aa36522afc02a4094c4d`（main，#731 合并后）。
> 证据根：`E:/projects/laneflow-evidence/issue-707/4de40e04/`（checkout 外、只新增不覆盖）。
> 测量 checkout：`E:/projects/worktrees/707-measure/laneflow`（detached @ 4de40e04，全程干净）。
> 本切片范围：WP A/B/C。**未执行正式长窗口（D/E）**；未改 Runtime 算法/调度/阈值/P4/SIMD/数据布局，未改 harness 代码。

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
- [ ] WP D 正式观察窗（r1/r2/r3 交错，每臂同 worker 三轮）——**未开始，见 experiment-plan.md**
- [ ] WP E 城市级结论与预算判定——未开始

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
