# #707：同边前车直接读

2026-09-22。承接 [已持有状态上的近门集合](held-set-results.md)。状态：实验已完成并淘汰。
结果见 [同边前车](same-edge-leader-results.md)。下文是跑数前的合同。

运动分发里的前车查询，先在当前边上找后杠最近的车，再沿路线看后面的边。
占用索引每拍已经把每条边上的车身按前杠排好。本轮只替换“前车就在本边、
而且后杠还没到边终点”这一种。不重排运动顺序，也不改写回顺序。

## 候选

`LF707_LEADER=scan` 保持现在的 `leader_relation`。
`ahead` 在占用排序之后，为每辆车记下本边前杠前方、后杠最近的那一辆。
运动时若这条记录的前杠位置等于本车进度，并且间隙严格小于到边终点的距离：

- 间隙不大于接纳窗，就用这辆前车。
- 间隙大于接纳窗，本拍没有前车。后面边上的车只会更远。

其余情况，包括本边没有前车、后杠正好在边终点，仍走原来的查询。
两条臂都开着 `LF707_COAST=on` 和 `LF707_DEMAND=held`。

未设置时为 `scan`。非法取值在安装时失败。

## 采用

100k 长尾、4 workers、33 ms、256 拍、
`approx/both/combined/boundary=on/decision=every`、`LF707_OBSERVE=fused`、
`LF707_BARRIER=skip`、`LF707_BINDING=dense`、`LF707_COAST=on`、
`LF707_DEMAND=held`，并打开阶段计时。
同一二进制 ABBA：scan、ahead、ahead、scan。统计 tick ≥ 65。

同时满足下面三条才保留：

1. 四臂 `quality.csv`、`ticks.jsonl`、`events.jsonl` 的 SHA-256 相同，
   `overlap_pairs` 最大值都是 0。这是同一查询的替换，输出必须一致。
2. `p5_dispatch_ns` 的两次 ahead 均值都低于两次 scan，并且 scan 的极差小于平均节省。
3. Core 均值同样满足第 2 条。分发变快但占用重建把收益吃掉，则淘汰。

诊断 Core 的绝对水平不替换 M3 dense 的 29.334 ms。
16 ms 目标不在本轮完成。
