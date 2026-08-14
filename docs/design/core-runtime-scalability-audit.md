# Core Runtime 可扩展性前置审计

**文档状态**: Review<br>
**最后更新**: 2026-08-14<br>
**适用范围**: #199 对 #72 的前置 Core API、道路机动车车辆特化证据，以及目标
Traffic Runtime 的多执行域身份（Identity）、批处理（Batch）、命令（Command）、
确定性调度（Deterministic Scheduling）与事件合并（Event Merge）审计<br>
**关联文档**:

- `../roadmap.md`
- `../adr/0001-project-scope.md`
- `../adr/0003-runtime-tick-and-determinism.md`
- `../adr/0005-core-identity-and-handle-model.md`
- `../adr/0016-scenario-population-and-recycle-lifecycle-authority.md`
- `../adr/0020-compiler-owned-static-network-and-static-image.md`
- `../adr/0021-city-simulation-game-traffic-foundation.md`
- `core-runtime.md`
- `core-runtime-performance-baseline.md`
- `core-id-handles.md`
- `adapter-api.md`
- `bevy-reference-adapter.md`
- `../reference/v0.3-vehicle-following-validation.md`
- `../reference/v0.4-signals-validation.md`
- `../reference/v0.5-parking-validation.md`
- `../reference/v0.6-spatial-validation.md`
- `../reference/v0.7-bevy-validation.md`

## 1. 结论

LaneFlow 现在应冻结一组不依赖具体 partition、线程池或内存布局的可扩展性约束，但不应立即实现城市级生产架构。

Accepted ADR 0021 把“面向中国特色城市模拟游戏的交通基础”定义为第一长期产品
目标；因此“不立即实现城市级生产架构”只描述当前交付时序，不表示
城市级单世界交通执行是非目标。多世界吞吐不能替代一个大型城市世界的扩展证据。

> #215 已在 [`core-runtime-performance-baseline.md`](core-runtime-performance-baseline.md)
> 冻结当前道路机动车的一万/十万产品目标、一百万研究包络、五项计数、workload、
> hardware、tick/frame budget、fidelity、benchmark protocol 与升级触发。#291
> 已接受目标另行定义交通参与单元的六项逐执行域计数；该目标契约不能改写 #215
> Accepted 的 current 契约、工作负载或历史证据。本文继续保存
> #199/#204/#207/#210/#212 的历史研究证据、no-regret constraints 和架构候选；
> 其中的历史阈值、产品未决描述与单机数字不能替代新基线要求的 integrated
> certification。

当前设计没有要求推倒重来。以下基础可以继续作为 v0.8/v0.9 的实现输入：

- Core 使用显式 fixed step，不读取 wall clock 或隐藏引擎状态；
- runtime handle 保持 opaque、typed、无 public ordering，external ID 与 handle 分层；
- lifecycle command 只在 step 之间执行，并保持单命令失败原子性；
- Adapter 不公开 `&mut CoreWorld`，只消费 committed state；
- Spatial pose 使用稳定、调用方拥有的 batch input/output 与 placement token；
- Traffic Data 不持久化 runtime handle、partition、Entity 或 runtime snapshot。
- #291 target 的编译器静态执行约束、可重建分区提示和每世界运行时执行计划必须
  分层，最终 partition/worker assignment 不进入共享 image。

但在未来 Stable Runtime API 的 G1 前，以下五个边界必须完成正式设计或明确迁移策略：

1. `CoreWorld::vehicles()` / `vehicle()` 返回 borrowed `VehicleState` 对内部 AoS/slot representation 的长期约束；
2. world-scoped handle 在物理 partition 迁移、多 Session 或多 World 下的 provenance 与 logical identity；
3. 面向多交通执行域、十万/一百万、可见区域和 fidelity tier 的 selective batch
   snapshot/query，且不能把 `VehicleState` 固化为所有参与单元的公共基类；
4. lifecycle/batch commands 的 canonical order、冲突规则和 whole-batch atomicity；
5. 不依赖 worker completion、partition ID 或容器迭代的 deterministic phase/event merge。

因此，本审计的判断是：

- v0.8/v0.9 可以继续，不由 #72 阻塞；
- 触及上述五个边界的新设计必须显式说明 #72/#199 影响；
- #215 已冻结产品目标和代表性 workload；#220 的 parallel phase/partition G1
  仍等待 #216/#217，并且只在
  [`core-runtime-performance-baseline.md`](core-runtime-performance-baseline.md)
  第 10 节的十万 Core p95 或同步 tick frame 条件触发后启动；完整多层级或
  分布式 Milestone 仍需相应产品硬件、integrated certification evidence 与独立
  G1/ADR；
- 在 Stable Runtime API G1 前，不能只依赖当前 public shape 自动推断未来兼容性。

## 2. 范围与非目标

本审计覆盖：

- `CoreWorld` ownership、step phase、candidate/commit 和 scratch/index 边界；
- Vehicle/Route/Edge handle、external ID、stale semantics 与迁移；
- Core snapshot/query、Spatial batch、Adapter Session 和 lifecycle transaction；
- fixed-step、多频率候选、command boundary 与 deterministic event order；
- 当前道路机动车执行域的一万/十万/历史一百万证据对 API 的约束；
- 后续 prototype 与生产实施的启动触发。

本审计不覆盖：

- 选择 grid、lane cluster、graph cut、R-tree 或其他 partition 算法；
- 实现线程池、work stealing、多 World/shard、分布式协议或 GPU controller；
- 把 partition ID 或物理 slot 编进 public handle；
- 全面迁移 AoS/SoA；
- 承诺十万/一百万实时 SLA、跨平台 bit-level determinism 或城市交通工程精度；
- 把登记机动车总量直接解释为同时活动的交通参与单元数，或把当前车辆证据外推到
  非机动车、行人和轨道交通。

## 3. 证据边界

现有数值全部来自 current `execution_domain=road_motor_vehicle`。它们来自不同
fixture、measurement scope 和版本，不能直接相加、外推成跨平台 SLA，或代表未来
非机动车、行人和轨道执行域。

|   规模 | 当前证据                                                                                                                                                                        | 可以支持的判断                                                                                          | 不能支持的判断                                                                                           |
| -----: | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------- |
|   一万 | v0.3 Vehicle Following 常规 workload 约 `0.630–0.929 ms/tick`，transition-heavy 为 `1.247 ms/tick`；v0.5 100% Reserved 为 `1.263 ms/tick`；v0.7 Adapter batch p95 为 `3.067 ms` | 一万高精度局部仿真和完整 Adapter 路径已有分阶段 Gate/研究证据，可支持优化归因和产品目标定义             | 不能把不同 benchmark 相加后声明完整帧 SLA、P10 Product Pass 或产品认证，也不覆盖 renderer 与所有未来规则 |
|   十万 | v0.3 dense platoon 为 `10.660 ms/tick`；v0.4 mixed Signals 为 `12.651 ms/tick`；Spatial batch p95 为 `5.189 ms`；Adapter batch p95 为 `35.852 ms`                               | 多条 production path 接近线性扩展，未出现已知全局 `O(V²)` 证据；十万足以作为复杂度和 API 压力观测       | 不是 60 Hz 高精度完整运行时承诺，不证明 Core + Adapter + renderer 的组合预算                             |
| 一百万 | v0.2 临时 steady-state 约 `16.39–16.97 ms/tick`、峰值工作集约 `379 MiB`，但不含 occupancy、IIDM、edge transition 或事件                                                         | 单线程逐车微观完整 Core 不能依赖该 optimistic 结果直接扩容；需要单独研究 fidelity、memory 和 scheduling | 不能证明一百万 Vehicle Following、Signals、Parking、Adapter 或城市级实时能力                             |

下表保留 #215 之前用于研究候选比较的 fidelity 问题矩阵。当前产品
target/measurement contract 以
[`core-runtime-performance-baseline.md`](core-runtime-performance-baseline.md) 为事实源；
该基线没有选择 production fidelity architecture，也没有把研究候选升级为产品 tier：

| 候选层级                             | 当前可引用的语义                                                                                                           | 仍待产品/G1 决策                                                               |
| ------------------------------------ | -------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------ |
| 局部精确（Local Exact）              | 当前完整固定步进、跟车、信号、停车与确定性安全不变量；一万具备历史 Gate/研究证据，产品状态仍为 `Product TBD / Uncertified` | 精确上限、目标硬件、固定步进频率、最坏交通密度                                 |
| 降频微观（Reduced-rate Microscopic） | 保留逐参与单元身份/状态，但并非每个控制器在每个基础固定步进都更新；当前原型及证据只覆盖道路机动车                          | 陈旧区间（Stale Interval）内的占用/安全语义、更新频率、插值和事件时间          |
| 中观/聚合（Mesoscopic/Aggregate）    | 只作为 #72 候选，不属于当前 Core 已接受能力                                                                                | 交通参与单元聚合/拆分、守恒、通行/信号语义以及与精确区域（Exact Region）的迁移 |

### 3.1 候选方案 A：个体优先分层扩展（Individual-first Layered Scaling）

**状态**：研究候选；具有明确产品价值，但不是 G1 决策、默认生产架构或城市级能力承诺。

该方案把较强的个体语义视为需要主动评估的产品价值，而不只视为性能成本。其核心
假设是：在可接受的硬件、内存和固定步进预算内，仍存续且保留身份的交通参与单元
应尽量跨分区（Partition）、保真度层级（Fidelity Tier）和生命
周期迁移保持连续的逻辑身份、通行定义/进度、执行域参数、停驻绑定、已提交状态与
事件因果。这样才能支持可解释的参与者行为、稳定的 Adapter 映射、逐单元调试和
后续参与者差异，而不是把所有远处交通默认降为无身份流量。当前 Core 对这些概念的
具体投影仍是车辆、`Route`、`VehicleProfile` 与 Parking，不能据此把目标模型重新
收窄为车辆。

候选形态为：

- 局部精确继续使用完整的逐参与单元固定步进语义，优先通过私有数据导向存储
  （Data-oriented Storage）、批处理、阶段调度和并行计算扩展；ECS、工作线程或
  分区不成为公共 Core API。
- 降频微观仍保留每个参与单元的身份、通行意图和已提交状态，只研究降低部分控制器
  或昂贵派生计算的刷新频率；未刷新固定步进的占用、安全约束、信号/停驻权威、插值
  与事件时间仍必须满足 C7。
- 中观/聚合保留为可选的远域或超大规模方案，不作为默认前提。进入或离开该层级必须
  定义身份保留/恢复、数量守恒、通行/信号语义和确定性迁移边界
  （Deterministic Migration Boundary）；若无法保持逐参与单元连续性，必须把语义
  损失作为产品能力差异显式暴露。
- 共享通行/成本场、聚合占用或其他 SC5-like 技术可以作为内部优化候选，但不得仅为
  吞吐量隐式替换已经承诺的逐参与单元通行、停驻或生命周期权威。

外部参考只用于校准，不构成规范性依赖：Cities: Skylines II 的公开资料展示了 persistent citizen/agent 语义与 ECS、Burst、多核批处理方向，但没有公开足以复制的 partition、multi-rate 或 deterministic merge contract；SimCity 2013 GlassBox 展示了低频 Unit/Map rules、轻量 resource-carrying agents 和共享距离场，但其聚合语义不能自动满足 LaneFlow 的逐车身份与路线需求。

- [Cities: Skylines II Traffic AI](https://www.paradoxinteractive.com/games/cities-skylines-ii/features/traffic-ai)
- [Cities: Skylines II Code Modding](https://www.paradoxinteractive.com/games/cities-skylines-ii/modding/dev-diary-3-code-modding)
- [Inside GlassBox developer talk](https://www.andrewwillmott.com/talks/inside-glassbox)

该方案进入 G1 前，必须与仅精确（Exact-only）和聚合优先（Aggregate-first）候选
使用相同代表性工作负载（Representative Workload）比较，至少记录：可保留的个体
语义、状态迁移复杂度、工作线程数变化下的确定性、CPU/内存成本、Adapter 读取成本
和失败恢复边界。在这些证据形成前，#72 只把个体优先作为一等候选，不预选最终
架构；任何主动丢弃仍存续交通参与单元身份的方案都应说明收益及不可逆语义损失。

### 3.2 候选方案决策矩阵

以下三种方案都是 #72 的研究输入，不是已经接受的架构。它们共享第 5 节的无悔约束
（No-regret Constraints）；差异只在于如何分配个体语义、计算成本和规模上限。

| 维度                   | A. 个体优先分层扩展（Individual-first）                    | B. 仅精确数据导向扩展（Exact-only）                                  | C. 聚合优先资源流扩展（Aggregate-first）                                   |
| ---------------------- | ---------------------------------------------------------- | -------------------------------------------------------------------- | -------------------------------------------------------------------------- |
| 基本形态               | 以精确为主，降频微观保留身份，聚合只作可选远域             | 所有存续个体单元始终执行完整精确语义，只优化布局、批处理、阶段和并行 | 大部分远域交通使用流/包，局部重要区域才展开为精确交通参与单元              |
| 个体语义               | 优先保持逻辑身份、通行/进度、停驻/生命周期和事件连续性     | 最强，所有存续个体单元都保持当前完整语义                             | 只在精确岛（Exact Island）内完整；聚合层可能只有数量、目的地类别和流量守恒 |
| 安全权威               | 基础固定步进仍需定义占用、信号、停驻和陈旧区间安全语义     | 直接复用当前固定步进安全管线                                         | 聚合/精确边界必须另定义容量、队列、冲突和展开后的安全初始条件              |
| 分区/迁移              | 内部所有权可迁移，公共身份不随分区改变；层级迁移需显式事务 | 只需物理所有权迁移，不存在保真度层级迁移                             | 同时需要分区交接与聚合/精确的聚合、拆分、身份翻译                          |
| Runtime/Adapter 连续性 | 可保留一个逻辑 `TrafficWorld` 门面和已提交选择性快照       | 与当前车辆 API 心智最接近，但仍需解除借用式 AoS/全扫描约束           | 最可能要求新的快照能力和明确的层级专用记录                                 |
| 预期收益               | 在保留大部分产品语义的同时降低昂贵控制器和远域计算成本     | 语义最简单、精确参考预言机最强；若硬件目标可满足，迁移成本最低       | 理论规模上限最高，适合活动参与单元远多于可见精确参与单元的目标             |
| 主要风险               | 陈旧区间安全、层级调度、内存占用和保身份迁移复杂           | CPU/内存上限可能不足，并行依赖和边界邻域（Halo）成本仍可能很高       | 语义损失最大，聚合/拆分、通行和信号守恒容易形成新的复杂系统                |
| 当前角色               | 一等研究候选；个体语义有明确产品价值，但尚未被 G1 选中     | 生产精确参考预言机与最低复杂度基线；必须先证明是否已经足够           | 扩展上限对照；只有目标和证据证明保身份路径不足时才提升优先级               |

候选收敛顺序：

1. 先以 B 作为生产路径的单线程精确参考预言机（Exact Reference Oracle），完成
   P1–P3 的存储无关顺序、
   分区边界邻域与选择性快照证据；
2. 在产品允许的保真度差异（Fidelity Delta）明确后，用 P4 和相同工作负载验证 A
   是否能在不破坏 C2、C6、C7、C9 的前提下降低 CPU/Adapter 成本；
3. 只有产品按执行域分解的活动交通参与单元目标、内存或 frame budget 证明 A/B 无法
   满足时，才启动 C 的聚合/精确迁移设计；
4. 最终选择必须在独立 G1 中记录适用场景、语义损失、兼容性、测试判定基准和 ADR
   判断，不由本审计自动决定。

## 4. 当前实现事实与压力点

### 4.1 `CoreWorld` 与 atomic step

当前 `CoreWorld` 同时拥有 lane graph、routes、vehicles、Signals、Parking、stable vehicle update order、candidate state、occupancy、longitudinal 和 command-spatial scratch/index。`step` 的可观察语义是 whole-world atomic：失败不提交 tick/time、vehicle state 或 events。

当前成功 step 的主要阶段为：

1. 计算下一 tick/time 与 Signal candidate snapshot；
2. 重建 occupancy 和 leader observation；
3. 计算 longitudinal motion、约束和 cross-vehicle safety projection；
4. 克隆 compact vehicle candidate state，并按 stable update order 推进车辆；
5. 校验 Parking release 等跨 domain invariant；
6. 生成 Signals 与车辆事件，更新私有派生 index；
7. 一次提交 vehicle/Signal/Parking/tick/time，返回 `StepResult`。

这套 compute-then-commit 是未来 phase graph 的可用基础，但不能把当前一个 Rust function、一个 `CoreWorld` struct 或一个线程解释为稳定 API 承诺。物理 partition 应优先作为 `CoreWorld` facade 后的私有 ownership 实现候选，而不是让 Adapter 或 Data 看见多个内部 shard。

### 4.2 Occupancy 与 longitudinal dependency

当前 occupancy 先按 physical edge 建立连续 scratch，再按 front progress 和 stable update sequence 排序。leader query 可以沿 follower route 查看后续 edge；longitudinal safety projection 又沿 leader graph 反向解析 final travel，并对 cycle 使用 stable update sequence 选择 anchor。

因此，未来并行不能简单地把 vehicles 均分后独立 step。至少需要证明：

- partition halo 覆盖本 tick 的最大 leader/stop horizon；
- 跨边界 leader、route occurrence 和 signal/parking constraint 使用同一 committed snapshot；
- leader graph 跨 partition dependency 与 cycle 仍产生单线程等价结果；
- 边界迁移和 occupancy membership 只在明确 commit phase 生效；
- first error、projection event 和 vehicle advance order 不受 worker 数量影响。

这些属于 prototype 输入，不要求当前 production Core 先抽象 `Partition` trait。

### 4.3 Handle 与 logical identity

当前 Vehicle/Route handle 使用 generation-ready opaque representation，Edge/Signals/Parking 静态 definition 使用 opaque dense handle。public API 不提供 `Ord`、index、generation 或 partition，Data 只使用 external ID。

这是正确的可扩展性方向，但 current contract 仍是 world/session scoped：generation 解决同一 world slot reuse，不解决 wrong-world handle。未来设计必须区分：

- **logical identity**：调用方、replay、日志或 population slot 识别的对象；
- **runtime handle**：同一 simulation session 内的 typed token；
- **physical ownership**：某 tick 中由哪个 partition/worker 保存或计算；
- **external ID**：Data/debug/业务边界中的稳定字符串。

No-regret 边界是：物理 partition 迁移不得要求调用方解码或重写 public handle。首选研究方向是在一个 logical `CoreWorld`/session facade 内保持 live handle，内部迁移 ownership；若未来选择多 World/shard，必须先通过新 ADR 定义 session provenance、translation 和 stale semantics。

### 4.4 Snapshot、query 与 Adapter batch

Spatial/Adapter 路径已经具备几个重要性质：

- `PoseInputRecord` 与 canonical pose batch 不让 Spatial 依赖 `CoreWorld`；
- input/output order 由调用方稳定提供；
- output/scratch 可以预留并复用；
- Adapter 只从 committed Core state 构造 batch；
- placement token 和两阶段 Transform commit 防止旧 frame/部分写入。

当前 Bevy specialization 仍从 `CoreWorld::vehicles()` 全量遍历 borrowed `&VehicleState`，并在单活动 Session/单 canonical frame 下重建 pose inputs。该形态对一万/十万 Reference Adapter 可接受，但不能作为未来稳定 API 的唯一读取方式：

- borrowed `&VehicleState` 使 authoritative AoS/slot state 容易被调用方假定；
- 全量 scan 不能表达 view region、fidelity tier、dirty set 或 partition-local snapshot；
- 直接返回 reference 难以从 SoA、compressed tier 或远端 snapshot 统一供给；
- 单 Session/单 frame 是 Bevy v0.7 specialization 的明确边界，不是 Core partition 方案。

Stable Runtime API G1 前必须选择并验证至少一种 batch/selective read contract，例如 caller-owned output buffer、stable cursor 或 immutable snapshot view；本审计不冻结具体 Rust 类型。

### 4.5 Commands 与 lifecycle transaction

当前 spawn/despawn/register/remove/Parking commands 在 step 之间执行，单命令失败原子；v0.8 又要求 Core replace 与 Adapter rebind 形成 typed transaction。这些边界避免了 hidden side effect，可以继续扩展。

未来 batch commands 或 partition-local command queue 仍必须明确：

- canonical application order；
- duplicate/conflict 规则；
- whole-batch atomic、逐命令原子或显式 partial result；
- command validation 读取哪个 committed snapshot；
- command 对下一 fixed phase、event order 和 partition migration 的生效点；
- Adapter transaction 失败时 Core/mapping 的共同恢复边界。

不得把 worker-local queue completion order当作 public command order，也不得让 Adapter 直接选择内部 partition。

### 4.6 Events 与稳定顺序

当前 `StepResult` 拥有 `Vec<CoreEvent>`。vehicle events 主要按 global stable vehicle update order append，Signal events按 normalization order 在后续阶段 append。event payload 只有 tick 与 domain handle/fields，没有显式 canonical merge key。

当前单线程 append order 是可重复的，但还不是可并行归并的完整语义。并行前必须为每类事件冻结等价于以下元组的稳定顺序：

```text
(tick_index, phase_rank, primary_stable_sequence, local_sequence, secondary_sequence, domain_tiebreaker)
```

最低规则：

- `phase_rank` 来源于正式 step phase，而不是 worker 完成时间；
- vehicle event 的 primary key 来源于 logical stable update sequence，不使用 raw handle、slot 或 partition ID；
- 同一 vehicle 的多次 route transition 使用实际发生顺序作为 local sequence；
- follower/leader 事件以被约束或被更新 vehicle 为 primary，另一对象只作稳定 secondary key；
- Signal/Controller/Group 等静态 domain 使用 normalization order；
- first error 与 event merge 不依赖 `HashMap`、ECS query、worker count 或 work stealing；
- public API 是否暴露 sequence 字段另行决定，内部必须能证明同 worker=1 等价。

### 4.7 Memory layout

当前 vehicle authority 是 `Vec<VehicleSlot>` 中的 `VehicleState`，candidate scratch 每 tick 克隆 compact state；occupancy/longitudinal/command spatial 使用按 vehicle slot 或 edge 预留的独立 scratch。由于这些容器和 handle 字段保持 crate-private，AoS、SoA、chunked storage 或 partition-local scratch 可以先作为私有 prototype。

真正会冻结 layout 的不是当前私有 `Vec`，而是长期承诺 borrowed `&VehicleState`、全量 iterator 顺序或 raw slot identity。因此：

- 当前不做全面 SoA 重构；
- 先用 profiler、retained-memory 和 representative workload 证明 candidate clone/scratch 是主要成本；
- 在 Stable API 前决定 borrowed state 是长期 authority view，还是迁移为 value/snapshot/accessor；
- public API 不新增 slice/contiguous-address/slot-index 保证。

## 5. No-regret constraints

以下约束用于后续 G1 设计审阅；它们不选择具体生产扩展架构。

### C1. `CoreWorld` 是 logical facade，不是物理单容器承诺

调用方可以继续拥有一个 `CoreWorld`/Session 入口，但不得假定全部 state 位于一个 `Vec`、线程、地址空间或 canonical frame。

### C2. Handle 保持 opaque、typed、无 public ordering

不得公开 slot、generation 或 partition；不得把 raw handle bits 作为 stable sort、持久化或网络 identity。物理 partition migration 不应使 live logical vehicle 的 public identity 随所有权移动而改变。

### C3. Stable order 与 storage order 分离

vehicle update、command、snapshot 和 event order 必须由显式 logical sequence 或 normalization order定义；不得依赖容器、partition 或 worker 遍历顺序。

### C4. Stable API 必须有 batch/selective read path

单记录 debug query 可以保留，但一万/十万/一百万默认路径必须支持预留、复用和选择范围，且不能要求每车 resolver/string lookup。

### C5. Commands 只在明确 lifecycle boundary 生效

command validation、application order、冲突和 atomicity 必须文档化。worker-local 或 Adapter-local queue 不能形成隐藏权威。

### C6. Event merge 必须与 worker/partition 数无关

同一版本、环境、初始状态和 input sequence 在允许的 worker/partition 配置下，必须得到文档承诺范围内相同 committed state 和 event order；扩大为跨 CPU bit-level determinism 仍需独立 ADR。

### C7. Multi-rate 不得绕过 safety authority

低频 vehicle/controller 在未更新 tick 的 committed state、occupancy contribution、signal/parking constraint、interpolation 与 event time 必须显式定义。Presentation LOD 不能隐式改变 Core fidelity。

### C8. Data 不持久化 runtime layout

Traffic/Spatial/Manifest 继续保存外部稳定事实，不保存 runtime handle、partition、worker、Entity、scratch 或 transient snapshot。若未来需要 save/resume，应建立独立 runtime snapshot/version contract。

### C9. Adapter 只消费 committed snapshot/transaction

Adapter 不读取 candidate state，不把宿主 Transform 反写 Core，不选择内部 partition，也不通过 raw `&mut CoreWorld` 拼装非原子跨层操作。

### C10. 私有 prototype 先于 production abstraction

partition、SoA、event merge 或 multi-rate 的实验优先放在 research/benchmark 边界，以单线程 production oracle 做状态、事件、不变量和性能对照；证据不足时不得把通用 trait/调度框架提前带入 Core API。

### C11. 静态执行约束与每世界执行计划分离

编译器可以保存 worker 数无关的资源依赖、可切分边界和规范合并键，也可以提供
可丢弃的性能提示；实际 partition/worker/边界缓冲、迁移和动态负载状态属于每世界
运行时执行计划，不得进入稳定标识、共享镜像或 public handle。

### C12. 精确分区不得增加逻辑延迟

所有 partition 在 tick 内读取同一 committed state `T`，经 proposal/claim 和
连接资源组件的唯一规范归约权威后原子提交 `T + Δ`。partition cut 不得增加一 tick
halo delay；互不相交资源组件可以并行归约，不能把“single reducer”误写成永久全局
单线程 reducer。

唯一规范归约权威约束结果与规范顺序，不要求一个连接组件由单一物理线程串行折叠。
生产方案必须比较资源分段、强连通分量（Strongly Connected Component，SCC）、凝聚
有向无环图（Condensation Directed Acyclic Graph，Condensation DAG）波次、稳定局部
归约和固定合并树，并报告组件/SCC 分布、最大 SCC 提案占比、归约工作量/跨度与临界
路径；集中式组件合并只保留为精确参考预言机（Exact Reference Oracle）。信号协调、
冲突区链、排队溢流和长前车链若形成巨型组件，必须在单大型世界 Gate 中显式暴露。
#220 拥有生产分区/归约设计，#72 保留研究证据根。逻辑提案/声明/归约/提交不强制单
工作线程物化队列或屏障；融合单工作线程执行器（Fused Single-worker Executor）允许
消除脚手架，但必须与参考路径的状态、事件、错误和规范顺序等价。

### C13. 路网修订、快照和执行布局各自版本化

共享静态路网按不可变修订发布；运行时世界通过失败关闭的镜像切换事务迁移。运行时
快照绑定 canonical artifact、origin image、revision、runtime、constraint、world、
tick 与全部每世界可变交通状态，但不把 worker/partition assignment 当作跨硬件恢复
权威。人口、出行、Routing 与游戏规则的 caller-owned seed/随机流由上层 Save
Manifest 绑定；只有后续 G1 显式授予 Traffic Runtime 的自有随机流才进入运行时
快照。dense ordinal 不得跨修订直接复用。

默认在线切换在旧世界继续固定步进时从基准已提交状态准备候选，并以有界迁移增量
日志记录已提交动态状态/生命周期变化及命令/事件游标。候选只重解释该已提交变更流，
不得重新执行输入、独立推进目标时间线或产生第二份已提交事件；迁移要求的切换事件
只形成未提交候选批次。提交边界短暂静默、排空日志尾，并把新绑定与规范排序事件
批次原子地只发布一次。日志溢出、无法追上或迁移失败时放弃候选且零发布。维护暂停
模式必须由宿主显式选择，不能把整个准备期停顿隐藏在“固定步进外完成”中。

## 6. Prototype 顺序与通过条件

### P1. Deterministic phase/event merge

目标：在不并行修改 production step 的情况下，把 current events映射到 canonical phase/key，并证明 1、2、N 个模拟 worker bucket 归并后与 current single-thread oracle 完全一致。

至少覆盖：route transition-heavy、projection-heavy、Signals phase change、Parking arrival/release、simultaneous completion 和失败 first-error。

#### P1 #204 研究结果（2026-07-22）

**状态**：测试专用原型通过；结论是继续把 canonical phase/key 作为 production scheduler 的候选输入，但不接受为 production architecture 或 public event contract。

原型位于 `crates/laneflow-core/src/world/event_merge_research_tests.rs`，仅由 `world` 下的 `#[cfg(test)]` 私有模块编译。production `CoreWorld::step`、`CoreEvent`、handle、Core/Data/Adapter API、data format、runtime dependency 和 runtime allocation 均未改变。原型采用以下六元组：

```text
(tick_index, phase_rank, primary_stable_sequence, local_sequence, secondary_sequence, domain_tiebreaker)
```

当前 producer/key 映射如下；这里的数字只属于研究 harness，不是稳定 ABI：

| Producer / event                                 | `phase_rank`     | `primary_stable_sequence`                | `local_sequence` / `secondary_sequence`                              |
| ------------------------------------------------ | ---------------- | ---------------------------------------- | -------------------------------------------------------------------- |
| SpeedLimit / SignalStop / ParkingStop projection | `VehicleAdvance` | logical stable vehicle sequence          | projection kind `0` + route occurrence；secondary `0`                |
| Following safety projection                      | `VehicleAdvance` | follower stable vehicle sequence         | kind `1`；leader stable sequence 只作 secondary                      |
| `VehicleChangedEdge`                             | `VehicleAdvance` | vehicle stable sequence                  | kind `2` + `from_route_edge_index`；to occurrence 作 secondary       |
| `ParkingReservationReleased`                     | `VehicleAdvance` | vehicle stable sequence                  | kind `3`；secondary `0`                                              |
| `VehicleParkingArrivalReached`                   | `VehicleAdvance` | vehicle stable sequence                  | kind `4` + selected route occurrence；secondary `0`                  |
| `VehicleCompletedRoute`                          | `VehicleAdvance` | vehicle stable sequence                  | kind `5` + terminal route occurrence；secondary `0`                  |
| `SignalPhaseChanged`                             | `SignalCommit`   | controller normalization sequence        | controller-local kind `0`；secondary `0`                             |
| `SignalGroupAspectChanged`                       | `SignalCommit`   | owning controller normalization sequence | kind `1` + group normalization sequence；group sequence 作 secondary |

`VehicleAdvance < SignalCommit` 是本原型唯一 phase order。stable vehicle sequence 从现有 logical update order 派生，Signal sequence 从 controller/group normalization order 派生；raw handle bits、slot、partition、bucket 和 worker completion order 均不进入 key。对 current 10 个 `CoreEvent` variant 的 match 是 crate 内 exhaustive mapping，未来新增 variant 会使该研究模块编译失败并要求重新审阅。

自动证据覆盖 7 个场景：多车辆、多 edge transition 与 simultaneous completion；SpeedLimit 与 Following projection；同 tick SignalStop projection 后的两个 controller / 三个 group commit；Parking projection/arrival；Parking release/completion；以及失败原子性和 deterministic first-error。每个成功场景都把事件分配到 `1/2/4/7` 个模拟 bucket，使用三种 assignment seed，反转 bucket 内与 bucket completion 顺序后再归并；结果与 single-thread `StepResult.events` 逐项完全相同。错误场景使用两个真实注入的 vehicle-advance `CoreError` 候选，按相同 logical phase/primary/local key 在相同 bucket/permutation 矩阵中始终选择 single-thread 最早错误；失败 world 不提交 candidate state、tick/time 或 events，清除注入后 retry 与 fresh replay 的 world 和 `StepResult` 完全相同。

研究结论与成本边界：

- current event order 可以脱离 physical storage/worker completion canonicalize，没有发现必须公开 slot、handle bits、partition 或修改 public API 的隐式依赖；因此该路径可以继续作为 #72 后续 scheduler/partition 研究输入。
- 当前 harness 为每个事件附加私有 key、建立模拟 bucket 并执行 `O(E log E)` sort；它只证明语义可表达和 exact equivalence，不是 production 性能或内存方案，也不应把测试分配和 sort 直接复制进 runtime。
- 原型没有并行计算 occupancy、longitudinal、vehicle candidate state 或 Signal state，不证明 worker speedup、partition halo、跨 CPU bit-level determinism、production buffer reuse 或十万/一百万 SLA。
- 若未来 production phase graph 接受该方向，应独立 Issue/G1 比较 k-way merge、预排序 worker buffer、caller-owned scratch 与无额外分配方案，并重新判断 ADR；在此之前不新增 ADR。

### P2. Partitioned occupancy/leader halo

目标：以 physical edge/lane cluster 的研究分区复现当前 occupancy、leader graph 和 projection 结果，测量 halo 数量、跨区 dependency、boundary migration 和 retained memory。

至少覆盖：长 horizon、多 edge route、环路 leader cycle、拥堵边界、Signals/Parking stop 与同 tick edge transition。

#### P2 #207 研究结果（2026-07-23）

**状态**：测试专用原型通过。结论是 route/horizon-driven read-only halo 与跨分区 logical dependency component 可以在当前强个体语义下精确复现 production oracle；该结果支持继续 P3 selective snapshot/batch，并保留 individual-first / exact-only 为一等候选，但不接受为 production partition、集中式 scheduler 或固定 halo architecture。

原型位于 `crates/laneflow-core/src/world/partitioned_occupancy_research_tests.rs`，仅由 `world` 下的 `#[cfg(test)]` 私有模块编译。`occupancy.rs` 与 `longitudinal.rs` 只增加 test-only research helper；production `CoreWorld::step`、occupancy/leader/projection 算法、handle、Core/Data/Adapter API、data format、runtime dependency 和发布构建均未改变。

原型冻结并验证了以下研究语义：

- fixture 提供显式、不可变、穷尽的 `EdgeHandle -> TestPartitionId` ownership；车辆 pre-step owner 只由当前 physical edge 决定，partition/slot/completion order 不进入 identity 或 stable key。
- 每个 owner 从 committed occupancy 出发，沿 owned follower 的 selected route occurrence 与 current dynamic leader/front horizon 收集 remote edge slice；halo 只读且按 vehicle/partition 去重。`max_vehicle_length` 作为 whole-world 只读 horizon 元数据共享，避免某 partition 恰好没有最长车辆时错误缩短搜索。
- owner view 继续按 `(front_progress, update_sequence)` 排序；不同 partition traversal、edge insertion 和 completion permutation 下，per-edge occupant、leader identity 与 bumper-gap float bits 均与 single-thread oracle 精确相同。
- one-leader-per-vehicle graph 先重建 weakly connected logical component。完全本地 component 可独立 projection；任何跨区 chain/cycle 保持在同一 component，再沿 current stable update order、leader final travel 和 deterministic cycle anchor 求解。component completion order 可以改变，component 内语义不能拆开。
- boundary migration 只比较 pre-step owner 与最终 committed edge owner；同 tick 多 edge transition 不创建中间 authority state，也不改写 live handle。

自动证据覆盖 corridor/branch/shared edge、长 horizon、多 edge halo、拥堵、同 tick 多 edge transition、route completion、repeated edge leader cycle、SignalStop、ParkingStop/arrival、两个真实 non-finite leader error 候选、late failure、retry/replay。成功场景在 `1/2/4/7` partitions、三种 connected/alternating/stable-hash assignment 与三种 traversal/completion permutation 下形成 144 组 observation；错误场景形成另外 36 组 first-error 对照。逐组比较：

- occupancy record 与 leader observation（包括 float bits）；
- projection motion 全字段及 13 个相关 float bit pattern；
- ordered `StepResult` events、committed world、tick/time 与 boundary migration；
- canonical first error、失败不提交，以及清除 test injection 后 retry 与 fresh replay。

代表性研究指标如下。数值只描述小型语义 fixture 的 cost shape，不是性能 benchmark：

| 场景 / assignment                   | owned vehicles | remote slices | halo unique / copies | cross dependencies | logical components                  | migrations | partition scratch / oracle |
| ----------------------------------- | -------------- | ------------- | -------------------- | ------------------ | ----------------------------------- | ---------- | -------------------------- |
| corridor, 1 partition               | `7–7`          | `0`           | `0 / 0`              | `0`                | `4 / 0 cross`                       | `0`        | `640 / 640 B`              |
| corridor, 4 partitions, clustered   | `1–3`          | `5`           | `4 / 7`              | `3`                | `4 / 2 cross`                       | `2`        | `2240 / 640 B`             |
| corridor, 7 partitions, alternating | `0–2`          | `5`           | `4 / 7`              | `3`                | `4 / 2 cross`                       | `3`        | `3616 / 640 B`             |
| 2-vehicle cycle, 2 partitions       | `1–1`          | `2`           | `2 / 2`              | `2`                | `1 / 1 cross`, `1 cycle`, depth `2` | `2`        | `640 / 320 B`              |

这些数据给出三个直接结论：

1. 当前 occupancy/leader/projection 语义没有发现必须公开 slot、partition 或修改 handle 的隐式依赖，强个体 identity、route/progress、Parking/event continuity 可以保留。
2. 按需 halo 避免无条件复制 whole world，但同一 remote vehicle 仍可能被多个 partition 复制；corridor 4-partition fixture 为 `4 unique / 7 copies`。未来 production 候选必须测量 representative density、route overlap 与 full-world degeneration，不能把本 fixture 外推为固定上界。
3. tiny fixture 中每 partition 的空 vector/capacity 已使 retained scratch 从 oracle 的 `640 B` 增至最多 `3616 B`。这不是拒绝 partition 的性能结论，但说明 production 研究必须比较 pooled/caller-owned scratch、稀疏 active partition 和 buffer reuse，不能按 partition 永久复制 current full scratch shape。

研究未运行真实线程、scheduler、work stealing、large-scale benchmark、P3 snapshot、P4 multi-rate 或 aggregate/exact migration，也没有证明 speedup、十万/一百万 SLA、跨 CPU bit-level determinism 或 production buffer layout。集中式 component merge 只作为 exact reference oracle；若进入 production，应独立 G1 比较 SCC/component ownership、预排序/k-way merge、错误归并和原子 commit protocol。

因此 P2 建议是：继续 P3 selective snapshot/batch，以同一 strong-individual oracle 验证 partition-local/caller-owned read path；在产品 workload 与硬件目标形成前，不创建 production Partition API/trait，不新增 ADR。只有选择 production ownership、scheduler、identity provenance 或跨 World/shard contract 时，才重新进入独立 G1/ADR 判断。

### P3. Selective snapshot/batch

目标：比较 current full `vehicles()` scan 与 caller-owned filtered/dirty/cursor prototype，在一万/十万下验证稳定顺序、零分配和 Adapter 等价 Transform；一百万只在有代表性数据布局后运行。

#210 已在 `cfg(test)` 私有模块中完成该研究，不修改 production `CoreWorld`、Spatial 或 Bevy API。研究以 `vehicle_update_order` 驱动的 current `vehicles()` 为唯一 Core 顺序 oracle，并把现有 Spatial canonical pose batch 与 Bevy local Transform 原子提交作为 Adapter oracle。研究 record、selection bitmap、dirty index、epoch 和 cursor 都没有进入 public type、Data format 或持久化边界。

读取与提交边界映射如下：

| 边界               | 当前 production authority                                                               | P3 private harness                                                                                                          |
| ------------------ | --------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------- |
| Core vehicle read  | `vehicle_update_order` 中的 live `VehicleState`，`vehicles()` 返回 borrowed stable scan | value record 复制 handle/profile/status/route occurrence/edge/progress/speed/acceleration/Parking binding 与相关 float bits |
| Parking            | `ParkingSnapshot` 只读 committed binding；Parked 必须为 Occupied                        | record 同时保存 Parking view 与 lane/Parking/none pose source                                                               |
| filtered selection | 无 production selective API                                                             | caller-owned membership 只决定是否输出，结果始终保持 global logical stable order                                            |
| dirty              | 无 production dirty snapshot                                                            | ordered `remove/upsert` delta + caller-owned retained cache；delta 本身不冒充完整 snapshot                                  |
| cursor             | 无 production cursor/version retention                                                  | 只在一个 private committed epoch 内分页；任一成功 mutation 后旧 cursor stale                                                |
| Spatial            | caller-owned input、committed output 与 scratch，成功后交换                             | selected inputs 直接复用 `extract_pose_batch`，逐字段和 `f32` bits 对照 full oracle                                         |
| Bevy               | mapping validation、Transform staging、全部通过后写入                                   | candidate pose 在 validation 成功后才替换 committed selected frame；失败恢复旧 cache/pose，既有 Transform 不变              |

三个 Core candidate 和 selected Adapter path 的结果为：

| Candidate                   | Exact / order                                                                                                                 | Warm allocation                              | 主要成本与限制                                                                                                                |
| --------------------------- | ----------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------- |
| stable filtered scan        | 与 full oracle 精确一致；selection 构造/遍历顺序不影响输出                                                                    | 一万/十万均为 `0`                            | 无论选择率都扫描 `V`；它是语义基线，不解决全量读取成本                                                                        |
| committed dirty delta       | no-change、selection churn、Parking、spawn/despawn、edge transition 与 atomic replace 均可重建 exact cache                    | 一万/十万均为 `0`                            | 需要 retained cache、slot-generation-aware index、remove/tombstone 和 selection-change delta；高水位内存不能忽略              |
| single-epoch cursor/page    | `1/64/1024/K/all` 拼接与 full oracle 一致，mutation 后稳定拒绝 stale cursor                                                   | 一万/十万均为 `0`                            | current borrowed iterator 不提供 seek；page 越多越会重复 traversal。跨 epoch resume 仍需要正式 snapshot retention，本轮不承担 |
| selected Spatial/Bevy frame | selected `PoseInputRecord`、canonical pose bits、mapped/unbound/applied counts 与 local Transform 均等于 filtered full oracle | 一万/十万 materialize/extract/apply 均为 `0` | downstream extract/apply 近似随 `K` 增长，但从 Core 生成 selection 仍扫描 `V`                                                 |

语义矩阵覆盖：

- Active、Stopped、Parked、Completed，lane/Parking/none 三类 pose source；
- edge transition、Parking leave/release 与 arrival/commit、spawn、despawn、selection enter/leave、Completed atomic replace；
- atomic replace 的 old remove + new upsert 保持同一 logical position；
- 同 epoch 重读、不同 page size、selection handle forward/reverse construction，以及 mutation 后 stale cursor；
- injected replace failure 保持 committed world，清除 injection 后 retry 与 fresh replay 相同；
- malformed dirty remove、Spatial invalid progress 与 stale mapped Entity 均保持旧 retained cache、canonical pose batch 和已预验证 Transform；失败帧 `applied=0`；
- 一万/十万的 contiguous、alternating、stable-hash selection，比例为 `0% / 0.1% / 1% / 10% / 50% / 100%`；scale selected canonical pose 与 local Transform 逐条对照 full oracle。

2026-07-23 在 AMD Ryzen 9 9955HX、Windows、Rust 1.96 `--release` 的单次 observation 如下。它只描述本机 fixture 的 cost shape，不是稳定 benchmark、SLA 或 speedup Gate：

| Path                                 | 一万 observation |   十万 observation |
| ------------------------------------ | ---------------: | -----------------: |
| filtered，`0%`                       | `0.024–0.041 ms` |   `0.321–0.384 ms` |
| filtered，`100%`                     | `0.249–0.365 ms` |   `2.520–3.631 ms` |
| cursor/page `1024`，`100%`           | `0.415–0.595 ms` | `16.088–17.341 ms` |
| Adapter materialize，`100%`          | `0.124–0.376 ms` |   `1.431–1.998 ms` |
| Spatial extract + Bevy apply，`0.1%` | `0.004–0.008 ms` |   `0.037–0.075 ms` |
| Spatial extract + Bevy apply，`10%`  | `0.242–0.485 ms` |   `3.346–3.842 ms` |
| Spatial extract + Bevy apply，`100%` | `2.054–2.584 ms` | `25.231–26.950 ms` |

容量高水位按测试 type 的 `size_of` 计量，不含 allocator 元数据，也不表示 production 必须同时保留所有 buffer：

| Caller-owned research buffer                                        |       一万 |        十万 |
| ------------------------------------------------------------------- | ---------: | ----------: |
| selection bitmap                                                    | `10,000 B` | `100,000 B` |
| value-record output 或 retained cache，各自                         |  `1.20 MB` |  `12.00 MB` |
| dirty operations                                                    |  `2.40 MB` |  `24.00 MB` |
| previous/current slot-generation dirty index                        |  `0.48 MB` |   `4.80 MB` |
| selected Adapter committed/candidate input + candidate/scratch pose |  `1.36 MB` |  `13.60 MB` |
| cursor token                                                        |     `16 B` |      `16 B` |

结果支持以下判断：

1. 强个体 vehicle identity、status、route occurrence、Parking source、canonical pose 和 local Transform 可以在 caller-owned selective read 中精确保留；没有发现必须暴露 slot、partition、ECS Entity 或改变 opaque handle 的语义障碍。
2. 只增加 filtered batch 不会消除 `O(V)` Core scan。它能减少后续 Spatial/Transform 的 `K` 成本，但不应被宣传为解决十万/一百万 read scaling。
3. current borrowed iterator 上的 cursor 只提供 bounded page output，不提供 bounded traversal；十万/full/page-1024 observation 明显慢于一次 filtered scan。因此本候选不应进入 production G1，除非未来已有 storage-independent seek/index 或正式 immutable snapshot。
4. dirty 是本轮唯一能表达 no-change/稀疏变化而不要求每次发送完整 selected snapshot 的候选，但十万 full high-water 下 cache + delta + two-generation index 已约 `40.8 MB`，还未包含所有 Adapter/session buffer。其 generation、selection churn、tombstone 和跨层事务成本需要 representative product profiling 才能证明值得。
5. 预热后零分配在三类 Core candidate 与 selected Adapter materialize/extract/apply 上均已证明；这只说明 caller-owned capacity reuse 可行，不代表 retained memory 或 wall-clock 可接受。

因此 P3 的建议是：**不冻结 production selective snapshot、dirty 或 cursor API，不新增 ADR；#72 可继续进入 P4 individual-first reduced-rate semantics。** 如果未来实际 Adapter profiling 证明 full `vehicles()` read 是主要瓶颈，应新建独立 G1，优先比较更紧凑的 value record、Adapter-local selection/dirty ownership 和真正 storage-independent traversal；不得直接把本研究的 120-byte record、bitmap、epoch width 或 cursor encoding提升为 Stable Runtime API。

### P4. Individual-first reduced-rate semantics

目标：在不改变 production Core API 的研究 harness 中，让昂贵 controller 以 `N=2/4/8` base ticks 更新，同时保留 live vehicle identity、route/progress、Parking binding、每 tick committed occupancy/safety authority 和确定性事件时间。该 prototype 不要求与 `N=1` state 完全相同，但必须先定义允许的 fidelity delta，并以 `N=1` production path 作为安全、不变量和性能 oracle。

至少覆盖：dense following、route transition、SignalStop、Parking arrival/release、controller 刷新边界和跨候选 partition bucket；记录 no-overlap/stop compliance、identity continuity、事件顺序、行为偏差、CPU、内存与 Adapter 输出。产品 tolerance 未明确或收益不足时，不进入 production G1。

#### P4 #212 研究结果（2026-07-23）

**状态**：测试专用语义原型通过，当前全量双缓冲事务 cache 的性能 Gate 未通过。ablation 已证明 IIDM controller-intent 降频本身有可测收益，因此保留 individual-first reduced-rate semantics 及其缓存事务优化作为研究候选；当前实现仍不是 production 候选，不新增 Core/Adapter API、runtime dependency、data format、默认行为或 ADR，也不启动 production G1。

**provenance 降级落地（2026-08-14，#380 切片 ③）**：按 `core-research-instrumentation-externalization.md` §3 A 类冻结与 #380 降级记录（comment-5288562896 / comment-5289304629），P4 harness 的可运行形态已从 main 移除：`world_reduced_rate_research_tests.rs` 模块、`CoreWorld.reduced_rate_research` 字段、reduced-rate Active 分叉与 P4 独占的 `evaluate_controller_intent_for_research` / `compute_motion_from_controller_intent_for_research` seam 随切片 ③ 删除。源码经 git 历史保留，最后可运行 commit 为 `879343a5`（切片 ③ 合并的父 commit，以合并时 #380 记录为准）。本节及以下 P4 证据按历史证据语义保留，不作为可重跑基线；语义 oracle 与 research-commit 复现由 #218 在仪器探针边界（`StepProbe` / `StageTimingProbe`）上重建。

原型原位于 `crates/laneflow-core/src/world_reduced_rate_research_tests.rs`（已随上条 provenance 降级移除），由 `world` 下的 `#[cfg(test)]` 私有模块编译；`longitudinal.rs` 只增加 test-only controller-intent seam。生产路径仍每个 base tick 重建 occupancy/leader、safe-speed、当前 edge/route speed limit、route end、SignalStop、ParkingStop、全局 leader chain/cycle projection、最终 motion/events，并原子提交全部 vehicle state。跨 tick 只允许缓存 finite signed comfort acceleration 及 cadence/失效元数据；leader observation、constraint、projection、candidate/final motion 和 applied acceleration 均不缓存。

研究矩阵冻结并验证了以下行为：

- cadence 为 `N=1/2/4/8`，phase 为 synchronized 或按 logical stable update sequence 的 stable-staggered；invalidations 为 minimal 或 semantic-reactive。
- missing/generation、spawn/replace、非 Active→Active、profile/route binding、route occurrence/edge transition、Parking release 后首次 Active 强制刷新；Completed/despawn 丢弃缓存。semantic-reactive 另外在 leader identity 或 restrictive Signal/Parking stop-set identity 变化时刷新。
- forced refresh 不改变固定 cohort phase；每车每 tick 最多刷新一次。cache 使用 committed/candidate 事务边界，failed step 不交换；清除注入后 retry 与 fresh replay 的 world、motion 和 events 相同。
- `N=1` harness 不创建无用 cache；256 tick 中 `StepResult`、authority state、leader identity 与 13 个 longitudinal float bit pattern 全部等于 production oracle。
- no-overlap、finite/non-negative speed、route/status/identity continuity 为零容忍；Signal phase/group events 逐 tick 完全一致。ChangedEdge、Parking arrival、route-completion release 与 Completed 的 payload/顺序/因果一致，允许的 tick shift 不超过 `N-1`；release 与 Completed 保持同 tick 且 release 在前。
- synchronized 只保留为 cadence spike stress；stable-staggered 不把 partition、slot、raw handle bits 或 worker completion order 引入 phase。

fidelity budget 使用：

```text
tau = (N - 1) * dt
acceleration_span = max_acceleration + comfortable_deceleration
speed_budget = acceleration_span * tau
distance_budget = desired_speed * tau + 0.5 * acceleration_span * tau^2
```

在 `dt=16 ms`、16-vehicle dense-following、512 tick 的 stable-staggered semantic-reactive fixture 中，progress 同时作为单直线 lane 的 committed pose 距离 oracle；既有 Spatial/Bevy exact test 继续证明 committed pose input 到 canonical pose/local Transform 的逐位映射。

|   N |        speed budget / p50 / p95 / max (m/s) |  progress/pose budget / p50 / p95 / max (m) |          gap p50 / p95 / max (m) |   H / 2H / 4H endpoint drift (m) |
| --: | ------------------------------------------: | ------------------------------------------: | -------------------------------: | -------------------------------: |
|   2 | `0.067200 / 0.002916 / 0.010888 / 0.021244` | `0.320538 / 0.005569 / 0.028066 / 0.032467` | `0.003431 / 0.013204 / 0.034043` | `0.009352 / 0.028097 / 0.027591` |
|   4 | `0.201600 / 0.008346 / 0.031054 / 0.039306` | `0.964838 / 0.016913 / 0.075167 / 0.097760` | `0.011005 / 0.039867 / 0.089333` | `0.028526 / 0.085569 / 0.071451` |
|   8 | `0.470400 / 0.019861 / 0.074392 / 0.095374` | `2.266342 / 0.039293 / 0.174198 / 0.230754` | `0.025956 / 0.095512 / 0.207645` | `0.068956 / 0.205869 / 0.166137` |

没有 candidate 超出 speed/progress/gap budget，H→2H→4H 也没有持续增长。完整 correctness matrix 同时运行两个相同 candidate replay；state、motion、cache 和全部 events 均保持 deterministic。

性能 primary workload 在测量前声明为：单 route/edge 上的 16-vehicle cohorts，cohort 内 `15 m` spacing、cohort 间 `120 m` gap，初速循环 `7–13 m/s`，约 `15/16` vehicles 处于 following 视野、其余为 free-flow 边界；scale 为一万/十万，fixed step 为 `16 ms`。本机为 AMD Ryzen 9 9955HX、Windows、Rust 1.96 release；每个 tail case 先 warm 32 tick，再观测 1024 tick（N=8 的 128 个完整 cadence cycle），串行运行三轮。UE Editor 与一个孤儿 filesystem scan 存在时的中断样本全部作废，以下只采用两者退出后的重跑。

H1 对 P0 的 whole-step p50 paired delta 为：

| scale |   round 1 |    round 2 |   round 3 | 三轮中位数 |       Gate |
| ----: | --------: | ---------: | --------: | ---------: | ---------: |
|  一万 | `-4.797%` |  `+1.944%` | `+2.852%` |  `+1.944%` | `≤5%` Pass |
|  十万 | `-0.127%` | `-13.640%` | `+2.132%` |  `-0.127%` | `≤5%` Pass |

十万 tail raw observations 如下；单位均为 ms。`long` 是整个 longitudinal rebuild，不是只计 IIDM 函数。

| round | case            |            whole p50 / p95 / p99 / max |          long p50 / p95 / p99 / max |
| ----: | --------------- | -------------------------------------: | ----------------------------------: |
|     1 | P0              |    `35.648 / 40.220 / 42.548 / 44.578` |                                 N/A |
|     1 | H1 N1           |    `35.603 / 40.103 / 42.962 / 47.998` | `19.490 / 22.365 / 23.954 / 26.800` |
|     1 | stable N4       |    `36.144 / 40.665 / 42.585 / 44.920` | `19.421 / 22.416 / 24.060 / 25.331` |
|     1 | stable N8       |    `35.767 / 40.423 / 42.333 / 45.561` | `19.124 / 22.147 / 23.841 / 26.945` |
|     1 | synchronized N8 |  `36.653 / 88.352 / 104.338 / 136.526` | `19.529 / 45.778 / 56.748 / 95.793` |
|     2 | P0              | `41.384 / 102.730 / 117.533 / 174.687` |                                 N/A |
|     2 | H1 N1           |    `35.739 / 40.341 / 42.262 / 45.104` | `19.530 / 22.627 / 24.445 / 27.916` |
|     2 | stable N4       |    `36.594 / 41.278 / 42.726 / 45.907` | `19.703 / 23.031 / 24.638 / 26.640` |
|     2 | stable N8       |    `36.601 / 41.605 / 43.801 / 47.888` | `19.568 / 22.926 / 24.829 / 27.997` |
|     2 | synchronized N8 |    `36.060 / 40.592 / 43.211 / 47.961` | `19.010 / 22.261 / 24.895 / 29.075` |
|     3 | P0              |    `35.077 / 39.329 / 41.104 / 50.195` |                                 N/A |
|     3 | H1 N1           |    `35.825 / 39.842 / 41.422 / 45.318` | `19.686 / 22.509 / 23.815 / 31.118` |
|     3 | stable N4       |    `36.688 / 41.134 / 43.116 / 49.510` | `19.690 / 22.840 / 24.230 / 28.834` |
|     3 | stable N8       |    `36.261 / 41.589 / 44.412 / 51.794` | `19.386 / 22.442 / 24.525 / 29.862` |
|     3 | synchronized N8 |    `35.584 / 40.373 / 42.558 / 50.028` | `18.847 / 22.452 / 24.298 / 27.166` |

tail Gate 的直接结果：

- stable N4 的十万 longitudinal gain 为 `+0.356% / -0.890% / -0.018%`，whole-step gain 为 `-1.518% / -2.392% / -2.409%`；未达到 `15% / 5%`。
- stable N8 的十万 longitudinal gain 为 `+1.878% / -0.197% / +1.527%`，whole-step gain 为 `-0.460% / -2.411% / -1.217%`；第三轮 p99 相对 H1 回归超过 5%，也未达到 median Gate。
- synchronized N8 的第一轮 p95/p99/max spike 显著，支持“只作 spike stress、不作为交付候选”的冻结结论。
- 一万 stable N4/N8 的 whole-step p50 均未超过 H1 5% guard，但这不能替代十万 gain Gate。

Criterion 使用相同十万 primary workload、每组 10 samples、1 s warm-up、3 s measurement，并保留 `target/criterion/reduced-rate_100k_round-*/.../new/estimates.json`。`median.point_estimate` raw 值如下：

| round | stage        |          P0 (ns) |       H1 N1 (ns) |   stable N8 (ns) | N8 vs H1 gain |
| ----: | ------------ | ---------------: | ---------------: | ---------------: | ------------: |
|     1 | whole        | `37,101,469.170` | `37,168,483.330` | `36,508,975.000` |     `+1.770%` |
|     2 | whole        | `35,911,948.960` | `36,492,293.750` | `36,045,094.440` |     `+1.230%` |
|     3 | whole        | `36,031,381.250` | `35,784,805.560` | `37,045,719.050` |     `-3.520%` |
|     1 | longitudinal |              N/A | `20,085,591.670` | `19,853,305.360` |     `+1.160%` |
|     2 | longitudinal |              N/A | `20,457,482.500` | `19,913,915.620` |     `+2.660%` |
|     3 | longitudinal |              N/A | `20,695,966.670` | `20,290,075.000` |     `+1.960%` |

Criterion 中 longitudinal 三轮方向一致但远低于 15%；whole-step 方向不一致且远低于 5%。因此 tail 与 Criterion 独立支持相同 no-go 结论：当前 step 的主要成本不在可跳过的 IIDM controller-intent，缓存访问与事务复制也抵消了大部分局部节省。

为把“缓存成本”和“降频收益”从上述组合结果中拆开，另补一轮 ablation。冻结的 workload、release profile、十万 scale、10 samples、`1 s` warm-up、`3 s` measurement、三轮串行执行与原 Gate 均不变：

- P0：production；
- H1：`N=1` test seam，不创建 cache，用于扣除 harness 本身；
- C1：`N=1` 且每 tick 强制刷新，但仍执行 committed/candidate cache 写入、整表事务复制与 sweep；相对 H1 是“纯缓存事务 bookkeeping”；
- C2：当前 stable-staggered semantic-reactive `N=8` 事务候选；
- C3：与 C2 成功步骤逐 tick、逐 bit 相同的 `N=8` 原地 cache，只移除 candidate 双缓冲/整表复制。C3 在注入 failed step 后会改变 cache，测试明确证明其**不满足失败原子性**，因此只作为性能上界，不能参与 production Pass 判定；
- controller-only：对相同十万 cohort 输入单独执行 IIDM comfort-acceleration intent，量出可跳过函数本身的成本。

ablation 的 Criterion `median.point_estimate` 如下，单位为 ms；`whole / long` 表示 whole-step / 整个 longitudinal rebuild，IIDM-only 为单独的十万 controller-intent batch：

| round | P0 whole |   H1 whole / long |   C1 whole / long |   C2 whole / long |   C3 whole / long | IIDM-only |
| ----: | -------: | ----------------: | ----------------: | ----------------: | ----------------: | --------: |
|     1 | `36.018` | `36.374 / 20.674` | `39.792 / 22.978` | `37.433 / 20.755` | `35.375 / 19.031` |   `3.491` |
|     2 | `37.231` | `36.013 / 20.543` | `39.250 / 23.584` | `37.118 / 20.275` | `34.747 / 19.307` |   `3.442` |
|     3 | `36.433` | `36.795 / 22.338` | `39.043 / 23.171` | `36.209 / 20.373` | `35.352 / 18.908` |   `3.443` |

逐轮百分比先各自按 paired baseline 计算，再取三轮中位数；正数表示更快，负数表示更慢，归因结果为：

| 问题       | whole-step | longitudinal | 解释                                                           |
| ---------- | ---------: | -----------: | -------------------------------------------------------------- |
| C1 相对 H1 |  `-8.988%` |   `-11.147%` | 每 tick 事务 cache 本身是显著负成本                            |
| C2 相对 C1 |  `+5.930%` |   `+12.076%` | `N=8` 跳过 intent 确实回收了一部分成本                         |
| C2 相对 H1 |  `-2.909%` |    `+1.302%` | 正式语义候选的净收益基本被事务 cache 抵消                      |
| C3 相对 C2 |  `+5.497%` |    `+7.189%` | 移除双缓冲/整表复制能回收明显成本，但会破坏 failed-step 原子性 |
| C3 相对 H1 |  `+3.517%` |    `+7.947%` | 即使采用不可交付的原地上界，三轮中位数仍未达到 `5% / 15%`      |

controller-only 占 H1 longitudinal 的三轮比例中位数为 `16.757%`。即使假设 `N=8` 对其中 `7/8` 的工作完全免费、没有 cache lookup/write、失效判断或事务成本，理论可跳过份额也只有约 `14.662%`，已低于冻结的 `15%` longitudinal Gate。由此可以把结论收窄为：**降频本身有可测价值，但当前 IIDM intent 太便宜；事务 cache 又足以吞掉该价值。** 这不是“所有缓存或所有 reduced-rate 都无效”的结论，而是当前 component、workload、语义和 Gate 组合的 no-go。

第二轮 ablation 将 C2 的 dense transaction 替换为 C4 sparse transaction journal：

- step 内始终只读 committed cache；刷新项顺序写入预分配 journal，Stopped/Parked/Completed 使用独立 invalidation journal；
- authority step 成功后才把 journal 应用到 committed cache，failed step 不写 committed cache，retry 时丢弃旧 journal；
- 不再每 tick 复制整个 committed cache，也不再扫描整个 cache 做 sweep；same-tick route completion 由 committed event identity 精确清理；
- C4 与 C2 在 512 个成功 tick 的 `StepResult`、authority、13 个 longitudinal float bit pattern、cache entries 和 metrics 全部相同；注入失败后 committed cache/metrics 不变，retry 与 fresh replay 相同；
- C4 仍使用相同 `128 B` cache entry，以便这一轮只隔离事务形态，不混入 compact layout。

C4 使用相同十万 workload 与 Criterion 参数重新配对 P0/H1/C2/C3，`median.point_estimate` 单位为 ms：

| round | P0 whole |   H1 whole / long |   C2 whole / long |   C4 whole / long |   C3 whole / long |
| ----: | -------: | ----------------: | ----------------: | ----------------: | ----------------: |
|     1 | `35.945` | `36.155 / 21.158` | `36.628 / 20.649` | `34.870 / 19.793` | `35.604 / 19.319` |
|     2 | `36.026` | `35.841 / 21.094` | `35.639 / 21.178` | `35.281 / 19.144` | `35.367 / 19.130` |
|     3 | `36.097` | `36.253 / 20.446` | `36.863 / 20.839` | `34.692 / 19.571` | `34.773 / 19.214` |

逐轮 paired gain 的三轮中位数为：

| 对照       | whole-step | longitudinal | 结论                                                                |
| ---------- | ---------: | -----------: | ------------------------------------------------------------------- |
| C4 相对 P0 |  `+2.991%` |          N/A | 三轮 whole-step 均快于 production                                   |
| C4 相对 H1 |  `+3.554%` |    `+6.453%` | 三轮两个 stage 均为正收益，但仍低于冻结的 `5% / 15%` Gate           |
| C4 相对 C2 |  `+4.799%` |    `+6.084%` | 去掉全量复制与全表 sweep，回收了 dense transaction 的主要成本       |
| C4 相对 C3 |  `+0.242%` |    `-1.858%` | C4 whole 已贴近/略优于原地上界，long 只付出小幅原子事务成本         |
| C2 相对 H1 |  `-1.308%` |    `-0.397%` | 同一轮配对再次确认 dense transaction 会吞掉 reduced-rate 的局部收益 |

因此 C4 给出的新结论是：**失败原子性不是获得性能收益的根本障碍；O(V) 全量复制与 sweep 才是。** sparse journal 在不放弃 strong-individual semantics、determinism 或 failed-step 原子性的前提下，把组合结果从接近零/负值推进为稳定净收益，并几乎达到不合规 C3 的上界。它尚未达到冻结 Gate，所以仍不是 production 候选；但它已经把“寻找性能提升方式”的下一靶点缩小到 per-tick full-entry key read、`128 B` entry/journal 写放大和 IIDM component 本身的成本占比，而不是 reduced-rate 调度或事务语义。

内存和 allocation 证据：

- `Option<ResearchIntentCacheEntry>` 为 `128 B`。一万 authoritative cache 为 `1.28 MB`，十万为 `12.80 MB`；failed-step 原子性需要等大的 candidate transaction scratch，因此 prototype 总高水位分别为 `2.56 MB / 25.60 MB`，不含 allocator metadata。
- N=1 harness 不分配 cache；C1/C2 使用 cache 加等大的 transaction scratch；C3 只保留 cache、transaction scratch 为 `0 B`，但不满足失败原子性。
- C4 为了让任意首轮刷新/失效都不分配，预留 `128 B × V` refresh journal 和 `8 B × V` invalidation journal；因此一万/十万 retained high-water 为 `2.64 MB / 26.40 MB`。它减少的是每 tick 实际触达与复制的数据量，当前版本尚未降低 retained high-water。
- H1、C1、C2、C3、C4 在一万/十万预热后连续 16 step 均为 `0 allocation / 0 reallocation / 0 allocated bytes / 0 reallocated bytes`。
- 既有 `selected_inputs_canonical_pose_counts_and_local_transforms_match_full_oracle` 重跑通过，确认 Adapter 仍只消费 committed snapshot；P4 没有新增 Core interpolation、cache exposure 或 presentation authority。

##### D5. 十万单线程阶段归因与 longitudinal 内核诊断

为回答“除了 IIDM intent 和降频之外，longitudinal 及整个 step 的下一性能靶点在哪里”，补充了两类只在 `#[cfg(test)]` 下启用的诊断：

1. coarse stage timing（粗粒度阶段计时）：每个 tick、每个阶段只取一次 `Instant`，把 whole-step 拆为 occupancy/leader rebuild、longitudinal proposal/store、global projection、advance/events/authority commit 和 research cache commit；这组数据按同一 tick 采样，可以用于阶段占比和近似加和。
2. independent Criterion kernels（独立内核基准）：对 IIDM intent、post-intent safe motion、scratch begin、motion store 和 global projection 分别测十万 batch；它们用于解释机制，但因输入布局、cache 状态和循环边界不同，**不得把独立数字简单相加当成 whole-step 分解**。

**探针替代边界回写（2026-08-14，#380 切片 ②）**：本节 coarse stage timing 原由深嵌生产路径的 5 处 `#[cfg(test)] Instant::now()` 钩子采集（occupancy、longitudinal 总计、proposal/store、global projection、advance/events/authority commit），research cache commit 由 P4 研究状态内嵌记录。该内嵌形态已按 `core-research-instrumentation-externalization.md` §3 B 类冻结收敛为 method-generic 仪器探针边界：`CoreWorld::step_with_probe<P: StepProbe>` + 默认 `NoOpProbe`（`ENABLED = false`，发布构建不读取时钟、空操作整体编译消除），研究态记录实现 `StageTimingProbe`（六段 `Duration` 快照，含 research-commit）由 `instrumentation` feature 或 crate 内测试启用，Criterion 取证（`criterion_100k_three_round_*`）已迁移为探针实现。D5 下表 H1/C4 行全部列与 kernel 诊断命令（`reduced-rate-ablation` controller-only 与 motion benchmark 输入）的可运行复现随 P4 provenance 降级整体暂停（降级记录见 #380 评论，P4 状态机移除在切片 ③ 落地）；已记录数值按历史证据语义保留，不作为可重跑基线。生产路径探针在 P0 workload 上的五段计时（occupancy、longitudinal 总计 / proposal / projection、post-longitudinal）复现能力完整保留：对任意 workload 以 `StageTimingProbe` 运行 `step_with_probe` 即可按 tick 采样六段计时。

外部 sampled profile 使用 Windows Performance Recorder（WPR）采集 CPU sampling ETL。非提权进程首次执行 `wpr -start CPU` 返回 `0xc5585011: Failed to enable the policy to profile system performance`；随后通过 UAC 启动一次性 elevated helper，确认令牌包含 `SeSystemProfilePrivilege` 后，WPR start、十万 H1 workload 和 WPR stop 的退出码均为 0，不需要修改本地安全策略。第一次 coarse run 还暴露了 instrumentation lifecycle 问题：`begin_step` 会清零刚记录的 occupancy duration；该轮已判无效、不计入结果，修正后从干净 workload 重跑。

修正后的 coarse timing 使用 release profile、十万 mixed cohort、H1/C4 各 512 observed ticks、三轮串行执行。下表为各分布的 p50，单位为 ms；各列分别取 p50，因此一行中的 p50 不要求严格相加：

| case | round | whole-step | occupancy / leader | proposal / store | global projection | longitudinal total | advance / events / authority commit | research cache commit |
| ---- | ----: | ---------: | -----------------: | ---------------: | ----------------: | -----------------: | ----------------------------------: | --------------------: |
| H1   |     1 |   `36.893` |           `13.074` |         `18.803` |           `1.822` |           `20.625` |                             `3.144` |               `0.000` |
| H1   |     2 |   `35.915` |           `12.689` |         `18.348` |           `1.767` |           `20.105` |                             `3.038` |               `0.000` |
| H1   |     3 |   `35.613` |           `12.622` |         `18.162` |           `1.742` |           `19.943` |                             `2.998` |               `0.000` |
| C4   |     1 |   `34.651` |           `12.521` |         `16.707` |           `1.720` |           `18.505` |                             `3.010` |               `0.532` |
| C4   |     2 |   `34.699` |           `12.474` |         `16.740` |           `1.738` |           `18.505` |                             `3.007` |               `0.533` |
| C4   |     3 |   `34.788` |           `12.617` |         `16.664` |           `1.764` |           `18.476` |                             `3.043` |               `0.530` |

H1 各轮 share 的三轮中位数表明：

- occupancy/leader rebuild 占 whole-step `35.438%`，是最大的 non-longitudinal 单项；
- longitudinal total 占 whole-step `55.980%`；
- proposal/store 占 longitudinal `91.166%`，约占 whole-step `51%`；global projection 只占 longitudinal `8.786%`，约占 whole-step `4.9%`；
- advance/events/authority commit 占 whole-step `8.457%`；
- 按每个 tick 先扣除已记录阶段后再取 p50，whole-step unattributed 只有约 `0.003 ms`，说明 coarse attribution 基本闭合，没有足以改变优先级的未知大块成本。

C4 的 savings 也由阶段数据得到机制解释：三轮 p50 中位数相对 H1，proposal/store 从 `18.348 ms` 降到 `16.707 ms`，约节省 `1.641 ms`；global projection、occupancy/leader 和 post stage 基本不变，同时 sparse journal commit 新增约 `0.532 ms`。因此 C4 的净收益主要来自跳过 IIDM intent 后的 proposal path，而不是 projection；compact journal 最多先回收约半毫秒，仍小于 occupancy/leader 和 proposal path 的可优化份额。

独立 Criterion kernels 使用相同十万 scale、每组 10 samples、`1 s` warm-up、`3 s` measurement、三轮串行执行。`median.point_estimate` 单位为 ms：

| kernel                  | round 1 | round 2 | round 3 | three-round median | 相对 H1 proposal p50 |
| ----------------------- | ------: | ------: | ------: | -----------------: | -------------------: |
| IIDM controller intent  | `3.470` | `3.498` | `3.459` |            `3.470` |             `18.91%` |
| post-intent safe motion | `5.874` | `5.782` | `5.915` |            `5.874` |             `32.02%` |
| scratch begin           | `0.119` | `0.122` | `0.119` |            `0.119` |              `0.65%` |
| motion store            | `2.467` | `2.478` | `2.509` |            `2.478` |             `13.50%` |
| global projection       | `1.679` | `1.627` | `1.648` |            `1.648` |                  N/A |

这组 kernel ablation 把 proposal 的下一靶点进一步收窄：

- post-intent safe motion（ballistic integration、emergency/safe-speed clamp、speed-ceiling clamp 和 motion materialization）是已隔离的最大内核，约为 IIDM intent 的 `1.69×`；只优化或降频 IIDM 不会消除它；
- motion store 的独立成本约 `2.478 ms`，scratch begin 只有 `0.119 ms`。因此“每 tick 清空 scratch”不是主要问题，motion record 写入与其数据布局更值得继续比较；
- 四个 proposal kernels 的独立中位数合计约为 H1 proposal p50 的 `65.08%`。剩余部分不能由独立数字直接相减定罪，但结合生产 loop 可将下一诊断范围放在 per-vehicle handle/state/profile/edge/leader lookup、constraint identity/key 构造、cache branch/journal bookkeeping 和 loop/data locality；
- 独立 projection `1.648 ms` 与 coarse projection `1.767 ms` 同量级，进一步支持 projection 不是当前第一优化靶点。即使把 projection 变成零成本，whole-step 上界也只有约 5%，且不能牺牲跨区 leader chain/cycle 的全局求解语义。

WPR 证据使用 `CPU` profile 覆盖十万 H1 external workload 的 1,024 ticks；workload 本身耗时 `54.98 s`，生成 `1,221,591,040 B` ETL。`xperf -symbols -a profile -detail` 使用同一 release binary 的本地 PDB 解码，以下占比均以目标 `laneflow_core` 进程的 exclusive sampled CPU weight 为分母；LaneFlow binary 自身占该进程 `92.681%`，其余主要是 CRT/kernel runtime：

| exclusive sampled function                           | target process share | LaneFlow module share |
| ---------------------------------------------------- | -------------------: | --------------------: |
| `CoreWorld::find_leader`                             |             `23.15%` |              `24.97%` |
| `CoreWorld::rebuild_longitudinal_motions`            |             `13.17%` |              `14.21%` |
| `iidm_acceleration`                                  |             `12.36%` |              `13.33%` |
| `compute_motion_from_controller_intent_for_research` |              `7.82%` |               `8.44%` |
| `CoreWorld::step`                                    |              `6.20%` |               `6.69%` |
| `CoreWorld::leader_horizon`                          |              `5.90%` |               `6.37%` |
| `safe_speed`                                         |              `5.04%` |               `5.44%` |
| `CoreWorld::build_occupancy`                         |              `3.63%` |               `3.91%` |
| `CoreWorld::rebuild_occupancy_and_leaders`           |              `2.88%` |               `3.11%` |
| `LongitudinalScratch::project`                       |              `2.41%` |               `2.60%` |
| `ReducedRateResearchState::controller_intent`        |              `1.90%` |               `2.05%` |
| `LongitudinalMotion::apply_speed_limit_constraint`   |              `1.88%` |               `2.02%` |
| `LongitudinalScratch::begin`                         |              `0.56%` |               `0.60%` |

sampled profile 与 coarse/criterion 交叉验证后的解释是：

- `find_leader + leader_horizon + build_occupancy + rebuild_occupancy_and_leaders` 的 exclusive samples 合计约占目标进程 `35.56%`，与 coarse occupancy/leader 的 `35.438%` 几乎相同；其中 `find_leader` 单项就占 `23.15%`，因此 occupancy 下一轮应先优化 horizon-bounded leader traversal、route-distance candidate/index access 和数据局部性，而不是先优化 edge sort。`sort_edges`、occupancy scratch begin/allocate 的单项 sampled share 均低于 `0.6%`。
- `iidm_acceleration` 占 `12.36%`；post-intent 的 `compute_motion... + safe_speed` exclusive samples 合计 `12.86%`，再次确认 IIDM 降频只覆盖 longitudinal 的一部分，safe-motion 计算同样值得 batch/SIMD 与数据路径 ablation。
- `rebuild_longitudinal_motions` 仍有 `13.17%` exclusive self weight，主要容纳未单独成函数或已内联的 per-vehicle lookup、branch、cache/bookkeeping 和 motion store，支持继续拆 proposal loop/data locality。
- `LongitudinalScratch::project` 只有 `2.41%` target-process share；WPR、coarse timing 与独立 Criterion 三种方法都把 global projection 排在 occupancy 和 proposal 之后。

因此本轮的单线程优化优先级是：

1. **occupancy/leader exact path**：占 whole-step 约 35.4%，先做数据访问、索引和 partition/halo exact ablation；它仍是每 tick safety authority，不能直接降频。真实分区或并行实施超出 #212，必须新 Issue 进入 G1，并继续满足全局 dependency component 求解。
2. **longitudinal proposal path**：优先隔离 post-intent safe motion 的 batch/SIMD 可行性、motion record 写放大与 lookup/data locality；IIDM cache 只是其中一部分。
3. **C4 cache/journal layout**：compact hot/cold entry 或 lifecycle epoch 可继续回收约 `0.532 ms` commit 和 per-tick key/journal 触达，但预期收益小于前两项。
4. **advance/events/authority commit**：约 8.5%，在前两项之后再拆 route advance、event materialization 与 commit。
5. **global projection**：约 4.9%，除非后续更复杂 leader graph 改变占比，否则不应作为第一项。

这些诊断仍全部是单线程、test-only seam，没有引入线程池、worker、work stealing、production partition、生产 cache 或 public API/Data/Adapter 变更。

可复现命令：

```text
cargo test -p laneflow-core reduced_rate_research_tests -- --test-threads=1
cargo test -p laneflow-core stable_semantic_fidelity_report_includes_distributions_and_horizon_drift -- --nocapture --test-threads=1
cargo test --release -p laneflow-core world::reduced_rate_research_tests::release_tail_matrix_reports_p50_p95_p99_max_and_gate_classification -- --exact --ignored --nocapture --test-threads=1
cargo test --release -p laneflow-core world::reduced_rate_research_tests::warm_10k_100k_reduced_rate_step_is_zero_allocation -- --exact --ignored --nocapture --test-threads=1
cargo test --release -p laneflow-core world::reduced_rate_research_tests::criterion_100k_three_round_whole_step_and_longitudinal_matrix -- --exact --ignored --nocapture --test-threads=1
cargo test --release -p laneflow-core world::reduced_rate_research_tests::criterion_100k_three_round_cache_and_downrate_ablation -- --exact --ignored --nocapture --test-threads=1
cargo test --release -p laneflow-core world::reduced_rate_research_tests::criterion_100k_three_round_sparse_transaction_ablation -- --exact --ignored --nocapture --test-threads=1
cargo test --release -p laneflow-core world::reduced_rate_research_tests::release_100k_three_round_step_stage_diagnostics -- --exact --ignored --nocapture --test-threads=1
cargo test --release -p laneflow-core world::reduced_rate_research_tests::criterion_100k_three_round_longitudinal_kernel_diagnostics -- --exact --ignored --nocapture --test-threads=1
cargo test --release -p laneflow-core world::reduced_rate_research_tests::release_100k_h1_external_profile_workload -- --exact --ignored --nocapture --test-threads=1
cargo test -p laneflow-bevy selected_inputs_canonical_pose_counts_and_local_transforms_match_full_oracle -- --test-threads=1
```

WPR CPU profile 需要从包含 `SeSystemProfilePrivilege` 的 elevated PowerShell 启动；`Disabled` 表示权限存在但尚未由进程启用，不是缺失。采集顺序为 `wpr -start CPU`、运行上述 external workload、`wpr -stop <trace.etl> <description>`；随后设置 `_NT_SYMBOL_PATH` 指向 `target/release/deps` 的本地 PDB，并使用 `xperf -i <trace.etl> -symbols -o <report.txt> -a profile -detail` 解码 exclusive function report。

因此当前 P4 的建议是：**接受 reduced-rate 语义可表达性、strong-individual safety/event contract 和 sparse atomic transaction 作为已验证的性能方向，拒绝把 dense C2 或尚未过 Gate 的 C4 直接提升为 production runtime。** 若继续本 component，应隔离 compact hot/cold entry 或 lifecycle epoch，降低每 tick full-entry key read 与 journal 写放大；但若目标是 whole-step 的下一轮显著收益，优先级应转向 occupancy/leader exact path 与 longitudinal post-intent/motion-store/data-locality，而不是先优化 global projection。冻结 ablation 中 `N=8` IIDM-only 的理论上限约为 longitudinal `14.662%`，若目标仍是 `15%`，必须同时找到另一个可安全优化的 component 或做 IIDM batch/SIMD。不得通过扩大 fidelity budget、减少 per-tick safety authority、隐藏 synchronized spike、移除 failed-step 原子性或把 cache 暴露给 Adapter 来追求过线。

### P5. Private memory-layout experiment

目标：只有 profiler 证明 candidate clone、AoS cache locality 或 scratch retained memory 是主要成本时，才比较 AoS/SoA/chunked storage。实验不得先改变 public handle 或 Adapter API。

上述 prototype 不自动进入生产。生产实施至少需要同时满足：

- 已按交通执行域明确产品个体/活动/意图更新/表现/聚合记录/聚合等价参与单元数，
  以及 fidelity、tick-rate、hardware/platform 和 frame-budget 目标；
- representative workload 同时覆盖 Core 交通求解与实际 Adapter 读取边界；
- single-thread oracle、determinism、不变量和失败原子性都有自动对照；
- 性能收益显著且没有以扩大 public API、内存或迁移风险换取；
- 已通过新 Issue 的 G1，并按高影响结论判断是否新增 ADR。

## 7. API 与兼容性判断

本审计文档本身不改变 production API：

- Core API：无变更；
- Data format：无变更；
- Adapter API：无变更；
- runtime behavior：无变更。

Stable Runtime API G1 前的待决项：

| 边界              | 当前 contract                                  | 最迟决策                                                                           |
| ----------------- | ---------------------------------------------- | ---------------------------------------------------------------------------------- |
| Vehicle read      | `vehicle()` / `vehicles()` 返回 borrowed state | 是否保留长期 AoS authority view，或提供 value/snapshot/accessor 与 selective batch |
| Handle provenance | world/session scoped，wrong-world 未稳定定义   | 单 logical world 内部 partition，或多 World/shard translation/session identity     |
| Step events       | owned `Vec<CoreEvent>`、单线程 append order    | semantic phase/key、buffer reuse/streaming 和 deterministic merge contract         |
| Batch commands    | 单命令原子，批量语义未统一                     | canonical order、conflict、whole-batch/partial atomicity                           |
| Adapter Session   | Bevy 单活动 Session/单 frame specialization    | 多 frame/migration 是否只属 Adapter，Core snapshot 如何选择范围                    |

## 8. ADR 判断

本审计最初未新增 ADR，因为当时只保护既有 ADR 0003/0005/0016 的可扩展性，没有
选择 production partition、scheduler、identity encoding 或 multi-rate model。
#291 后续通过 ADR 0020/0021 补充了静态执行约束/每世界执行计划、产品北极星、
不可变路网修订和快照边界；它们仍未选择具体 partition 算法或任务运行库。

以下任一决策进入 production 前必须重新判断 ADR：

- 将一个 logical Core session 拆为多个 public World/shard；
- 改变 handle provenance、跨 partition migration 或 stale semantics；
- 冻结多频率 Core state/safety/event-time 语义；
- 冻结并行 phase graph 与 deterministic event merge contract；
- 引入持久 runtime snapshot、跨进程 identity 或分布式 authority；
- 以 breaking API 替换 borrowed VehicleState read contract。

## 9. 后续 G1 检查清单

触及 Core/Adapter public boundary 的 Issue 在 G1 应回答：

- 是否暴露或依赖 physical slot、partition、thread、container 或 ECS iteration order？
- handle 是否仍 opaque，是否错误地承担 external/persistent identity？
- update、snapshot、command 和 event 是否有 storage-independent stable order？
- 一万/十万默认路径是否存在 batch/capacity reuse，还是只能逐交通参与单元调用？
- command 在哪个 committed boundary 校验和生效，失败是否原子？
- worker/partition 数变化是否会改变 state、first error 或 event order？
- 所有 partition 是否读取同一 committed state，是否错误增加跨边界一 tick 延迟？
- 静态执行约束、可重建提示与每世界执行计划是否保持权威分离？
- 单资源连接组件是否有唯一规范归约权威，互不相交组件是否仍可并行？
- multi-rate/LOD 是否改变 Core safety/fidelity，还是只改变 Presentation？
- Data/Manifest 是否仍不包含 runtime handle、partition 或 Adapter entity？
- 路网修订、运行时快照和跨修订 dense-handle 迁移是否失败关闭？
- 是否有 single-thread production oracle 和 representative benchmark？
- 该结论是否需要 ADR、prototype 或新的实施 Issue？

## 10. 与 #72 和路线图的关系

#199 只完成 #72 验收中的 API 可扩展性前置切片；fidelity/target、
partition/identity、deterministic scheduling、memory layout 和 Adapter/Core
boundary 的完整研究仍由 #72 后续规划。#291 提供的静态约束与修订边界是该研究的
输入，不构成 parallel runtime G2。#72 的当前 Project、Milestone 与 Gate 状态以
GitHub 为准，不在本长期审计中镜像。

本审计不阻塞 v0.8/v0.9，也不把 v1.0 Scope TBD 自动变成城市级实现 Milestone。它只冻结一个最迟门槛：未来 Stable Runtime API 的 G1 必须引用本审计，并关闭或显式接受第 7 节待决项；完整并行、多层级或分布式实施仍需产品目标和性能证据后另立 Milestone。
