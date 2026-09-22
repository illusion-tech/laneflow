# #707：持久近门集合上的按需 frontier

2026-09-22。承接 [按需 frontier](demand-frontier-results.md)。状态：实验已完成并淘汰。
结果见 [持久近门集合](near-set-results.md)。下文是跑数前的合同。

上一轮把本拍让行目标缩到 564 个 cell、约 2,049 辆车，短窗输出不变，但每拍为发现这些目标打开全部 Active 车的路线，frontier 多了 3.016 ms。本轮不再做这遍路线打开。

## 候选

`LF707_DEMAND=off` 保持按 hop 复用，每辆 Active 车都复用或重走。
`near` 在 `approx` 且 `LF707_COAST=on` 时：

- 整段重走时，把当时到下一道 Gate 的距离记在这辆车的缓存上。
- 运动写回时用进度差和本拍可达上界更新近门集合，并记下缓存已经失效的车。不打开路线。
- 下一拍 frontier 只对近门集合收集让行目标。路线打开只发生在这个集合上。
- 缓存失效的车仍整段重走，反向索引仍只在缓存重写时更新。其余车只在目标 cell 的索引命中时复用。
- 第一拍近门集合还不存在，沿用上一轮的全量发现。统计从 tick ≥ 65 开始，不包含这一拍。

沿 Active 序号写一次并列用的序号，并读缓存槽判断失效。这一遍不打开路线。

未设置环境变量时用 `off`。`near` 但没有打开 `COAST`，或 frontier 不是 `approx`，安装失败。`on` 仍是上一轮已淘汰的全量扫描，本轮不跑。

## 采用

100k 长尾、4 workers、33 ms、256 拍、
`approx/both/combined/boundary=on/decision=every`、`LF707_OBSERVE=fused`、
`LF707_BARRIER=skip`、`LF707_BINDING=dense`、`LF707_COAST=on`，并打开阶段计时。
同一二进制 ABBA：off、near、near、off。统计 tick ≥ 65。

同时满足下面三条才保留：

1. 四臂 `quality.csv` 的 `overlap_pairs` 最大值都是 0。
2. `frontier_ns` 的两次 near 均值都低于两次 off，并且 off 的极差小于平均节省。
3. Core 均值同样满足第 2 条。frontier 变快但 Core 被运动消费吃回去，则淘汰。

末拍 completed 和 parked 与 off 相差超过 1% 则淘汰。短窗交通文件不要求与
M3 逐字节相同；若相同，记下来。
诊断 Core 的绝对水平不替换 M3 dense 的 29.334 ms，也不替换 M8 的
frontier 4.304 ms。
16 ms 目标不在本轮完成。
