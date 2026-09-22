# #707：车道块闭合路径

2026-09-22。承接 [无计时最佳候选](untimed-best-results.md)。状态：已淘汰，见 [车道块结果](lane-block-results.md)。下文是跑数前的合同。

同边前车那张表不再做。本轮的车道块只在进入、离开、换边时改成员，
不在占用重建里再扫后缀。前车是同一物理边上按前杠排在前面的一辆。
车长不一致时后杠顺序可能不同，这种边整条退回原来的查询。
计算和写回仍是现在的 Active 顺序，不再另排一批然后恢复。

## 三组

Core 用 `public_step_ns`。都不开 `short-profile`。

- best：`target/issue707-m16-untimed-results/binaries/harness-m16-plain.exe`，
  `LF707_COAST=on`、`LF707_DEMAND=held`。不重新编译。
- before：新源码，同样的 coast 和 held，`LF707_LANE=off`。
- after：同一新源码，`LF707_LANE=block`。

其余固定 `approx/both/combined/boundary=on/decision=every`、
`LF707_OBSERVE=fused`、屏障跳过、稠密 binding。

100k 长尾、4 workers、33 ms、256 拍。顺序是
best、before、after、after、before、best。统计 tick ≥ 65。

## 采用

1. 六臂 `overlap_pairs` 最大值都是 0。after 的交通文件必须与 before 逐字节相同，
   否则这条前车替换是错的，即使更快也淘汰。
2. `public_step_ns` 的两次 after 都低于两次 before，并且 before 的极差小于平均节省。
3. 较慢的 after 均值不高于较快的 best。不成立则不替换已封存的 best。
   第 2 条仍单独记录。

16 ms 目标不在本轮完成。M3 的 29.334 ms 不改写。
