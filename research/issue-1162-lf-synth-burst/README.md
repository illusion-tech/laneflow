# LF-SYNTH W3/W4 命令闭环与失败重试验证

## 来源与边界

2026-10-30，Windows x86-64、Rust 1.99.1。正式消费实现固定为
`1b08df57e6e17112e31fc2663a59f6fb2c25bacc`；输入包的设计来源与生成提交均为
`9029c021d1f10a7506eedbf5f2fae3b5a7d57629`。每个消费进程重新校验四文件、三个
输入摘要、规范编码和完整重编译结果；未改写输入来源，也未缩短冻结窗口。

本记录验证 W3/W4 的正式规模命令消费、单 worker 确定性与指定失败点的同拍重试。
不覆盖 Spatial/Adapter、60 Hz 外层帧、性能/分配/内存、worker 扫描、快照恢复、
Routing 或完整五轴认证。`Product TBD / Uncertified` 保持不变；十万 16 ms
仍为原合同的 observation 输入，不升级为产品主行。

## 正常矩阵

**8/8 行通过，16 个全新进程，无本轮失败或重跑。** 共完成 344848 次成功步进，
核验 344864 个边界（含初始边界）、52800 条命令收据和 56 对完整状态检查点。
各行两个进程的 `ticks.bin`、`commands.bin`、`batches.json` 与完整 JSON 回执
逐字节相等。所有实际命令均成功，本轮 `Blocked` 尝试为零。

| 规模   | 场景 | 步长 ms | 预热拍数 | 观察拍数 | 每轮命令数 | 最终 Active / Parked | 每轮过门数 | 实际入位 boundary |
| ------ | ---- | ------- | -------- | -------- | ---------- | -------------------- | ---------- | ----------------- |
| 10000  | W3   | 16      | 14508    | 29016    | 600        | 2500 / 7500          | 2500       | 24986             |
| 10000  | W3   | 33      | 7044     | 14088    | 600        | 2500 / 7500          | 2500       | 12104             |
| 10000  | W4   | 16      | 14508    | 2        | 600        | 7600 / 2400          | 100        | 不适用            |
| 10000  | W4   | 33      | 7044     | 2        | 600        | 7600 / 2400          | 100        | 不适用            |
| 100000 | W3   | 16      | 14508    | 29016    | 6000       | 25000 / 75000        | 25000      | 24990             |
| 100000 | W3   | 33      | 7044     | 14088    | 6000       | 25000 / 75000        | 25000      | 12117             |
| 100000 | W4   | 16      | 14508    | 2        | 6000       | 76000 / 24000        | 1000       | 不适用            |
| 100000 | W4   | 33      | 7044     | 2        | 6000       | 76000 / 24000        | 1000       | 不适用            |

W3 每 cell 的 reservation、arrival、park 和 leave 精确成立；全部到达后按实际
屏障入位，保留一条成功 step 的 24/76 人口，下一 boundary 离位。完成探针的
`Active → Completed` 是宿主观察，随后按冻结输入原子替换；它不冒充 Runtime
转换事件。结束时每个初始 Active 逻辑身份恰好过门一次，Completed 为零。

W4 依次完成 B-1 的完成探针、B0 的 replace/leave/A-green 和 B1 的指定 entry→exit
转换；B0 无过门，B1 恰好每 cell 一个探针过门，随后结束。全部逐拍身份、句柄
失效、人口、泊位绑定/占用者、整数运动、重叠、信号和有序事件断言通过。

W3 16/33 ms 最大观察减速度分别为 868750 / 421213 mm/s²；超 profile 次数保留在
回执。它们来自现行停止线硬投影，不作物理平顺性合格结论。W4 对应最大值为
1375 / 1394 mm/s²，超 profile 次数为零。

## 失败重试矩阵

**8/8 行通过，八个独立 optimized 测试进程。** 每行恰好注入一次预定失败；全部
规范快照逐值断言、快照文件字节比较及正常矩阵对照通过，无本轮非预期失败或重跑。
共保存 56 份原始快照，外部复核的 24 对相等快照摘要全部匹配。

| 场景 | 步长 ms | 失败 boundary | 最终 tick | 规模           |
| ---- | ------- | ------------- | --------- | -------------- |
| W3   | 16      | 14509         | 14510     | 10000 / 100000 |
| W3   | 33      | 7045          | 7046      | 10000 / 100000 |
| W4   | 16      | 14508         | 14510     | 10000 / 100000 |
| W4   | 33      | 7044          | 7046      | 10000 / 100000 |

普通执行二进制不含 failpoint。optimized Runtime test binary 对每行独立安装
两个世界并推进完整预热，在 W3 第一批 atomic replacements 后或 W4 B0 注入
`AfterTransitions`。预期错误为 `ParkingObservationAllocFailed`。

比较命令前/后规范状态、命令批次与 step 输入；失败前后规范快照、状态摘要、
已发布事件、信号和 command cursor 必须不变。仅重试同一 step，不重复命令，
outcome、有序事件和完整规范快照必须等于独立 fresh replay；W4 随后完成 B1。
重试结果还与普通执行对应命令边界和检查点相等。

W3 重试 guardrail 在前两条观察拍结束后停止，完整 W3 窗口由正常矩阵证明。
保存的 LFRS 文件仅用于本次捕获字节比较，没有执行解码恢复或恢复后继续运行，
不构成快照恢复认证。该失败点也不代表所有错误路径已经穷尽。

## 证据

- [正常矩阵回执](normal-summary.json)：完整输入身份、来源/实现提交、实际计数、
  到达/完成观测、命令批次、检查点及双进程精确比较。
- [正常文件独立复核](normal-audit.json)：PowerShell `Get-FileHash -Algorithm SHA256`
  与 .NET `BinaryReader` 核对 16 个轨迹文件的头部、长度和全部连续 tick 编号，
  16 个命令文件的记录数、尝试序号、boundary 和 cursor 连续性；32 对文件摘要相等。
- [失败重试回执](retry-summary.json)：逐行保存预定错误、唯一失败次数、命令批次、
  输入、游标、失败前后及重试状态，全部与正常矩阵对应边界和检查点相等。
- [快照独立复核](retry-audit.json)：56 份 LFRS 文件的完整长度和 SHA-256；每行
  post-command / failed / fresh-post-command 三者相等，retry / fresh 相等。
- [构建环境](environment.json)：完整 rustc/cargo 版本、构建命令、两种二进制的
  SHA-256、并发/worker 配置，无额外 RUSTFLAGS；在干净实现提交上构建并记录。

SHA-256 是供外部通用工具重算的文件互操作摘要，覆盖完整文件，不替换 harness
内部的 BLAKE3 摘要。逐拍轨迹仅覆盖[执行设计](../../docs/design/lf-synth-execution.md)
列出的公开字段及已逐值核验的信号/事件；检查点使用现行完整状态摘要，不声称
每一拍全部内部状态都已比较。

正常矩阵最多四个消费进程并发，重试矩阵最多两个测试进程并发；测试进程内有
独立的失败/正常两世界，每个世界固定一个 worker。本次不采集性能，不把进程
耗时或多世界吞吐作为 Runtime latency/tail 结果。两轮正常对照不替代性能协议三轮。

## 复现与材料

从固定实现提交的干净工作树构建：

```text
cargo +1.99.1 build --release --locked -p laneflow-synth-harness
cargo +1.99.1 test --release --locked -p laneflow-runtime --lib lf_synth_contract --no-run
```

使用固定生成提交的输入矩阵及 Cargo 输出的实际测试二进制路径，先运行正常矩阵，
再运行重试矩阵；所有输出目录必须不存在：

```text
laneflow-synth-harness burst-matrix <generator-matrix-root> 1b08df57e6e17112e31fc2663a59f6fb2c25bacc <new-normal-output>
laneflow-synth-harness retry-matrix <runtime-test-binary> <generator-matrix-root> 1b08df57e6e17112e31fc2663a59f6fb2c25bacc <normal-output> <new-retry-output>
```

原始材料保存在本机工作树外；清理工作树不会删除：

- `E:\projects\laneflow-evidence\issue-1156-9029c021`：正式输入。
- `E:\projects\laneflow-evidence\issue-1162-normal-1b08df57`：正常矩阵、全部逐拍轨迹、
  命令收据、批次、回执和子进程日志。
- `E:\projects\laneflow-evidence\issue-1162-retry-1b08df57`：失败重试矩阵、原始
  LFRS 捕获、回执和测试日志。

父进程日志位于同名目录旁的 `-run.log`。此前 `issue-1162-smoke-v1-*` 为开发态
检查，使用占位实现 SHA，未纳入正式结果；原始目录保留。

## 其他验证

- Harness release 回归 12 项、普通测试构建 12 项通过；涵盖正式一万初始化、
  运动/身份/停车负向检查、信号边界、重复到达/预约、输入顺序摘要、越界/篡改拒绝、
  输出保留、双进程轨迹/命令比较及失败收集。
- Runtime LF-SYNTH 常规合同 5 项通过；正式规模重试测试默认 ignored，单独运行。
- 两包 all-targets Clippy、fmt、Runtime 架构检查、差异空白及提交 hook 通过。
- cargo-deny advisories/bans/licenses/sources 通过，既有依赖警告保留；没有引入
  新外部依赖版本。
- 本地未运行无关 crate 的完整 workspace 测试，由 PR CI 承担跨 crate 回归。
