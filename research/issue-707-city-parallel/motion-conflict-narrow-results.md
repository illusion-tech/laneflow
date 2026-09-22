# #707 M6：收窄仍在执行的冲突停止

2026-09-22。状态：**淘汰。** 同一二进制上 narrow 的 Core 均值比 direct 少 0.529 ms，
小于 direct 两次运行的极差 1.012 ms。短窗交通输出与 M3 逐字节一致。
16 ms 目标未达到。

合同见 [M6 计划](motion-conflict-narrow-plan.md)。保留组合仍见
[停车 binding 稠密下标](motion-input-results.md)。

## 1. 候选

`direct` 继续用两张距离表，并且冲突和等待都扫描。`narrow` 把两段距离合成
一次读取；只有一侧在本拍行程上界内时，另一侧不扫描。reservation 只读准入
hop。`progress == 0` 且 `carry == 0` 仍走全量扫描。每一臂都固定
`LF707_BARRIER=skip` 和 `LF707_BINDING=dense`。

## 2. 性能

100k 长尾、4 workers、33 ms、256 拍，统计 tick ≥ 65 的 192 拍。
Active 均为 70,761–74,392。单位是毫秒。顺序是 direct、narrow、narrow、direct。

| 臂 | Core 均值 | 标准差 | p95 | observation 均值 | iteration 均值 |
| --- | ---: | ---: | ---: | ---: | ---: |
| direct 1 | 38.763 | 2.781 | 43.007 | 72.670 | 115.219 |
| narrow 2 | 38.677 | 2.723 | 42.323 | 74.917 | 117.476 |
| narrow 3 | 38.802 | 2.421 | 42.008 | 75.847 | 118.561 |
| direct 4 | 39.774 | 2.743 | 43.873 | 78.120 | 121.906 |

两次 direct 的 Core 均值是 39.268 ms，极差 1.012 ms。两次 narrow 的均值是
38.739 ms。未四舍五入的均值相差 0.529 ms。较慢的 narrow（38.802）高于较快的
direct（38.763）。direct 极差大于平均节省，合同里的保留条件没有成立。

observation 均值 direct 75.395 ms、narrow 75.382 ms。iteration 相差 0.544 ms，
也落在 direct 的 Core 极差里。四臂 p95 为 42.008–43.873 ms。工作集峰值观测
约 448–451 MiB。

这次二进制的 Core 绝对水平约 38.7–39.3 ms，高于 M3 dense 当天的 29.334 ms。
direct 两次自己就差了 1.012 ms。同一二进制里的 0.529 ms 没有超过这个波动，
不替换保留组合的 Core 数字。

## 3. 交通结果

四臂的 `quality.csv`、`ticks.jsonl`、`events.jsonl` SHA-256 与 M3 的 100k
短窗相同。末拍 completed 4,039、parked 25,200。收窄扫描没有改变这个短窗的
交通输出。

## 4. 淘汰与保留

研究组合仍是 B1 + H1 + 屏障跳过 + 稠密 binding。源码仍以
`target/issue707-m3-source` 为准。M6 原型留在 `target/issue707-m6-source`，
不进入正式 PR。

把够不着的一侧从冲突停止里拿掉，并改成一次距离读取，没有从 Core 里拿出
超过运行间极差的时间。

## 5. 证据

- 父源码是 M3，再往上是封存 H1，提交
  `4de40e045398e4b010b2aa36522afc02a4094c4d`。
- Rust 1.98.0，release，locked，offline，`CARGO_INCREMENTAL=0`，
  feature `entry-frontier,barrier-reach`。
- 二进制 SHA-256：
  `6b78a1cfa2c73a7d511961725557ca65e28d6ad94718da154531270e185af218`。
- 单测 `s1_fixture_compiles_the_single_maneuver_path` 在
  `entry-frontier,barrier-reach` 下通过，并核对了合成距离与两张原表一致。
  未跑完整 runtime 测试套件。
- 结果目录：`target/issue707-m6-results/`。平衡电源方案，WPR 未在录制。
