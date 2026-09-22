# #707：封存无计时最佳候选

2026-09-22。状态：**封存 held。** 同一二进制、没有阶段计时。
`public_step_ns` 的 old 均值 30.517 ms，held 均值 28.462 ms，少 2.056 ms。
四臂短窗交通输出与 M3 逐字节一致，重叠为 0。16 ms 目标未达到。

合同见 [封存计划](untimed-best-plan.md)。M3 记录里的 29.334 ms 不改写。

## 1. 候选

`old` 是 M3 的配置：屏障跳过、稠密 binding，`LF707_COAST=off`、
`LF707_DEMAND=off`。`held` 在同样的屏障和 binding 上打开
`LF707_COAST=on`、`LF707_DEMAND=held`。Core 用 `public_step_ns`。
阶段计时列是 0。

## 2. 性能

100k 长尾、4 workers、33 ms、256 拍，统计 tick ≥ 65 的 192 拍。
顺序是 old、held、held、old。单位是毫秒。

| 臂 | Core 均值 | 标准差 | p95 | observation 均值 | iteration 均值 |
| --- | ---: | ---: | ---: | ---: | ---: |
| old 1 | 30.511 | 2.032 | 33.809 | 62.623 | 96.005 |
| held 2 | 28.639 | 2.687 | 32.443 | 63.355 | 95.001 |
| held 3 | 28.284 | 1.975 | 31.692 | 62.591 | 93.694 |
| old 4 | 30.523 | 1.959 | 33.907 | 62.732 | 96.188 |

old 的均值是 30.517 ms，极差 0.012 ms。held 的均值是 28.462 ms。
两次 held 都低于两次 old，节省 2.056 ms，大于 old 的极差。
held 的 p95 是 32.443 和 31.692 ms。16 ms 未达到。

同一配置的 old 比 M3 记录的 29.334 ms 高 1.183 ms。这是今天的负载，
不是算法退步。29.334 ms 仍只属于那天的记录。

## 3. 交通结果

四臂的 `quality.csv`、`ticks.jsonl`、`events.jsonl` SHA-256 与 M3 的 100k
短窗相同。重叠峰值是 0。

## 4. 封存

无计时最佳候选是 `held`。以后每一轮的 best 跑这份程序，不重新编译：

- 二进制：`target/issue707-m16-untimed-results/binaries/harness-m16-plain.exe`
- SHA-256：
  `19bf7ecb5dfd93a7f77c4deae10ffabf36f53fb70f13483d0a98cbb0f8992602`
- 环境：`LF707_COAST=on`、`LF707_DEMAND=held`、`LF707_BARRIER=skip`、
  `LF707_BINDING=dense`，以及 `approx/both/combined/boundary=on/decision=every`、
  `LF707_OBSERVE=fused`。不开 `short-profile`。

带计时的研究源码仍是 `target/issue707-m16-source`。那份带计时的
`harness-m16.exe` 留在 `target/issue707-m16-results/binaries/`，不拿来当 best。
本轮没有进入正式 PR。

下一次有新候选时用三组：best 是上面这份封存程序，before 是新源码关掉候选
开关，after 是候选。顺序 best、before、after、after、before、best。

## 5. 证据

- 源码是已保留的近门判断，提交
  `4de40e045398e4b010b2aa36522afc02a4094c4d` 之上的研究 checkout。
- Rust 1.98.0，release，locked，offline，`CARGO_INCREMENTAL=0`，
  feature `entry-frontier,barrier-reach`。没有 `short-profile`。
- 结果目录：`target/issue707-m16-untimed-results/`。平衡电源方案，
  WPR 未在录制。未跑完整 runtime 测试套件。
