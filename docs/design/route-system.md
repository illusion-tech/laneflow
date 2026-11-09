# Route System 设计

**文档状态**: Accepted  
**最后更新**: 2026-11-09

**适用范围**: route definition、route validation、route lifecycle 和 simple route following 边界  
**关联文档**:

- `core-id-handles.md`
- `lane-graph.md`
- `road-junction-model.md`
- `parking-system.md`
- `../adr/0003-runtime-tick-and-determinism.md`
- `../adr/0005-core-identity-and-handle-model.md`
- `../adr/0028-integer-millimeter-traffic-geometry.md`
- `../adr/0017-static-road-junction-maneuver-and-gate-identity.md`
- `../adr/0029-retire-precompiled-static-route.md`
- `../roadmap.md`

## 1. 目标

本文定义 `TrafficWorld` 的路线定义、校验、生命周期与 traversal 合同。车辆运动状态见
[`vehicle-following.md`](vehicle-following.md)；路线不得终止在声明 StopLine 的 edge 上；
路线注册期编译 ManeuverPath/Gate/WaitingZone occurrence，并做 (class, Route) 静态准入与
profile-route-cursor 静态可行性校验。唯一生产入口是 runtime `register_route`（ADR 0029）；
出现记录编译器只服务该入口。

### 与停车的关系

静态 ParkingSpace 不持有 RouteHandle。caller 显式选择 entry anchor 与 exact
occurrence，并要求它从 committed cursor 前向可达。leave 显式携带恢复 route/exit
occurrence；rebind 同时携带新 route、当前 physical edge 在新 route 上的 occurrence 与
entry occurrence，并要求新旧 route 展开的完整车身 physical occupancy footprint 相同，
不能只匹配前缘 edge 后把跨 predecessor 的车尾换到另一支路。Parked/Reserved vehicle 均
保留 live route reference，`remove_route`
不得删除任一 live vehicle 正在引用的 route。Overflow-safe route prefix 不得新增“整条 route 累计距离必须 finite”的合法性条件。
详细契约见 [`parking-system.md`](parking-system.md)。

### 路线距离

**现行合同是 ADR 0028**：路线前缀与距离查询为 `u32` mm，溢出 `BeyondFinite`；单边
`u32` mm，`10_000_000` mm 上界。`register_route` **不得**因整条或
前缀累计超过 `u32::MAX` mm 失败。距离按查询窗口独立 checked 加：从起点的前缀溢出
只影响「从起点算」；从当前进度到终点、以及局部视距从查询起点加，靠近终点后可再
`Finite`，`Completed` 只在剩余 `Finite(0)`。索引用分段 `u32` + 后缀
`BoundedDistance`，不上 `u64`（ADR 0028）。

### 目标与非目标

目标：

- 定义 route definition 的最小长期模型。
- 明确 route edge sequence、loop、断连、目标行为和 validation 输入。
- 明确 route following 与 fixed tick、event order 和 lifecycle 的关系。

非目标：

- 不实现 pathfinding 或自动路线规划。
- 不设计 vehicle following、signals、parking、intersection priority 或 lane change。
- 不冻结 Adapter API。
- 不支持运行时修改 lane graph topology。
- 不把 route 设计绑定到 SUMO / CARLA / libsumo 等外部运行时。

## 2. 术语

- **Route definition**：外部数据或运行时命令提交给 Traffic Runtime 的 route 输入。
- **RouteHandle**：Traffic Runtime 内部和 public runtime API 使用的不透明 typed route handle。
- **Route edge sequence**：route 中按行驶顺序排列的 edge 序列。
- **Route cursor**：车辆在 route 中的位置，至少由 route edge index 和 edge-local progress 表达。
- **Route target**：route 的完成目标，固定为最后一个 route edge 的出口边界。
- **Live vehicle**：仍存在于 `TrafficWorld` 车辆表中的车辆，不等同于 `VehicleStatus::Active`。

## 3. 设计决策

### D1. route 是有限显式 edge sequence

状态：已接受。

路线是有限、显式、有序的车道图边序列。生产入口是
`TrafficWorld::register_route([e0, e1, …])`，输入为共享静态路网 `LaneEdgeOrdinal`。

调用方（场景 catalog、规划器）自己持有 `route_id` 或边键；Runtime 热表只有
`RouteHandle { index, generation }`。

route following 时：

- 当前边来自已注册序列的 `route_edge_index`。
- 下一条边来自 `route_edge_index + 1`。
- 车道图连接只用于验证这两个边是否可连接。
- 边序列是车辆实际行驶顺序。

### D2. route target 是最后一个 edge 的出口边界

状态：已接受。

route target 固定为最后一个 route edge 的 edge length 位置。车辆到达该边界后：

- `VehicleStatus` 进入 `Completed`。
- `routeEdgeIndex` 保持最后一个 route edge index。
- 边内进度 clamp 到最后一个 edge length。
- 调用方通过已提交车辆状态观察 `Completed`。

若业务需要在 edge 中途完成 route，应在 lane graph 中把该位置拆成 terminal edge boundary，再把 route 的最后一个 edge 指向拆分后的 terminal edge。不引入 per-route `targetProgress`。

### D3. 显式 loop 合法，不提供隐式无限 loop

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

持续循环路线、巡逻路线或随机巡航应由后续 route policy / scheduler 设计处理，不在 route definition 中隐式表达。

### D4. route validation 依赖稳定 lane graph

状态：已接受。

注册 route 时必须校验：

- route edge sequence 非空。
- 序列中的边都是当前共享静态路网上的合法 `LaneEdgeOrdinal`。
- 任意相邻 edge pair 都存在车道后继或机动转移候选。
- route edge sequence 的长度可以用 `usize` 索引，且不会造成实现中的计数溢出。
- Runtime **不**校验 external / catalog route ID，也不按字符串去重。

route validation 不检查：

- vehicle following 安全距离。
- signal phase 或 intersection right-of-way。
- parking availability。
- path optimality。
- 几何曲率、turn radius 或碰撞。
- 整条 route 累计距离必须 finite（#496：溢出为 `BeyondFinite`，不是注册失败）。

### D5. route registry 支持动态注册和受控移除

状态：已接受，依据 ADR 0005。

Traffic Runtime 支持在步进之间 register / remove route definition。

规则（ADR 0029）：

- `register_route` 只接收共享静态路网 `LaneEdgeOrdinal` 有序非空序列，不接收 external
  route ID。catalog / 调用方若有 `route_id`，自己持有并交叉引用句柄。
- `TrafficWorld` 不按字符串做路线去重；同一边序列注册两次得到两个句柄。
- `register_route` 成功后返回 active `RouteHandle`。
- `register_route` 失败必须保持 route registry 不变。
- `remove_route` 只能移除没有 live vehicle 引用的 route。
- 这里的 live vehicle 包括 `Active`、`Parked` 和 `Completed` 状态，只要车辆仍在
  `TrafficWorld` 中存在，就视为引用该 route。
- `remove_route` 成功后旧句柄 stale；调用方若要诊断用的 catalog ID，对照自己的表。
- 旧 `RouteHandle` 在 route 移除后变为 stale；若槽位复用，新 handle 必须拥有不同 generation。

不提供 in-place route mutation。需要替换 route 时，应注册新 route，并把车辆迁移策略作为单独设计处理。

### D6. route following 使用 fixed tick、edge-local progress 和领域边界策略

状态：已接受。

route following 遵守 ADR 0003 与 `traffic-runtime-integer-geometry.md`：

- Runtime 不读取 wall clock。
- 每个 world 使用固定 `fixed_delta_time_ms ∈ [4, 1000]`。
- `TickInput.delta_time_ms` 必须与 world 固定步长一致。
- IIDM 仍在瞬时 SI 中算出本拍 travel 与 `next_speed`；已提交行程是整数毫米，
  `carry_um` 保留未凑满 1 mm 的余数。不得用 `2×travel/Δt − v0` 反推已提交速度。
- 已提交速度、边长和边进度的正本是 `u32` mm / mm/s；公开米制面只作编制输入或只读换算。
- 满边且下一跳许可时进入下一条 `progress_mm = 0`；满边但下一跳拒绝则停在
  `progress_mm == length_mm`。

单 tick 可以跨越多个 edge，但实现必须有硬上界：

- 上界为当前 route 剩余 edge 数。
- 每次跨 edge 必须递增 `routeEdgeIndex`。
- 到达最后 edge 后完成并退出。
- 失败时 world 必须保持不变，不产生部分状态或部分事件。

### D7. route event order 必须确定

状态：已接受。

事件顺序规则：

- 多个 vehicle 的更新顺序由显式、确定的 `live_order` 决定。
- 不得依赖每 tick external string sort。
- 不得依赖 `VehicleHandle` 或 `RouteHandle` 的内部数值排序作为公开合同。

跨边与路线完成不发布独立事件，调用方读取已提交车辆状态。路线可读 ID 不进 Runtime
resolver，查调用方自己的表（ADR 0029）。

### D8. route system 不承担 pathfinding

状态：已接受。

route system 只消费调用方提供的 explicit route。它不计算最短路、最快路、随机路径或交通感知路径。

后续若引入 route planner，应单独设计：

- planner 输入和输出是否进入 Traffic Runtime。
- cost model。
- deterministic tie-breaker。
- 与 lane connection、signals、parking、vehicle following 的关系。
- planner 是否可以在 runtime 中改变 active vehicle route。

## 4. Vehicle State 影响

车辆运行时状态引用路线句柄与路线序列下标，不保存外部字符串：

```text
VehicleState
  handle: VehicleHandle
  route: RouteHandle
  route_edge_index: u32
  progress_mm: u32
  carry_um: u16
  speed_mm_s: u32
  status: VehicleStatus
```

校验规则：

- spawn vehicle 时 route handle 必须 active。
- `routeEdgeIndex` 必须在 route edge sequence 范围内。
- 边内进度必须落在当前 edge 的 `[0, edge length]` 范围内。
- parked / completed vehicle 不移动，但仍保留 route 引用，直到 leave/rebind 原子替换或
  despawn 原子移除。

## 5. Runtime API 影响

现行 `TrafficWorld` API（ADR 0029）：

- route input 使用共享静态路网 `LaneEdgeOrdinal` 序列，不使用 external route ID。
- route registry 成功后返回 `RouteHandle`。
- vehicle spawn input 使用 `RouteHandle`；runtime state 只持句柄。
- route query / event payload / vehicle runtime state 使用 handle。
- catalog `route_id` ↔ `RouteHandle` 的对照若需要，放在 scenario / Adapter，不进
  Runtime 热表。
- route removal 需要 route-in-use、stale handle、unknown route；**没有**
  Runtime 级 duplicate route ID 错误路径。

**换道能力拒绝衔接注记（#237，规范定义见 [`lane-change-minimal-contract.md`](lane-change-minimal-contract.md) §6）**：
含固定预制过渡的路线在运行时不支持过渡执行时，`register_route`、车辆准入与
恢复入口按该合同结构化拒绝（`TransitionUnsupported` 类、失败关闭、注册表
不变/零部分恢复）；换道支持能力属实现能力，不新增配置字段（不加
`WorldConfig`、不扩义 `ExecutionConfig`）。静态序列合同与失败原子（D4/D5）
不变。

## 6. 持久化影响

路网制品不声明路线（ADR 0029）；路线由调用方持有的边序列在运行时注册。`RouteHandle`
与 `VehicleHandle` 不进入路网制品。若示例数据需要中途目标，应通过拆分 lane edge 表达。

## 7. Adapter 影响

Adapter 可以：

- 读取 route event 和 vehicle runtime state。
- 通过共享路网修订的冷标识索引查询边的稳定标识。车辆与路线的可读 ID 归调用方。
- 使用自身 geometry 数据把 edge progress 转换为 transform。
- 在 debug UI 中显示 route edge sequence 和 completion。

Adapter 不应：

- 在 Traffic Runtime 外部自行修复断连 route。
- 把引擎对象生命周期当作 Traffic Runtime route registry 生命周期。
- 依赖 route handle 的内部数值排序。

## 8. 测试与验证输入

后续实现 issue 至少应覆盖：

- empty route。
- unknown route edge。
- disconnected route edge。
- repeated edge route。
- explicit self loop route。
- route completion clamps progress to final edge length。
- stopped / completed vehicle 后续 tick 不移动。
- single tick 跨多个 edge 的状态推进。
- route registration 成功返回 handle。
- route registration 失败保持 registry 不变。
- route removal 拒绝 live vehicle 正在引用的 route。
- stale route handle rejection。
- 路线不经 Runtime resolver 回查 external ID。
- 10 km 边长上界、整数毫米进度与 `BeyondFinite` 路线距离，在生产矩阵中通过精确边界、单 tick 多 edge、多轮 route、溢出和失败原子性验证。

## 9. Maneuver occurrence

ADR 0017 保持 Route 是车辆实际 traversal authority。ManeuverPath 不替代、
补全或重排 Route；它只在 Route 中完整连续匹配时形成语义 occurrence。

`register_route` 在命令路径编译出现记录（路网制品不声明路线，ADR 0029）：

```text
ManeuverOccurrence(route, entryRouteEdgeIndex, exitRouteEdgeIndex, maneuverPath)
GateOccurrence(routeTransitionIndex, maneuverGate)
```

Repeated edge 继续由 `routeEdgeIndex` 区分。Topology registry 必须使用
entry-transition candidate index 缩小匹配范围，不得对每个 Route position 扫描全部
ManeuverPaths。Compiled metadata 由 Route 共享；vehicle tick 不匹配 path、不查
external ID、不扫描全局 catalog，也不为每辆车复制 occurrence。

`register_route` 必须先完成 path/Gate/StopLine coverage 编译和验证，再原子提交 handle、
definition 与 metadata。失败不得留下部分 occurrence 或可观察 allocation/order。
完整 shape、歧义规则与性能边界见
[`road-junction-model.md`](road-junction-model.md)。
