# #707：运动按车道边成批

2026-09-22。承接 [按车辆下标放运动热行](motion-hot-results.md)。状态：实验已完成并淘汰。
结果见 [按车道边成批](motion-lane-batch-results.md)。下文是跑数前的合同。

按路线上的边序号排序整份车辆状态已经淘汰。前车查询用的是车道边的占用桶。
这一刀只改分发顺序：先取出每辆车的车道边序号，只排序下标，再按这个顺序
分块。不搬动车辆状态去做比较。消费仍按 Active 序号。

## 候选

每一臂都固定 `LF707_COAST=on`、`LF707_BARRIER=skip`、`LF707_BINDING=dense`。
`LF707_BATCH=off` 保持 Active 顺序。`lane` 用车道边序号成批。

未设置时用 `off`。非法取值在安装时失败。成功路径上的计算不变。短窗交通
输出应与 `off` 逐字节一致；不一致则淘汰，即使更快。

## 采用

100k 长尾、4 workers、33 ms、256 拍、
`approx/both/combined/boundary=on/decision=every`、`LF707_OBSERVE=fused`，
并打开阶段计时。同一二进制 ABBA：off、lane、lane、off。统计 tick ≥ 65。

同时满足下面三条才保留：

1. 四臂的 `quality.csv`、`ticks.jsonl`、`events.jsonl` SHA-256 与 off 的
   第一臂相同，且重叠峰值都是 0。
2. `p5_dispatch` 的两次 lane 均值都低于两次 off，并且 off 的极差小于平均节省。
3. Core 均值同样满足第 2 条。准备阶段变慢可以接受，只要 Core 仍然变快。

16 ms 目标不在本轮完成。分发不降，P5 的布局尝试结束。
