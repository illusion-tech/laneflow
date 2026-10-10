# #707 M2：跳过够不着的屏障查询

> **证据归档**：本目录的历史诊断 JSON 已迁入[冻结归档](../archives/2026-10-01-json-migration.md)。
> 本文的 JSON 链接指向原提交；文中相对 JSON 路径及依赖它们的历史命令按归档内 `source/` 目录解释。

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

| 臂          | Core 均值 | 标准差 |    p95 | observation 均值 |
| ----------- | --------: | -----: | -----: | ---------------: |
| H1 parent 1 |    31.568 |  2.130 | 35.396 |           67.550 |
| skip 2      |    30.146 |  1.903 | 33.487 |           65.618 |
| skip 3      |    30.409 |  1.967 | 33.832 |           65.189 |
| H1 parent 4 |    31.202 |  2.137 | 35.443 |           67.543 |

两次 H1 的 Core 均值极差 0.366 ms。
按 192 拍原始整数纳秒重新聚合，两次 skip 均值 30.277 ms，比两次 H1 均值
31.385 ms 少 1.108 ms（3.53%）。旧值 30.278 / 1.107 来自先将各臂显示到
三位小数后再求均值；本页改用未舍入样本作为权威口径。
较慢的 skip（30.409）仍快于较快的 H1（31.202）。

同一新二进制再跑一组 direct/skip/skip/direct：

| 臂       | Core 均值 | 标准差 |    p95 | observation 均值 |
| -------- | --------: | -----: | -----: | ---------------: |
| direct 1 |    31.279 |  2.224 | 35.794 |           66.776 |
| skip 2   |    30.094 |  1.933 | 33.574 |           65.189 |
| skip 3   |    29.689 |  1.813 | 33.290 |           64.457 |
| direct 4 |    31.202 |  2.479 | 35.583 |           65.023 |

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

后续研究组合是 B1 + H1 + 本轮屏障跳过。原构建源码仍在作者机器的
`target/issue707-m2-source`；仓库另提交从基准到 M2 的累计二进制 Git 补丁和
从基准到 H1 父源码的补丁，以及 H1→M2 七文件增量，默认对照用
`LF707_BARRIER=skip`。
尚未整理成正式实现 PR，也没有改权威设计。

输入读取仍是上一轮里的第二大段，约 11.6–11.9 ms CPU。它不在本轮候选里。

## 5. 证据

- 机器可读封套见
  [`motion-barrier-evidence-manifest.json`](https://github.com/illusion-tech/laneflow/blob/bc1bf666a54aebc50a2b7efa50fb1bc3b05ba567/research/issue-707-city-parallel/evidence/motion-barrier-evidence-manifest.json)：
  它绑定十次运行各 17 个文件的完整树身份、二进制与计划身份、每臂
  `LF707_BARRIER` 处理模式、完整 `prefix-result`、进程资源包络、交通文件哈希、
  精确统计和最终计数。逐拍紧凑报告见
  [`motion-barrier-public-step.csv`](evidence/motion-barrier-public-step.csv)，包含
  10 × 192 个 `public_step_ns` 与 `observation_ns` 样本。
- [`motion-barrier-source-patch.tar.zst`](evidence/motion-barrier-source-patch.tar.zst)
  包含三个只涉及 Rust/Cargo 构建源码的二进制 Git 补丁：从基准提交到 H1 的
  47 文件父源码补丁、从基准到 M2 的 48 文件累计补丁，以及从 H1 到 M2 的 7 文件
  增量。M2 累计补丁逐字节回放为 451 文件的 M2 构建源；父补丁和增量使用为 M2
  谱系冻结的 H1 文本表示。它与历史 H1 在源码文本上相同，但
  `tools/laneflow-urban-harness/Cargo.toml` 使用 LF，而历史 provenance 记录为 CRLF，
  因此不再把该父补丁称为历史 H1 的原始字节副本。历史 H1 的精确 Rust/Cargo 字节
  与 B1→H1 增量改由
  [`retained-foundation-source-patch.tar.zst`](evidence/retained-foundation-source-patch.tar.zst)
  保存；两套历史非 Rust 运行脚本均未提交。
- 基准提交是 `4de40e045398e4b010b2aa36522afc02a4094c4d`；M2 增量谱系使用的 H1
  文本表示可由该基准加 47 文件父源码补丁重建。父处理臂对应的历史 H1 原始字节
  由前述 foundation archive 重建。
- Rust 1.98.0，release，locked，offline，`CARGO_INCREMENTAL=0`，
  feature `entry-frontier,barrier-reach`。
- skip/direct 二进制 SHA-256：
  `997c377104f0ae302dfa1bd0c8d9e700f59ab28d58748fa506a6937d55492a59`
- H1 parent SHA-256：
  `5525f9aea127793c69ab08d4891fd05e3049dd79585199e00272b5d050391080`
- 距离表单测 `nearest_barrier_distance_is_measured_from_each_hop_start` 通过。
  带该 feature 的 runtime lib 测试能够编译。未跑完整 runtime 测试套件。
- 原结果目录 `target/issue707-m2-results/` 与两个 EXE 当前只存在于作者机器，
  不承诺由项目长期留存；仓库内封套、逐拍紧凑报告和源码补丁是本页结论的可移植
  长期证据。运行使用平衡电源方案，WPR 未在录制。
