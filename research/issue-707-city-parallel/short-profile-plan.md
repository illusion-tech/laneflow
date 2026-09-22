# #707 源码成本分析与短窗口诊断草案

状态：完整诊断设计；第一轮子集已实施，见[分层短测结果](short-profile-results.md)。2026-09-21。

> 2026-09-21 方向更新：用户明确取消等价优化与严格对拍目标，采用激进优化，
> 以事实和合理交通结果验收。当前执行依据为[激进优化计划](performance-first-next-plan.md)。
> 下文保留首轮诊断的历史设计；其中保持首错、逐拍/worker 等价、失败原子性和
> 旧实现顺序等条件不再作为后续优化门槛，替换为新模型的结果与失败处理验证。

后续执行记录见 `short-profile-results.md`；本文件保留完整诊断设计，
不以第一轮实现覆盖全部列项。2026-09-21 外部评估的采纳边界如下：

- 前缀用于计时校准与早期筛查；另以公共接口生命周期场景验证局部机制。
  后段代表性验证保留，但不建设通用恢复系统作为本轮前提。
- 明确比较冻结原始记录与新记录、插桩关闭与开启、1w 与 4w；不能只比较新版 worker。
- 同 tick 求和后计算分位数；阶段 p95 不相加。最慢 Core 拍另看同拍阶段分布。
- 优化收益必须经过无插桩 release 复测，且超过扰动和重复波动；首轮筛查不宣称收益。
- 16ms 正式预算保持；33ms 仅作讨论中的阶段里程碑。生产服务时间还含命令与发布。
- 异步宿主边界单独设计：命令生效 tick、已提交状态发布、状态年龄/插值、积压与过载。
  表现帧可合并不能推导出生命周期事件、命令或仿真 tick 可以丢弃；本轮不实现异步接入。

源码锚点：测量基线 `4de40e045398e4b010b2aa36522afc02a4094c4d`；研究 checkout
`c961e963d4b1cba4b68e8ef3fbb2d1544efd5ff3`。本次核对两者的
`crates/laneflow-runtime/src` 与 `tools/laneflow-urban-harness/src` 无差异。
下文符号及行号均据该源码核对。Refs #707，不改变现行性能预算或关闭研究任务。

草案阶段只做源码阅读与方案整理；L3 运行期间未并发编译、短测或重型 ETL 分析。
L3 完成后的新增证据见 [L3 离线诊断](l3-findings.md)。冻结测量 worktree、
二进制、计划和既有证据保持不变。

## 1. 本次短测要回答的问题

1. 公共 Core step 的主要绝对成本在哪里？协调器的准备/消费占多少？
2. P2/P3/P5 实际分发什么工作，缓存命中与候选稀疏时是否仍支付大量组织成本？
3. 命令、Harness 检查与记录输出分别占多少，哪些工作不在既有三项计时中？

短测用于选择优化候选，不证明正式观察窗达标，也不替代 L3 的后段画像。
原始 D2 各臂观察窗均为 29696 拍；4w Core 均值 77.38ms、p95 89.35ms，
Active 中位数 45468。已经足以优先诊断，无须先补完 r2/r3。

既有本地证据：

- `E:/projects/laneflow-evidence/issue-707/4de40e04/formal/d2-batch-summary.md`
- `E:/projects/laneflow-evidence/issue-707/4de40e04/diagnostics/d2-offline-decomposition.md`
- `E:/projects/laneflow-evidence/issue-707/4de40e04/diagnostics/wpr/l1-findings.md`

## 2. 源码已经确认的成本与归因限制

| 编号 | 已确认的源码事实 | 待测问题与边界 |
| --- | --- | --- |
| C1 | `world.rs::conflict_state_valid` 每拍遍历车辆槽位；`conflict.rs::authority_owners_valid` 多次遍历 owners 统计并检查权威；Waiting 成员校验也位于生产路径 | 先测检查总成本及对象数量。不能把必要检查直接删除或改成 debug-only；缩减检查须保持首错、失败原子性、恢复/命令后的有效性 |
| C2 | `occupancy.rs::rebuild_occupancy_index` 两次访问 Active 车身区间；`finish_layout` 初始化记录和后缀表；`sort_buckets` 遍历全部路段桶 | Active 投影已存在，不能重复提议“改成只遍历 Active”。优先测区间生成、桶维护、排序/后缀各自成本 |
| C3 | P2 在 live 序上发现输入；P3 先统计 Active 且无旧 reservation 的投影，再扫 live 生成输入；P5 复制 Active 状态；三者都在发现之后判断阈值 | 记录发现访问量和真实计算量；不能把 1024 阈值统一理解为同一种对象，也不能默认提高阈值能解释 45k Active 的退化 |
| C4 | P5 已复用同拍 motion cache，但仍逐车检查 binding、Waiting/Conflict 约束并生成结果；协调器回填 updates | 统计真实 cache hit/miss、约束变化与重算量。不能称为“所有车辆重复计算两遍” |
| C5 | `parking.rs::ParkingRuntimeState::binding` 使用 HashMap，Core 和 Harness 多个遍历调用它 | L1 有 binding/哈希热点，但进程级函数采样无法把它全部归入 Core；各调用域须分开计数，不能把函数权重当独占阶段墙钟 |
| C6 | frontier 清理遍历 cell workspace，重建遍历 live 并对 Active 的相关出现项计算 ETA；已有 horizon 提前终止；P4 包含候选调度及 Waiting 依赖准备 | 分别统计清理长度、live/Active、访问出现项、实际候选与资源声明；不能预先认定 P4 最大，也不能称 frontier 无界扫描完整路线 |
| C7 | `leave_parking` 的 follower 安全校验扫描 Active；成功状态变更可使 occupancy 来源失效；Active Vec 插入/删除移动元素 | `ensure_current_occupancy` 已缓存未改变状态下的索引，重复拒绝不必重建；分别计成功、拒绝、重建次数和移动量，不把每条命令都视为完整重建 |
| C8 | `observe::state/counts/parking_invariants` 多次遍历 individual，状态检查生成并排序车身区间，包含哈希与字符串/JSON 编码 | 这是 Harness 成本；优化应保持完整检查及精确摘要编码，单独报告，不能算作 Core 加速 |
| C9 | `signal_stop_distance` 沿受控门链找限制点；运动内核存在浮点计算 | 仅当门访问量或数值内核占比足够大时提升优先级；SIMD、近似和改变表达式顺序不作为第一候选 |
| C10 | `replace_completed_vehicle` 在入口占用检查前线性定位 live 位置；成功后全量重建 Active 并使位置索引失效 | L3 晚窗该函数占进程采样约 4.29%；分开测定位、入口预检、成功重建。区分调用者未 Completed 延期与公共 API 的 Blocked 重试 |

源代码主要入口（相对仓库根）：

- `crates/laneflow-runtime/src/kernel/world.rs:466`、`conflict.rs:2613`
- `crates/laneflow-runtime/src/kernel/occupancy.rs:658`
- `crates/laneflow-runtime/src/kernel/waiting.rs:1676`、`:1864`
- `crates/laneflow-runtime/src/kernel/conflict_tick.rs:728`、`:1352`、`:1916`
- `crates/laneflow-runtime/src/kernel/tick.rs:2120`、`:2227`
- `crates/laneflow-runtime/src/kernel/parking.rs:648`、`:1379`、`:1488`
- `crates/laneflow-runtime/src/kernel/active_order.rs:100`
- `tools/laneflow-urban-harness/src/runner.rs:1294`、`observe.rs:731`

## 3. 第一层：完整一拍的计时边界

外层记录从命令边界前开始，到本拍语义记录交给 writer 后结束。初始化、检查点
捕获和最后 flush/汇总分别记账。不得将一个窗口的均值与另一窗口总墙钟相减。

| 区间 | 精确边界 | 与旧口径的关系 |
| --- | --- | --- |
| `tick.total` | 外层调用 advance 前至本拍 ticks/commands/events 写入循环结束 | 新增完整驱动区间；不等于 Core step |
| `host.commands` | `advance` 的 schedule.remove/排序/execute 循环 | 包含准备、重试安排、证据处理及实际公共命令 |
| `command.public` | 每次 `measure_command` 闭包调用 | 旧 last_command_ns；嵌套于 host.commands，不能再加到 tick.total |
| `obs.pre.capture` | step_before collect 加 intent 计数 | 旧 observation 的前半部分 |
| `obs.pre.red_waiters` | `observe::red_waiters` | 原计划暖机前直接返回；前缀无法覆盖正式观察期此项负载 |
| `core.public` | `host.rs::Host::step` 中公共 world.step 调用 | 与原 traffic_world_step_ns 边界一致 |
| `obs.post.signal_arrival` | signal_signature、相位证据与停车到达处理 | 单列，含每次 arrival 查找 plan.arrivals 的线性 find 候选 |
| `obs.post.events` | `observe::events` | 含状态更新，不能在同一状态上无条件重复执行 |
| `obs.post.state` | `observe::state` | 状态摘要、车身排序和不变量检查 |
| `obs.post.counts` / `obs.post.parking` | counts / parking_invariants 各自调用 | 拆开旧 observation 尾部 |
| `record.finish` | last_observation_ns 记账之后至 TickRecord 构造完成 | 旧 observation 未覆盖：live 计数、事件/命令 JSON 编码及其 SHA-256 等 |
| `writer.tick_records` | report 外层写 ticks/commands/events | 缓冲写入时间单列；最终 flush 不塞回最后一拍 |

关键纠正：`runner.rs:1368` 停止 observation 计时，而 `:1407–1408` 才生成
event_digest/commands_digest。因此 L1 的 SHA 热点不能全部算进原 observation。
正常正式采样边界保持原样；新增字段只进入独立诊断文件。

## 4. 第二层：Core 的互斥区间与嵌套子项

现有 `performance_profile.rs` 已有 11 个标签，但仅在 cfg(test) 启用；普通 release
Harness 不会产生这些数据。可复用其边界定义，不能直接声称现有二进制支持阶段输出。

| Core 父区间 | 放置位置 | 本轮需拆出的子项 |
| --- | --- | --- |
| `preflight` | step_vehicles 的 delta/状态不变量/时钟序号预检 | conflict_state_valid 总计；其内部 owner 检查先由计数或采样定位 |
| `occupancy` | rebuild_occupancy_index 调用 | count、layout、fill、sort_suffix |
| `waiting_prepare` | prepare_commit 中 prepare_waiting_step 调用 | preamble、discover、slot_prepare、dispatch 或 fused、consume、assembly |
| `conflict_prepare` | stage_vehicle_transitions 中 prepare_conflict_step 调用 | reset、frontier、P3、candidate_tail、P4；五者不重叠 |
| `motion_loop` | P5 路径选择及 prepare_motion_* | discover、slot_prepare、dispatch 或 fused、consume |
| `waiting_finalize` | finalize_waiting_step | 先记总计 |
| `signals` | fill_signal_aspects | 先记总计 |
| `conflict_finalize` | finalize_conflict_step | 先记总计 |
| `waiting_outputs` | finalize_waiting_outputs 及现有尾部清理 | 输出/转移准备；不归入 P4 |
| `commit` | CommittedStateMut::commit | 含资源转移、状态写回、Active retain、信号交换；先记总计 |

`core.public` 包含上表互斥父区间，以及 facade/execution 包装和未覆盖的边界工作。
按同 tick 计算 `residual = core.public - sum(parent_intervals)`；残差单列，不能
强行归到分发。子项嵌套于父项，不再次计入 Core 合计；不相加 p95。
失败拍保留完成到哪里、返回错误和已记录区间，不假定每阶段均被调用。

P3 的边界必须对应实际结构：

1. `prepare_conflict_candidates`：清理、frontier，然后候选求值。
2. `prepare_conflict_candidates_dispatched`：projected 计数、发现、槽位准备、分发、规范消费。
3. 调用方的候选尾部 reserve/sort 独立记录。
4. `acquire_conflict_candidates` 才是 P4，其内部先拆 schedule/dependencies 准备与裁决循环。

P2 既有 PREAMBLE 会累计公共前置与发现部分，不能解释为一个连续区间。
新诊断按上述真实边界分列，同时保留父项以便闭合。P5 分发判断必须包含输入
复制/槽位初始化/回填，不只比较 worker 闭包。

## 5. 最小工作量与线程诊断记录

先开批次墙钟计时；只记录决策所需的工作量。逐元素计数会改变热循环，应放在
独立机制运行或块内局部累加，和计时运行分列，不能逐车加共享原子或 Instant。

| 类别 | 字段 |
| --- | --- |
| 身份 | execution_id、完整源码 SHA、诊断 patch 摘要、二进制/PDB 摘要、计划/输入摘要、worker、工具链/flags/profile、环境、实际区间 |
| 每拍规模 | tick、time_ms、live/Active/Parked/Completed、车辆槽位 len/配置容量、路段桶数、occupation records、非空桶数 |
| P2/P3/P5 | 唯一执行路径枚举 fused/dispatched/fallback，加 fallback 原因；发现访问量、输入量、任务块数、消费量 |
| 真实计算 | P2 有无 preview；P3 无候选/资源/无资源/失败报告数，cells/claims 数；P5 cache hit/miss 与运动重算量 |
| 串行项 | frontier 清零单元、访问冲突出现项、P4 候选/接受/拒绝、最终 updates/转移/事件量 |
| 命令 | 每种命令调用/成功/拒绝数、公共调用总时间、occupancy 重建次数、follower 访问量、Active 移动量 |
| 替换命令 | 调用者延期/Runtime Blocked/成功分别计数；live 定位访问量与耗时、入口预检耗时、成功后 Active 重建访问量与耗时 |
| 内存组织 | 输入/输出/缓存/声明表 len、capacity、增长事件；大小估算是逻辑搬运量，不冒充硬件带宽 |

线程诊断按需增加每块开始/结束偏移、稳定线程编号及工作量，共用进程单调时钟。
记录槽在窗口前预留，块独占写，完整 join 后消费；不能仅开协调线程 TLS 就声称
计入辅助线程。诊断缓冲不足必须显式报失效，不能丢样本后报完整。

`dispatch.wall` 是进入分发至完整 join 返回的墙钟，包含计算，不是调度开销。
块耗时合计是块墙钟之和，不是 CPU time；不能用 dispatch 减最长块当纯调度。
区间重叠只证明任务寿命重叠，Running/Ready 和有效 CPU 并发仍需要 ETW。
P3 的 first_error 固定为 MAX，现有 DispatchStats.extra_work 不表示报告内错误后的工作量。
既有 P2 的 fallback 计数有与 fused/dispatched 重叠的路径，不可简单相加推拍数。

## 6. 只做必要的短窗口矩阵

机器释放后才实施及执行。原正式 100k 计划不变，从原初态执行到完成 tick 512；
完成 tick 1–64 单列为早期段，65–512 为第一筛查区间。这不是正式暖机结束后的
观察窗，不覆盖 red_waiters 后段成本、稳定 100k Active 或所有生命周期命令。
某类命令/候选没有发生时标为未覆盖，不据此判其低成本。

第一批仅 1w 与 4w 各一个新进程，顺序运行。目的是找主要阶段；只在结果含糊或
需要判断候选净收益时补反序短对照，不预铺 1/2/4/8/16 或三轮完整矩阵。
单臂仿真预计分钟量级；初始化、构建及环境不同会改变实际耗时，不承诺固定完成分钟数。

前缀入口可以利用公开 `Harness::install/advance/checkpoint`。输出必须标为
diagnostic-prefix，记录 expected_plan_end=44544 和实际 completed_ticks=512；
不得伪造 performance-round-complete，也不把截断目录交给正式 compare 后放宽校验。

保留原 Harness 调度和全部观测。逐拍 TickRecord 全字段比较，加相同边界的
checkpoint 摘要；命令/事件 payload 与顺序由诊断 writer 完整保存并比较。
这些证明所执行前缀的语义相符，不证明未执行后缀或完整角色验收。
检查点和汇总放在计时区间之外；缓冲写入策略在对照臂保持一致并明确记录。

实现采用独立研究 checkout/源码副本与诊断 patch，源自冻结提交，单独 target。
诊断标记只暴露所需观测，不使用全局 `--cfg test` 激活故障注入/候选实现，也不新增
生产公共 profiler API。普通库启用测试代码不会自动提供一套可用的城市诊断入口。
具体 patch/构建命令与已实现范围见执行记录，本文件不是工具使用说明。

插桩后首先与未插桩的同前缀作语义对照；有性能候选时再用同一前缀的未插桩
release 对照确认收益，避免把插桩成本变化当收益。所有耗时与正式记录分列。
已保存正式 ticks 可用于校验其前缀语义，但不能从已排序 measurements 下标重建
前缀计时或建立 tick/Active 关联。

不为短测加速而跳过原计划命令、强制调整 Active 或删除观测维护的状态。
`Harness::checkpoint()` 只返回摘要，现有文件不是可恢复的完整 Harness 运行点。
后段问题先消费此次 L3 证据，不能假装从早窗即可证明后段根因。

## 7. 优化候选与选择条件

| 候选 | 提升为实施项的证据 | 最小实验与必须保留的边界 |
| --- | --- | --- |
| A：减少并行输入准备/回填 | discover/slot/consume 在对应父阶段占主要成本；真实候选或重算量较小 | 单阶段比较融合与现行分发的完整成本；先动一处重复发现/搬运。保留 live 顺序、身份校验、Active 缓存下标、可选预留回退及首错 |
| B：occupancy 减少重复构造和空桶工作 | count/fill/layout/sort 中出现明确热点，且工作量计数解释成本 | 比较单次区间生成加复用暂存、非空桶维护或安全减少重复初始化；单独核算新 scratch、容量失败、重叠/leader 精确结果 |
| C：停车 binding 访问组织 | Core 内绑定查询密度及采样占比高，非仅 Harness 热点 | 先消除同一安全边界内重复查询；再考虑 generation 校验侧表。不可建立两套可独立修改的权威；命令、恢复、切换、槽位复用需覆盖 |
| D：校验与清理工作集 | preflight/reset/Waiting 校验占比明显，并随槽位/容量而非实际工作增长 | 先研究多次统计遍历合并；增量检查或 touched-cell 清理需另证失效及失败重试。不能先删校验或任意调整错误优先级 |
| E：停车批次命令 | 对应命令确实发生且 rebuild/follower/Vec 搬运占主导 | 先缩小 follower 查询或复用可证明有效的索引；不能批量提前提交/重排命令改变逐条可见状态 |
| F：Harness 观测 | obs/record.finish 或临时容器成为主要实验成本 | 复用缓冲、减少重复映射查找、索引 arrival slot；保持精确编码、全部校验和事件处理；收益不归入 Core |
| G：替换定位与成功后 Active 重建 | L3 已出现热点，仍需分解证明线性定位或重建的绝对成本 | 先评估已有 `LiveOrderIndex` 复用和失效成本，保持 live 顺序、句柄 generation、首错、失败原子性与迁移日志；需覆盖 Completed 且入口被占重试和成功替换 |

优先级由绝对毫秒和可消除工作量决定，上表不是已测收益排序。P4 组件并行、SIMD、
增加 worker 和更改产品预算不作为缺少分解时的默认修复。

候选 A 的最小具体切片是 P3 投影计数快路径：当前先 `inputs.clear()`，再遍历 live
计算 projected，仅用于 `try_reserve(projected)`，随后再次发现输入。若
`inputs.capacity() >= active_order.len()`，已有容量覆盖候选输入的上界，可研究省去
仅用于预留的 projected 计数；不足时保持现有精确计数与预留。后面的规范发现循环、
身份/序号检查、reservation 跳过、cache_index 递增及可选分配失败回退均不改变。

这条快路径依赖现有 Active 投影不变量，并须保留测试注入行为，尚未实现或证明收益。
验证至少覆盖容量充足/不足、Active 增长/收缩、已有 reservation、Completed 留存、
可选输入预留失败、失败重试及 worker 等价。记录“跳过计数拍数/总拍数”和 discover
实际耗时；若容量条件很少成立或计数成本很小，即不提升为优先实现项。

## 8. 首次交付与退出条件

- 逐拍原始阶段记录及同窗汇总：均值、p50/p95/p99/max、计数、残差、路径覆盖。
- 1w/4w 已执行前缀语义核验；插桩构建与未插桩参考身份完整，失败样本保留。
- 给出前两项绝对热点、各自源码机制、一个最小候选和仍未覆盖的后段问题。
- 阶段记账不闭合、缓冲溢出、字段缺失、语义差异或机器竞争负载时停止扩大运行。
- 候选取得实质性短窗收益或接近预算后，再决定是否扩大区间/恢复正式认证；不为了
  重复确认已知超预算而自动追加数小时运行。

本文件只定义分析设计；实际测试、已覆盖范围与剩余问题以执行记录为准。
优化候选和正式预算尚未改动。
