# Traffic Runtime WaitingZone 运行时

**文档状态**: Accepted（#282 G1，2026-09-01）<br>
**适用范围**: `laneflow-runtime` / `TrafficWorld` 的 WaitingZone 准入、物理存储、
成员关系、队列、固定步进、只读观察、快照与路网修订切换<br>
**交付边界**: 本文是 #282 当前唯一详细合同；实时交付进度以 GitHub Issue 为准

**关联文档**:

- `../adr/0019-waiting-zone-conflict-right-of-way-authority.md`
- `waiting-zone-conflict-right-of-way.md`
- `traffic-runtime-shared-consumption.md`
- `traffic-runtime-integer-geometry.md`
- `traffic-runtime-snapshot.md`
- `traffic-runtime-revision-cutover.md`
- `traffic-runtime-conflict-occurrence.md`
- `vehicle-following.md`
- `signal-system.md`
- `parking-system.md`
- GitHub: #227、#235、#281、#282、#284、#541、#544、#559

## 1. 权威与重基线

本文是 #282 WaitingZone 动态运行时已接受的当前唯一详细设计。ADR 0019 继续拥有
WaitingZone 是 Gate 有界资源、行为 authority 属于交通运行时、Adapter/Spatial
不得反推行为的架构决策。

`waiting-zone-conflict-right-of-way.md` 保存 Waiting、Conflict 与通行权的联合边界；本文
进一步冻结 #282 的本地 Waiting 动态合同。发生不一致时，#282 的字段、管线、版本与
验收以本文为准，组合仲裁以 #284 后续接受的设计为准。

当前实现基线如下：

- 唯一运行世界是 `TrafficWorld`，唯一静态输入是受检 LFCA 4 构造的
  `SharedNetworkRevision`；
- 静态路网由受检 LFCA 构造 `SharedNetworkRevision`，WaitingZone identity、
  entry/release Gate、`maxOccupancy` 与 route `WaitingOccurrence` 已存在；
- 已提交纵向状态以整数毫米、微米余数与 `mm/s` 表达；不得恢复米制 tolerance；
- `TrafficWorld` 当前只接受一个 worker，生命周期命令只在两次 `step` 之间调用；
- `ParkingBinding`、停驻/离场生命周期已经生产化；Waiting membership 必须与它正交；
- `ConflictPassageOccurrence`、`route_conflict_occurrence_capacity`、路线 conflict Gate
  ranges 与 `ConflictRuntimeUnavailable` 3A 保护已经生产化；
- LFRS 4、runtime state 4、deterministic digest 6、同修订/跨修订切换和在线迁移日志
  共同保存 Waiting 逻辑状态；
- fixed step、车辆状态与生命周期消费 route `WaitingOccurrence`，但不取得 #284 的
  downstream/conflict 组合资源。

## 2. 目标与非目标

### 2.1 目标

#282 交付以下单一生产路径：

1. 车辆在 WaitingZone entry/release Gate 之间的显式 traversal state 与正交
   membership；
2. `maxOccupancy` 和实际车辆长度/minimum gap/no-overlap 共同约束的本地物理存储；
3. zone-local `WaitingAdmissionClaim`、确定性准入与同 tick 原子提交；
4. Waiting 约束进入现有纵向 hard projection，但只收紧运动；
5. 将 Waiting 可行性与现有 spawn、completed replacement、Parking 离场/路线重绑定、
   conflict 3A 能力保护组合为同一失败关闭边界；
6. 只读 latest decision、Waiting transition event 与失败原子性；
7. 快照、恢复、摘要、切换和迁移日志中的完整 Waiting 逻辑状态；
8. 一万道路机动车产品证据与十万道路机动车 scaling 证据。

### 2.2 非目标

#282 不交付：

- ConflictZone、ParticipantStream、gap acceptance、right-of-way priority、
  `ConflictArbiter` 或 `ConflictReservation`；
- 通用 `DownstreamClearanceClaim`，或把 Waiting/Conflict/downstream 组合起来的
  `GrantResourceBundle` / 通用组合 ledger；这些由 #284 拥有；
- 跨多个 WaitingZone 的原子取得、未提交 bundle 选择或 prospective cycle prevention；
  需要这些保证的场景在 #284 合入前保持 dependency-blocked，#282 不以本地 claim
  声称网络级活性，也不承诺恢复已经形成的物理网格锁；
- Adapter mutation、debug UI、Spatial Waiting region 或从画面反推 Waiting 行为；
- 除 #541 `rebind_parking_route` 外新增通用 active route reassignment，或从 stateful
  maneuver interior 创建车辆的 bootstrap transaction；现有 lifecycle 命令必须接入
  Waiting 清理/校验，interior bootstrap 继续失败关闭；
- 删除、绕过或弱化 #559 的 `ConflictRuntimeUnavailable`；该临时保护只能由 #284 在
  正式冲突 grant/reservation 与组合 ledger 同一切片移除；
- 多 worker candidate producer、分布式/城市级预约、概率驾驶人耐心、死锁传送或
  长时域预测；
- fixed equal slot、按速度阈值推断 Waiting membership，或另一套米制物理权威。

## 3. 静态输入与路线编译

### 3.1 共享静态输入

`TrafficWorld` 只读 `SharedNetworkRevision` 中已经受检的：

- `WaitingZoneOrdinal` 与稳定 identity；
- 所属 `ManeuverPath`；
- 严格有序的 entry/release `ManeuverGateOrdinal`；
- 非零 `maxOccupancy`；
- lane length、vehicle profile length/minimum gap 与 AccessCell；
- route `ManeuverOccurrence`、`hop_gate` 和现有分段整数距离表；
- 已编译的 `ConflictPassageOccurrence`、conflict Gate ranges 与独立
  `route_conflict_occurrence_capacity`，仅作为不可弱化的相邻能力约束。

Runtime 不依赖 compiler、文件系统、Serde 或 Spatial，也不按 external string 在热路径
查找 WaitingZone。

路线注册同时编译按 route order 排列的实际 Gate hop 索引。固定步进按 cursor 定位下一
Gate，不逐车辆扫描没有 Gate 的剩余路线后缀；没有 Waiting coverage 的 Gate 仍保留
`NotRequired` 观察语义。

### 3.2 `WaitingOccurrence`

共同路线编译器继续 profile-agnostic，并为每个 Waiting occurrence 物化：

```text
WaitingOccurrence
  zone
  maneuverOccurrenceIndex
  entryHop
  releaseHop
  storageLengthMm
```

`storageLengthMm` 是 entry Gate 之后的下一 route edge 起点到 release Gate 所在
transition boundary 的 zone-local 有界整数距离。它不得使用会因较早 route prefix
溢出而拒绝合法 occurrence 的全路线 `u32` 前缀；计算复用 segmented route-distance
合同。局部跨度无法证明为 `u32` finite 时，路线注册失败关闭，不把
`BeyondFinite` 当成无限空间。

同一 ManeuverPath 上相邻 WaitingZone 可以共享 release/entry boundary；内部重叠或
嵌套继续由静态编译拒绝。路线不得终止在 WaitingZone 或 stateful maneuver interior。

### 3.3 profile-route-cursor 绑定

路线注册保持 profile-agnostic。`spawn_vehicle`、`replace_completed_vehicle` 与
`leave_parking` 新建没有既有 Waiting authority 的 `Active` 候选；在 Access、
route/cursor/progress、speed 检查之后，对 cursor 尚未越过 release Gate 的每个
Waiting occurrence 执行：

```text
vehicle.length_mm <= occurrence.storage_length_mm
```

这里证明的是空 WaitingZone 至少能容纳该车辆，不承诺 `maxOccupancy` 辆同车型一定
同时放得下；实际组合由 tick admission 检查。

若 cursor 位于包含多个 Gate 或任一 WaitingZone 的 maneuver occurrence 第一个 Gate
之后、maneuver exit 之前，则候选 `Active` 绑定原子拒绝。第一版不根据 cursor 猜测
已经发生的 Gate crossing、membership 或 admission sequence。随后仍须执行 #559 的
整车身 conflict 3A 保护；Waiting 检查成功不构成跳过
`ConflictRuntimeUnavailable` 的授权。

`rebind_parking_route` 不属于上述无 authority bootstrap。它只接受已有
`Active + Reserved`，并先执行 #541 已冻结的 current occurrence、完整物理 footprint、
前向可达与 route/access 检查。若车辆已有 `ManeuverTraversalState` 或 Waiting
membership，目标路线还必须按稳定 ManeuverPath、Gate、WaitingZone identity 与 crossing
side 对当前状态建立一一对应：

- `PreGate / Committed / Waiting` phase 原样保留，只重绑 route-local occurrence/hop；
- membership 的 WaitingZone、admission sequence、queue position 与 release Gate identity
  原样保留；occupancy、counter 和 intrusive link 不增减、不重排；
- 当前 physical cursor/footprint 必须同时满足旧/新 route occurrence，不能从目标 cursor
  猜测一次历史 crossing，也不能创建或删除 membership；
- 当前 state 映射成功后，才对目标路线中尚未持有的 future Waiting occurrences 执行车型
  可行性检查，并继续执行 #559 3A 保护。

任一映射或后续检查失败都保留旧 route/cursor、Parking binding、traversal、membership、
queue/occupancy/counter 与 command cursor。若车辆没有既有 traversal/membership，rebind
候选执行与新 Active 相同的 interior guard；因此没有命令可借 rebind 伪造 authority。

### 3.4 Parking entry 与 Waiting occurrence

`ParkingBinding` 仍是正交状态轴，但所选 parking entry 必须能在不伪造 Waiting release
或 traversal completion 的前提下到达。`reserve_parking`、`rebind_parking_route`、
snapshot restore 与 same/cross-revision cutover 都按精确 route occurrence 和 segmented
route anchor 检查：

```text
waitingEntryBoundary <= parkingEntryAnchor <= waitingReleaseBoundary
```

若所选 entry 落入任一 future 或 active Waiting occurrence 的上述区间，绑定失败关闭。
比较必须包含 route occurrence/hop，不能只比较可重复的 LaneEdge identity 或全路线
`u32` prefix。entry 与 Waiting entry boundary 相同时，`ParkingStop` 虽会在 membership
取得前截停，但车辆仍持有 `PreGate` traversal state；entry 位于 release boundary 时仍
未发生 successful release crossing。两端都不能完成 `park_vehicle`，因此均属于拒绝
区间。当前静态停车锚点的端点留白会排除精确 Gate endpoint，但 Runtime 合同不依赖该
偶然事实。

区间检查之外，route compiler/binding validator 还必须证明车辆在 exact parking arrival
提交后没有 `ManeuverTraversalState`。该判定复用同一 maneuver occurrence/Gate route
operands，覆盖位于 Waiting entry 上游但仍会保留 `PreGate` 的 parking anchor；不得只以
“尚无 membership”推断可以停车。parking entry 位于 stateful maneuver 之前，或位于
maneuver exit 已提交、traversal 已清空之后，才可能通过这项检查。

这项检查只约束实际选择的 Parking binding，不让 profile-agnostic `register_route` 全局
禁止合法路线，也不新增持久字段。失败保留旧 Parking binding、route/cursor、Waiting
membership/queue/occupancy/counter 和 command cursor。`park_vehicle` 不获得清除 Waiting
membership/traversal 的旁路；合法绑定必须在 parking arrival 前已经按正常 Gate/maneuver
生命周期清除两者，或在 traversal 尚未建立前完成停车。

## 4. 已提交状态与不变量

### 4.1 每 zone 状态

每个静态 WaitingZone 在 world 内有一个稠密状态：

```text
WaitingZoneState
  occupancy
  nextAdmissionSequence
  queueHead?
  queueTail?
```

- `occupancy` 等于全部 active semantic memberships 的数量；
- `nextAdmissionSequence` 是该 world、该 zone 的 checked `u64` 单调 counter；
- head/tail 和 per-member link 是运行时稠密索引，不是持久 identity；
- zone 数来自共享静态根，不新增 caller 配置的 WaitingZone capacity 轴。

### 4.2 每车辆状态

活动 maneuver occurrence 的车辆可以拥有：

```text
ManeuverTraversalState
  route
  maneuverOccurrenceIndex
  phase
    PreGate {
      nextGateHop
    }
    Committed {
      lastCrossedGateHop
    }
    Waiting {
      releaseGateHop
    }
  activeWaitingMembership?
    waitingZone
    admissionSequence
    releaseHop
```

当前 #282 不增加 `Clearing`。该 phase 必须与 #284 的真实
`ConflictReservation` 同时设计、进入快照并可被恢复验证；在 #282 先放置一个永远
不可进入的空变体会制造第二套不完整 authority。

membership 与“车辆是否已经停住”正交：

- `PreGate` 不得有 membership；
- 跨 entry Gate 后，仍在 zone 内移动的车辆是携带 membership 的 `Committed`；
- 只有 membership 车辆到达 release Gate 且被该 Gate 的当前硬约束阻止时才进入
  `Waiting`；因前车而停在 release boundary 之前仍是 `Committed`；
- release 条件重新允许但尚未 crossing 时，`Waiting -> Committed`，membership 不变；
- successful release crossing 才移除 membership；
- shared release/next-entry boundary 以一次事务原子替换 membership；
- maneuver exit 后清除 traversal state；Completed/Parked 车辆不得保留 traversal
  state 或 membership。

membership 与 `ParkingBinding` 也是正交状态轴：通过 §3.4 绑定检查的
`Active + Reserved` 可以继续携带 Waiting membership；预约停车不释放队列 authority。
只有真正提交 `park_vehicle`（`Active -> Parked + Occupied`）前才必须先证明
traversal/membership 已清空。
`spawn_parked_vehicle` 直接构造无 traversal/membership 的 `Parked + Occupied`；其
retained route cursor 不是 arrival anchor，也不授予道路占用，不执行 Active parking-entry
检查。恢复该状态使用同一规则；离场时才对新的 Active 候选执行完整绑定检查。
`leave_parking` 创建的新 `Active` 候选从无 Waiting membership 开始，并同时经过
Waiting 绑定与 conflict 3A 检查。

### 4.3 队列索引

每个 member 使用 vehicle-capacity 有界的稠密 previous/next link。队列按
`admissionSequence ASC`；同一路径内 no-overlap 与禁止超车使它与物理前后顺序保持
一致。跨 zone 不共享可同时生效的节点 authority。

队列 link、head/tail、occupancy 和车辆 semantic membership 必须在同一提交中改变。
任何不一致是 invariant error，不通过扫描后静默修复。快照只保存语义状态，恢复时
重建并验证索引。

## 5. Waiting admission 与本地存储

### 5.1 `WaitingAdmissionClaim`

claim 是 `TrafficWorld` 私有、tick-local、不可由 Adapter 构造的 staged capability：

```text
WaitingAdmissionClaim
  waitingZone
  vehicle
  entryHop
  releaseHop
  storageSpan
```

它只声明 WaitingZone 本地 capacity 与 storage，不声明 release Gate 后方道路净空，
也不包含 Conflict resource。#282 对每辆车每 tick 最多只为 tick-start 尚未持有的
route-anchor 最早 Waiting occurrence 生成一个 admission request/claim；即使取得该
claim 后的 unconstrained candidate 还能到达更晚 Waiting entry，本 tick 也只允许前
保险杠到达而不得跨越该更晚 entry boundary，后者到下一 tick 再求值。车辆在
tick-start 已持有旧 membership 时，shared release/next-entry 上的 next entry 是本 tick
第一个新 admission，仍可按 §4.2 原子替换 membership。

这是保守的 per-vehicle Waiting evaluation horizon，不是多资源 ledger：#282 不形成
all-or-nothing multi-zone claim set，也不在各 zone 独立归约后做重分配。两个车辆以相反
route order 争用多个短 zone 时，#282 只保证每个本地决定确定且无部分 claim rollback；
它不保证跨 zone cycle-free。需要进入前证明整条 downstream/组合资源可清空的产品场景
必须等待 #284。

claim 未伴随 successful entry crossing 时在本 tick 结束失效，不消费 admission
sequence，也不留下 committed reservation。

更晚 entry 的 horizon stop 本身是一个真实 hard projection：若 tick-start 前保险杠严格
位于该 boundary 上游、原 motion candidate 会跨越它，而 per-vehicle horizon 将
post-step 前保险杠投影到 boundary，则同 tick 产生
`VehicleWaitingZoneProjectionApplied(EvaluationHorizon)`。下一 tick 才对该 occurrence
求值；若结果为 capacity/storage no-grant，由 latest decision 记录实际资源原因，不再
补发第二个 projection event。这样既不把尚未求值的资源误报为 capacity/storage，也不
需要为连续 boundary no-grant 增加持久 suppression bit。

### 5.2 容量

对同一 zone 的稳定 reducer 只观察：

```text
tickStartOccupancy + earlierStagedAdmissionCount < maxOccupancy
```

同 tick staged release 不返还 capacity。这样一个 tick 的 winner 不依赖车辆先算还是
后算，也不会让先离开的车辆为同 tick 后算 candidate 提供旁路容量。

成功跨 entry 后，即使同 tick 又跨 release，也算 successful admission：分配 sequence，
按 route anchor 顺序产生 enter/leave event，并且本 tick 不返还容量。

### 5.3 物理存储

`maxOccupancy` 不替代物理空间。candidate 还必须证明：

- 空 zone 的 `storageLengthMm` 能容纳其 `length_mm`；
- 非空 zone 中，candidate 可在 release boundary/物理队尾之后的可用区间内安放车身；
- 与队尾之间满足作为 follower 的 minimum gap；
- committed member、tick-start occupancy 和 earlier staged Waiting claims 均不可重叠；
- leader、RouteEnd、下一 Gate 与 no-overlap 等现有 hard boundary 仍然成立。

实现使用当前 route-relative `OccupancyIndex`、compiled Waiting local span 和稠密队尾
索引；不得为每个 candidate 扫描全部 WaitingZone、构造字符串 key、遍历 HashMap，
也不得把道路上 release Gate 之后的通用下游空间纳入 #282 claim。

### 5.4 稳定顺序与 admission sequence

WaitingZone 只覆盖同一 ManeuverPath 上的物理队列，不做 #284 的 policy arbitration。
同 zone candidate 从 tick-start committed state 计算前保险杠到其 `entryHop` 的 finite
route-relative `approachDistanceMm`，并按以下键归约：

```text
(
  approachDistanceMm ASC,       // 离 entry Gate 更近的物理前车先
  vehicleUpdateSequence ASC,    // 不可能物理重合时的防御性 vehicle tie
  entryHop ASC                  // 同一车辆重复 occurrence 的 route-anchor tie
)
```

`approachDistanceMm` 必须在 Waiting claim 改变 GateStop 之前从同一 tick-start snapshot
求值；前车无 claim 时，后车不得仅因较早 spawn、较早进入 lookahead 或较小 raw handle
取得 claim。这样不会出现“后车拿到唯一 capacity、又被物理前车阻挡，而前车因无 claim
永久不能进入”的活锁，也不需要在 motion 失败后迭代转授 claim。

`vehicleUpdateSequence` 是当前 `live_order` 中的唯一位置且已进入快照，completed
replacement 保留其位置。raw `VehicleHandle`、slot/generation、worker index、提案完成
顺序或容器迭代顺序不得参与业务排序。

motion staging 完成后，对同 zone 实际 successful entry crossings 按 post-step
physical front-to-back 排序；规范总序为 physical rank、`vehicleUpdateSequence`、
`entryHop`。最后一项为 route occurrence 提供显式防御性 tie，不落到
proposal/container order。随后从
`nextAdmissionSequence` 连续 checked 分配。counter 不足以覆盖本 tick successful
admissions 时整个 step 失败，禁止 rollover、饱和或重新编号既有 members。

## 6. 固定步进与约束组合

### 6.1 管线

一次 successful step 的规范顺序是：

```text
0. preflight fixed delta、tick/time 与 observation sequence
1. 保留 tick-start committed signal snapshot(T)
2. 从 committed vehicles 重建 route-relative occupancy/leaders
3. 验证 WaitingZone state、membership、queue 与 counters
4. 刷新 maneuver/Waiting evaluation frontier、per-vehicle 最早新 Waiting entry horizon
   与 tick-start approach distance
5. 从 snapshot(T) checked 计数并预留 frontier/request/decision scratch，再生成
   regulatory decision 和 Waiting admission requests
6. 按 zone-local stable order stage Waiting admission claims
7. 构造纵向 intent，并加入 Gate/Waiting capacity/storage hard constraints
8. 与 leader、safe-speed、speed limit、RouteEnd、Parking/no-overlap 取最严格值
9. stage motion、Gate crossings、membership/queue/phase transitions
10. 按 staged crossing exact count 预留 transition/event scratch，再按 post-step
    physical order checked 分配 admission sequence
11. stage latest decisions、Waiting events 与 migration journal deltas
12. 原子提交 vehicle/zone state、tick/time、signals 与 observation sequence
```

步骤 0–11 不得修改已提交 Waiting authority。任一 allocation、算术、route invariant、
counter、journal 或 motion 错误都使 world、最新 batch、tick/time 与失败前一致。

### 6.2 约束只收紧

Waiting 只为现有 `hard_room_mm` 增加边界：

```text
finalTravelMm = min(
  current motion candidate,
  leader/minimum-gap/no-overlap,
  speed limit,
  signal/regulatory Gate,
  Waiting capacity/storage Gate,
  RouteEnd/ParkingStop
)
```

取得 admission claim 只移除对应 Waiting entry stop，不能覆盖更严格约束。无 claim、
zone 满或本地存储不足是正常 `NoGrant`，不是 `StepError`；hard projection 必须保证车辆
前保险杠不会越过未获准 entry Gate。

### 6.3 Waiting phase

#282 下 release Gate 可由现有 signal/regulatory decision 阻止；#284 后续可在同一
Gate 增加 Conflict no-grant。member 只有实际到达该 release boundary 且最终最严格
约束归因为 release Gate 时才进入 `Waiting`。`speed_mm_s == 0`、leader jam 或任意低速
阈值都不能单独建立该 phase 或 membership。

release Gate 与 leader/minimum-gap 给出的 normalized `finalTravelMm` 相等时（包括车辆
tick-start 已在 boundary、两者都给出零 travel），phase attribution 明确以 release Gate
胜出；因此 red/deny 下进入 `Waiting`。只有 release Gate 当前允许、最严格 travel 单独
来自 leader/minimum-gap 时才保持 `Committed`。这个 tie 只决定 phase/event/snapshot
归因，不放宽任一 hard constraint，也不复用 #284 的 business right-of-way priority。

phase 保存产生该 committed state 的 tick-start 约束归因。提交时信号推进到下一时点，
不追溯改写该归因。restore/cutover 校验 route、Gate、crossing side、membership 与 release
boundary 的结构一致性，不以恢复时或目标修订的当前 signal aspect 重判历史 phase。

## 7. 生命周期命令

- `register_route`：profile-agnostic 编译 Waiting local operands；任何失败不占 route
  capacity、不推进 command cursor；
- `spawn_vehicle`：执行 §3.3 可行性与 interior guard，成功后从当前 cursor 之前没有
  伪造 traversal/membership；
- `replace_completed_vehicle`：旧 Completed 必须没有 traversal/membership；新车执行
  与 spawn 相同的 Waiting 绑定检查和 #559 3A 保护，失败保留旧车；
- `reserve_parking`：先执行 §3.4 route-specific parking-entry/traversal 检查，再只改变正交
  `ParkingBinding`；不得清除或重排 Waiting membership；
- `park_vehicle`：只有 traversal/membership 已清空的 committed arrival 才能提交；不得
  以停车为由静默释放 Waiting authority；
- `leave_parking`：从无 traversal/membership 的 Parked 车辆构造 `Active` 候选，执行
  §3.3 Waiting 绑定检查与 #559 3A 保护后原子提交；
- `rebind_parking_route`：按 §3.3 保留并重绑既有 traversal/membership authority，目标
  parking entry 还必须通过 §3.4；任一条件失败都保留旧 route/cursor、Parking 与
  Waiting 状态；
- route completion：只能在 traversal state 和 membership 均已清空后提交；
- `despawn_vehicle`：同一事务摘除 membership/queue link、更新 occupancy，并释放
  Parking 绑定；成功 `VehicleDespawnRecord` 按 §8.4 同步回显可选 Waiting release，
  任一步失败不得留下半清理状态或成功 record；
- `remove_route`：现有 live-vehicle guard 继续拒绝仍被 Active/Parked/Completed 车辆或
  Waiting state 引用的路线；
- `replace_completed_vehicle`、despawn、停车、完成、路线替换/移除后的成功态都不得有
  悬空 membership、queue link 或 occupancy。

未来若新增 #541 parking rebind 之外的 active route reassignment 或 interior bootstrap，
必须独立冻结同一事务中 membership、queue、counter、event、snapshot 与 cutover 语义；
#282 不预留旁路命令。

## 8. 只读观察与事件

### 8.1 查询

公开 API 提供只读借用视图：

- `VehicleState::maneuver_traversal` / `waiting_membership`；
- `TrafficWorld::waiting_zone` 与 `waiting_zone_members`；
- `TrafficWorld::latest_waiting_decisions`；
- `TrafficWorld::latest_waiting_events`；
- `VehicleDespawnRecord::waiting_release`；不把 command transition 塞入历史 tick batch。

Adapter、scenario 与 caller 不获得 claim、counter、queue 或 phase mutation authority。
`StepOutcome` 继续只承担 tick/time；批量观察从 world 借用，避免让 `StepOutcome` 拥有
分配型载荷。

### 8.2 latest decision

Waiting 相关 record 区分：

```text
NotEvaluated              // signal/regulatory deny，未尝试资源
NotRequired               // Gate coverage 无 Waiting admission
Granted                   // 本 tick Waiting claim 完整取得
NoGrant(Capacity)
NoGrant(PhysicalStorage)
```

同一 Waiting entry occurrence 每车每 tick 至多一条 record，record 至少携带可审计的
`entryHop`/route occurrence identity；batch 规范总序为
`(vehicleUpdateSequence ASC, entryHop ASC)`，不得依赖 request/proposal 容器顺序。
record 只描述刚完成的 successful tick，不是下一 tick 的 permission lease。#284 可以
在同一 resource-outcome 面增加 Conflict/downstream reason，但不得改变 #282 Waiting
reason 的含义或让历史 record 成为 authority。

`NotRequired` 只覆盖正式 staged motion 实际 crossing 或接触的 Gate，不使用尚未施加
Waiting capacity/storage/horizon 约束的预览位置。claim 的 `Granted` 仍表示本 tick 已取得
claim，不等同于实际 successful entry。
非 Waiting entry 的 Gate（包括 release Gate）同样先按 tick-start 灯色判定：限制通行时
输出 `NotEvaluated`，允许通行且无需新 Waiting admission 时才输出 `NotRequired`。
tick 结束时的信号刷新不改写该批次的历史决定。

### 8.3 事件

#282 的 fixed-step committed transition event 至少包含：

- `VehicleEnteredWaitingZone`；
- `VehicleLeftWaitingZone`；
- `VehicleWaitingZoneProjectionApplied`，带 `EvaluationHorizon`、`Capacity` 或
  `PhysicalStorage` attribution；
- `VehicleManeuverTraversalCompleted`。

projection event 不是“本 tick 再次算出 no-grant”事件，只在可由已提交位置完整判定的
首次 boundary-contact transition 产生：tick-start 前保险杠严格位于 entry boundary
上游、原 motion candidate 会跨越该 boundary，且 post-step 前保险杠被以下任一 hard
boundary 投影到该 entry：

- §5.1 的 per-vehicle evaluation horizon；
- 已实际求值的 Waiting capacity/storage no-grant。

下一 tick 从 boundary 起步，即使 candidate 再次为正且仍 no-grant，也不重复发出；实际
资源原因由当 tick latest decision 表达。该定义不需要新增 suppression bit；位置、route
cursor 与 occurrence identity 已进入快照，因此 restore/replay 后结果一致。batch 以 tick
标记，调用方若需要耐久历史自行记录。当前 `event_cursor` 继续只表示切换事件批次，
#282 不复用或改变它的基线语义。

committed Waiting transition event batch 的规范总序为：

```text
(vehicleUpdateSequence ASC, routeAnchor ASC, eventKindRank ASC)
```

`routeAnchor` 使用精确 route occurrence、hop 与 segmented route position，不使用 raw
handle 或 producer/staging 完成顺序。同一 route anchor 的 `eventKindRank` 固定为
`VehicleWaitingZoneProjectionApplied`、`VehicleLeftWaitingZone`、
`VehicleEnteredWaitingZone`、`VehicleManeuverTraversalCompleted`；因此同一 shared
boundary 仍是 Waiting leave 后 Waiting enter，maneuver completion 最后。projection 与
同一车辆实际 crossing 正常不会在同一 entry boundary 同时成立，固定 rank 仍消除不同
reducer 实现的自由度。

### 8.4 Lifecycle command record

`VehicleLeftWaitingZone` 只表示 fixed step 中由 successful release crossing 提交的
membership transition。`despawn_vehicle` 是 step 边界的同步 lifecycle command；它移除
Active member 时不得修改“刚完成 successful tick”的 latest event batch，也不得推进只
属于 cutover event batch 的 `event_cursor`。

现有 `VehicleDespawnRecord::waiting_release` 是可选的
`WaitingMembershipReleaseRecord`：

```text
WaitingMembershipReleaseRecord?
  waitingZone
  routeAnchor
  admissionSequence
  reason = Despawn
```

payload 与 vehicle/route/Parking release 在同一 command transaction 提交，按现有
`command_cursor` / caller order 观察，并进入 migration journal 的同一 despawn record。
没有 membership 时为 absent；stale handle、invariant、cursor、allocation 或 journal 失败
时命令零副作用且不返回成功 record。Park/completion/replace 本来要求 membership 已清空，
route removal 有 live-reference guard，rebind 必须保留 authority，因此第一版只有 despawn
需要这条 command-driven Waiting release payload，不新增全局 lifecycle event backlog。

## 9. 快照、摘要与修订切换

### 9.1 快照内容

Waiting 是逻辑状态，必须进入 `CapturedSnapshot` / LFRS：

- 每车辆 traversal phase 与可选 semantic membership；
- membership 使用 WaitingZone stable identity、admission sequence 与 route occurrence
  identity，不保存 runtime handle/link；
- 每个有历史状态的 WaitingZone 保存 stable identity、occupancy 与
  `nextAdmissionSequence`；空队列但 counter 非零仍须保存；
- queue link/head/tail、route-relative occupancy、latest decisions/events 与 tick-local
  claims 是可重建或输出状态，不持久化。

跨修订成功切换使源修订 latest decision/event batch 的 route/zone anchors 失效，原子
置空这两批临时输出；需要历史记录的调用方必须在 commit 前消费。不同于生命周期命令，
跨修订不把旧 ordinal 当成新修订引用，也不为历史输出保留旧 root、强制重绑已删除路线或
增加新的迁移失败条件。同修订切换保留批次；失败或放弃切换保留源批次。恢复后的批次为空。

restore 解析稳定 identity 后重建稠密索引，并拒绝 unknown zone、route occurrence
不匹配、重复 member/sequence、sequence 不小于 next counter、phase/membership 不一致、
occupancy/count 不等、超过 `maxOccupancy`、物理顺序/空间非法或 Completed/Parked
携带 state。恢复 `Active + Reserved` 时还必须按 §3.4 重验所选 parking entry；落入
Waiting 拒绝区间的快照整体失败，不通过恢复时清除 reservation 或 membership 修复。

### 9.2 版本

当前唯一生产版本轴为：

- `formatVersion=4`；
- `runtime_state_version=4`；
- deterministic state digest version=6。

实现只保留当前 writer/reader；旧 v3 输入明确失败关闭，不提供 v3→v4 reader、
转换器、迁移 shim、feature flag、双读或双写。摘要 version 6 纳入 Waiting semantic
state、zone counter 与 queue order，不纳入派生 links 或 latest output batch。

### 9.3 同修订与跨修订切换

- same-revision cutover 必须逐项保留 Waiting 逻辑状态、counter 和未来结果；
- online migration journal 的 tick delta 必须覆盖 vehicle traversal/membership 变化与
  zone counter/occupancy 变化；command delta 同步保存 despawn 的 Waiting release reason，
  候选 world 重放后摘要相等；
- cross-revision 通过 WaitingZone stable identity、route/maneuver/Gate occurrence
  rebind；active zone/gate 被移除或改变、目标 `maxOccupancy` 小于已提交 occupancy、
  新 storage/profile 组合不再可行时失败关闭；
- same/cross-revision 候选 world 都按 §3.4 重验每个 `Active + Reserved` 的目标 route
  occurrence；parking entry 落入 Waiting 拒绝区间时整体切换失败，不发布半重绑状态；
- 不得因目标修订缺少 Waiting metadata 而静默清除 membership/counter，也不得在
  in-flight transaction 中暂时关闭 Waiting 行为；
- 任一 occupancy/member 或 `nextAdmissionSequence != 0` 都是必须恒等迁移的 Waiting
  逻辑状态；target 缺少其 stable WaitingZone identity 时，整个跨修订事务按 #302
  “不可映射即整体失败”关闭，即使该 zone 在静默点为空也不得丢弃历史 counter；
- 从未产生逻辑状态的 base-only 空 zone 不进入持久 Waiting state；target-only zone 从
  occupancy/counter 零值开始。已注册路线仍须在 target 重编译并重验全部 future
  occurrence，不能用“当前无 active member”跳过缺失 metadata；
- target-only zone 的 `(entryHop, releaseHop]` 内若已有 `Active` cursor 而无对应
  membership，切换失败；不得自动补造 admission 或以零 occupancy 接受区间内车辆。
  `Parked` 的 retained cursor 不代表进入，不适用这项 Active 约束；
- restore 与 cutover 必须继续核对 `route_conflict_occurrence_capacity`、重建
  `ConflictPassageOccurrence` 并对全部候选 `Active` 车辆执行 #559 3A 保护；Waiting
  状态迁移成功不得绕过 `ConflictRuntimeUnavailable`。

## 10. 内存、性能与执行计划

- per-zone state 按共享静态 WaitingZone 数量在 install 时稠密分配；
- per-vehicle traversal/link 与“每车至多一个最早新 admission request/claim”以
  `vehicle_capacity` 为上界；同一车辆仍可因已有 membership leave、新 membership
  enter、same-tick enter+leave、maneuver completion 等产生多个 decision/transition/
  event，因此这些输出 scratch 不得套用“一车一条”的上界；
- frontier/request enumeration 与 motion staging 分别得到各 scratch batch 的 exact
  checked record count；在任何已提交状态改变前，对 claim/decision/transition/event
  batch 执行 checked `try_reserve` 并复用高水位。预留失败使整个 step 原子失败；这些
  当 tick 实际 count 是各 batch 的有限上界，不新增调用方可配置的 Waiting token
  budget，也不按理论全路线笛卡尔积常驻预留；
- successful steady tick 在 warm-up 后 Waiting phase heap allocation count 必须为零；
- 当前只实现一个 worker；数据布局和 stable reducer 不得阻止未来并行 producer，
  但 #282 不创建线程池、锁、CAS winner 或 worker-specific state；
- 一万道路机动车在项目基线硬件/协议下报告 Waiting 增量 phase p50/p95，沿联合设计
  保持 p95 目标 `<= 1 ms/tick`、硬门槛 `<= 4 ms/tick`；
- 十万道路机动车执行相同 correctness/determinism 合同，报告 elapsed scaling、
  retained bytes/vehicle、candidate/member/claim visits；它是 scaling 证据，不为
  达成墙钟数字而牺牲微观 Waiting 语义或提前实现多 worker。

## 11. 错误与诊断

正常 outcome：

- signal/regulatory deny；
- Waiting capacity full；
- Waiting physical storage insufficient；
- claim 因更早稳定 winner 不可得；
- member 正常等待或本 tick 未 crossing。

这些通过 latest decision/只读 state 观察，不是 `StepError`。

失败关闭错误至少覆盖：

- spawn/replacement 的 empty-zone storage 不可行；
- stateful maneuver interior bootstrap unavailable；
- reserve/rebind 的 parking arrival 仍会持有 traversal state，或 parking entry 落入
  Waiting occurrence 的 `[entryBoundary, releaseBoundary]`；该命令验证失败不是
  `StepError`，restore/cutover 遇到同一非法状态则候选 world 失败关闭；
- Waiting local span 无法证明 finite；
- admission counter exhaustion；
- membership/queue/occupancy/counter invariant 破坏；
- 快照/迁移中的 stable identity、phase 或物理状态不一致；
- 现有 `ConflictRuntimeUnavailable`，语义与优先级保持 #559 定义；
- scratch/journal/摘要等已有可失败基础设施错误。

诊断优先级在同一 Waiting admission 中固定为：regulatory deny（`NotEvaluated`），
capacity，physical storage，最后才由现有 motion reducer 报告更严格的
leader/RouteEnd/no-overlap attribution。优先级只决定观察记录，不改变所有约束取最小。

## 12. 验证矩阵

### 12.1 路线与绑定

- single/repeated ManeuverPath 与 multiple Waiting occurrences；
- entry/release local span、shared boundary、分段距离与 `BeyondFinite`；
- 短车成功、长车 empty-storage 失败，路线注册仍 profile-agnostic；
- spawn/replacement interior cursor、route terminal 与失败原子性；
- parking entry 位于 stateful maneuver 前、Waiting entry 上游但仍为 `PreGate`、Waiting
  entry boundary、内部、release boundary、maneuver exit 与 exit 后方的 route-specific
  案例：arrival 后仍有 traversal 的锚点拒绝，`[entry, release]` 必然拒绝，只有 arrival
  后 traversal/membership 均为空才接受；repeated route occurrence 必须按 occurrence/hop
  判定，reserve/rebind 失败保持旧绑定、route/cursor 与 Waiting 状态；
- Access/route/cursor/speed/Waiting 检查顺序稳定。

### 12.2 运行时

- `PreGate -> Committed -> Waiting -> Committed` 与 maneuver exit；
- entry crossing 后继续移动仍保留 membership；
- capacity 恰好满/差一辆、variable length、minimum gap 恰好相等/差一毫米；
- 同 zone 多 candidate、同 tick leave 不返还 capacity；
- 后车先 spawn、前车后 spawn，且两者同 tick 到达 frontier 时，仍由物理前车先取得
  capacity；不得因 `live_order` 形成可重复活锁；
- same-tick entry+release 与 shared-boundary leave+enter；
- 同一车辆的 candidate 可到达两个尚未持有的 Waiting entry 时，只为最早 `entryHop`
  求值/claim，并在下一 entry boundary 前停止；重复 static zone occurrence 下一 tick
  才能取得新的 admission；horizon stop 当 tick 产生一次
  `EvaluationHorizon` projection event，下一 tick capacity/storage no-grant 只进入 latest
  decision，不重复 projection；
- A 按 X→Y、B 按 Y→X 争用两个短 zone 时，不构造会 split 后整体 rollback 的 multi-zone
  claim set；验证本地结果确定，并明确该跨 zone cycle 在 #284 前不具备活性保证；
- post-step physical front-to-back admission sequence，proposal/update permutation 不变；
- counter exhaustion 全 step rollback；
- leader/red 同为 release boundary 零 travel 时 release Gate attribution 胜出；Gate allow
  且只有 leader 阻止时保持 `Committed`，不把任意 `speed==0` 误判为 Waiting；
- Parking、completion、completed replacement 与 route removal guard；
- reserve/park/leave/rebind/despawn 各生命周期边界不留下悬空 membership；已有
  traversal/membership 的 rebind 精确保留 phase、admission sequence、queue order 与
  occupancy/counter，没有 authority 的 rebind 不能 bootstrap interior。Waiting 校验不能
  绕过 `ConflictRuntimeUnavailable`；
- Active member despawn 的成功 typed result 恰好回显一个 Waiting release payload，按
  command cursor 观察且不改 latest tick event batch/`event_cursor`；无 membership 时 absent，
  失败不产生 record；
- failed step 的 vehicle/zone/decision/event/tick/time 全部不变。

### 12.3 持久化与切换

- capture/encode/restore/replay pointwise equality；
- queue link 在不同 local handle/slot 下重建，semantic digest 相等；
- malformed/unknown/duplicate/over-capacity/physical-invalid snapshot 拒绝；
- same-revision cutover 与不切换 world 后续逐 tick 相等；
- online journal 覆盖 enter/leave/wait/resume/counter；
- cross-revision stable rebind 成功，以及 zone/gate/storage/capacity drift 失败关闭；
- restore 与 same/cross-revision cutover 对 arrival 后仍有 traversal 或落入
  `[entry, release]` 的非法 parking-entry/Waiting 组合失败关闭，且不自动清除
  reservation、traversal 或 membership；
- 空 zone 但 `nextAdmissionSequence != 0` 时，target 删除其 identity 必须整体失败，
  不得以无 active member 为由丢弃 counter；
- v3 snapshot 明确拒绝，v4 closed-shape extra fields 明确拒绝；
- restore/cutover 同时覆盖 Waiting 4/4/6 状态与 #559 conflict occurrence 容量/3A 保护。

### 12.4 规模与观察

- latest decision 的五类 Waiting outcome、按
  `(vehicleUpdateSequence, entryHop)` 的稳定 record order 与下一 tick 不可复用；
- transition event 按
  `(vehicleUpdateSequence, routeAnchor, eventKindRank)` 全序；多车辆 producer/staging/
  container permutation 得到逐项相同 batch。首次从 entry 上游因 evaluation horizon 或
  实际 capacity/storage 投影到 boundary 发一次，连续 no-grant 及 capture/restore 后下一
  tick 不 spam，失败零事件；
- 单车 same-tick enter+leave、shared-boundary membership replacement 与 maneuver
  completion 的 scratch 条目数可以大于
  `vehicle_capacity`，扩容发生在提交前且 allocation failure 保持世界不变；
- warm-up 后稳态零 heap allocation；
- 10k 产品门与 100k scaling/retained-memory 报告；
- #544 `WAITING-RELEASE` 在 #282 完成后从 `dependency-blocked` 切为真实 runnable
  capacity/FIFO/entry/release oracle，不读取 workload 名称选择 Runtime 行为。

## 13. #282 / #284 边界终稿

项目 Owner 于 2026-08-30 决定：

> #282 只拥有 WaitingZone 本地 admission/storage claim，通用
> downstream-clearance 和组合 ledger 归 #284。

因此：

- #282 可以拥有 zone-local stable reducer、每车每 tick 最早一个新 Waiting claim 的
  staging、membership 与物理队列；
- #282 不创建 `DownstreamClearanceClaim`、Conflict claim、reservation 或通用
  `GrantResourceBundle`；
- #282 不承诺跨 WaitingZone cycle-free；跨 zone 原子取得、未提交 bundle 选择与
  prospective cycle prevention 必须由 #284 的通用组合 ledger 一并设计，依赖这些保证的
  场景此前保持 blocked；
- #284 组合资源时消费 #282 已提交的 Waiting state，并在自己的 G1 中扩展
  resource outcome/ledger；不得把 Waiting capacity、counter 或 queue mutation
  authority 搬出 `TrafficWorld`；
- #282 完成不会声称无保护转向或完整 keep-clear 已生产化；这些仍受 #283/#284
  原生依赖约束。
