# #707 第 1 刀：frontier 按 hop 记住

2026-09-22。状态：**保留。** 同一二进制上 frontier 均值从 5.468 ms 降到
4.304 ms，少 1.165 ms。Core 均值从 31.876 ms 降到 30.431 ms，少 1.445 ms。
四臂短窗交通输出与 M3 逐字节一致，重叠对数为 0。16 ms 目标未达到。

合同见 [第 1 刀计划](frontier-replay-plan.md)。

## 1. 候选

`off` 每拍重走路线并重算 ETA。`on` 在同一条边上复用进入该边时记下的冲突
距离，用进度差得到剩余距离，再套原来的近似 ETA。换边、进度回退，或更远的
冲突进入全局时窗时才重走。每一臂都固定屏障跳过、稠密 binding 和阶段计时。

## 2. 性能

100k 长尾、4 workers、33 ms、256 拍，统计 tick ≥ 65 的 192 拍。
顺序是 off、on、on、off。单位是毫秒墙钟。

| 臂 | Core 均值 | 标准差 | p95 | frontier 均值 | 冲突准备均值 |
| --- | ---: | ---: | ---: | ---: | ---: |
| off 1 | 31.875 | 2.478 | 36.090 | 5.433 | 11.604 |
| on 2 | 30.559 | 2.682 | 36.326 | 4.294 | 10.480 |
| on 3 | 30.303 | 2.434 | 34.893 | 4.313 | 10.466 |
| off 4 | 31.878 | 2.370 | 35.871 | 5.504 | 11.648 |

frontier 的两次 off 均值是 5.468 ms，极差 0.071 ms。两次 on 的均值是
4.304 ms。两次 on 都低于两次 off。Core 的 off 极差是 0.003 ms，节省
1.445 ms。两条保留线都成立。p95 仍在 34.9–36.3 ms。

这次带阶段计时的 Core 绝对水平约 30.3–31.9 ms，不把 M3 无计时对照的
29.334 ms 改写成这一次的数字。相对节省来自同一二进制。

## 3. 交通结果

四臂的 `quality.csv`、`ticks.jsonl`、`events.jsonl` SHA-256 与 M3 的 100k
短窗相同。末拍 completed 4,039、parked 25,200，重叠对数 0。这条进度差假设
在这个短窗里没有改变交通输出。

## 4. 保留

研究组合现为 B1 + H1 + 屏障跳过 + 稠密 binding + frontier 按 hop 记住。
源码在 `target/issue707-m8-source`，测量时加 `LF707_COAST=on`，并保持
`LF707_BARRIER=skip` 和 `LF707_BINDING=dense`。没有进入正式 PR。

第 2 刀仍是远车滑行。frontier 已经下降，可以做。16 ms 还要运动循环少做
完整求解。

## 5. 证据

- 父源码是 M3，提交
  `4de40e045398e4b010b2aa36522afc02a4094c4d` 之上的研究 checkout。
- Rust 1.98.0，release，locked，offline，`CARGO_INCREMENTAL=0`，
  feature `entry-frontier,barrier-reach,short-profile`。
- 二进制 SHA-256：
  `5b34dbf98cb8282626c17f3855882b1c7701ad5078e946c28a7b4365c6f0fd0f`。
- 结果目录：`target/issue707-m8-results/`。平衡电源方案，WPR 未在录制。
  未跑完整 runtime 测试套件。
