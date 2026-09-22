# #707：按车辆下标放运动热行

2026-09-22。承接 [运动按路线边成批](motion-batch-results.md)。状态：实验已完成并淘汰。
结果见 [热行结果](motion-hot-results.md)。下文是跑数前的合同。

按路线边排序整份车辆状态已经淘汰：准备阶段多约 12 ms，分发没有变快。
这一刀不排序。车辆换边时，把当前边的限速、边长、到下一冲突和等待入口的
距离，以及车辆 profile，写进按车辆下标索引的槽。同一条边上的后续拍直接
读这个槽。求解顺序仍是 Active 序号。

## 候选

每一臂都固定 `LF707_COAST=on`、`LF707_BARRIER=skip`、`LF707_BINDING=dense`。
`LF707_HOT=off` 仍读城市表和路线上的距离。`row` 在槽与当前路线、边、代际
一致时：

- 冲突停止的够不着判断用槽里的两段距离，不再为这个判断打开路线。
- 当前边限速、边长和 profile 用槽里的值。

前车查询和跟车仍读全局长度表，并仍打开路线上的边表。槽在运动结果写回时
更新，只在换边、换路线或槽无效时重填。

未设置时用 `off`。非法取值在安装时失败。这是精确替换。短窗交通输出应与
`off` 逐字节一致；不一致则淘汰，即使更快。

## 采用

100k 长尾、4 workers、33 ms、256 拍、
`approx/both/combined/boundary=on/decision=every`、`LF707_OBSERVE=fused`，
并打开阶段计时。同一二进制 ABBA：off、row、row、off。统计 tick ≥ 65。

同时满足下面三条才保留：

1. 四臂的 `quality.csv`、`ticks.jsonl`、`events.jsonl` SHA-256 与 off 的
   第一臂相同，且重叠峰值都是 0。
2. `p5_dispatch` 的两次 row 均值都低于两次 off，并且 off 的极差小于平均节省。
3. Core 均值同样满足第 2 条。

16 ms 目标不在本轮完成。
