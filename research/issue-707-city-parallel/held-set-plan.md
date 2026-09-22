# #707：在已持有的车辆状态上更新近门集合

2026-09-22。承接 [持久近门集合](near-set-results.md)。状态：实验已完成并保留。
结果见 [已持有状态上的近门集合](held-set-results.md)。下文是跑数前的合同。

上一轮近门集合是 2,449 辆车、564 个让行目标，短窗输出不变。frontier 没有下降，
运动消费多了 1.64 ms，因为 frontier 和消费又各读了一遍全部缓存槽。
本轮不再为集合成员单开遍历。

## 候选

`LF707_DEMAND=off` 保持按 hop 复用。
`held` 在 `approx` 且 `LF707_COAST=on` 时：

- 运动准备已经逐辆复制 Active 状态。复制完成后，用同一次持有的状态和换边时
  记下的 Gate 距离做算术判断，写下一批的近门集合和缓存失效名单。
  判断用两拍可达上界，覆盖从准备到下一拍 frontier 之间的那一次运动。
- 这一步只借一次缓存，不在消费阶段再读槽。
- frontier 不沿 Active 序号遍历。只对上一拍留下的近门集合打开路线，
  只整段重走失效名单，其余车仍靠反向索引复用。
- 第一拍名单还不存在，沿用全量发现。统计从 tick ≥ 65 开始。
- 本拍才进入 Active、上一拍准备没见过的车，要到下一拍才进入名单。

未设置时为 `off`。`held` 但没有 `COAST`，或 frontier 不是 `approx`，安装失败。

## 采用

100k 长尾、4 workers、33 ms、256 拍、
`approx/both/combined/boundary=on/decision=every`、`LF707_OBSERVE=fused`、
`LF707_BARRIER=skip`、`LF707_BINDING=dense`、`LF707_COAST=on`，并打开阶段计时。
同一二进制 ABBA：off、held、held、off。统计 tick ≥ 65。

同时满足下面三条才保留：

1. 四臂 `quality.csv` 的 `overlap_pairs` 最大值都是 0。
2. `frontier_ns` 的两次 held 均值都低于两次 off，并且 off 的极差小于平均节省。
3. Core 均值同样满足第 2 条。frontier 变快但准备或消费把收益吃掉，则淘汰。

末拍 completed 和 parked 与 off 相差超过 1% 则淘汰。短窗交通文件不要求与
M3 逐字节相同；若相同，记下来。
诊断 Core 的绝对水平不替换 M3 dense 的 29.334 ms，也不替换 M8 的
frontier 4.304 ms。
16 ms 目标不在本轮完成。
