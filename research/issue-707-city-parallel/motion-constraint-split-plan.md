# #707 M5：拆开还在执行的非前车约束

2026-09-22。承接 [路线本地输入](motion-input-local-results.md)。状态：测量已完成，没有桶通过排名线。
结果见 [M5 结果](motion-constraint-split-results.md)。下文是跑数前的合同。

保留组合是 B1 + H1 + 屏障跳过 + 稠密 binding。屏障跳过已经去掉本拍
够不着的冲突、等待、信号和 hop 许可。够得着的屏障、停车停止和路终距离
仍在每辆车上计算，M1 没有把它们分开。本轮只测量，不改交通计算，
也不把诊断 Core 写成性能收益。

## 桶

每个 Active 车辆在运动原语里按下面的互斥区间记账。时钟是
`lfence; rdtsc; lfence`。采样拍是运行时 `tick_index` 为 4 的倍数且不小于 64。
未采样拍不读时钟。

| 桶 | 包含的工作 |
| --- | --- |
| waiting | `waiting_stop_for`，含够不着时的提前返回 |
| reach | 调用方为屏障跳过读取 profile 并计算本拍行程上界 |
| conflict | `conflict_stop_for`，含够不着时的提前返回 |
| cache | 运动缓存探测和预览复用判断 |
| route | 当前路段到路线终点的剩余距离 |
| signal | 计算侧的行程上界，加 `signal_stop_distance` |
| parking | `parking_stop_distance` |
| merge | 信号、停车、路终、等待、冲突的停止点合并 |
| hard | 当前路段长度、hop 许可和 `hard_room` |

前车查询、跟车求解、输入表和下一状态不在本轮桶里。`decision=every` 时
资格门控不进入热路径，决策尝试的构造单独留在桶外。

校正值减去每个区间一次空转计时。空转中位数在采样拍上测量。
校正值小于被减去开销的两倍时，该桶不参加名次。

## 采用

100k 长尾、4 workers、33 ms、256 拍、
`approx/both/combined/boundary=on/decision=every`、`LF707_OBSERVE=fused`、
`LF707_BARRIER=skip`、`LF707_BINDING=dense`。同一诊断二进制连续三次。
统计 harness tick ≥ 65 且 `sampled=1` 的拍。CPU 毫秒是四个 worker 之和，
不是墙钟。

三次运行里，可排名桶的校正均值第一名相同，并且它相对第二名的平均领先
大于它自己三次均值的极差，才把它定为下一次原型的目标。交通文件
SHA-256 必须与 M3 的 100k 短窗相同；不一致则这次测量无效。
16 ms 目标不在本轮完成。
