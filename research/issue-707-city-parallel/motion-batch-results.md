# #707：运动按路线边成批

2026-09-22。状态：**淘汰。** 分发没有变快，准备阶段因为排序多了大约 12 ms。
四臂交通输出逐字节一致，重叠为 0。16 ms 目标未达到。

合同见 [成批计划](motion-batch-plan.md)。保留组合仍是 frontier 按 hop 记住。

## 1. 候选

`off` 按 Active 序号分发。`edge` 先按路线序号和路线上的边序号排序，再把
同一条路线边上的车分到相邻任务里。消费仍按 Active 序号。排序键取自已经
在手上的车辆状态，没有再查车道边。

## 2. 性能

100k 长尾、4 workers、33 ms、256 拍，统计 tick ≥ 65 的 192 拍。
顺序是 off、edge、edge、off。单位是毫秒墙钟。每一臂都开着 frontier 缓存、
屏障跳过、稠密 binding 和阶段计时。

| 臂 | Core 均值 | p5 准备 | p5 分发 | 运动循环 |
| --- | ---: | ---: | ---: | ---: |
| off 1 | 38.668 | 0.938 | 9.282 | 12.491 |
| edge 2 | 57.514 | 13.272 | 9.710 | 33.534 |
| edge 3 | 60.251 | 13.619 | 10.783 | 35.544 |
| off 4 | 37.905 | 0.936 | 9.276 | 12.440 |

p5 准备的 off 均值是 0.937 ms，edge 是 13.446 ms。p5 分发的 off 均值是
9.279 ms，edge 是 10.247 ms。edge 比 off 更慢。Core 大约多 20 ms。

排序搬动的是整份车辆状态，大约 7 万条。分发顺序虽然按路线边挨在一起，
前车查询用的是车道边的占用桶；不同路线可以走同一条车道，这个键没有把
它们放进同一桶。

## 3. 交通结果

四臂的 `quality.csv`、`ticks.jsonl`、`events.jsonl` SHA-256 相同，重叠峰值
都是 0。顺序变化没有改变这个短窗的结果。

## 4. 淘汰

研究组合不变，源码仍以 `target/issue707-m8-source` 为准。本轮原型在
`target/issue707-m10-source`。没有进入正式 PR。

按车辆下标放热行这一条还没做。若再试成批，键要用车道边序号，并且不要在
热路径上排序整份车辆状态。

## 5. 证据

- 父源码是第 1 刀，提交
  `4de40e045398e4b010b2aa36522afc02a4094c4d` 之上的研究 checkout。
- Rust 1.98.0，release，locked，offline，`CARGO_INCREMENTAL=0`，
  feature `entry-frontier,barrier-reach,short-profile`。
- 二进制 SHA-256：
  `6e2e41e53f3387e8997bb9d15dd45eebe0f7483ade0e3cd316c2e5f9b21c1250`。
- 结果目录：`target/issue707-m10-results/`。平衡电源方案，WPR 未在录制。
  未跑完整 runtime 测试套件。
