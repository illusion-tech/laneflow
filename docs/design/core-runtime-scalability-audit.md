# Core Runtime 可扩展性前置审计

**文档状态**: Review<br>
**最后更新**: 2026-07-22<br>
**适用范围**: #199 对 #72 的前置 Core API、identity、batch、command、deterministic scheduling 与 event merge 审计<br>
**关联文档**:

- `../roadmap.md`
- `../adr/0001-project-scope.md`
- `../adr/0003-runtime-tick-and-determinism.md`
- `../adr/0005-core-identity-and-handle-model.md`
- `../adr/0016-scenario-population-and-recycle-lifecycle-authority.md`
- `core-runtime.md`
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

当前设计没有要求推倒重来。以下基础可以继续作为 v0.8/v0.9 的实现输入：

- Core 使用显式 fixed step，不读取 wall clock 或隐藏引擎状态；
- runtime handle 保持 opaque、typed、无 public ordering，external ID 与 handle 分层；
- lifecycle command 只在 step 之间执行，并保持单命令失败原子性；
- Adapter 不公开 `&mut CoreWorld`，只消费 committed state；
- Spatial pose 使用稳定、调用方拥有的 batch input/output 与 placement token；
- Traffic Data 不持久化 runtime handle、partition、Entity 或 runtime snapshot。

但在未来 Stable Runtime API 的 G1 前，以下五个边界必须完成正式设计或明确迁移策略：

1. `CoreWorld::vehicles()` / `vehicle()` 返回 borrowed `VehicleState` 对内部 AoS/slot representation 的长期约束；
2. world-scoped handle 在物理 partition 迁移、多 Session 或多 World 下的 provenance 与 logical identity；
3. 面向 100k/1M、可见区域和 fidelity tier 的 selective batch snapshot/query；
4. lifecycle/batch commands 的 canonical order、冲突规则和 whole-batch atomicity；
5. 不依赖 worker completion、partition ID 或容器迭代的 deterministic phase/event merge。

因此，本审计的判断是：

- v0.8/v0.9 可以继续，不由 #72 阻塞；
- 触及上述五个边界的新设计必须显式说明 #72/#199 影响；
- 在产品目标和代表性 workload 明确前，不建立完整并行、多层级或分布式 Milestone；
- 在 Stable Runtime API G1 前，不能只依赖当前 public shape 自动推断未来兼容性。

## 2. 范围与非目标

本审计覆盖：

- `CoreWorld` ownership、step phase、candidate/commit 和 scratch/index 边界；
- Vehicle/Route/Edge handle、external ID、stale semantics 与迁移；
- Core snapshot/query、Spatial batch、Adapter Session 和 lifecycle transaction；
- fixed-step、多频率候选、command boundary 与 deterministic event order；
- 当前 10k/100k/历史 1M 证据对 API 的约束；
- 后续 prototype 与生产实施的启动触发。

本审计不覆盖：

- 选择 grid、lane cluster、graph cut、R-tree 或其他 partition 算法；
- 实现线程池、work stealing、多 World/shard、分布式协议或 GPU controller；
- 把 partition ID 或物理 slot 编进 public handle；
- 全面迁移 AoS/SoA；
- 承诺 100k/1M 实时 SLA、跨平台 bit-level determinism 或城市交通工程精度；
- 把登记机动车总量直接解释为同时在途 active agents。

## 3. 证据边界

现有数值来自不同 fixture、measurement scope 和版本，不能直接相加，也不能外推成跨平台 SLA。

| 规模 | 当前证据                                                                                                                                                                        | 可以支持的判断                                                                                          | 不能支持的判断                                                               |
| ---: | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------- |
|  10k | v0.3 Vehicle Following 常规 workload 约 `0.630–0.929 ms/tick`，transition-heavy 为 `1.247 ms/tick`；v0.5 100% Reserved 为 `1.263 ms/tick`；v0.7 Adapter batch p95 为 `3.067 ms` | 10k 高精度局部仿真和完整 Adapter 路径已有独立 Gate，可继续作为当前产品基线                              | 不能把不同 benchmark 相加后声明完整帧 SLA，也不覆盖 renderer 与所有未来规则  |
| 100k | v0.3 dense platoon 为 `10.660 ms/tick`；v0.4 mixed Signals 为 `12.651 ms/tick`；Spatial batch p95 为 `5.189 ms`；Adapter batch p95 为 `35.852 ms`                               | 多条 production path 接近线性扩展，未出现已知全局 `O(V²)` 证据；100k 足以作为复杂度和 API 压力观测      | 不是 60 Hz 高精度完整运行时承诺，不证明 Core + Adapter + renderer 的组合预算 |
|   1M | v0.2 临时 steady-state 约 `16.39–16.97 ms/tick`、峰值工作集约 `379 MiB`，但不含 occupancy、IIDM、edge transition 或事件                                                         | 单线程逐车微观完整 Core 不能依赖该 optimistic 结果直接扩容；需要单独研究 fidelity、memory 和 scheduling | 不能证明 1M Vehicle Following、Signals、Parking、Adapter 或城市级实时能力    |

当前只能建立研究用 fidelity 问题矩阵，不能冻结产品 tier：

| 候选层级                 | 当前可引用的语义                                                                              | 仍待产品/G1 决策                                                    |
| ------------------------ | --------------------------------------------------------------------------------------------- | ------------------------------------------------------------------- |
| Local exact              | 当前完整 fixed-step、Vehicle Following、Signals、Parking 与确定性安全不变量；10k 为已验证基线 | 精确上限、目标硬件、tick rate、最坏交通密度                         |
| Reduced-rate microscopic | 仍保留 per-vehicle identity/state，但并非每个 controller 每个 base tick 都更新                | stale interval 内的 occupancy/safety 语义、更新频率、插值和事件时间 |
| Mesoscopic/aggregate     | 只作为 #72 候选，不属于当前 Core 已接受能力                                                   | agent 聚合/拆分、守恒、route/signal 语义和与 exact region 的迁移    |

### 3.1 候选方案 A：Individual-first 分层扩展

**状态**：研究候选；具有明确产品价值，但不是 G1 决策、默认生产架构或城市级能力承诺。

该方案把较强的个体语义视为需要主动评估的产品价值，而不只视为性能成本。其核心假设是：在可接受的硬件、内存和 tick budget 内，live vehicle 应尽量跨 partition、fidelity tier 和 lifecycle transition 保持连续的 logical identity、route/progress、Vehicle Profile、Parking binding、committed state 与事件因果。这样才能支持可解释的车辆行为、稳定的 Adapter 映射、逐车调试和后续可能出现的业务车辆差异，而不是把所有远处交通默认降为无身份流量。

候选形态为：

- `Local exact` 继续使用完整 per-vehicle fixed-step 语义，优先通过私有 data-oriented storage、batch、phase scheduling 和并行计算扩展；ECS、worker 或 partition 不成为 public Core API。
- `Reduced-rate microscopic` 仍保留每辆车的 identity、route intent 和 committed state，只研究降低部分 controller 或昂贵派生计算的刷新频率；未刷新 tick 的 occupancy、安全约束、Signal/Parking authority、插值与事件时间仍必须满足 C7。
- `Mesoscopic/aggregate` 保留为可选的远域或超大规模方案，不作为默认前提。进入或离开该层级必须定义 identity 保留/恢复、数量守恒、route/signal 语义和 deterministic migration boundary；若无法保持逐车连续性，必须把语义损失作为产品能力差异显式暴露。
- 共享 route/cost field、聚合 occupancy 或其他 SC5-like 技术可以作为内部优化候选，但不得仅为了吞吐量隐式替换已经承诺的逐车 route、parking 或 lifecycle authority。

外部参考只用于校准，不构成规范性依赖：Cities: Skylines II 的公开资料展示了 persistent citizen/agent 语义与 ECS、Burst、多核批处理方向，但没有公开足以复制的 partition、multi-rate 或 deterministic merge contract；SimCity 2013 GlassBox 展示了低频 Unit/Map rules、轻量 resource-carrying agents 和共享距离场，但其聚合语义不能自动满足 LaneFlow 的逐车身份与路线需求。

- [Cities: Skylines II Traffic AI](https://www.paradoxinteractive.com/games/cities-skylines-ii/features/traffic-ai)
- [Cities: Skylines II Code Modding](https://www.paradoxinteractive.com/games/cities-skylines-ii/modding/dev-diary-3-code-modding)
- [Inside GlassBox developer talk](https://www.andrewwillmott.com/talks/inside-glassbox)

该方案进入 G1 前，必须与 exact-only 和 aggregate-first 候选使用相同 representative workload 比较，至少记录：可保留的个体语义、状态迁移复杂度、worker 数变化下的确定性、CPU/内存成本、Adapter 读取成本和失败恢复边界。在这些证据形成前，#72 只把 individual-first 作为一等候选，不预选最终架构；任何主动丢弃 live vehicle identity 的方案都应说明收益及不可逆语义损失。

### 3.2 候选方案决策矩阵

以下三种方案都是 #72 的研究输入，不是已经接受的 architecture。它们共享第 5 节 no-regret constraints；差异只在于如何分配个体语义、计算成本和规模上限。

| 维度                  | A. Individual-first 分层扩展                                                                   | B. Exact-only 数据导向扩展                                                    | C. Aggregate-first 资源流扩展                                                  |
| --------------------- | ---------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------- | ------------------------------------------------------------------------------ |
| 基本形态              | exact 为主，reduced-rate microscopic 保留身份，aggregate 只作可选远域                          | 所有 live vehicle 始终执行完整 exact 语义，只优化 layout、batch、phase 和并行 | 大部分远域交通使用 flow/packet，局部重要区域才展开为 exact vehicle             |
| 个体语义              | 优先保持 logical identity、route/progress、Parking/lifecycle 和事件连续性                      | 最强，所有 live vehicle 都保持当前完整语义                                    | 只在 exact island 内完整；aggregate tier 可能只有数量、目的地类别和流量守恒    |
| safety authority      | base tick 仍需定义 occupancy、Signals、Parking 和 stale interval 安全语义                      | 直接复用当前 fixed-step safety pipeline                                       | aggregate/exact 边界必须另定义容量、队列、冲突和展开后的安全初始条件           |
| partition / migration | 内部 ownership 可迁移，public identity 不随 partition 改变；tier transition 需显式 transaction | 只需 physical ownership migration，不存在 fidelity tier migration             | 同时需要 partition handoff 与 aggregate/exact 聚合、拆分、identity translation |
| Core/Adapter 连续性   | 可保留一个 logical `CoreWorld` facade 和 committed selective snapshot                          | 与当前 API 心智最接近，但仍需解除 borrowed AoS/full scan 约束                 | 最可能要求新的 snapshot capability 和明确的 tier-specific record               |
| 预期收益              | 在保留大部分产品语义的同时降低昂贵 controller 和远域计算成本                                   | 语义最简单、oracle 最强；若硬件目标可满足，则迁移成本最低                     | 理论规模上限最高，适合 active population 远大于可见 exact fleet 的目标         |
| 主要风险              | stale interval safety、tier scheduler、内存占用和 identity-preserving migration 复杂           | CPU/内存上限可能不足，parallel dependency 和 halo 成本仍可能很高              | 语义损失最大，聚合/拆分、路线和信号守恒容易形成新的复杂系统                    |
| 当前角色              | 一等研究候选；个体语义具有明确产品价值，但尚未被 G1 选中                                       | production oracle 与最低复杂度基线；必须先证明它是否已经足够                  | 扩展上限对照；只有目标和证据证明 identity-preserving 路径不足时才提升优先级    |

候选收敛顺序：

1. 先以 B 作为单线程 production oracle，完成 P1–P3 的 storage-independent order、partition halo 和 selective snapshot 证据；
2. 在产品允许的 fidelity delta 明确后，用 P4 和相同 workload 验证 A 是否能在不破坏 C2、C6、C7、C9 的前提下降低 CPU/Adapter 成本；
3. 只有产品 active-agent 目标、内存或 frame budget 证明 A/B 无法满足时，才启动 C 的 aggregate/exact migration 设计；
4. 最终选择必须在独立 G1 中记录适用场景、语义损失、兼容性、测试 oracle 和 ADR 判断，不由本审计自动决定。

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

当前 Bevy specialization 仍从 `CoreWorld::vehicles()` 全量遍历 borrowed `&VehicleState`，并在单活动 Session/单 canonical frame 下重建 pose inputs。该形态对 10k/100k Reference Adapter 可接受，但不能作为未来稳定 API 的唯一读取方式：

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

单记录 debug query 可以保留，但 10k/100k/1M 默认路径必须支持预留、复用和选择范围，且不能要求每车 resolver/string lookup。

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

## 6. Prototype 顺序与通过条件

### P1. Deterministic phase/event merge

目标：在不并行修改 production step 的情况下，把 current events映射到 canonical phase/key，并证明 1、2、N 个模拟 worker bucket 归并后与 current single-thread oracle 完全一致。

至少覆盖：route transition-heavy、projection-heavy、Signals phase change、Parking arrival/release、simultaneous completion 和失败 first-error。

#### P1 #204 研究结果（2026-07-22）

**状态**：测试专用原型通过；结论是继续把 canonical phase/key 作为 production scheduler 的候选输入，但不接受为 production architecture 或 public event contract。

原型位于 `crates/laneflow-core/src/world_event_merge_research_tests.rs`，仅由 `world` 下的 `#[cfg(test)]` 私有模块编译。production `CoreWorld::step`、`CoreEvent`、handle、Core/Data/Adapter API、data format、runtime dependency 和 runtime allocation 均未改变。原型采用以下六元组：

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
- 原型没有并行计算 occupancy、longitudinal、vehicle candidate state 或 Signal state，不证明 worker speedup、partition halo、跨 CPU bit-level determinism、production buffer reuse 或 100k/1M SLA。
- 若未来 production phase graph 接受该方向，应独立 Issue/G1 比较 k-way merge、预排序 worker buffer、caller-owned scratch 与无额外分配方案，并重新判断 ADR；在此之前不新增 ADR。

### P2. Partitioned occupancy/leader halo

目标：以 physical edge/lane cluster 的研究分区复现当前 occupancy、leader graph 和 projection 结果，测量 halo 数量、跨区 dependency、boundary migration 和 retained memory。

至少覆盖：长 horizon、多 edge route、环路 leader cycle、拥堵边界、Signals/Parking stop 与同 tick edge transition。

### P3. Selective snapshot/batch

目标：比较 current full `vehicles()` scan 与 caller-owned filtered/dirty/cursor prototype，在 10k/100k 下验证稳定顺序、零分配和 Adapter 等价 Transform；1M 只在有代表性数据布局后运行。

### P4. Individual-first reduced-rate semantics

目标：在不改变 production Core API 的研究 harness 中，让昂贵 controller 以 `N=2/4/8` base ticks 更新，同时保留 live vehicle identity、route/progress、Parking binding、每 tick committed occupancy/safety authority 和确定性事件时间。该 prototype 不要求与 `N=1` state 完全相同，但必须先定义允许的 fidelity delta，并以 `N=1` production path 作为安全、不变量和性能 oracle。

至少覆盖：dense following、route transition、SignalStop、Parking arrival/release、controller 刷新边界和跨候选 partition bucket；记录 no-overlap/stop compliance、identity continuity、事件顺序、行为偏差、CPU、内存与 Adapter 输出。产品 tolerance 未明确或收益不足时，不进入 production G1。

### P5. Private memory-layout experiment

目标：只有 profiler 证明 candidate clone、AoS cache locality 或 scratch retained memory 是主要成本时，才比较 AoS/SoA/chunked storage。实验不得先改变 public handle 或 Adapter API。

上述 prototype 不自动进入生产。生产实施至少需要同时满足：

- 已明确产品 active-agent、fidelity、tick-rate、hardware/platform 和 frame-budget 目标；
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

本审计暂不新增 ADR。原因是当前结论主要保护既有 ADR 0003/0005/0016 的可扩展性，没有选择新的 production partition、scheduler、identity encoding 或 multi-rate model。

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
- 10k/100k 默认路径是否存在 batch/capacity reuse，还是只能逐实体调用？
- command 在哪个 committed boundary 校验和生效，失败是否原子？
- worker/partition 数变化是否会改变 state、first error 或 event order？
- multi-rate/LOD 是否改变 Core safety/fidelity，还是只改变 Presentation？
- Data/Manifest 是否仍不包含 runtime handle、partition 或 Adapter entity？
- 是否有 single-thread production oracle 和 representative benchmark？
- 该结论是否需要 ADR、prototype 或新的实施 Issue？

## 10. 与 #72 和路线图的关系

#199 只完成 #72 验收中的 API 可扩展性前置切片。#72 继续保持 Backlog、Milestone N/A、G0 Pass；fidelity/target、partition/identity、deterministic scheduling、memory layout 和 Adapter/Core boundary 的完整研究仍由 #72 后续规划。

本审计不阻塞 v0.8/v0.9，也不把 v1.0 Scope TBD 自动变成城市级实现 Milestone。它只冻结一个最迟门槛：未来 Stable Runtime API 的 G1 必须引用本审计，并关闭或显式接受第 7 节待决项；完整并行、多层级或分布式实施仍需产品目标和性能证据后另立 Milestone。
