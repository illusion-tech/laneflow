# #707 M4：路线本地的长度、限速和 profile

2026-09-22。承接 [停车 binding 稠密下标](motion-input-results.md)。状态：实验已完成并淘汰。
结果见 [M4 结果](motion-input-local-results.md)。下文是跑数前的合同。

稠密 binding 已保留，Core 均值少 0.629 ms。M1 的输入读取里，路线、长度、
限速和车辆 profile 还在。本轮只改这三处的读取组织，不改跟车、屏障跳过、
停车规则、信号 ETA 或观测频率。

## 候选

每个 Active 车辆在完整运动计算开始时，都要拿当前路段的限速，并按八个独立
数组拼出车辆 profile。硬边界处还要再按路段序号读一次全局长度。
`LF707_INPUT=direct` 保持这两次全局下标和原来的 profile 拼装。
`local` 改为：

- 路线编译时按 `edges` 的顺序记下每段长度和限速。当前路段用路线下标读取，
  不再用城市级长度表和限速表的路段序号。
- 世界安装时把每份 `VehicleProfileView` 抄进一张按 profile 序号索引的表。
  `local` 读这张表，不再每次走八个关系数组。

前车查询和跟车求解仍然使用全局长度表和限速表。两份路线数组和 profile 表
在两种模式下都分配，开关只改读取。未设置环境变量时用 `direct`。
非法取值在安装时失败，与 `LF707_BINDING` 相同。

长度和限速是编译时从同一张全局表抄下来的。profile 表是安装时从同一组关系
抄下来的。短窗交通输出应与 M3 逐字节一致；不一致则淘汰，即使更快。

## 采用

100k 长尾、4 workers、33 ms、256 拍、
`approx/both/combined/boundary=on/decision=every`、`LF707_OBSERVE=fused`。
每一臂都固定 `LF707_BARRIER=skip` 和 `LF707_BINDING=dense`。同一二进制
ABBA：direct、local、local、direct。Core 用 tick ≥ 65 的均值。

同时满足下面四条才保留：

1. 四臂的 `quality.csv`、`ticks.jsonl`、`events.jsonl` SHA-256 与 M3 的 100k
   短窗相同。
2. 两次 local 的 Core 均值都低于两次 direct。
3. 两次 direct 的 Core 均值极差小于平均节省。
4. local 的 Core 均值低于 M3 dense 均值 29.334 ms。新二进制若因为多出来的
   数组把 direct 和 local 一起拖慢，内部对照不能单独作为保留理由。

16 ms 目标不在本轮完成。p95 即使有一臂低于 33 ms，也不写成已经达到。
