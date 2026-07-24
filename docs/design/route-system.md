# Route System 设计

**文档状态**: Accepted  
**最后更新**: 2026-07-24（#228 v0.9 static-domain target 同步）

**适用范围**: v0.2 Lane Graph + Route 的 route definition、route validation、route lifecycle 和 simple route following 边界  
**关联文档**:

- `core-runtime.md`
- `core-id-handles.md`
- `lane-graph.md`
- `road-junction-model.md`
- `parking-system.md`
- `../adr/0003-runtime-tick-and-determinism.md`
- `../adr/0005-core-identity-and-handle-model.md`
- `../adr/0014-residual-aware-f32-core-authority-and-migration-gates.md`
- `../adr/0017-static-road-junction-maneuver-and-gate-identity.md`
- `../roadmap.md`

## 1. 目标

本文固化 v0.2 阶段 Core route system 设计，作为 #29 的 G1 冻结输入。

### 当前 v0.7 覆盖说明

本文保留 v0.2 route definition、validation、lifecycle 和 traversal 契约。v0.3 由 [`vehicle-following.md`](vehicle-following.md) 第 5 节替换 vehicle motion state；v0.4 将 external sequence 字段改为 `edgeIds`，并规定 route 不得终止在声明 StopLine 的 edge 上；v0.5 增加 static Parking anchors；current v0.7 再增加 route-occurrence 降限速 metadata。initial route 与 runtime `register_route` 复用同一规则；#96 已激活 permission-aware traversal。

Current static ParkingSpace 不持有 RouteHandle。#108/#109 current runtime 消费有限显式 route/occurrence：Reserved approach 选择当前 cursor 后的 first-reachable entry occurrence，leave/rebind 由 caller 提供明确 route occurrence，Parked/Reserved vehicle 保留 live route reference。Overflow-safe route prefix 不得新增“整条 route 累计距离必须 finite”的合法性条件。完整端到端验证由 #110 固化，详细契约见 [`parking-system.md`](parking-system.md)。

ADR 0014 已接受下一数值契约：单 edge 硬上限为 10 km，`EdgeLength` 使用经过检查的 `f32`，`EdgeProgress` 使用补偿残差感知的高位/残差表示。该候选在 #144 no-go 后没有进入 production；本节当前 v0.7 继续使用 `f64`。route 距离只冻结派生权威、有限视距查询、复杂度与防溢出语义；物理存储由 #127 比较 `f64` 前缀基线与分块局部 `f32` 候选。

目标：

- 定义 route definition 的最小长期模型。
- 明确 route edge sequence、loop、断连、目标行为和 validation 输入。
- 明确 route following 与 fixed tick、epsilon、event order 和 lifecycle 的关系。
- 为 #30 data format、#31 validation 和 #32 Core 对齐提供可引用输入。

非目标：

- 不实现 pathfinding 或自动路线规划。
- 不设计 vehicle following、signals、parking、intersection priority 或 lane change。
- 不冻结 Adapter API。
- 不支持运行时修改 lane graph topology。
- 不把 route 设计绑定到 SUMO / CARLA / libsumo 等外部运行时。

## 2. 术语

- **Route definition**：外部数据或运行时命令提交给 Core 的 route 输入。
- **RouteHandle**：Core runtime 内部和 public runtime API 使用的不透明 typed route handle。
- **Route edge sequence**：route 中按行驶顺序排列的 edge 序列。
- **Route cursor**：车辆在 route 中的位置，至少由 route edge index 和 edge-local progress 表达。
- **Route target**：route 的完成目标。v0.2 固定为最后一个 route edge 的出口边界。
- **Live vehicle**：仍存在于 CoreWorld vehicle registry 中的车辆，不等同于 `VehicleStatus::Active`。

## 3. 设计决策

### D1. route 是有限显式 edge sequence

状态：已接受。

v0.2 route definition 使用有限、显式、有序的 lane edge sequence。

概念模型：

```text
RouteInput
  externalId: string
  edgeExternalIds: string[]
```

Core 不根据 lane graph 自动选择下一 edge。route following 时：

- 当前 edge 来自 `edgeExternalIds[routeEdgeIndex]`。
- 下一 edge 来自 `edgeExternalIds[routeEdgeIndex + 1]`。
- lane graph connection 只用于验证这两个 edge 是否可连接。
- route edge sequence 是车辆实际行驶顺序。

### D2. route target 是最后一个 edge 的出口边界

状态：已接受。

v0.2 的 route target 固定为最后一个 route edge 的 edge length 位置。车辆到达该边界后：

- `VehicleStatus` 进入 `Completed`。
- `routeEdgeIndex` 保持最后一个 route edge index。
- `edgeProgress` clamp 到最后一个 edge length。
- 只在车辆从 `Active` 首次进入 `Completed` 的 tick 发出一次 `vehicle.completedRoute` 事件。

若业务需要在 edge 中途完成 route，应在 lane graph 中把该位置拆成 terminal edge boundary，再把 route 的最后一个 edge 指向拆分后的 terminal edge。v0.2 不引入 per-route `targetProgress`，避免在 #29 中提前冻结 partial-edge target、parking target 和 stop line 语义。

### D3. 显式 loop 合法，隐式无限 loop 不属于 v0.2

状态：已接受。

route edge sequence 可以重复引用同一个 edge，例如：

```text
R1: [A, B, A]
```

重复 edge 表示有限 route 中的显式回环。车辆位置由 `routeEdgeIndex` 区分，而不是只靠 edge ID。

合法性要求：

- 每个相邻 edge pair 必须在 lane graph 中存在显式 connection。
- `A -> A` self loop 只有在 lane graph 中显式声明 self connection 时才合法。
- route 到达 sequence 最后一个 edge 的出口边界后完成，不自动回到开头。

持续循环路线、巡逻路线或随机巡航应由后续 route policy / scheduler 设计处理，不在 v0.2 route definition 中隐式表达。

### D4. route validation 依赖稳定 lane graph

状态：已接受。

注册 route 时必须校验：

- route external ID 在 active route registry 中唯一。
- route edge sequence 非空。
- route 引用的所有 edge external ID 都存在于当前 lane graph。
- 任意相邻 edge pair 都存在 lane connection。
- route edge sequence 的长度可以用 `usize` 索引，且不会造成实现中的计数溢出。

route validation 不检查：

- vehicle following 安全距离。
- signal phase 或 intersection right-of-way。
- parking availability。
- path optimality。
- 几何曲率、turn radius 或碰撞。

### D5. route registry 支持动态注册和受控移除

状态：已接受，依据 ADR 0005。

v0.2 Core runtime 支持运行时 register / remove route definition。

规则：

- `register_route` 接收 external route ID 和 edge external ID sequence。
- `register_route` 成功后返回 active `RouteHandle`。
- `register_route` 失败必须保持 route registry 不变。
- `remove_route` 只能移除没有 live vehicle 引用的 route。
- 这里的 live vehicle 包括 `Active`、`Stopped` 和 `Completed` 状态，只要车辆仍在 CoreWorld 中存在，就视为引用该 route。
- `remove_route` 成功应返回 lifecycle record，至少包含 route handle 和 external route ID，便于 debug / Adapter 记录。
- 旧 `RouteHandle` 在 route 移除后变为 stale；若槽位复用，新 handle 必须拥有不同 generation。

v0.2 不提供 in-place route mutation。需要替换 route 时，应注册新 route，并把车辆迁移策略作为单独设计处理。

### D6. route following 使用 fixed tick、edge-local progress 和领域边界策略

状态：已接受。

route following 继续遵守 ADR 0003 与 `core-runtime.md`：

- Core 不读取 wall clock。
- 每个 world 使用固定 `fixed_delta_time_ms`。
- `TickInput.delta_time_ms` 必须与 world 固定步长一致。
- travel distance 先把 fixed delta 转成秒，再由 `effective_speed * (fixed_delta_time_ms as f64 / 1000.0)` 得到；计算结果必须保持 finite。
- 当前生产实现的速度、距离、edge 长度和 edge 进度使用 `f64` 新类型，并拒绝非有限值。
- edge boundary snap 与 remainder 使用 crate-private 的 edge boundary/remainder owner；它不与最小 edge length、纵向约束或物理 gap owner 合并。

ADR 0014 的目标不直接改写上述 current-f64 行为：迁移后 `EdgeLength` 和单值控制域使用经过检查的 `f32`，`EdgeProgress` 的唯一有效值由高位/残差组合得到；行程、剩余量、边界和快照不得只读取高位分量。#125 拆分 current-f64 领域 owner，#127 离线标定 target-f32；#144 的原子候选未通过性能门槛并已回退，所以本节生产描述仍是 current-f64。

单 tick 可以跨越多个 edge，但实现必须有硬上界：

- 上界为当前 route 剩余 edge 数。
- 每次跨 edge 必须递增 `routeEdgeIndex`。
- 到达最后 edge 后完成并退出。
- 失败时 world 必须保持不变，不产生部分状态或部分事件。

### D7. route event order 必须确定

状态：已接受。

事件顺序规则：

- 单个 vehicle 在同一 tick 内按实际 route transition 顺序输出事件。
- 多个 vehicle 的更新顺序必须由显式 deterministic update order 决定。
- v0.2 不得继续依赖每 tick external string sort 作为长期 hot path 策略。
- 不得依赖 `VehicleHandle` 或 `RouteHandle` 的内部数值排序作为 public contract。

event payload 应使用 handle：

```text
VehicleChangedEdgeEvent
  tickIndex
  vehicle: VehicleHandle
  route: RouteHandle
  fromEdge: EdgeHandle
  toEdge: EdgeHandle
  fromRouteEdgeIndex
  toRouteEdgeIndex

VehicleCompletedRouteEvent
  tickIndex
  vehicle: VehicleHandle
  route: RouteHandle
  edge: EdgeHandle
  routeEdgeIndex
```

Adapter、debug 和日志需要可读 ID 时，应通过 resolver 查询 external ID。

### D8. route system 不承担 pathfinding

状态：已接受。

v0.2 route system 只消费调用方提供的 explicit route。它不计算最短路、最快路、随机路径或交通感知路径。

后续若引入 route planner，应单独设计：

- planner 输入和输出是否进入 Core。
- cost model。
- deterministic tie-breaker。
- 与 lane connection、signals、parking、vehicle following 的关系。
- planner 是否可以在 runtime 中改变 active vehicle route。

## 4. Vehicle State 影响

v0.2 vehicle runtime state 应引用 route handle 和 edge handle，而不是 external string：

```text
VehicleRuntime
  handle: VehicleHandle
  route: RouteHandle
  routeEdgeIndex: usize
  edgeProgress: EdgeProgress
  speed: Speed
  status: VehicleStatus
```

校验规则：

- spawn vehicle 时 route handle 必须 active。
- `routeEdgeIndex` 必须在 route edge sequence 范围内。
- `edgeProgress` 必须落在当前 edge 的 `[0, edge length]` 范围内。
- initial completed vehicle 必须位于 route 最后 edge，且 progress 在最后 edge length 的 edge boundary 领域阈值范围内。
- stopped / completed vehicle 不移动，但仍保留 route 引用，直到 despawn。

## 5. Runtime API 影响

v0.2 Core API 预期变化：

- route input 使用 external route ID 和 external edge ID sequence。
- route registry 成功后返回 `RouteHandle`。
- vehicle spawn input 使用 route external ID 或 `RouteHandle`，具体入口可按实现阶段选择，但 runtime state 必须归一化为 handle。
- route query / event payload / vehicle runtime state 使用 handle。
- resolver 提供 route handle 与 external route ID 的双向查询。
- route removal 需要 route-in-use、stale handle、unknown route 和 duplicate route ID 错误路径。

这是 Core API breaking change，但符合 ADR 0005 的 handle 化方向。

## 6. Data Format 影响

#30 data format 应至少能表达：

- route external ID。
- ordered edge external ID sequence。
- route 与 lane graph 的同一数据包或引用关系。

data format 不应持久化 `RouteHandle`、`EdgeHandle` 或 `VehicleHandle`。

partial-edge target、route policy、planner cost、traffic condition、parking target 和 signal compliance 不在 v0.2 route data format 的最小冻结范围内。若示例数据需要中途目标，应通过拆分 lane edge 表达。

## 7. Adapter 影响

Adapter 可以：

- 读取 route event 和 vehicle runtime state。
- 通过 resolver 查询 vehicle / route / edge external ID。
- 使用自身 geometry 数据把 edge progress 转换为 transform。
- 在 debug UI 中显示 route edge sequence 和 completion。

Adapter 不应：

- 在 Core 外部自行修复断连 route。
- 把引擎对象生命周期当作 Core route registry 生命周期。
- 依赖 route handle 的内部数值排序。

## 8. ADR 评估

本设计不新增 ADR。

原因：

- route following 的 tick、determinism 和失败原子性已由 ADR 0003 覆盖。
- route handle、dynamic route lifecycle 和 resolver 边界已由 ADR 0005 覆盖。
- 本文只把 route system 的 v0.2 设计细化为可实现输入。

若后续新增 pathfinding、runtime route mutation、动态 lane graph 拓扑或 partial-edge target，应重新评估是否需要新增 ADR。

## 9. 测试与验证输入

后续实现 issue 至少应覆盖：

- empty route。
- duplicate route ID。
- unknown route edge。
- disconnected route edge。
- repeated edge route。
- explicit self loop route。
- route completion clamps progress to final edge length。
- completed event 只发一次。
- stopped / completed vehicle 后续 tick 不移动。
- single tick 跨多个 edge 的事件顺序。
- route registration 成功返回 handle。
- route registration 失败保持 registry 不变。
- route removal 拒绝 live vehicle 正在引用的 route。
- stale route handle rejection。
- event payload 使用 handle，resolver 可回查 external ID。
- 目标 10 km edge 上界、补偿残差感知进度与 route 距离候选，在生产迁移矩阵中通过精确边界、单 tick 多 edge、多轮 route、溢出和失败原子性验证。

## 10. v0.8 直行走廊 route profile

#184 不改变 route 是 finite explicit edge sequence、且 Core 不负责 pathfinding 的既有决策。v0.8 generator 为 6 个 portal-level 直行 movement 生成 14 条 lane-level routes：主干道两个方向各三条，两个次干道的两个方向各两条。主干道 route 穿越两个独立 connector，次干道 route 穿越一个 connector；不同 lane route 不互连，因此首版没有换道或转向。

route completion event 的稳定顺序是 #203/caller-owned policy 建立 pending recycle plan 的输入。回流后是复用 logical external ID 的新旅程和新 `VehicleHandle`，不是把 completed route cursor 原地重置。Core 不拥有人口或回流 policy，只由 #186 提供 caller-driven atomic replace；完整 ID、portal 和 route 表见 `example-scenarios.md`，production generator/policy 分别由 #188/#203 交付。

## 11. v0.9 Maneuver occurrence target

#228/ADR 0017 保持 Route 是车辆实际 traversal authority。ManeuverPath 不替代、
补全或重排 Route；它只在 Route 中完整连续匹配时形成语义 occurrence。

Initial 和 runtime `register_route` target 在 command/normalization path 编译：

```text
ManeuverOccurrence(route, entryRouteEdgeIndex, exitRouteEdgeIndex, maneuverPath)
GateOccurrence(routeTransitionIndex, maneuverGate)
```

Repeated edge 继续由 `routeEdgeIndex` 区分。Topology registry 必须使用
entry-transition candidate index 缩小匹配范围，不得对每个 Route position 扫描全部
ManeuverPaths。Compiled metadata 由 Route 共享；vehicle tick 不匹配 path、不查
external ID、不扫描全局 catalog，也不为每辆车复制 occurrence。

Dynamic Route 必须先完成 path/Gate/StopLine coverage 编译和验证，再原子提交 handle、
definition 与 metadata。失败不得留下部分 occurrence 或可观察 allocation/order。
完整 shape、歧义规则与性能边界见
[`road-junction-model.md`](road-junction-model.md)。该 target 由 #229 实现前，
current Route production behavior 不变。
