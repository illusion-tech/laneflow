# #707：车道块闭合路径

2026-09-22。状态：**淘汰。** 不开阶段计时。`public_step_ns` 的 before
均值 32.972 ms，after 均值 40.503 ms，多 7.530 ms。两次 after 都高于两次
before。六臂短窗交通输出与 M3 逐字节一致，重叠为 0。16 ms 目标未达到。
已封存的 best 不替换。

合同见 [车道块计划](lane-block-plan.md)。M3 记录里的 29.334 ms 不改写。
先前封存的 held 28.462 ms 也不改写。

## 1. 候选

best 是已封存的 `harness-m16-plain.exe`，`LF707_COAST=on`、
`LF707_DEMAND=held`，不重新编译。before 和 after 来自同一份新源码。
before 是 `LF707_LANE=off`。after 是 `LF707_LANE=block`：成员只在进入、
离开、换边时改；车长一致、两辆车都完全在本边上、并且间隙小于到边终点的
距离时，直接用前杠顺序的前一辆。其余仍走 `leader_relation`。
计算和写回仍是 Active 顺序。

Core 用 `public_step_ns`。阶段计时列是 0。

## 2. 性能

100k 长尾、4 workers、33 ms、256 拍，统计 tick ≥ 65 的 192 拍。
顺序是 best、before、after、after、before、best。单位是毫秒。

| 臂 | Core 均值 | 标准差 | p95 | observation 均值 | iteration 均值 |
| --- | ---: | ---: | ---: | ---: | ---: |
| best 1 | 36.591 | 3.037 | 40.749 | 73.080 | 113.430 |
| before 2 | 36.356 | 3.078 | 40.945 | 73.632 | 113.577 |
| after 3 | 41.404 | 3.024 | 46.024 | 75.507 | 120.609 |
| after 4 | 39.601 | 5.206 | 48.099 | 74.427 | 117.658 |
| before 5 | 29.589 | 2.967 | 34.575 | 65.500 | 98.154 |
| best 6 | 33.932 | 4.808 | 40.368 | 72.371 | 109.753 |

before 的均值是 32.972 ms，极差 6.767 ms。after 的均值是 40.503 ms。
较快的 after（39.601）仍高于较慢的 before（36.356）。best 的均值是
35.261 ms，极差 2.659 ms。较慢的 after 高于较快的 best。best 不替换。

这批 best 的 35.261 ms 比封存当天的 held 28.462 ms 高一截。这是本批负载，
不改那条封存记录。before 自己的极差也有 6.767 ms，后半段明显更快。
after 的两次都落在较慢的前半段，并且都高于那次较慢的 before。

## 3. 快路径没有命中

after 的 `entry-work.csv` 在 tick ≥ 65 上，命中均值是 0，未命中均值是
71022.7，整块重建均值是 0，成员数量对齐均值是 0.1。查询都退回了原来的
前车查找，车道块的维护仍然每拍做完。

随后用同一规则加了退回原因，单独跑了一次 after，不进入上面的六臂均值。
tick ≥ 65：车长不一致 64403.6，队首或块外 6619.1，车身未完全在本边 0.0，
前车不可用 0.0，几何条件退回 0.0，命中 0。长尾配置里的车长是 4.0 m、
4.5 m 和 6.0 m。一条边上只要混有不同车长，整条边都不用这条快路径。
本窗里几乎每一条有车的边都是这样。

## 4. 交通结果

六臂的 `quality.csv`、`ticks.jsonl`、`events.jsonl` SHA-256 与 M3 的 100k
短窗相同。重叠峰值是 0。

## 5. 淘汰

研究组合不变。无计时 best 仍是
`target/issue707-m16-untimed-results/binaries/harness-m16-plain.exe`，
SHA-256
`19bf7ecb5dfd93a7f77c4deae10ffabf36f53fb70f13483d0a98cbb0f8992602`。
本轮程序是 `target/issue707-m18-results/binaries/harness-m18.exe`，
SHA-256
`e37c49b2b42fd10f729f9449d73a3b08d3f1a95df6f5faaac9f51d1bc17a3ac8`。
源码在 `target/issue707-m18-source`。没有进入正式 PR。

## 6. 证据

- Rust 1.98.0，release，locked，offline，`CARGO_INCREMENTAL=0`，
  feature `entry-frontier,barrier-reach`。没有 `short-profile`。
- 结果目录：`target/issue707-m18-results/`。平衡电源方案
  `381b4222-f694-41f0-9685-ff5bb260df2e`，WPR 未在录制。
- 未跑完整 runtime 测试套件。
