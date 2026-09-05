# TrafficWorld 串行阶段协议与内部状态分区

**状态**: Active（#581 实现五分区、嵌套聚合拆分与串行阶段借用边界）<br>
**适用范围**: `laneflow-runtime` 的道路机动车固定步进、内部状态所有权与后续等价重构<br>
**设计入口**: [#580](https://github.com/illusion-tech/laneflow/issues/580)<br>
**关联决策**: [ADR 0003](../adr/0003-runtime-tick-and-determinism.md)、
[ADR 0020](../adr/0020-compiler-owned-static-network-and-static-image.md)、
[ADR 0021](../adr/0021-city-simulation-game-traffic-foundation.md)、
[ADR 0025](../adr/0025-checked-canonical-network-and-shared-static-network.md)

## 1. 决策与权威边界

保留一个公开 `TrafficWorld`，在其内部区分世界绑定、已提交状态、派生索引、步进工作区
和管理状态。领域计算通过窄借用视图读取拍初状态，在工作区中准备结果；只有一个提交
入口能把完整结果变成新的已提交世界。串行阶段协议（Serial Phase Protocol）使用
P0～P8 表达逻辑依赖和访问权限，不要求九次遍历、九个模块或九份中间数组。

本设计收口现行暂存、校验和无可恢复错误提交段，不改变公开行为。以下文档继续唯一
规定领域算法，本文只定义它们的组合和所有权边界：

- [共享消费合同](traffic-runtime-shared-consumption.md)：世界安装、固定步长、拍初读取与公开查询。
- [Waiting 本地合同](traffic-runtime-waiting-zone.md)：membership、队列、物理顺序与观察。
- [Waiting/Conflict 联合合同](waiting-zone-conflict-right-of-way.md)：§6.3～6.9 的候选总序、
  组合资源账本、间隙、实际 crossing、车尾净空、事件和首错；其组合规则约束本协议。
- [路权策略合同](traffic-runtime-right-of-way-policy.md)：策略绑定与完整资格谓词。
- [停车合同](parking-system.md)、[快照合同](traffic-runtime-snapshot.md)、
  [修订切换合同](traffic-runtime-revision-cutover.md)及
  [观测与 Routing 合同](traffic-observation-and-routing-integration.md)：各自的命令、
  序号、发布和生命周期边界。

内部五分区和阶段视图由 `kernel/state.rs`、`admin/state.rs`、`kernel/phase.rs` 及各领域
模块实现；私有目录和格式入口见[模块边界](traffic-runtime-module-boundary.md)。不会新增公共 API、
wire 版本、crate、线程池、Rayon、锁或调度器，也不实现性能优化、第二交通执行域或
通用事务框架。运行时内部实现不得继续保留一条供切换的旧路径；对照基线留在 git 历史。

### 1.1 等价范围

比较同一共享根、配置、初始状态和命令/tick 序列，在同一运行环境中的已提交状态、
事件、最新决策、公开错误和重试/重新回放结果。内部结构地址、容器容量、grant 私有
序号和可重建索引的字节布局不属于状态等价判据；它们仍必须保持原有有效性、错误
条件和资源约束。不得扩展为跨 CPU、跨浮点实现的位级承诺。

实现 PR 必须在 GitHub 记录用于对照的完整基线提交，基线须包含正式 Conflict 集成。
分支趋势评估、已退役 `CoreWorld` 与持续移动的 `main` 名称都不能替代这个预言机。
已确认的行为缺陷按独立修复处理，再明确更新对照基线，不能夹在内部重构中静默改变。

## 2. 五分区所有权

```text
TrafficWorld
  binding:   WorldBindingState
  committed: CommittedWorldState
    stores + clocks/cursors + resource authorities + published batches
  derived:   DerivedIndexes
  workspace: TickWorkspace
  admin:     AdministrativeState
```

| 分区                  | 所有权和寿命                                                       | 固定步进的写权限                                                |
| --------------------- | ------------------------------------------------------------------ | --------------------------------------------------------------- |
| `WorldBindingState`   | 活动根/source/world identity/generation/config/policy 的一致绑定   | P0～P8 只读；恢复、切换和来源 rebase 由各自事务更新             |
| `CommittedWorldState` | 路线和车辆登记、资源权威、时钟游标、成功提交后可查询的发布结果     | 只有 P7；生命周期命令在两次 step 之间按原有原子边界写入         |
| `DerivedIndexes`      | 对同一已提交基线的 occupancy、队列定位、owner 查询和活动顺序等缓存 | 构建或失效可以写，但不能修改交通权威或提前暴露候选状态          |
| `TickWorkspace`       | 预览、候选、claim、裁决、转移、下一状态、待发布结果及复用存储      | P1～P6 准备；P7 消费/交换；失败丢弃本次逻辑结果并保留可复用容量 |
| `AdministrativeState` | 有界迁移日志及其配对世代等控制数据                                 | 本拍只通过受限日志视图参与 P7；不授予 phase 任意管理操作能力    |

“已提交”不等于“必须持久化”。发布批次属于已提交可观察结果，但既有快照不保存这些
批次；槽位世代和空闲表属于当前实例的登记状态，也不因此加入快照。反之，迁移日志
位于管理分区，却必须反映成功提交，不能当作可随意丢弃的索引。

五分区只整理世界自身拥有的数据。外部 `CutoverTransaction` 的候选及 scratch、
`ObservationExportSession` 的导出状态继续由各自对象拥有，不为填满 Admin 而移入
`TrafficWorld`，也不把候选世界混入拍初读取视图。

### 2.1 世界字段归属

下表按现行字段说明迁移边界。允许把同一责任的字段组成私有聚合；不得通过字段改名
隐藏权限，或在两个分区维护可独立修改的同一份权威。分区不改变既有稀疏、稠密和
容量策略，也不要求把与路线条目共寿命的只读编译结果再复制到一张全局表。

| 现行字段                                                                                                                | 归属                                     | 约束                                                                                                                               |
| ----------------------------------------------------------------------------------------------------------------------- | ---------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------- |
| `revision`, `source`, `world_id`, `world_generation`, `config`, `policy_binding`                                        | Binding                                  | 同一活动绑定；step 不替换根、策略或世界世代                                                                                        |
| `tick_index`, `time_ms`, `command_cursor`, `event_cursor`, `observation_state_sequence`                                 | Committed                                | 逐操作沿用原有递增规则，见 §5.3                                                                                                    |
| `routes`, `free_routes`, `live_route_count`, `live_route_edge_occurrence_count`, `live_route_conflict_occurrence_count` | Committed route store                    | 登记、槽位世代、空闲顺序与准入计数一起维护；条目内编译出现项是只读执行表示，不是共享静态根的一部分                                 |
| `vehicles`, `free_vehicles`, `live_order`                                                                               | Committed vehicle store                  | `live_order` 决定正式更新序号，不能由槽位顺序重新生成；空闲表顺序不能任意改变后续 handle                                           |
| `active_order`                                                                                                          | Derived                                  | 从 `live_order` 保序投影；成功提交后移除非 Active，失败不改变公开遍历和后续业务顺序                                                |
| `parking`                                                                                                               | Committed resources                      | 显式/虚拟占用和 bindings 的唯一停车聚合；step 只检查预约和产生到达观察，不暗中执行 park/unpark 命令                                |
| `waiting_zones`                                                                                                         | Committed resources + Derived queue view | `next_admission_sequence` 是历史权威；occupancy 是随 membership 原子维护的已提交计数；`head`/`tail` 与 queue link 是可重建定位表示 |
| `waiting_links`, `waiting_member_rows`                                                                                  | Derived                                  | 由已提交 membership、admission sequence 和 zone 得到；缓存公开查询结果时仍受同一提交边界保护                                       |
| `conflict_eligibility`                                                                                                  | Committed resources                      | 保存首次资格时钟；用拍后状态和下一时刻信号规范化后提交                                                                             |
| `conflict_arbiter`                                                                                                      | 按 §2.2 拆开                             | 禁止把整个混合聚合直接放入 Committed 后继续在 prepare 中修改                                                                       |
| `signal_aspects`                                                                                                        | Committed resources 的信号只读表示       | 可从绑定和已提交时钟导出，但成功 step 返回前必须与该时钟一致；不是独立于时钟的第二权威                                             |
| `latest_waiting_decisions`, `latest_conflict_decisions`, `latest_transition_events`                                     | Committed published batches              | 保留上次成功结果；失败不清空，生命周期命令不把自己的 record 塞入历史 tick 批次                                                     |
| `occupancy`                                                                                                             | Derived，构建暂存属 Workspace            | 查询只使用拍初车辆状态；内部构建 scratch 与已完成索引分开借用，不复制整份已提交世界                                                |
| `next_states`, `next_state_by_vehicle`, `next_signal_aspects`                                                           | Workspace                                | 预览可复用 `next_states`，正式 next state 仍从拍初状态计算；槽位索引只定位，不排序                                                 |
| `waiting_claims`, `waiting_plans`, `waiting_plan_by_vehicle`                                                            | Workspace                                | Waiting 预选、组合 claim 与车辆定位；本地可行不等于完整 grant                                                                      |
| `waiting_staged_decisions`, `staged_transition_events`                                                                  | Workspace                                | 输出暂存；实际 crossing 和最终 traversal 明确后才形成完整批次                                                                      |
| `waiting_next_counters`, `waiting_staged_occupancy`, `waiting_staged_storage_mm`, `waiting_dependencies`                | Workspace                                | 本拍计数、存储与候选依赖图事务；失败撤销暂存，不修改已提交 admission sequence                                                      |
| `conflict_candidates`, `conflict_schedule`, `conflict_candidate_cells`, `conflict_candidate_downstream`                 | Workspace                                | 候选、顺序和候选资源输入                                                                                                           |
| `conflict_cell_work`, `conflict_downstream_work`, `conflict_grants`, `conflict_motion_by_vehicle`                       | Workspace                                | 组合 reducer 工作集、grant 与 motion 定位                                                                                          |
| `conflict_next_eligibility`, `conflict_passage_transitions`, `conflict_changed_owners`, `conflict_staged_decisions`     | Workspace                                | 拍后资格、实际资源转移、日志 owner 变化集与决策暂存                                                                                |
| `migration_journal`, `migration_epoch`                                                                                  | Admin                                    | 日志内容随成功提交推进；epoch 只在武装事务时变化，不是 tick 序号                                                                   |

原任务描述中的 `waiting_staged_events` 对应的现行统一字段是
`staged_transition_events`；不恢复独立 Waiting 事件缓冲或兼容别名。

### 2.2 Conflict 和其他嵌套聚合

资源规则仍只有一个规范归约权威；拆分状态不制造多个独立 arbiter。后续实现应让
Conflict 的规则函数显式借用下列不同部分，而不是取得含全部部分的 `&mut` 聚合：

| 现行内部内容                                                                                                                                                  | 逻辑归属与处理                                                                                                        |
| ------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------- |
| `committed_cells`, `committed_downstream`，owner 的 `reservation`、已提交 cell/downstream 范围，cell 的 committed owner、occupant、clear/lag history          | Committed Conflict authority；实际资源集合只保留一个可变 owner                                                        |
| `addresses`、`owner_lookup` 的 committed 查询部分、committed owner 范围的定位和统计、committed downstream 查询索引                                            | Derived；地址由 Binding 派生，其他部分按已提交 authority 重建；内部存储顺序不进入业务总序                             |
| `staged_cells`, `staged_downstream`, `staged_grants`, `scratch_cell_indices`，cell 的 `frontier`/staged owner，owner 的 staged serial/count/start/grant index | Workspace；只能描述当前候选和已成功暂存的较早 bundle                                                                  |
| `owner_lookup` 与 `downstream_index` 的 staged 查询部分                                                                                                       | Workspace；若物理索引融合 committed 与 staged，必须分别借用只读基线和暂存 overlay，不能在失败后留下 staged 查询可见性 |
| `pending_commit` 与 crossing 的延迟整理                                                                                                                       | P7 的已验证执行记录；提交前不能把 pending crossing 当成 committed reservation；提交返回时不得遗留待完成的资源权威     |
| `next_serial`, `reservation_serial` 及其配对                                                                                                                  | 私有 capability 防陈旧 bookkeeping；保留现行生成、存活配对和耗尽语义，不参与赢家、事件或摘要排序，不加入快照          |
| `conflict_capacity`, `vehicle_capacity`                                                                                                                       | Binding 的容量约束及派生分配尺寸；不新增可独立漂移的配置来源                                                          |

具体嵌套字段也必须有明确归属：

- `cells` 中的 `zone_committed_owner`、`reservation`、`occupant`、`cleared` 和 `lag`
  属于 committed 资源表示；`zone_staged_owner` 与 `frontier` 属于 Workspace。
  `reservation_serial` 是已提交 reservation 的私有配对信息，不可作为新的权威副本。
- `owner_authorities` 按是否已有 committed reservation 区分已提交 owner 与暂存 owner。
  `owner` 和 `reservation` 的逻辑关系只有一份；`committed_cell_count`、
  `committed_downstream_claim_count`、`committed_cell_start`、
  `committed_downstream_start`、`uncleared_cell_count` 是其可重建定位/统计，随该
  owner 的提交一致更新；`staged_serial`、`staged_cell_count`、
  `staged_downstream_claim_count`、`staged_cell_start`、`staged_downstream_start`、
  `staged_grant_index` 属于 Workspace。
- `owner_lookup` 的 committed 查询部分属于 Derived，staged 查询覆盖层属于 Workspace。
  P4 只新增暂存映射；grant 校验和 crossing 提交定位 owner 时显式读取包含暂存映射的
  查询视图，不得把它暴露为 committed 查询。P7 随 reservation 的提交/释放更新
  committed 映射，并清除本拍暂存映射；失败清理撤销本拍暂存映射，保持 committed
  查询对应原基线。物理融合时分别约束只读基线与覆盖层的借用和清理；owner 行移动
  须修正对应定位，不要求复制整表或采用两张物理表。
- `downstream_index_dirty` 跟随它标记的索引部分；不能用一次 staged index rebuild
  冒充 committed 查询已更新。`next_serial` 由工作区的私有 capability bookkeeping
  持有，保留跨调用的现行生成规则；`pending_commit` 只在 P7 消费。

`ConflictCellAuthority` 和 `ConflictOwnerAuthority` 按这些职责拆解，不能只拆外层
字段。持久 reservation 的逻辑身份仍是现行 stable occurrence/owner 含义；私有 serial
不是新的交通身份。bookkeeping 可以保留复用寿命，但不得借机重置原有耗尽条件，
也不要求失败后恢复全部暂存内存的逐字节内容。

Waiting 的 `head`/`tail`、link 和 member rows 虽然可重建，也不能在错误检查前静默
修复本来应拒绝的不一致输入。P1/P2 保留既有校验位置与首错；成功提交后提供当前状态
的完整只读查询。Parking 的现行三部分聚合保留一个 writer，不因引入五分区增加
车辆槽位中的第二份可变 parking binding。

## 3. P0～P8 阶段与读写集

记 `B` 为 Binding、`C(T)` 为拍初 Committed、`D(T)` 为由该基线建立的索引、`W` 为
Workspace、`A` 为 Admin。对外命令不能插入一次 step；世界的独占借用一直保持到返回。

| 阶段                  | 读取                                                | 允许写入及产出                                                                                                 | 失败边界                                                                               |
| --------------------- | --------------------------------------------------- | -------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------- |
| P0 输入和不变量预检   | 输入、B、C(T)                                       | W 中的下一 tick/time/观测序号等标量                                                                            | 沿用 delta mismatch、Conflict invariant、tick/time、观测序号的现有先后；失败零提交     |
| P1 拍初视图与基础索引 | B、C(T)                                             | D(T) 的 occupancy/leader 查询结构、其构建 scratch；创建 `StepReadView`                                         | 构建或分配失败零提交；不提前执行本来较晚的领域检查                                     |
| P2 局部意图           | B、C(T)、D(T)                                       | `LocalIntentBatch`：有限 horizon、Waiting 求值前沿与局部运动预览                                               | Waiting 状态/路线/数值检查与预留失败零提交；预览没有提交权                             |
| P3 资源请求           | 同上及局部意图                                      | Waiting 本地预选、approach frontier、Gate regulatory records、完整组合请求与定位结构                           | 保留本地预选先于 Conflict 请求的依赖；请求和输出容量 checked；拒绝候选与 step 错误分开 |
| P4 确定性联合裁决     | C(T) 的资源基线、D(T)、请求及较早成功 staged 的资源 | `ResolutionBatch`：all-or-nothing bundles、grant/no-grant 和资源 stop；只写 W                                  | 正常 no-grant 不失败 step；结构/数值/分配等错误丢弃本拍全部结果                        |
| P5 最终运动与转移     | C(T)、D(T)、裁决及拍初信号                          | W 中最终 next states、实际 crossing、Waiting/Conflict 转移、下一信号、资格、决策、事件、停车到达观察和日志输入 | 按 §3.1 的内部依赖完成；所有可恢复失败仍在提交前                                       |
| P6 完整计划校验       | B、C(T)、W，A 的只读日志需求                        | 消费暂存视图形成 `CommitPlan`，完成提交所需验证和容量证明                                                      | 保留原有首错；不得延迟预留到 P7/P8，也不强制重复扫描整个世界                           |
| P7 单次原子提交       | 绑定同一基线的计划                                  | C(T+Δ)、必要 D 更新/失效、A 的本次日志效果及输出缓冲交换                                                       | 仅执行已预留、已验证的操作，无可恢复错误出口；见 §5                                    |
| P8 只读结果交付       | C(T+Δ) 与已提交发布结果                             | 返回已有 `StepOutcome` / 借用批次的访问能力，回收已消费的暂存                                                  | 无新的领域计算、分配失败、回调或第二次提交                                             |

索引可在首个消费者之前延迟建立；例如 approach frontier 依赖当前 Conflict 求值，
不因名字叫 Derived 就必须提前到 P1。阶段归属不授予移动检查的权限。现有函数往往
跨多个逻辑阶段；重构可先分离借用，再逐步提取函数，不需要改动合法输入的执行顺序。

### 3.1 联合裁决与转移依赖

```text
拍初 committed state + occupancy
  -> Waiting 局部预览/预选与请求
  -> approach frontier + Gate candidate
  -> 规范顺序的组合资源取得
  -> 最终 motion（所有 hard constraints 只收紧）
  -> Waiting leave/enter 及 admission sequence 暂存
  -> 下一时刻信号 + Conflict crossing/clear/release 暂存
  -> 唯一 traversal 推导 + 下一资格规范化
  -> 完整决策/事件/日志效果校验
  -> 原子提交
```

依赖图中的顺序是语义约束，不要求每个箭头都分配一份记录。特别保留：

1. P2 的预览只确定候选和有限 horizon；P5 从 C(T) 计算正式运动，不能在预览位置上
   再推进一次，也不能用前一辆车未提交位移作为后一辆车的前车权威。
2. Waiting 本地可行只产生预选；P4 以 C(T) 加较早成功 staged bundle 仲裁全部
   Waiting、Conflict、downstream 资源。未完整取得资源就不能发布 `Granted`。
3. 本拍释放不返还 Waiting capacity 给后续候选；Waiting cycle 预防和依赖图事务
   沿用联合合同。资源 slot、HashMap 遍历或执行完成顺序均不能选赢家。
4. grant 只解除其精确 Gate，不能覆盖 leader、safe speed、RouteEnd、ParkingStop
   或后续未授权 Gate。没有实际 crossing，整份 bundle 到期，不能建立 membership
   或 reservation。正式输出不能从无完整约束的预览推断 crossing。
5. Waiting 和 Conflict 先暂存真实资源变化，再唯一推导 traversal 与事件。拍初有
   reservation 的车辆可在同拍车尾净空后完成路线；同拍 enter/leave 仍消耗正确的
   admission sequence，同拍 acquire/release 即使终态为空也保留两种事件。
6. P2～P5 的运动与法规判定使用 `signal_aspects(T)`；下一信号在复用缓冲中每拍
   计算一次，拍后资格按 C(T+Δ) 的候选状态及 `signal_aspects(T+Δ)` 规范化。
   最新决策保存本拍历史判断，不按拍末信号重写；成功返回时快照已经合法。

### 3.2 现行函数到逻辑阶段

| 现行接缝                                                                                                                 | 逻辑职责                                                |
| ------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------- |
| `step_vehicles` 的输入/时钟检查、`rebuild_occupancy_index`                                                               | P0～P1                                                  |
| `prepare_waiting_step`                                                                                                   | P2 预览及 P3 Waiting 本地预选；其工作区不能越权提交     |
| `prepare_conflict_step`、candidate 构造与 acquire                                                                        | P3～P4，包含较早 staged bundle 的规范读取               |
| `stage_vehicle_transitions` 中的 active vehicle 遍历                                                                     | P5 正式 motion 和停车到达观察；没有停车生命周期隐式提交 |
| `finalize_waiting_step`、下一信号计算、`finalize_conflict_step`、`finalize_waiting_outputs`                              | P5 转移和输出形成，交织 P6 已有校验；保留首错先后       |
| `commit_conflict_transitions`、Waiting removals、车辆/日志写回、Waiting additions、`commit_conflict_step`、时钟/信号交换 | 同一个 P7 内部提交段，允许按既有依赖有序写多个数组      |
| `StepOutcome` 返回及既有 latest batch 查询                                                                               | P8                                                      |

## 4. 窄视图和计划寿命

以下是职责示意，不是新增公开签名，也不要求把 Rust borrowing 实现冻结成唯一布局：

```rust,ignore
fn build_local_intents(
    read: &StepReadView<'_>,
    output: &mut LocalIntentBatch,
) -> Result<(), StepError>;

fn resolve_resources(
    read: &StepReadView<'_>,
    intents: &LocalIntentBatch,
    output: &mut ResolutionBatch,
) -> Result<(), StepError>;

fn prepare_commit<'w>(
    read: &StepReadView<'_>,
    workspace: &'w mut TickWorkspace,
) -> Result<CommitPlan<'w>, StepError>;
```

`StepReadView` 只借用 B、C(T) 中当前阶段需要的部分及 D(T) 的只读查询；局部函数
再缩窄为车辆、路线、signal、Waiting 或 Conflict 的视图。它不能包含 `&TrafficWorld`
作为绕过边界的后门，也不提供内部可变的 committed resource accessor。需要读取
earlier-staged claims 时显式增加 W 的资源视图，不把它伪装成拍初已提交状态。

当前 `StepWorkspace` 通过只实现 `Deref` 的 `StepCommitted` / `StepDerived`
借用守卫读取基线。唯一的受限准备操作产生 `ConflictResolution`，只提供 frontier、
组合取得和丢弃暂存的方法，不提供 reservation 提交或释放方法。Conflict 的原有
预留顺序同时准备 committed 容器容量及默认空 cell，因此允许这一不改变业务权威的
存储准备；这不授予阶段任意 `&mut CommittedWorldState`，也不把可失败预留移入 P7。
`StepReadView` 的 Conflict 查询省略 W；组合裁决的查询显式包含 W overlay。

Waiting 的 occupancy/历史计数留在 C，队列首尾、link 和 member rows 位于 D；
Occupancy 完成的查询索引位于 D，桶计数及构建游标位于 W。切换与恢复同步维护两者，
保留原有不一致输入检查位置。

局部意图批次（`LocalIntentBatch`）、裁决批次（`ResolutionBatch`）与确定性提交计划
（`CommitPlan`）可以是复用数组的私有分段借用、索引范围或类型化 wrapper。逻辑阶段
允许融合：无须全世界快照、每车堆对象、复制 request 或建立通用任务图。不能让多个
可变视图别名同一份 authority；也不能只把全部 `&mut TrafficWorld` 改名成另一大视图。

`CommitPlan` 只在本次 step 的独占借用中创建和消费，绑定同一世界、世代、修订、
拍初 tick/time 和观测序号。Rust 生命周期与私有构造保证它不能被宿主持有、复制、
重复提交，不能在生命周期命令、restore 或 cutover 之后复用；不为这种内部借用
再新增公共 token、全世界哈希或持久化身份。若实现使用不带借用的私有计划，也必须
在同一未释放的 step 借用中消费，不能新增跨 step 缓存入口。

协调函数在逻辑上按阶段分借用字段。进入 P7 前结束只读阶段借用，再将所需的
committed stores、resources、published batches、derived maintenance 和 journal sink
交给唯一私有提交函数。Waiting、Conflict、Parking 计算函数都不能取得整个世界的
可变引用；公开 facade 可以保留 `step(&mut self, ...)`。

## 5. 完整 CommitPlan 与提交

### 5.1 完整性的含义

完整表示每个成功提交效果都有已验证输入和明确 writer，不表示复制整个世界。

| 计划覆盖项                        | 提交前必须证明                                                                                                             |
| --------------------------------- | -------------------------------------------------------------------------------------------------------------------------- |
| 车辆 next states 与最终 traversal | 原 handle/slot/route 有效，motion 有限且符合 hard constraints，Completed 不遗留资源权威                                    |
| Waiting 变化                      | 实际 leave/enter、membership、物理队列顺序、计数与 admission sequence 一致；同拍空终态仍保留应消耗计数                     |
| Conflict 变化                     | 精确 grant 的 crossing、passage enter/clear、reservation/downstream owner 的 acquire/release 和 lag history 闭合           |
| 拍后资格与信号                    | 共用完整资格谓词；运动依据拍初信号，资格依据拍末信号，信号缓冲与下一时钟配对                                               |
| 观察与输出                        | Waiting/Conflict latest decisions、统一 transition events 和停车到达 observations 已形成且容量已足够；事件顺序来自真实转移 |
| 时间和序号                        | 下一 tick/time/observation sequence 已 checked；其他游标按 §5.3 保持原值                                                   |
| 派生维护                          | active order、Waiting member 查询和 owner 查询能在返回前更新或按合同失效；不能把重建失败留到已提交之后                     |
| 迁移日志效果                      | 捕获相同成功转移及规范化资格所需的旧值/新值；需要的稀疏 owner 变化集已预留；有界日志溢出按 §5.3 处理                       |

验证可以嵌入现有转移遍历，并在 P6 形成完整性证明；不要求再执行一遍
`capture_snapshot`、全量恢复校验、全世界 digest 或二次资源仲裁。后续任何确需新增
全量扫描的行为变更须独立说明，不借“Validate complete plan”扩大稳态成本。

### 5.2 P7 与 P8

P7 内允许按现行依赖先提交 Conflict 转移、移除 Waiting 旧 membership，再写车辆与
日志、增加 Waiting 新 membership、更新资格/索引/批次以及时钟和信号。这里的
“单次原子提交”是一次对外不可分割且无可恢复错误的逻辑提交，不是一次 CPU 原子写，
也不要求换掉整份 world。

提交前必须完成所有实际容量计数、checked 算术、可能失败的预留、资源引用验证与
输出准备。P7 不返回新的 `StepError`，不调用用户 callback，不做 I/O 或需要回滚的
控制面操作。内部 `expect`/断言只表达已经验证的实现不变量，不构成把可恢复失败
推迟到提交的许可，也不新增 panic 后仍可继续使用半提交世界的保证。

新的 latest decisions/events、时钟与信号在 P7 作为一个结果切换。P8 只交付该结果，
不能再排序、分配、校验、重新计算事件或推进游标。现行 `StepOutcome` 的停车到达
观察载荷在 P5 准备，P8 移交已经准备好的载荷；不因本设计改动它的 API。
宿主另行请求的 full/delta observation export 仍由原有 session 管理，不自动每拍导出。

### 5.3 游标和管理数据

- 普通 successful step 增加 `tick_index`、`time_ms` 和
  `observation_state_sequence`；`command_cursor`、`event_cursor`、world generation
  和 migration epoch 不因一次 tick 统一加一。`event_cursor` 仍只计切换事件批次，
  transition event batch 继续按 tick 标记。生命周期命令和观测导出各守原有合同。
- `migration_journal` 武装后，每次成功 step 都记录已提交变化，零车辆变化仍有
  TICK 时钟帧。日志编码可在 P7 按预验证输入流式写入；不强制再保存一份全量 delta。
- 固定容量日志溢出保持粘性标记，使切换候选失败关闭，旧世界继续成功步进。
  “日志参与提交”不等于日志容量不足就回滚交通世界。日志结束前不允许候选消费半帧；
  原有 step 之间的 catch-up 调用边界保持。
- step 失败不追加日志、不改变事务配对世代或发布切换事件。snapshot/cutover 的
  prepare、catch-up、commit/retire 不成为本协议中的并行 phase，也不复用 tick plan
  代替它们各自的迁移计划。

## 6. 失败、首错和重试

P0～P6 任一 `StepError` 都保留拍初车辆/路线/资源权威、时钟、游标、绑定、迁移日志
和上一成功发布批次。本拍的 claim、grant、依赖图暂存、转移和下一状态失效。
清理必须没有可恢复错误，并恢复各工作区的可复用条件；可以留下容量、无权威的
暂存字节或标为待重建的索引，不用世界副本、snapshot 或通用 undo log 回滚权威。

分配失败后的重试可复用已增长容量，不承诺 allocator 再现同一个失败点；清除一次性
故障后，重试结果必须等于从原提交状态重新执行该输入的无故障结果。取消/故障不能
留下可被下一拍复用的 grant，也不能依赖下一次 step 才修复公开状态。

fresh replay 比较相同命令流产生的成功结果；经 snapshot 恢复的对照应先按既有规则
映射逻辑身份，再比较后续结果。latest batches 原本不入档，不能要求 restore 凭空
恢复历史批次；失败原子性检查则必须比较失败前后仍在同一世界中的已发布批次。

正常法规拒绝、容量/存储不足、Waiting cycle、gap 不足、优先级落选及未 crossing
的 grant 保留为 normal outcome，不改成 `StepError`。联合合同定义资源拒绝归因的
总序；它与使整个 step 失败的首错顺序不是同一排序。

首错沿用现有逻辑检查先后和稳定车辆/路线/实体键。P0～P8 不是新错误代码排序表：
不能把所有 invariant 前移到 P0，不能因函数提取、迭代器或并行完成顺序改变先前错误。
同一阶段有多个合法检查次序时，选择现行总序；发现当前实现与权威合同不一致时
先独立修复并更新预言机，不把问题掩盖成“等价重构”。

## 7. 后续实现的验收

验收观察逻辑结果，不要求 struct 内存或所有缓存字节一致。以下矩阵约束实现 PR，
不表示仅新增本设计文档就完成了运行时改造：

| 维度               | 必须比较的结果与代表场景                                                                                                                     |
| ------------------ | -------------------------------------------------------------------------------------------------------------------------------------------- |
| 普通成功路径       | 相同输入下的车辆状态、资源 ownership、tick/time/观测序号、信号、snapshot 和 deterministic digest；同一路线、跨边与 route end                 |
| 联合资源           | Waiting 满/可用、pure Waiting、Conflict grant/no-grant、cycle/间隙/下游拒绝、取得 grant 但 motion 未 crossing                                |
| 同拍转移           | enter+leave 的 counter、acquire+release 的两种事件、车尾净空后完成、后续未授权 Gate 的停止边界                                               |
| 信号边界           | 拍初决策与 motion 不被拍末信号重写；拍后资格可立即 snapshot/restore；失败保留已发布信号和批次                                                |
| 原子失败           | 现有分配故障点、grants 后和转移后故障、时间/计数耗尽及非法引用；对比所有 committed state、latest batches、日志和公开查询                     |
| 首错               | 代表性的双错误组合，覆盖输入/时钟、Waiting/Conflict 准备和转移校验的跨阶段先后；不要求构造所有错误的笛卡尔积                                 |
| 重试与回放         | 一次失败后重试 vs 同一拍初状态无故障执行；相同命令流的 fresh replay；snapshot/restore 后比较按现行映射归一化的逻辑身份                       |
| 管理事务           | 日志未武装/武装/溢出、空变化 tick、Waiting/Conflict 变化的 catch-up；旧世界继续和候选失败的边界不变                                          |
| 生命周期与公开视图 | route/vehicle register、replace/despawn、park/unpark、restore/cutover、观测 session 失效；free list 与 live order 不因结构调整改变实例内结果 |
| 资源成本           | 现有 Waiting/Conflict 暖机后零分配场景继续通过；其他路径无新增稳态分配/重分配或 retained 全量副本；停车到达等既有输出分配不被冒称原已零分配  |

复用现有的 `tick::transaction_tests`、信号发布失败重试、日志溢出后继续 step、
Waiting/Conflict allocation evidence，以及正式 compiled-network 行为覆盖。只补
分区和借用改造实际新增的覆盖缺口；不建立镜像实现或只验证字段搬家的测试。
保留内存账本必须随嵌套字段迁移更新，不能因更换外层结构漏记 buffer 或重复计量。

#581 的固定对照提交为 `e011745e94986a17049c66e730d54ac9fccc59f9`。
`tick::phase_equivalence` 在相同 fixture 上比较每拍状态摘要、实例句柄/顺序、决策、
事件、信号、观测序号/世代/游标及迁移日志，并检查故障重试和恢复后的继续步进。
六组对照结果保存于 `tests/fixtures/phase-protocol-e011745e.txt`；期望值来自上述
提交，同一测试输入在两边执行，不能从待测实现生成期望值。

`state::tests::complete_retained_memory_covers_warm_partitions_and_armed_journal`
检查五分区的 owner-local 穷尽计账及唯一实例总账；共享静态根另列，跨世界汇总时
去重。Conflict 分离 committed/staged 查询后，需要额外的暂存 owner lookup 和
downstream 索引容量，Waiting 首尾数组在安装/切换准备时独立分配。既有
`conflict_budget_evidence`、`waiting_budget_evidence` 继续验证暖机后的分配与重分配；
不新增领域遍历或 retained 全世界副本。

性能比较在相同构建配置和具名工作负载上进行，报告本次新增遍历、字节和分配；
已有耗时阈值及硬件角色沿用原合同。本文不预设新的百分比门槛，不以结构拆分宣称
一万/十万产品预算已经通过，也不把已知二次复杂度优化作为本设计的开工条件。

## 8. 实施和演进边界

- #581 承接状态分区、嵌套聚合与借用边界的实现，按本协议证明外部行为等价。
  如需将阶段提取拆成独立实现 Issue，应显式列出剩余验收，不能把字段包装完成
  当成整个协议实现完成。
- 仿真与管理的私有目录、format/wire 唯一入口和架构检查见
  [模块边界](traffic-runtime-module-boundary.md)；phase 不获得解析文件、切换根或其他
  管理权限，不拆 crate。
- #583 负责把现有性能研究输入重绑定到正式 `TrafficWorld`。#220 消费本协议，
  在其性能/工作负载输入就绪后设计 physical partition、halo、per-worker scratch、
  确定性合并与取消；本设计不是该并行设计的验收替代。
- 当前仅冻结道路机动车的内部边界。将来其他交通执行域可以使用不同局部状态和
  求解器；本次不创建通用 participant 基类或域调度框架。

阶段融合、缓存或存储布局可以随证据调整，已提交权威、完整组合资源、首错、输出
顺序与失败边界必须保持。1.0 前只保留一套当前合同；确需变更语义时直接修订对应
领域设计并单独验证，不增加旧/新协议选择开关。
