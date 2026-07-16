# 0010 Parking Binding and Vehicle Lifecycle Authority

**状态**: Accepted  
**日期**: 2026-07-16  
**适用范围**: LaneFlow v0.5 Parking 的静态泊位事实、运行时 binding authority、车辆生命周期、位置 authority 与 Core/Adapter 分层  
**关联文档**:

- 上游决策:
  - `0001-project-scope.md`
  - `0003-runtime-tick-and-determinism.md`
  - `0005-core-identity-and-handle-model.md`
  - `0006-vehicle-following-control-and-safety.md`
  - `0007-traffic-data-crate-and-loader-boundary.md`
  - `0008-pre-1.0-data-format-version-policy.md`
  - `0009-signal-indication-gate-and-policy-separation.md`
- 详细设计:
  - `../design/parking-system.md`
  - `../design/route-system.md`
  - `../design/vehicle-following.md`
  - `../design/signal-system.md`
  - `../design/data-format.md`
  - `../design/data-loading.md`

## 背景

LaneFlow v0.4 已拥有 fixed tick、typed handles、有限显式 route、Vehicle Following、Signals 与原子 lifecycle command。v0.5 需要在停车场泊位和道路旁专用路边泊位上支持预约、抵达、停车、离开与占用查询，同时保持 live vehicle identity、纵向安全和确定性。

如果把停车 binding 只写入 `VehicleState`、只写入 `ParkingSpace`，或交给 Adapter 动画维护，车辆状态、空间占用、区域统计和表现位置会形成多份可独立写入的 authority。把 `Stopped`、`Completed` 或 despawn/respawn 当作 Parked 也会分别造成 travel-lane 幽灵占用、无法恢复 route，或 handle/展示 identity 断裂。

Parking 还需要从 on-lane route position 原子切换到 off-lane space pose，并在 leave 时安全切回 route。该边界会长期影响 Core API、Adapter 责任、数据模型和失败原子性，属于高影响且难回退的架构决策。

## 决策

### 1. ParkingSpace 是最小静态泊位事实

- `ParkingSpace` 是可预约、可占用的最小静态实体，具有稳定 external ID，并在 Core normalization 后使用 world-scoped opaque `ParkingSpaceHandle`。
- `ParkingArea` 只对 spaces 做逻辑分组，可表达停车场或一段专用路边停车区；area capacity、vacant、reserved 和 occupied counts 全部由 member spaces 派生。
- 不提供匿名 capacity-only area，也不让 area 保存第二份 `spaceIds` authority。
- 停车场内部通道和接入道路继续使用普通 LaneGraph/Route，不建立第二套路网。
- v0.5 支持 off-travel-lane 的停车场泊位和专用路边泊位/停车带；不支持占用共享正常行车道、双排停车或动态缩窄行车道。

### 2. Committed runtime authority 属于 Core 私有 aggregate

Core 私有 `ParkingRuntimeState`（名称为设计概念，不冻结私有表示）拥有唯一 committed authority：

```text
space -> Vacant | Reserved(vehicle) | Occupied(vehicle)
vehicle -> None | Reserved(space) | Occupied(space)
```

两个方向是同一 aggregate 的一致视图，不是两个可独立写入的事实源。每个 space 至多绑定一辆 live vehicle，每辆 live vehicle 至多绑定一个 space。区域/global counts 是提交时增量维护、可校验的派生缓存，不是独立 authority。

`VehicleState` 不增加可直接写入的 parking-space 字段；Adapter、route、lane occupancy、静态 Parking registry 和查询 snapshot 都不能修改 binding。Public API 不暴露 raw map/vector、direct setter、`force_occupy` 或通用 `set_parking_state`。

### 3. Parked 保持 live vehicle identity

- `VehicleStatus` 增加 `Parked`，但只有 Parking lifecycle command 可以建立完整 `Parked + Occupied` invariant。
- `Parked` vehicle 保留原 `VehicleHandle`、external ID、profile、route reference 与 stable lifecycle identity；正常停车不得通过 despawn/respawn 模拟。
- `Parked` 强制 `current_speed = 0`、`applied_acceleration = 0`，不进入 travel-lane occupancy、leader resolution、longitudinal motion、route traversal 或 route completion。
- `Stopped` 继续表示占用道路的显式停止；`Completed` 继续表示 route 终态；跟驰或信号导致的临时零速继续保持 `Active`。
- approach、arrival 与 leaving 是 operation/query 语义，不新增可独立写入的 `Approaching`、`Arrived` 或 `Leaving` public status。

### 4. Lifecycle 由显式同步 command 驱动

v0.5 采用显式 reserve、pair-specific cancel、commit、leave、reserved-route rebind 与 parked spawn。Command 在两个 step 之间同步执行，实际 caller 调用/提交顺序就是唯一线性化和 replay 顺序。

- Core 不自动选位、最近匹配、满位 reroute、排队、抢占、调度或 pathfind。
- Reservation 没有 wall-clock TTL、隐式 expiry 或自动 replacement。
- 正常运行时必须经过 `Vacant -> Reserved -> Occupied -> Vacant`；不允许普通 command 直接 `Vacant -> Occupied`。
- `spawn_parked_vehicle` 是一次构造完整 invariant 的专用 lifecycle 入口，不改变正常停车必须先 reservation 的规则。
- Command 使用 validate/compute 后一次提交；失败和明确允许的窄幂等 no-op 对全部 authoritative state 零副作用。
- Command records 与 `StepResult.events` 严格分流；Core 不建立 command queue、global sequence 或延迟 event backlog。

### 5. Position authority 随 lifecycle 原子切换

- `Active` approach/arrival 阶段由 route handle、route occurrence 与 front-bumper progress 提供交通位置 authority。
- 成功 commit 时，position authority 与 binding/status 一起切换到 `ParkingSpace` 的 edge-relative static geometry；休眠 route cursor 只保留 live route context，不代表 lane occupancy 或 parked render pose。
- 成功 leave 时，Core 先在 `Parked + Occupied` committed state 上完整验证 caller 指定的 active route/occurrence、exit anchor、route-aware no-overlap 与 direct-follower emergency envelope，再一次切换为 `Active` route position并释放 space。
- Core 不计算 world transform、转向、倒车轨迹或动画。Adapter 可做视觉插值，但不得延迟、覆盖或回滚 Core 已提交的 occupancy、status、position authority 或 lane participation。

### 6. Parking 复用 Core 纵向安全与 traversal authority

Reserved Active vehicle 的 approach target 通过 Core 私有 ParkingStop 进入 Vehicle Following/Signals/RouteEnd 的统一 constraint pipeline。所有 target 从同一 tick-start snapshot 产生，按最严格 admissible motion 归约；subsystem 调用顺序或 Adapter 不能成为隐含优先级。

Spatial hard projection、Following no-overlap、final-travel traversal、Parking boundary guard、arrival 与 route-completion cleanup 必须形成同一原子 step。不得合入“可以 reserve，但车辆会静默越过 entry、忽略 signal/leader，或完成 route 后遗留 owner”的公开中间能力。

### 7. 不完整 capability 必须显式 guard

实施可以分阶段合入，但中间主线必须保持安全：

- static registry/current data 可以在所有 spaces 初始 Vacant 且不改变车辆行为时先交付；
- runtime commands/query 交付而 ParkingStop/traversal 尚未完成时，只对 committed `reserved_count > 0` 的 step 返回结构化 `ParkingVehicleCapabilityUnavailable`；
- empty/all-Vacant 与 occupied-only Parked world 仍可 step，reservation 可通过 cancel/despawn 清除，立即 Arrived pair 可 command-side commit；
- 完整 ParkingStop、arrival、route completion release、traversal 与事件交付后，保留 legacy error variant 兼容，但合法 production world 不再返回。

## 后果

正向后果：

- Parking binding、vehicle status、area counts 与 position authority 有唯一一致的 Core owner。
- Parked vehicle 保持稳定 handle 和展示 identity，又不会成为 travel-lane 幽灵 leader。
- Adapter 可消费 immutable registry、snapshot、records 与 events，而不能复制或覆盖 Parking lifecycle。
- Caller-order commands、fixed-tick step 和 typed handles 可以形成可重放、可测试的确定性边界。
- ParkingStop 复用现有 safety/traversal pipeline，不复制 IIDM、Signal controller 或第二套运动积分器。
- 分阶段实施通过窄 capability guard 保持每次 main 都是可解释且安全的状态。

成本和风险：

- `VehicleStatus::Parked`、Parking handles/snapshot/commands/events/errors 和 despawn record 扩展是 planned pre-1.0 Core API breaking change。
- `InitialTrafficData` 与 current external package 会在后续实施中加入 static Parking registry；按 ADR 0008，格式需要 current-only `0.4 -> 0.5` 原子替换。
- 双向 binding、lifecycle cleanup、route occurrence、safe insertion 与共同 step commit 增加实现和测试复杂度。
- Parked/Reserved vehicle 继续持有 live route reference，会阻止 route removal，直到 leave/rebind/despawn 或 route completion cleanup。
- 当前 LaneGraph 不拥有世界几何，Core 不能证明 space rectangle 与 lane/其他 space 不重叠，也不模拟 maneuver feasibility。

## 替代方案

### 只在 VehicleState 保存 parkingSpaceId

这会让 space availability、area counts 和 vehicle binding 依赖扫描或第二份派生 map，也允许 status/binding 分叉，因此拒绝。

### 只在 ParkingSpace 保存 owner

Vehicle command、despawn、route completion 和 O(1) hot-path lookup 会被迫扫描全部 spaces，且仍缺少 Parked/status invariant，因此拒绝。

### 把 Parked 等同于 Stopped 或 Completed

`Stopped` 仍进入 lane occupancy；`Completed` 是不可自动恢复的 route 终态。两者都不表达独立 space ownership，因此拒绝。

### 用 despawn/respawn 表达正常停车

这会使原 handle stale、打断 external identity、Adapter binding、稳定更新顺序和事件连续性，因此拒绝。

### 由位置或速度自动推断 reservation/occupancy

同一位置可能只是信号/跟驰停车，浮点容差也不能成为资源 authority；自动 commit 还会绕过显式 caller intent，因此拒绝。

### 由 Adapter 或动画拥有占用与 release 时点

不同引擎会复制并漂移交通规则，且可能让渲染状态覆盖 Core safety/route authority，因此拒绝。

### 匿名 capacity-only ParkingArea

它无法支持 caller-selected individual space、排他 binding、geometry pose 与一对一 lifecycle，且会形成与 spaces 并行的 occupancy authority，因此拒绝。

### Parking 建立第二套路网

停车场通道和路边接入可以用普通 LaneGraph/Route 表达。第二套路网会复制 traversal、signals 和 following 规则，因此拒绝。

## 实施与复核

- #105：交付本 ADR 与 `../design/parking-system.md`；只冻结 planned v0.5，不改变 production current v0.4。
- #106：vehicle lifecycle、route-distance 与 command spatial 性能底座。
- #107：static Parking domain 与 current 0.5 schema/loader/fixtures/current docs 原子切换。
- #108：Parking runtime、snapshot、commands 与 lifecycle integration，并使用过渡 guard。
- #109：ParkingStop、arrival、parking-aware traversal、route completion cleanup 与完整 activation。
- #110：端到端示例、property/replay/failure tests、10k/100k、allocation/memory 与验证基线。
- #19：全部子切片 G4 后进行独立 milestone closure。

本 ADR 不冻结 route-prefix representation、spatial index、tombstone/compaction、scratch container、allocation instrumentation 或单次 benchmark 数字；这些属于 `parking-system.md` 的复杂度约束、实施 Issue 和 validation artifact。若未来改变 Parking authority owner、允许 Adapter 决定 lifecycle、支持共享行车道停车，或引入自动调度/隐式占用推断，应新增或 supersede 本 ADR，不得静默修改。
