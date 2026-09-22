# #707 M3：停车 binding 稠密下标

2026-09-22。承接 [屏障跳过](motion-barrier-results.md)。状态：实验已完成并保留。
结果见 [M3 结果](motion-input-results.md)。下文是跑数前的合同。

上一轮之后，Motion 里还没动过的最大一块是输入读取。每个 Active 车辆每拍都用
`HashMap` 查询停车 binding，绝大多数结果是没有 binding。本轮把这张表改成按
车辆下标直接取值，不改停车规则、跟车或屏障跳过。

## 候选

`LF707_BINDING=hash` 保持原来的 `HashMap`。`dense` 在安装时按
`vehicle_capacity` 分配槽位，查询只核对代际并读出 `Option`。插入、删除和
占用转移走同一张表。未设置环境变量时用 `hash`，既有测试不改路径。

两张表都在同一个二进制里。性能对照固定 `LF707_BARRIER=skip`，A 为 hash，
B 为 dense。短窗交通输出应保持一致；不一致则淘汰，即使更快。

## 采用

100k 长尾、4 workers、33 ms、256 拍、`approx/both/combined/boundary=on/decision=every`、
`LF707_OBSERVE=fused`。ABBA。Core 用 tick ≥ 65 的均值。dense 的四次对照里
每次都快于 hash，且 hash 均值极差小于平均节省，才保留。16 ms 目标不在本轮完成。
