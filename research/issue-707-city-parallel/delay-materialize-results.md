# #707：延迟物化接近记录

2026-09-22。状态：**淘汰。** 不开阶段计时。交通文件与 before、best 逐字节相同，
重叠为 0。`public_step_ns` 的 before 均值 37.225 ms，after 均值 34.683 ms。
较慢的 after（38.199）仍高于较快的 before（36.283），保留线没有成立。
已封存的 best 不替换。

合同见 [延迟物化计划](delay-materialize-plan.md)。M3 记录里的 29.334 ms
不改写。封存的 held 28.462 ms 也不改写。

## 1. 候选

best 是已封存的 `harness-m16-plain.exe`。before 和 after 来自同一份新源码，
都开着 `LF707_COAST=on` 和 `LF707_DEMAND=held`。before 是 `LF707_DELAY=off`。
after 是 `LF707_DELAY=on`：脏缓存仍整段重走；缓存还可用的车，只为本拍要查询的
cell 写入接近记录，不再打开路线，也不再扫描该车缓存里用不到的 cell。

Core 用 `public_step_ns`。阶段计时列是 0。

## 2. 性能

100k 长尾、4 workers、33 ms、256 拍，统计 tick ≥ 65 的 192 拍。
顺序是 best、before、after、after、before、best。单位是毫秒。

| 臂 | Core 均值 | 标准差 | p95 | observation 均值 | iteration 均值 |
| --- | ---: | ---: | ---: | ---: | ---: |
| best 1 | 37.310 | 4.129 | 43.231 | 76.844 | 118.263 |
| before 2 | 36.283 | 5.381 | 42.800 | 76.050 | 116.058 |
| after 3 | 38.199 | 3.592 | 43.619 | 79.699 | 121.904 |
| after 4 | 31.168 | 4.954 | 40.229 | 67.725 | 101.995 |
| before 5 | 38.168 | 3.260 | 42.909 | 79.185 | 121.335 |
| best 6 | 36.443 | 3.018 | 40.865 | 75.399 | 115.846 |

before 的均值是 37.225 ms，极差 1.885 ms。after 的均值是 34.683 ms，
少 2.542 ms。两次 after 没有都低于两次 before。best 的均值是 36.877 ms。
较慢的 after 高于较快的 best。best 不替换。

这一批的绝对水平和封存的 28.462 ms 不是同一负载。不改那条记录。

## 3. 写入路径

after 的 `entry-work.csv` 在 tick ≥ 65 上：活动车 72765.0，要查询的 cell
564.1，整段重走 33.1，直接写入 819.9，近门车 2454.5，退回整段重走 2.1。
约 820 条接近记录直接写到约 564 个 cell 上，几乎不再重放整段缓存。

## 4. 交通结果

六臂的 `quality.csv`、`ticks.jsonl`、`events.jsonl` SHA-256 相同，也与
M3 短窗的 best 对照一致。重叠峰值是 0。这条写入没有改变查询结果。

## 5. 淘汰

工程决定是不纳入 best。知识结论是这次没有分辨出稳定收益，不是证明没有收益。
这里延迟的是接近记录。预检拒绝的候选仍留在仲裁顺序里。不能把这次结果写成
P3 候选延迟已经失败。

研究组合不变。无计时 best 仍是
`target/issue707-m16-untimed-results/binaries/harness-m16-plain.exe`，
SHA-256
`19bf7ecb5dfd93a7f77c4deae10ffabf36f53fb70f13483d0a98cbb0f8992602`。
本轮程序是 `target/issue707-m20-results/binaries/harness-m20.exe`，
SHA-256
`15a82fab54ff01824a945f43e30dd1bea74a75fdc251f66aed4626d5804eafe3`。
源码在 `target/issue707-m20-source`。没有进入正式 PR。

## 6. 证据

- Rust 1.98.0，release，locked，offline，`CARGO_INCREMENTAL=0`，
  feature `entry-frontier,barrier-reach`。没有 `short-profile`。
- 结果目录：`target/issue707-m20-results/`。平衡电源方案
  `381b4222-f694-41f0-9685-ff5bb260df2e`，WPR 未在录制。
- 未跑完整 runtime 测试套件。
