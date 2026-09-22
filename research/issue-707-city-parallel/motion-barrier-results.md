# #707 M2：跳过够不着的屏障查询

2026-09-22。状态：**保留。** 100k 上 Core 均值下降约 1.1–1.3 ms，短窗交通结果与
H1 逐字节一致。p95 仍约 33–36 ms，16 ms 目标未达到。

合同见 [M2 计划](motion-barrier-plan.md)。测量背景见
[Motion 四段成本](motion-phase-results.md)。

## 1. 候选

本拍位移上界是 `速度 × 步长 + 0.5 × 最大加速度 × 步长² + 50 mm`。
最近的冲突准入、等待入口或信号门远于这个上界时，不再做对应的停止查询。
当前边剩余长度也远于上界时，`hard_room` 不再调用 `hop_permitted`。
进度和亚毫米余量都为 0 的车辆仍走原来的冲突查询。

`LF707_BARRIER=skip` 启用该路径，`direct` 保持原查询。性能 A 是封存 H1
二进制，B 是 skip。同一新二进制上的 direct/skip 用来区分算法和二进制布局。

## 2. 性能

100k 长尾、4 workers、33 ms、256 拍，统计 tick ≥ 65 的 192 拍。
Active 均为 70,761–74,392。单位是毫秒。

| 臂 | Core 均值 | 标准差 | p95 | observation 均值 |
| --- | ---: | ---: | ---: | ---: |
| H1 parent 1 | 31.568 | 2.130 | 35.396 | 67.550 |
| skip 2 | 30.146 | 1.903 | 33.487 | 65.618 |
| skip 3 | 30.409 | 1.967 | 33.832 | 65.189 |
| H1 parent 4 | 31.202 | 2.137 | 35.443 | 67.543 |

两次 H1 的 Core 均值极差 0.366 ms。
两次 skip 均值 30.278 ms，比两次 H1 均值 31.385 ms 少 1.107 ms（3.53%）。
较慢的 skip（30.409）仍快于较快的 H1（31.202）。

同一新二进制再跑一组 direct/skip/skip/direct：

| 臂 | Core 均值 | 标准差 | p95 | observation 均值 |
| --- | ---: | ---: | ---: | ---: |
| direct 1 | 31.279 | 2.224 | 35.794 | 66.776 |
| skip 2 | 30.094 | 1.933 | 33.574 | 65.189 |
| skip 3 | 29.689 | 1.813 | 33.290 | 64.457 |
| direct 4 | 31.202 | 2.479 | 35.583 | 65.023 |

两次 direct 均值 31.241 ms，极差 0.077 ms。两次 skip 均值 29.892 ms，
少 1.349 ms（4.32%）。最后一臂 direct 的 Core 回到 31.202 ms，而
observation 没有回到第一臂，说明 Core 差异跟着开关走，不完全是整机变快。
四次 skip 都快于全部四次 H1 或 direct 基线；最小差距是 31.202 − 30.409 = 0.793 ms。

10k 长尾同一二进制：direct Core 均值 2.647 ms，skip 2.474 ms。

这些 p95 仍高于 33 ms。按约 1.3 ms 的 Core 节省，离 16 ms 还远。

## 3. 交通结果

100k 的八臂（四次基线、四次 skip）里，`quality.csv`、`ticks.jsonl`、
`events.jsonl` 的 SHA-256 分别相同。10k 的 direct 与 skip 也相同。
完成数、停车数、停车比例和重叠在这 256 拍里没有变化；100k 末拍 completed
4,039、parked 25,200，重叠对数为 0。这是短窗上的一致，不是长等待或红灯
ETA 的验收。那两个问题没有在本轮修改。

## 4. 保留与边界

后续研究组合是 B1 + H1 + 本轮屏障跳过。源码在
`target/issue707-m2-source`，默认对照用 `LF707_BARRIER=skip`。
尚未整理成正式实现 PR，也没有改权威设计。

输入读取仍是上一轮里的第二大段，约 11.6–11.9 ms CPU。它不在本轮候选里。

## 5. 证据

- 父源码是封存 H1，提交 `4de40e045398e4b010b2aa36522afc02a4094c4d`。
- Rust 1.98.0，release，locked，offline，`CARGO_INCREMENTAL=0`，
  feature `entry-frontier,barrier-reach`。
- skip/direct 二进制 SHA-256：
  `997c377104f0ae302dfa1bd0c8d9e700f59ab28d58748fa506a6937d55492a59`
- H1 parent SHA-256：
  `5525f9aea127793c69ab08d4891fd05e3049dd79585199e00272b5d050391080`
- 距离表单测 `nearest_barrier_distance_is_measured_from_each_hop_start` 通过。
  带该 feature 的 runtime lib 测试能够编译。未跑完整 runtime 测试套件。
- 结果目录：`target/issue707-m2-results/`。平衡电源方案，WPR 未在录制。
