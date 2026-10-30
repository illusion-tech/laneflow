# LF-SYNTH 执行核验

消费 [`laneflow-synth-generator`](../laneflow-synth-generator/README.md) 的正式四文件
输入。交通合同来自 [性能基线 §4](../../docs/design/core-runtime-performance-baseline.md)，
消费及证据编码见 [执行核验设计](../../docs/design/lf-synth-execution.md)。

## 运行

```powershell
cargo +1.99.1 build --release --locked -p laneflow-synth-harness
```

单个输入的完整窗口：

```text
laneflow-synth-harness run <input-directory> <implementation-commit> <new-output-directory>
```

`implementation-commit` 必须是构建本二进制的完整提交 SHA。使用独立干净工作树从该
提交构建；参数仅记录调用者声明，不替代构建来源证明。输入 manifest 的设计来源和
生成提交保留原值，不因消费实现变化而改写。

W1/W2 八行矩阵及每行两个独立进程：

```text
laneflow-synth-harness matrix <generator-matrix-root> <implementation-commit> <new-output-root>
```

W3/W4 使用同样的输入布局及两轮独立进程：

```text
laneflow-synth-harness burst-matrix <generator-matrix-root> <implementation-commit> <new-output-root>
```

输入根目录采用生成器矩阵布局，例如 `10000-W1-16-a`、`100000-W2-33-a`。
每个消费进程都重新验证四文件和重编译结果。按规模、场景、16/33 ms 排队，
最多四个 fresh-process 消费进程并发，每行两轮；原始日志分别保存。这只是语义
对照，不能用并发进程的耗时评价性能。输出目录必须不存在。

## 结果与边界

- `report.json`：完整输入 manifest、实现提交、窗口、实际计数、逐拍轨迹摘要、初始/
  预热结束/H/2H/4H/最终检查点；成功才有 `complete=true`。
- `ticks.bin`：包含初始边界的全部逐拍公开观测摘要，矩阵逐字节比较两份轨迹。
- `summary.json`：矩阵已通过行、完整回执、独立进程一致性；中断/失败保持
  `complete=false`，已写材料保留，不覆盖或跳过失败行。

W1/W2 初始化后只调用 `step` 和只读查询。W3/W4 按受检计划执行停车和生命周期
命令，实际收据写入 `commands.bin`，批次前后状态写入 `batches.json`；矩阵同时
逐字节比较这些文件。W3 保留到达屏障与单拍入位，W4 在 B1 后立即结束。
上述 `run` / `matrix` / `burst-matrix` 固定单 worker，不隐式回收、替换或缩短窗口。
每拍检查人口/身份、停车占用、精确整数运动、重叠、信号与事件因果。
完整状态摘要只在检查点生成；逐拍轨迹覆盖设计列出的公开字段，不宣称逐拍内部状态
全部受检。完整窗口执行会较长，进度每 1024 拍写入 stderr。

上述入口包含大量宿主核验，只用于语义对照。两轮不能替代性能合同的三轮；
表现与测量使用下述独立 `measure` 入口。跨 worker 成功路径对照使用下一节的
独立矩阵；快照恢复、Routing 与完整五轴认证不在这些入口的范围内。
`Product TBD / Uncertified` 保持不变。

## 跨 worker 确定性

```text
laneflow-synth-harness worker-run <input-directory> <implementation-commit> <workers> <new-output-directory>
laneflow-synth-harness worker-matrix <generator-matrix-root> <implementation-commit> <environment.json> <new-output-root>
```

`worker-run` 走同一完整语义窗口，将显式配置传给真实 `TrafficWorld`，成功回执的
`workers` 从安装后的世界读取。当前支持 1–16，不自动降级；普通 `run` 仍固定为 1。

`worker-matrix` 固定一万/16 ms 与十万/33 ms，各 W1–W4，使用 1/2/4/8/16 worker
各三个全新进程，共 120 次。每轮配置顺序左移七项，最多四个进程且运行中的配置
worker 之和不超过 32。它只核验语义，不采集延迟或加速比。

每个身份与第一轮单 worker 参考作十四次对照，共 112 对。回执核对实际 worker、
源码、完整输入 manifest、窗口和检查点，再只排除 `workers` 比较全部其他字段。
轨迹、有序事件逐字节比较；W3/W4 另比较命令收据与批次。任一失败停止启动新进程，
收集已经启动的运行；不覆盖旧目录，不自动补跑。`summary.json` 保留每次完整回执、
启动次序、binary/运行文件 SHA-256、环境与比较结果，全部完成才有 `complete=true`。

从冻结提交的干净检出构建并复制二进制到工作树外。`environment.json` 至少包含
`implementation_commit`（与参数相同）、`source_clean`（true）、`rustc_vv`、
`cargo_version`、`target`、`build_command`，并附 OS、CPU、内存与采集时间。
这些是执行者声明；工具记录实际可执行文件的 SHA-256 并核对运行前后身份，不能
替代源码到二进制的构建证明。正式采集期间不修改源码、二进制或输入。

该矩阵只覆盖标称输入和成功路径。非标称交叉组合、强制调度置换、跨 worker
失败/重试、快照恢复、Routing、表现接线与完整五轴仍需各自证据，不能把相等结果
写成产品认证。具体比较与证据边界见[执行设计 §7](../../docs/design/lf-synth-execution.md#7-标称矩阵的跨-worker-对照)。

## 表现与测量

完整矩阵、独立原始数据复核、数值预算缺口与资源口径见
[LF-SYNTH-v2 表现与测量报告](../../research/issue-1166-lf-synth-measurement/README.md)。
该报告是冻结源码的研究结果，保留产品未认证边界。

接线遵循 [LF-SYNTH 测量设计](../../docs/design/lf-synth-measurement.md)。同一提交的
`measure` 使用真实 Bevy Session/Plugin、Spatial 与 Transform；W2 只调用 Core。
固定选择绑定 logical rank，Completed 暂时没有来源，despawn/spawn 与原子替换
复用同一 Entity。完整批次预检成功后才写入所有 Transform。

先生成独立语义 oracle，再运行串行、轮换顺序的 fresh-process 正式矩阵：

```text
measure prepare-oracles <semantic-binary> <generator-matrix-root> <implementation-commit> <new-oracle-root>
measure matrix <generator-matrix-root> <oracle-root> <implementation-commit> <environment.json> <new-output-root>
measure stretch-matrix <generator-matrix-root> <oracle-root> <implementation-commit> <environment.json> <new-stretch-output>
measure guardrail <W1-input-directory> <new-guardrail-output>
```

标称矩阵是一万/16 ms 和十万/33 ms，各 10 行：W1 与 W3 的 1/10/50/100%、
W2 Core-only、W4 对应规模的固定主行。每行三个全新进程，共 60 次；各轮将行顺序
左移 7 项。stretch 入口仅覆盖十万 W1/16 ms/10% 的三轮，单列为 observation，
不表示其他 stretch 行已测量。guardrail 独立覆盖两种步长的零/一/两拍、两 quantum
backlog 与恢复，并与单拍重放的状态及完整事件字节比较。

`environment.json` 由执行者采集：记录 `rustc -Vv`、`cargo -V`、构建命令、target、
源码干净状态、OS/CPU/内存、AC、电源计划、厂商模式和后台进程。工具保留这些声明，
不把缺项当成已验证。每个运行另绑定当前 binary SHA-256、输入 manifest、比例、轮次、
完整帧序列与固定选择 SHA-256。源码与 binary 的关联须由干净检出、构建日志和归档
共同证明。各目录必须全新；失败停止矩阵并保留首错与已有材料。

正常 release 构建不启用任何测量 feature。独立诊断构建启用 `allocation`，同时启用
已有 execution-evidence 与 pose-profiling；诊断矩阵每行一个完整窗口，其耗时明确排除：

```powershell
cargo +1.99.1 build --release --locked -p laneflow-synth-harness --bins
cargo +1.99.1 build --release --locked -p laneflow-synth-harness --bin measure --features allocation --target-dir target/synth-allocation
```

诊断 binary 也运行相同 `matrix` 命令。普通 binary 与诊断 binary 必须分别归档，
不得在三轮之间覆盖 binary。`stats_alloc` 记录 Core 区间和 lifecycle 区间的
allocation/reallocation 次数及字节；W3 lifecycle 分配单列。Core 计时包围 Bevy
`Step` set，含相邻调度边界开销；W2 则直接包围 `TrafficWorld::step`。
连续 integrated 区间包括 App 调度、生命周期、全部 Core steps、逐拍事件消费、
选择映射、Spatial 提取、完整 Transform staging/apply，不暂停或扣除中间区间。
状态快照、文件写入、统计排序放在声明的帧间检查点。

结果文件：

- `ticks.json`：包括预热的每拍 Core/lifecycle 时间、帧号、命令数、分配记录和
  step 前实际 Active 人口。后者由宿主状态变化账本维护，并在检查点全量核对；
  不将其冒称为未公开的昂贵 intent 计算次数。
- `frames.json`：包括预热的完整输入、步数、余量、上限标记、生命周期命令数、
  selected/sourced/applied 数量、人口与同帧两段时间。分类优先级为 W4 突发、边界、
  lifecycle、catch-up、普通零拍/单拍；独立字段保留重叠属性。
- `frame-inputs.json`、`selection.json`：计时前冻结的完整输入。W4 observation
  恰好两条 quantum 输入，各一次提取/提交；B0/B1 raw 和 burst max 单列，不做 percentile。
- `events.bin`、`commands.bin`、`batches.json`、`checkpoints.json`：与同输入完整窗口
  oracle 比较的有序事件、命令回执、批次字段与状态检查点。W3 比较共同的
  初始/预热/H/2H/4H/最终检查点；W4 比较初始/预热/B0/B1。
- `resources.json`：声明边界的 Runtime owned-capacity 账本、宿主选择/暂存容量、
  working set、private bytes、进程生命周期 working-set/commit peak。Windows 只读
  `GetProcessMemoryInfo`；其他平台明确报告不可用。进程峰值包含输入重建、初始化和
  证据缓冲，不能解释为 Core heap peak。Runtime capacity 是 retained storage，
  不能解释为每拍写入量。
- 子进程日志：诊断构建的现有 Adapter/Spatial 容量计数；矩阵汇总每字段最大值，
  保留 capacity、元素大小和实际 batch 长度的不同口径。HashMap capacity 不是含
  allocator/control-byte 开销的完整堆占用。
- `report.json`、`summary.json`：原始文件 SHA-256、同源检查、分组统计和三轮聚合。
  percentile 使用 nearest-rank；先求每轮 p50/p95/p99，再取三轮中位数；max 取三轮最坏。

单个完整窗口可用 `measure run` 重现，参数顺序以 `measure --help` 为准。任何开发态
试跑、失败、instrumented 数据和缺少完整环境的运行均须保持各自标签；完成本切片
不解除 P10/P100 平台与数值内存上限 TBD，也不证明完整游戏的 60 FPS。

## 失败重试

先从同一提交构建 optimized Runtime test binary；普通生产二进制没有 failpoint：

```powershell
cargo +1.99.1 test --release --locked -p laneflow-runtime --lib lf_synth_contract --no-run
```

从 Cargo 输出取得 `laneflow_runtime-<hash>` 测试程序的真实路径，再用**已完成的**
同提交 W3/W4 正常矩阵运行：

```text
laneflow-synth-harness retry-matrix <runtime-test-binary> <generator-matrix-root> <implementation-commit> <normal-matrix-output> <new-output-root>
```

八行各启动一个新测试进程，最多两进程并发；每个测试独立安装两个世界，先走完整
预热，保存命令前后规范快照，在 `AfterTransitions` 注入一次失败，然后只重试相同
step。W3 运行到前两条观察拍，W4 完成 B1；完整 W3 窗口由正常矩阵证明。

输出包含 `report.json`、七份 `.lfrs` 捕获及测试日志。矩阵比较失败前后、独立世界
的命令后状态和 retry/fresh 快照字节，还将输入、批次摘要、游标和最终状态与正常
矩阵对应检查点比较。原始快照用于本次逐值/字节比较，不代表恢复认证；测试构建
耗时不作 latency/tail 结论。失败保留 `complete=false` 及已写材料。

## 回归

```powershell
cargo +1.99.1 test --locked -p laneflow-synth-harness
cargo +1.99.1 clippy --locked -p laneflow-synth-harness --all-targets -- -D warnings
```

默认测试覆盖两步长相位边界、正式一万人口初始化与短序列、错误状态拒绝、重复
到达/预约拒绝、屏障与窗口边界、输入顺序摘要、输出保留、轨迹/检查点/重试证明
篡改比较和并发子进程失败收集。长窗口与正式失败重试由上述三个矩阵入口显式执行。
