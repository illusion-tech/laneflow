# LF-SYNTH-v2 表现与测量接线

**文档状态**：Accepted（[#1165][issue-1165]）；
设计已接受，实施与正式测量由 #1166 承担，不能据此宣称实现或产品认证完成。
§7 逐段负载记录与受扰轮次已经 [#1388][issue-1388] G1 接受，实现由 #1388 承担。

**适用范围**：LF-SYNTH-v2 宿主 harness 的固定表现选择、外层帧输入与同次运行计时。
交通输入、步进规则和产品预算仍由
[`core-runtime-performance-baseline.md`][performance-baseline]、
[`lf-synth-input-bundle.md`][lf-synth-input-bundle] 和
[`lf-synth-execution.md`][lf-synth-execution] 约束。

## 1. 需要裁决的接缝

性能基线 §4.3 要求 W4 的 B0、B1 分别完成 Spatial extract 与 Adapter apply，
同时禁止两拍之间插入 outer-frame input。Bevy 合同 §4/§7 则规定每个 outer frame
最多一次表现提取和提交。这两项要求无法直接组合成两个各自完整的 integrated frame。
当前语义 harness 没有 Spatial/Adapter，已有 W4 状态与重试证据不裁决这一接缝。

保持生产 Adapter 的每帧一次提交合同，把 W4 明确定义为两个连续的指定外层帧。
这会修订 W4 的**宿主测量输入**，不改变其 topology、state、input-plan、命令次序、
Runtime step 输入、B0/B1 状态或 observation_ticks。性能基线 §4.3/§8 与本文同步，
只允许这两次声明的 observation 输入；统计口径按 §4 执行。

另一方案是在同一外层帧中提交两次表现，会改变 Adapter 的批次频率合同，并使 W4
偏离普通接入路径，因此不建议。只在 B1 后提交一次表现则缺少 B0 的规定测量，
不能满足当前 W4 验收。

## 2. 固定表现集合与生产接线

每个 run 启动时，按性能基线 §4.4 的 SplitMix64 排序从全部 logical ranks 选择
精确比例。集合跨 parked、Completed、despawn/spawn 和 atomic replacement 保持
不变；不能按当前状态重选。每帧按固定 rank 顺序解析当前句柄，分别报告选择数、
具有来源的记录数和实际应用数。Completed 没有来源不等于身份消失或重新选择。

W1/W3/W4 使用真实 `LaneFlowSession`、同根 `SpatialSession`、
`LaneFlowPlugin` 与 Bevy ECS Transform；`max_catch_up_steps = 2`。
生命周期系统在 `Lifecycle` 执行冻结命令，`Observe` 恰好一次消费每个成功 Tick
的结果和有序事件，包括同帧的中间拍。已绑定车辆必须经 typed despawn/replace
组合事务更新映射，不能绕过 Session 的映射正本。

每帧步进结束后最多一次调用 `extract_selected_committed_pose_batch`，复用缓冲。
宿主对完整批次检查 context、placement token、句柄/Entity 映射与有限 Transform，
全部暂存后统一提交；任一失败保留所有旧 Transform，并使本 round 失败。
这不恢复尚未交付的公共 placement API，不把宿主转换写入 Runtime 或 Spatial。

W2 继续使用 Runtime-only 路径；不安装 Spatial/Adapter，不产生 integrated frame 数据。

## 3. W1/W3 的确定性外层帧输入

以整数纳秒构造 60 Hz 序列，令 `t(n) = floor(n * 1_000_000_000 / 60)`，
第 n 帧输入为 `t(n + 1) - t(n)`，n 从零开始。使用完整 `Duration`，不能每帧
取整为 16 ms 或 17 ms。帧输入不取决于本机执行耗时；测得的服务时间不能据此
解释为真实墙钟排队延迟或完整 60 FPS SLA。

为精确停在 warm-up、observation 内 H/2H/4H 及最终边界，在可能越过该边界时，
把本帧 delta 截至目标累计模拟时间。截短值必须大于零，显式保存在输入序列中，
并标记 `boundary_frame`；后续继续使用下一个 n，不清空 Session accumulator，
不丢弃已输入时间。相邻目标相同时只建立一个边界，不生成补零帧。
每个边界满足目标 tick、目标 time 和零 backlog；执行不满足时该 round 失败。

这些边界帧保留原始样本并单独统计；不混入普通 60 Hz 帧的预算统计。每个成功的
observation Runtime tick 仍进入 Traffic Runtime 统计，不能因其属于边界帧而删除。输入生成后
先冻结完整序列，再开始预热；相同 run 配置必须产生相同序列。

追赶是独立 guardrail 序列，不在运行中临时加延迟以修改正常 latency row。序列
显式覆盖零输入、亚 quantum 输入、零/一/两拍和超过上限的输入；至少一个输入从
零 backlog 开始提供四个 quantum，使两拍后仍保留两个 quantum。之后提供零输入
直至恢复到小于一个 quantum。逐帧报告输入、成功步数、剩余 backlog、上限标记和
恢复帧数，保留全部中间结果与事件；不把该序列混入普通帧 percentile。

## 4. W4 两个指定帧

预热使用 §3 的序列，在 `warm_up_ticks` 精确结束且 backlog 为零。随后只输入：

1. **B0 帧**：delta 为一个固定 quantum。执行 pending replace、Parking leave、
   B0 Runtime step、有序结果消费、一次 Spatial 提取和一次 Adapter apply。
2. **B1 帧**：紧接 B0 帧成功后输入一个固定 quantum。执行 route-transition
   probe commands、B1 Runtime step、有序结果消费、一次提取和一次 apply，立即结束。

这两个输入是 W4 声明的唯一 observation 外层输入。两帧之间不插入第三次输入、
额外步进、额外表现提交或其他宿主命令。probe commands 的边界和顺序仍按性能
基线 §4.3；B1 读取 B0 已提交的 green snapshot。任何失败停止本 round。

B0/B1 是固定步长驱动的因果突发帧，不是 60 Hz cadence 样本；不进入普通帧或
追赶帧 percentile。每轮分别保存 Traffic Runtime、Spatial+Adapter、完整帧的 B0/B1 原始值
和 `burst_max`，三轮报告该层 `burst_max` 的 median 与 worst。不从两个样本
推导 p95/p99，也不把两帧相加冒充一次普通外层帧。

已有 D 证据继续证明相同交通输入的状态与失败重试语义；它不成为这些新增表现/
计时路径的证据。E 必须重新核对 B0/B1 的状态、命令游标、批次和事件，并覆盖
表现失败时整批不写、成功替换保持选定逻辑身份与 Entity 连续性。

## 5. 计时与证据边界

正式 latency binary 不启用 allocator instrumentation、failpoint 或 profiling。
每个 integrated 样本用连续墙钟区间覆盖该帧 lifecycle、全部成功 Runtime steps、
有序结果消费、选择映射、Spatial 提取、Transform staging/apply；不得暂停、扣除
中间区间或把不同进程/不同帧的 component percentile 相加。Traffic Runtime 和表现分段计时
必须来自同一次调用，原始记录带相同 frame/tick 编号。

完整状态快照、独立 oracle 比较、统计排序与文件落盘放在声明的帧间检查点；
正常服务路径自身需要的命令/事件消费和映射校验不得移出计时。若实现采用额外
全量审计，必须明确标记其开销是否在帧内，并单独保留对应运行配置，不能无记录地
减去该耗时。帧间工作不改变冻结 delta 序列。

报告分别保留普通零拍帧、普通单拍帧、两拍/达到上限的追赶帧、生命周期突发帧与
边界帧；用独立标记表达重叠属性，不能重复累计样本或把突发混入普通帧。所有行
保留样本数和完整 raw 数据；一万普通单拍 integrated budget 只应用到对应组。

运行清单绑定源码提交、工具链、binary SHA-256、输入 manifest、固定选择集合、
完整 outer-frame delta 序列及其 SHA-256、轮次、全部配置和环境。外层帧序列属于
run 配置，不改写既有交通输入文件和 workload identity；它与表现比例一起参与
运行身份。任何失败保留不完整目录与首错，不以补跑覆盖原始记录。

三轮 fresh-process、轮换顺序、两档规模/表现比例、W2、allocation 独立诊断、
component capacity/high-water、working set/private bytes/commit peak、环境
provenance 与产品 TBD 边界继续按性能基线 §4/§5/§8 执行。实现 Issue 必须给出
明确测量入口和字段口径；本提案不声明这些工具已经存在。

## 6. G1 接受与实现前置

G1 已接受 §1/§4 的 W4 两帧方案，以及 §3 的确定性输入与边界帧统计方案。
性能基线的对应条款同步后，E 按本设计进入代码实现。
不新增 Runtime 生产 API、不改变 Adapter 每帧一次提交合同，也不改变 LFCA、LFRS
或既有交通输入编码。

实现至少验证：固定选择的测试向量及生命周期继承；16/33 ms 的零/一/两拍、余量
与 backlog 恢复；每拍命令/事件恰好消费一次；W4 恰好两拍两帧；提取到 Transform
的完整批次失败原子性；与语义路径相同的状态/事件；计时分类、原始样本和三轮
聚合的可复核性。完整正式矩阵和资源记录完成前，E 不得标记完成。

## 7. 逐段负载记录与受扰轮次

**本节状态**：Accepted（[#1388][issue-1388] G1，2026-11-16）；§7.4 阈值为初值，按该节
校准后于实现 PR 定稿。设计已接受不等于实现完成。

### 7.1 目的与边界

性能基线 §8.4 要求外部干扰导致废弃时重跑整个 round，但 harness 只在整组矩阵运行
前后各记录一次环境与后台进程，没有逐轮负载记录，受扰 round 只能事后靠跨轮对比推断
（[#1273][issue-1273]）。本节规定 harness 在每个 round 内逐段记录系统负载信号，
并按预先声明的规则判定 round 是否受扰。

- 判定只读负载记录，不读计时结果。不得依据 max、p99 或超限帧决定废弃哪一轮，
  否则等同于按结果挑选 round。
- 采样调用全部位于帧间（W2 为拍间）不计时位置，不进入 §5 的任何计时区间，
  不改变冻结的 delta 序列。
- 不使用 WMI、ETW 会话、性能计数器服务或常驻采集进程。这些方式经服务进程转发，
  自身就会产生被测干扰。只使用本进程直接发起的系统调用或 `/proc` 读取。
- 工作区禁止 `unsafe`。所需系统接口经 [`os-probe.md`][os-probe] 登记的受审计例外
  `laneflow-os-probe` 取得，不在 harness 内另开 `unsafe`；该 crate 不提供的信号
  如实记为不可用。
- 没有标记不能证明未受扰：内存带宽、SMT 同核争用、降频与温控等干扰不一定体现在
  下列信号中。降频不设标记：Windows 的线程与进程周期计数可能以固定速率计数、不随
  实际频率变化，以周期数除以 CPU 时间得不到实际频率；不经服务进程、可经安全封装
  取得的频率信号目前没有可用来源。

### 7.2 采样项

每段记录下列累计量在段首与段尾的差值：

| 信号               | Windows                                    | Linux                                    | 用途               |
| ------------------ | ------------------------------------------ | ---------------------------------------- | ------------------ |
| 墙钟               | 单调时钟                                   | 单调时钟                                 | 段长               |
| 整机忙碌 CPU 时间  | `GetSystemTimes`（内核时间含空闲，须扣除） | `/proc/stat` 汇总行                      | 外部 CPU 占用      |
| 本进程 CPU 时间    | `GetProcessTimes` 用户与内核时间           | `/proc/self/stat` 的 utime 与 stime      | 外部 CPU 占用      |
| 本进程线程就绪等待 | 不可用                                     | `/proc/self/task/*/schedstat` 第二列之和 | 直接反映被抢占时长 |
| 缺页次数           | `GetProcessMemoryInfo` 的 `PageFaultCount` | `/proc/self/stat` 的 minflt 与 majflt    | 逐帧归因           |

派生量：

- **外部负载** `L_ext = (Δ整机忙碌 − Δ本进程 CPU) / Δ墙钟`，单位为逻辑 CPU 当量，
  小于零时记为零。
- **就绪等待** `Q = Σ Δrun_delay / Δ墙钟`，单位为线程当量，仅 Linux。各线程之和在
  线程退出时可能减少（[`os-probe.md`][os-probe] §3.4）；差值为负的段 `Q` 记为
  `null`，不按零处理。
- Windows 活动处理器组多于一个时整机 CPU 时间不可用（`os-probe.md` §3.2），
  `L_ext` 与 `external_load` 判定随之不可用，如实记录。

缺页另按帧记录：每个计时帧开始前与结束后各读一次，差值写入该帧原始记录（W2 按拍）。
两次读取都在计时区间外。

### 7.3 分段与记录

- 墙钟距段首达到 1 s 后，在第一个帧间位置结束当前段并开始下一段；W2 用拍间位置。
  H/2H/4H 边界与 checkpoint 不强制分段。
- W1–W3 在预热与观测分界处强制分段，使每段整段属于预热或观测。各阶段末尾不足 1 s
  的段并入同阶段的前一段，因此观测期的每一段都不短于 1 s。
- W4 的观测期只有 B0/B1 两帧，墙钟远短于 1 s，因此不在分界处强制分段：round 末段
  不足 1 s 时并入前一段，使包含 B0/B1 的末段不短于 1 s，并含紧邻的预热帧。该段作为
  观测段参与判定；其中的预热帧只用于判定，不进入任何统计。
- 参与判定的段墙钟不得短于 1 s。Windows 的 CPU 时间按时钟中断（约 15.6 ms）累计，
  段越短 `L_ext` 的量化误差越大；段长不短于 1 s 时，误差估计约为 0.04 逻辑 CPU
  当量，远低于 §7.4 的初值，实测噪声以 §7.4 的校准为准。整个 round 墙钟不足 1 s、
  无法构成合格判定段时，该 round 记为无法判定并报告维护者，不按未受扰处理。
- 段记录写入每轮目录的 `load.json`，并列入 `files` 文件摘要清单。每段包含段序号、
  首末帧（W2 为首末拍）、是否观测段、墙钟、§7.2 各差值、派生量与标记列表。不可用
  信号写 `null`，并在文件头说明原因，不得填零。
- 文件头记录平台、逻辑 CPU 数、目标段长、采样开关、阈值取值和各信号的来源接口。
- 每轮测量记录增加负载汇总：段数、观测段 `L_ext` 最大值、`Q` 最大值、受标记段
  列表与 round 判定。
- 采样开关属于运行配置，参与运行身份。正式 latency run 默认开启；开启与关闭两种
  配置的计时不得合并统计。

### 7.4 段标记与 round 判定

观测段满足任一条件即被标记：

| 标记             | 条件（初值）           |
| ---------------- | ---------------------- |
| `external_load`  | `L_ext ≥ 1.0`          |
| `run_queue_wait` | `Q ≥ 0.05`（仅 Linux） |

- 观测期内任一段被标记，该 round 即判为受扰。预热段只记录，不参与判定。
- 判定在 round 结束时依据负载记录完成，结果写入测量记录；矩阵聚合只使用未受扰
  round。
- 阈值初值在 #1388 首个安静测量窗口的采样开销对比中校准：以开启采样的安静 round
  得到各信号的噪声分布。初值落在安静噪声范围内或明显过宽时，在实现 PR 中修订本表，
  经维护者接受后才用于正式测量。正式测量所用阈值写入文件头，事后不改判。

### 7.5 废弃与重跑

- 受扰 round 整轮废弃：保留原目录，在矩阵汇总中登记废弃原因（受标记段及其信号值），
  不删除样本，不改写原始记录。
- 废弃后立即以同一配置、同一轮号在新进程中重跑该 round，不改变其余 round 的轮换
  顺序。每个（行，轮号）最多重跑 2 次。
- 3 次尝试都受扰时停止该行，报告维护者。维护者决定继续的，按受扰 round 如实计入并
  在报告中标注；不得改用更宽的阈值重新判定。
- 废弃的 round 可供归因分析引用，不进入 §8.2 的 p50/p95/p99 与 max。
- 需要识别干扰进程时，只在受扰 round 结束后、下一个 round 开始前采集一次进程快照，
  写入该 round 目录；round 运行中不采集。

### 7.6 实现验证

实现至少验证：

- 采样调用与缺页读取都不在计时区间内；
- 各段首末帧连续覆盖整个 round，无重叠；W1–W3 预热与观测分界处恰好分段；
- 参与判定的段都不短于 1 s：阶段末尾短段并入前一段，W4 末段含 B0/B1 与紧邻预热帧，
  round 过短时记为无法判定；
- 构造的负载序列下各标记与 round 判定符合本节，且判定不读取计时字段；
- 不可用信号记为 `null`；
- 同一输入上开启与关闭采样的计时分布对比，说明采样开销。该对比属于负载敏感计量，
  进入结论须在安静测量窗口内运行。

[issue-1165]: https://github.com/illusion-tech/laneflow/issues/1165
[issue-1273]: https://github.com/illusion-tech/laneflow/issues/1273
[issue-1388]: https://github.com/illusion-tech/laneflow/issues/1388
[os-probe]: os-probe.md
[performance-baseline]: core-runtime-performance-baseline.md
[lf-synth-input-bundle]: lf-synth-input-bundle.md
[lf-synth-execution]: lf-synth-execution.md
