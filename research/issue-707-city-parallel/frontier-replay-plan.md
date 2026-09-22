# #707 第 1 刀：frontier 按 hop 记住

2026-09-22。承接 [运动以外的 Core](core-outside-motion-results.md)。状态：实验已完成并保留。
结果见 [第 1 刀结果](frontier-replay-results.md)。下文是跑数前的合同。

冲突准备里最大的子段是 frontier，约 6.0–6.2 ms。现在每拍清空后，对全部
Active 车重走路线并重算 ETA。这一刀只改这段。远车滑行、占用平移和 SIMD
不在本轮。

## 候选

`LF707_COAST=off` 保持每拍重走。`on` 在 `approx` 模式下：

- 车辆还在同一条边、同一条路线上时，沿用进入这条边时记下的冲突距离。
  本拍剩余距离用进度差减去，再套原来的近似 ETA。不再做
  `distance_to_occurrence_progress` 和冲突后缀扫描。
- 换边、换路线、进度回退，或更远的冲突已经进入全局时窗时，重新走一遍并
  更新记录。
- 证明时窗之外的冲突仍然不插入。记住的到达用最大加速度，偏早，不偏晚。

未设置环境变量时用 `off`。非法取值在安装时失败。`Local` 和 `Queue` 不走
这条缓存。

同一条边上的进度差视为沿路线前进的距离。这是激进假设。短窗里的重叠对数
必须仍是 0。交通文件不要求与 M3 逐字节相同。

## 采用

100k 长尾、4 workers、33 ms、256 拍、
`approx/both/combined/boundary=on/decision=every`、`LF707_OBSERVE=fused`、
`LF707_BARRIER=skip`、`LF707_BINDING=dense`，并打开已有的阶段计时。
同一二进制 ABBA：off、on、on、off。统计 tick ≥ 65。

同时满足下面三条才保留：

1. 四臂 `quality.csv` 的 `overlap_pairs` 最大值都是 0。
2. `frontier_ns` 的两次 on 均值都低于两次 off，并且 off 的极差小于平均节省。
3. Core 均值同样满足第 2 条。frontier 变快但 Core 没有变快，则淘汰。

末拍 completed 和 parked 只作参照，不要求与 off 相同。
诊断 Core 的绝对水平不替换 M3 dense 的 29.334 ms。
16 ms 目标不在本轮完成。frontier 没有明显下降时，不开始第 2 刀。
