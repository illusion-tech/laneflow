# #707：只为本拍让行目标构造 frontier

2026-09-22。承接 [需求比例](demand-ratio-results.md)。状态：实验已完成并淘汰。
结果见 [按需 frontier](demand-frontier-results.md)。下文是跑数前的合同。

上一轮在保留栈上看到：约 7.3 万辆车里约 7.27 万辆走缓存复用，真正被让行查询读到的
插入只占约 0.25。本轮把构造方向反过来。远车滑行、车道工作集和约束投影不在本轮。

## 候选

`LF707_DEMAND=off` 保持现在的按 hop 复用：每辆 Active 车都复用或重走。
`on` 在 `approx` 且 `LF707_COAST=on` 时：

- 先找出本拍可能做让行查询的车。判定与 P3 的一拍可达范围相同：下一道以及
  同一拍还能碰到的后续 Gate。受保护候选和拒绝停车不计入。这是查询目标的超集，
  不看最终授予。
- 只为这些目标 cell 插入接近记录。
- 车辆缓存仍然在换边、换路线、进度回退或更远冲突进入时窗时整段重走，并维护
  cell 到车辆的反向索引。缓存有效、且本拍目标用不到的车，不再做 ETA，也不再
  为无关 cell 插入。
- 反向索引只在缓存重写时更新。无效缓存先重写，再按索引取本拍要复用的车。

未设置环境变量时用 `off`。非法取值在安装时失败。`on` 但没有打开 `COAST`，
或 frontier 不是 `approx`，安装失败。

## 采用

100k 长尾、4 workers、33 ms、256 拍、
`approx/both/combined/boundary=on/decision=every`、`LF707_OBSERVE=fused`、
`LF707_BARRIER=skip`、`LF707_BINDING=dense`、`LF707_COAST=on`，并打开阶段计时。
同一二进制 ABBA：off、on、on、off。统计 tick ≥ 65。

同时满足下面三条才保留：

1. 四臂 `quality.csv` 的 `overlap_pairs` 最大值都是 0。
2. `frontier_ns` 的两次 on 均值都低于两次 off，并且 off 的极差小于平均节省。
3. Core 均值同样满足第 2 条。frontier 变快但 Core 没有变快，则淘汰。

末拍 completed 和 parked 与 off 相差超过 1% 则淘汰。短窗交通文件不要求与
M3 逐字节相同；若相同，记下来。
诊断 Core 的绝对水平不替换 M3 dense 的 29.334 ms，也不替换 M8 的
frontier 4.304 ms。本轮 on 与 off 的差才是这把刀的结果。
16 ms 目标不在本轮完成。
