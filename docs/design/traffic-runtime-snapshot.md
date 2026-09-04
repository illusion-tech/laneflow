# 运行时快照

**文档状态**: Accepted（#302 G1；停车 #540；路线冲突出现项 #283；Waiting #282）<br>
**最后更新**: 2026-09-03<br>
**适用范围**: 版本化 Runtime Snapshot 的设计原则、绑定集、保存/恢复语义、回放、确定性状态摘要与跨修订迁移入口<br>
**关联文档**:
[`../adr/0020-compiler-owned-static-network-and-static-image.md`](../adr/0020-compiler-owned-static-network-and-static-image.md)（§12；static image / receipt 条款已被 ADR 0025 §8 取代，origin 以 LFCA 为准）、
[`../adr/0021-city-simulation-game-traffic-foundation.md`](../adr/0021-city-simulation-game-traffic-foundation.md)、
[`../adr/0028-integer-millimeter-traffic-geometry.md`](../adr/0028-integer-millimeter-traffic-geometry.md)、
[`../adr/0029-retire-precompiled-static-route.md`](../adr/0029-retire-precompiled-static-route.md)、
[`traffic-runtime-revision-cutover.md`](traffic-runtime-revision-cutover.md)、
[`traffic-observation-and-routing-integration.md`](traffic-observation-and-routing-integration.md)、
[`retire-precompiled-static-route.md`](retire-precompiled-static-route.md)、
[`shared-static-network.md`](shared-static-network.md)、
[`parking-system.md`](parking-system.md)

本文是 ADR 0020 §12（按 ADR 0025 §8 修订后语义）的 #302 G1 设计。路线与车辆的
快照表示沿用 ADR 0029 §6 已冻结形状；容器 schema、字段映射与测试构造由 G2
落定。

本文的 #303 观测/Routing 恢复与回放接缝已由 #303 G1 接受，与 #302 合同共同构成
当前唯一实现权威。

> **实现状态**：当前唯一生产合同与实现均为 Runtime Snapshot v5；停车使用 tagged
> `ExplicitSpace | VirtualPool` binding，并保存 Reserved/Occupied 状态、所有 Reserved
> binding 的精确 entry route occurrence 和 virtual reservation 的 semantic entry anchor；
> Waiting 保存 traversal、semantic membership 与非零历史 admission counter；
> Conflict 保存 first eligibility、Clearing reservation/passages/downstream authority 与
> 非 `NoHistory` lag reference，occupancy 从整车位置和 passage 锚点重建；
> `WorldConfig` 保存独立的路线边出现项与路线冲突出现项容量。旧 schema、reader 与
> writer 不属于当前生产入口，不提供双读或自动迁移。

## 1. 问题与设计立场

#284 的策略绑定、Conflict reservation 与历史状态增量见
[`traffic-runtime-right-of-way-policy.md`](traffic-runtime-right-of-way-policy.md) §6、§8
（Accepted）。当前已实现策略绑定、Conflict eligibility/reservation/Clearing、
downstream authority 与 lag history 的持久化和同/跨修订迁移；生产 fixed-step 接线仍由
W7 完成。

城市游戏需要存档、恢复与回放。Runtime Snapshot 是每世界可变状态的独立版本化
制品：不进入 LFCP 发布链，真实性由宿主存档清单在对象外绑定（ADR 0021）。
设计立场：

- **保存逻辑状态，不保存机器**：持久的是稳定引用、快照局部标识与整数毫米值
  （ADR 0028）；进程句柄、槽位、generation、密集序号、数组布局不持久，恢复
  时新分配句柄。
- **派生即不存**：凡确定性派生的状态不入快照——信号灯色由 `time_ms` 与共享根
  program 派生，车道占用索引每 tick 从已提交状态重建。
- **单一时点**：保存在固定步进安全边界原子捕获一个快照点，编码只读该不可变
  捕获，世界随后可恢复步进；不存在跨提交状态的混合捕获。

## 2. 版本轴与绑定集

版本轴分离：容器 `formatVersion` 与被绑定事实的版本（runtime 版本、
static-contract versions、`networkRevisionDerivationVersion`、identity registry
revision）不混用单一数字；未知版本值失败关闭。

必绑：LFCA origin；`CommittedNetworkSource`（editable 世界 = committed
`RoadEditingState`；runtime-only 世界 = `PublishedLfcaReference`）；上列版本；
世界身份（快照局部）；`tick` / `time_ms` / 输入命令与已提交事件双游标（切换
事务的 `worldBinding` 需要双游标基线）；全部每世界可变状态（§3）；
`WorldConfig` 与 `WorldPolicySelection`。必填 `world_policy` 使用封闭 tag：
`NotRequired = 1` 且无 policy，或 `Pinned = 2` 且带 policy StableId；0、未知 tag、
缺失/多余身份及未知 table 字段都拒绝。恢复把选择传入唯一 install 入口，
拒绝根内未知策略与不合法 NotRequired；规则内容及法规来源由绑定的 LFCA 确定，
不保存 dense ordinal、解析表或步长派生间隙。

`WorldConfig` 分两类，恢复语义不同：**行为语义配置**（`fixed_delta_time_ms`
与语义容量）参与恢复核对——`fixed_delta_time_ms` 必须精确相等；语义容量只许
放大（不得小于容纳快照状态所需）；容量差异不改变恢复合法性，但**精确回放的
对拍前提是语义容量一致**——不一致容量下的重放分歧按失同步信号处理，不判为
实现缺陷（容量差异会改变重放中生命周期命令的成败）；**可重建执行计划
字段**（worker 数等）不参与——执行计划按当前硬件重建、精确结果与 worker 数
无关（ADR 0021）。

`route_edge_occurrence_capacity` 与 `route_conflict_occurrence_capacity` 都是行为语义
容量：前者统计全部存活动态路线边序列 occurrence，后者统计路线重编译所得全部
`ConflictPassageOccurrence`；两者都保留重复出现。恢复配置必须分别容纳快照路线表
重建出的 exact 总数，检查点后精确回放还要求与原配置一致。

世界世代是活动进程内失效状态，不是被恢复的逻辑内容：`WorldGeneration` 不进入
快照；同进程原位恢复必须从活动 world checked 取得下一世代，重新安装的世界从
初值建立且不得与仍存活的旧 `world_id`/会话并存。恢复实现不得从快照复制旧世代，
也不得用初值回绕伪装一次原位恢复。

禁绑（出现即拒绝）：runtime handle / slot / entity generation、`WorldGeneration`、
密集序号、共享静态数组、`EditableDiffBase`、partition / worker assignment、数组地址 / layout /
capacity、调用方自有 seed/随机流（宿主存档清单绑定；Runtime 无自有随机流）。

## 3. 每世界可变状态

| 状态        | 快照表示                                                                                                                                                                                                                                                                                                                                                                                  |
| ----------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 路线表      | ADR 0029 §6 形状：`snapshot_route_id` + 有序边 `StableId128` 序列（允许重复边）；机动、等待区与冲突 passage 出现项均由目标共享根重新编译，不入快照                                                                                                                                                                                                                                        |
| 车辆        | ADR 0029 §6 形状 + 每车唯一 `snapshot_vehicle_id`：所属 `snapshot_route_id`、`route_edge_index`、`progress_mm` / `carry_um` / `speed_mm_s` / `status`；profile / class 等静态绑定用 `StableId128`                                                                                                                                                                                         |
| 停车状态    | 保存 `Reserved | Occupied` + tagged target；显式 target 保存 `ParkingSpace StableId128`，虚拟 target 保存 `ParkingFacility StableId128`；Reserved 保存 entry route occurrence，所属 route 即同一车辆的 `snapshot_route_id`，Reserved virtual 另存 `(entry LaneEdge StableId128, progress_mm)`；counts/capacity 不作为第二 authority 入档                                                  |
| Waiting     | 每车保存可选 maneuver traversal 与 semantic membership；每个有逻辑历史的 zone 保存 `WaitingZone StableId128`、occupancy 与单调 `nextAdmissionSequence`。queue link、tick-local claim 与 latest output batch 不入档                                                                                                                                                                        |
| Conflict    | 每车保存可选 exact Gate occurrence eligibility（含 `firstEligibleTick`）或 `Clearing` reservation（owner 由车辆记录、acquired tick、passage stable locator/route occurrence 证明、committed downstream 物理区间并集）；每个非 `NoHistory` cell 保存 tagged `ActualClear | CutoverFloor` 与时间。occupant/cleared、profile 派生的跟车间隙、frontier、tick-local grant 和内部 serial 不入档 |
| live 顺序   | 车辆 `snapshot_vehicle_id` 的规范排序序列                                                                                                                                                                                                                                                                                                                                                 |
| tick / 时钟 | `tick` / `time_ms` / 输入命令游标 / 已提交事件游标                                                                                                                                                                                                                                                                                                                                        |

车辆是运行时实体，没有 `StableId128`：它以 `snapshot_vehicle_id` 持存并被
停车、live 序引用；静态实体（边、profile、class、停车位和停车设施）用 `StableId128`。
快照局部标识只在单个快照内稳定，恢复经 `SharedIdentityIndex` 与局部标识表
重建。一维数值全部整数毫米 / 微米 / `u32` mm/s，无浮点字段。

arrival 不保存独立布尔值。恢复后只从同一 Reserved binding 与已保存车辆状态派生：
route handle 必须是该 binding 所属 route，`route_edge_index` 必须等于 exact entry
occurrence，`progress_mm` 必须精确等于解析后的整数毫米 anchor，且
`speed_mm_s == 0`、`carry_um == 0`。任一项不同都不是 Arrived；reader 不允许用同一
LaneEdge、距离容差或“已经越过”补成到达。virtual entry 的 semantic anchor 恢复为目标
修订内的 typed selector后，仍必须满足上述同一谓词。

Reserved entry 还必须从保存的 vehicle cursor 前向可达。对同一 route，entry occurrence
更大即合法；occurrence 相同时，entry progress 必须更大，或 progress 相同且
`carry_um == 0`。因此“同一 `progress_mm` 但保存了非零 carry”既不是 Arrived，也不是
仍可到达的 reservation，reader 必须拒绝，不能清零 carry 修复。

**#303 G1 已接受合同**：观测导出/admission session/基线、动态成本快照、未注册
候选与成本 provenance 是调用方拥有或可重建的交付状态，不进入快照。候选注册
成功后形成的普通路线只按上表路线表示保存。任何成功恢复建立新的世界世代/
观测 stream；恢复前的 session 与
未注册候选全部 stale，不因同修订恢复而复活；新 stream 从初始
`observationStateSequence` 开始（Routing 合同 §5）。路线表恢复还必须经唯一
`register_admitted_route` 规范化入口核对修订、稳定标识、
两个路线 occurrence 容量与共同路线编译器；超限零部分恢复。

输入命令游标只统计成功应用的生命周期命令；direct / candidate / admitted 三类路线
注册在共同提交点恰好计数一次，保存不计数。游标以 checked arithmetic 推进；耗尽时
所有路线、生成、替换与停车入口都在修改状态前失败关闭，不允许回绕或饱和伪装成功。

## 4. 容器

封闭契约：size-prefixed FlatBuffers、file identifier `LFRS`、`formatVersion = 5`；
schema 位于 `schemas/runtime-snapshot/v5`；生成物隔离于私有 wire package（沿
`laneflow-road-editing-wire` 先例）。读取 verifier-first：语义 lowering 前完成
长度、基数与版本预检。确认 `formatVersion = 5` / `runtime_state_version = 5` 后，
reader 还逐 table 拒绝超过 v5 schema 对该 table 登记字段数的 vtable 槽；该上界只在
上述两个 version-5 gate 成功后选择。FlatBuffers verifier
本身允许旧 reader 忽略未知字段，不能替代禁绑字段的封闭性检查。发布链的自定义
规范制品仍只有 LFCA / LFSM / LFSD / LFCP；
快照不是发布对象，不要求跨实现字节规范序，只要求逻辑确定性（§6）。
G2 writer 入口为 `encode_lfrs(&CapturedSnapshot)`：它只读边界捕获，不回读活动
world；输出为带 `LFRS` file identifier 的 size-prefixed buffer，必需空表也编码为
存在的空 vector。

当前实现只读取 `schemas/runtime-snapshot/v5`；旧 reader/writer 不保留。恢复先解析
全部 target/anchor StableId，再验证
显式排他性和每设施 `reserved + occupied <= virtual_capacity`，成功后才发布 world。

## 5. 保存与恢复

- 保存只接受活动聚合的 `CommittedNetworkSource`；working / candidate 与
  `EditableDiffBase` 不入档。候选准备期的保存仍只捕获旧聚合（切换文档 §8）。
- **恢复对称原则**：editable 与 published 两类来源都先重建可信根再恢复——
  editable 从 committed `RoadEditingState` 重编译，published 经
  `PublishedLfcaReference` 重新认证。同修订判据 = `NetworkRevisionId` +
  `networkRevisionDerivationVersion` + 契约版本精确相等（与切换文档 §3 一致）；
  origin 字节差异仅承担来源审计，published 重发布的摘要错配在判据满足时允许
  恢复。资产缺失、重编译失败、版本不匹配或缺 `EditableDiffBase` 对应关系失败
  关闭。
- 发布资产启用道路编辑须另带 committed 道路状态并完成同修订
  root / source / diff-base rebase；成功前不得启用编辑。
- **捕获与编码的失败关闭分工（#532）**：`capture_snapshot` 的全部容量按计数
  可失败预留（路线/车辆/边表、句柄查表 HashMap 与 source 克隆的 asset key），
  预留失败返回 `SnapshotCaptureError`，世界无感知、宿主清点后可直接重试；
  `LFRS` wire 编码含第三方 FlatBufferBuilder 的内部分配，失败关闭化不可达，
  保存路径的失败关闭由捕获侧承载（编码输入是已物化的有界快照）。
- **完整性原则**：恢复的状态必须满足与 `install` / spawn 命令路径同一的不变量
  集。语义 lowering 拒绝：重复局部标识、悬空引用、live 序不是活跃车辆的精确排列、
  parking 状态矩阵不一致（只允许 `Active + None/Reserved`、`Parked + Occupied`、
  `Completed + None`）、Reserved binding 所属 route/entry occurrence 与 vehicle record
  不一致或 entry 不再前向可达、车辆值不变量破坏（`carry_um` 越界、进度超
  边长、超速、profile 与 class 不一致、活跃车辆物理重叠），以及时钟不变量
  破坏（`time_ms` 必须等于 `tick × fixed_delta_time_ms`，checked arithmetic
  验证——不可达时钟对会派生出错误的信号与回放分歧）。任一违反零部分恢复。
- 路线只从稳定边序列重编译一次；staging world 先累计 exact edge/conflict 两类出现项
  总数并核对快照与目标容量，再以保存的 `carry_um` 直接提交最终车辆状态。每个
  `Active` 都必须满足冲突仲裁能力缺席期的 3A 车尾清除谓词；`Parked/Completed`
  不经过瞬时 `Active`。任一失败不发布 world。
- 普通恢复严格核对已保存的 Waiting traversal/membership；缺失状态不得自动补成
  `PreGate`。首次 Waiting 覆盖的零历史初始化只属于显式跨修订切换（切换文档 §3.3）。
- 恢复成功后世界处于快照 `tick` 边界的一致状态；`install` 核对与
  `register_admitted_route` 规范化路线重建沿现行消费契约执行。
- W5 恢复出的 Conflict eligibility/reservation 是可继续保存、同修订换根、跨修订迁移或
  显式 despawn 的权威状态；W7 生产 tick 接线前，含这些 live authority 的 `step` 以
  `ConflictRuntimeUnavailable` 在任何运动/Waiting 写回前原子失败，避免首拍丢失
  `Clearing`。仅含 lag history、没有 live authority 的世界不受该保护影响。
- G2 fresh restore 入口 `restore_lfrs` 的顺序不可绕过：调用方 wire / asset-key
  上限 → size prefix / file identifier → 有界 FlatBuffers verifier → 版本/绑定/配置与
  表基数预检 → 标识、引用、排列、停车和值不变量 lowering → 局部 world 路线/车辆/
  占用重建。任一失败只丢弃 staging；成功才返回 world 与快照局部路线/车辆 ID 到新
  句柄的映射。fresh restore 从初始 `WorldGeneration` / 初始观测 stream 建立；调用方
  不得让同一 `world_id` 的旧 world 或旧 session 并存。Published 目标允许同修订
  重发布的 digest / length 不同，但目标 source 与目标根、快照 source 与快照根各自的
  `NetworkRevisionId` 必须闭合。

## 6. 回放与确定性状态摘要

- 输入命令序列由宿主记录并按序重放（术语表：输入命令序列）；Runtime 不新增
  隐藏输入权威。
- **回放身份**：命令目标用耐久身份（快照局部标识或宿主自有 ID），不用进程
  句柄；恢复经「局部标识 → 新分配句柄」映射重提交；检查点之后由命令新建的
  实体，其耐久 ID 由命令载荷自带。
- **G2 身份接缝**：`CapturedRoute` / `CapturedVehicle` 以只读访问器公开局部 ID 与
  逻辑字段；`RestoredSnapshot::route_mappings` / `vehicle_mappings` 返回按局部 ID
  升序的完整新句柄表，单 ID 查询只作便利入口。宿主据此把自己保存的耐久 ID
  重绑到本次进程句柄；Runtime 不保存或解释宿主 ID。
- **#303 G1 已接受合同——候选路线回放**：`register_candidate_route` 的世界世代、
  观测与成本绑定不进入命令序列。成功准入后只记录规范化路线注册命令：宿主耐久
  路线 ID、当时的路网修订标识/派生版本和有序 LaneEdge `StableId128`。恢复后核对
  当前修订、解析稳定标识并经同一 `route_edge_occurrence_capacity` 与唯一路线编译器
  的 `register_admitted_route` 入口重建，再把新句柄映射回宿主 ID；不重新执行
  Routing，不重放 stale admission。
  跨修订不匹配必须经 #302 受信任迁移策略显式迁移命令稳定引用，否则拒绝。
- **确定性状态摘要（术语表）**：当前唯一算法为 SHA-256，域分隔字节是
  `laneflow:runtime-state-digest:v1\0`，随后写 `u16` 摘要版本 `7` 与 `u16`
  `runtime_state_version` `5`；整数均小端。顶层依次写：`world_id`、语义
  `NetworkRevisionId`、六轴静态契约版本、行为语义配置（vehicle/route、edge/conflict
  两类 occurrence 容量与 fixed dt；不写 worker）、tick/时间/双游标、策略选择
  （`u8`：NotRequired=0，Pinned=1 后紧跟 16 字节 policy StableId）、按 stable
  WaitingZone identity 排序的 `occupancy + nextAdmissionSequence` 记录，再写路线实例
  分组多重集和 live 更新序列。每条记录写 `u64 byteLength + bytes`，每个集合先写
  `u64` 数量。路线实例
  分组 = 路线记录 + `u64` 绑定车辆数 + 按记录字节升序的绑定车辆记录多重集；路线记录 =
  `u64 edgeCount` + 有序边 `StableId128`。车辆记录依次写 route index / progress / carry /
  speed、status 封闭 `u8`（Active=1/Parked=2/Completed=3）、profile/class
  `StableId128`，随后写下列 parking binding：

  ```text
  parkingBindingPresence:u8       // 0=Absent, 1=Present
  if Present:
    bindingState:u8               // 1=Reserved, 2=Occupied
    targetKind:u8                 // 1=ExplicitSpace, 2=VirtualPool
    targetStableId:StableId128    // ParkingSpace 或 ParkingFacility
    if bindingState == Reserved:
      entryRouteEdgeIndex:u32     // vehicle 所属 route 内的精确 edge occurrence
    semanticEntryPresence:u8      // 0=Absent, 1=Present
    if semanticEntry Present:
      entryLaneEdgeStableId:StableId128
      entryProgressMillimetres:u32
  ```

  `entryRouteEdgeIndex` 当且仅当 Reserved 时存在，并按 `u32` 数值区分同一 LaneEdge 在
  route 中的不同 occurrence；它必须解析到该 target 的 entry。`semanticEntry` 当且仅当
  `Reserved + VirtualPool` 时存在；ExplicitSpace 和 Occupied VirtualPool 均禁止该字段。
  parking binding 后继续写可选 maneuver traversal（route occurrence、ManeuverPath
  stable identity、`PreGate/Committed/Waiting` phase 与 phase Gate stable identity）
  和可选 Waiting membership（WaitingZone、maneuver occurrence、entry/release Gate
  stable identity 与 admission sequence）。parking binding 不重复写 route handle/ID：车辆所在 route group 就是 Reserved binding
  的 route，改变 vehicle 的 route group 已会改变摘要。Parked 车辆必须是 Occupied
  binding，Reserved 车辆必须是 Active，Completed 必须 unbound。counts/capacity 从
  绑定与共享修订派生，不进入摘要。这样只改变 target tag、Reserved/Occupied 状态、
  route occurrence 或所选 virtual entry 的两个世界一定产生不同摘要。

  路线内容由所属分组承载，车辆记录不内嵌。分组多重集按完整分组字节升序；绑定分布
  形状因此可区分（分挂与同挂的 remove 语义不同），内容与绑定均相同的实例可交换
  （全部后续命令行为一致）。
  摘要计算的全部缓冲按计数可失败预留（#532：记录表/分组表用「按局部 ID
  排序的扁平表 + 二分查找」承载原 `BTreeMap` 语义——`BTreeMap` 无可失败预留
  API，输出字节不变）；预留失败返回 `SnapshotDigestError`，无副作用、可重试。
  等价类边界：同内容实例间的**归属**差异（哪条实例拥有哪份绑定集合）是实例
  重标记轨道上的同一逻辑状态，摘要不区分——区分它只能依赖槽位派生的局部
  ID，与下述「局部 ID、句柄与表排列变化不影响摘要」直接冲突（restore 的
  Parked-先 staging 本身就会重排局部 ID）。其可观测面（如对特定实例的
  `remove_route` InUse/成功）由宿主耐久 ID 映射与命令结果比对承接，检查点
  比对可纳入身份映射；失同步显形因此可能延迟到首个路线目标命令之后，首个
  分歧区间未必包含归属置换时刻，根因定位归宿主映射比对。
  live 序按运行时更新顺序逐位写「车辆记录 + 所属路线记录」：跨路线换序可
  区分（pose 批次按 live 序产出，顺序可观测）；所属路线内容相同的同值车辆
  换序保持等价（有序对内容相同，全部后续命令行为一致）。这样局部 ID、句柄
  与表排列变化不影响摘要，但多重度、车辆-路线绑定与 live 顺序仍被保留。LFCA exact-byte digest/length、Published 审计来源、
  worker、`WorldGeneration`、观测 session 不入摘要。同一逻辑状态在任何合法容器编码
  与恢复句柄分配下摘要相等；切换事务的期望值从源捕获独立计算，只允许 target origin
  替换与首次 Waiting 覆盖的零历史 `PreGate` 规范化（切换文档 §3.3/§5），不省略
  traversal 摘要字段，也不从候选动态状态取值。
- 检查点（checkpoint）是回放序列中的快照锚点，与命令游标共同界定重放区间；
  G2 对拍点冻结为 `(command_cursor, tick, deterministic_state_digest)`；宿主从检查点
  依序重放并比较各点，首个不等点与上一个相等点构成首个分歧区间。失同步只诊断，
  不自动纠偏、不重启世界，处置归宿主。

## 7. 跨修订迁移入口

跨修订恢复必须由受信任 `NetworkRevisionCutoverDescriptor` 驱动、经 LFSD 显式
迁移（切换文档 §3）；旧密集序号不得直接解释为新修订实体，稳定引用经
`SharedIdentityIndex` 完整重建。同修订换根（含重发布制品）走切换事务的
`same_revision_restore` 路径。

跨修订恢复对 Reserved target 采用与在线 cutover 相同的 entry 规则：virtual
reservation 的已保存 semantic entry 必须 exact 存在；explicit reservation 从同一
`ParkingSpace` 的目标静态 entry 重新解析，并相对已保存 vehicle cursor 重做前向可达
判定。显式 entry 向前移动可恢复，移到 cursor 后方则整次失败；reader 不倒车、teleport
或自动改派另一 entry。Occupied binding 不保留旧 entry，仍按目标修订验证 target 与
离场静态闭合。

## 8. G1 预算与度量

度量协议与切换文档 §9 相同：同机描述性基线；1.0 前可依据可复现证据收紧、放宽
或重构，影响合同或验收结论时返回 G1。维度：快照制品
exact bytes、save 停顿（墙钟与主线程）、load 停顿（editable 重编译与
published 认证分别度量）、恢复峰值内存、保存期间对稳态 tick 的干扰。

### 历史切片 B 初值（v1 published fresh restore）

以下是切换前 v1 的同机描述性基线，不得作为当前 v5 exact bytes 或性能结论复用：
`LF-P100-REF-01`（2026-08-28，rustc 1.98.0，release）。固定
workload `signalized-corridor-v1` = `v0.2-signalized-corridor.lfca`（exact
420,332 bytes，28 条 catalog 路线、2 车辆、`4 ms` 固定步进），安装后运行 64 tick；
快照点 `tick = 64`、`command_cursor = 30`。以下数字均为初值，不是产品 Pass 阈值。

证据按仪器隔离：`snapshot_budget_evidence.rs` 用 `stats_alloc` 量捕获/编码/恢复账本
并硬断言保存前后稳态 tick 分配账本相等（实测均为零）及保存后的确定性；`snapshot_peak_evidence.rs` 是只有一个
测试的独立 DHAT integration binary，profiler 只包围一次 fresh restore，量恢复调用
新增堆块的实际高水位；`snapshot_wall_clock_evidence.rs` 不插桩且 `#[ignore]`，量
release 墙钟中位和后台编码竞争。复现命令：

```text
cargo +1.98.0 test --release --locked -p laneflow-runtime --test snapshot_budget_evidence -- --nocapture
cargo +1.98.0 test --release --locked -p laneflow-runtime --test snapshot_peak_evidence -- --nocapture
cargo +1.98.0 test --release --locked -p laneflow-runtime --test snapshot_wall_clock_evidence -- --ignored --nocapture
```

| 维度                 | 切片 B 初值（release，LF-P100-REF-01）                                                                                                                                                                                                          |
| -------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 快照制品 exact bytes | LFRS `3,248` bytes                                                                                                                                                                                                                              |
| save 主线程停顿      | 边界 `capture_snapshot` 中位 `0.002 ms`；账本 34 次分配 / 3,435 bytes，返回时净 live `2,955` bytes。随后 `encode_lfrs` 可移到后台，中位 `0.0072 ms`；账本 34 次分配 + 12 次重分配 / 9,480 allocated bytes，返回时净 live `7,448` bytes          |
| published load 停顿  | 目标 Published 根/source 已由调用方取得后，`restore_lfrs` 内绑定认证 + verifier + 语义 lowering + fresh world 重建整次中位 `0.063 ms`；账本 386 次分配 + 13 次重分配 / 23,240 allocated bytes，返回时净 live `18,256` bytes                     |
| editable load 停顿   | **尚未登记，不视为已满足**：当前没有 committed `RoadEditingState` 生产来源变体；随该类型落地后单列重编译与恢复，不用 published 数值代替                                                                                                         |
| 恢复峰值内存         | DHAT 增量堆实际高水位 `19,240` bytes / 272 blocks；调用返回时 `17,920` bytes / 266 blocks。输入 LFRS 与既有共享根/source 在 profiler 前准备，不重复计入增量                                                                                     |
| 保存期稳态 tick 干扰 | 保存前后各 32 tick 的分配账本均 0 次 / 0 bytes；后台连续执行 4,096 次 encode 时，128 tick 墙钟中位 `0.001 ms`，与无竞争基线 `0.001 ms` 相同（比值 `1,000,000 ppm`）；同 tick 序列最终确定性状态摘要相等。CPU 干扰数值仅描述本机，不作跨机硬断言 |

## 9. Runtime Snapshot v5 的 G2 边界与必测义务

v5 的 schema 与版本轴到字段映射已在
`schemas/runtime-snapshot/v5/README.md` clean-generate 并逐项绑定本文 §3–§6；摘要输入的
精确规范化序列化见 §6。历史版本的 G2 证据不能自动替 v5 通过；不受当前字段集改变的
义务继续作为回归 oracle。

必测义务：save → load exact oracle（逻辑状态含双游标与 `time_ms` 全等，句柄
不比对）；检查点 + 命令重放逐点相等与失同步定位；`fixed_delta_time_ms` 不一致
拒绝；语义容量按 §2 判据（恢复只许放大，精确回放对拍要求容量一致，不一致时
分歧按失同步信号处理）；worker 数差异不影响 exact 语义；容器拒绝面（未知版本 / 长度 / 基数 / 禁绑
字段）；完整性拒绝面（标识 / 引用 / 排列 / 停车绑定 / 值不变量 / 时钟不变量）；两类来源
恢复端到端与 published 同修订重发布恢复；边界捕获拒绝跨提交状态混合；候选
准备期保存只捕获旧聚合。路线 edge/conflict occurrence 容量 max/max+1 均须恢复原子；
检查点后成功候选注册按规范化命令重放且不调用 Routing/旧 cost binding。parking
状态矩阵、Reserved route ownership、entry occurrence/edge 闭合与
前向可达单项反例（含同 `progress_mm` 非零 carry）；跨修订 virtual semantic entry exact
重绑，以及 explicit entry 前移允许/移到 cursor 后方失败；任一失败零部分恢复。冲突
出现项不得入档；恢复须证明 exact 重建计数、3A 的一微米边界、保存 carry 生效以及
Completed 不经过瞬时 Active。捕获与摘要的预留失败注入须证明失败关闭（世界无感知）、清点后重试得到
同一快照/摘要；save 路径的失败关闭由捕获侧承载（编码边界见 §5）。

Conflict downstream 区间不是 route occurrence 的第二份持久索引。writer 先从
reservation 的 exact route/Gate/passages、车辆全长与当前边长重建物理区间并集，要求与
committed 热状态相等，再按 `(edge StableId, startMm, endMm)` 写入。reader 先检查这一
wire 规范序，解析到当前根后按 ordinal 排序，再从 reservation 证明独立重建并逐项比较。
循环路线中同一物理边的重叠 occurrence 可以合并；合并区间没有唯一 route hop，禁止用
“Gate 后第一个同边 occurrence”补造来源。owner 和最小跟车间隙分别由车辆记录及 profile
派生。已清 reservation cell 必须存在同地址 `ActualClear`；缺行和 `CutoverFloor` 均拒绝。

### 当前 production v5 证据与遗留边界

下表说明当前 v5 exact head 的功能证据；tagged target、Reserved route occurrence、
virtual semantic entry、两类路线 occurrence 容量、资源守恒、未知 parking 枚举和
Waiting traversal/membership/counter、Conflict eligibility/reservation/lag、digest 往返均纳入
v5 测试。

| 义务                        | 当前事实                                                                                                                                                                                                                                                                                                                           |
| --------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| save → load exact oracle    | 已覆盖：完整逻辑状态、双游标、`time_ms` 与局部 ID → 新句柄映射；句柄值不作 oracle                                                                                                                                                                                                                                                  |
| 检查点回放 / 首个失同步区间 | 已覆盖：宿主耐久 ID 重绑、检查点后新实体 ID、已准入路线稳定边序列重放；逐点 `(command_cursor, tick, digest)` 相等，偏移 spawn 命令定位首个分歧区间                                                                                                                                                                                 |
| 配置判据                    | 已覆盖：fixed dt 不等拒绝，vehicle/route/edge occurrence/conflict occurrence 四类语义容量缩小拒绝/放大允许，保存 worker 与目标 worker 差异不影响恢复；容量不同不冒充 exact replay                                                                                                                                                  |
| 容器与完整性拒绝面          | 已覆盖：framing / identifier / verifier / wire 与 asset-key 上限、format/runtime/静态版本、未知 vtable 槽/枚举、必需字段、标识/引用/live 排列/停车/数值/时钟/Active 重叠；错误只返回失败，不暴露 staging                                                                                                                           |
| Published 来源              | 已覆盖：端到端 fresh restore；同语义修订、不同 asset key / exact-byte digest / length 的已认证重发布来源允许恢复                                                                                                                                                                                                                   |
| Editable 来源               | **尚未覆盖，不视为已满足**：当前没有 committed `RoadEditingState` 生产来源变体；类型落地后必须补重编译 + `EditableDiffBase` 对应关系 + 端到端恢复                                                                                                                                                                                  |
| 边界捕获 / 候选准备期保存   | v5 保持结构闭合：`capture_snapshot(&self)` 与所有提交入口 `&mut self` 不能在 safe Rust 中交错。切换候选在同步 `&mut self` 调用内局部持有、无可并发观测的半提交 world，调用前捕获旧聚合、成功返回后捕获新聚合；未来异步候选形态必须补可交错定向测试                                                                                 |
| occurrence max / max+1      | 已覆盖：edge 与 conflict 两类总 occurrence 正好等于保存/目标上限时恢复成功；max+1 分别以 `RouteEdgeOccurrences` / `RouteConflictOccurrences` 上限错误失败；实际 conflict 总数由完整 staging 路线表重建后核对                                                                                                                       |
| 冲突能力保护                | 已覆盖：保存的微米 carry 参与车尾位置；clearance 前一微米拒绝、相等允许；Completed 直接恢复最终态，不经过瞬时 Active；失败零发布                                                                                                                                                                                                   |
| Conflict 持久状态           | 已覆盖：firstEligibleTick 的 None/tick 0 区分、Clearing owner/passages 与可重建 downstream 物理并集往返、1 mm 改写/缺区间拒绝、ActualClear tick 0、cleared cell 历史类别闭合、悬空 locator/错误 occurrence/重复及 future history 拒绝；same-revision 原样保持，cross-revision 使用最终 T_commit floor 并在连续再次切换时保留原基准 |
| 快照五维预算                | Published 初值已登记于 §8；editable load 初值随 Editable 来源生产化补齐                                                                                                                                                                                                                                                            |
