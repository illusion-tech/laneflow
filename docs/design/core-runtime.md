# Core Runtime Design

**文档状态**: Review

**最后更新**: 2026-07-22

**适用范围**: v0.1 Core Prototype 的 runtime、tick、vehicle state、最小 lane graph traversal 与 simple route following

**关联文档**:

- `../architecture.md`
- `../roadmap.md`
- `../adr/0001-project-scope.md`
- `../adr/0002-dependency-and-licensing-constraints.md`
- `../adr/0003-runtime-tick-and-determinism.md`
- `../adr/0004-core-implementation-language.md`
- `../adr/0005-core-identity-and-handle-model.md`
- `core-id-handles.md`
- `lane-graph.md`
- `route-system.md`
- `data-format.md`
- `parking-system.md`
- `../governance/development-gates.md`

## 1. 目标

本文定义 v0.1 Core runtime 的最小设计基线，使后续实现 issue 不再依赖口头约定。

v0.1 的目标是建立一个可测试、可嵌入、引擎无关的最小交通 runtime：

- 支持显式 tick；
- 支持最小 vehicle state；
- 支持基础 lane graph traversal；
- 支持简单 route following；
- 支持确定性测试；
- 不承诺 v0.2 的稳定数据格式。

### v0.1 历史基线与当前 Accepted 契约

本文保留为 v0.1 Core Prototype 的 `Review` 基线，不是当前默认实现输入。v0.1 中未被后续决策取代的 tick、确定性、错误处理和最小 route following 原则仍可作为历史背景参考；若与后续 Accepted 文档冲突，以当前契约为准。

当前事实来源如下：

- external ID、typed handle、vehicle / route lifecycle 和稳定 update order：[`core-id-handles.md`](core-id-handles.md) D8 及 ADR 0005；
- lane graph topology、edge / connection 语义：[`lane-graph.md`](lane-graph.md)；
- route definition、route target 和 traversal 边界：[`route-system.md`](route-system.md)；
- 当前外部 lane graph / per-edge speed limit / route / Vehicle Profile / Signals / Parking 数据格式、版本、单位和 schema：[`data-format.md`](data-format.md) 与 [`schemas/laneflow-data-v0.7.schema.json`](../../schemas/laneflow-data-v0.7.schema.json)；
- 当前 Vehicle Profile、front-bumper progress、`current_speed`、`applied_acceleration` 和 inactive motion invariant：[`vehicle-following.md`](vehicle-following.md) 第 4-5 节。
- v0.5 Parking 的 static registry/current data 已由 #107 交付，runtime binding/snapshot、`Parked` lifecycle 与同步 commands 已由 #108 交付；#109 已按 [`parking-system.md`](parking-system.md) 激活 moving reservation 的 ParkingStop、arrival、traversal、completion release 与 step events，legacy capability guard在合法 world中不可达。

因此，后续实现不得依据本文要求在每个 tick 按 external ID 字符串排序，也不得把本文的最小内部输入当作当前外部 data-format contract。

## 2. 非目标

v0.1 不覆盖：

- 完整 lane graph / route 数据格式稳定性；
- vehicle following；
- signal / intersection rules；
- parking；
- Engine Adapter API；
- authoring tool；
- 车辆动力学、动画、mesh、LOD、debug draw 或 UI；
- SUMO / CARLA / libsumo 等客户端运行时依赖。

## 3. Core 边界

Core 负责：

- 保存和推进 runtime state；
- 消费引擎无关的 traffic data；
- 根据 tick 输入更新车辆沿 route 的 progress；
- 输出可被 Adapter 读取的车辆运行时状态；
- 保证相同输入下的可重复行为。

Core 不负责：

- 读取真实时间；
- 调度游戏引擎生命周期；
- 管理 actor / entity / prefab / scene object；
- 计算渲染 transform 的引擎坐标转换；
- 调试绘制、UI、动画、LOD；
- 从外部重型仿真服务实时取数。

Adapter 负责把引擎帧循环转换为 Core tick 调用，并把 Core 输出映射到具体引擎对象。

## 4. 设计决策

### D1. Core 使用显式 tick，不读取 wall clock

状态：已接受（ADR 0003）。

Core API 不读取 `Date.now()`、系统时钟、引擎帧时间或全局 clock。调用方必须显式传入 tick 输入。

原因：

- 保持 Core 可测试；
- 避免不同引擎生命周期污染 Core；
- 允许 Adapter 自行处理 variable frame time、暂停、快进和固定步长累积。

### D2. v0.1 使用固定步长语义

状态：已接受（ADR 0003）。

v0.1 Core 是 fixed-step runtime。每个 `CoreWorld` / simulation session 在初始化时确定一个正整数 `fixedDeltaTimeMs`；v0.1 不冻结全局唯一 tick 数值，但同一 session 运行中不得改变该值。

约束：

- `fixedDeltaTimeMs` 必须大于 0；
- v0.1 不固定具体值，实现和示例可以选择 `16`、`20` 或 `33` 等固定步长；
- Core step 不接受任意 variable delta；若 `TickInput` 保留 `deltaTimeMs` 字段，该值必须等于当前 world 的 `fixedDeltaTimeMs`；
- delta 不一致时应返回明确 validation error，而不是按 variable delta 推进；
- variable frame time、catch-up、drop/backlog、暂停、慢放、快进和 render interpolation 应由 Adapter 或上层 scheduler 处理。

### D3. 确定性目标限定在同一实现版本和运行环境

状态：已接受（ADR 0003）。

v0.1 的确定性目标是：同一 Core 版本、同一运行环境、同一初始状态和同一 tick/input 序列，输出一致。

v0.1 不承诺跨语言、跨 CPU、跨浮点实现的 bit-level determinism。

原因：

- Core 初始实现目标已收敛为 Rust，但数值策略仍处于 v0.1 最小约束阶段；
- bit-level deterministic math 会显著增加早期成本；
- v0.1 需要先建立可验证 runtime 闭环。

### D4. Runtime step 语义应显式输入输出

状态：已接受（ADR 0003 / ADR 0004）。

Core public API 应接近以下语义：

```text
step(world, input) -> stepResult
```

其中：

- `world` 是当前 Core runtime state；
- `input` 可以包含用于校验的 `deltaTimeMs`，但不得携带 v0.1 不支持的运行时 command；
- `stepResult` 至少包含本次 step 的可观察结果；纯函数实现可以返回更新后的 world，Rust mutable API 默认通过 `&mut world` 写回状态，`StepResult` 不重复携带 world；
- 实现可以内部优化 mutation，但对调用方应保持无隐藏全局状态的语义。

Rust API 方向可接近：

```rust
fn step(world: &mut CoreWorld, input: TickInput) -> Result<StepResult, CoreError>
```

具体类型命名和错误模型由后续实现 issue 固化，但不得引入隐藏 clock、随机数或引擎全局状态。ADR 0003 中的 `step(world, input) -> stepResult` 是概念表达，不要求 Rust API 使用纯函数形态。v0.1 Rust API 若采用 `&mut CoreWorld`，成功返回后的 `world` 即为更新后状态；`StepResult` 应保留 `tickIndex`、`timeMs` 和 `events` 等观察结果，避免为了返回 world 克隆完整 runtime state。

### D5. v0.1 只定义最小内部 lane graph / route 输入

状态：已接受。

v0.1 可以使用最小内部结构表达 lane graph 和 route，但不得把它声明为稳定数据格式。

最小输入：

- lane edge id；
- edge length；
- edge connection；
- route edge id sequence。

约束：

- lane edge id、route id 和 vehicle id 在各自集合内必须唯一；
- route 必须至少包含一个 edge id；
- route 引用的所有 edge id 必须存在；
- route edge sequence 必须连通：相邻 edge 中，下一 edge id 必须出现在当前 edge 的 `nextEdgeIds`；`nextEdgeIds` 引用的 edge id 必须存在；
- `nextEdgeIds` 的顺序不参与 v0.1 route following；route edge sequence 是车辆实际行驶顺序；
- lane graph 中的 edge id 必须唯一，但 route edge sequence 可以重复引用同一 edge id；重复 edge 表示有限 route 中的显式回环，车辆位置由 `routeEdgeIndex` 区分；
- v0.1 内部结构只作为实现输入和测试 fixture，不是长期 data spec。

v0.1 Rust 实现使用 `EdgeLength` newtype 暴露 lane edge length。#125 已把最小 edge length 输入下限与后续 edge boundary/remainder 阈值拆成两个 crate-private 领域 owner；current-f64 数值都保持 `1.0e-9`，但不再通过统一公开常量耦合。`LaneGraph` / `Route` 可以使用 `indexmap` 作为内部稳定顺序存储，但 public API 不暴露 `IndexMap`，避免把内部容器选择冻结为长期 data spec。

本文的 v0.1 最小输入仅作为历史实现输入和测试 fixture。v0.2 的正式 lane graph / route 数据模型、版本、校验和示例数据已经由 `lane-graph.md`、`route-system.md`、`data-format.md` 和 JSON Schema 接受；后续实现必须以这些文档为准。

### D6. Simple route following 使用 edge progress

状态：已接受。

v0.1 车辆沿 route 前进时使用 route edge sequence 和 edge-local progress。距离单位为 engine-agnostic distance unit；示例可按 meter 理解，speed 单位为 distance_units_per_second。正式外部数据单位由 v0.2 data spec 冻结。

车辆每次 tick：

1. 如果 `status` 不是 `active`，不推进位置；
2. 根据当前速度和 `fixedDeltaTimeMs` 计算 travel distance；
3. 若 travel distance 小于或等于 edge boundary/remainder 阈值，不推进位置，也不触发 edge boundary transition；
4. 若 travel distance 或计算出的下一个 progress 不是 finite number，step 必须返回 validation error 且 world 不变；
5. 增加当前 edge progress；
6. 当 `edgeProgress + edge_boundary_tolerance >= edge length` 时进入 edge boundary 处理；
7. 若 route 还有下一 edge，emit `vehicle.changedEdge`，切换到下一 edge 并携带 remainder；
8. 若 route 已到最后 edge，emit `vehicle.completedRoute`，status 改为 `completed`，`routeEdgeIndex` 保持最后 edge index，`edgeProgress` clamp 到最后 edge length；
9. 一个 tick 可以跨越多个 edge，事件按实际跨越顺序输出。

边界规则：

- 若 `edgeProgress` 与 `edge length` 的差值命中 edge boundary 领域阈值，吸附（snap）到 boundary；该 owner 不得与输入最小尺寸或物理间距阈值互相别名；
- remainder 严格小于 edge boundary/remainder 阈值时按 0 处理，等于阈值时不归零；
- active vehicle 即使初始 progress 已在 edge boundary，若本 tick travel distance 小于或等于 edge boundary/remainder 阈值，也不得仅因 boundary proximity 触发 transition；
- 单 tick 跨多 edge 的实现必须有硬上界；v0.1 Rust 实现以上限为当前 route 剩余 edge 数，每次 transition 必须递增 `routeEdgeIndex`，到最后 edge 后完成并退出；
- completed 事件只在从 `active` 进入 `completed` 的 tick 发出一次；
- `completed` / `stopped` vehicle 后续 tick 不再移动；
- `speed` 字段表示车辆配置或当前期望速度；当 status 为 `stopped` 或 `completed` 时，Core step 使用的 effective speed 为 0，字段值不因状态转换被隐式改写。

v0.1 不处理：

- 前车；
- 红绿灯；
- 路口让行；
- 加减速曲线；
- 车道变换；
- 曲线插值和真实世界坐标几何。

### D7. v0.1 vehicle state 保持小集合

状态：已接受。

最小 vehicle state：

```text
VehicleState
  id
  routeId
  routeEdgeIndex
  edgeProgress
  speed
  status
```

`status` 最小值：

- `active`：随 fixed tick 沿 route 推进；
- `stopped`：手工或初始保持状态，v0.1 不因前车、信号或路口规则自动进入该状态；
- `completed`：route 结束后的终止状态。

约束：

- `routeId` 必须引用存在的 route；
- `routeEdgeIndex` 必须落在 route edge sequence 范围内；
- `edgeProgress` 必须落在当前 edge 的 `[0, edge length]` 范围内；
- `speed` 必须大于或等于 0，且必须是 finite number；
- `stopped` 和 `completed` 车辆不移动；`active` 且 `speed = 0` 合法，但不会产生 route progress；
- `completed` / `stopped` 不会隐式把 `speed` 字段归零，Adapter 或调试工具应以 `status` 判断 effective movement。
- 初始 `completed` vehicle 必须位于 route 最后一个 edge，且 `edgeProgress` 必须等于或在 edge boundary 阈值内接近最后 edge length；v0.1 Rust 实现会把合法的近终点 progress snap 到最后 edge length。若 `completed` vehicle 位于 route 中间或 progress 未到终点，应返回 validation error。

v0.1 Rust 实现使用 `Speed` 与 `EdgeProgress` newtype 暴露 `speed` 和 `edgeProgress`，而不是在 public API 中直接散落裸 `f64`。调用方必须通过 newtype constructor 创建这两个值；constructor 负责拒绝 `NaN`、`Infinity`、`-Infinity` 和负数。

后续 milestone 可以扩展 acceleration、leader、signal、parking、reservation 等状态，但不得在 v0.1 隐式加入。

### D8. v0.1 不稳定 Adapter API

状态：已接受。

Core 输出应足以被测试和临时示例消费，但 v0.1 不冻结 Adapter API。当前路线图先由 `v0.6 Numeric & Spatial Foundation` 冻结数值、空间几何与最小查询输入，再在 `v0.7 Bevy Reference Adapter` 开工前通过独立 design/G1 冻结实际 Adapter 契约；未来 Stable Runtime API Milestone 只在真实宿主证据和 #72/#199 的 [`core-runtime-scalability-audit.md`](core-runtime-scalability-audit.md) 基础上承诺长期兼容。

### D9. Core 初始实现目标为 Rust crate

状态：已接受（ADR 0004）。

v0.1 Core 的初始实现目标是 Rust library crate。Rust API 是 v0.1 的首要实现边界；C ABI、WASM、Unity / Unreal / Godot / O3DE Adapter 绑定和稳定 Adapter API 不在本设计中冻结。初始仓库边界采用 Cargo workspace，并把 Core crate 放在 `crates/laneflow-core`；crate 名称使用 `laneflow_core`。

原因：

- Rust 适合作为小而可测、可嵌入的 engine-agnostic runtime；
- Rust ownership 和显式 mutation 能清楚表达 Core state 推进；
- Rust 可以通过后续绑定支持 native engine adapter 和 WASM；
- 相比把 Core 写进某个引擎语言，Rust 更能降低引擎依赖反向污染 Core 的风险。

Rust 实现约束：

- Core crate 不得依赖 Unity、Unreal、Godot、O3DE、DOM、WebGL 或 presentation / engine API；
- tick 与时间累计字段使用整数类型，例如 `fixed_delta_time_ms: u64`、`tick_index: u64`、`time_ms: u64`；
- v0.1 可以用 `f64` 表达 speed、distance 和 edge progress，但这不构成跨平台 bit-level determinism 承诺；
- deterministic tests 不得依赖 `HashMap` 等无稳定迭代顺序集合的输出顺序；
- 事件、车辆更新和 edge traversal 输出应有稳定顺序；
- 距离、速度和 progress 若跨模块或 public API 暴露，应优先使用 Rust newtype 包装，而不是在 API 边界散落裸 `f64`；
- lane edge length 若跨模块或 public API 暴露，应使用 `EdgeLength` newtype，并拒绝 `NaN`、`Infinity`、`-Infinity` 和小于或等于领域专用最小 edge length 的值；
- 测试应对 tick/time/status/index/events 使用精确断言，对 speed/distance/progress 使用带单位、领域明确的断言阈值；
- edge boundary 和 route completion 等离散行为必须通过稳定、领域化的绝对阈值与吸附规则转换为精确事件和状态断言。

## 5. 最小概念模型

```text
CoreWorld
  fixedDeltaTimeMs
  tickIndex
  timeMs
  laneGraph
  routes
  vehicles

LaneGraph
  edges: LaneEdge[]

LaneEdge
  id
  length
  nextEdgeIds

Route
  id
  edgeIds

VehicleState
  id
  routeId
  routeEdgeIndex
  edgeProgress
  speed
  status
```

## 6. Tick 输入与输出

```text
TickInput
  deltaTimeMs

StepResult
  tickIndex
  timeMs
  events
```

v0.1 Rust 实现将 `TickInput.delta_time_ms` 固化为必填字段，且该值必须等于 `CoreWorld.fixedDeltaTimeMs`；不一致时返回 validation error。这样 Adapter 或测试调用方必须显式暴露 fixed-step delta mismatch。若未来版本选择移除该字段，或改为完全由 `CoreWorld` 配置决定 tick 长度，应先更新本设计。

成功 step 的时间语义：若 step 前 `world.tickIndex = N`、`world.timeMs = T`，则 validation 和 state update 成功后，`world.tickIndex = N + 1`、`world.timeMs = T + fixedDeltaTimeMs`；`StepResult.tickIndex`、`StepResult.timeMs` 和所有 event 的 `tickIndex` 使用更新后的 post-step 值。失败 step 不递增 tick/time，不修改 world，也不产生可观察事件。

v0.1 不支持 runtime commands。暂停、恢复、车辆注入、速度修改、路径切换等 command API 必须在后续设计中单独定义，不得通过未文档化字段进入 Core。

事件用于测试和 Adapter 观察，不应成为隐藏控制流。

v0.1 最小事件：

- `vehicle.completedRoute`
- `vehicle.changedEdge`

通用 event payload：

```text
Event
  tickIndex
  vehicleId
  kind
```

`vehicle.changedEdge` payload：

```text
routeId
fromEdgeId
toEdgeId
fromRouteEdgeIndex
toRouteEdgeIndex
```

`vehicle.completedRoute` payload：

```text
routeId
edgeId
routeEdgeIndex
```

v0.1 Rust 实现使用结构化 enum 和 payload structs 表达事件，而不是用字符串 `kind` 做内部分派：

```text
CoreEvent
  VehicleChangedEdge(VehicleChangedEdgeEvent)
  VehicleCompletedRoute(VehicleCompletedRouteEvent)
```

事件顺序必须稳定。以下 `vehicleId` 排序只描述 v0.1 历史基线：

- v0.1 基线按 `vehicleId` 升序；v0.1 推荐使用 ASCII identifier，若 `vehicleId` 为字符串，排序使用 Rust `str` / `String` 的 `Ord` 字典序；若后续改为数值 id，则使用数值升序；
- v0.2 使用 `core-id-handles.md` D8 的稳定 `vehicleUpdateOrder`，初始化时一次按 external ID 确定初始顺序，tick 热路径直接遍历 active handle；不得每 tick 排序 external ID 字符串；
- 单个 vehicle 在同一 tick 内按实际 route transition 顺序输出事件；
- v0.1 vehicle 之间不发生交互，因此更新顺序只影响可观察事件顺序，不影响位置计算结果；
- 实现不得依赖 `HashMap` 等无稳定迭代顺序集合直接决定 event order。

## 7. Validation 与错误处理

v0.1 应把校验分为两个阶段：`CoreWorld` 初始化校验静态 graph / route / initial vehicle 数据；`step` 校验 tick input 和运行时不变量。无论哪个阶段失败，都必须返回明确错误，不得产生部分状态。实现 issue 至少处理以下错误路径：

- lane edge id、route id 或 vehicle id 重复；
- route 引用了不存在的 lane edge；
- route 为空；
- route edge sequence 不连通；
- lane edge length 小于或等于 0，或不是 finite number；
- vehicle 引用了不存在的 route；
- vehicle `routeEdgeIndex` 越界；
- vehicle `edgeProgress` 超出当前 edge 的 `[0, edge length]`；
- `fixedDeltaTimeMs` 小于或等于 0；
- `TickInput.deltaTimeMs` 存在但不等于 `fixedDeltaTimeMs`；
- vehicle speed 小于 0，或不是 finite number；
- vehicle `edgeProgress`、edge length、speed 等 `f64` 值为 `NaN`、`Infinity` 或 `-Infinity`；
- simple route following 计算出的 travel distance 或下一个 progress 不是 finite number；
- 初始 `completed` vehicle 未位于 route 最后 edge，或 progress 不在最后 edge length 的 edge boundary 阈值范围内；
- edge length 小于或等于最小 edge length 输入下限时应返回 validation error；该输入语义不再由 boundary snap 阈值顺带决定；
- 未支持的 runtime command 字段不得被接受为隐式行为。

v0.1 Rust 实现中，带 traffic data 和 vehicles 的 world 必须通过 `CoreWorld::with_traffic_data(fixed_delta_time_ms, lane_graph, routes, vehicles)` 创建并完成上述静态校验；不得保留绕过 graph / route / vehicle 一致性校验的非空 vehicles 构造入口。`CoreWorld::new(fixed_delta_time_ms)` 仅用于创建空 graph、空 routes 和空 vehicles 的基础 tick world。

step validation error 必须保持原子性：返回错误时不得部分推进 `tickIndex`、`timeMs`、vehicle state 或 events。实现可以通过预校验、临时结果或 compute-then-apply 达成该语义。初始化校验失败同样不得返回可部分使用的 `CoreWorld`。

v0.2 应把这些规则提升为正式 validation 设计。

## 8. 测试策略

v0.1 最小测试：

- 同一 world 和 tick 序列重复运行，输出一致；
- `fixedDeltaTimeMs` 在 world 初始化后保持不变；
- fixed tick 推进后的 `timeMs` 和 `tickIndex` 正确，event 使用 post-step tick/time；
- 若 `TickInput.deltaTimeMs` 与 `fixedDeltaTimeMs` 不一致，应返回 validation error；
- vehicle 按 speed 和 fixed delta 推进 progress；
- progress 跨 edge 时切换到下一 edge；
- 一个 tick 跨多个 edge 时输出完整且有序的 transition events；
- route 结束时进入 completed；
- `stopped` 和 `completed` vehicle 后续 tick 不移动；
- 多 vehicle 事件按 `vehicleId` 升序稳定输出，并覆盖字符串 id 排序规则；
- event payload 包含可定位 route transition 的最小字段；
- 非法输入失败路径明确，且初始化失败不返回部分 world、step 失败不部分修改 world；
- tick/time/status/index/event order 使用精确断言；
- speed/distance/progress 使用带单位、领域明确的断言阈值，并覆盖 `NaN` / `Infinity` rejection；
- edge boundary 和 route completion 的事件与状态使用精确断言。

如果实现时测试框架尚未落地，Rust Core crate 骨架 issue 必须先补最小单元测试能力。

## 9. Canonical v0.1 test fixture

后续实现 issue 应优先使用以下 fixture 作为最小确定性测试基线：

```text
CoreWorld
  fixedDeltaTimeMs: 1000
  tickIndex: 0
  timeMs: 0

LaneEdge A
  length: 10.0
  nextEdgeIds: [B]

LaneEdge B
  length: 5.0
  nextEdgeIds: []

Route R
  edgeIds: [A, B]

Vehicle V1
  routeId: R
  routeEdgeIndex: 0
  edgeProgress: 0.0
  speed: 6.0
  status: active
```

预期结果：

- tick 1：`world.tickIndex = 1`、`timeMs = 1000`；`V1` 留在 `A`，`edgeProgress = 6.0`，无 route transition event；
- tick 2：`world.tickIndex = 2`、`timeMs = 2000`；`V1` 从 `A` 切到 `B`，`edgeProgress = 2.0`，emit `vehicle.changedEdge(A -> B)`，event `tickIndex = 2`；
- tick 3：`world.tickIndex = 3`、`timeMs = 3000`；`V1` 完成 `R`，`status = completed`，`routeEdgeIndex = 1`，`edgeProgress = 5.0`，emit `vehicle.completedRoute`，event `tickIndex = 3`；
- tick 4：`world.tickIndex = 4`、`timeMs = 4000`；`V1` 保持 completed，不移动且不重复发出 completed event。

补充 fixture：

- `fixedDeltaTimeMs = 1000` 但 `TickInput.deltaTimeMs = 500` 时返回 validation error，world 不变；
- `edgeProgress = 10.0 - edge_boundary_tolerance / 2` 且 travel distance 足以过线时应 snap 到 boundary，并按 edge transition 精确发出事件；
- v0.1 基线中，多 vehicle 输入顺序不同但 id 集合相同时，events 仍按 `vehicleId` 升序输出；v0.2 对应测试应断言 `vehicleUpdateOrder` 的稳定顺序，不应要求每 tick external ID 排序；
- route `[A, B, A]` 在 `B.nextEdgeIds` 包含 `A` 时合法，用于验证重复 edge 依赖 `routeEdgeIndex` 区分位置；
- 任一 `f64` 输入为 `NaN` / `Infinity` 或 edge length 小于等于最小 edge length 输入下限时 validation error，world 不变。

## 10. 与 v0.2 的边界

v0.1 的 lane graph 和 route 结构是实现输入，不是稳定数据格式。

v0.2 已单独稳定：

- lane graph data model；
- lane connection；
- route definition；
- route validation；
- example route data；
- 兼容性和版本策略。

当前 v0.2 的正式输入分别见 `lane-graph.md`、`route-system.md`、`data-format.md` 和 JSON Schema。任何后续实现不得把 v0.1 内部结构描述为长期 data spec，也不得以本文否定已接受的 v0.2 格式。

## 11. ADR 判断

Runtime tick 和 determinism 属于高影响设计决策，已通过 `../adr/0003-runtime-tick-and-determinism.md` 接受。

Core implementation language 属于高影响设计决策，已通过 `../adr/0004-core-implementation-language.md` 接受。

后续 v0.1 runtime 实现 PR 必须引用 ADR 0003 和 ADR 0004；若实现需要改变 fixed-step、Rust crate 或 f64 测试口径，应先更新 ADR 或拆分新的设计 issue。

## 12. 历史 v0.1 实现 issue 输入

本节保留 v0.1 原型阶段的历史拆分记录，不表示这些工作仍是当前路线图待办；当前 v0.2 后续工作应从 Accepted v0.2 设计和 roadmap 的对应 Issue 进入。

后续 v0.1 实现 issue 至少包括：

- 初始化 Cargo workspace、`crates/laneflow-core` Rust Core crate 与 `cargo test` 骨架；
- 实现 vehicle state 与显式 tick；
- 实现最小 lane graph traversal；
- 实现 simple route following；
- 补充 v0.1 最小确定性测试。

每个实现 issue 必须引用本文，并在 G2 记录中说明是否改变 Core API、数据格式假设或 Adapter API。
