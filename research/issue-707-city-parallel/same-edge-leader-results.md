# #707：同边前车直接读

2026-09-22。状态：**淘汰。** 分发少了 0.561 ms，占用重建多了约 1.19 ms，
Core 没有同时低于两次 scan。四臂短窗交通输出逐字节一致，重叠为 0。
16 ms 目标未达到。

合同见 [同边前车计划](same-edge-leader-plan.md)。保留组合不变，仍是
已持有状态上的近门判断。

## 1. 候选

`scan` 仍走原来的 `leader_relation`。`ahead` 在占用排序之后记下本边前杠
前方的最近后杠。运动时只在后杠严格处于边终点之前时使用它，其余仍走原查询。
两条臂都开着 `LF707_COAST=on` 和 `LF707_DEMAND=held`。

## 2. 性能

100k 长尾、4 workers、33 ms、256 拍，统计 tick ≥ 65 的 192 拍。
顺序是 scan、ahead、ahead、scan。单位是毫秒墙钟。

| 臂 | Core 均值 | p5 分发 | 占用 | 运动循环 |
| --- | ---: | ---: | ---: | ---: |
| scan 1 | 31.412 | 6.720 | 4.140 | 10.016 |
| ahead 2 | 31.383 | 6.166 | 5.164 | 9.385 |
| ahead 3 | 31.918 | 6.096 | 5.359 | 9.388 |
| scan 4 | 30.822 | 6.664 | 4.000 | 9.874 |

p5 分发的 scan 均值是 6.692 ms，极差 0.056 ms，ahead 均值是 6.131 ms。
两次 ahead 都更低，少 0.561 ms。占用从 4.070 ms 增到 5.262 ms，多 1.192 ms。
Core 的 scan 均值是 31.117 ms，极差 0.590 ms，ahead 均值是 31.651 ms。
较慢的 ahead（31.918）高于较快的 scan（30.822）。Core 的保留线没有成立。

这次带计时的绝对水平不替换 M3 的 29.334 ms。

## 3. 交通结果

四臂的 `quality.csv`、`ticks.jsonl`、`events.jsonl` SHA-256 与 M3 的 100k
短窗相同。重叠峰值是 0。这条替换没有改变查询结果。

## 4. 淘汰

研究组合不变，源码仍以 `target/issue707-m16-source` 为准，测量时
`LF707_DEMAND=held`。本轮原型在 `target/issue707-m17-source`。
没有进入正式 PR。

同边直接读本身是对的，也确实少做了分发里的查找。代价出在占用重建里
又为每辆车走了一遍后缀最小值。分发省下的 0.561 ms 盖不住这 1.192 ms。

## 5. 证据

- 父源码是已保留的近门判断，提交
  `4de40e045398e4b010b2aa36522afc02a4094c4d` 之上的研究 checkout。
- Rust 1.98.0，release，locked，offline，`CARGO_INCREMENTAL=0`，
  feature `entry-frontier,barrier-reach,short-profile`。
- 二进制 SHA-256：
  `5543c6628e6c07de3bb1cd00cb08b94838cb354bb978f725b50ecceeb8550d08`。
- 结果目录：`target/issue707-m17-results/`。平衡电源方案，WPR 未在录制。
  未跑完整 runtime 测试套件。
