# #707：本拍约束投影

2026-09-22。承接 [车道块闭合路径](lane-block-results.md)。状态：已淘汰，见 [约束投影结果](constraint-project-results.md)。下文是跑数前的合同。

车道块不再做。本轮不维护跨拍成员表。普通跟车先按无前车的 IIDM
算出本拍位置，再按同一条物理边上的后杠顺序做一次前缀最小：

`y_i = min(q_i, rear_{i-1} - min_gap_i)`

只缩短行程，不把车向前推。换边、车身跨边、等待、机动、停车预约，
以及本拍最大加速度够得到边终点、信号、冲突或路线终点的车，仍走完整求解。
后杠顺序来自本拍开始时的占用片段。车长可以不同。

## 三组

Core 用 `public_step_ns`。都不开 `short-profile`。

- best：`target/issue707-m16-untimed-results/binaries/harness-m16-plain.exe`，
  `LF707_COAST=on`、`LF707_DEMAND=held`。不重新编译。
- before：新源码，同样的 coast 和 held，`LF707_PROJECT=off`。
- after：同一新源码，`LF707_PROJECT=on`。

其余固定 `approx/both/combined/boundary=on/decision=every`、
`LF707_OBSERVE=fused`、屏障跳过、稠密 binding。

100k 长尾、4 workers、33 ms、256 拍。顺序是
best、before、after、after、before、best。统计 tick ≥ 65。

## 采用

1. 六臂 `overlap_pairs` 最大值都是 0。before 的交通文件必须与 best 逐字节相同。
   after 可以不同。after 最后一拍的 completed 和 parked 都要落在 before 的
   ±1% 以内。重叠不为 0 则淘汰，即使更快。
2. `public_step_ns` 的两次 after 都低于两次 before，并且 before 的极差小于平均节省。
3. 较慢的 after 均值不高于较快的 best。不成立则不替换已封存的 best。
   第 2 条仍单独记录。

16 ms 目标不在本轮完成。M3 的 29.334 ms 不改写。封存的 held 28.462 ms 不改写。
