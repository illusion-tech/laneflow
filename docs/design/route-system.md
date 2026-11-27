+++
schema_version = 1
id = "design-route-system"
kind = "design"
status = "accepted"
revised_date = "2026-11-27"
summary = "定义每世界路线、注册与车辆跟随。"
scope = "route definition、route validation、route lifecycle 和 simple route following 边界"
related = ["adr-0003", "adr-0005", "adr-0017", "adr-0028", "adr-0029", "design-core-id-handles", "design-lane-graph", "design-parking-system", "design-road-junction-model", "design-traffic-observation-and-routing-integration", "roadmap"]
+++

# Route System 设计

## 相关文档

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
[`vehicle-following.md`][vehicle-following]；路线不得终止在声明 StopLine 的 edge 上；
路线注册期编译 ManeuverPath/Gate/WaitingZone occurrence。车辆绑定时，以当前路线下标为起点，
按实际车型检查可达后缀的静态准入与可行性。直接注册与带来源绑定的候选注册共用
同一个出现记录编译器（ADR 0029）。

### 与停车的关系

静态 ParkingSpace 不持有 RouteHandle。caller 显式选择 entry anchor 与 exact
occurrence，并要求它从 committed cursor 前向可达。leave 显式携带恢复 route/exit
occurrence；rebind 同时携带新 route、当前 physical edge 在新 route 上的 occurrence 与
entry occurrence，并要求新旧 route 展开的完整车身 physical occupancy footprint 相同，
不能只匹配前缘 edge 后把跨 predecessor 的车尾换到另一支路。Parked/Reserved vehicle 均
保留 live route reference，`remove_route`
不得删除任一 live vehicle 正在引用的 route。Overflow-safe route prefix 不得新增“整条 route 累计距离必须 finite”的合法性条件。
详细契约见 [`parking-system.md`][parking-system]。

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

#### 候选路线的横向交接试算

`TrafficWorld::bind_route_handoffs` 在两拍之间为同一有序边序列绑定多个窗口。每项声明用源下标
`i` 指定出现对 `i,i+1`，各源下标严格递增。两次经过同一对物理边可以有不同窗口。
只有指定的出现对使用横向交接，其余各段按普通后继或完整机动编译。绑定从结构边界和两条边
确定所属走廊、框架和双方实际拥有的共同站位，按有向规则把内切点归给下游边。

`TrafficWorld::preview_route_handoff_at` 按源出现下标选择窗口，从同一修订读取车型的通行类别、规范车长、车体几何和转向限值，校验整条路线及其机动的静态准入。
调用方提供候选轨迹的后轴三维车速。交接查询再从实际两条边对应的车道生成
五次横移，求证前保险杠的唯一交接截面。只有完整截面包络在允许窗口内，并可在两条
指定边上按本修订的规范切点唯一定位时，才能返回其站位与各自的局部进度。两侧准入覆盖
整个车身投影窗口（`station_window_nm`）。如果扩大窗口后纳入一条实际车身未触及的禁行边，
试算会返回 `SideAccessUnproven`。调用方宜给出能覆盖整车的紧凑窗口。[横断面与路线交接合同][lateral-handoff]

源车道远端有停止线时，车辆仍可在路口前换出。普通路线段可以在该源出现处交给横移，
完整路线的终点仍按普通注册规则校验。试算核对运动全程在源侧扫过的车身，包含前杠交接后仍留在源侧的车尾。
源出现及其物理续行边上，车身须在首条未选择机动的停止线上游清空。源出现以前也可能有未选择机动的停止线，
此时车身可始终处于线的任一侧。目标侧也须与未选机动的停止线严格分离，包括前杠交接前已经进入目标车道的车身。
车辆在线的下游才进入目标车道时可以通过。两侧都按本次路线出现和车道的物理后继核对已选机动，其门由当拍交通规则裁决。
若路线选择在停止线处转出，沿原车道继续扫过的车身仍须与线分离。
先核对两侧投影、车型通行规则与路线出现，再用剩余扫过额度求证停止线。两侧共用一次按需计算的连续扫过。
任一侧外包可能触线或求证额度不足时，`StopLineUnproven` 报告停止线所在边和已消耗的扫过工作。

绑定保留一份有序边列、编译后的机动、每个窗口及原共享修订。编译时跨各段累计路线冲突出现容量。
临时存储和按车型检查准入的扫描成本随路线长度增长，在 `GeometryWorkLimits` 的几何预算之外。
单窗口调用可使用 `bind_route_handoff` 和 `preview_route_handoff`。绑定和试算均保持命令游标与路线表原状。

成功定位后，`travel` 在原候选上求站位对应的后轴三维行程，或从行程反求站位。结果为保守闭区间。`body_side` 对给定后轴站位区间证明整车在源侧、同时跨两侧或已全在目标侧，包含车身上下表面。前保险杠交接时车身仍可横跨共边。这两项查询各自计量工作并报告未证。固定步进中的预约还须结合当拍前后车、信号和下游空间裁决。可执行路线的现行注册规则见 D5。

`sample_after_travel` 以先前累计行程和本拍实际位移（均为后轴三维微米数）取样。
一次受检相加后反查后轴站位，在完整站位闭区间内求前杠中点、各侧完整车身外包及实体点见证。
前杠物理边须唯一，并与所选出现对的普通路线段连续相接。这样车头在源道较早的一条边、
交接窗口对应的目标边、或目标道后续边时，都可返回其在本次路线上的实际下标。
某侧实体点确实存在，才能报告该侧有车体。外包跨多个切点时，每个边内区间仍是可能范围。
该取样只描述拍末姿态。拍内扫过、其他车辆的可达性及下游清空仍须由固定步进裁决。

`preview_route_handoff_traffic` 供外部候选在进入前查询当前世界的已提交车辆。
它逐侧检查车身外包可能触及的物理边，包括路线边列之外的源侧续行边。
各侧按行驶顺序查边，报告首个可能重叠的车辆及实际查询边数，毫米闭端相接也算可能重叠。
占用索引保留整毫米位置。车尾刚越过闭端 1 微米时，索引仍可能报告相接；
下述连续扫过查询计入车辆的微米余量，可以确认车尾已越过该端点。
同一车辆若出现在两侧，合并枚举时只出现一次。占用索引在需要时从已提交状态重建。
即使两侧均没有返回车辆，下一拍可能进入的直行车、信号和下游保护区仍须另行核对。

`preview_route_handoff_approaches` 为尚未进入世界的候选检查一段连续运动。输入为后轴从同一起点累计的
三维行程闭区间（微米）。两端共用一次逆查预算，零位移只查一次。沿后轴站位闭区间包住源、目标两侧
可能扫到的全部物理边与边内范围。车头过缝、源侧车尾清空以及只在拍中途触及的边均计入扫过。
每侧最多返回八条边；投影不完整或超过扫过预算时保留失败原因。这个外包用于防止冲突，实体占用仍按车体姿态取样。

查询按世界已提交的车辆更新顺序，沿每辆活动普通车自己的路线核对车头、车尾和两侧区间。
对于前方区间，先在完整路线出现序列中算距离，再扣除车头的微米余量。一辆车已经离开同一物理边时，
车尾可能仍占前一次出现，也可能在下一圈重新接近该区间。各侧报告首个占用或无法证明可以停车的车辆。
服务停车距离包含两拍反应、该进入者车型的服务制动距离和最小间距。若车辆本拍还到不了，但停车距离已不足，
仍报告车辆及所需距离。前杠触及闭区间入口即为占用，包括停在前驱边末端、入口位于后继边零点的情形。
前方仍有正距离时，停车距离等于可用空间可以让行。每个候选显式给出进入者工作预算，预算不足返回未证。

两侧均无阻挡只表示普通车辆可让行。固定步进还须处理拍初信号、下游清空、同拍申请与既有预约，
并在预约持有期间逐拍重验。世界的路线、命令日志和预约均保持已提交状态。

在目标世界调用 `rebind_route_handoff` 可重验全部登记候选窗口。每端从受检制品集的 LFSM 重算来源集合摘要，与 LFCA 核对，再验证走廊所属模块的 LFRE 实物。当前证明要求这份文档的键、长度和内容摘要相同。随后核对各窗范围、路面、两侧路幅、结构边界及窗口内的有向跨越许可，按稳定身份直移完整有序边列并重编译普通机动。全部成立后可以用原车型的稳定身份在目标修订上重新试算。

其他模块可以改变。只有两份 LFCA、同一来源文档内的远处编辑改变了文档字节，或原路线上任一边无法重绑时，本入口返回错误。首版也会拒绝所选共边带护栏的情况，端帽和实体长度可能越过登记端点，只用锚点无法证明窗内净空。正式路线接管的原子性要求见[横断面与路线交接合同][lateral-handoff]。

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

给定起终车道和车辆配置时，宿主可以在同一修订内选路，再向世界提交显式边序列。
宿主区分搜索时的车型禁行与拓扑不连通。Runtime 拒绝注册或绑定时，宿主保留对应命令的错误。
参见[宿主按起终车道和车型静态选路][host-static-routing]。

后续若由 LaneFlow 提供可复用的规划器，应单独设计：

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

**换道能力拒绝衔接注记（#237，规范定义见 [`lane-change-minimal-contract.md`][lane-change-contract] §6）**：
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
[`road-junction-model.md`][road-junction-model]。

[vehicle-following]: vehicle-following.md
[lateral-handoff]: cross-section-stationing-and-lateral-geometry.md#41-从换道意图到一次路线交接
[parking-system]: parking-system.md
[lane-change-contract]: lane-change-minimal-contract.md
[road-junction-model]: road-junction-model.md
[host-static-routing]: traffic-observation-and-routing-integration.md#host-static-routing
