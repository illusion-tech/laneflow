# #707 第 2 刀：远车滑行

2026-09-22。承接 [frontier 按 hop 记住](frontier-replay-results.md)。状态：实验已完成并淘汰。
结果见 [第 2 刀结果](motion-glide-results.md)。下文是跑数前的合同。

运动循环里 p5 分发约占 9 ms。第 1 刀已经留下 frontier 缓存。这一刀只改运动：
本拍够不着冲突、信号、边终点和前车的车不再做完整求解。占用平移和单独的
SIMD 不在本轮；滑行积分仍是逐车标量。

## 候选

每一臂都固定 `LF707_COAST=on`、`LF707_BARRIER=skip`、`LF707_BINDING=dense`。
`LF707_GLIDE=off` 保持完整求解。`on` 时，下面任一成立就仍完整求解：

- 有等待成员、机动穿越、本拍冲突计划或停车 binding。
- `progress == 0` 且 `carry == 0`。
- 到边终点、下一冲突或下一信号控制点，不超过本拍最大加速度行程。
  行程用全城最大加速度，比单车加速度更保守。
- 还没有前车间隙记录，或记录减去本车已滑行距离后不超过该行程。

其余车辆用记下的加速度积分，速度钳在当前边限速内，不换边。没有前车时，
把上次查询窗当作已知空距，随滑行减少。加速度在完整求解后按速度差更新。

未设置 `LF707_GLIDE` 时用 `off`。非法取值在安装时失败。

这是激进近似。短窗重叠对数必须是 0。末拍 completed 和 parked 与 off
相差不超过 1%。交通文件不要求与 M3 逐字节相同。

## 采用

100k 长尾、4 workers、33 ms、256 拍、
`approx/both/combined/boundary=on/decision=every`、`LF707_OBSERVE=fused`，
并打开阶段计时。同一二进制 ABBA：off、on、on、off。统计 tick ≥ 65。

同时满足下面三条才保留：

1. 四臂 `overlap_pairs` 最大值都是 0，且 on 的末拍 completed、parked 与
   off 均值相差不超过 1%。
2. `motion_loop_ns` 的两次 on 均值都低于两次 off，并且 off 的极差小于平均节省。
3. Core 均值同样满足第 2 条。

16 ms 目标不在本轮完成。运动循环没有下降时，不开始第 3 刀。
