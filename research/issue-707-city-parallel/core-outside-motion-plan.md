# #707 M7：拆开运动以外的 Core

2026-09-22。承接 [收窄冲突停止](motion-conflict-narrow-results.md)。状态：测量已完成。
下一次原型的目标是冲突准备。结果见 [M7 结果](core-outside-motion-results.md)。下文是跑数前的合同。

冲突停止的收窄没有超过运行间极差。运动里面剩余的桶都已经落在大约 1 ms
的 Core 波动里。M1 当时即使整段运动准备变成 0，其余 Core 仍约 21.5 ms。
本轮只测量这条保留栈上、运动循环以外的 Core 时间，不改交通计算。

## 计时

使用已有的 `short-profile` 阶段计时。它在主线程用 `Instant` 包住整段，
不是逐车 CPU。下面九段在一次 `step` 里顺序执行，互不重叠：

| 段 | 计时名 |
| --- | --- |
| 预检 | preflight |
| 占用索引 | occupancy |
| 等待准备 | waiting_prepare |
| 冲突准备 | conflict_prepare |
| 等待收尾 | waiting_finalize |
| 信号 | signals |
| 冲突收尾 | conflict_finalize |
| 等待输出 | waiting_outputs |
| 提交 | commit |

`motion_loop` 一并记下，但不参加名次。占用索引内部还有 count、layout、
fill、sort。冲突准备内部还有 frontier、p4 和 p3 的准备、分发、消费。
等待准备内部有 p2，运动循环内部有 p5。这些只用来解释赢的那一段。

## 采用

100k 长尾、4 workers、33 ms、256 拍、
`approx/both/combined/boundary=on/decision=every`、`LF707_OBSERVE=fused`、
`LF707_BARRIER=skip`、`LF707_BINDING=dense`。同一诊断二进制连续三次。
统计 tick ≥ 65 的 192 拍。单位是毫秒墙钟。

三次运行里，九段中的第一名相同，并且它相对第二名的平均领先大于它自己
三次均值的极差，才把它定为下一次原型的目标。领先若小于 1 ms，不指定原型。
交通文件 SHA-256 必须与 M3 的 100k 短窗相同；不一致则这次测量无效。
诊断二进制的 Core 绝对水平不替换 M3 dense 的 29.334 ms。
16 ms 目标不在本轮完成。
