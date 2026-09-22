# #707 M4：路线本地的长度、限速和 profile

2026-09-22。状态：**淘汰。** 同一二进制上 local 的 Core 均值比 direct 少 0.065 ms，
小于 direct 两次运行的极差 0.142 ms。短窗交通输出与 M3 逐字节一致。
16 ms 目标未达到。

合同见 [M4 计划](motion-input-local-plan.md)。保留组合仍见
[停车 binding 稠密下标](motion-input-results.md)。

## 1. 候选

`direct` 继续用全局长度表、限速表，并按八个关系数组拼车辆 profile。
`local` 用路线编译时按 `edges` 记下的长度和限速，以及安装时按 profile
序号抄下的 `VehicleProfileView`。前车查询和跟车求解仍读全局表。
两份路线数组和 profile 表在两种模式下都分配。每一臂都固定
`LF707_BARRIER=skip` 和 `LF707_BINDING=dense`。

## 2. 性能

100k 长尾、4 workers、33 ms、256 拍，统计 tick ≥ 65 的 192 拍。
Active 均为 70,761–74,392。单位是毫秒。顺序是 direct、local、local、direct。

| 臂 | Core 均值 | 标准差 | p95 | observation 均值 | iteration 均值 |
| --- | ---: | ---: | ---: | ---: | ---: |
| direct 1 | 37.604 | 2.660 | 41.306 | 72.415 | 113.500 |
| local 2 | 37.456 | 2.924 | 41.913 | 71.906 | 112.895 |
| local 3 | 37.763 | 2.505 | 41.520 | 73.280 | 114.528 |
| direct 4 | 37.746 | 2.529 | 41.414 | 73.026 | 114.350 |

两次 direct 的 Core 均值是 37.675 ms，极差 0.142 ms。两次 local 的均值是
37.609 ms。未四舍五入的均值相差 0.065 ms（0.17%）。较慢的 local（37.763）
高于较快的 direct（37.604）。direct 极差大于平均节省，合同里的保留条件没有成立。

observation 均值 direct 72.720 ms、local 72.593 ms，相差 0.127 ms。
iteration 相差 0.213 ms。都落在运行间波动里。

四臂 p95 为 41.306–41.913 ms。工作集峰值观测约 448–451 MiB，和 M3 的
448–452 MiB 在同一档。这是 250 ms 轮询，不是精确峰值。

这次二进制的 Core 绝对水平约 37.6 ms，高于 M3 dense 当天的 29.334 ms。
direct 和 local 一起偏高，observation 也一起偏高，交通输出和工作集没有拉开。
同一二进制里的 0.065 ms 才是这次采用的对照。跨日的绝对水平不记成路线本地表的成本，
也不替换 M3 已保留的 Core 数字。

## 3. 交通结果

四臂的 `quality.csv`、`ticks.jsonl`、`events.jsonl` SHA-256 与 M3 的 100k
短窗相同。末拍 completed 4,039、parked 25,200。长度、限速和 profile 的这组
读取替换在这个短窗里没有改变交通结果。

## 4. 淘汰与保留

研究组合仍是 B1 + H1 + 屏障跳过 + 稠密 binding。源码仍以
`target/issue707-m3-source` 为准，测量用 `LF707_BINDING=dense` 和
`LF707_BARRIER=skip`。M4 原型留在 `target/issue707-m4-source`，不进入正式 PR，
也没有改权威设计。

把当前路段的长度、限速和车辆 profile 收到路线本地，没有从输入读取里拿出
可重复的 Core 时间。前车和跟车仍在读全局长度表和限速表。

## 5. 证据

- 父源码是 M3，再往上是 M2 和封存 H1，提交
  `4de40e045398e4b010b2aa36522afc02a4094c4d`。
- Rust 1.98.0，release，locked，offline，`CARGO_INCREMENTAL=0`，
  feature `entry-frontier,barrier-reach`。
- 二进制 SHA-256：
  `19d356ef02da479aeba8935f7698a1f021a64e550145c114c2805c1eceff8a19`。
- 单测 `s1_fixture_compiles_the_single_maneuver_path` 通过，其中核对了
  路线本地长度、限速和 profile 表与全局表一致。未跑完整 runtime 测试套件。
- 结果目录：`target/issue707-m4-results/`。平衡电源方案，WPR 未在录制。
