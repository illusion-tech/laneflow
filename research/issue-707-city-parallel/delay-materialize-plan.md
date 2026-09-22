# #707：延迟物化接近记录

2026-09-22。承接 [本拍约束投影](constraint-project-results.md)。状态：已淘汰，见 [延迟物化结果](delay-materialize-results.md)。下文是跑数前的合同。

held 已经只关心本拍让行会读到的 cell。剩下的开销是：为了这约 564 个 cell，
把大约 2,000 辆车的整段缓存重放一遍，并且每辆车都打开一次路线。

`LF707_DELAY=on` 时，脏缓存仍走原来的整段重走。缓存仍可用的车，只沿着
本拍要查询的 cell 名单，把该车在这个 cell 上的接近记录写进 frontier。
不再为这辆车打开路线，也不再扫描它缓存里本拍用不到的 cell。

预检已经拒绝的候选仍留在原来的仲裁顺序里。拒绝原因来自让行查询，
决定摘要也按这个顺序写入。本轮不把这些候选从仲裁序列里拿掉。

## 三组

Core 用 `public_step_ns`。都不开 `short-profile`。

- best：`target/issue707-m16-untimed-results/binaries/harness-m16-plain.exe`，
  `LF707_COAST=on`、`LF707_DEMAND=held`。不重新编译。
- before：新源码，同样的 coast 和 held，`LF707_DELAY=off`。
- after：同一新源码，`LF707_DELAY=on`。

其余固定 `approx/both/combined/boundary=on/decision=every`、
`LF707_OBSERVE=fused`、屏障跳过、稠密 binding。

100k 长尾、4 workers、33 ms、256 拍。顺序是
best、before、after、after、before、best。统计 tick ≥ 65。

## 采用

1. 六臂 `overlap_pairs` 最大值都是 0。after 的交通文件必须与 before 逐字节相同，
   before 必须与 best 逐字节相同。不一致说明这条写入换了查询结果，即使更快也淘汰。
2. `public_step_ns` 的两次 after 都低于两次 before，并且 before 的极差小于平均节省。
3. 较慢的 after 均值不高于较快的 best。不成立则不替换已封存的 best。
   第 2 条仍单独记录。

16 ms 目标不在本轮完成。M3 的 29.334 ms 不改写。封存的 held 28.462 ms 不改写。
