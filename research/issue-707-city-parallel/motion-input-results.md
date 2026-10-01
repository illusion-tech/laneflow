# #707 M3：停车 binding 稠密下标

> **证据归档**：本目录的历史诊断 JSON 已迁入[冻结归档](../archives/2026-10-01-json-migration.md)。
> 本文的 JSON 链接指向原提交；文中相对 JSON 路径及依赖它们的历史命令按归档内 `source/` 目录解释。

2026-09-22。状态：**保留。** 同一二进制上 dense 的 Core 均值比 hash 少 0.629 ms，
observation 少约 2.8 ms。短窗交通输出与 H1 / 屏障跳过逐字节一致。
p95 仍约 32.5–33.0 ms，16 ms 目标未达到。

合同见 [M3 计划](motion-input-plan.md)。上一轮保留组合见
[屏障跳过](motion-barrier-results.md)。

## 1. 候选

每个 Active 车辆每拍都查询停车 binding，绝大多数没有 binding。
`hash` 继续用 `HashMap<VehicleHandle, ParkingBinding>`。
`dense` 在安装时按 `vehicle_capacity` 分配槽位，用车辆下标和代际读取。
停车规则没有改。两组都固定 `LF707_BARRIER=skip`。

## 2. 性能

100k 长尾、4 workers、33 ms、256 拍，统计 tick ≥ 65 的 192 拍。
Active 均为 70,761–74,392。单位是毫秒。顺序是 hash、dense、dense、hash。

| 臂      | Core 均值 | 标准差 |    p95 | observation 均值 | iteration 均值 |
| ------- | --------: | -----: | -----: | ---------------: | -------------: |
| hash 1  |    29.946 |  2.547 | 34.650 |           65.308 |         98.169 |
| dense 2 |    29.067 |  1.814 | 32.525 |           62.799 |         94.641 |
| dense 3 |    29.601 |  2.028 | 33.044 |           62.299 |         94.707 |
| hash 4  |    29.980 |  2.160 | 34.263 |           65.374 |         98.202 |

两次 hash 的 Core 均值是 29.963 ms，极差 0.034 ms。两次 dense 的均值是
29.334 ms，少 0.629 ms（2.10%）。较慢的 dense（29.601）仍快于较快的 hash
（29.946）。hash 的 observation 均值 65.341 ms，dense 62.549 ms，少 2.792 ms。
iteration 少 3.511 ms。observation 下降是因为质量观测也读同一张 binding 表，
不是 Core 算法变快。

dense 的 p95 是 32.525 和 33.044 ms。其中一臂低于 33 ms，另一臂不是。
不能写成已经达到 33 ms，更没有达到 16 ms。

工作集峰值观测约 448–452 MiB，四臂没有拉开。这是 250 ms 轮询，不是精确峰值。

## 3. 交通结果

四臂的 `quality.csv`、`ticks.jsonl`、`events.jsonl` SHA-256 与 M2 的 100k
短窗相同。末拍 completed 4,039、parked 25,200。这是表示替换后的同一短窗，
不是长等待或红灯 ETA 的验收。

## 4. 保留与边界

后续研究组合是 B1 + H1 + 屏障跳过 + 稠密 binding。源码在
`target/issue707-m3-source`；仓库另提交从基准到 M3 的累计二进制 Git 补丁和
M2→M3 三文件增量。对照用 `LF707_BINDING=dense` 和 `LF707_BARRIER=skip`。
尚未进入正式实现 PR，也没有改权威设计。

这次只换了停车 binding 的读取组织。M1 的输入读取还包含路线、长度、限速和
profile。0.629 ms 的 Core 节省说明哈希查询只是那一块里的一部分。

## 5. 证据

- 机器可读封套见
  [`motion-input-evidence-manifest.json`](https://github.com/illusion-tech/laneflow/blob/bc1bf666a54aebc50a2b7efa50fb1bc3b05ba567/research/issue-707-city-parallel/evidence/motion-input-evidence-manifest.json)：
  它绑定四次运行各 17 个文件的完整树身份、同一精确二进制与计划身份、每臂
  `LF707_BINDING` 处理模式、完整 `prefix-result`、进程资源包络、交通文件哈希、
  未舍入统计和最终计数。逐拍紧凑报告见
  [`motion-input-public-step.csv`](evidence/motion-input-public-step.csv)，包含 4 × 192 个
  `public_step_ns`、`observation_ns` 与 `iteration_ns` 样本。
- [`motion-input-source-patch.tar.zst`](evidence/motion-input-source-patch.tar.zst)
  包含两个只涉及 Rust/Cargo 构建源码的二进制 Git 补丁：从基准提交到 M3 的
  49 文件累计补丁，以及从已提交精确 M2 源码到 M3 的 3 文件增量。两条路径均已在
  新目录回放，并与 451 个原 M3 构建源文件逐字节一致。
- 基准提交是 `4de40e045398e4b010b2aa36522afc02a4094c4d`；M2 父源码可由已提交的
  [`motion-barrier-source-patch.tar.zst`](evidence/motion-barrier-source-patch.tar.zst)
  重建。
- Rust 1.98.0，release，locked，offline，`CARGO_INCREMENTAL=0`，
  feature `entry-frontier,barrier-reach`。
- 二进制 SHA-256：
  `90f6e67cbd8062b1baa01a47d0e7c9aaeb81002b0de1306270a0898e2fe9eb5d`。
- 单测 `dense_binding_reads_by_index_and_generation` 通过。
  hash 路径的 `binding_reuse_preserves_parking_validation_and_atomic_failure`、
  `binding_reuse_observes_reserve_and_cancel_between_ticks`、
  `declared_virtual_capacity_is_not_a_runtime_storage_axis` 通过。
  未跑完整 runtime 测试套件。
- 原四臂结果目录、精确 EXE 和历史 Windows 驱动仍只在作者机器，不承诺项目长期
  留存；仓库内封套、逐拍样本与源码补丁是本页结论的可移植证据。运行使用平衡电源
  方案，WPR 未在录制。
