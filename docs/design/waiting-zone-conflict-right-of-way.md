# WaitingZone、ConflictZone 与通行权分层

**文档状态**: Accepted（#235 G1）<br>
**最后更新**: 2026-07-29<br>
**适用范围**: #235 的多阶段 ManeuverGate、WaitingZone、ConflictZone、versioned jurisdiction/right-of-way policy、车辆级 grant/reservation、确定性与 Core constraint 集成<br>
**实现状态**: #281 已交付 multi-Gate、WaitingZone static registry/Data 0.10、
Route occurrence compilation 与绑定期 capability guards；#282–#285 的
Waiting runtime、Conflict/Spatial、policy/arbiter 与组合验证尚未生产化

**关联文档**:

- `../architecture.md`
- `../roadmap.md`
- `../adr/0003-runtime-tick-and-determinism.md`
- `../adr/0005-core-identity-and-handle-model.md`
- `../adr/0006-vehicle-following-control-and-safety.md`
- `../adr/0009-signal-indication-gate-and-policy-separation.md`
- `../adr/0012-core-numeric-authority-and-presentation-precision.md`
- `../adr/0013-engine-neutral-spatial-geometry-and-length-authority.md`
- `../adr/0015-bounded-f32-canonical-spatial-frames.md`
- `../adr/0017-static-road-junction-maneuver-and-gate-identity.md`
- `../adr/0018-multimodal-cross-section-and-access-overlay.md`
- `../adr/0019-waiting-zone-conflict-right-of-way-authority.md`
- `road-junction-model.md`
- `signal-system.md`
- `vehicle-following.md`
- `numeric-representation.md`
- `spatial-geometry.md`
- `cross-section-access.md`
- GitHub: #227、#228、#229、#234、#235、#264

## 1. 目标、状态与非目标

### 1.1 目标

本文冻结：

- 同一 ManeuverPath 上多个 ManeuverGate 的 identity、顺序、coverage、StopLine
  关系与 Route occurrence 编译；
- WaitingZone 的 identity、path interval、容量、队列、进入/离开条件与 Spatial
  引用边界；
- committed `PreGate`/`Committed`/`Waiting`/`Clearing` 状态、tick-local
  release/admission decision 及黄灯、signal 切换、route completion 原子语义；
- ConflictZone、ParticipantStream、path anchor 与 Spatial geometry 的 authority；
- protected/permissive、yield/priority、directed-bound approach frontier、
  mandatory downstream-clearance、同 tick 资源 claim、grant/reservation 的分层；
- jurisdiction/compliance policy 的版本、有效区间、证据输入与可审计来源；
- signal、regulatory、conflict、leader/safe-speed/no-overlap 的约束组合顺序；
- validation first-error、event total order、失败原子性、replay determinism 与
  一万/十万性能门槛；
- Core/Data/Spatial/Adapter/API/schema/fixtures 影响矩阵与后续实施切片。

### 1.2 当前 production 基线

本文建立在以下已验证事实上：

- current Traffic loader 只接受 exact `formatVersion: "0.10"`；v0.10 schema
  已按固定 provenance 公开发布并通过 live availability 与 byte-equality 验证；
  v0.9 已公开发布并按 immutable publication contract 固定；
- current `VehicleProfile` 必填 `participantClassId`，Core 已拥有
  `ParticipantClassRegistry`、`CrossSectionRegistry` 与 `AccessRegistry`；
  AccessRule 在 normalization 期消解为 `(edge, class)` / `(path, class)` 的
  `AccessCell` 稀疏 resolved 表；
- vehicle initial/spawn/replace 与 runtime route assignment 已对当前 route cursor
  可达后缀执行 `(ParticipantClass, Route)` 静态准入校验，任一 resolved deny 都会
  原子拒绝绑定；
- current `ManeuverGate` 已包含
  `(externalId, maneuverPathId, transitionIndex, stopLineId, signalControl)`；
- current normalization 接受同一 path 上多个不同 `transitionIndex` Gate，并按
  transition 顺序提供 path range；每个 path-transition 仍至多一个 Gate；
- current `WaitingRegistry` 已验证同 path、Gate order、positive capacity、
  interior non-overlap，并允许相邻 WaitingZone 共享 boundary；
- initial/dynamic Route 注册时已编译 `ManeuverOccurrence`、`GateOccurrence`、
  `WaitingZoneOccurrence`、next Gate/exit boundary 与 empty-storage route-distance
  operands，steady tick 不匹配 path，也不缓存未经证明的累计 `f64`；
- #281 G3 retained-memory 测量对一万/十万 repeated maneuver occurrences 分别
  编译三万/三十万 Gate occurrences 与两万/二十万 Waiting occurrences；三类 route
  metadata retained bytes 为 `4,849,664` / `55,574,528`，比例 `11.4595x`，
  通过 `<= 12x` 线性门槛；WaitingRegistry retained bytes 在两档均为 `516`；
- profile-route-cursor 绑定先执行现有 Access validation，再对 release Gate 尚未越过
  的 pending Waiting 复用 overflow-safe segmented `RouteDistanceIndex` 执行
  empty-storage feasibility；finite distance 不足返回 `InsufficientStorage`，
  无法证明 finite 则返回 `StorageDistanceUnprovable`。随后由 ManeuverOccurrence
  统一拒绝 first canonical Gate 与 exit 之间的 stateful bootstrap（包括无
  WaitingZone 的纯 multi-Gate path），最后才执行 pending Waiting runtime
  capability guard；`register_route` 保持 profile-agnostic；
- current signal tick 先计算 next-time indication candidate，但当前 interval 的
  vehicle/compliance 仍读取 tick-start committed snapshot(T)，随后再构建
  occupancy/leader 与 longitudinal motion；
- current `SignalStop`、ParkingStop、RouteEnd、speed limit 与 leader/no-overlap
  已进入同一纵向约束/硬投影管线；
- current permission-aware traversal 在跨 edge 前再次拒绝 denied Gate；
- current event 顺序先按 vehicle update order 产生 projection/edge/completion
  events，再追加 Controller phase 与 SignalGroup aspect events，最后原子提交
  vehicle/signal/tick/time。

本文只能扩展这些契约，不能静默改变其既有顺序。

### 1.3 非目标

- 不在 #235 实现 Rust runtime、Traffic/Spatial schema、fixture、Adapter 或示例。
- 不交付完整专业交通工程 conflict solver、自适应信号、城市级预约或 V2X 协议。
- 不把 SignalController、Adapter、JunctionGroup、mesh collider 或二维相交升级为
  right-of-way owner。
- 不定义 pedestrian/cyclist crossing traversal；它由 #236 或后续独立 G1 冻结。
- 不扩张 #229/v0.9 protected-turning 的 production profile。
- 不替 #237 决定 lane change、lane-use state 或 resolved lane plan。

## 2. 术语与核心不变量

| 术语                | 定义                                                                                       |
| ------------------- | ------------------------------------------------------------------------------------------ |
| Gate boundary       | `ManeuverGate.transitionIndex` 指向的 path edge transition；StopLine 位于其 from-edge      |
| Gate occurrence     | 某 Route 的某 Maneuver occurrence 内、映射到 exact route transition 的 Gate                |
| WaitingZone         | 同一 ManeuverPath 上 entry/release 两个 Gate 之间可排队的路径内区域                        |
| ParticipantStream   | 穿越一个或多个 ConflictZone 的规范 path interval；v1 road-vehicle stream 引用 ManeuverPath |
| Conflict passage    | 一个 stream 穿越一个 ConflictZone 的 entry/exit anchor 区间                                |
| Gate coverage       | 从某 Gate boundary 之后到下一 Gate boundary 之前的 Waiting/Conflict admission 集合         |
| Candidate           | 已通过 indication/compliance，但尚未取得最终 conflict grant 的车辆级准入请求               |
| ConflictGrant       | 当前 tick 可跨 admission/release Gate 的车辆级许可                                         |
| ConflictReservation | 车辆跨 Gate 后直到清除覆盖 zone 前持有的互斥运行时资源                                     |
| Clearing            | 已进入受保护 conflict interval、尚未清除其最后 exit anchor 的车辆阶段                      |

长期硬不变量：

1. Route 是实际 traversal authority，ManeuverPath 是规范 traversal identity。
2. 一个 Gate 精确覆盖一个 path transition；同一 path transition 最多一个 Gate。
3. WaitingZone 与 ConflictZone 不拥有 Controller clock、Route 或车辆生命周期。
4. allow/candidate/grant 只移除对应 regulatory/conflict stop，不覆盖任何更严格
   Core safety constraint。
5. behavior conflict 只能来自 Traffic/Core 中显式声明的 stream-zone 关系，不能从
   presentation geometry 自动派生。
6. 任何 ConflictGrant 都必须先证明车辆能在不跨越下一未获准 Gate 的前提下让车尾
   清空全部 coverage zone，并原子取得 Waiting/Conflict/downstream 所需资源；
   不能证明下游安全存储或资源已被 committed/本 tick staged claim 占用时 fail
   closed。
7. steady tick 只使用 typed handle、route occurrence metadata、dense state 与
   预分配 scratch。
8. candidate 计算可以并行，但资源 winner 只由 stable-sort 后的单一
   `ConflictArbiter` 顺序决定；线程调度、锁竞争或 task 完成顺序不得决定 grant。
9. raw typed-handle 数值不得成为业务或事件 total-order tie-break。需要跨 static
   input permutation 稳定的 tie 时，normalization 按 external ID 的 ASCII byte
   order 一次编译 `canonicalRank`；steady tick 只比较该 dense rank，不比较字符串。

## 3. 多阶段 ManeuverGate

### 3.1 Identity 保持不变

沿用 ADR 0017：

```text
ManeuverGate
  externalId
  maneuverPathId
  transitionIndex
  stopLineId
  signalControl
```

不增加 `junctionId`、`movementId` 或 path interval 字段：

- Junction/Movement 可从 ManeuverPath 唯一解析；
- Gate coverage 由其在有序 Gate sequence 中的位置与 Waiting/Conflict 引用推导；
- 重复存储这些引用会产生可漂移的第二事实源。

### 3.2 顺序与 StopLine

对 path edge sequence：

```text
[entry, internal-0, internal-1, ..., exit]
```

`transitionIndex = i` 表示 `path.edges[i] -> path.edges[i + 1]`。合法范围为
`0 <= i < path.edges.len - 1`。同一路径 Gate 规范顺序只由 transition 决定：

```text
transitionIndex ASC
```

同一 transition 的 duplicate 在发布 sequence 前拒绝，不使用 raw
`ManeuverGateHandle` 为非法输入构造伪顺序。
StopLine 必须绑定 Gate transition 的 from-edge；current `EdgeEnd` location 已足以
表达路径内 Gate，不需要为多 Gate 新增 StopLine location。

不同 ManeuverPath 可以在共享 from-edge/transition 上引用同一 StopLine；每条
ManeuverPath 仍必须拥有自己的 Gate identity，避免一次 indication 绑定被误当作
多个 traversal 的最终通行权。

### 3.3 Coverage

Gate 的规范 coverage 是：

```text
coverage(gate_i) =
  gate_i boundary 之后
  到 gate_(i+1) boundary 之前（若无下一 Gate，则到 occurrence exit）
  的 WaitingZone admission 与 ConflictPassage admission
```

- 位于 coverage 内的多个 ConflictZone 必须一次原子取得 grant，车辆不得先进入
  一个 zone 再停在另一个不可安全等待的 conflict 内；
- 下一 Gate 是新的独立裁决点，之前的 signal allow 不传递到下一 Gate；
- WaitingZone entry/release Gate 自身仍各自执行 capacity/release 语义；
- 没有 Waiting/Conflict coverage 的 Gate 仍可作为纯 signal/regulatory boundary。

### 3.4 Route occurrence compilation

Route registration 在现有 `ManeuverOccurrence` 之上编译：

```text
CompiledManeuverOccurrence
  maneuverPath
  entryRouteEdgeIndex
  exitRouteEdgeIndex
  gateRange
  waitingZoneRange
  conflictPassageRange

GateOccurrence
  gate
  stopLine
  fromRouteEdgeIndex
  toRouteEdgeIndex
  coverageWaitingRange
  coverageConflictRange
```

映射公式：

```text
fromRouteEdgeIndex =
  maneuverOccurrence.entryRouteEdgeIndex + gate.transitionIndex
toRouteEdgeIndex = fromRouteEdgeIndex + 1
```

同一个 Route 重复经过同一 ManeuverPath 时，每次拥有不同 occurrence index；
车辆状态、grant、reservation 和 event 必须携带 Route handle + occurrence index，
不能只用 ManeuverPath/Gate handle 归因。

## 4. WaitingZone 静态模型

### 4.1 概念 shape

```text
WaitingZone
  externalId
  maneuverPathId
  entryGateId
  releaseGateId
  maxOccupancy
```

- `entryGateId` 与 `releaseGateId` 必须属于同一 `maneuverPathId`；
- `entryGate.transitionIndex < releaseGate.transitionIndex`；
- zone path interval 是 entry Gate 的 to-edge 起点至 release Gate 的 from-edge
  终点；两个 Gate boundary 不属于可停车 interior；
- `maxOccupancy` 是非零整数运营上限；
- Traffic wire/Core entity 不保存 Spatial artifact 内部 ID；可选 Spatial region
  通过单向 `trafficWaitingZoneId` 绑定，不改变 Core path interval；
- Junction 从 ManeuverPath 派生，query 可返回 parent handle，但 wire 不重复存储。

### 4.2 Zone 间关系

同一 ManeuverPath 的 WaitingZone：

- 可以按 Gate interval 串联；
- interior 不得重叠；
- 允许共享边界 Gate，例如 zone A 的 release Gate 同时是 zone B 的 entry Gate；
- 不允许嵌套，避免一个 occupancy 同时计入两个运营容量；
- 不得与 ConflictPassage interior 重叠；待行位置必须是安全存储区域，不是
  ConflictZone 的一部分。

zone 排序为：

```text
(
  entryGate.transitionIndex,
  releaseGate.transitionIndex,
  waitingZoneCanonicalRank
)
```

### 4.3 Capacity 与物理存储

`maxOccupancy` 只回答“最多接纳多少辆”，不能证明“当前车辆一定放得下”。实际进入
必须同时满足：

```text
occupancy < maxOccupancy
AND physical storage available
AND entry Gate regulatory candidate
AND downstream Core safety permits crossing
```

physical storage 由 current route-relative occupancy、车辆长度、minimum gap 与
release Gate 边界计算：

- zone 队尾车辆是 entry Gate 后最靠后的 occupant；
- 新车跨 Gate 后的前保险杠位置必须与队尾保持 minimum gap；
- 队首不得越过未获 release grant 的 release Gate；
- 计算仍使用 `vehicle-following.md` 的 front progress/no-overlap authority；
- 无法以 finite、确定值证明可存储时，结果是“本 tick 不接纳”，不是 runtime error。

capacity/storage 的最终判断不在 candidate 构建阶段一次性完成。stable arbitration
必须针对 tick-start committed membership/occupancy 与此前已成功 staged 的
`WaitingAdmissionClaim` 原子重查：

```text
WaitingAdmissionClaim
  waitingZone
  vehicle
  route
  maneuverOccurrenceIndex
  entryGateOccurrenceIndex
  proposedStorageSpan
```

- `occupancy + stagedAdmissionCount < maxOccupancy` 才可 claim；
- `proposedStorageSpan` 必须与 committed occupant 及既有 staged claim 保持该
  subject 的 minimum gap；
- v1 不把尚未落位的 staged admission 当作可依赖其继续移动的虚拟队尾，因此不做
  同 tick convoy/packing；与未落位 claim 的必经 span 重叠时 no-grant；
- claim 与同一 Gate 的 Conflict/downstream claim 组成一个原子 bundle；任一资源
  失败都不得留下部分 Waiting capacity；
- 车辆未跨 entry Gate 时 claim 在 tick 末失效；成功 crossing 时才原子提交
  membership/admission sequence。arbiter 不消费同 tick staged leave 腾出的 capacity，
  下一 tick 才观察 committed leave。

data 不声明固定 slot、固定车长或等距 queue point。Spatial 可以显示建议停车标线，
但 Core 不以这些 visual point 替代物理约束。

### 4.4 Queue identity 与顺序

`admissionSequence: u64` 来自每个 WaitingZone 唯一的全局单调 counter，不按
Route 或 Maneuver occurrence 分叉；否则不同 Route 对同一物理队列会产生不可比较的
sequence。一个 tick 的 motion staging 完成后，Core 对该 WaitingZone 中实际成功跨
entry Gate 的车辆按 post-step canonical physical queue order（front-to-back）排序，
再连续分配 sequence，并与 occupancy 一起原子提交。未 crossing 的 staged admission
不消费 sequence；同一位置会违反 no-overlap，defensive tie 使用唯一
`vehicleUpdateSequence`，不用 `VehicleHandle`。规范队列键为：

```text
admissionSequence ASC
```

实际前后顺序仍由 route progress 验证，admission sequence 用于公平裁决、审计与
replay tie-break，不允许车辆通过改换 Adapter actor 顺序插队。commit 前必须对
`nextAdmissionSequence + successfulCrossingCount` 做 checked preflight；sequence
overflow 使整个 step 失败且不提交 membership、counter 或其他部分状态。

### 4.5 进入与离开

进入发生在车辆前保险杠成功跨 entry Gate boundary 的同一 atomic step。若 motion
恰好停在 boundary，不计入 zone；只有 route cursor 进入 to-edge 才提交 occupancy。

离开发生在车辆成功跨 release Gate boundary：

- occupancy 在同一 step 原子减少；
- 若 release Gate coverage 含 ConflictZone，则 grant 同时升级为 reservation；
- 若 coverage 已取得 ConflictReservation，车辆直接进入 Clearing；否则保持/
  进入 Committed，而不是变为不受约束的普通 Active；
- signal 在下一 tick 变为 deny 不会把已离开 zone 的车辆退回，但后续 Gate 仍独立
  求值。

route completion 不得发生在 WaitingZone interior。运行时 route replace 若不能
证明新旧 Route 对 active zone occurrence 完全等价，必须原子拒绝；第一版 production
可以对所有 active Waiting/Clearing 状态使用 capability guard。

## 5. ConflictZone 与 ParticipantStream

### 5.1 一等 identity

```text
ConflictZone
  externalId
  junctionId

ParticipantStream
  externalId
  junctionId
  maneuverPathId
  passages[]               // 非空

ConflictPassage
  conflictZoneId
  entryAnchor
  exitAnchor
```

`junctionId` 在 ConflictZone/ParticipantStream 上显式存在，因为它们属于独立
ConflictRegistry；normalization 必须校验 stream 的 ManeuverPath 派生 Junction 与
声明 Junction 一致。这里的重复引用是跨 registry 完整性锚点，不承担第二套
ManeuverPath owner。

ConflictZone 的 participant set 只从
`ParticipantStream.passages[].conflictZoneId` 派生；normalization 按 stream
declaration order 收集唯一成员并编译 dense `zoneStreamHandles`。每个 zone 至少
必须派生出两个 stream；单 stream region 不是 conflict，可由 Spatial/authoring
自行表达。wire 不保存反向 participant list，避免同一关系出现两个事实源。

### 5.2 PathAnchor

road-vehicle stream 的 boundary 使用：

```text
PathAnchor
  pathEdgeIndex
  progressMeters
```

- `pathEdgeIndex` 索引 ManeuverPath 的规范 edge sequence；
- `progressMeters` 使用 Core canonical longitudinal distance，必须 finite、
  non-negative 且不超过对应 EdgeLength；
- 为保证单一编码，非最后 edge 的 `progress == edgeLength` 必须编码为下一 edge 的
  `(index + 1, 0)`；non-canonical 输入直接拒绝，不做静默修正；
- entry 必须严格早于 exit；
- passage 必须位于 path occurrence 内，不能跨出 ManeuverPath；
- 同一 stream 的 passages 按 entry anchor、exit anchor、
  `conflictZoneCanonicalRank` 排序；rank 在 normalization 编译，不能用 raw
  `ConflictZoneHandle` 破坏 input-permutation 语义。

Core 行为以 PathAnchor 为准；Spatial 绑定用于验证该 anchor 与 3D region 的合理
一致性，但不能改写 progress。

PathAnchor crossing、distance-to-anchor 归零、ConflictZone enter/clear 与对应事件
统一使用 Core 私有 `CONFLICT_ANCHOR_CROSSING_TOLERANCE_METERS`。它是独立的
longitudinal conflict-boundary owner，不能与 edge boundary/remainder、普通
longitudinal constraint 或 physical gap tolerance 互相别名。authoring 的 canonical
endpoint 判定保持精确结构规则，不使用该 runtime tolerance 静默改写输入。

#235 尚未生产化。后续实现必须使用整数毫米比较，不得再引入米制哨兵。进入/离开判定
的示意为：

```text
frontReached(anchor) =
  cursor is after anchor in canonical Route order
  OR (
    cursor and anchor are on the same canonical route segment
    AND cursorProgress + tolerance >= anchorProgress
  )

frontCrossedThisStep(anchor) =
  NOT frontReached(preStepCursor, anchor)
  AND frontReached(postStepCursor, anchor)

tailCleared(exitAnchor) =
  postStepFrontRouteDistance + tolerance
    >= routeDistance(exitAnchor) + vehicle.length
```

所有加法/route-distance conversion 必须 checked finite；无法证明是 invariant error，
不是把 non-finite 当作 reached。entry event 只在 `frontCrossedThisStep(entry)` 发出；
clear/release 只在 `tailCleared(exit)` 从 false 变 true 时发出，确保 tolerance 内
one-shot。ETA 的 `d <= tolerance -> 0` 与 downstream proof 的
`storageUpperBound + tolerance >= clearanceFrontTarget` 使用同一个 owner 和上述
inclusive boundary；PathAnchor wire canonicalization 仍精确，不接受近似 endpoint。

### 5.3 Gate coverage 推导

每个 ConflictPassage 的 admission Gate 是其 entry anchor 之前最近的 Gate：

```text
gate.transition boundary <= passage.entryAnchor
AND 在二者之间不存在另一个 Gate
```

如果 entry anchor 之前没有 Gate，normalization 失败；conflict traversal 不能在无
明确 admission boundary 的情况下进入 runtime。一个 Gate coverage 内的 passages
按 entry anchor 排序并整体请求 grant。

若 passage 横跨下一 Gate boundary，normalization 失败。车辆不能在尚未清除同一个
ConflictZone 时遇到新的独立 Gate，因为这会把一份 reservation 拆成相互矛盾的两次
裁决。

### 5.4 Zone overlap 与 stream order

- 同一 ConflictZone 的不同 stream 可以相交，这是其存在理由；
- 同一 stream 可以依次通过多个 zone，也允许两个 zone 的 longitudinal interval
  重叠；grant coverage 取并集并原子预留；
- WaitingZone interior 与任何 ConflictPassage interior 不得重叠；
- 同一 `(ConflictZone, ParticipantStream)` 只能有一个 passage；
- 同一 ConflictZone 内引用同一 ManeuverPath 的多个 stream 只有在 path interval
  不同且 identity 意图明确时才允许，否则视为 duplicate behavior。

### 5.5 Spatial authority

Spatial 可新增：

```text
SpatialConflictZoneRegion
  trafficConflictZoneId
  frameId
  canonical 3D region

SpatialWaitingZoneRegion
  trafficWaitingZoneId
  frameId
  canonical 3D region / markings
```

规则：

- geometry 使用 ADR 0013/0015 的 frame、有限数值与 canonical 3D authority；
- Traffic package 可无 Spatial pairing 并保持 headless Core 行为；
- Traffic entity 不保存 Spatial region ID；Spatial region 只能单向引用 known
  Traffic WaitingZone/ConflictZone，同一 Traffic entity 同类型最多一个 region；
- paired artifact 的 region 引用必须类型匹配、ID 唯一、frame 一致；缺少可选
  region 合法，不把 optional geometry 误写成完整覆盖要求；
- authoring 工具可以从中心线/region 相交提出 ConflictZone 候选，但输出必须成为
  显式 Traffic entity 后才能生效；
- 2D projection、mesh bounds、physics collider、render LOD 都不是行为输入；
- 高架、下穿等 3D 分离不得因为俯视相交而自动冲突。

## 6. Jurisdiction 与 right-of-way policy

### 6.1 统一 provenance

right-of-way policy 复用 ADR 0018 的法规身份：

```text
RegulationIdentity
  jurisdiction
  version
  source?

RightOfWayPolicySet
  externalId
  regulation
  effectiveFrom?          // inclusive ISO date
  effectiveUntil?         // exclusive ISO date
  evidenceRefs[]           // opaque audit provenance strings
  gapProfiles[]
  streamRules[]
```

- Traffic v0.9 的 AccessRegistry normalization 已保证同一 package 中所有显式
  `AccessRule.regulation` 共享一个 `(jurisdiction, version)`；#235 在该已规范化
  保证之上，将这一个 optional identity 与每个 RightOfWayPolicySet 比较，不重新
  定义 AccessRule provenance 或在 tick 扫描 raw rules；
- 显式 AccessRule regulation 必须与 RightOfWayPolicySet 共享
  `(jurisdiction, version)`；未声明 provenance 的规则继续沿用 ADR 0018 的合法
  “未指定”语义，不被本设计静默升级为必填；
- `source`/`evidenceRefs` 是 provenance，不以 URL 抓取结果作为 runtime 输入；
- `evidenceRefs[]` 的每个元素是非空、按原文保留的 opaque audit provenance
  string；数组可以为空，同一 owner 内 duplicate 拒绝。它不是 Traffic entity ID，
  不解析到本地 registry，loader/Core/Adapter 不联网验证或抓取其内容；
- `[effectiveFrom, effectiveUntil)` 为空或反向时拒绝；
- Scenario/World 初始化显式 pin policy set 与 `regulationDate`，不得读取宿主墙钟；
- 未配置日期而 package 存在多个适用候选时拒绝，不用“最新版本”猜测；
- runtime 不热切换 policy set；多版本共存/切换是独立 G1。

### 6.2 标志标线与车辆类别输入

政策可以消费已规范化事实：

- SignalAspect / Gate signal binding；
- ParticipantClass/VehicleProfile；
- Traffic sign/road marking 的稳定 external identity；
- ManeuverPath、Movement、Junction、ConflictZone/ParticipantStream；
- 当前 maneuver state、zone occupancy/reservation 与 Core kinematic snapshot。

raw mesh、texture、OCR、国家代码 if/else 或 Adapter tag 不能进入裁决。若当前
Traffic schema 尚无 sign/marking registry，而规则声明了此类引用，production
loader 必须 capability-unavailable fail closed；不得忽略证据引用后继续放行。

### 6.3 Policy rule

```text
StreamRightOfWayRule
  participantStreamId
  participantClassIds[]?
  priority                 // i32，越高越先
  yieldToStreamIds[]
  gapProfileId?
  evidenceRefs[]

GapAcceptanceProfile
  externalId
  minimumLeadGapMs        // subject latest Gate crossing 到 foe earliest entry
  minimumLagGapMs         // incompatible passage clear 后的最小冷却
  clearanceBufferMs
```

- `yieldToStreamIds[]` 只可引用与 subject 共享至少一个 ConflictZone 的 stream；
- `participantClassIds` 省略表示通用 fallback；显式提供时必须非空、无 duplicate
  且全部引用已声明 class。对具体 profile 的 rule specificity 取该 rule 中最近的
  matching ancestor；省略 selector 的 fallback 最低，不能用空数组伪装成 dead rule；
- participant class 匹配沿用 ADR 0018 已规范化的 class hierarchy，不在 tick 做
  字符串/祖先遍历；
- class specificity 优先于通用 rule，随后 `priority`；仍有矛盾规则时
  normalization 拒绝，不设隐式 deny-overrides；
- self-yield、duplicate yield edge 与严格 priority cycle 直接拒绝；
- 同 priority 的互相让行场景（如 all-way stop）不编码为 cycle，而使用相同
  priority + arrival ticket 总序；
- `yieldToStreamIds[]` 的 resolved target 必须拥有更高 effective priority；同
  priority arbitration 不使用 yield edge，避免 stable sort 在 target 之前处理
  subject；
- gap 数值必须是 `0..=2^53-1` 的整数毫秒；Core/Data 分别使用同一数值定义的
  `MAX_PORTABLE_GAP_TIME_MS`，不复用 Signal domain error/type 形成隐式耦合。

current Traffic v0.10 的静态
`conflictEligible(stream, profile)` 只消费 `AccessRegistry` 已发布的 resolved
cells，不能读取 raw rule、重复层级匹配或在 tick 重新组合：

```text
pathCell =
  AccessRegistry::path_access(stream.maneuverPath, profile.participantClass)

edgeCells =
  AccessRegistry::edge_access(edge, profile.participantClass)
  for every physical edge from the admission Gate to the last passage exit

conflictEligible =
  pathCell is not Decided { effect: Deny, .. }
  AND every edgeCell is not Decided { effect: Deny, .. }
```

`AccessCell::Unconstrained` 与 `Decided { effect: Allow, .. }` 都属于 eligible；
只有 `Decided { effect: Deny, .. }` 排除该组合。current v0.10 public vehicle/route
绑定入口已经拒绝 route suffix 上的静态 deny，因此运行时 active vehicle 不应再次
遇到该 deny；这里的静态 eligibility 用于编译 policy totality、排除不可能出现的
stream/profile 组合和验证 authoring coherence，不是第二套 route-access enforcement。
若内部状态违反该不变量，step 必须作为 invariant failure 原子失败，不能降级为
normal no-grant。

current v1 production 仍按 ADR 0018 对任一 `timeWindows` 返回
capability-unavailable，因此当前只存在 static segment。未来时变 AccessRule runtime
G1 应把同一 cell 语义扩展为 `conflictEligible(stream, profile, segment)`，且不得
缩小 policy totality：只要组合在任一规范 time segment 中 eligible，就必须进入
`everConflictEligible` 并拥有完整 right-of-way rule；当前 segment 的 deny 只把该
tick candidate 收紧为 deny。以下所有“可合法进入/可用 profile”均严格指该定义，
包括 target priority、protected coherence 与 static clearance validation。

World 初始化必须把 pinned policy 编译为
`ResolvedStreamRule[ParticipantStreamHandle][VehicleProfileHandle]` 的稠密总表：

- 对每个 `everConflictEligible` 的已注册 VehicleProfile，必须恰好解析出一条 rule；
  无匹配、同 specificity/priority 的矛盾匹配或 dangling profile 都是初始化错误，
  不在 tick 默认 priority、默认 deny 或默认 allow；
- `yieldToStreamIds[]` 非空时 `gapProfileId` 必填且必须解析成功；yield set 为空时
  `gapProfileId` 必须省略，避免把未消费参数伪装成行为；
- v1 yield target 只选择 stream，不选择 target class。因此对 subject 的每条 yield
  edge，所有 target stream 的 `everConflictEligible` resolved VehicleProfile
  priority 都必须
  严格高于 subject；若法规需要按 target class 变化，必须扩展显式 target selector
  并重新走 G1，不能在 runtime 猜测；
- 每条 resolved rule 必须按 subject stream 的规范 passage-cell 顺序，预编译
  per-subject-cell `yieldTargetCellRange`：对 subject cell
  `(zone, subjectStream)`，只保留确实存在 exact
  `(zone, targetStream)` passage cell 的 yield target。一个 policy-level yield edge
  只要求在至少一个共享 zone 中产生 target cell；target 未参与 subject 的其他 zone
  时，该 zone 的 range 中没有该 target，不合成 missing cell，也不把 missing
  解释为 `OutsideHorizon`/deny/allow。每个 yield edge 若在全部 subject cells 中都
  没有 target cell，按前述“至少共享一个 ConflictZone”规则在 normalization
  拒绝；
- 每个 per-subject-cell target range 按 `targetStreamCanonicalRank` 编译，wire
  declaration order 只用于 first-error attribution；tick 不做 target/zone
  笛卡尔积、handle lookup 或 missing-cell 分支；
- 一个 Gate coverage 可以包含同一 ManeuverPath 上多个不同 subject stream 的 exact
  passage cells。对具体 profile，每个 subject cell 都按自己的 stream 从总表解析
  rule；candidate 的 `candidateRightOfWayPriority` 取全部 distinct resolved subject
  rules 的最小值，作为 atomic coverage 的保守 effective priority。yield/gap 仍按
  各 subject cell 自己的 rule/range 求值，不把多个 rules 合成一条虚构 policy row；
- `Protected`、`Permissive` 与 `Uncontrolled` candidate 都执行上述 rule resolution
  与 priority reduction；Protected 只跳过 yield/gap，不跳过规则解析。没有任何
  ConflictPassage、只有 Waiting admission 的 resource request 不读取 stream rule，
  `candidateRightOfWayPriority` 为 absent，由 arbitration key 的显式 presence rank
  与 arrival/waiting ticket 排序，不能伪造默认 policy priority。

### 6.4 protected 与 permissive

protected/permissive 不是 SignalAspect 的别名。versioned Gate compliance policy
按 vehicle + Gate + pre-step state 产生：

```text
GateRegulatoryDecision
  DenyAndStop
  Candidate(Protected)
  Candidate(Permissive)
  Candidate(Uncontrolled)
```

- `Protected`：无需服从 streamRules 中配置的 yield relation，但仍必须等待已占用/
  已 reservation 的 ConflictZone 清除，也仍受 Core safety 约束；
- `Permissive`：必须执行 yield/priority/gap acceptance；
- `Uncontrolled`：没有 signal-layer deny，仍按 right-of-way policy 执行；
- `DenyAndStop`：不进入 conflict candidate set；
- AccessRule/时变准入等任一 regulatory deny 都把 candidate 收紧为 deny；
- 任何 policy allow 都不能把另一个平面的 deny 改写为 allow。

yellow、flashing 与条件通行的具体 aspect mapping 属 policy data，而不是
SignalController。policy 至少消费当前 Gate、vehicle class、是否已 committed 及
Core safe-stop feasibility；同一固定输入必须产生同一 decision。

World 初始化必须对 pinned policy、全部 registered VehicleProfile 与每个 Controller
steady phase 做 protected-coherence validation：若两个 Gate coverage 共享任一
ConflictZone，且一对 `everConflictEligible` PreGate profile 在同一 phase 都会得到
`Candidate(Protected)`，则初始化拒绝。runtime reservation 仍保留为 invariant
防线，但不能把错误的 protected authoring 静默降级为“按 candidate 顺序等待”。
无法静态解析的条件 mapping 使用 capability-unavailable fail closed；transition
yellow 的 committed 解释不反向改变 steady-phase authoring 结论。

## 7. ConflictArbiter、gap acceptance 与 reservation

### 7.1 Core owner

`ConflictArbiter` 是 CoreWorld 私有运行时 aggregate：

- static input 来自 normalized ConflictRegistry/RightOfWayPolicyRegistry；
- dynamic input 来自 pre-step vehicle snapshot、WaitingZone state、
  ConflictZone occupancy/reservation、committed downstream claims 与 tick-start
  committed signal snapshot；
- 输出是 vehicle-specific tick-local grants；
- 只有 arbiter 持有 tick-local resource ledger 的 mutable authority；candidate
  producer 只读 immutable snapshot 并生成 resource request；
- Adapter 只能 query latest committed decision batch，不能把其中的历史 grant
  attribution 作为下一 tick permission，也不能注入 winner。

### 7.2 Gate evaluation frontier 与 arbitration candidate

`Gate evaluation frontier` 包含 next Gate occurrence 已进入本 tick lookahead 的全部
道路交通活动车辆，不能因 Waiting full、downstream storage 不足或 request 无法构造就
从 latest-decision observation 中消失：

- regulatory decision 为 `DenyAndStop` 时记录 `NotEvaluated`，并保留 GateStop；
- regulatory decision 为 Candidate、且 coverage 不含 Waiting admission 或
  ConflictPassage 时记录 `NotRequired`，沿用 current pure signal/regulatory Gate
  permission，不创建空 `ConflictGrant`；
- 有 resource-bearing coverage 的 Candidate 先只读构造 request；finite authoritative
  input 下不能满足 Waiting/downstream proof 是 `NoGrant(reason)` normal outcome，
  metadata/handle/non-finite invariant 破坏才使 step error；
- 只有 request 完整且已建立稳定 arrival ticket 的 vehicle 才成为
  `arbitration candidate`。

arbitration candidate 按稳定键排序：

```text
(
  protectedRank ASC,                 // Protected = 0, others = 1
  policyPriorityPresentRank ASC,     // conflict-backed = 0, pure Waiting = 1
  candidateRightOfWayPriority DESC,  // present 时为 coverage 全部 subject rules 的 min
  firstEligibleTick ASC,
  waitingTicketPresentRank ASC,      // existing Waiting member = 0, absent = 1
  waitingAdmissionSequenceOrZero ASC,
  vehicleUpdateSequence ASC          // live vehicle 中唯一
)
```

显式 presence rank 避免用合法 `u64::MAX` 冒充“无 sequence”；raw
`VehicleHandle` 不参与业务排序。`protectedRank` 不能让车辆进入已占用 zone，只影响
多个可同时申请者的顺序。
`policyPriorityPresentRank` 同样不是默认 priority：有 ConflictPassage 的 candidate
必须解析至少一个 subject rule 并携带 coverage-min priority；纯 Waiting admission
没有 conflict policy row，数值 priority slot 规范为 `0` 且在 presence rank 相同前
不参与比较。同一 Gate coverage 含多个 subject streams 时，不能任选第一条、最后
一条或最高 priority。
`firstEligibleTick` 在车辆首次满足 Gate arrival predicate 时分配；车辆离开该 Gate
lookahead、换 Route/occurrence 或完成 crossing 后清除。

candidate 构建不得把上述 request 预判为最终资源所有权。排序后
`ConflictArbiter` 对每个 candidate 顺序执行一次 `tryAcquireGrantBundle`：

```text
GrantResourceBundle
  covered ConflictZone claims
  WaitingAdmissionClaim?
  DownstreamClearanceClaim?
```

查询视图固定为 tick-start committed authority 加 stable order 中此前成功取得的
staged claims；同 tick staged leave/clear/release 不返还 capacity。bundle 要么全部
取得并产生 grant，要么全部失败且 ledger 不变，不允许 hold-and-wait 或部分取得。
同一 bundle 内同 owner 的 Waiting/downstream spans 可以重叠且只提交一次 owner
identity；冲突检查针对其他 committed/staged owner，不能让 bundle 自己阻塞自己。
bundle 内 Conflict/Waiting resources 按各自 `canonicalRank`、downstream intervals
按 `(edgeCanonicalRank, startProgress, endProgress)` 的预编译 physical order只读
preflight；`EdgeHandle` 只用于确认同一 physical edge，不作为跨 entity 的业务
tie-break。
candidate proposal 可以并行生成，但必须在单一 deterministic reducer 中按上述稳定键
取得资源；不得让 `Mutex`、atomic CAS、worker index 或 task completion order 决定
winner。

一个 evaluation 同时命中多个 normal stop 原因时，`NoGrantReason` 的诊断优先级固定
为：Waiting capacity、Waiting physical storage、Conflict occupied/reserved/staged、
lag gap、approach `Unprovable`、lead gap、downstream storage boundary、resource
claim conflict。同类多个实体按 `canonicalRank`，同一 claim 的多个 physical
interval 按上述预编译顺序归因。该顺序只决定 record/stop attribution；resource
bundle 仍先完整 preflight 后一次 stage，不以“先检查到”产生部分状态。

### 7.3 Downstream-clearance guard

v1 强制执行 keep-clear：grant 之前必须先从 tick-start committed occupancy 与
route-local hard-boundary metadata 建立 subject 即使不依赖任何前车继续移动也能
让车尾清空本 Gate coverage 全部 ConflictPassage 的 base proof，再原子取得与该
proof 对应的 `DownstreamClearanceClaim`。该 guard 是 Core safety/liveness
predicate，不是可由 Adapter、solver tier 或 Junction 配置关闭的表现选项。

Route registration 为每个 Gate occurrence 编译 ordered coverage passage range、
最远 exit anchor、其后的 next Gate/route terminal fast boundary，以及空
WaitingZone/storage 与 conflict-clearance 的 static feasibility operands。该
compiler 保持 class/profile-agnostic；runtime 对具体车辆计算：

```text
clearanceFrontTarget =
  max(routeDistance(passage.exitAnchor)) + vehicle.length

storageUpperBound =
  min(
    next leader rear route distance - vehicle.minimumGap,
    next ManeuverGate boundary,
    RouteEnd,
    tick-start-derived ParkingStop / WaitingCapacityStop / other hard stop boundary
  )

downstreamClearanceAvailable =
  finite(clearanceFrontTarget, storageUpperBound)
  AND storageUpperBound + CONFLICT_ANCHOR_CROSSING_TOLERANCE_METERS
      >= clearanceFrontTarget
  AND tryClaim(clearanceSpan) against committed + earlier staged claims
```

规则：

- coverage 有多个 zone 时使用 route traversal 上最远的 exit；相同 route anchor 按
  `conflictZoneCanonicalRank` tie-break，但不改变距离；
- next Gate 即使当前 indication 为 allow，也仍是 storage boundary；当前 grant
  只覆盖一个 Gate occurrence，v1 不预借后续 Gate permission；
- leader 查询使用 current route-relative occupancy；前车不会倒退，因此不要求它在
  subject clearing 期间继续移动；
- `clearanceSpan` 从 admission Gate boundary 的 crossed side 贯穿全部 coverage，
  直到 `clearanceFrontTarget`；按 Route occurrence 分段并规范化为
  `(EdgeHandle, startProgress, endProgress)` 的有序 physical intervals。不能只
  claim 最远 exit 之后的最终 vehicle footprint，否则两个 disjoint zone 的 route
  可能先在 coverage 内汇入同一 physical edge，再被 no-overlap 停在 zone 中；
- claim 冲突按 physical edge/progress 判定：interval 在 existing physical-gap
  predicate 下重叠即冲突；同一 directed physical edge 上的非重叠相邻 span 必须先
  按 progress 确定 follower owner，再满足
  `gap + MINIMUM_GAP_TOLERANCE_METERS >= follower.minimumGap`。candidate 位于后方时
  使用 candidate profile，candidate 位于前方时使用 existing claim owner profile；
  不能无条件使用 subject minimum gap。ledger 必须在 claim 中保留 profile/minimum
  gap 或通过 owner 做 O(1) dense lookup。不能只按 Route/Junction identity，因此
  不同 Route 或不同 Junction 汇入同一下游 edge 时仍争用同一资源；
- committed claim 与本 tick earlier staged claim 都进入唯一 ledger。v1 不把尚未
  落位的 claim 当作“未来 leader”允许后车在其后 packing，因为那会让后车的
  keep-clear proof 依赖 claim owner 继续移动；candidate 的必经/存储 span 与未落位
  claim 冲突时 normal no-grant。互不共享 physical span 的 candidate 不受影响；
- `clearanceFrontTarget`、vehicle length/minimum gap 与 tolerance 的组合使用
  overflow-safe segmented route-distance query 和 checked finite addition；storage
  比较与 §5.2 `tailCleared` 完全同构，不得把 tolerance 加到 target 一侧而额外改变
  boundary；
- Route registration 只编译每个 resource-bearing occurrence 的 empty-occupancy
  Waiting interval、最远 conflict exit 与 next Gate/RouteEnd operands，不枚举
  VehicleProfile，也不因某个已注册长车型不适配而拒绝整条 Route；
- initial/spawn/replacement/runtime route assignment 在 current static access
  validation 成功后，对实际 `(VehicleProfile, Route, cursor)` 的 pending
  resource-bearing occurrences 执行一次 static feasibility check：空 WaitingZone
  必须能完整容纳该 vehicle length，且
  `nextBoundaryRouteDistance + tolerance >= farthestExitRouteDistance + vehicle.length`。
  checked finite 失败或 boundary 不足时以结构化 binding error 原子拒绝；它不是
  steady-tick no-grant。initial 与 dynamic Routes 使用同一 compiler 和 binding
  validator，普通较短车型仍可绑定，不能让不相关长车型把 route-global 注册毒化；
- 缺少 finite route distance、可用存储不足、下一 Gate 位于车尾清空目标之前或任一
  hard boundary/claim 更近时，结果是 normal no-grant；
- guard 只证明存在可清空存储，不承诺 subject 本 tick 一次完成 traversal；crossing
  后仍按实际 passage enter/clear 持有 reservation 与 committed claim；
- hot path 必须复用 route-local boundary index 与 leader/occupancy query，不得为每个
  candidate 扫描全 Route、全 vehicle 或全 Conflict catalog。

grant 未 crossing 时其中全部 staged resource claim 在 tick 末失效；crossing 时
ConflictReservation 与其中的 Waiting/downstream claim 作为一组原子提交。downstream
claim 持有到 owner 车尾清空本 Gate 全部 coverage zone；同一 successful step 中
actual post-step occupancy 已成为下一 tick 的权威 leader view，再释放 claim，不出现
无 owner 的空窗。despawn 原子释放，active claim 下的 route replace 由 v1 capability
guard 拒绝；失败 step 不提交 ledger、vehicle、reservation、event、tick 或 time 的
任何部分。

后续若产品需要模拟“驶入后阻塞路口”的违规驾驶行为，必须以独立 versioned policy
和 G1 显式放宽；不能通过删除 guard 或 Adapter LOD 静默改变 v1。

### 7.4 Gap acceptance

v1 选择性能优先的保守确定性 critical-gap filter，不做概率 driver model、随机
critical-gap distribution、长时域 trajectory search 或 per-Junction solver
dispatch。`minimumLeadGapMs` 是 policy 已校准的完整 maneuver critical gap：从
subject 在本 tick 最晚跨 admission Gate 的时点，到更高优先级 foe 最早到达其
ConflictPassage entry 的最小间隔；它已包含 subject 完成 maneuver 所需的行为裕量。
`clearanceBufferMs` 是额外 fail-closed buffer，不替代该 policy 参数。

World 初始化对每条非空 yield rule checked-add
`fixedDeltaTimeMs + minimumLeadGapMs + clearanceBufferMs`，取最大值为
`maxRequiredLeadMs`；不存在 yield rule 时为 `0` 且可跳过 approach frontier。
随后 checked-add 得到 `frontierProofHorizonMs = maxRequiredLeadMs + 1`。Route
registration 编译一份向前有序的 `forwardConflictPassageOccurrences` 与
`routeEdgeIndex -> first passage index` fast index，使任意 current cursor 能从首个
未清除 passage 开始有界遍历；它必须包含 current 和 upcoming Maneuver occurrences，
并以 route occurrence index 区分 repeated path。不得为每个 cursor 复制 remainder
list 形成 `O(route edges × passages)` retained memory。

occupancy rebuild 从每个 active vehicle 的 pre-step committed cursor 出发，以
VehicleProfile `maxAcceleration = a`、current speed `v` 和
`frontierProofHorizonMs` 的向上舍入最大可达距离为界，访问范围内尚未清除的 passage
entry。位于当前 ManeuverPath 之前、但能在 horizon 内到达的车辆仍必须贡献；不得把
“尚未进入 occurrence”当作“不存在 approach”。

对每个 entry anchor 通过 segmented route-distance query 计算 exact canonical
`f64` operands 所代表距离的 finite、non-negative directed lower enclosure
`dLower`。edge remainder、跨 edge 累加与 cursor subtraction 都必须使用下述
`lower_*` helper，不能先以普通 round-to-nearest 得到 `d` 再把它当作下界。随后使用
以下语义求最快到达：

```text
if dLower <= CONFLICT_ANCHOR_CROSSING_TOLERANCE_METERS:
  etaLowerSeconds = 0
else if a > 0:
  etaLowerSeconds =
    lower_div(
      lower_mul(2, dLower),
      upper_add(
        upper_sqrt(
          upper_add(
            upper_mul(v, v),
            upper_mul(upper_mul(2, a), dLower))),
        v))
else if v > 0:
  etaLowerSeconds = lower_div(dLower, v)
else:
  frontierValue = OutsideHorizon

foeEarliestEntryMs =
  floor(max(0, lower_mul(etaLowerSeconds, 1000)))
```

对 `a >= 0, v >= 0`，规范 ETA 对 distance 单调不减，因此分子、radicand 与
`a = 0` 分支都使用同一个 `dLower`；directed operation 得到的值不大于
`ETA(dLower)`，进而不大于 canonical exact distance 的 ETA。不得在 radicand
混入普通舍入的 distance，或用 independently rounded 较大 distance 破坏这条证明。

`lower_*`/`upper_*` 是 Core 私有、仅接受 canonical `+0` 或正 finite 输入的
directed-bound helper。`+`、`-`、`*`、`/` 与 `sqrt` 的 IEEE-754 binary64
round-to-nearest-even 结果分别向 `-infinity`/`+infinity` 扩一 representable
value；lower result 下穿零时规范为 `+0`，upper result 变为 infinity、任一
NaN/非法除数或无法证明运行环境 primitive 契约时返回 `Unprovable`。`2.0`、
`0.5`、`1000.0` 与 `<= 2^53` 的整数毫秒转换都是 exact operand；`a == 0`/
`v == 0` 在 signed-zero canonicalization 后精确判断，不使用通用 near-zero
tolerance。

上述 operation order 是规范语义，不允许换用
`(-v + sqrt(v² + 2ad)) / a`、普通 round-to-nearest 后直接 floor 或其他“等价”
公式。`lower_sub(left, right)` 是非负饱和向下界：
`max(+0, nextDown(roundToNearest(left - right)))`。只有输入、全部 required upper
intermediate 与 checked `floor -> u64` 转换有效时才得到 `Finite(ms)`；有限输入导致
中间 overflow、除数无效、整数转换越界或无法建立 enclosure 时记
`Unprovable`，作为 normal no-grant，不使整个 step error。

Outside proof 使用以下固定 operation order：

```text
tUpper = upper_div(exactF64(frontierProofHorizonMs), 1000.0)
tSquaredUpper = upper_mul(tUpper, tUpper)
travelUpper =
  upper_add(
    upper_mul(v, tUpper),
    upper_mul(upper_mul(0.5, a), tSquaredUpper))
distanceLower =
  lower_sub(dLower, CONFLICT_ANCHOR_CROSSING_TOLERANCE_METERS)

OutsideHorizon iff travelUpper < distanceLower
```

整数到 f64 的转换因 horizon `<= 2^53` 精确。任一 upper intermediate 非 finite 或
无法完成该严格证明都不能误记为 Outside，而是继续 ETA enclosure；后者仍无法证明
时才为 `Unprovable`。production helper 必须用 predecessor/equal/successor、
subnormal、最大 finite、中间 overflow 与高精度 interval oracle 锁定 enclosure，
不能只用普通示例值。单个 contribution 的规范状态为：

```text
OutsideHorizon                  // 无 approach contribution，或已证明全部在 horizon 外
Finite(foeEarliestEntryMs)      // 至少一个有限、可证明的 lower-bound ETA
Unprovable                      // 至少一个相关 contribution 无法证明安全
```

frontier cell 不能只保存上述 tri-state aggregate，因为 subject 可能在 looping/
repeated Route 的 future target-stream passage 为同一 cell 贡献并错误阻塞自己。
cell key 固定为静态
`(ConflictZoneHandle, ParticipantStreamHandle)`；§5.4 已保证每对最多一个 passage。
它不能按 Route-specific `ConflictPassageOccurrence` 建 cell，否则 retained memory
随注册 Route/重复 occurrence 膨胀，yield query 也会退化成 candidate × route-range
扫描。每个 static cell 保存按“更保守优先”排序的两个**不同 vehicle owner**：

```text
FrontierCell
  first:  (VehicleHandle, Unprovable | Finite(ms))?
  second: (VehicleHandle, Unprovable | Finite(ms))?

ordering:
  Unprovable before Finite
  Finite(ms) by ms ASC
  exact tie by vehicleUpdateSequence ASC
```

`OutsideHorizon` 不占 slot。一个 vehicle 对同 cell 有多个 contribution 时先在 owner
内归约：任一 `Unprovable` 则 owner 为 `Unprovable`，否则取最小 `Finite`；再维护
top-two distinct owners。candidate 查询使用 `valueExcluding(subject)`：`first`
owner 不是 subject 时返回 first，是 subject 时返回 second，两者都缺失才是
`OutsideHorizon`。因此只排除 exact subject handle，不排除同 profile/Route 的其他
车辆，也不需要 candidate × all-vehicles 回扫。

occupancy rebuild 在一次 active-vehicle pass 中，对 horizon 内的 forward
Route-specific passage occurrences 产生 owner-attributed contribution，再通过其
compiled static passage handle 归约到上述 static cell。同 owner 的
current/upcoming/repeated occurrences 先合并，不能为每个 Route occurrence 保留一个
cell。只有 exact `(stream, profile, current static segment)` 为
`conflictEligible` 的 PreGate approach 才贡献；normalization 中静态
AccessRule-denied 的合成 stream/profile 组合不贡献，而 public route-binding API
不会让该组合成为 active vehicle。已经 occupied/reserved 的异常或 committed
traversal 仍由 zone authority fail closed。current red/leader/parking stop 不放宽
eligible foe ETA，因为 foe 下一 tick 可能重新加速；只使用物理最大加速度上界。

候选对 coverage 中每个 exact subject passage cell，从已选择的 resolved rule 直接
索引该 subject-cell ordinal 的预编译 `yieldTargetCellRange`，再对 range 内每个 exact
target cell 执行 O(1) self-exclusion。target stream 没有参与当前 zone 时不会出现在
该 range 中；tick 不查询不存在的 `(zone, targetStream)` cell，也不扫描 registered
Routes 或 all vehicles。benchmark 必须报告 frontier contribution count、static cell
count、编译后 target-cell count、top-two retained bytes 与每 active vehicle 的
visited-passage 分布，发现接近 vehicle × whole-conflict-catalog 时阻断。

对 subject coverage 中每个 ConflictZone，按以下规范顺序求值：

1. 任一 incompatible occupant、committed reservation 或本 tick 已先行 grant
   存在：no-grant；
2. 对当前 subject cell 的 compiled target-cell range 中每个 target，若最近一次
   incompatible passage clear 存在，要求
   `currentTimeMs - lastClearTimeMs >= minimumLagGapMs + clearanceBufferMs`；
3. 以 checked integer addition 计算
   `requiredLeadMs = fixedDeltaTimeMs + minimumLeadGapMs + clearanceBufferMs`；
4. 当前 subject cell range 中每个 exact target cell 的
   `valueExcluding(subject)` 为 `OutsideHorizon` 时通过，为
   `Finite(foeEarliestEntryMs)` 时必须严格大于 `requiredLeadMs`；等于 boundary
   或 `Unprovable` 时 no-grant；
5. 所有 coverage zones 均通过才产生一个原子 grant。

`fixedDeltaTimeMs` 覆盖 subject 可能在当前 interval 末尾才跨 Gate 的最坏时点。
World 初始化必须以 checked addition 验证所有 gap profile：
`requiredLeadMs` 与 `minimumLagGapMs + clearanceBufferMs` 都不得超过
`MAX_PORTABLE_GAP_TIME_MS`，`frontierProofHorizonMs` 不得超过 `2^53`。
stationary foe 若 `a > 0` 仍得到有限 ETA；只有 `v = 0 && a = 0` 或经 directed
proof 确认超出 horizon 时才是 `OutsideHorizon`。非 finite route
distance/speed/acceleration、负值、越界 handle 或 frontier/state 不变量破坏是
error 且 step 不提交；有限输入的保守算术无法证明属于 `Unprovable` normal
no-grant。`lastClearTimeMs > currentTimeMs` 同样是 invariant error。合法
`OutsideHorizon` 通过 lead 检查；gap 不足与 `Unprovable` 是 normal no-grant。

该 v1 有意偏保守：它不宣称复现专业交通工程容量或现场 driver distribution。
任何加入 stochastic/calibrated driver behavior、long-horizon trajectory prediction
或按 Junction 选择不同 solver 的方案都必须独立 G1，并以本 v1 作为 correctness/
performance oracle；不能作为“更保守实现细节”静默替换。

### 7.5 Grant 与 reservation

```text
ConflictGrant
  tickIndex
  vehicle
  route
  maneuverOccurrenceIndex
  gateOccurrenceIndex
  conflictZoneRange
  waitingAdmissionClaim?
  downstreamClearanceClaim?

ConflictReservation
  vehicle
  route
  maneuverOccurrenceIndex
  gate
  conflictZoneRange
  downstreamClearanceClaim
  acquiredTick
```

- grant 仅当前 tick 有效；
- `conflictZoneRange` 可为空；只有 range 非空时才创建
  `ConflictReservation`，且此时 `downstreamClearanceClaim` 必须存在。纯
  Waiting admission 可以提交 membership 而不伪造空 reservation；
- grant 是已取得 staged resource bundle 的私有 capability；未跨 Gate 时不提交
  reservation/membership/claim；
- 成功跨 Gate 时，grant、vehicle state、WaitingZone leave/enter 与 bundle 中存在的
  zone claims 原子升级出的 reservation/downstream claim 一起提交；
- reservation 覆盖该 Gate coverage 的 zone 并集；
- vehicle 前保险杠跨 passage entry 后 zone 变为 occupied；车尾跨 exit anchor 后
  该 zone cleared；
- 若 clear 发生在 `[T, T + D)`，staging 必须把对应 `lastClearTimeMs` 记录为
  post-step `T + D`；本 step 先执行的 arbitration 不观察该 staged clear，下一 tick
  以 elapsed `0 ms` 开始 lag cooldown，禁止写成 tick-start `T` 提前一个 delta；
- 全部 coverage zone cleared 后释放 reservation；
- committed downstream claim 与 reservation 同 owner、同 crossing commit，并在
  全部 coverage zone cleared、post-step actual occupancy 已可供下一 tick 查询时
  一起释放；
- 同一个 ConflictZone 不得同时存在 incompatible reservations；
- route completion 前必须清空 active reservation，否则 invariant error 使 step
  原子失败；
- 城市级提前预约、跨多个 Junction 的 slot booking 与外部 reservation API 不在
  本设计范围。

Rust implementation 应把 tick-local grant 表达为 Core 私有、字段不可伪造且不实现
`Copy`/`Clone` 的 `#[must_use]` capability；crossing 通过消费该值完成
`StagedGrant -> committed bundle resources`，candidate scratch 可用
`Option::take` 保证单次迁移。实际 ledger 使用 tick/generation 校验 stale ID，避免
长生命周期 `&mut` borrow 阻断后续仲裁。权威 release 必须是显式 state
transition；不得依赖 `Drop`、panic unwinding 或 destructor 顺序改变 committed
simulation state。

## 8. ManeuverTraversalState

### 8.1 状态 shape

Core vehicle 新增私有权威状态；public API 暴露只读 snapshot：

```text
ManeuverTraversalState
  route
  maneuverOccurrenceIndex
  phase
    PreGate {
      nextGateOccurrenceIndex,
      firstEligibleTick?
    }
    Committed {
      lastCrossedGateOccurrenceIndex
    }
    Waiting
    Clearing {
      reservation
    }
  activeWaitingMembership?
    waitingZone
    admissionSequence
    releaseGateOccurrenceIndex
```

不在 active Maneuver occurrence 的车辆没有该状态；`PreGate` 可在 occurrence
lookahead 内惰性建立。

Waiting membership 与“当前是否已停住”等 motion phase 正交：

- `Waiting` phase 必须有且只有一个 `activeWaitingMembership`；
- `Committed` 可以没有 membership，也可以在车辆已跨 entry Gate、仍向 zone
  内前进或从等待重新起步但尚未跨 release Gate 时携带一个 membership；
- `PreGate` 与 `Clearing` 不得携带 membership；crossing release Gate 必须先原子
  移除旧 membership，若同一 boundary 同时是下一 WaitingZone entry Gate，则以
  本次 successful admission claim 原子替换为新 membership；
- semantic membership record 是 occupancy/release/despawn/route-command 的车辆侧
  authority；per-zone head/tail 与 per-member intrusive links 只是与其同事务维护的
  稠密索引，任一不一致都是 invariant failure；
- public snapshot 可以暴露上述 semantic membership，不暴露 intrusive link 或
  mutable queue authority。

grant/no-grant 的可观察性由 CoreWorld 级稀疏 latest-decision batch 承担；它只为
本 tick 实际求值的 active Gate frontier 生成有序 records：

```text
LatestGateDecisionBatch
  tick
  records[]
    vehicle
    route
    maneuverOccurrenceIndex
    gateOccurrenceIndex
    regulatoryDecision
    conflictOutcome          // NotEvaluated | NotRequired | Granted | NoGrant(reason)
    stopAttribution?
```

batch 只描述刚完成 successful tick 的决策归因，不是 permission lease；未进入
Gate evaluation frontier 的 vehicle 没有 record，下一 step 必须重新求值。
`NotEvaluated` 对应 regulatory deny，`NotRequired` 对应无 resource-bearing coverage
的 pure Gate；二者都不能伪装成 grant/no-grant。records 按唯一
`vehicleUpdateSequence` 排序，不按 arbitration completion order 或 raw handle；
使用复用的稀疏 buffer，不要求每 vehicle 永久增加 decision slot。
`ConflictGrant` 本身只存在于 step candidate/staging scratch，不进入 committed
vehicle state 或可由 Adapter 回写的公共对象。

### 8.2 迁移

```text
none
  -> PreGate
PreGate
  -> Committed | Waiting | Clearing
Committed
  <-> Waiting
Committed | Waiting
  -> Clearing
Clearing
  -> Committed | PreGate
  -> none
```

- crossing 第一 Gate 使用与任意 Gate 相同的 resource-specific commit：无
  Waiting/Conflict resource 时 PreGate -> Committed；提交 Waiting membership 且
  post-step 已停住时进入 Waiting，仍在移动时进入携带 membership 的 Committed；
  non-empty ConflictReservation 则直接进入 Clearing。第一 Gate 不得用无条件
  `PreGate -> Committed` 覆盖 grant bundle 的 committed resources；
- crossing WaitingZone entry Gate：提交 `activeWaitingMembership`；若当 tick
  继续前进，phase 仍为 Committed；停止等待时只把 phase 转为 Waiting，membership
  不变；
- Waiting member 在 release Gate 前重新起步时只把 phase 转回 Committed，
  membership 不变；只有 successful release crossing、despawn 或被允许的等价
  transaction 才能移除它；
- 取得 release/admission grant 但未 crossing：Waiting/Committed committed state
  不变，只更新 latest-decision snapshot，grant 在 step 结束时失效；
- crossing grant Gate：若后续还有 safe non-conflict path，直接进入/保持
  Committed；若取得 reservation 则直接进入 Clearing；同一 crossing 的旧 Waiting
  membership leave 与可选下一 zone admission 必须和 phase/reservation 原子提交；
- 清除 reservation 后，若 occurrence 尚有下一 Gate，进入 Committed/PreGate；
  occurrence exit 后清除 state；
- 多 WaitingZone 允许上述中段重复，不把 enum 误解为只能单次线性经过。

### 8.3 黄灯、signal 切换与原子 crossing

- `[T, T + D)` tick 使用 tick-start committed signal snapshot(T)；预先计算的
  next-time candidate 只在 successful step 末尾提交为 snapshot(T + D)，供下一
  tick 使用；
- compliance policy 用 pre-step maneuver state 与 safe-stop feasibility 解释 yellow；
- motion 未跨 Gate 时，下一 tick 必须重新求值，旧 grant 不延续；
- motion 成功跨 Gate 时，crossing 与 state/reservation 在同一 step 提交；随后 signal
  变化不能追溯撤销；
- 已 committed 不代表后续 Gate 自动允许；
- denied Gate、无 grant Gate 与 WaitingZone capacity boundary 都必须在 traversal
  hard guard 再检查，不能只依赖舒适减速。

### 8.4 Route replace、despawn 与 completion

- current `spawn_vehicle`、initial vehicle normalization 与
  `replace_completed_vehicle` 都允许 caller 指定非零 route cursor；#235 v1 不从
  任意 interior cursor 猜测已发生的 Gate crossing、Waiting admission sequence 或
  Conflict reservation；
- `stateful occurrence` 指包含多个 Gate、任一 WaitingZone 或任一
  ConflictPassage 的 Maneuver occurrence；`first stateful Gate` 是其中规范序最小的
  GateOccurrence。只有 current protected-entry baseline 的单 Gate、无
  Waiting/Conflict occurrence 不触发本 guard；
- 对含 stateful Gate/Waiting/Conflict metadata 的 occurrence，travel-lane vehicle
  只可在 first stateful Gate 的未跨越侧或 occurrence exit 之后生成/替换。精确停在
  Gate from-side boundary 仍是未 crossing，可建立/惰性建立 `PreGate`；位于 to-side
  `(routeEdgeIndex, 0)` 已属于 crossed side；
- cursor 位于 first stateful Gate crossed side 到 occurrence exit 的半开区间时，
  spawn、initial batch 与 Completed replacement 一律返回
  capability-unavailable/structured validation error，并保持 world 不变。普通 route
  segment、尚未 crossing 的 approach 与已完成 occurrence 不受此 guard 影响；
- future 若要支持 interior materialization，必须独立 G1 冻结显式 bootstrap
  transaction，并一次提供/验证 maneuver state、Waiting membership/admission
  sequence、Conflict reservation/downstream claim、occupancy 与事件基线；不能只按
  cursor 自动补一个 enum；
- active PreGate/Committed/Waiting/Clearing vehicle 的 route reassignment 必须证明
  新旧 route 从 current cursor 起拥有同一 Maneuver/Gate/Waiting/Conflict
  occurrence identity；第一版可以对全部 non-none maneuver state 一律
  capability-unavailable；
- despawn/removal 必须原子释放 WaitingZone occupancy 与 reservation，并产生明确
  release reason；active downstream claim 同事务释放；
- route 不得终止在 WaitingZone、未清除 ConflictPassage 或 active maneuver
  occurrence 内；
- completion event 必须晚于 zone/reservation release event。

## 9. Fixed-tick 管线与约束组合

规范 phase：

```text
0. validate fixed delta / command batch
1. compute next-time SignalController + SignalGroup candidate；保留
   tick-start committed snapshot 供本 tick 决策
2. rebuild route-relative occupancy、leaders 与 conflict approach ETA frontier from
   pre-step vehicles
3. refresh WaitingZone occupancy / ConflictZone occupants and validate sentinels
4. resolve Gate regulatory decisions against tick-start committed signal snapshot
5. build Gate evaluation records、normal no-grant attribution 与 finite resource
   requests from immutable snapshot
6. stable-sort and atomically acquire grant bundles against committed + earlier staged
   Waiting/Conflict/downstream claims in a single-writer ledger
7. build longitudinal intents
8. add RouteEnd / speed-limit / ParkingStop / GateStop /
   WaitingCapacityStop / ConflictStop constraints
9. reduce to strictest motion; apply leader safe-speed and final no-overlap
10. permission-aware traversal hard guards
11. stage candidate motion 与 successful Gate-crossing sets
12. checked-assign per-WaitingZone admission sequence；stage
    vehicle/Gate/Waiting/Conflict/reservation/claim state、decision records 与 events
13. append controller phase/aspect events
14. atomically commit state, tick and time
```

约束只会收紧：

```text
finalTravel =
  min(
    free/IIDM candidate,
    route end,
    speed limit,
    parking stop,
    denied signal/regulatory Gate,
    WaitingZone capacity/storage,
    missing ConflictGrant,
    leader/safe-speed/no-overlap
  )
```

实现可以共享 candidate reducer，但 attribution 与 hard guard 必须区分
GateStop/WaitingCapacityStop/ConflictStop。数值相同时使用规范 attribution priority
稳定事件；business right-of-way priority 绝不能复用 reducer attribution priority。

## 10. Events、查询与 normal outcome

### 10.1 Public events

候选事件面：

- `VehicleGateStopProjectionApplied`
- `VehicleWaitingZoneCapacityProjectionApplied`
- `VehicleConflictStopProjectionApplied`
- `VehicleCrossedManeuverGate`
- `VehicleEnteredWaitingZone`
- `VehicleLeftWaitingZone`
- `VehicleConflictReservationAcquired`
- `VehicleEnteredConflictZone`
- `VehicleClearedConflictZone`
- `VehicleConflictReservationReleased`
- `VehicleManeuverTraversalCompleted`

事件 payload 至少包含 tick、vehicle、Route、maneuver occurrence index 与相关 typed
handles。projection event 只在 hard projection 超出普通 comfort envelope 时发出；
正常等待/no-grant 不以 error 或每 tick spam event 表达，可通过 snapshot/query
观察。

### 10.2 Total order

每个 successful tick 的总序：

1. vehicle 按 existing update order；
2. 同一 vehicle 先 primary longitudinal projection，再 following safety projection；
3. boundary event 按 traversal 已经过的 canonical route anchor 顺序；实现按 cursor
   traversal 直接 append，不按 event kind 重新排序；
4. reservation release；
5. maneuver completion；
6. route completion；
7. Controller phase changed；
8. SignalGroup aspect changed。

canonical route anchor key 使用 `(routeEdgeIndex, canonicalEdgeProgress)`；非最后
edge 的 end anchor 按 PathAnchor 规则规范化为下一 edge 的 `(index + 1, 0)`。只有
同一 anchor 上的事件才使用以下 tie-break：

```text
Gate crossing
  -> Waiting leave
  -> Waiting enter
  -> reservation acquire
  -> Conflict enter
  -> Conflict clear
  -> VehicleChangedEdge
```

同 kind、同 anchor 涉及多个 static entity 时，按对应 `canonicalRank` 排序；同一
vehicle 的 dynamic occurrence index 只在 static rank 仍相同时作为最后 tie-break。
raw handle 数值不得参与事件顺序。

因此若 Gate/edge boundary 后的 to-edge 内部才出现 Conflict entry，必须先发
`VehicleChangedEdge`，随后到达内部 anchor 时再发 `VehicleEnteredConflictZone`。
没有改变 current projection-before-edge、vehicle-before-signal 的既有契约。

### 10.3 Error 与 normal outcome

以下是 normal outcome，不返回 error：

- red/deny；
- WaitingZone full 或物理存储不足；
- downstream-clearance storage 不足；
- Waiting admission/downstream span 与 committed 或 earlier staged claim 冲突；
- permissive candidate gap 不足；
- priority/yield loser；
- zone 被占用或已 reservation；
- approach frontier 为 `Unprovable`；
- 车辆因更严格 leader/no-overlap 没有使用 grant。

以下是 error，且 step 必须完全不提交：

- non-finite authoritative input/state 或 directed-bound helper 之外的 non-finite
  computation；有限 approach 输入在该 helper 内无法建立 bound 已规范为
  `Unprovable` normal no-grant；
- occurrence/handle 越界；
- WaitingZone occupancy 与 vehicle state 不一致；
- incompatible double reservation；
- duplicate/stale claim、partial grant bundle 或 claim owner/state 不一致；
- denied/no-grant Gate 被 motion 穿越；
- route completion 留下 active zone/reservation；
- sequence/tick/time overflow；
- tick-start committed signal snapshot 与 Gate decision 代次不一致。

## 11. Normalized storage 与性能

### 11.1 Static flat storage

```text
WaitingRegistry
  zones: Vec<ResolvedWaitingZone>
  pathZoneHandles: Vec<WaitingZoneHandle>
  pathZoneRanges: Vec<Range>
  canonicalRanks: Vec<u32>

ConflictRegistry
  zones: Vec<ResolvedConflictZone>
  streams: Vec<ResolvedParticipantStream>
  passages: Vec<ResolvedConflictPassage>
  passageCells: Vec<ResolvedPassageCell>       // unique (zone, stream)
  zoneStreamHandles: Vec<ParticipantStreamHandle>
  pathStreamRanges: Vec<Range>
  gateClearancePassageRanges: Vec<Range>
  zoneCanonicalRanks: Vec<u32>
  streamCanonicalRanks: Vec<u32>

RightOfWayPolicyRegistry
  rules: Vec<ResolvedStreamRule>
  yieldTargets: Vec<ParticipantStreamHandle>
  yieldTargetCells: Vec<PassageCellHandle>
  ruleSubjectCells: Vec<ResolvedRuleSubjectCell>
    subjectPassageCell
    yieldTargetCellRange
  streamProfileRuleTable: dense total resolved table
```

external ID 用于 load/resolve/public diagnostics，并在 normalization 一次编译
permutation-stable `canonicalRank`；hot path 只用 handle/index/range/rank，不做字符串
比较或排序。每个 `ResolvedStreamRule` 的 `ruleSubjectCells` 与 subject stream 的
规范 passage-cell 顺序一一对应；其 target range 只包含同一 ConflictZone 中实际存在
的 exact target cells。

### 11.2 Route-local metadata

Route registration 对每个 occurrence 一次性编译：

- ordered Gate occurrences；
- WaitingZone occurrence/range；
- ConflictPassage occurrences 及其 exact static `PassageCellHandle`；
- route-level forward ConflictPassage occurrence list 与
  `routeEdgeIndex -> first passage` fast index；
- each Gate coverage；
- each Gate coverage 的最远 clearance exit 与 next Gate/route-terminal boundary；
- each Waiting admission 的 empty-occupancy storage operands，以及每个
  resource-bearing Gate 的 profile-binding static feasibility operands；
- each Gate coverage 的 downstream clearance span segmentation/physical edge index；
- route-distance index 到所有 anchors；
- next regulated/admission Gate fast index。

复杂度以 Route edge count + matched occurrence metadata 线性有界；registration
失败不产生半注册 Route。

### 11.3 Runtime state

- per WaitingZone：occupancy count、head/tail、next admission sequence；per active
  member 使用按 VehicleHandle 稠密索引的 intrusive prev/next link，以 O(1) 维护
  head/tail、release 与 despawn；link 与车辆侧 semantic membership 同事务提交，
  不作为独立业务 authority；
- per ConflictZone：dense occupant/reservation owner range；
- per active Clearing vehicle：committed downstream claim 与 flat physical interval
  range；claim owner 与 reservation owner 必须一致；
- per static `(ConflictZone, ParticipantStream)` passage cell：optional last-clear time 与
  owner-attributed top-two approach frontier；
- per vehicle：optional maneuver state/arrival ticket；
- per latest successful tick：只覆盖 evaluated Gate frontier 的稀疏 ordered
  decision records，复用 candidate high-water capacity；
- per tick：预分配 active candidates、non-Clone staged grant tokens、Waiting/
  Conflict/downstream single-writer claim ledger、static-cell frontier contribution
  scratch、downstream-clearance query scratch、stable radix/counting-sort scratch；
- 只扫描 active Gate frontier/occupied zones，不扫描全 vehicle × 全 zone；
- scratch 可随 high-water mark 扩容，但同一容量 steady tick 零 allocation；
- 禁止 HashMap iteration 决定 winner/event order。

### 11.4 一万/十万 guard

后续 production 必须新增 deterministic benchmark topology，至少包含：

- protected + permissive 混合；
- 多 WaitingZone queue；
- 多 stream 共享 ConflictZone；
- occupied/reserved/no-grant 与 successful grant；
- empty/far/unprovable approach、current/upcoming/repeated passage frontier；
- repeated Route 中 subject 自己的 future target-stream contribution 与至少两个
  distinct contributor 的 self-exclusion；
- 大量 dynamic Routes/repeated occurrences 映射到固定 static passage cells，不按
  Route 数复制 frontier cell，也不让 candidate 扫 route ranges；
- downstream storage available/blocked 与 next-Gate clearance boundary；
- 两个不共享 ConflictZone/Junction 但争用同一 physical downstream span 的
  candidate，以及 unused/committed/released claim lifecycle；
- WaitingZone 同 tick capacity/storage contention、unused admission expiry 与
  staged leave 不返还 capacity；
- repeated Route/Maneuver occurrence；
- 当前道路机动车执行域 `N_traffic_active=10000` 与
  `N_traffic_active=100000` 的 research
  observation；不外推为其他执行域能力。

门槛：

- 一万 conflict/waiting 增量 phase：p95 目标 `<= 1 ms/tick`，硬门槛
  `<= 4 ms/tick`（项目 baseline hardware/protocol 下）；
- 一万 steady state：hot-path allocation count = 0；
- 十万/一万 elapsed scaling ratio `<= 20x`，并报告 retained bytes/vehicle、
  bytes/zone、candidate count、static passage cell count、frontier contribution
  count、visited forward passages/active vehicle、top-two frontier bytes、Waiting/
  downstream claim count/collision count、downstream-clearance query count/visited
  boundaries 与 occupancy density；
- 相同 fixture/seed 重跑 state digest 与 ordered event digest 完全相同；
- 未提交正式 benchmark 证据前，不得声明 #235 runtime 已完成。

该门槛是新 conflict/waiting phase 的约束，不降低 `vehicle-following.md` 与
`core-runtime-performance-baseline.md` 的现有整体产品预算。

## 12. 概念 wire、版本与 capability guard

以下只表达候选字段语义，不分配 production format version：

```json
{
  "waitingZones": [
    {
      "id": "wait-left-1",
      "maneuverPathId": "path-left-1",
      "entryGateId": "gate-a",
      "releaseGateId": "gate-b",
      "maxOccupancy": 4
    }
  ],
  "conflictZones": [
    {
      "id": "conflict-center",
      "junctionId": "junction-1"
    }
  ],
  "participantStreams": [
    {
      "id": "stream-left",
      "junctionId": "junction-1",
      "maneuverPathId": "path-left-1",
      "passages": [
        {
          "conflictZoneId": "conflict-center",
          "entryAnchor": { "pathEdgeIndex": 2, "progressMeters": 3.5 },
          "exitAnchor": { "pathEdgeIndex": 2, "progressMeters": 8.0 }
        }
      ]
    },
    {
      "id": "stream-opposing",
      "junctionId": "junction-1",
      "maneuverPathId": "path-opposing-1",
      "passages": [
        {
          "conflictZoneId": "conflict-center",
          "entryAnchor": { "pathEdgeIndex": 1, "progressMeters": 4.0 },
          "exitAnchor": { "pathEdgeIndex": 1, "progressMeters": 9.0 }
        }
      ]
    }
  ],
  "rightOfWayPolicySets": [
    {
      "id": "policy-cn-example-v1",
      "regulation": {
        "jurisdiction": "example",
        "version": "v1",
        "source": "authoritative-reference"
      },
      "effectiveFrom": "2026-01-01",
      "effectiveUntil": "2027-01-01",
      "evidenceRefs": [],
      "gapProfiles": [
        {
          "id": "gap-left-yield-v1",
          "minimumLeadGapMs": 4500,
          "minimumLagGapMs": 1000,
          "clearanceBufferMs": 500
        }
      ],
      "streamRules": [
        {
          "participantStreamId": "stream-left",
          "priority": 10,
          "yieldToStreamIds": ["stream-opposing"],
          "gapProfileId": "gap-left-yield-v1",
          "evidenceRefs": []
        },
        {
          "participantStreamId": "stream-opposing",
          "priority": 20,
          "yieldToStreamIds": [],
          "evidenceRefs": []
        }
      ]
    }
  ]
}
```

版本规则：

- current Traffic source version 是 `0.10`；#281 已从已发布且 immutable 的
  `0.9` 按 ADR 0008 原子 clean-break，并保留 #262 的 ADR 0018 静态准入；
- current v0.10 loader 必须继续拒绝 Conflict/policy 等尚未交付的 unknown fields；
- 不允许先接受 schema 再静默忽略 runtime 语义；
- SpatialPackage 是否 bump 由 geometry implementation G1 根据实际 shape 决定，
  Traffic behavior 不依赖该 bump；
- 已发布 schema 与显式固定 provenance 的 artifacts immutable，禁止 dual-shape
  alias 或 migration shim；current canonical fixtures 在新版本切换时原子迁移，
  不回写旧版本语义。

若 production 只实现 static identity/compilation，runtime fields 必须通过明确
capability guard 拒绝或由独立 feature/version 隔离；不能让 WaitingZone/ConflictZone
数据载入成功却按普通 protected Gate 行驶。不得修改已发布 v0.9 schema 来承载
“临时可选字段”，也不得留下 v0.9/new-version dual loader。

## 13. Validation 与 first-error

后续 Traffic/Data + Core normalization 必须扩展、而不是重排 current v0.10 的
canonical prefix。单一 Traffic load API 的 phase 顺序为：

1. JSON syntax 与 minimal version header；
2. exact current version；
3. strict current wire shape；
4. distance/time units；
5. ParticipantClasses；
6. VehicleProfiles，包含必填 `participantClassId` 引用；
7. LaneGraph edge length/speedLimit/connections；
8. Junction identities；
9. Movement owner/cardinality；
10. ManeuverPath edge references/connectivity/ownership；
11. StopLines；
12. SignalGroups；
13. Controllers/Phases/States；
14. ManeuverGates；
15. signal path/Gate coverage/ownership/usage；
16. Parking areas/spaces/anchors/geometry/reverse indexes；
17. FacilityBands/RoadSections/LaneGroups/RoadCorridors；
18. AccessRules，包含 capability guard、provenance 与 resolved `AccessCell` tables；
19. WaitingZone identities/refs/same-path/Gate order/capacity/overlap；
20. ConflictZone/ParticipantStream identities/refs/Junction coherence；
21. PathAnchor 与 passage finite/range/canonical/order/duplicate/coverage；
22. RegulationIdentity/effective interval/evidence、gap profiles、stream rules、
    AccessRule/right-of-way provenance coherence；
23. routes、current ManeuverOccurrence 与 final-StopLine rule；
24. Gate/Waiting/Conflict/forward-passage occurrence compilation；
25. Waiting empty-storage、clearance exit/next-boundary operands 与 route
    terminal/active-zone structural invariants；
26. `InitialTrafficData` final assembly/rebind。

Spatial/Scenario 与 World/runtime 不是 Traffic loader 内可任意穿插的 phase。它们各自
拥有独立、串接在成功 Traffic normalization 之后的失败原子边界：

1. Scenario/Spatial pairing：沿用 Manifest/Traffic/Spatial 的现有加载顺序，随后校验
   Waiting/Conflict geometry 的 unknown/duplicate/type/frame 与 Traffic pairing；
2. World initialization：选择显式 pinned policy/effective date，编译
   `everConflictEligible`、stream/profile totality、target-profile priority，并验证
   signal protected-conflict coherence；
3. initial/spawn/replacement/runtime route assignment：先完成各 command 既有的
   handle/cursor shape 与 range normalization，再执行 current v0.10
   `(ParticipantClass, Route)` static access validation，再按实际 VehicleProfile
   执行 pending Waiting/conflict occurrence static feasibility validation，最后执行
   新增的 stateful occurrence-interior bootstrap 与其他 runtime capability guards。

每一层只在上一层成功后开始；不同 public API 的错误不得伪装成一个可以跨层重排的
全局 first-error 序列。

first-error 规则：

- phase 小者优先；
- 同 phase 按 wire declaration order；
- derived pair/cycle 按
  `(owner declaration index, first member index, second member index)`；
- external ID 只在 normalization 中用于显式 ASCII `canonicalRank` 编译和
  diagnostics；不得按 hash iteration 顺序或在 hot path 做字符串排序；
- Data 报 JSON path/unit/shape，Core 报 domain invariant；Data 不复制 Core 行为校验；
- permutation tests 只允许在语义声明为 unordered 的集合上比较规范结果，ordered
  path/Gate/stream sequence 不做排序掩盖 authoring 意图。

## 14. Public Core 与 Adapter observation

候选只读 API：

- resolve/iterate WaitingZone、ConflictZone、ParticipantStream、policy handles；
- query parent Junction/ManeuverPath、Gate range、passage anchors；
- query Route compiled Gate/Waiting/Conflict occurrences；
- query vehicle `ManeuverTraversalSnapshot`；
- query WaitingZone occupancy/capacity/ordered members；
- query ConflictZone occupied/reserved snapshot；
- query latest committed Gate decision batch/record attribution；其中
  historical grant outcome 不可复用为当前 permission；
- ordered events/batch snapshots。

不公开：

- mutable occupancy/reservation；
- arbiter scratch、winner injection 或 policy override；
- raw dense index/range；
- Adapter-provided collider result；
- host wall-clock policy switching；
- route-independent “canGo” boolean（缺失 occurrence/vehicle/safety 上下文会误导）。

Adapter 职责：

- render StopLine/Gate/Waiting/Conflict region；
- 显示 indication、candidate/grant/deny attribution；
- 绑定 actor 与只读 vehicle state；
- debug draw queue/stream/reservation；
- 选择 presentation LOD。

Adapter 不得移动 authoritative progress、修改 queue order、授予 reservation、重新
解释 jurisdiction 或把 visual overlap 回写为 behavior。

## 15. 跨层影响矩阵

| 层               | 后续影响                                                                        | #235 是否实现              |
| ---------------- | ------------------------------------------------------------------------------- | -------------------------- |
| Core static API  | Waiting handles/registry/route metadata 已交付；Conflict/Policy 待后续          | #281 部分交付              |
| Core runtime     | state、capacity constraint、arbiter、top-two frontier、claim、grant/reservation | 否                         |
| Traffic Data     | Traffic 0.10 WaitingZone wire/loader/constructors                               | #281 已交付                |
| Spatial          | Waiting/Conflict canonical 3D region、pairing/validation                        | 否                         |
| Adapter API      | readonly snapshots、batch query、debug attribution                              | 否                         |
| Schema           | v0.10 已按 immutable provenance 发布并通过 live byte-equality 验证              | #281 已交付                |
| Fixtures         | multi-Gate/WaitingZone static fixture 已交付；Conflict/runtime 组合待后续       | #281 部分交付              |
| Scenario/example | 中国多阶段/无保护转向示例与 deterministic policy pin                            | 否                         |
| Authoring        | geometry-assisted candidate generation、显式确认、policy provenance             | 否                         |
| Docs/reference   | ADR/design、验证报告、性能证据、closure review                                  | 本 Issue 只交付 ADR/design |

## 16. 验证矩阵

### 16.1 Static/normalization

- multi Gate order、duplicate transition、out-of-range、StopLine mismatch；
- WaitingZone same-path/order/overlap/capacity；
- PathAnchor canonical endpoint、range、entry-before-exit；
- stream/Junction coherence、zone member unique、passage duplicate；
- passage crossing next Gate、Waiting/Conflict overlap；
- regulation interval/version mismatch、`evidenceRefs` element shape/duplicate，且
  验证不联网、不把 opaque provenance 当作本地 entity reference；
- AccessRule omitted provenance 保持合法，只有显式 regulation mismatch 被拒绝；
- yield self-edge/cycle、class selector omitted/present-empty/duplicate/unknown/
  specificity ambiguity、`everConflictEligible`、stream-profile totality、gap
  required/forbidden；
- target-profile priority 与 simultaneous protected conflict；
- 同一 Gate coverage 含多个 subject streams 时，对同一 profile 解析全部 subject
  rules 并取 minimum effective priority；per-cell yield/gap 保持各自 rule，
  permutation 不得变为任选一条。pure Waiting admission 使用 absent-priority rank，
  不创建默认 stream rule；
- subject stream 覆盖 zone A/B、yield target 只覆盖 A 时，normalization 只为 A
  编译 exact target cell，B range 为空；不得查询 missing `(B, target)` cell，也
  不得误要求 target 覆盖全部 subject zones；
- exact first-error phase 与 JSON path。

### 16.2 Route compilation

- repeated edge、repeated ManeuverPath、multiple occurrence identity；
- occurrence Gate mapping formula；
- Gate coverage/clearance ranges、physical span segmentation、next-boundary fast
  index；
- Route 注册保持 profile-agnostic；同一 Route 对短车型 static feasibility binding
  成功、对无法容纳的长车型原子失败，且无关长车型不毒化 Route 注册；
- current/upcoming/repeated forward ConflictPassage occurrence index；
- Route occurrence 到唯一 static `(ConflictZone, ParticipantStream)` passage cell 的
  mapping；
- dynamic Route registration/removal/rebind；
- route terminal inside Waiting/Conflict rejection；
- initial/spawn/replacement cursor 位于 stateful occurrence interior 的
  capability guard；
- registration failure atomicity与 retained memory。

### 16.3 Runtime/state

- PreGate/Committed/Waiting/Clearing 正常迁移；
- multiple WaitingZone cycle；
- entry crossing 后仍移动的 Committed vehicle 保留 active membership；stop/resume
  只切 phase，release/shared-boundary replacement/despawn 原子更新 membership、
  occupancy、intrusive links 与 admission sequence；
- full count、physical tail storage、variable vehicle length；
- 同 WaitingZone 多辆车同 tick crossing 时按 post-step physical front-to-back
  顺序分配 zone-global admission sequence，且与 reducer/update order permutation
  无关；unused claim 不消费 sequence、counter overflow 全 step rollback；
- yellow before/at/after crossing；
- signal changes after committed；
- protected but occupied zone；
- permissive gap accept/reject boundary；
- ETA 的 `d=0`、`a>0` directed-bound 公式、`a=0/v>0`、`a=0/v=0`、
  segmented route-distance lower enclosure、millisecond predecessor/successor
  float 与 finite-intermediate overflow；
- empty approach、OutsideHorizon pass、Unprovable no-grant；
- normalization 中 static AccessRule-denied synthetic foe 不贡献 approach frontier，
  public binding 不能创建该 active vehicle；occupied/reserved authority 仍 fail
  closed；
- foe 位于 current Maneuver occurrence 前但在 horizon 内，以及 repeated upcoming
  occurrence；
- looping/repeated Route 的 exact-subject self contribution 被排除，但其他 vehicle
  contribution 仍生效；top-two distinct owner 的 Unprovable/Finite/tie 组合；
- lead equality no-grant、lead +1 ms grant、lag equality grant 与 checked-add
  overflow；
- passage 在 `[T,T+D)` clear 时 `lastClearTimeMs = T+D`，下一 tick elapsed 为 0；
- equal priority arrival tie；
- unused grant expiry 且 committed maneuver state 不变；
- unused Waiting/downstream claim expiry、crossing 后 committed claim、tail-clear/
  despawn release 与 failed-step rollback；
- latest-decision batch 的 `NotEvaluated`/`NotRequired`/`Granted`/`NoGrant`、
  multi-reason precedence、vehicle-update record order 与不可复用性；
- Gate/edge boundary 后的 internal Conflict anchor event route order 与同 anchor
  tie-break；
- reservation acquire/clear/release；
- route completion/despawn/replace guard；
- failed step state/event/tick/time unchanged。

### 16.4 Safety/composition

- current static AccessRule deny 在 route binding 阶段先于 candidate 原子拒绝；
- static access 通过后，Waiting empty-storage/conflict tail-clear 的
  profile-route-cursor feasibility 在绑定阶段原子拒绝永久不可能组合；
- future time-segment AccessRule deny wins over signal candidate；
- signal deny excludes candidate；
- grant never bypasses leader；
- leader/next Gate/RouteEnd 造成 downstream storage 不足时 no-grant；
- vehicle-length + minimum-gap 恰好命中/越过 clearance boundary，且 storage proof
  与 `tailCleared` 在 tolerance predecessor/equal/successor 上同构；
- disjoint ConflictZone/Junction 的 candidate 争用同一 physical downstream span 时
  stable winner/no partial bundle；互不共享 physical span 时互不阻塞；
- candidate claim 位于 existing claim 前/后两种相邻顺序，均使用实际 follower
  owner 的 minimum gap；
- 同 WaitingZone 多 candidate 不超出 maxOccupancy，且不消费同 tick staged leave；
- hard projection prevents no-grant crossing；
- multiple zones acquired atomically；
- conflict winner cannot cause incompatible double reservation；
- no-overlap/minimum gap after every successful tick；
- normal wait/no-grant never becomes error。

### 16.5 Determinism/performance

- same seed/commands exact state and ordered event digest；
- candidate proposal evaluation/completion permutation 在 stable reducer 后产生 exact
  same grant/state/event digest；
- static declaration permutation 后按 external ID 对齐的 grants、NoGrant attribution、
  same-anchor events 与 canonical ranks 语义等价；raw handle permutation 不改变结果；
- dynamic Route/occurrence 数增长只增加 occurrence metadata/contribution visits，
  不增加 static frontier cell count；
- no hot string lookup/hash iteration；
- 一万 p95/hard budget、zero allocation；
- 十万 scaling/retained memory、forward-passage visits 与 clearance-boundary visits；
- debug/release profile and benchmark hardware evidence。

## 17. 后续实施切片

G1 已接受；#280 已将后续生产化范围拆为以下独立 Issue。每个 Issue 自行完成
G0-G4 与元数据审计：

1. **#281 multi-Gate + WaitingZone static/Data**：已解除 entry-only guard，新增
   WaitingZone registry，并从 Traffic 0.9 原子升级到 0.10 source contract，
   完成 schema/loader/fixtures、Route occurrence compilation 与
   profile-route-cursor static feasibility binding，不激活 tick runtime。
2. **#282 WaitingZone runtime**：vehicle state、capacity/physical storage、
   queue、admission claim、constraint/hard guard/events。
3. **#283 Conflict static + Spatial**：ConflictZone/ParticipantStream/
   PathAnchor、route passage compilation、可选 Spatial pairing 与 authoring
   validation。
4. **#284 policy + ConflictArbiter runtime**：RegulationIdentity、stream
   rules、gap profile、top-two forward approach frontier、downstream-clearance
   guard、single-writer claim ledger、candidate/grant/reservation、失败原子性。
5. **#285 cross-layer validation/performance**：canonical fixtures、Data
   round-trip、Adapter observation、一万/十万、独立 closure review。

切片顺序建议：

```text
#281 multi-Gate/Waiting static
  ├─> #282 Waiting runtime ───────────┐
  └─> #283 Conflict static/Spatial ───┴─> #284 policy/arbiter runtime
                                              └─> #285 validation/closure
```

#282 与 #283 在共同前置 #281 完成后可以并行；#284 不得在二者的 runtime/static
identity、coverage 与 occurrence compilation 完成前先行；#285 最后验证组合后的
Data/Spatial/Core/Adapter 契约、确定性与性能。

#264 消费本设计与 #237 的 accepted 结论后，才拆 JunctionGroup、环岛、停车连接与
互通组合设施；不得在 #264 内重新定义 ConflictZone/right-of-way owner。

## 18. G1 接受结论

#235 的冻结点：

1. WaitingZone 是同一 ManeuverPath 上 entry/release Gate 有界的一等实体；
   admission sequence 由 zone-global counter 按 successful crossing 后的 physical
   queue order 原子分配；
2. 多 Gate 仍使用 ADR 0017 identity，coverage 与 occurrence 在 Route 注册期编译；
3. ConflictZone/ParticipantStream 属独立 ConflictRegistry，显式 Traffic 关系拥有
   behavior，Spatial 只拥有 canonical 3D geometry/validation；
4. final vehicle-specific right-of-way owner 是 Core ConflictArbiter；
5. policy 复用 ADR 0018 jurisdiction/version provenance，Scenario 显式 pin，运行时
   不读墙钟、不隐式热切换；
6. tick-local grant 只有 crossing 成功才升级为 reservation；
7. v1 gap frontier 覆盖 current/upcoming occurrence，以 directed lower-bound ETA
   区分 Finite/OutsideHorizon/Unprovable，并在 static zone-stream passage cell 上以
   top-two distinct owner 支持 exact subject self-exclusion；概率/长时域预测或
   per-Junction solver dispatch 需独立 G1；
8. grant 前 mandatory downstream-clearance guard 必须证明车尾可在下一未获准 Gate
   前清空全部 coverage zone，并与 Waiting/Conflict resources 一起原子取得
   committed + earlier-staged 可见的 physical downstream claim；storage/tail-clear
   tolerance predicate 必须同构，相邻 claims 使用实际 follower 的 minimum gap；
   Route 注册只编译 operands，实际车型的永久可行性在 profile-route-cursor 绑定期
   校验，不能让无关长车型拒绝整条 Route；
9. PreGate/Committed/Waiting/Clearing committed 状态与 latest-decision
   batch/zone/reservation/event 在 successful step 原子提交；batch 区分
   NotEvaluated/NotRequired/Granted/NoGrant，tick-local grant 不伪装成跨 tick
   vehicle state；
10. arbitrary-progress spawn/initial/replacement 不得猜测 stateful occurrence interior
    authority；v1 capability guard 拒绝需 bootstrap 的 cursor；
11. allow/grant 永不覆盖 leader、safe-speed、RouteEnd、minimum-gap 或 no-overlap；
12. candidate proposal 可并行，winner 只由 stable single-writer arbitration 决定；
    static permutation tie 使用预编译 canonical rank，raw handle 不决定业务或事件
    顺序；
13. steady tick 不做 external-ID/path/geometry catalog scan 或 per-vehicle allocation；
14. current Traffic v0.10 `AccessRegistry`/`AccessCell` 是静态准入唯一 SSOT；
    route binding 已拒绝 static deny，#281 不复制求值器或修改已发布 v0.9；
15. implementation 必须以独立切片和一万/十万证据推进，#235 本身不生产化。

#235 G1 已接受本文设计输入；本文仍不授权 production 实现，后续各实现切片必须独立完成 G0-G4。
