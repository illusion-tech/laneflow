# #707：在已持有的车辆状态上更新近门集合

> **证据归档**：本目录的历史诊断 JSON 已迁入[冻结归档](../archives/2026-10-01-json-migration.md)。
> 本文的 JSON 链接指向原提交；文中相对 JSON 路径及依赖它们的历史命令按归档内 `source/` 目录解释。

2026-09-22。状态：**作者本地插桩诊断曾支持继续筛查；当前保留依据见后续无
计时封套。** 本页历史记录中，同一二进制的 frontier 均值从 5.329 ms 降到
3.310 ms，Core 均值从 38.198 ms 降到 37.036 ms。四臂短窗交通输出与 M3
逐字节一致，重叠为 0。16 ms 目标未达到。

**证据边界：**精确插桩二进制虽有摘要身份，但四份计时输出和对应可重放封套
只存在于作者本地 `target/`，没有作为项目证据保留。全新克隆不能独立复算本页
的 5.329 → 3.310ms 或 38.198 → 37.036ms；这些数字不得作为可移植的保留证明。
M16 held 当前保留依据是已提交的
[`retained-best-evidence-manifest.json`](https://github.com/illusion-tech/laneflow/blob/bc1bf666a54aebc50a2b7efa50fb1bc3b05ba567/research/issue-707-city-parallel/evidence/retained-best-evidence-manifest.json)、
源码补丁、Research Evidence 制品及[无计时结果](untimed-best-results.md)，其机器
记录为 30.517096 → 28.461283ms。

合同见 [已持有状态计划](held-set-plan.md)。

## 1. 候选

`off` 仍对每辆 Active 车做 hop 缓存复用。`held` 在运动准备复制完 Active
状态之后，用已经在手上的进度和换边时记下的 Gate 距离，写下一批近门集合和
失效名单。frontier 不再沿 Active 序号遍历：只对近门集合打开路线，只整段
重走失效车，其余车靠反向索引复用。

## 2. 作者本地历史性能诊断

100k 长尾、4 workers、33 ms、256 拍，统计 tick ≥ 65 的 192 拍。
顺序是 off、held、held、off。单位是毫秒墙钟。每一臂都开着 frontier 缓存、
屏障跳过、稠密 binding 和阶段计时。

| 臂     | Core 均值 | frontier | p5 准备 | p5 消费 |
| ------ | --------: | -------: | ------: | ------: |
| off 1  |    38.217 |    5.390 |   0.944 |   2.140 |
| held 2 |    36.949 |    3.318 |   1.675 |   2.100 |
| held 3 |    37.123 |    3.302 |   1.673 |   2.137 |
| off 4  |    38.179 |    5.267 |   0.927 |   2.092 |

作者本地记录中，frontier 的 off 均值是 5.329 ms，极差 0.123 ms；held 均值
是 3.310 ms。两次 held 都低于两次 off，节省 2.019 ms，大于 off 的极差。
Core 的 off 极差是 0.038 ms，节省 1.162 ms；两次 held 都低于两次 off。
两条线按当时筛选规则通过，但因缺少项目保留的原始输出与可重放封套，不构成
当前可移植的保留证明。

p5 准备从 0.936 ms 增到 1.674 ms，多 0.739 ms。判断就放在这一段。
p5 消费没有再增加。Core 的 p95 仍在 41.984–42.610 ms。

held 每拍：Active 72,765，让行目标 564，近门集合 2,455，整段重走 33，
索引复用 2,011。目标数与前两轮相同。

这次带阶段计时的 Core 约 37.0–38.2 ms，不替换 M3 无计时的 29.334 ms，
也不把 M8 的 frontier 4.304 ms 改写成 3.310 ms。相对节省来自作者本地的同一
二进制比较，不是可从全新克隆复算的项目证据。

## 3. 交通结果

四臂的 `quality.csv`、`ticks.jsonl`、`events.jsonl` SHA-256 与 M8 的 on 臂、
也与 M3 的 100k 短窗相同。重叠峰值是 0。两拍可达上界没有改变这个短窗的输出。

## 4. 历史推进与当前保留依据

历史筛选把 B1 + H1 + 屏障跳过 + 稠密 binding + frontier 按 hop 记住 +
近门判断放在已持有的运动状态上继续带入后续研究。作者本地源码在
`target/issue707-m16-source`。测量时加 `LF707_COAST=on` 和
`LF707_DEMAND=held`，并保持 `LF707_BARRIER=skip` 和
`LF707_BINDING=dense`。没有进入正式 PR。

当前 M16 held 的保留结论不依赖本页未入库的插桩结果，而由后续无计时封套的
manifest、源码补丁、精确 EXE、fixture、四臂小封套和独立 smoke 共同支撑；详见
[无计时最佳候选](untimed-best-results.md)。

16 ms 仍未达到。本拍 frontier 使用上一拍运动准备写下的名单。新进入
Active、路线替换、句柄 generation 不一致、换边或进度回退的车，本拍还不是
接近来源。自己的门候选仍按活动车辆求值。这个短窗的重叠为 0 只说明同边
车身没有相交。后来的计数见 [覆盖与分辨](calibration-results.md)。

## 5. 作者本地历史证据身份

- 父源码是第 1 刀，提交
  `4de40e045398e4b010b2aa36522afc02a4094c4d` 之上的研究 checkout。
- Rust 1.98.0，release，locked，offline，`CARGO_INCREMENTAL=0`，
  feature `entry-frontier,barrier-reach,short-profile`。
- 二进制 SHA-256：
  `cc1a8beba48581b0fcc64c2cecfccff33e102e98dc816ae0584bc2844eaa0792`。
- 结果目录：`target/issue707-m16-results/`。平衡电源方案，WPR 未在录制。
  未跑完整 runtime 测试套件。
  held 臂的 `entry-work.csv` 六列依次是 Active、让行目标、整段重走、
  索引复用、近门集合、处理合计。

以上身份用于防止误写历史记录；二进制与结果目录没有进入项目保留制品，不能
据此重放本页分阶段数字。
