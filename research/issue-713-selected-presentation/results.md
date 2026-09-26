# #713 按需表现完整链路结果

2026-09-26，23 个独立进程均完成完整 performance 窗口，最新严格分析器通过全部原始文件和逐帧配对核验。在相同最终表现集合下，稳定/动态 10% 选择的完整表现链路 p50 分别下降 71.07% / 40.68%。本结果验证 #713 的按需表现链路，不作为 #220 产品性能认证。

## 测量对象与复现边界

源码冻结在 `62170d39ce11bd2f7cd8930b619d0c8d4ee1bd6b`，测量开始前为干净、已推送且可达的提交。后续提交仅补充测试、分析器和报告；已核对生产路径与该提交相同。

本次为 10k `MIXED-PEAK` 的完整 performance 窗口：seed 544、worker 1、cycle 7,656 ticks，暖机 30,624 ticks，观察 61,248 ticks，每进程总计 91,872 ticks。计划、LFCA、配置和各输入文件摘要由逐轮 evidence 及结果 JSON 绑定。不能用本结果代替 100k 测量或 #220 产品性能认证。

宿主为 Windows 开发笔记本，AMD Ryzen 9 9955HX，16 核 / 32 逻辑处理器，内存 66,232,508,416 bytes，Windows Balanced 电源方案保持不变。所有测量进程串行执行，但宿主仍是共享交互环境，没有隔离系统调度、温度或其他应用的影响。

原 FullValidation、匹配稳定选择的全量/Selected、匹配动态选择的全量/Selected，共五种正常构建配置，各运行三个独立进程。第二轮反转次序。四种匹配配置另各运行一次完整 allocator 构建和一次完整内部阶段插桩构建，共 23 个进程。正常墙钟与诊断结果分别报告，不混合样本。

宿主选择基于当前 live 个体的稳定 `IndividualId` 排序，比例 10%，offset 53、reverse true，稳定 stride 0，动态 stride 137。分母为 live 个体，非 presentable；全量匹配对照仍提取全部来源，但最终应用与 Selected 相同的集合。Selected 不在测量路径中额外执行全量 pose oracle。

复现入口与严格分析命令见 [README](README.md)。原始逐 tick 文件保留在本次运行目录，[机器可读结果](results.json) 保留来源、配置、执行 ID、文件大小与 SHA-256；仓库不提交约 20 GB 的逐帧日志。

从固定源码的干净 checkout 重建输入时，使用以下命令；所有输出路径均须尚不存在。`MIXED-PEAK` 的计划展开固定 seed 544，命令不提供另一个 seed 覆盖入口。

```powershell
cargo +1.98.0 build --release --locked -p laneflow-urban-generator -p laneflow-urban-harness --features laneflow-urban-harness/adapter
target/release/laneflow-urban-generator.exe --config examples/config/cn-urban.toml --scale 10k --output target/issue-713-inputs-10k
target/release/laneflow-urban-harness.exe plan target/issue-713-inputs-10k target/issue-713-performance-10k.toml --case MIXED-PEAK --performance
$env:LANEFLOW_HARDWARE_ROLE = '<本机实测硬件和宿主用途>'
$env:LANEFLOW_POWER_ROLE = '<本机实际电源方案>'
pwsh -NoProfile -File research/issue-713-selected-presentation/run.ps1 -Artifacts target/issue-713-inputs-10k -Plan target/issue-713-performance-10k.toml -Output target/issue-713-performance-10k-runs -RemoteBranch '<精确指向当前干净 HEAD 的已推送分支>'
node research/issue-713-selected-presentation/analyze.mjs target/issue-713-performance-10k-runs
```

本次冻结计划 SHA-256 为 `139fc82241a728c50b5c33fb16600584b9ab5a5f3e33dbe1fa5800d4ae350853`，manifest 为 `d9f08f511b50f19eab18a5a87d6f9db5a3836b34f8d82054ab3323ac1583d9dd`，LFCA 为 `c97872ecd20a39e76c9d007ec28cc28dc5b317358f6c5a69a8c9cce8c3722210`。新测量必须记录自己的来源和硬件，不能沿用本次执行编号或把新的耗时写回旧证据。

## 完整证据验证

23 个进程各有 91,872 帧，共 2,113,056 条完整逐 tick 记录。编排器附带分析和 `8d0b29172138f3876baa894aa835a1cd31ac1623` 的最新严格分析器均以退出码 0 返回 `complete-paired-chain-verified`。正式结果 JSON 使用后者，另绑定分析器文件 SHA-256。

验证覆盖独立执行编号、干净源码及可达提交、tree/manifests/lock、三种独立构建与二进制摘要、工具链、硬件/电源、固定 LFCA/计划/选择配置、完整窗口及端点检查点。每个原始帧文件、preview 和 resolved plan 的字节数与 SHA-256 均重新核对；四份 profile 日志另外绑定摘要。

所有 23 组逐 tick 交通、命令及事件都与原 FullValidation 第 1 轮对照。六组正常构建配对和四组诊断配对还逐 tick 核对实际应用数及按稳定身份生成的 Transform 摘要；全部检查点一致。分析同时拒绝缺行、重复 tick、短窗口、错误构建、字段缺失和矛盾数量。结果汇总程序另将全部 23 组从原始帧重新计算的七类表现计时分布与各自 evidence 中的统计逐项核对，全部一致。

## 正常构建墙钟

以下由 15 个正常构建进程汇总，已与最新全批次分析输出交叉核对。各轮均排除暖机，保留 61,248 个观察样本；每个汇总分位数取三轮对应分位数的中位数，不池化样本，也不把阶段分位数相加。

完整表现链路 `presentation_ns` 覆盖宿主选择、封闭提取、Transform 转换、绑定维护、隐藏/恢复及应用；表现验证另外计量。单位 ms：

| 策略 / 轮次 | 匹配全量 p50 / p95 / p99 | Selected p50 / p95 / p99 |
| --- | --- | --- |
| 稳定 / 1 | 1.6066 / 2.2384 / 2.6099 | 0.4598 / 0.6808 / 0.8931 |
| 稳定 / 2 | 1.5500 / 2.0831 / 2.4133 | 0.4388 / 0.6471 / 0.8721 |
| 稳定 / 3 | 1.5893 / 2.1535 / 2.4611 | 0.4688 / 0.7259 / 0.9260 |
| 动态 / 1 | 2.8416 / 3.6385 / 4.0203 | 1.6857 / 2.3637 / 2.7368 |
| 动态 / 2 | 2.8386 / 3.5565 / 3.9100 | 1.6748 / 2.3063 / 2.7071 |
| 动态 / 3 | 2.8636 / 3.6224 / 3.9718 | 1.7141 / 2.3517 / 2.6818 |

| 配置 | 三轮 p50 中位数 ms | 三轮 p95 中位数 ms | 三轮 p99 中位数 ms |
| --- | --- | --- | --- |
| 原 FullValidation | 3.6240 | 4.5531 | 4.9663 |
| 稳定匹配全量 | 1.5893 | 2.1535 | 2.4611 |
| 稳定 Selected | 0.4598 | 0.6808 | 0.8931 |
| 动态匹配全量 | 2.8416 | 3.6224 | 3.9718 |
| 动态 Selected | 1.6857 | 2.3517 | 2.7071 |

相同最终集合下，稳定策略的表现链路 p50/p95/p99 分别下降 71.07% / 68.39% / 63.71%；动态策略分别下降 40.68% / 35.08% / 31.84%。三个独立轮次的三个分位数均改善。原 FullValidation 用于保留原验证负载，其最终应用集合不同，不用它计算选择路径收益。

正常构建各阶段的三轮 p50 中位数如下，单位 μs：

| 阶段 | 稳定全量 | 稳定 Selected | 动态全量 | 动态 Selected |
| --- | --- | --- | --- | --- |
| 宿主选择 | 70.4 | 71.4 | 71.5 | 69.2 |
| 封闭 pose 提取 | 708.5 | 97.3 | 714.0 | 115.7 |
| Transform 转换 | 428.8 | 33.8 | 427.5 | 45.9 |
| 选择映射、绑定及显示集合维护 | 337.4 | 243.8 | 1,556.2 | 1,436.0 |
| Transform 应用 | 7.7 | 7.1 | 9.7 | 9.5 |
| 表现验证（单列） | 1,007.6 | 791.8 | 1,124.6 | 828.3 |

动态 Selected 的绑定及显示集合维护 p50 仍为 1.436 ms，明显高于稳定 Selected 的 0.2438 ms。选择 API 减少采样与转换后，这部分宿主成本仍保留，不能据此声称全链路 O(K)。

整帧 p50 的三轮中位数在稳定策略中为 14.0663 → 12.3563 ms，动态策略为 15.3993 → 13.7031 ms。整帧计时包含命令、Runtime/Bevy step、完整交通 oracle、表现及其验证，但不含逐帧日志序列化与检查点写出。这是该共享宿主完整负载的实测结果，不等于 Runtime 求解本身提速或产品性能认证。

## 数量与生命周期

所有配置在观察窗口内的 live 个体均为 10,000；实际 Active 的 min / p50 / max 为 3,550 / 4,004 / 5,101，presentable 为 3,660 / 4,114 / 5,201。初始 Active 为 7,500，不用它代替观察期实数。下表为 min / p50 / max，完整 p95/p99 见结果 JSON。

| 配置 | host_selected | requested | extracted | applied | N_presented |
| --- | --- | --- | --- | --- | --- |
| 原 FullValidation（本次 10k） | 3660/4114/5201 | 10000/10000/10000 | 3660/4114/5201 | 3660/4114/5201 | 3660/4114/5201 |
| 稳定匹配全量 | 1000/1000/1000 | 10000/10000/10000 | 3660/4114/5201 | 277/314/460 | 3660/4114/5201 |
| 稳定 Selected | 1000/1000/1000 | 1000/1000/1000 | 277/314/460 | 277/314/460 | 277/314/460 |
| 动态匹配全量 | 1000/1000/1000 | 10000/10000/10000 | 3660/4114/5201 | 269/429/586 | 3660/4114/5201 |
| 动态 Selected | 1000/1000/1000 | 1000/1000/1000 | 269/429/586 | 269/429/586 | 269/429/586 |

Selected 的 requested 是实际选择句柄数；全量模式的 requested 记录其覆盖的全部 live 输入域，API 仍直接消费借用来源迭代器，没有创建全量句柄列表。全量匹配对照即使只应用约十分之一，其 N_presented 仍包含所有已提取身份。Selected 的 extracted、applied 和 N_presented 相等，未再次应用 `/10`。

四组已核验 allocator 运行在观察窗口内的宿主生命周期计数如下。相同策略的全量匹配对照与 Selected 完全一致；计数为各帧发生次数之和，重复复用/显示同一实体会重复计数，不能解释为不同实体总数。

| 策略（全量与 Selected 相同） | created | reused | hidden | shown | retired_bindings |
| --- | --- | --- | --- | --- | --- |
| 稳定窗口 | 5 | 20,579,251 | 400 | 225 | 220 |
| 动态窗口 | 24 | 25,596,357 | 3,507,114 | 3,507,011 | 3,162 |

选择缺席仅隐藏表现，身份与已有绑定继续保留。真正 removal/replacement 仍由 typed 生命周期事务处理；`retired_bindings` 只反映宿主已绑定对象的退役，不等于全部交通个体的 removal 总数。完整运行中的交通 births、removals、replacements 各为 4,790，分别由完整交通记录及检查点验证。

## 分配、容量与冷启动

四组完整 allocator 运行均已与对应正常构建第 1 轮逐帧核对。每组暖机后的 61,248 个样本中，`allocations`、`reallocations`、`bytes_allocated`、`bytes_reallocated` 总和均为 0，非零分配帧数也为 0。该结论同时覆盖稳定与动态集合的全量匹配对照和 Selected。

冷启动单列如下。初始化为整个 harness 初始化时间；第一帧耗时和分配仅对应本报告的表现区间，不能将其解释为整个进程的冷启动总分配。

| 独立 allocator 配置 | 初始化 ms | 第一帧表现 ms | alloc / realloc 次数 | 分配 bytes / realloc 增长 bytes |
| --- | --- | --- | --- | --- |
| 稳定全量 | 1,599 | 3.1695 | 68 / 158 | 1,852,600 / 1,440,176 |
| 稳定 Selected | 1,532 | 1.2220 | 78 / 149 | 903,252 / 694,704 |
| 动态全量 | 1,552 | 3.3972 | 68 / 158 | 1,852,600 / 1,440,176 |
| 动态 Selected | 1,525 | 1.4830 | 78 / 149 | 903,252 / 694,704 |

四组第一帧均创建并显示 760 个 Entity。Selected 的冷启动 alloc 次数并非更少，但字节数较小；不能将暖态零新增分配写成无冷启动成本。

末帧宿主存储按 `len/capacity` 列出：

| 宿主所有者 | 稳定全量 | 稳定 Selected | 动态全量 | 动态 Selected |
| --- | --- | --- | --- | --- |
| live_order | 10000/16384 | 10000/16384 | 10000/16384 | 10000/16384 |
| requested | 1000/1024 | 1000/1024 | 1000/1024 | 1000/1024 |
| requested_index | 1000/1792 | 1000/1792 | 1000/1792 | 1000/1792 |
| candidates | 3660/8192 | 284/1024 | 3660/8192 | 410/1024 |
| extracted_index | 3660/14336 | 284/896 | 3660/14336 | 410/896 |
| outputs | 284/1024 | 284/1024 | 410/1024 | 410/1024 |
| bindings | 765/819 | 765/823 | 7624/14146 | 7624/14230 |
| visible | 284/896 | 284/896 | 410/896 | 410/896 |
| previous_visible_scratch | 284/896 | 284/896 | 424/896 | 424/896 |

HashMap 随机化与删除后的容量变化会导致相同逻辑内容的 capacity 略有差异。动态集合在末帧仅显示 410 个实体，但仍保留 7,624 个绑定，直接体现选择缺席与真正 despawn 的区别，也解释了不能将绑定维护成本按当前显示数计算。

定向 Adapter 测试还覆盖 0/1/10/100% 非前缀重排、单/双输出交替、新输出、失败重试、小→大→小与旧尾部保护。测试轨迹中选择查重表容量在大选择后保留为 112，后续小选择与空选择不代表释放该容量；Spatial 和 Adapter 交换缓冲仍分别持有存储。

分配统计范围为选择至应用区间，不是进程整体或峰值 RSS。HashMap 的 capacity 不能直接换算成完整底层字节账；逻辑复制字节为零也不等于没有内存访问。

## 独立内部阶段诊断

稳定选择两组独立 profile 已完成，每组的来源、Spatial、Adapter 日志各有 91,872 条。下表只统计暖机后 61,248 条，单位 μs；这些插桩阶段值用于解释成本，不替代正常构建墙钟，也不把分位数相加。

| 稳定集合内部阶段 | 全量 p50 / p95 / p99 | Selected p50 / p95 / p99 |
| --- | --- | --- |
| 来源前置校验（Selected 含完整查重） | 0.3 / 0.4 / 0.6 | 30.7 / 46.8 / 78.8 |
| 来源查询与候选收集 | 145.8 / 185.7 / 398.8 | 12.2 / 15.8 / 19.5 |
| Spatial 采样 | 585.7 / 831.0 / 1,039.1 | 53.8 / 73.1 / 149.5 |
| Spatial 提交 | 0.1 / 0.1 / 0.2 | 0.1 / 0.1 / 0.1 |
| Adapter 提交 | 0.1 / 0.2 / 0.4 | 0.1 / 0.2 / 0.4 |

选择入口确实付出了查重校验成本，但只查询输入句柄，并只采样实际有来源的结果。两组完整帧流均与各自正常构建第 1 轮的交通、命令、事件、实际应用数和 Transform 摘要一致。

稳定集合末帧的 Adapter/Spatial 存储如下；`len/capacity` 均指当前所有者。容量保留来自此前更大的可表现数量，不是末帧条数。

| 所有者 | 全量 len / capacity | Selected len / capacity | 元素大小 |
| --- | --- | --- | --- |
| Adapter PoseInput | 3,660 / 8,192 | 284 / 1,024 | 16 bytes |
| Adapter 候选句柄缓冲 | 0 / 8,192 | 0 / 1,024 | 8 bytes |
| 对外输出句柄缓冲 | 3,660 / 8,192 | 284 / 1,024 | 8 bytes |
| Spatial scratch records | 0 / 7,600 | 0 / 760 | 40 bytes |
| 对外输出 records | 3,660 / 7,600 | 284 / 760 | 40 bytes |
| Adapter 查重 HashMap | 0 / 0 | 1,000 / 1,792 | 不换算底层字节 |

两个提交阶段记录的 `logical_copy_bytes` 均为 0，原因是成功后交换缓冲；表中的两份缓冲容量仍须分别计入。计时分辨率允许极短阶段出现 0，不代表没有执行工作。

动态策略两组 profile 也已完成完整帧流核验。内部阶段分位数如下，单位 μs：

| 动态集合内部阶段 | 全量 p50 / p95 / p99 | Selected p50 / p95 / p99 |
| --- | --- | --- |
| 来源前置校验（Selected 含完整查重） | 0.3 / 0.4 / 0.6 | 31.9 / 50.7 / 81.8 |
| 来源查询与候选收集 | 145.3 / 187.0 / 404.1 | 12.7 / 15.9 / 19.5 |
| Spatial 采样 | 586.6 / 836.5 / 1,053.2 | 68.6 / 89.7 / 238.4 |
| Spatial 提交 | 0.1 / 0.1 / 0.2 | 0.1 / 0.1 / 0.1 |
| Adapter 提交 | 0.1 / 0.2 / 0.4 | 0.1 / 0.2 / 0.4 |

动态策略末帧的缓冲 capacity、元素大小与上表稳定策略相同；Selected 的 PoseInput、输出句柄与输出 records 的 len 为 410，其余对应 len 不变。四组 profile 都包含三类各 91,872 条日志，各阶段统计均排除 30,624 tick 暖机。

## 合同与验收映射

| 验收要求 | 直接证据 |
| --- | --- |
| 有序完整代际句柄；0/1/10/100%；过滤后顺序与连续 record ID | `session::capacity_tests::selected_percentages_and_alternating_outputs_match_full_pose_oracle`、`selected_reordering_filters_virtual_parking_and_renumbers_records` |
| context → Spatial → 根配对 → 完整查重 → 顺序查询 → Spatial 首错 | `selected_context_spatial_and_pairing_precede_duplicate_validation`、`selected_errors_preserve_old_output_and_duplicates_precede_unknown` |
| Active、显式停车、虚拟停车、Completed；最新已提交状态；槽位复用与 typed replacement | `mixed_lifecycle_selection_reads_latest_state_and_preserves_typed_replacement_identity`、`pose_extraction_commit`、`replace_rebind` |
| 完整旧输出保护；首/中/末 Spatial 失败；重试；空批头更新 | `pose_extraction_commit`、`session::capacity_tests` |
| 不同世界、成功/失败切换、同修订新根 restore | `selected_errors_preserve_old_output_and_duplicates_precede_unknown`、`selected_context_survives_failed_cutover_but_not_successful_cutover_or_restore`；分别断言 foreign context 拒绝、失败切换后继续提取、成功切换/同修订新根后旧 context 拒绝，以及新 context 可用 |
| 选择退出/重入保留身份；placement 拒绝；失败保留旧可见性及重试 | `selection_exit_reentry_and_placement_rejection_preserve_identity` |
| 每句柄规范 pose 和实际 Transform 对照，不误比 record ID | Bevy 选择测试、`selected_and_full_extraction_apply_the_same_dynamic_host_selection` |
| 未选车辆的 Spatial 问题仍使全量失败 | `selected_mixed_frames_fail_atomically_in_each_order_and_full_validation_remains` |
| 单/双输出、新输出、小→大→小、失败重试及暖态零新增分配 | `session::capacity_tests`、`warm_extraction_path_has_no_new_allocations` |
| 原验证负载和完整交通 oracle 保留 | `bevy_and_headless_share_demand_and_committed_results`；本次原 FullValidation 三轮及全部运行逐 tick 交通比较 |
| 三轮正常墙钟与独立诊断，完整窗口、不可变来源、相同最终表现集合 | 23 个完整进程、最新严格全批次分析及结果 JSON，全部通过 |
| 分析器拒绝缺轮、重复执行、来源漂移、短窗口、错误构建、摘要/交通/表现不符 | `analyze.test.mjs` 五项测试，含摘要同步更新后的截短帧流和缺失插桩行 |

## 实现验证

实现阶段本地执行以下测试命令，88 项通过，4 项按已有定义忽略：

```text
cargo +1.98.0 test --locked -p laneflow-bevy -p laneflow-urban-harness --features laneflow-urban-harness/adapter
```

正常构建以及 allocator/profile features 的 Clippy（all targets、`-D warnings`）、严格 rustdoc、格式检查均通过。后续宿主失败可见性断言和分析器拒绝路径已在 `8d0b29172138f3876baa894aa835a1cd31ac1623` 的 CI 通过；最终提交的门禁记录见 [PR #761 Checks](https://github.com/illusion-tech/laneflow/pull/761/checks)。Node 分析器的五项测试包含结构检查、完整文件流、外层摘要同步更新后的截短帧拒绝，以及缺失内部计时行拒绝；合成夹具只测试分析器，性能结论全部来自本次真实完整运行。

另外执行已构建的 Bevy 测试二进制，取得七项容量测试和一项暖态分配测试的原始轨迹；轨迹与摘要随结果 JSON 保存。对应 Adapter 源文件在测量提交与后续测试提交之间无变更。这份补充轨迹明确记录输出/候选所有权交换及大→小后的保留容量，不声称它是一次新构建或进程总内存测量。

## 解释与限制

选择入口减少所选可表现车辆之外的来源查询、Spatial 采样和 Transform 转换，Runtime 仍完成全部交通计算与验证。宿主选择仍遍历并排序 live 个体，绑定维护仍与累计绑定数量及容器容量有关，因此没有证明整个表现链路为 O(K)。动态窗口会持续积累绑定，这部分成本必须随结果一起解释。

可恢复失败的原子性属于既有 `Result` 边界，不扩大为 allocator OOM 或任意 panic 的恢复承诺。原全量 API 继续使用借用来源迭代器，没有先构造全量句柄列表或复用选择查重前处理。

本报告只覆盖上述输入、规模、选择策略、完整窗口和硬件。0/1/100% 的合同正确性由测试覆盖；本次性能测量只覆盖 10%，不推断其他比例、其他硬件或 100k 规模的性能。
