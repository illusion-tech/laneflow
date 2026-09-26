# #707：封存无计时最佳候选

2026-09-22。承接 [已持有状态上的近门集合](held-set-results.md)。状态：实验已完成。
结果见 [无计时最佳候选](untimed-best-results.md)。下文是跑数前的合同。

M3 dense 的 29.334 ms 是 `motion-input-results.md` 里的记录，二进制没有阶段计时，
也没有 hop 复用和近门判断。跨天的绝对 Core 会漂，以后每一轮都要把封存的
最佳候选放进同一批。本轮先把这个最佳候选封存下来。

## 候选

同一份 `target/issue707-m16-source`，编译时不开 `short-profile`。
Core 用 `public_step_ns`，与 M3 的记录同一口径。

`old`：`LF707_BARRIER=skip`、`LF707_BINDING=dense`、`LF707_COAST=off`、
`LF707_DEMAND=off`。这是 M3 的配置，用今天的负载再跑一次。

`held`：同样的屏障和 binding，加上 `LF707_COAST=on`、`LF707_DEMAND=held`。

其余固定 `approx/both/combined/boundary=on/decision=every`、
`LF707_OBSERVE=fused`。

## 采用

100k 长尾、4 workers、33 ms、256 拍。顺序是 old、held、held、old。
统计 tick ≥ 65 的 192 拍。分位数用 `(n-1)*p` 线性插值。

1. 四臂 `overlap_pairs` 最大值都是 0。`held` 的交通文件必须与 `old` 逐字节相同，
   否则 `held` 不能封存。
2. `public_step_ns` 的两次 held 均值都低于两次 old，并且 old 的极差小于平均节省，
   才把 `held` 封存为无计时最佳。
3. 若不成立，封存 `old`。已有的带计时保留结论不在本轮翻案。
4. old 均值相对记录里的 29.334 ms 只说明今天的负载漂移，不改那份记录。

16 ms 目标不在本轮完成。best、before、after 三组从下一次有新候选时开始；
本轮 before 和候选在同一个二进制上，先分出最佳。
