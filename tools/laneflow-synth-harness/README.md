# LF-SYNTH 执行核验

消费 [`laneflow-synth-generator`][laneflow-synth-generator-readme] 的正式四文件
输入。交通合同来自 [性能基线 §4][performance-baseline]，
消费及证据编码见 [执行核验设计][lf-synth-execution]。

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
- `semantic_contract`：每份语义回执（含初始 `complete=false`）携带当前验收语义
  协议 `lf-synth-execution-2`——entry→exit 以拍初状态独立证明信号侧许可
  （绿灯允许、红灯拒绝、黄灯按 D1 困境区严格不等式 `v_before² > 2·b_e·d_before`，
  等号拒；W4 指定过门必须 Green）。矩阵子进程、worker、oracle 预检与
  retry/restore 引用严格校验该字段；缺失、旧值或未知值一律拒绝，旧报告不得
  转标复用。
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
laneflow-synth-harness worker-matrix <generator-matrix-root> <implementation-commit> <environment-input.json> <new-output-root>
laneflow-synth-harness worker-retry-matrix <runtime-test-binary> <generator-matrix-root> <implementation-commit> <environment-input.json> <new-output-root>
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
启动次序、binary/运行文件 SHA-256、环境记录、每个子进程的观测记录与比较结果，
全部完成才有 `complete=true`。

从冻结提交的干净检出构建并复制二进制到工作树外，在该检出目录内运行矩阵。环境
记录与子进程观测见下文[环境记录与子进程观测][host-observe-section]。工具记录实际
可执行文件的 SHA-256 并核对运行前后身份，不能替代源码到二进制的构建证明。正式
采集期间不修改源码、二进制或输入。

该矩阵只覆盖标称输入和成功路径。非标称交叉组合、强制调度置换、跨 worker
失败/重试、快照恢复、Routing、表现接线与完整五轴仍需各自证据，不能把相等结果
写成产品认证。具体比较与证据边界见[执行设计 §7][lf-synth-execution-matrix]。

`worker-retry-matrix` 覆盖一万/16 ms、十万/33 ms 的 W3/W4 四个标称身份。
它先由普通工具二进制生成四个完整单 worker 正常参考，再调用同一实现的 optimized
Runtime 测试二进制，执行 1/2/4/8/16 worker、三轮独立进程，共 60 次失败重试。
每次保留完整预热与前两拍 guardrail，注入点固定为 `AfterTransitions`。
失败/fresh 两世界均显式安装请求的 worker，确认失败零发布，只重试原 step 输入。

每身份与第一轮单 worker 比较，共 56 对；比较完整语义回执（仅排除 `workers`）、
七个规范快照及失败/重试边界的实际事件、信号与结果。跨 worker 结果也须与同源
正常参考的命令、游标和状态检查点匹配。正常参考全部完成后才启动失败重试；
最多四个进程，全部活动世界（含 fresh）的配置 worker 总和不超过 32。

环境 JSON 除前述字段外还必须含 `test_build_command`。构建测试二进制的命令为：

```text
cargo +1.99.1 test --release --locked -p laneflow-runtime --lib --no-run --message-format=json
```

从 Cargo 的 compiler-artifact 记录提取实际测试 executable，与普通工具二进制一起
从干净冻结检出构建、复制至仓库外并记录身份；不根据文件名猜测构建来源。
两类二进制都在矩阵前后核对 SHA-256。输出目录必须全新；首错停止新启动并保留
已启动材料，不补跑拼接。此入口不采集性能，不证明其他 failpoint、多错误首错、
强制调度置换或快照解码恢复；详见[执行设计 §8][lf-synth-execution-retry]。

## 验证成本计量

验证器成本使用独立的 `synth-verification-cost` 二进制，依据
[交通验证设计 §9][traffic-verification-cost] 消费 W1–W4 的完整正式输入。它复用
普通与 burst 的同一运行驱动、检查集合和会话协议；`run`/`worker-run` 默认通过
单态化 `Noop` 接入，不增加逐车共享原子或额外世界扫描。此入口不接受短窗口或
规则开关，W4 仍只观察 B0/B1 两拍；原 `measure` 的产品计时口径保持原义。

时间与计数分别构建，放入不同目标目录，并以独立进程运行：

```text
cargo +1.99.1 build --release --locked -p laneflow-synth-harness --bin synth-verification-cost --target-dir <timing-target>
cargo +1.99.1 build --release --locked -p laneflow-synth-harness --bin synth-verification-cost --features verification-cost-counts --target-dir <counts-target>
<timing-binary> run <input-directory> <implementation-commit> <workers> <new-timing-output> --mode timing
<counts-binary> run <input-directory> <implementation-commit> <workers> <new-counts-output> --mode counts
```

`workers` 显式传入真实 Runtime；支持范围与普通语义运行相同。timing 拒绝含
`allocation` 或 `verification-cost-counts` 的构建；counts 必须有
`verification-cost-counts`，并拒绝混入原产品 `allocation` 插桩。新 feature 只启用
已有 `stats_alloc` 依赖。二进制装配对应全局 allocator；直接调用库 API 的计数
调用方也必须装配它，入口用计量区间外的实分配探针验证，不把未装配报告成零分配。

每次输出保留原 `report.json`、`ticks.bin`、`events.bin`，burst 另有原
`commands.bin` 与 `batches.json`，可与同输入、同实现、同 worker 的普通运行逐字节
或原有摘要比较。成本材料仅写入 `verification-cost.json`，格式
`lf-synth-verification-cost-1`，包含 `mode`、输入 manifest 与 `input_identity`、
实现提交、worker、完整窗口及已完成边界数。输出目录必须全新；入口先写
`complete=false`，语义执行或 I/O 失败时保留已有材料及错误，不生成通过结论。
`complete=true` 表示该次完整语义运行与成本采集完成，不表示成本预算通过。

时间构建输出 `timing_by_phase_ns`、`boundary_samples` 和 `observation_ns`。
各阶段边界如下；阶段互不嵌套，不能把其和当作整轮墙钟：

| 阶段                    | 计量内容                                                                                                        |
| ----------------------- | --------------------------------------------------------------------------------------------------------------- |
| `setup`                 | Runtime/Sample/burst 安装、共享会话绑定，以及轨迹/事件记录器初始化；不含输入目录验证、Plan 解码与成本元数据绑定 |
| `observation`           | 实际 observer 的事实读取、语义断言、融合的会话登记、边界编码与摘要；包括既有车辆/信号/事件遍历                  |
| `session`               | 驱动器外层的命令数量声明、阶段推进、步进发起与成功结果核销；observer 内的登记仍计入 `observation`               |
| `commands_and_receipts` | burst 边界命令形成、真实 Runtime 命令、结果检查/会话核销、重排、命令收据与批次证据写入                          |
| `runtime_step`          | 真实 `TrafficWorld::step` 调用                                                                                  |
| `evidence`              | 逐边界事件记录、轨迹/流摘要、检查点与循环内刷新输出                                                             |
| `finish`                | 原宿主终结检查、会话 finish 和验证回执编码                                                                      |

`whole_run_wall_ns` 从初始成本文件成功写入后起算，到原语义报告最终写入并返回为止。
它包含输入验证、Plan 解码、成本元数据绑定、样本预分配/维护，以及原语义证据
写入；不含参数准入、目录创建、初始成本文件写入及最终成本摘要转换/文件写入。
这是一条明确包含计量维护成本的宿主墙钟，不宣称已消除仪器开销。终结后的事件
文件/报告写入等不在上述阶段数组中，仍包含在该墙钟内。

每行 `tick` 与 `observation_scope` 标识本行 observer 所见状态；`runtime_step_tick`
标识该行 step 要产生的结果拍，`command_boundary` 标识命令执行边界，未发生则为
`null`。普通 W1/W2 先 step(tick) 再 observe(tick)，burst 先 observe(tick)，再执行
边界 tick 的命令和 step(tick+1)。因此 W4 预热末行含 B0 命令/step，最后观察行没有
新的 step，不能按观察行的 scope 对命令或 Runtime 成本直接分组。`observation_ns`
仅汇总 observer 时间，分 `initial`、`warm_up`、`observation`；初始单点及 W4 的
两点正式观察窗保留原值与最大值，p50/p95/p99 为 `null`。不得以较长预热样本替代
W4 观察尾延迟。

计数构建的时间字段为 `null`。`workload_counts` 按初始/预热/观察分别记录实际
车身区间数 K、单边界 K 最大值及各既有 observer 遍历的入口数和逻辑访问行数。
W 族采用连续走廊坐标，每辆 Active 投影一个区间，Parked/Completed 不产生区间；
K 不等于总人口。普通 W 的车身检查融合在车辆循环中，不虚报第二次 body 遍历。
这些行数不代表 CPU load 次数，也不包括 Runtime 内部、快照和事件记录器内部扫描。
失败回执可能累计末个未完成边界已经发生的访问，`completed_boundaries` 仅计完整
边界；该诊断数据不能作为完整窗口的成本证据。

`allocation.phases` 使用进程全局 `stats_alloc` 的互不重叠区间差值，分别记录各
命名阶段的 alloc/dealloc/realloc 次数及字节数。这是上述受计阶段的分配，排除了
输入验证、Plan 解码、成本绑定、终结后的事件/报告文件写入和成本 JSON 转换；
不得作为整轮总分配。计数器自身只更新预先持有的标量数组，成本样本转换和写出
都在这些区间之外。Runtime 的相关 worker 分配计入；同进程无关并发活动会污染
精确归因，因此正式计数使用专用进程，不能引用并行测试进程的分配数作为预算。

`scratch_high_water` 仅采样宿主拥有的六类 Vec：`sample.previous`、
`sample.crossed`、`sample.trace_bytes`、`burst.lane_order`、`burst.changed` 与
`burst.pending`，记录初始容量、最大长度/容量及按元素大小计算的保留字节高水位。
`capacity_high_water_increases` 是采样时容量创新高的次数，既不是 allocator 的
realloc 次数，也不覆盖采样间发生后又释放的峰值；它不代表总堆或 Runtime 内存。

正式对照前须单独登记基线/受检提交、隔离补丁、实际二进制 SHA-256、构建日志和
feature、工具链、输入身份、workers、完整窗口、运行顺序、安静窗口、各组预算及
受扰/失败拒绝规则。可复用[宿主观测工具][host-observe]的环境与子进程观测能力；
此单轮入口不会自动启动正式矩阵或据性能阈值判断通过。三组比较遵循设计 §9.3，
旧检查器或关闭新增能力的诊断基线仅在独立检出以受审隔离补丁准备，生产入口不提供
降级开关。内部短 fixture 测试只证明插桩不改变语义、真实计数与拍号映射，不替代
完整窗口或安静条件下的正式性能证据。

## 表现与测量

完整矩阵、独立原始数据复核、数值预算缺口与资源口径见
[LF-SYNTH-v2 表现与测量报告][lf-synth-measurement-readme]。
该报告是冻结源码的研究结果，保留产品未认证边界。

接线遵循 [LF-SYNTH 测量设计][lf-synth-measurement]。同一提交的
`measure` 使用真实 Bevy Session/Plugin、Spatial 与 Transform；W2 只调用 Traffic Runtime。
固定选择绑定 logical rank，Completed 暂时没有来源，despawn/spawn 与原子替换
复用同一 Entity。完整批次预检成功后才写入所有 Transform。

先生成独立语义 oracle，再运行串行、轮换顺序的 fresh-process 正式矩阵：

```text
measure prepare-oracles <semantic-binary> <generator-matrix-root> <implementation-commit> <new-oracle-root>
measure matrix <generator-matrix-root> <oracle-root> <implementation-commit> <environment-input.json> <new-output-root> [--rows <label,...>] [--no-load-sampling]
measure stretch-matrix <generator-matrix-root> <oracle-root> <implementation-commit> <environment-input.json> <new-stretch-output> [--no-load-sampling]
measure guardrail <W1-input-directory> <new-guardrail-output>
```

标称矩阵是一万/16 ms 和十万/33 ms，各 10 行：W1 与 W3 的 1/10/50/100%、
W2 Runtime-only、W4 对应规模的固定主行。每行三个全新进程，共 60 次；各轮将行顺序
左移 7 项。stretch 入口仅覆盖十万 W1/16 ms/10% 的三轮，单列为 observation，
不表示其他 stretch 行已测量。guardrail 独立覆盖两种步长的零/一/两拍、两 quantum
backlog 与恢复，并与单拍重放的状态及完整事件字节比较。

环境记录由矩阵生成（见[环境记录与子进程观测][host-observe-section]）。`measure run`
的第四个参数是矩阵写出的 `environment-before.json`：子进程核对其格式、版本、提交与
干净状态，并在 `manifest.json` 中以 `environment_record_sha256` 引用，不再复制
`environment.json`。每个运行另绑定当前 binary SHA-256、输入 manifest、比例、轮次、
完整帧序列与固定选择 SHA-256。源码与 binary 的关联须由干净检出、构建日志和归档
共同证明。各目录必须全新；失败停止矩阵并保留首错与已有材料。

矩阵为每个子进程记录 `process`（退出状态与 1 s 间隔的采样峰值、采样 CPU 时间，
二者是下界），并以 `self_report_check` 核对子进程自报的生命周期驻留峰值（Windows
`peak_working_set_bytes`、Linux `linux_vm_hwm_bytes`，取 `resources.json` 与
`report.json` 的 `final_process`）不小于父进程采样峰值；不满足时 `consistent=false`，
如实记录，不改变运行结论。

逐段负载采样按[测量设计 §7][load-section]默认开启：每轮在帧间（W2 为拍间）经
`laneflow-os-probe` 分段读取整机忙碌 CPU、本进程 CPU、线程就绪等待（仅 Linux）与
缺页，段长不短于 1 s，采样不进入任何计时区间。round 结束时只依据负载记录判定
`undisturbed`、`disturbed`、`undeterminable` 或 `sampling_disabled`，写入
`report.json` 的 `load`。末尾加 `--no-load-sampling` 关闭采样，仅用于采样开销对比；
开关写入 `manifest.json` 的 `load_sampling` 并参与运行身份，两种配置的计时不得
合并统计。`external_load` 阈值已在参考机上校准为 2.5；受标记观测段占比达到 5% 或任一
观测段 `L_ext ≥ 5.0` 时 round 判为受扰；`run_queue_wait` 阈值仍是初值，
在 Linux 上校准前不得用于正式测量。其他机器的校准要求见 §7.4。

正式 latency 矩阵按 §7.5 使用判定：`disturbed` 的 round 整轮废弃，原目录保留，
随即采集一次进程快照写入该目录的 `process-snapshot.json`，再以同一配置、同一轮号
在新进程中重跑，目录名加 `-retry1`、`-retry2`；不改变其余 round 的轮换顺序。
每个（行，轮号）最多重跑 2 次，3 次都受扰、或判定为 `undeterminable` 时停止该行，
其余行继续，矩阵结束后以失败退出并在 `summary.json` 的 `stopped_rows` 中报告，
须由维护者决定。`summary.json` 的 `runs` 记录每次尝试的 `attempt`、`load` 与
`round_status`，`discarded` 登记废弃原因（受标记段及其信号值）与快照摘要；聚合只
使用 `accepted` 的 round。allocation 诊断构建与 `--no-load-sampling` 运行不按负载
废弃。

`matrix` 加 `--rows`（逗号分隔的标称行标签，如 `10000-W1-16-p100`）时只跑这些行，
按标称顺序排列；环境记录、子进程观测、负载判定与重跑、三轮语义比较和聚合与完整
矩阵相同。空列表、重复或非标称行一律拒绝。`summary.json` 记录 `rows_filter`，
`matrix_scope` 标为子集。子集矩阵只用于采样开销对比与 §7.4 阈值校准，不是正式
矩阵结果，不得作为性能基线的测量引用。

正常 release 构建不启用任何测量 feature。独立诊断构建启用 `allocation`，同时启用
已有 execution-evidence 与 pose-profiling；诊断矩阵每行一个完整窗口，其耗时明确排除：

```powershell
cargo +1.99.1 build --release --locked -p laneflow-synth-harness --bins
cargo +1.99.1 build --release --locked -p laneflow-synth-harness --bin measure --features allocation --target-dir target/synth-allocation
```

诊断 binary 也运行相同 `matrix` 命令。普通 binary 与诊断 binary 必须分别归档，
不得在三轮之间覆盖 binary。`stats_alloc` 记录 Traffic Runtime 区间和 lifecycle 区间的
allocation/reallocation 次数及字节；W3 lifecycle 分配单列。Traffic Runtime 计时包围 Bevy
`Step` set，含相邻调度边界开销；W2 则直接包围 `TrafficWorld::step`。
连续 integrated 区间包括 App 调度、生命周期、全部 Runtime steps、逐拍事件消费、
选择映射、Spatial 提取、完整 Transform staging/apply，不暂停或扣除中间区间。
状态快照、文件写入、统计排序放在声明的帧间检查点。

结果文件：

- `ticks.json`：包括预热的每拍 Runtime/lifecycle 时间、帧号、命令数、分配记录和
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
  working set、private bytes、进程生命周期 working-set/commit peak。进程内存经
  [`laneflow-os-probe`][os-probe] 读取（`method = "laneflow-os-probe"`，#1391 起；此前
  为 `GetProcessMemoryInfo/PROCESS_MEMORY_COUNTERS_EX`，Windows 字段名与口径不变）。
  Linux 改报 `linux_vm_rss_bytes`、`linux_vm_hwm_bytes`、`linux_rss_anon_bytes` 与
  `linux_major_page_fault_count`，Windows 专属字段为 `null`；已发布记录不改写。进程峰值包含输入重建、初始化和
  证据缓冲，不能解释为 Traffic Runtime heap peak。Runtime capacity 是 retained storage，
  不能解释为每拍写入量。
- 子进程日志：诊断构建的现有 Adapter/Spatial 容量计数；矩阵汇总每字段最大值，
  保留 capacity、元素大小和实际 batch 长度的不同口径。HashMap capacity 不是含
  allocator/control-byte 开销的完整堆占用。
- `load.json`：逐段负载记录。文件头写平台、逻辑 CPU 数、目标段长、采样开关、
  阈值、各信号来源与不可用原因；每段写首末位置（W2 为拍号）、是否观测段、墙钟、
  各计数差值、`external_load`（`L_ext`）、`run_queue_wait`（`Q`）与标记。不可用
  信号为 `null`，不填零。逐帧缺页差值写入 `frames.json` 的 `page_faults`，W2 写入
  `ticks.json`。
- `report.json`、`summary.json`：原始文件 SHA-256、同源检查、分组统计和三轮聚合。
  percentile 使用 nearest-rank；先求每轮 p50/p95/p99，再取三轮中位数；max 取三轮最坏。

单个完整窗口可用 `measure run` 重现，参数顺序以 `measure --help` 为准。任何开发态
试跑、失败、instrumented 数据和缺少完整环境的运行均须保持各自标签；完成本部分
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
预热。宿主显式设置 `LF_SYNTH_WORKERS=1`；直接调用测试程序也必须显式提供该值。
保存命令前后规范快照，在 `AfterTransitions` 注入一次失败，然后只重试相同
step。W3 运行到前两条观察拍，W4 完成 B1；完整 W3 窗口由正常矩阵证明。

输出包含 `report.json`、七份 `.lfrs` 捕获、`observations.json` 及测试日志。矩阵比较失败前后、独立世界
的命令后状态和 retry/fresh 快照字节，还将输入、批次摘要、游标和最终状态与正常
矩阵对应检查点比较。原始快照用于本次逐值/字节比较，不代表恢复认证；测试构建
耗时不作 latency/tail 结论。失败保留 `complete=false` 及已写材料。

## 环境记录与子进程观测

`worker-matrix`、`worker-retry-matrix`、`measure matrix`/`stretch-matrix` 与
`restore-probe`/`restore-matrix` 使用 [`laneflow-host-observe`][host-observe]
（`host-observe.md` §6）：

- 第三个参数是执行者输入文件 `environment-input.json`，只含环境记录的必填字段与
  构建声明，不得含自动采集字段：

  ```json
  {
    "environment": { "vendor_performance_mode": "无厂商性能模式", "hardware_role": "uncertified" },
    "build_command": "cargo +1.99.1 build -p laneflow-synth-harness --release --locked",
    "test_build_command": "仅 worker-retry-matrix 要求"
  }
  ```

  `hardware_role` 取 `R0`、`P10`、`P100`、`O1` 或 `uncertified`；`vendor_performance_mode`
  不得为空或占位值，没有厂商性能模式时写明“无厂商性能模式”。
- 启动任何子进程前，工具在当前 git 检出内生成 `environment-before.json`
  （`laneflow-environment-record` 第 1 版，工具链为 `+1.99.1`），核对工作树干净、HEAD
  与实现提交参数相同、工具链可用，`P100` 角色另须匹配参考机指纹；任一不满足即
  停止，不启动子进程。全部运行完成后生成 `environment-after.json` 并比较，前后不一致
  时矩阵不标记完整。两份记录的文件名、SHA-256 与字节数写入 `summary.json` 的
  `environment`。
- 每个子进程按“仅直接子进程”范围、1 s 间隔采样，记录在对应运行或错误条目的
  `process` 中。

旧 `environment.json` 字段与新记录的对应：

| 旧字段                                                    | 新位置                                                                |
| --------------------------------------------------------- | --------------------------------------------------------------------- |
| `implementation_commit`                                   | 环境记录 `source.commit`，工具核对与命令行参数相同                    |
| `source_clean`                                            | 环境记录 `source.clean`，由工具判定                                   |
| `rustc_vv`、`cargo_version`、`target`                     | 环境记录 `toolchain.rustc_vv`、`cargo_version`、`host_target`         |
| `build_command`、`test_build_command`                     | 执行者输入文件的构建声明，原样写入 `summary.json`                     |
| OS、CPU、内存、AC、电源计划、厂商模式、后台进程、采集时间 | 环境记录的自动字段、`manual` 与 `processes`，采集时刻为 `recorded_at` |

已发布测量记录与历史 `environment.json` 不改写。

## 快照恢复与继续运行

正式矩阵、独立文件复核与覆盖边界见
[LF-SYNTH-v2 快照恢复核验报告][lf-synth-restore-readme]。

```text
laneflow-synth-harness restore-probe <generator-matrix-root> <implementation-commit> <environment-input.json> <new-output-root>
laneflow-synth-harness restore-matrix <generator-matrix-root> <implementation-commit> <environment-input.json> <new-output-root>
laneflow-synth-harness restore-reference <input-directory> <implementation-commit> <new-output-directory>
laneflow-synth-harness restore-run <input-directory> <implementation-commit> <workers> <reference-directory> <checkpoint-0-or-1> <new-output-directory>
```

先执行非正式 `restore-probe`：八个单 worker 短程参考，每个两个检查点分别以
1/16 worker 恢复，共 32 个新进程。输入与预热不缩短，W1/W2 检查点为预热结束和
随后第 128 拍，继续运行段最多 256 拍；W3 保留真实替换批次前后，W4 保留 B0 前后
并到 B1 结束。分阶段耗时只用于正式运行预算，不产生恢复性能认证。

正式 `restore-matrix` 生成八个完整单 worker 参考与十六个检查点，随后执行
1/2/4/8/16 worker × 三轮，共 240 次新进程恢复。每次重新读取、验证和构建输入，
认证共享路网修订后从 LFRS 恢复，重建宿主身份映射，精确比较下一拍起至原终点的
全部轨迹、事件、命令收据、检查点与末态。最大四个并发进程、配置 worker 合计 32。

环境声明与来源冻结沿用跨 worker 入口，必填 `implementation_commit`、
`source_clean=true`、`rustc_vv`、`cargo_version`、`target`、`build_command`。
目录必须全新；首错停止新启动、收集已启动进程，不自动补跑。输出保留两份 LFRS、
两份宿主检查点、完整参考与恢复后缀、分阶段耗时、SHA-256 文件身份及汇总。
原始材料与副本留仓库外。完整规则见[执行设计 §9][lf-synth-execution]。

## 回归命令

```powershell
cargo +1.99.1 test --locked -p laneflow-synth-harness
cargo +1.99.1 clippy --locked -p laneflow-synth-harness --all-targets -- -D warnings
```

默认测试覆盖两步长相位边界、正式一万人口初始化与短序列、错误状态拒绝、重复
到达/预约拒绝、屏障与窗口边界、输入顺序摘要、输出保留、轨迹/检查点/重试证明
篡改比较和并发子进程失败收集。长窗口与正式失败重试由上述三个矩阵入口显式执行。

[laneflow-synth-generator-readme]: ../laneflow-synth-generator/README.md
[os-probe]: ../../docs/design/os-probe.md
[performance-baseline]: ../../docs/design/core-runtime-performance-baseline.md
[lf-synth-execution]: ../../docs/design/lf-synth-execution.md
[lf-synth-execution-matrix]: ../../docs/design/lf-synth-execution.md#7-标称矩阵的跨-worker-对照
[lf-synth-execution-retry]: ../../docs/design/lf-synth-execution.md#8-标称-w3w4-的跨-worker-失败与重试
[traffic-verification-cost]: ../../docs/design/traffic-verification.md#9-执行成本与确定性
[lf-synth-measurement-readme]: https://github.com/illusion-tech/laneflow/blob/e267d4c9eead22d14f7111555a70c70637cd9f04/research/issue-1166-lf-synth-measurement/README.md
[lf-synth-measurement]: ../../docs/design/lf-synth-measurement.md
[load-section]: ../../docs/design/lf-synth-measurement.md#7-逐段负载记录与受扰轮次
[lf-synth-restore-readme]: https://github.com/illusion-tech/laneflow/blob/e267d4c9eead22d14f7111555a70c70637cd9f04/research/issue-1197-lf-synth-restore/README.md
[host-observe]: ../laneflow-host-observe/README.md
[host-observe-section]: #环境记录与子进程观测
