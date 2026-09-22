# #707：本拍约束投影

2026-09-22。状态：**淘汰。** 不开阶段计时。`public_step_ns` 的 before
均值 27.859 ms，after 均值 31.454 ms，多 3.595 ms。两次 after 都高于两次
before。重叠为 0，但最后一拍完成车辆从 4039 降到 3850，超出 before 的 1%。
已封存的 best 不替换。

合同见 [约束投影计划](constraint-project-plan.md)。M3 记录里的 29.334 ms
不改写。封存的 held 28.462 ms 也不改写。

## 1. 候选

best 是已封存的 `harness-m16-plain.exe`。before 和 after 来自同一份新源码。
before 是 `LF707_PROJECT=off`。after 是 `LF707_PROJECT=on`：车身完全在当前边上、
本拍最大加速度够不到边终点、信号、冲突和路线终点，且没有等待、机动或停车预约时，
先做无前车的 IIDM。运动写回前，按后杠做前缀最小，只缩短行程。
排序键是拍初占用后杠，前车先处理；后车的上限用前车已经写下的下一状态后杠。
行程收到拍初位置时速度写成 0，否则速度不超过这段位移除以步长。
其余车仍走完整求解。

Core 用 `public_step_ns`。阶段计时列是 0。

## 2. 性能

100k 长尾、4 workers、33 ms、256 拍，统计 tick ≥ 65 的 192 拍。
顺序是 best、before、after、after、before、best。单位是毫秒。

| 臂 | Core 均值 | 标准差 | p95 | observation 均值 | iteration 均值 |
| --- | ---: | ---: | ---: | ---: | ---: |
| best 1 | 28.240 | 2.062 | 32.060 | 61.340 | 92.401 |
| before 2 | 28.094 | 2.179 | 32.022 | 60.862 | 91.718 |
| after 3 | 31.264 | 2.869 | 36.543 | 58.912 | 93.046 |
| after 4 | 31.643 | 2.965 | 36.823 | 59.805 | 94.393 |
| before 5 | 27.625 | 2.097 | 31.248 | 60.092 | 90.436 |
| best 6 | 27.628 | 2.013 | 31.350 | 60.046 | 90.396 |

before 的均值是 27.859 ms，极差 0.469 ms。after 的均值是 31.454 ms。
较快的 after（31.264）仍高于较慢的 before（28.094）。best 的均值是
27.934 ms。较慢的 after 高于较快的 best。best 不替换。

## 3. 预测和投影都发生了

after 的 `entry-work.csv` 在 tick ≥ 65 上，无前车预测均值 69003.3，
实际被缩短的车均值 41634.5，边上的车身片段均值 73392.2，参与排序的边均值
7401.3。快路径覆盖了大部分活动车辆，串行的后杠扫描仍让 Core 变慢。

## 4. 交通结果

before 与 best、以及 M3 短窗的 `quality.csv`、`ticks.jsonl`、`events.jsonl`
逐字节相同。after 不同。六臂重叠峰值都是 0。最后一拍 parked 都是 25200。
before 和 best 的 completed 是 4039，after 是 3850，少 189 辆，相当于
4.7%。这超出合同里的 1%。

## 5. 淘汰

停止的是无前车预测再加全量硬投影。不另加一层补偿把这次预测救回来。
这次也没有建立可用的稳定行为复用。

研究组合不变。无计时 best 仍是
`target/issue707-m16-untimed-results/binaries/harness-m16-plain.exe`，
SHA-256
`19bf7ecb5dfd93a7f77c4deae10ffabf36f53fb70f13483d0a98cbb0f8992602`。
本轮程序是 `target/issue707-m19-results/binaries/harness-m19.exe`，
SHA-256
`9d118358a5171676214ac31b2ae55e5d1880550078eb162efcc7c0ed0f82b79d`。
源码在 `target/issue707-m19-source`。没有进入正式 PR。

## 6. 证据

- Rust 1.98.0，release，locked，offline，`CARGO_INCREMENTAL=0`，
  feature `entry-frontier,barrier-reach`。没有 `short-profile`。
- 结果目录：`target/issue707-m19-results/`。平衡电源方案
  `381b4222-f694-41f0-9685-ff5bb260df2e`，WPR 未在录制。
- 未跑完整 runtime 测试套件。
