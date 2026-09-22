# #707：持久近门集合上的按需 frontier

2026-09-22。状态：**淘汰。** 近门集合选对了车，但 frontier 没有下降，
运动消费多了约 1.64 ms。四臂短窗交通输出与 M3 逐字节一致，重叠为 0。
16 ms 目标未达到。

合同见 [持久近门集合计划](near-set-plan.md)。保留组合不变。

## 1. 候选

`off` 仍对每辆 Active 车做 hop 缓存复用。`near` 在换边重走时记下到下一道
Gate 的距离。运动写回时用进度差更新近门集合。下一拍只对这个集合打开路线、
收集让行目标。缓存失效的车仍整段重走。

## 2. 性能

100k 长尾、4 workers、33 ms、256 拍，统计 tick ≥ 65 的 192 拍。
顺序是 off、near、near、off。单位是毫秒墙钟。每一臂都开着 frontier 缓存、
屏障跳过、稠密 binding 和阶段计时。

| 臂 | Core 均值 | frontier | p5 消费 | 运动循环 |
| --- | ---: | ---: | ---: | ---: |
| off 1 | 38.246 | 5.186 | 2.094 | 12.195 |
| near 2 | 39.711 | 5.439 | 3.595 | 13.644 |
| near 3 | 39.478 | 5.453 | 3.597 | 13.637 |
| off 4 | 31.360 | 4.360 | 1.815 | 9.370 |

frontier 的两次 near 是 5.439 ms 和 5.453 ms，没有同时低于两次 off
（5.186 ms 和 4.360 ms）。p5 消费的 near 均值是 3.596 ms，off 均值是
1.955 ms，多 1.64 ms，两次 near 都更高。Core 同样没有成立。
off 的 Core 极差有 6.886 ms，这次不能把绝对水平写成新成绩。

near 每拍的工作量与上一轮全量扫描相同：Active 72,765，让行目标 564，
整段重走 35，索引复用 2,014，近门集合 2,449。短窗输出不变。

路线打开已经限制在近门集合上。frontier 仍要沿 Active 序号读每辆车的缓存槽，
运动消费还要再读一次。这两遍槽位读取把省下的路线打开吃掉了。

## 3. 交通结果

四臂的 `quality.csv`、`ticks.jsonl`、`events.jsonl` SHA-256 与 M8 的 on 臂、
也与 M3 的 100k 短窗相同。重叠峰值是 0。

## 4. 淘汰

研究组合不变，源码仍以 `target/issue707-m8-source` 为准。本轮原型在
`target/issue707-m15-source`。没有进入正式 PR。

564 个目标和约 2,000 辆车这个范围仍然成立。没成立的是维护方式：
只要 frontier 或运动消费还逐车去探缓存，近门集合就不会比 hop 复用更便宜。
下一次若继续，失效和越线只能发生在已经拿着这辆车状态的那一次计算里，
不能再为集合成员另做一遍全车槽位读取。

## 5. 证据

- 父源码是第 1 刀，提交
  `4de40e045398e4b010b2aa36522afc02a4094c4d` 之上的研究 checkout。
- Rust 1.98.0，release，locked，offline，`CARGO_INCREMENTAL=0`，
  feature `entry-frontier,barrier-reach,short-profile`。
- 二进制 SHA-256：
  `a6c0c995db2b9a941cafb337d36c6edba547f39969abcda96b6c46b7542f9009`。
- 结果目录：`target/issue707-m15-results/`。平衡电源方案，WPR 未在录制。
  未跑完整 runtime 测试套件。
  near 臂的 `entry-work.csv` 六列依次是 Active、让行目标、整段重走、
  索引复用、近门集合、处理合计。
