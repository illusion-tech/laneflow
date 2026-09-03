# ADR 0019：WaitingZone、ConflictZone 与车辆级通行权 authority

**状态**: Accepted（架构决策与 #282 G1 详细设计）<br>
**日期**: 2026-09-01<br>
**适用范围**: 多阶段 ManeuverGate、WaitingZone、ConflictZone、车辆级通行权、
Traffic Runtime 安全组合、持久化与引擎边界<br>

**关联文档**:

- `../design/traffic-runtime-waiting-zone.md`
- `../design/waiting-zone-conflict-right-of-way.md`
- [`../design/traffic-runtime-right-of-way-policy.md`](../design/traffic-runtime-right-of-way-policy.md)（#284 实施细化，Review）
- `../design/traffic-runtime-conflict-occurrence.md`
- `../design/traffic-runtime-shared-consumption.md`
- `../design/traffic-runtime-integer-geometry.md`
- `../design/traffic-runtime-snapshot.md`
- `../design/traffic-runtime-revision-cutover.md`
- `../design/parking-system.md`
- `0017-static-road-junction-maneuver-and-gate-identity.md`
- `0018-multimodal-cross-section-and-access-overlay.md`
- `0025-checked-canonical-network-and-shared-static-network.md`
- `0028-integer-millimeter-traffic-geometry.md`
- GitHub: #235、#281、#282、#283、#284、#541、#559

## 背景

中国路口中的左转待转区、多阶段信号、无保护转向与条件通行需要同时表达：

- 同一 ManeuverPath 上 Gate 之间可排队的有限物理空间；
- 多条 ParticipantStream 在 ConflictZone 内的互斥或有条件兼容；
- 车辆级准入、通行权、车尾清除与下游净空；
- 固定步进下可重放、可持久化且失败原子的状态转换。

这些职责不能由 SignalController、Adapter、Spatial 几何相交或一个设施级大 solver
隐式代替。Signal 只提供许可输入，Spatial 只提供规范几何，最终车辆运动仍必须服从
Traffic Runtime 的 leader、minimum gap、no-overlap、Gate、路线终点和停车约束。

当前生产基线已经统一为：

- 受检 LFCA 构造唯一 `SharedNetworkRevision`；
- `TrafficWorld` 消费该共享根并拥有唯一动态交通 authority；
- 已提交一维运动使用整数毫米、`mm/s` 与 `carry_um`；
- `ParkingBinding`、停驻/离场生命周期已经存在；
- `ConflictPassageOccurrence` 与 #284 前的
  `ConflictRuntimeUnavailable` 全车身能力保护已经存在。

本 ADR 冻结长期 ownership；#282 的实现细节由已接受的详细设计冻结，G2 实现不得把
#284 的 downstream-clearance、Conflict 仲裁或组合 ledger 提前并入本地 Waiting claim。

## 决策

### 1. 静态 identity 与动态 authority 分层

`ManeuverPath` 继续拥有规范 lane-level traversal，`Route` 继续拥有车辆实际 edge
occurrence。Gate、WaitingZone、ConflictZone 与 ParticipantStream identity 由 LFCA
及其 `SharedNetworkRevision` 提供，运行时不得从 external string、文件系统或画面
几何重新推导。

`WaitingZone` 是同一 ManeuverPath 上严格有序 entry/release Gate 之间的一等静态
实体。它声明非零 `maxOccupancy`，但该值只是一项运营容量上限；实际可存储性仍由
zone 局部长度、车辆实际长度、minimum gap 与 no-overlap 共同决定。

`ConflictZone` 与 `ParticipantStream` 描述冲突覆盖和策略输入。路线注册将稳定
路线输入编译为 route-local `ConflictPassageOccurrence` 与 conflict Gate ranges；
`route_conflict_occurrence_capacity` 是这类派生项的独立显式容量。

所有动态 membership、claim、grant、reservation、counter、queue 与车辆 phase 都属于
`TrafficWorld`。Adapter、Spatial、Scenario 和调用方只能读取已提交观察，不能修改这些
状态。

### 2. #282 只拥有 WaitingZone 本地状态与 claim

#282 拥有：

- `TrafficWorld` 内 WaitingZone 本地 membership；
- `PreGate / Committed / Waiting` traversal phase；
- zone-local、tick-local `WaitingAdmissionClaim`；
- `maxOccupancy` 与实际车辆长度/minimum gap/no-overlap 形成的本地物理存储判定；
- occupancy、单调 admission counter 与稠密 intrusive queue；
- lifecycle、snapshot、restore、digest、journal 与 cutover 中的 Waiting 状态；
- 只读 decision、member batch 与 transition event。

`WaitingAdmissionClaim` 只证明当前 zone 的本地 admission/storage，不声明 release
Gate 后的道路净空，也不声明任何 Conflict resource。

每辆车每 tick 只为 route order 中最早一个尚未持有的 Waiting occurrence 请求新
claim。即使取得后 unconstrained travel 能到达更晚 Waiting entry，本 tick 也必须停在
更晚 entry boundary 前；后者下一 tick 再求值。#282 不形成同车多 zone 的原子 claim
集合。

### 3. #284 拥有通用组合仲裁

#284 独占以下职责：

- 通用 downstream-clearance claim；
- `ConflictArbiter`、candidate、grant 与 reservation；
- Waiting、Conflict 与 downstream 资源的组合 ledger；
- 跨 WaitingZone 的原子取得、失败回滚、未提交 bundle 选择与 prospective cycle
  prevention；
- 正式冲突能力接管后的 vehicle-level right-of-way 与 tail-clear 生命周期。

#284 可以消费 #282 已提交的 Waiting state，也可以扩展资源 outcome，但不得把
Waiting occupancy、admission counter 或 queue mutation authority 移出
`TrafficWorld`。在 #284 交付前，#282 不承诺跨 zone 活性或完整 keep-clear。

[`waiting-zone-conflict-right-of-way.md`](../design/waiting-zone-conflict-right-of-way.md)
§6 保存 #235 已接受、#284 直接消费的 current-compatible 详细合同：policy
normalization、multi-subject coverage priority、directed lower-bound ETA、top-two distinct
owner frontier、gap acceptance、mandatory downstream-clearance、single-writer 组合 ledger、
grant/reservation/Clearing、first-error、持久化与验证矩阵。ownership 重划没有重新打开
这些选择；如果未来不保留任一项，#284 必须显式返回 G1。

### 4. Waiting membership、phase 与 Parking 正交

车辆实际跨过 entry Gate 后才取得 Waiting membership；仍在 zone 内运动时 phase 为
`Committed`。只有 member 到达 release Gate 且最终最严格约束归因为 release Gate
时才进入 `Waiting`。leader jam、低速或 `speed_mm_s == 0` 本身不能创建 Waiting
phase。

release Gate 与 leader/minimum-gap 得到相同 `finalTravelMm` 时，release Gate
attribution 胜出；该 tie 只决定 phase、事件和快照归因，不放宽运动约束。

Waiting membership 与 `ParkingBinding` 是正交状态轴。`Active + Reserved` 可以继续
携带 Waiting membership；预约停车不得释放队列状态。实际选择的 parking entry 若按
精确 route occurrence/hop 落入 Waiting occurrence 的
`[waitingEntryBoundary, waitingReleaseBoundary]`，或 exact arrival 后仍会保留
`ManeuverTraversalState`，则 reserve/rebind、restore 与 same/cross-revision cutover 必须
失败关闭。entry 正好位于 Waiting entry boundary 时虽未取得 membership，仍持有
`PreGate`，不能停车。真正进入 Parked、完成、
completed replacement、despawn、路线替换或移除时，不得留下悬空 membership、
queue link 或 occupancy。Parked/Completed 不持有 traversal 或 Waiting membership；
离场产生的新 Active 候选从无 membership 开始并重新执行全部能力检查。

`rebind_parking_route` 是已有 authority 的精确迁移，不是 interior bootstrap。目标 route
只有在物理 footprint、ManeuverPath/Gate/WaitingZone identity、phase、crossing side 与
release anchor 全部一一映射时，才能原样保留 admission sequence、queue order、occupancy
和 counter；没有既有 traversal/membership 的 rebind 与新 Active 使用同一 interior guard。

### 5. 确定性与失败原子性

同一 WaitingZone 的 admission candidate 使用 tick-start committed state，并按下列
规范键排序：

```text
(approachDistanceMm ASC, vehicleUpdateSequence ASC, entryHop ASC)
```

`approachDistanceMm` 必须在 Waiting claim 改变 GateStop 之前计算。raw handle、
slot/generation、worker index、proposal 完成顺序、HashMap 顺序或调用方输入顺序都
不能决定 winner。

#284 的组合仲裁必须先消费按该键产生的 zone-local
`WaitingAdmissionEntitlement`，再使用法规/冲突全局 priority；全局 priority 不能把同
zone 物理后车提到前车之前。entitlement 不是可持久化 claim，最终
membership/queue/counter mutation 仍由 Waiting reducer 与组合事务共同提交。

motion staging 后，实际 successful entry 按 post-step 物理前后顺序分配连续
admission sequence，再以 `vehicleUpdateSequence`、`entryHop` 防御性破除不可能
出现的并列。latest decision batch 使用稳定 route order。

committed Waiting transition event batch 使用
`(vehicleUpdateSequence ASC, routeAnchor ASC, eventKindRank ASC)` 全序；同一 route
anchor 的 rank 固定为 projection、Waiting leave、Waiting enter、maneuver completion。
raw handle、worker/producer 或 staging 完成顺序不得影响公开 batch。

command-driven removal 使用另一条既有顺序轴：Active member 的 `despawn_vehicle` 在同步
`VehicleDespawnRecord` 中回显可选 Waiting release payload，按 `command_cursor`/caller
order 观察。它不改写刚完成 tick 的 event batch，也不推进 cutover `event_cursor`；因此
不新增全局 lifecycle event backlog。

claim、decision、transition 与 event scratch 必须分别以实际 checked batch count 在
任何 committed mutation 前 `try_reserve`。算术、counter、allocation、journal、
snapshot 或 invariant 失败时，world、tick/time、latest batch 与事件保持失败前状态。

### 6. 所有许可只收紧运动约束

Waiting、signal、Conflict 与 downstream grant 只允许移除其负责的 Gate stop，不能覆盖
leader、minimum gap、no-overlap、safe speed、RouteEnd 或 ParkingStop。正常
no-grant/等待不是 `StepError`。

同 tick staged release 不返还 Waiting capacity；未伴随 successful entry crossing 的
claim 在 tick 结束失效且不消费 admission sequence。successful entry 即使同 tick 又
release，也必须分配 sequence 并按 route order 产生 enter/leave event。

每车一个新 claim 不妨碍 #284 预防跨 zone cycle。#284 的 bundle 还要携带只读 Waiting
dependency footprint：既有 membership、crossing release intent，以及当前 membership
释放前必需的 downstream Waiting occurrences。single-writer reducer 对 tentative owner /
resource wait-for graph 做稳定 SCC 检查；任何 candidate 若会形成 committed cycle，按
normal no-grant 零提交拒绝。same-tick staged release 仍不返还 capacity，不能交换已提交
membership 或让同车取得多个新 claim；restore/cutover 发现既有 cycle 时整体失败关闭。

### 7. #559 临时能力保护不可弱化

在 #284 正式冲突仲裁交付前，`ConflictPassageOccurrence` 的 3A 保护继续覆盖：

- `spawn_vehicle`；
- `replace_completed_vehicle`；
- `leave_parking`；
- `rebind_parking_route`；
- snapshot restore；
- same/cross-revision cutover。

#282 不得删除、绕过、改名掩盖或降低
`ConflictRuntimeUnavailable` 的失败关闭条件。Waiting 检查成功不代表 conflict
能力可用。只有 #284 在同一切片安装正式 grant/reservation/组合 ledger 后，才能原子
移除这项临时保护。

### 8. 快照、摘要与切换是逻辑合同

#282 落地时曾执行以下一次性闭合升级；这些数字记录该决策的演进：

- LFRS `formatVersion: 3 -> 4`；
- `runtime_state_version: 3 -> 4`；
- deterministic digest `5 -> 6`。

当前唯一入口已由显式策略绑定统一推进至 LFRS 5 / runtime state 5 / digest 7，见
[`traffic-runtime-snapshot.md`](../design/traffic-runtime-snapshot.md) 与
[`traffic-runtime-right-of-way-policy.md`](../design/traffic-runtime-right-of-way-policy.md)。
只保留当前 writer/reader；旧版本快照明确失败关闭，不提供双读、双写、转换器、feature
flag 或迁移 shim。Waiting semantic membership、zone occupancy、单调 counter 与
queue order 继续参与当前摘要，派生 link 或 latest output batch 不参与。

快照按稳定 identity 保存语义状态；restore 重建稠密 link 并验证 capacity、物理顺序、
phase、route occurrence 与 Parking 状态，包括所选 parking entry 不得落入 Waiting
拒绝区间。跨修订切换按稳定 identity 重绑并执行同一检查；任何
occupancy/member，或 occupancy 为零但 counter 非零的 zone，都不能因目标修订缺失而
静默丢弃。

Waiting restore/cutover 与 #559 能力检查必须同时成功。候选世界还须重编译
`ConflictPassageOccurrence`、核对独立容量并对全部 Active 车辆执行 3A 保护。

### 9. 容量与性能边界

WaitingZone 状态按共享静态根中的 zone 数量稠密构造，不新增调用方 WaitingZone
capacity 配置轴。per-member link 与每车最早新 claim 受 `vehicle_capacity` 约束；
临时批次使用 actual checked count，不用理论全路线笛卡尔积常驻预留。

steady tick 不扫描全部静态 zone、不按 external ID 热查找、不分配每车对象；当前仍是
单 worker。数据布局和稳定 reducer 可以为未来 producer 并行保留空间，但 #282 不引入
线程池、锁或 worker-specific authority。

## 后果

正面后果：

- Waiting 本地存储可以先独立生产化，不假装完整冲突仲裁已经存在；
- #282/#284 ownership 唯一，组合 ledger 不会出现两套竞争实现；
- 物理队列顺序、snapshot/replay 与 cutover 具有统一确定性；
- #284 全局法规 priority 不会覆盖 Waiting 物理 winner，prospective SCC 在提交前阻止
  committed cross-zone cycle；
- Parking 和 conflict 已交付能力被显式组合，不会因新切片形成旁路；
- 容量按真实静态/车辆规模和当 tick 实际工作量付费。

代价：

- #284 前依赖跨 zone 原子性、完整 downstream-clearance 或无保护转向的场景仍然
  dependency-blocked；
- 每辆车每 tick 只获取一个新 Waiting claim，可能比未来组合仲裁更保守；
- #284 只预防新 committed cycle，不通过车辆换位、同 tick capacity 复用或 teleport
  恢复已经形成的物理网格锁；
- 持久化版本直接替换当前入口，开发期旧版本存档拒绝，不承担兼容转换成本；
- Waiting lifecycle、restore 与 cutover 必须增加一致性验证和失败路径测试。

## 未选择的方案

### 让 #282 同时实现 downstream-clearance 与组合 ledger

拒绝。它会把 Conflict 资源和跨 zone 活性提前塞进本地 Waiting 切片，形成与 #284
重叠的 authority，也会扩大审阅和回滚面。

### 同车同 tick 获取多个独立 Waiting claim

拒绝。各 zone 独立成功后无法保证组合原子性，反向 route order 会产生现实可达的
cycle。#282 采用最早 occurrence 的保守 horizon；真正的多资源取得由 #284 统一实现。

### 只按 `maxOccupancy` 判断容量

拒绝。不同车长与 minimum gap 会让计数未满但物理空间不足；运行时必须同时证明本地
storage 与 no-overlap。

### 通过低速或停车推断 Waiting membership

拒绝。车辆可能因 leader、signal、ParkingStop 或其他约束停止。membership 只由
entry/release Gate crossing 与提交事务改变。

### 为 WaitingZone 增加调用方容量参数

拒绝。zone 数来自静态共享根，member/link 来自 `vehicle_capacity`，临时工作量来自
checked exact count；额外轴只会制造错误配置和常驻浪费。

### 为开发期快照保留兼容层

拒绝。1.0 前只维护一套当前权威，v3 失败关闭比双读、双写和迁移 shim 更可验证。
