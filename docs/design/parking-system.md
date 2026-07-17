# Parking System 设计

**文档状态**: Accepted  
**最后更新**: 2026-07-17
**适用范围**: v0.5 Parking 的 current 静态领域/data contract、runtime authority/commands、ParkingStop/route 集成、确定性、失败原子性与性能边界
**实现状态**: #105 已冻结设计与 ADR 0010；#106/#107 已交付 substrate 与 static/current data；#108 已交付 runtime/commands；#109 已交付 ParkingStop/arrival/traversal/release/events 与 capability activation；#110 已交付 milestone 全面验证；#19 已完成独立收口审阅

**关联文档**:

- `../architecture.md`
- `../roadmap.md`
- `../adr/0003-runtime-tick-and-determinism.md`
- `../adr/0005-core-identity-and-handle-model.md`
- `../adr/0006-vehicle-following-control-and-safety.md`
- `../adr/0007-traffic-data-crate-and-loader-boundary.md`
- `../adr/0008-pre-1.0-data-format-version-policy.md`
- `../adr/0009-signal-indication-gate-and-policy-separation.md`
- `../adr/0010-parking-binding-and-vehicle-lifecycle-authority.md`
- `core-id-handles.md`
- `lane-graph.md`
- `route-system.md`
- `vehicle-following.md`
- `signal-system.md`
- `data-format.md`
- `data-loading.md`

## 1. 目标、状态与分阶段事实

v0.5 的目标是在 v0.4 Accepted fixed tick、typed handles、lane graph、route、Vehicle Following、Signals 与 current data contract 上，交付基础、确定且可验证的 Parking 闭环：

- 支持停车场内显式泊位和道路旁专用路边泊位/停车带；
- 支持 individual ParkingSpace、optional ParkingArea grouping 与静态 entry/exit/pose；
- 支持 caller-selected reservation、arrival、explicit commit、leave、route resume/rebind 与 lifecycle cleanup；
- 保持 live vehicle identity，并明确 lane position 与 parking pose 的 authority transfer；
- 将 ParkingStop 与 SignalStop、RouteEnd、leader、safe-speed 和 no-overlap 放入同一 fixed-tick pipeline；
- 公开不可变 static registry、borrowed committed snapshot、typed commands/records/events/errors；
- 冻结 current data format 0.5、migration、canonical fixtures 和验证/性能门禁。

本文既保留 #105 Accepted 实现输入，也记录 #106-#110 current 交付事实；#19 的独立完成判断与剩余风险见 [`../reference/v0.5-closure-review.md`](../reference/v0.5-closure-review.md)。分阶段事实必须保持：

1. #105 当时只新增 design/ADR；该历史阶段不再代表 current tree。
2. #106 已交付无 Parking public types 的 lifecycle/route-distance/command-spatial 性能底座；验证基线见 [`../reference/v0.5-lifecycle-substrate-validation.md`](../reference/v0.5-lifecycle-substrate-validation.md)。
3. #107 已把 static Parking、schema、private DTO、loader、fixtures 与 current docs 原子切换到 production 0.5。
4. #108 已交付 runtime commands/query，并用窄 capability guard 保护当时尚未激活的 Reserved step。
5. #109 已完整交付 ParkingStop、arrival、traversal/release/events 并解除 guard。
6. #110 已固化端到端、性能、allocation/memory 和 validation artifact；#19 已完成最终独立 closure review。

#110 的 milestone 全面验证已形成 current production source 与 validation artifact；#19 随后独立复核治理、设计、实现、数据、验证、依赖与剩余边界，未以子切片 G4 替代父级 closure。

### 1.1 #106 已交付边界

#106 只建立后续 Parking commands 可复用的私有 substrate：overflow-safe route occurrence distance、O(1) resolver removal、stable tombstone order 与确定性 compaction、exact route reference、physical-edge-local command index、full-scan oracle、allocation/retained-memory instrumentation 和 Criterion matrix。

它没有增加 `ParkingArea` / `ParkingSpace` / `Parked`、schema 0.5、loader、reservation/binding、ParkingStop、leave/commit command 或 Adapter API。#107 随后补齐 static types/data，#108/#109 交付 production runtime behavior，#110 完成端到端验证；Adapter API 仍不在 v0.5 范围内。

### 1.2 #107 已交付边界

#107 已交付 `ParkingArea` / `ParkingSpace`、opaque dense handles、immutable `ParkingRegistry`/resolvers、entry/exit anchors、edge-relative geometry、`InitialTrafficData` foreign-graph rebind、`CoreWorld::parking()`，以及 current 0.5 schema/loader/fixtures。Static normalization 使用固定输入顺序；10k all-vacant registry 不进入 step hot path。

#107 不包含 reservation、occupancy、commands、ParkingSnapshot、`VehicleStatus::Parked`、ParkingStop、arrival/leave/release events 或 Adapter API。

### 1.3 #108 已交付边界

#108 交付 Core-owned dense Parking authority、增量 global/area counts、borrowed `ParkingSnapshot`、`VehicleStatus::Parked`、despawn release，以及 reserve/cancel/commit/leave/rebind/parked-spawn 六条同步 command。Commit/parked spawn/leave 原子切换 lane 与 parking position authority；Parked 不参与 occupancy、leader、motion 或 traversal。Local leave validation 使用 command spatial index 缩小候选，并以 route-aware final authority 与保留的 full-scan oracle/property tests校验。

#108 不交付 moving Reserved step、ParkingStop、step-side arrival、route-completion release或 Parking events。只要 committed `reserved_count > 0`，step 在 delta/time/integrity 检查后返回 `ParkingVehicleCapabilityUnavailable`；all-Vacant、unbound 与 occupied-only world保持可运行。该 guard 由 #109 在完整 activation 同一切片中解除。

### 1.4 #109 已交付边界

#109 从同一 committed snapshot(T) 生成 SignalStop、ParkingStop 与 RouteEnd，并以 canonical `Signal -> Parking -> RouteEnd` tie attribution归约最严格 admissible motion；Parking spatial projection先于 Following projection，traversal只消费 final travel并同时 guard Gate/Parking boundaries。Reserved target在 command-time缓存 first reachable occurrence，tick内通过 O(1) reverse binding与 route-distance index读取，不增加 full-S/full-V Parking pass。

Successful step 现在可一次提交 arrival false-to-true、Dormant route-completion reservation release、vehicles/parking/signals/tick/time与完整事件总序。`VehicleParkingStopProjectionApplied`、`VehicleParkingArrivalReached`、`ParkingReservationReleased` 已成为 current Core events；#108 的 `ParkingVehicleCapabilityUnavailable` variant仅保留 legacy API identity，合法 production world不可达。

## 2. 非目标

v0.5 明确不实现：

- Core 自动选位、最近泊位、成本选择、满位 reroute、排队、调度或随机分配；
- reservation TTL、wall-clock timeout、dwell scheduler 或自动离开；
- 共享正常行车道上的 on-road parking、双排停车或动态缩窄道路容量；
- 自由空间 pathfinding、倒车轨迹、turn radius、Ackermann/车辆动力学或碰撞物理；
- space-space、space-lane 世界几何重叠、vehicle-size fit 或 maneuver feasibility；
- valet、充电、付费、门禁、停车楼层导航或停车场运营系统；
- permissive intersection/conflict/reservation、lane changing 或 alternate incoming branch merge gap；
- dynamic ParkingArea/ParkingSpace definition lifecycle；
- Engine Adapter ABI、mesh/prefab、动画、调试 UI 或 authoring UI；
- runtime schema `$id` 网络解析、100k realtime SLA、跨 CPU bit-level determinism；
- million-scale partition/parallel/mesoscopic Parking runtime。

## 3. 术语与产品边界

- **ParkingSpace**：具有稳定 identity、可被唯一车辆 Reserved/Occupied 的最小静态泊位事实。
- **ParkingArea**：spaces 的 optional 逻辑分组，可表达停车场或专用路边停车区；不拥有匿名 capacity。
- **LaneAnchor**：`EdgeHandle + edge-local progress` 的 lane-relative attachment。
- **Entry anchor**：Active reservation owner 在 park commit 前必须到达的 route-relative边界。
- **Exit anchor**：leave 成功后 Active vehicle 被插入目标 route occurrence 的边界。
- **Parking binding**：同一 committed aggregate 中的 `space <-> vehicle` 一对一关系。
- **Dormant**：reservation 有效，但当前 route 剩余 sequence 没有 reachable entry occurrence。
- **Approaching**：存在 selected entry occurrence，但尚未满足 arrival predicate。
- **Arrived**：exact Reserved pair 已在 selected entry 处以精确零速满足 commit readiness。
- **ParkingStop**：Core 私有的 route-relative zero-speed spatial target，不是 VehicleStatus。
- **Position authority**：Adapter 应以哪个 Core source 解释车辆的表现位置。
- **Travel-lane occupancy**：Vehicle Following 用于 leader/no-overlap 的 tick-local物理占用视图，不是 parking occupancy authority。

停车场内部通道、接入道路和专用路边停车带的行驶部分继续复用普通 LaneGraph/Route。`ParkingArea` 不引入第二套路网或行为分支；停车场和路边专用泊位使用同一 `ParkingSpace` runtime 模型。

## 4. Static Parking domain

### 4.1 Identity 与 handles

新增两个 external-ID domain：

```text
ParkingArea
  externalId

ParkingSpace
  externalId
  areaExternalId?
  entry
  exit
  geometry
```

- ID 使用 current external ID ASCII token、长度、大小写和 domain-local uniqueness 规则。
- Core normalization 后使用 opaque `ParkingAreaHandle` / `ParkingSpaceHandle`。
- 两类 handle 是随 immutable static registry 建立的 dense world-scoped token，不使用 generation，不公开 index/ordering/numeric conversion，也不进入 external package。
- Definitions 在 world 生命周期内 immutable；v0.5 不支持 runtime add/remove/mutate。
- Empty Parking registry 合法。

### 4.2 Area membership

- `ParkingSpace.areaId` 是 optional 且唯一的 membership 输入；standalone space 合法。
- `ParkingArea` 不反向持久化 `spaceIds`。Registry 建立 area -> member spaces reverse index，顺序为 space normalization/input order。
- 已声明但没有 member space 的 orphan area 非法。
- Area capacity 与 runtime counts 由 member spaces 派生；不保存可独立写入的 capacity/availability。
- v0.5 不增加影响 Core 行为的 `parkingLot | curbside` kind。

### 4.3 Entry/exit lane anchors

每个 space 必须提供 entry 与 exit：

```text
ParkingLaneAnchor
  edge: EdgeHandle
  progress: f64 distance newtype
```

- Entry/exit 可以完全相同，也可引用不同 edge，以表达单向停车场通道。
- Anchor 只表示 park commit 前的到达边界和 leave 成功后的 route 插入边界，不表示 maneuver path。
- Anchor edge 必须存在。
- Progress 必须 finite，且严格满足：

```text
EDGE_BOUNDARY_EPSILON
  < progress
  < edge_length - EDGE_BOUNDARY_EPSILON
```

- 不做 friendly clamp。接近 edge endpoint 的泊位由 authoring 拆分/延伸 edge，避免 Parking 与 StopLine/Gate/edge transition 共享同一点的未设计优先级。
- Space 不持有 RouteHandle、route ID 或 route occurrence；具体 occurrence 只在 vehicle command/step 上下文解析。

### 4.4 Edge-relative rectangular pose

最终 parked pose 以 entry edge 在 `entry.progress` 的正向切线为局部基准：

```text
ParkingSpaceGeometry
  lateral_offset
  heading_offset_radians
  length
  width
```

- 正 lateral 表示沿行驶方向的左侧；`abs(lateral_offset) > GEOMETRY_GAP_EPSILON`。
- 正 heading 表示逻辑 road frame 中逆时针；canonical 范围为 `[-π, π)`。
- Length 沿 parked heading，width 与其正交；二者 finite 且严格大于 `GEOMETRY_GAP_EPSILON`。
- Core 保存、校验并 query normalized anchors/geometry，但不计算 world transform。
- Adapter 使用自身 edge geometry 映射 position/orientation，并处理引擎坐标轴差异。
- Core 不拥有 lane width、centerline polygon 或世界 geometry，因此 v0.5 不证明 rectangle 与 lane/其他 space 不重叠。

### 4.5 Normalization 与稳定顺序

Parking static normalization 顺序固定为：

```text
areas identity
-> spaces identity + optional membership
-> entry/exit edge anchors
-> geometry
-> orphan areas
-> ordered reverse indexes
```

Array error、handle allocation、definition iteration、member iteration和 first-error 顺序使用 normalization/input order；不得依赖 `HashMap`、handle 数值或 external-ID sort。`InitialTrafficData` final assembly 必须把 Parking registry 按自身 LaneGraph 重新解析/复验，禁止携带另一张 graph 的 dense handles。

## 5. Runtime authority 与 committed invariants

### 5.1 唯一 committed aggregate

Core 私有 Parking runtime aggregate 维护两个 O(1) 视图：

```text
space -> Vacant
       | Reserved { vehicle }
       | Occupied { vehicle }

vehicle -> None
         | Reserved { space }
         | Occupied { space }
```

两个方向一起验证/提交，不是两个 authority。不得把 mutable binding 放入 static registry、VehicleState、route、lane occupancy、Adapter 或 public snapshot。

### 5.2 Strong invariants

- 每个 space 至多绑定一辆 live vehicle；每辆 live vehicle 至多绑定一个 space。
- `Reserved(space, vehicle)` 要求 owner 为 live `VehicleStatus::Active`。
- `Occupied(space, vehicle)` 当且仅当 owner 为 live `VehicleStatus::Parked`。
- `Parked` 必须且只能拥有 exact Occupied binding，并强制 speed/acceleration 为精确零。
- `Stopped` 与 `Completed` 不得持有 Reserved/Occupied binding。
- `Parked` 不进入 lane occupancy、leader、longitudinal motion、route traversal 或 completion。
- 加载 static data 后全部 spaces 初始 `Vacant`。
- 正常运行时转换只有：

```text
Vacant
  -- reserve --> Reserved
  -- parked spawn construction exception --> Occupied + Parked

Reserved
  -- cancel --> Vacant
  -- explicit commit --> Occupied + Parked
  -- route completion/despawn cleanup --> Vacant

Occupied + Parked
  -- successful leave/despawn --> Vacant
```

### 5.3 Counts

Global/area `ParkingCounts` 固定包含：

```text
capacity
vacant
reserved
occupied
```

- `available == vacant`；reservation 降低 availability，但不增加 occupied。
- Standalone spaces 进入 global counts，不进入任何 area counts。
- Counts 在 command/step atomic commit 中 O(1) 增量更新，可由 exhaustive audit 重算，但不是独立 authority。
- Release hot path 只检查 touched pair 与 O(1) integrity sentinel；完整 forward/reverse/count audit 只进入 debug/test/model helper。

## 6. Reservation commands

### 6.1 Reserve

概念 API：

```text
reserve_parking_space(vehicle, space)
  -> ParkingReservationRecord
```

固定判定顺序：

1. Resolve live vehicle。
2. Resolve space。
3. Exact Reserved pair -> `AlreadySatisfied`。
4. Vehicle 必须 `Active`，否则 status mismatch。
5. Vehicle 必须 unbound，否则 already-bound error。
6. Space 必须 `Vacant`，否则 unavailable error。
7. 一次提交双向 Reserved binding、target cache/count delta -> `Applied`。

Reserve 只验证 identity/lifecycle/binding/availability；不验证 route 可达性、ETA、最近距离或 maneuver geometry，也不修改 route、cursor、speed、acceleration 或 status。

若当前 route 没有 reachable entry，reserve 仍成功并缓存 `None`，snapshot 显示 `Dormant`。不得返回 `ParkingEntryUnreachable`；该错误只属于 reserved-route rebind。

### 6.2 Pair-specific cancel

概念 API：

```text
cancel_parking_reservation(vehicle, space)
  -> ParkingReservationCancellationRecord
```

- Resolve vehicle/space 后，exact Reserved pair 原子释放并返回 `Applied`。
- Vehicle unbound 且 space Vacant 是窄幂等 `AlreadySatisfied`。
- 其他组合一律 reservation mismatch；不能按 space 强制释放、删除其他 owner 或释放 Occupied。
- 取消只改变 binding/cache/counts；vehicle 继续保持 Active 并在后续 tick 通过普通 pipeline 恢复。

### 6.3 Competition、replacement 与 timeout

- Commands 在 step 之间逐条同步执行；第一个成功提交者获胜。
- 不收集“同 tick requests”，不按 external ID、handle、线程、随机或容器顺序仲裁。
- 同一 vehicle 换位必须显式 cancel old 后 reserve new；两条命令不是事务，后者失败不恢复 old。
- v0.5 不提供 force steal、queue、batch transaction、TTL、deadline 或 implicit expiry。

## 7. Approach、arrival、commit 与 position authority

### 7.1 First-reachable entry occurrence

对 exact Reserved Active vehicle，从当前 route cursor 选择第一个尚未越过的 entry-edge occurrence：

1. 当前 occurrence 的 physical edge 匹配，且 `entry.progress + EDGE_BOUNDARY_EPSILON >= edge_progress`，选择当前 index。
2. 否则沿有限 route sequence 选择第一个后续匹配 occurrence。
3. Repeated edge 使用最早可达 `route_edge_index`；caller 不能跳过早 occurrence 隐式选择晚 occurrence。

Reserve/rebind command-time 允许一次 O(remaining route length) 搜索并缓存 private `ApproachTarget`。Cache 不是 authority；cancel、commit、leave、route completion、rebind、despawn 必须替换/失效它。

### 7.2 Overflow-safe route distance

Route 注册时可以建立 occurrence prefix，但不得改变现有 route 合法性：

- 每条 edge length finite 仍是 route 数值要求；不能只因大量 finite edge 的 total sum 溢出 `f64` 就新增“整条 route cumulative total 必须 finite”。
- #106 必须采用 overflow-safe、segmented 或等价 private representation。
- First-reachable occurrence 选择必须确定；当 target 进入有限 lookahead 后，tick route-distance lookup 为 O(1)。
- 无关 vehicle/route 不能因 prefix total overflow 在 step 中失败，也不得退回 per-tick route scan。

### 7.3 ParkingStop

Reachable reservation 从 tick-start snapshot 产生一个 Core 私有 target：

```text
ParkingStop
  position: selected entry anchor
  reference: vehicle front bumper
  target speed: 0
  desired clearance: 0
  hard clearance: 0
```

Vehicle 始终保持 `Active`。成功 step 可以舒适/紧急减速；无论输入多极端，committed cursor 不得越过 entry。Dormant reservation 不产生 ParkingStop，也不阻断普通 step。

### 7.4 Arrival predicate

只有同时满足以下条件才是 `Arrived/can_commit`：

- exact Reserved pair；
- live `Active` vehicle；
- 当前 occurrence 等于 selected entry occurrence；
- `abs(edge_progress - entry.progress) <= EDGE_BOUNDARY_EPSILON`；
- `current_speed == 0`。

到达成功 step 把 progress 规范化为 exact entry、speed 规范化为正零。`applied_acceleration` 不要求为零；抵达 tick 可以有负的有效平均加速度。若 reserve 前 vehicle 已以 Active + zero speed 位于 entry epsilon 内，可以立即 Arrived，不要求空走一个 tick。

Command 创建 reservation/rebind 后若 snapshot 已是 Arrived，不补造 step arrival event；调用方读取 command 后 snapshot。

### 7.5 Explicit commit

概念 API：

```text
commit_parking(vehicle, space)
  -> ParkingCommitRecord
```

固定判定顺序：

1. Resolve vehicle/space。
2. Exact Occupied pair + Parked -> `AlreadySatisfied`。
3. Vehicle 必须 Active。
4. 必须 exact Reserved pair。
5. 必须满足 arrival predicate。
6. 一次提交 -> `Applied`。

成功 commit 原子完成：

- `Reserved -> Occupied` 与 reverse binding；
- `Active -> Parked`；
- speed/acceleration -> exact zero；
- cursor 保留 route/occurrence并规范化到 entry；
- position authority 从 lane cursor 切换到 space geometry；
- 从下一 committed snapshot 起排除 lane occupancy。

Commit 不推进 tick/time，不修改 route definitions、Signals 或其他 vehicles；不隐式 reserve、等待或自动播放 maneuver。

Arrived 但尚未 commit 的 vehicle 仍是 Active zero-speed lane occupant，会作为 stationary leader 阻挡后车。

## 8. Leave、route rebind、spawn/despawn 与 route lifecycle

### 8.1 Leave input 与 route occurrence

概念 API：

```text
leave_parking {
  vehicle,
  space,
  route,
  route_edge_index
}
```

Caller 必须给出 exact Occupied pair、active RouteHandle 与明确 occurrence。Occurrence physical edge 必须等于 `space.exit.edge`；progress 固定使用 canonical exit anchor，不由 caller 指定。目标 route 可以等于或替换休眠旧 route；Core 不自动选路、搜索路径、等待 gap 或修改 lane graph。

固定判定顺序：

1. Resolve vehicle -> space -> route。
2. Validate route occurrence bounds。
3. Validate occurrence edge == exit edge。
4. Exact target route/occurrence/exit progress + Active/unbound/zero motion + space still Vacant -> narrow `AlreadySatisfied`。
5. Vehicle 必须 Parked。
6. 必须 exact Occupied pair。
7. Route-aware physical overlap validation。
8. Direct follower emergency-envelope validation。
9. 一次提交 -> `Applied`。

Malformed target 不能被 no-op 分支吞掉。Vehicle 已移动、route 改变或 space 被新 owner 取得后，旧 leave 失去幂等资格，不能 teleport 或释放新 owner。

### 8.2 Safe insertion

Core 在 vehicle 仍为 Parked/Occupied 时构造 speed/acceleration 为零的 candidate Active occupant at exit：

- 必须与全部 Active/Stopped lane occupants 无物理 overlap，包括 same edge、相邻 boundary、repeated occurrence 与双向 route visibility。
- 对会把 candidate 视为新 stationary direct leader 的全部 Active followers，插入不能迫使它们只能依赖 geometry hard projection 才避免 overlap。
- 不要求 comfort `min_gap`；emergency-feasible 的较小 gap 可以由后续 Following 自然恢复。
- Stopped follower 只需满足 geometry。
- 检查保守地不借用当前 SignalStop/ParkingStop 来放宽 gap，也不修改 follower、tick 或 events。
- Alternate incoming branch 的 merge/conflict geometry 仍是非目标。

Local spatial index 只用于缩小候选，不能退化为 same-edge-only。必须以保留的 full-scan reference oracle/property tests 证明 same-edge、adjacent boundary、repeated occurrence、candidate/existing 双向 visibility 和全部 direct followers 无 false positive/negative。多个冲突/error candidate 以 stable `vehicleUpdateOrder` 选择，不依赖 cache/container/handle。

### 8.3 Successful leave commit

一次提交同时完成：

- route/cursor -> caller target occurrence + exit progress；
- position authority -> route cursor；
- `Parked -> Active`；
- speed/acceleration -> zero；
- `Occupied -> Vacant` 与 reverse binding -> None；
- 从 committed state 起进入 lane occupancy。

失败保持 vehicle、space、route、counts、bindings、update order、tick/time、Signals 和 events 全部不变。Adapter 可播放视觉驶出动画，但不能继续占用 space 或延迟 lane participation。

### 8.4 Reserved-route rebind

概念 API：

```text
rebind_reserved_vehicle_route {
  vehicle,
  space,
  route,
  route_edge_index
}
```

只允许 exact Reserved Active pair：

- 先 resolve handles/occurrence，检查 Active + exact reservation。
- Exact same route/occurrence 为 `AlreadySatisfied`，即使仍 Dormant。
- Target occurrence 的 physical edge 必须等于当前 physical edge；current progress、speed、acceleration、status 不变，不能 teleport。
- 从 target occurrence/current progress 起必须存在 reachable entry，否则 `ParkingEntryUnreachable`。
- Candidate route mapping 必须通过 route-aware no-overlap。
- 成功只替换 route handle/index 与 ApproachTarget；reservation 不变。

这是 Parking 专用受限 rebind，不是通用 Active reroute API。

### 8.5 Route completion cleanup

Dormant Reserved Active vehicle 若到达 route end而未 cancel/rebind：

- 成功 step 把 `Active -> Completed`、`Reserved -> Vacant`、reverse binding -> None 与 counts/event candidate 一次提交。
- Release 是 vehicle lifecycle cleanup，不是 timeout，也不把 reservation 转给其他 vehicle。
- Reachable ParkingStop 严格位于 route end 前，因此 arrival 与 route completion 互斥。
- 任何后续 step error 都不得提交 completion、release、signal(T+D)、tick/time 或部分 events。

### 8.6 Despawn cleanup

`despawn_vehicle` 对 Parking 的扩展：

- Unbound：保持现有行为。
- Reserved：release + reverse clear + despawn 一次提交。
- Occupied/Parked：release + reverse clear + despawn 一次提交，不要求先 leave。
- 失败不得消费 vehicle slot/generation/update order 或留下 stale binding。
- 成功后旧 VehicleHandle 按既有规则 stale；despawn record 返回 optional Parking release。

### 8.7 Atomic parked spawn

专用 `spawn_parked_vehicle`：

- 验证 external vehicle ID、profile、route、space Vacant 与 route occurrence。
- Occurrence physical edge 必须等于 space entry edge；cursor 规范化到 entry progress。
- 一次提交 vehicle allocation、`Parked`、zero kinematics、Occupied binding 与 reverse view。
- Parked 不进入 lane overlap/occupancy，因此不得调用普通 Active spawn 的全车 overlap scan。
- 失败不得消费 slot/generation/update sequence 或占用 space。

普通 `spawn_vehicle` 不得用 `VehicleStatus::Parked` 制造半个 invariant；external static package 也不持久化 initial parked vehicles。

### 8.8 Route reference 与 removal

Reserved/Occupied/Parked 都是 live vehicle 并继续持有 active route reference。`remove_route` 必须拒绝被任一 live vehicle 引用的 route；多个引用时返回 stable `vehicleUpdateOrder` 中第一个 live vehicle，而不是 slot/map/handle order。Leave/rebind 原子替换旧 route reference，despawn 才移除引用。

## 9. Fixed-tick composition

### 9.1 Status participation

- `Active`：进入 occupancy、leader、constraints、projection 和 traversal；Reserved/Approaching/Arrived 均属于 Active。
- `Stopped`：进入 occupancy并作为 stationary leader，自身 motion zero；不得持有 Parking binding。
- `Parked`：排除 occupancy、leader、motion、traversal/completion。
- `Completed`：沿用现有排除规则。

Parking commands 只发生在 step 之间；next tick snapshot 完整观察 command 后 state，不允许 mid-step command 注入。

### 9.2 Authoritative phases

一次 v0.5 step 的概念 phases：

```text
1. validate delta / tick overflow / time overflow
2. validate committed Parking local/integrity sentinel
3. freeze vehicle / route / parking / signal snapshot(T)
4. build occupancy from Active + Stopped
5. per Active vehicle resolve route-aware leader
6. resolve SignalStop, ParkingStop and RouteEnd from snapshot(T)
7. evaluate IIDM comfort + leader safe-speed + ballistic base motion
8. reduce spatial targets to the strictest admissible motion
9. apply selected spatial hard projection when required
10. solve deterministic no-overlap projection
11. traverse route using final travel and guard Gate/Parking boundaries
12. derive arrival or Completed + reservation release sparse deltas
13. derive signal snapshot(T + D) and signal events
14. atomically commit vehicles + parking + signals + tick/time + events
```

实现可为 scratch reuse 调整纯计算代码位置，但 snapshot authority、依赖方向、error phase order和 observable commit order不能改变。Vehicles 只读取 committed signal snapshot(T)。

### 9.3 Unified stop reducer

每个 Active vehicle 最多形成：

- nearest denied `SignalStop`；
- one reachable `ParkingStop`；
- finite route `RouteEnd`。

三者归一化为 finite/nonnegative route distance、target speed 0、front-bumper clearance 0。以 IIDM + leader safe-speed 的 ballistic base motion 为上界，为每个 target 计算 admissible speed/travel，选择 travel 最小、同 travel 时 speed 最小的共同可行结果。

只有 canonical numeric result 完全相同时才按 `SignalStop -> ParkingStop -> RouteEnd` 做 attribution tie-break；tie-break 不改变 motion，也不是 configurable priority。不得依赖 provider registration、external ID、handle 或 map order。

### 9.4 Observable combinations

- Denied Gate 在 entry 前：先停 Gate；allow 后继续受 ParkingStop 约束。
- Entry 在 denied Gate 前：先停 entry，downstream signal 不推动 vehicle越过 entry。
- Permitted Gates 在 entry 前：按 route traversal 穿越并产生正常 edge events。
- Leader/no-overlap 更近：先停 leader 后方，不满足 Arrived；leader 移动后继续。
- Arrived 未 commit：仍是 Active stationary leader；commit 后 next snapshot 排除。
- Reservation/occupancy 不改变 signal permission；Signals 不取得/释放 ParkingSpace。

### 9.5 Projection、speed mapping 与 traversal guard

1. Reducer 选中的 SignalStop/ParkingStop若把 travel 压到 emergency-minimum以下，可产生一次 spatial hard-projection attribution；exact Signal/Parking tie 只归因 Signal。
2. Spatial-constrained candidates 进入全局 no-overlap；只有进一步压到 emergency envelope以下才产生 Following projection。
3. 同一 vehicle/tick 可同时有一个 spatial projection 和一个更严格的 Following projection，顺序 spatial -> Following。
4. Travel cap 后复用 Vehicle Following 唯一 speed mapping；精确到 zero-speed target 时 speed 规范化为正零。
5. Traversal 只消费 final travel。Denied Gate/Parking boundary 若仍会被越过，返回 internal invariant error并原子失败，不能末尾倒车/修补。
6. 到达 entry 时 snap、zero speed、保持 Active + Reserved，不自动 commit。

### 9.6 Sparse atomic step

- Step 不 clone/copy 全部 S 个 ParkingSpaceState，也不新增独立 full-S/full-V Parking pass。
- `reserved_count == 0` 时，除 O(1) integrity sentinels 外跳过 Parking motion/candidate；all-Vacant 与 occupied-only world 不因 static S 线性付费。
- Nonzero Reserved work 在既有 stable vehicle loop 内通过 O(1) reverse binding + cached target读取，产生 sparse candidate deltas。
- Arrival、route-completion release、counts 和 events 在成功后一次提交。
- Runtime 只检查 touched pairs/local sentinels；完整 forward/reverse/count audit 只用于 debug/test/model。
- Scratch/cache capacity/pointer不是 semantic equality，但任何 candidate failure不得留下 counts/binding/event 部分写入。

### 9.7 I3 transitional guard history 与 current activation

在 #108 已公开 runtime commands/query但 #109 尚未交付 moving capability 时，历史过渡规则为：

- 只在 committed `reserved_count > 0` 的 step 返回 `CoreError::ParkingVehicleCapabilityUnavailable`。
- Empty/all-Vacant、unbound Active 与 occupied-only Parked world 可正常 step。
- Reserve 后仍可 cancel/despawn；exact immediate Arrived pair可 command-side commit。
- Step priority 固定为：delta mismatch -> tick overflow -> time overflow -> committed Parking integrity -> capability unavailable -> normal pipeline。
- Guard 前不得修改 vehicle/parking/signal/tick/time/events；Reserved world不执行 completion，因而不会漏 release。
- #109 已激活完整 pipeline并保留 legacy variant；合法 production world不再返回，current 注释与测试证明其不可达。

## 10. Public Core API boundary

### 10.1 Module、handles 与 static registry

新增 public `parking` module并 re-export主要类型：

```rust
ParkingAreaHandle
ParkingSpaceHandle
ParkingArea
ParkingSpace
ParkingLaneAnchor
ParkingSpaceGeometry
ParkingRegistry
```

Handle只实现 `Clone + Copy + Debug + Eq + Hash`；不提供 public ordering/index/serialization。`CoreWorld::parking(&self) -> &ParkingRegistry` 提供 immutable resolver/query：

```text
area_handle / area / area_external_id / areas
space_handle / space / space_external_id / spaces
area_spaces
space_area
space_entry / space_exit / space_geometry
```

Definition/member iteration使用 normalization order。`space_area` 需要区分 unknown handle 与 valid standalone space。

### 10.2 Borrowed committed snapshot

```rust
CoreWorld::parking_snapshot(&self) -> ParkingSnapshot<'_>
```

Snapshot借用同一个 committed world 的 registry、parking aggregate、vehicle/route和 tick/time；不 clone全量 vector/map，不可写。借用存活期间Rust阻止同一 world mutation；command/step 后重新获取。

Public query values：

```rust
#[non_exhaustive]
enum ParkingSpaceState {
    Vacant,
    Reserved { vehicle: VehicleHandle },
    Occupied { vehicle: VehicleHandle },
}

#[non_exhaustive]
enum VehicleParkingState {
    Unbound,
    Reserved {
        space: ParkingSpaceHandle,
        approach: ParkingApproachState,
    },
    Occupied { space: ParkingSpaceHandle },
}

#[non_exhaustive]
enum ParkingApproachState {
    Dormant,
    Approaching { route: RouteHandle, route_edge_index: usize },
    Arrived { route: RouteHandle, route_edge_index: usize },
}
```

Approach是从 committed route/cursor/kinematics派生的 query，不是 writable cache authority。Snapshot还提供 normalization-order `space_states()`、global `counts()`和 `area_counts(area)`；single/count query为 O(1)，iterator creation不预复制 S。

### 10.3 Six synchronous commands

```text
reserve_parking_space(vehicle, space)
  -> ParkingReservationRecord

cancel_parking_reservation(vehicle, space)
  -> ParkingReservationCancellationRecord

commit_parking(vehicle, space)
  -> ParkingCommitRecord

leave_parking(LeaveParkingInput { vehicle, space, route, route_edge_index })
  -> ParkingLeaveRecord

rebind_reserved_vehicle_route(
  RebindReservedVehicleRouteInput { vehicle, space, route, route_edge_index }
) -> ReservedVehicleRouteRebindRecord

spawn_parked_vehicle(ParkedVehicleSpawnInput {
  id, profile, route_id, route_edge_index, space
}) -> ParkedVehicleSpawnRecord
```

`ParkedVehicleSpawnInput.route_id` 沿用现有 spawn external route ID 风格；不接受 caller speed/progress/status，成功固定 Parked/zero并从 entry规范化 cursor。

Records使用：

```rust
#[non_exhaustive]
enum ParkingCommandEffect { Applied, AlreadySatisfied }
```

Reservation/cancel/commit至少回显 vehicle/space/effect；leave含 target route/occurrence；rebind含 from/to route/occurrence；parked spawn含新 vehicle/space/route/occurrence且无 no-op。

### 10.4 Vehicle/despawn compatibility

- `VehicleStatus` current 包含 `Parked`。
- 普通 `spawn_vehicle` 必须拒绝 Parked input，返回 `ParkedVehicleRequiresParkingCommand`。
- `VehicleState` 不增加 parking binding字段；通过 parking snapshot组合查询。
- `VehicleDespawnRecord` current 包含 `parking_release: Option<ParkingReleaseRecord>`。

```rust
#[non_exhaustive]
enum ParkingBindingKind { Reserved, Occupied }

#[non_exhaustive]
enum ParkingReleaseReason { RouteCompleted, VehicleDespawn }

struct ParkingReleaseRecord {
    vehicle: VehicleHandle,
    space: ParkingSpaceHandle,
    previous_binding: ParkingBindingKind,
    reason: ParkingReleaseReason,
}
```

这些是 current pre-1.0 breaking API；#108 已交付 commands/records，#109 已交付 step events。

### 10.5 Command records 与 step events

- Command成功立即返回 typed record；不进入下一次 `StepResult.events`，不建立 backlog。
- `AlreadySatisfied` 不产生 transition event；失败无 record/state/event。
- `CoreEvent` 只描述成功 fixed step 内的离散变化。

新增三个 step-only variants：

```text
VehicleParkingStopProjectionApplied {
  tick_index, vehicle, space, route, route_edge_index
}

VehicleParkingArrivalReached {
  tick_index, vehicle, space, route, route_edge_index
}

ParkingReservationReleased {
  tick_index, vehicle, space, reason: RouteCompleted
}
```

- Parking projection只在 selected ParkingStop hard projection超出 emergency envelope时产生；exact tie归Signal时不产生。
- Arrival只在 successful step中 derived predicate false -> true时一次性产生；command-created Arrived不补发。
- Release step event只覆盖 route completion；cancel/leave/despawn由同步 records覆盖。
- Payload只含 handles、occurrence、enum和整数；不复制 external IDs、geometry/f64、VehicleState或 counts。

### 10.6 Private-only implementation

严格保持 crate-private：

- mutable ParkingRuntimeState、forward/reverse storage、candidate deltas、count/cache写入口；
- route prefix exact representation、ApproachTarget cache、ParkingStop、constraint/reducer/projection scratch；
- command spatial index、leave follower lookup、occupancy/leader internals；
- raw handles、vector/map、skip-validation入口、Adapter constraint injection trait；
- direct status/binding/space-state setters。

## 11. Determinism、errors 与失败原子性

### 11.1 Replay contract

在同一 Core version/build/runtime environment、相同 normalized initial world/fixed delta和相同 ordered call log下：

```text
Command(input) -> Result<Record, CoreError>
Step(TickInput) -> Result<StepResult, CoreError>
Query() -> committed value/snapshot
```

必须得到相同 Result分支、error variant/fields、records、committed states、resolver/update/query order和完整 event sequence。Private scratch capacity/address/cache hit和可编辑 Display措辞不属于 replay contract；机器调用方匹配 enum/fields，不解析中文 Display。继续不承诺跨 CPU/compiler 的 bit-identical f64。

Handles保持 opaque/world-scoped caller contract；v0.5不单独为Parking加入 world nonce，也不承诺检测foreign token恰好与当前 active token内部表示相同的误用。

### 11.2 Static errors

新增诊断 enums：

```text
ParkingAnchorKind { Entry, Exit }
ParkingCommandKind {
  Reserve,
  CancelReservation,
  Commit,
  Leave,
  RebindReservedVehicleRoute,
  SpawnParkedVehicle,
}
```

Static normalization errors：

```text
DuplicateParkingAreaId { area_id }
DuplicateParkingSpaceId { space_id }
UnknownParkingSpaceArea { space_id, area_id }
UnknownParkingAnchorEdge { space_id, anchor, edge_id }
ParkingAnchorProgressOutOfRange {
  space_id, anchor, edge_id, edge_progress, edge_length
}
InvalidParkingGeometryValue { space_id, field, value, requirement }
OrphanParkingArea { area_id }
```

`InvalidExternalId`继续复用。`field`只允许 `lateralOffset | headingOffsetRadians | length | width`；`requirement/stage`使用Core固定static text，不拼接wire输入。CoreError不携带JSON path，data crate映射最窄path。

### 11.3 Runtime/lifecycle errors

除复用现有 vehicle/route/profile/overlap errors，新增：

```text
UnknownParkingSpaceHandle { space }
ParkedVehicleRequiresParkingCommand { vehicle_id }

ParkingVehicleStatusMismatch {
  command, vehicle, expected, actual
}
ParkingVehicleAlreadyBound {
  command, vehicle, requested_space, current_space, binding
}
ParkingSpaceUnavailable {
  command, space, requested_vehicle, current_vehicle, binding
}
ParkingReservationMismatch { command, vehicle, space }
ParkingOccupancyMismatch { command, vehicle, space }
ParkingVehicleNotArrived { vehicle, space }

InvalidParkingRouteOccurrence {
  command, vehicle, route, route_edge_index, route_edge_count
}
ParkingRouteOccurrenceEdgeMismatch {
  command, space, anchor, route, route_edge_index,
  expected_edge, actual_edge
}
ParkingRouteRebindEdgeMismatch {
  vehicle, space, route, route_edge_index,
  current_edge, target_edge
}
ParkingEntryUnreachable {
  vehicle, space, route, from_route_edge_index
}
ParkingLeaveUnsafeFollower { vehicle, space, follower }
```

`VehiclePhysicalOverlap`统一表示spawn/rebind/leave的route-aware overlap；只有geometry不重叠但direct follower必须依赖hard projection时使用 `ParkingLeaveUnsafeFollower`。不增加同义 string reason/code。

### 11.4 Internal invariant errors

```text
ParkingBindingInvariantViolation {
  stage, vehicle: Option<VehicleHandle>, space: Option<ParkingSpaceHandle>
}
NonFiniteParkingComputation { stage, vehicle, space, value }
ParkingTraversalBoundaryInvariant {
  vehicle, space, route, route_edge_index,
  remaining_travel, final_speed
}
```

这些variants表示合法public API不应制造的aggregate/compute矛盾，不替代可预期的 command conflict errors。

### 11.5 Atomic failure/no-op

任一 command/step `Err` 必须保持以下 authority不变：

- tick/time、vehicles/routes/signals/parking；
- forward/reverse bindings、counts、handle resolver；
- free list、slot generation、vehicleUpdateOrder；
- public records/events（Core无backlog）。

Failed parked spawn不消费slot/generation/order；failed leave不先Vacant；failed completion不先release。`AlreadySatisfied`返回完整typed record但同样无authority副作用，只适用于设计明确列出的exact target state。

### 11.6 Successful step event total order

按 stable `vehicleUpdateOrder`逐车：

```text
VehicleSignalStopProjectionApplied
  XOR VehicleParkingStopProjectionApplied
-> VehicleFollowingSafetyProjectionApplied?
-> VehicleChangedEdge*
-> VehicleParkingArrivalReached?
-> ParkingReservationReleased(reason=RouteCompleted)?
-> VehicleCompletedRoute?
```

然后按 controller/group normalization order：

```text
SignalPhaseChanged?
-> SignalGroupAspectChanged*
```

Arrival与route completion互斥；release在completed event前。Failed step没有部分event list，retry从同一 committed world得到与fresh replay相同结果。

## 12. Current data format 0.5

### 12.1 Implementation truth

#107 已在同一个 Delivery PR 中原子更新 schema、private DTO、loader、Core normalization、fixtures、examples、format constant 和 current data docs；本节的 static/data 内容现为 production current 事实。Runtime parking state 仍不进入 external package。

### 12.2 Package shape

唯一 current version：

```text
formatVersion: "0.5"
active schema: schemas/laneflow-data-v0.5.schema.json
```

Root新增必填 closed object：

```text
LaneFlowDataPackage
  formatVersion: "0.5"
  units
  laneGraph
  routes
  vehicleProfiles
  signals
  parking: ParkingData
  extensions?

ParkingData
  areas: ParkingAreaData[]
  spaces: ParkingSpaceData[]
```

`parking`、`areas`、`spaces`均必填，数组可空。External package继续只承载static traffic data，不包含initial/parked vehicles、reservation、occupancy、capacity、runtime handles或command log。

### 12.3 Exact wire fields

```json
{
  "parking": {
    "areas": [
      { "id": "lot-main" }
    ],
    "spaces": [
      {
        "id": "lot-main-01",
        "areaId": "lot-main",
        "entry": { "edgeId": "parking-in", "progress": 12.5 },
        "exit": { "edgeId": "parking-out", "progress": 4.0 },
        "geometry": {
          "lateralOffset": -3.0,
          "headingOffsetRadians": -1.5707963267948966,
          "length": 5.0,
          "width": 2.4
        }
      }
    ]
  }
}
```

- Area仅含 `id`。
- Space `areaId`可省略表示standalone；explicit `null`非法。
- Entry/exit/edgeId/progress和geometry四字段全部必填。
- Root/parking/area/space/anchor/geometry均 closed shape。
- 不加入kind、capacity、spaceIds、routeId/occurrence、world pose、polygon、maneuver、access/charging/fee或nested extensions。

Private DTO必须用custom decode区分omitted `areaId`与explicit `null`；普通 `Option<String>`把null静默归一为None不可接受。

### 12.4 Schema/Core ownership

Draft 2020-12 schema负责required/type/closed shape、externalId、单字段numeric range和optional-but-non-null `areaId`。Core负责domain unique/reference/membership、edge existence、upper anchor bound、finite/canonical geometry、orphan area、reverse indexes和稳定validation order。Core对schema已表达的numeric invariant仍重检，以保持programmatic/JSON callers一致。

Schema identifier：

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://illusion-tech.github.io/laneflow/schema/laneflow-data-v0.5.schema.json",
  "title": "LaneFlow Data Package v0.5"
}
```

该字符串只作为absolute versioned identifier。Runtime、loader、Adapter与hermetic tests永不联网解析 `$id`/`$schema`；identifier-only/network publication contract由独立 #103决定。在其G1前，文档不得承诺URL可下载。

### 12.5 Loader order 与 paths

Production fail-fast order：

```text
JSON syntax
-> minimal formatVersion shape
-> exact 0.5 check
-> strict 0.5 DTO
-> units
-> Vehicle Profiles
-> LaneGraph
-> Signals
-> Parking areas identity
-> Parking spaces identity/membership
-> entry/exit anchors
-> geometry
-> orphan areas/reverse indexes
-> routes + final-StopLine
-> InitialTrafficData final assembly/rebind
```

同一输入错误优先级因此为 Signals -> Parking -> Routes。DataError使用最窄path：duplicate指向第二个 `.id`、unknown area指向 `.areaId`、anchor指向具体 `entry|exit.edgeId|progress`、geometry指向字段、orphan指向 `parking.areas[i]`。

Public loader surface不变，仍只接收in-memory bytes/string并返回单一current `LoadedPackage`；不公开raw DTO/history enum/file/async API，也不运行production schema validator。

### 12.6 Migration 与 fixtures

按 ADR 0008 已原子替换：

1. `CURRENT_FORMAT_VERSION` -> `0.5`。
2. 已新增 0.5 schema/fixtures/DTO/tests 并删除 active 0.4 schema/fixtures。
3. 仓库 active examples 已在同一交付迁移。
4. Current docs 已在 #107 中改写；Git/收口报告保留历史事实。
5. v0.4 输入返回 `UnsupportedFormatVersion`，不自动补 empty Parking、不提供 shim/converter。
6. 若实施前发现真实外部0.4资产，另立migration Issue，不扩张production loader。

只保留两个active canonical fixtures：

- `v0.5-parking-signals-baseline.laneflow.json`：non-empty Signals + Parking，area members + standalone、same/distinct entry/exit、正负lateral、zero/angled heading，不含runtime state。
- `v0.5-empty-signals-and-parking.laneflow.json`：Signals四数组和Parking两数组显式为空，继续承担route/profile/repeated-edge回归。

## 13. Performance contract

### 13.1 Shared Core budget

Parking没有独立宽松预算：

- 10k common方向目标 median `<= 1 ms/tick`。
- 10k G3 hard limit median `<= 4 ms/tick`，60 ticks `<= 240 ms`。
- 既有非Parking场景同机回退 `>20%`必须分析，`>30%`默认阻断；若Parking门禁更严格，取更严格者。
- 100k只用于scaling observation，不是realtime SLA。

### 13.2 Matched step gates

每个workload使用10,000 vehicles/spaces、16 ms fixed tick、连续60 ticks、同机base/candidate配对三轮：

| 比较 | 目标 | 必须 profile | 默认阻断 |
| --- | ---: | ---: | ---: |
| pre-Parking base -> candidate + empty registry | `<= 5%` | `> 5%` | `> 10%` |
| empty registry -> 10k spaces all Vacant | `<= 5%` | `> 5%` | `> 10%` |
| all Vacant -> 1% Reserved reachable | `<= 10%` | `> 10%` | `> 15%` |
| all Vacant -> 10% Reserved reachable | `<= 15%` | `> 15%` | `> 20%` |
| all Vacant -> 100% Reserved pressure | `<= 25%` | `> 25%` | `> 35%` 或 >4 ms |

Delta使用每轮 `median.point_estimate`配对后再取三轮delta中位数；保留confidence interval、outliers与原始Criterion artifact，不用单次wall-clock结论。

### 13.3 Workloads 与 scaling

- 10k：10,000 vehicles、10,000 spaces、100 areas、400 routes；100k同步放大10x。
- Route length覆盖8/64 occurrences与repeated edge；command覆盖8/64/512。
- Reserved ratios：0%、1%、10%、100%，entry targets在route horizon分布。
- 场景覆盖far/near/arrived waiting、Signal-before-Parking、Parking-before-Signal、leader-before-Parking、spatial+Following dual projection、route completion release。
- 与free-flow、dense、stop-and-go、projection、transition和Signals场景同轮回归。
- 10k -> 100k matched目标 `<=20x`；超过即阻断并profile。

### 13.4 Complexity ownership

- Static normalization `O(A + S + route/edge lookup)`；禁止A×S、route×S、V×S、V×Signals。
- Route prefix空间 O(route occurrences)，reserve/rebind一次 O(remaining route length)，relevant tick lookup O(1)。
- Reverse binding/count/snapshot single query O(1)；ordered full iteration O(S)且不预复制。
- Cancel/commit O(1)；parked spawn除slot/hash扩容外amortized O(1)。
- Leave只查询local overlap/direct followers，不扫描全部vehicles。
- Resolver removal O(1)；stable update order保序且不能每次 `retain` 全表，可采用tombstone/reverse slot/deterministic amortized compaction等private方案，但compaction不进入normal tick。
- Fixed100-command batch从10k到100k vehicles目标 `<=2x`、`>4x`阻断；world+command count同时10x目标 `<=20x`、`>30x`阻断。

### 13.5 Allocation 与 retained memory

- Warm-up后无event steady step为0 heap allocations；Parking不增加per-tick/per-vehicle allocation。
- Event allocation只与实际离散E成正比，需单独报告。
- Borrowed snapshot、single/count query、iterator creation不全量allocate；稳定capacity下reserve/cancel/commit目标0allocation。
- 报告fixed bytes、bytes/space、bytes/bound vehicle、bytes/route occurrence，不能只报RSS。
- 10k ->100k扣除fixed overhead后Parking retained bytes目标 `<=12x`、`>15x`阻断。
- V×S、route×S或per-vehicle full-route/spaces副本直接阻断。

Exact private containers、compaction threshold和allocation crate不由本文指定。若新增dev dependency，适用Issue/PR必须审计来源、license、RustSec/cargo-deny与分发影响。

## 14. Tests、examples 与 validation evidence

### 14.1 Static/schema/loader

- Empty、standalone、area-owned、mixed、stable normalization/member order。
- Duplicate/unknown area/space/edge、anchor boundaries、geometry、orphan、foreign graph rebind。
- Planned0.5 exact schema ID/title/version/required/closed shape；omitted areaId成功、explicit null拒绝。
- v0.4/future version在current shape/units前拒绝；Core source映射最窄JSON path。
- 两个canonical fixtures同时通过schema、production loader、InitialTrafficData/CoreWorld。

### 14.2 Commands/lifecycle

- 六条commands覆盖Applied、exact AlreadySatisfied、每个fixed error priority和失败/no-op authority equality。
- 两车争一位、一车争两位、stale handle与world-scoped caller-contract边界。
- Safe/overlap/unsafe-follower leave、same/new route、repeated edge。
- Parked spawn、Reserved/Occupied despawn、remove-route in-use、generation reuse和stable update order。

### 14.3 Step composition/events

- Zero-binding parity、Dormant、far/near/arrival、leader/signal/route-end相对顺序、exact tie、dual projection。
- Arrival one-shot、commit后occupancy exclusion、completion release。
- 多vehicle、多edge traversal、arrival/release/completed和signal events严格总序。
- 每个phase代表性failure injection，验证vehicle/parking/signal/tick/time/events/free-list/generation/order均未提交。

### 14.4 Property/model/replay

- Small-state reference model随机执行reserve/cancel/commit/leave/rebind/spawn/despawn/step/query并逐操作比较authority/count/error variant。
- Local command spatial index与full-scan oracle property compare，要求无false positive/negative并覆盖atomic failure。
- 任意成功序列保持Vacant无owner、双向binding、Parked iff Occupied、Active Reserved至多一位和lane participation invariant。
- 任意成功step保持finite/no-overlap，不越Parking/Signal boundary，final travel不超过任一constraint。
- 相同initial world + ordered call log逐调用比较Result/snapshot/resolver/event sequence。

### 14.5 Example 与 evidence

- engine-independent loader-to-Core `parking_lifecycle` example：load/register -> reserve -> approach/arrival -> commit -> leave -> resume。
- Example/fixture assertions由integration tests复用；benchmark builder复用相同topology generator。
- #110已新增 [`../reference/v0.5-parking-validation.md`](../reference/v0.5-parking-validation.md)，记录commit/dirty status、CPU/OS/rustc/LLVM/target/profile/power mode、workload、commands、三轮raw results、confidence interval、outliers、noise、profile与remaining risks。
- Shared CI运行functional/scale smoke、allocation invariant和benchmark compile，不使用shared runner wall-clock阻断。
- Windows incremental-cache finalize note仅作为local noise；只有clean build/CI实际失败才另立build Issue。

## 15. Implementation slices 与 activation chain

```text
                 -> #106 lifecycle/performance --\
#105 design G4 --                              -> #108 runtime/commands
                 -> #107 static/current data --/          |
                                                           v
                                              #109 compliance/activation
                                                           |
                                                           v
                                                #110 validation/performance
                                                           |
                                                           v
                                                  #19 final closure
```

- #106/#107 only after #105 G4；二者可并行。
- #108 only after #106/#107 G4。
- #109 only after #108 G4；ParkingStop与traversal/release必须同一切片，不能产生非法中间能力。
- #110 only after #109 G4；做milestone组合证明，不替代前置切片自身tests/performance gates。
- 每个Issue有独立Gate Ledger、唯一Delivery PR和native blocked-by；branch存在不构成dependency完成。
- #103保持独立、不阻断；#107 G1/G2复核其当时schema publication结论。

## 16. Core/Data/Adapter impact 与后续边界

- Core API：#107 已以 pre-1.0 breaking change 新增 static Parking types/handles/registry/errors；#108 新增 runtime snapshot、commands/records、`VehicleStatus::Parked` 和 despawn record extension；#109 已交付 step events与 moving capability activation。
- Data format：已完成 breaking `0.4 -> 0.5` current-only replacement；无 production compatibility shim。
- Adapter API：v0.5不冻结ABI；Adapter未来只消费Core query/events/position authority，不能复制Parking lifecycle。
- Runtime dependency：不预期新增；testing/performance dependency出现时独立审计。
- ADR：只新增ADR0010；data version、loader、handles、determinism、safety和Signals分层继续分别由ADR0008/0007/0005/0003/0006/0009负责。

明确延后exact private containers/cache/compaction/allocation instrumentation、free-space maneuver、自动选位/queue/fee/charging、Parking Adapter/authoring、network schema hosting、100k realtime和cross-platform bit determinism。真实需求出现时新建Issue/ADR，不得暗中扩张#106-#110。

## 17. G1 审阅结论

本设计已经：

- 合并#105 D1-D12全部Accepted决策；
- 吸收全面审阅P1-1至P1-4：overflow-safe route prefix、local lookup correctness oracle、sparse atomic step、exact transitional guard；
- 分配P2：command-created Arrived event、stable remove_route error、legacy guard unreachable、areaId omitted/null、staged current docs、三轮性能统计和Windows cache noise；
- 对齐ADR0003/0005/0006/0007/0008/0009与 current v0.5 static data 实现边界；
- 明确Core/data/Adapter影响、determinism、error/event order、tests、10k/100k、allocation/memory与activation chain；
- 已由 #107 原子切换 production current v0.5，由 #108 交付 runtime authority/commands，由 #109 交付 activation，并由 #110 完成端到端、性能、allocation/memory 与 pathological profile 验证；#19 已完成最终独立收口审阅。

若后续实施发现 authority 矛盾、局部 lookup 无法与 full-scan 语义等价、sparse atomicity 必须退化为 full-S/V hot path，或 guard 无法保持安全中间主线，必须回到本设计/ADR或拆 follow-up；不得用 private 实现静默改变 Accepted 语义。
