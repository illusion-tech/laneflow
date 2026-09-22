# #707：运动按路线边成批

2026-09-22。承接 [远车滑行](motion-glide-results.md)。状态：实验已完成并淘汰。
结果见 [按路线边成批](motion-batch-results.md)。下文是跑数前的合同。

P5 分发大约 6.5 ms，是运动循环 9.16 ms 里的大头。逐车求解按 Active
序号往前走，相邻两辆车的边通常不挨着，前车桶和限速表每次都是随机访问。
这一刀只改分发顺序：同一条路线边上的车连着算。热行数组不在本轮。

## 候选

每一臂都固定 `LF707_COAST=on`、`LF707_BARRIER=skip`、`LF707_BINDING=dense`。
`LF707_BATCH=off` 保持 Active 顺序。`edge` 在派发前按
`(路线序号, 路线上的边序号, Active 序号)` 排序再分块。算完后仍按
Active 序号消费，停车到达和边界的顺序不变。

未设置时用 `off`。非法取值在安装时失败。成功路径上的计算是同一批纯函数，
短窗交通输出应与 `off` 逐字节一致；不一致则淘汰，即使更快。

## 采用

100k 长尾、4 workers、33 ms、256 拍、
`approx/both/combined/boundary=on/decision=every`、`LF707_OBSERVE=fused`，
并打开阶段计时。同一二进制 ABBA：off、edge、edge、off。统计 tick ≥ 65。

同时满足下面三条才保留：

1. 四臂的 `quality.csv`、`ticks.jsonl`、`events.jsonl` SHA-256 与 off 的
   第一臂相同，且 `overlap_pairs` 最大值都是 0。
2. `p5_dispatch` 的两次 edge 均值都低于两次 off，并且 off 的极差小于平均节省。
3. Core 均值同样满足第 2 条。

16 ms 目标不在本轮完成。
