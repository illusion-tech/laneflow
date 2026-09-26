# #707 M6：收窄仍在执行的冲突停止

2026-09-22。承接 [非前车约束拆分](motion-constraint-split-results.md)。状态：实验已完成并淘汰。
结果见 [M6 结果](motion-conflict-narrow-results.md)。下文是跑数前的合同。

拆分里冲突停止的校正 CPU 最大，约 4.7–4.9 ms，比第二名高出约 2.5 ms。
它没有通过两倍计时开销线，所以那一轮不指定原型。本轮仍只改这一块：
调用方显式要继续做性能。停车停止、信号和输入表不动。

## 候选

`LF707_CONFLICT=direct` 保持现在的冲突停止。`narrow` 在屏障跳过已经打开时：

- 路线编译时把「从 hop 起点到最近冲突」和「到最近等待入口」合成一个
  `u64`。够不着的判断读这一次，不再读两张距离表。
- 只有一侧在本拍行程上界内时，另一侧不扫描，也不参加最近停止点。
- 需要已有 reservation 的准入 hop 时，只读该 hop，不组装完整 owner 权威。

`progress == 0` 且 `carry == 0` 仍走原来的全量扫描。未设置环境变量时用
`direct`。非法取值在安装时失败。两张旧距离表保留，两种模式都分配合成表，
开关只改读取和扫描。

够不着的一侧不返回远处停止点。这与已经保留的屏障跳过同一条边界：
本拍行程到不了的停止点不改变钳制和硬边界。短窗交通输出应与 M3 逐字节一致；
不一致则淘汰，即使更快。

## 采用

100k 长尾、4 workers、33 ms、256 拍、
`approx/both/combined/boundary=on/decision=every`、`LF707_OBSERVE=fused`。
每一臂都固定 `LF707_BARRIER=skip` 和 `LF707_BINDING=dense`。同一二进制
ABBA：direct、narrow、narrow、direct。Core 用 tick ≥ 65 的均值。

同时满足下面三条才保留：

1. 四臂的 `quality.csv`、`ticks.jsonl`、`events.jsonl` SHA-256 与 M3 的 100k
   短窗相同。
2. 两次 narrow 的 Core 均值都低于两次 direct。
3. 两次 direct 的 Core 均值极差小于平均节省。

16 ms 目标不在本轮完成。当天的绝对水平和 M3 的 29.334 ms 不是同一次对照，
不单独作为保留或淘汰条件。
