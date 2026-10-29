# LF-SYNTH-v2 表现与测量接线

**文档状态**：Draft（[#1165](https://github.com/illusion-tech/laneflow/issues/1165)）；
G1 待接受，不能据此宣称测量实现或产品认证完成。

**适用范围**：LF-SYNTH-v2 宿主 harness 的固定表现选择、外层帧输入与同次运行计时。
交通输入、步进规则和产品预算仍由
[`core-runtime-performance-baseline.md`](core-runtime-performance-baseline.md)、
[`lf-synth-input-bundle.md`](lf-synth-input-bundle.md) 和
[`lf-synth-execution.md`](lf-synth-execution.md) 约束。

## 1. 需要裁决的接缝

性能基线 §4.3 要求 W4 的 B0、B1 分别完成 Spatial extract 与 Adapter apply，
同时禁止两拍之间插入 outer-frame input。Bevy 合同 §4/§7 则规定每个 outer frame
最多一次表现提取和提交。这两项要求无法直接组合成两个各自完整的 integrated frame。
当前语义 harness 没有 Spatial/Adapter，已有 W4 状态与重试证据不裁决这一接缝。

建议保持生产 Adapter 的每帧一次提交合同，把 W4 明确定义为两个连续的指定外层帧。
这会修订 W4 的**宿主测量输入**，不改变其 topology、state、input-plan、命令次序、
Core step 输入、B0/B1 状态或 observation_ticks。接受本提案时，必须同步替换性能
基线 §4.3 中禁止这两次指定输入的文字，并按 §4 更新统计名称；不得让两文冲突存续。

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
组合事务更新映射，不能绕过 Session 的映射权威。

每帧步进结束后最多一次调用 `extract_selected_committed_pose_batch`，复用缓冲。
宿主对完整批次检查 context、placement token、句柄/Entity 映射与有限 Transform，
全部暂存后统一提交；任一失败保留所有旧 Transform，并使本 round 失败。
这不恢复尚未交付的公共 placement API，不把宿主转换写入 Runtime 或 Spatial。

W2 继续使用 Core-only 路径；不安装 Spatial/Adapter，不产生 integrated frame 数据。

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
observation Core tick 仍进入 Core 统计，不能因其属于边界帧而删除。输入生成后
先冻结完整序列，再开始预热；相同 run 配置必须产生相同序列。

追赶是独立 guardrail 序列，不在运行中临时加延迟以修改正常 latency row。序列
显式覆盖零输入、亚 quantum 输入、零/一/两拍和超过上限的输入；至少一个输入从
零 backlog 开始提供四个 quantum，使两拍后仍保留两个 quantum。之后提供零输入
直至恢复到小于一个 quantum。逐帧报告输入、成功步数、剩余 backlog、上限标记和
恢复帧数，保留全部中间结果与事件；不把该序列混入普通帧 percentile。

## 4. W4 两个指定帧

预热使用 §3 的序列，在 `warm_up_ticks` 精确结束且 backlog 为零。随后只输入：

1. **B0 帧**：delta 为一个固定 quantum。执行 pending replace、Parking leave、
   B0 Core step、有序结果消费、一次 Spatial 提取和一次 Adapter apply。
2. **B1 帧**：紧接 B0 帧成功后输入一个固定 quantum。执行 route-transition
   probe commands、B1 Core step、有序结果消费、一次提取和一次 apply，立即结束。

这两个输入是 W4 声明的唯一 observation 外层输入。两帧之间不插入第三次输入、
额外步进、额外表现提交或其他宿主命令。probe commands 的边界和顺序仍按性能
基线 §4.3；B1 读取 B0 已提交的 green snapshot。任何失败停止本 round。

B0/B1 是固定步长驱动的因果突发帧，不是 60 Hz cadence 样本；不进入普通帧或
追赶帧 percentile。每轮分别保存 Core、Spatial+Adapter、完整帧的 B0/B1 原始值
和 `burst_max`，三轮报告该层 `burst_max` 的 median 与 worst。不从两个样本
推导 p95/p99，也不把两帧相加冒充一次普通外层帧。

已有 D 证据继续证明相同交通输入的状态与失败重试语义；它不成为这些新增表现/
计时路径的证据。E 必须重新核对 B0/B1 的状态、命令游标、批次和事件，并覆盖
表现失败时整批不写、成功替换保持选定逻辑身份与 Entity 连续性。

## 5. 计时与证据边界

正式 latency binary 不启用 allocator instrumentation、failpoint 或 profiling。
每个 integrated 样本用连续墙钟区间覆盖该帧 lifecycle、全部成功 Core steps、
有序结果消费、选择映射、Spatial 提取、Transform staging/apply；不得暂停、扣除
中间区间或把不同进程/不同帧的 component percentile 相加。Core 和表现分段计时
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

G1 待裁决的是 §1/§4 的 W4 两帧方案，以及 §3 的确定性输入与边界帧统计方案。
接受后把本文置为 Accepted，同步改写性能基线的对应条款，再开始 E 的代码实现。
不新增 Runtime 生产 API、不改变 Adapter 每帧一次提交合同，也不改变 LFCA、LFRS
或既有交通输入编码。

实现至少验证：固定选择的测试向量及生命周期继承；16/33 ms 的零/一/两拍、余量
与 backlog 恢复；每拍命令/事件恰好消费一次；W4 恰好两拍两帧；提取到 Transform
的完整批次失败原子性；与语义路径相同的状态/事件；计时分类、原始样本和三轮
聚合的可复核性。完整正式矩阵和资源记录完成前，E 不得标记完成。
