# #707：按车辆下标放运动热行

2026-09-22。状态：**淘汰。** 分发少了 0.373 ms，消费多了 0.384 ms，运动循环
相差 0.004 ms。短窗交通输出逐字节一致，重叠为 0。16 ms 目标未达到。

合同见 [热行计划](motion-hot-plan.md)。保留组合仍是 frontier 按 hop 记住。

## 1. 候选

`off` 仍从城市表和路线读限速、边长、profile 和冲突距离。`row` 在换边时
把这些数写入按车辆下标索引的槽，同一条边上的后续拍直接读槽。前车查询仍
打开路线并使用全局长度表。求解顺序仍是 Active 序号。

## 2. 性能

100k 长尾、4 workers、33 ms、256 拍，统计 tick ≥ 65 的 192 拍。
顺序是 off、row、row、off。单位是毫秒墙钟。每一臂都开着 frontier 缓存、
屏障跳过、稠密 binding 和阶段计时。

| 臂 | Core 均值 | p5 分发 | p5 消费 | 运动循环 |
| --- | ---: | ---: | ---: | ---: |
| off 1 | 30.937 | 6.624 | 1.821 | 9.297 |
| row 2 | 30.968 | 6.303 | 2.217 | 9.358 |
| row 3 | 30.534 | 6.262 | 2.205 | 9.295 |
| off 4 | 31.261 | 6.688 | 1.832 | 9.365 |

p5 分发的 off 均值是 6.656 ms，极差 0.064 ms，row 均值是 6.282 ms，少
0.373 ms。两次 row 都低于两次 off。p5 消费从 1.827 ms 增到 2.211 ms。
运动循环的 off 均值是 9.331 ms，row 是 9.327 ms。

Core 的 off 均值是 31.099 ms，极差 0.323 ms，row 均值是 30.751 ms。
较慢的 row（30.968）高于较快的 off（30.937），Core 的保留线没有成立。

## 3. 交通结果

四臂的 `quality.csv`、`ticks.jsonl`、`events.jsonl` SHA-256 相同，重叠峰值
都是 0。热行读到的值和原来的表一致。

## 4. 淘汰

研究组合不变，源码仍以 `target/issue707-m8-source` 为准。本轮原型在
`target/issue707-m11-source`。没有进入正式 PR。

槽省下的是当前边的限速、边长、profile 和冲突距离。前车查询仍要打开路线。
填槽发生在写回时，消费阶段把分发省下的时间拿了回去。

## 5. 证据

- 父源码是第 1 刀，提交
  `4de40e045398e4b010b2aa36522afc02a4094c4d` 之上的研究 checkout。
- Rust 1.98.0，release，locked，offline，`CARGO_INCREMENTAL=0`，
  feature `entry-frontier,barrier-reach,short-profile`。
- 二进制 SHA-256：
  `46d6773f2f7185c8e39668d9dd0cb1a395bd3ca3e9dc45d912cd294f2d8203ef`。
- 结果目录：`target/issue707-m11-results/`。平衡电源方案，WPR 未在录制。
  未跑完整 runtime 测试套件。
