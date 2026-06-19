# Core Runtime Design

**文档状态**: Review

**最后更新**: 2026-06-20

**适用范围**: v0.1 Core Prototype 的 runtime、tick、vehicle state、最小 lane graph traversal 与 simple route following

**关联文档**:

- `../architecture.md`
- `../roadmap.md`
- `../adr/0001-project-scope.md`
- `../adr/0002-dependency-and-licensing-constraints.md`
- `../adr/0003-runtime-tick-and-determinism.md`
- `../adr/0004-core-implementation-language.md`
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
- `stepResult` 至少包含事件列表和本次 step 的可观察结果；纯函数实现可以返回更新后的 world，Rust mutable API 可以通过 `&mut world` 写回状态；
- 实现可以内部优化 mutation，但对调用方应保持无隐藏全局状态的语义。

Rust API 方向可接近：

```rust
fn step(world: &mut CoreWorld, input: TickInput) -> Result<StepResult, CoreError>
```

具体类型命名和错误模型由后续实现 issue 固化，但不得引入隐藏 clock、随机数或引擎全局状态。ADR 0003 中的 `step(world, input) -> stepResult` 是概念表达，不要求 Rust API 使用纯函数形态。

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
- route edge sequence 必须连通：相邻 edge 中，下一 edge id 必须出现在当前 edge 的 `nextEdgeIds`；
- `nextEdgeIds` 的顺序不参与 v0.1 route following；route edge sequence 是车辆实际行驶顺序；
- v0.1 内部结构只作为实现输入和测试 fixture，不是长期 data spec。

v0.2 负责稳定正式 lane graph / route 数据模型、版本、校验和示例数据。

### D6. Simple route following 使用 edge progress

状态：已接受。

v0.1 车辆沿 route 前进时使用 route edge sequence 和 edge-local progress。距离单位为 engine-agnostic distance unit；示例可按 meter 理解，speed 单位为 distance_units_per_second。正式外部数据单位由 v0.2 data spec 冻结。

车辆每次 tick：

1. 如果 `status` 不是 `active`，不推进位置；
2. 根据当前速度和 `fixedDeltaTimeMs` 计算 travel distance；
3. 增加当前 edge progress；
4. 当 `edgeProgress + epsilon >= edge length` 时进入 edge boundary 处理；
5. 若 route 还有下一 edge，emit `vehicle.changedEdge`，切换到下一 edge 并携带 remainder；
6. 若 route 已到最后 edge，emit `vehicle.completedRoute`，status 改为 `completed`，`routeEdgeIndex` 保持最后 edge index，`edgeProgress` clamp 到最后 edge length；
7. 一个 tick 可以跨越多个 edge，事件按实际跨越顺序输出。

边界规则：

- 若 `edgeProgress` 与 `edge length` 的差值在 epsilon 内，snap 到 boundary；`epsilon` 必须是实现中单一命名常量，并在测试中显式使用；
- remainder 小于 epsilon 时按 0 处理；
- completed 事件只在从 `active` 进入 `completed` 的 tick 发出一次；
- `completed` / `stopped` vehicle 后续 tick 不再移动。

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
- `speed` 必须大于或等于 0；
- `stopped` 和 `completed` 车辆不移动；`active` 且 `speed = 0` 合法，但不会产生 route progress。

后续 milestone 可以扩展 acceleration、leader、signal、parking、reservation 等状态，但不得在 v0.1 隐式加入。

### D8. v0.1 不稳定 Adapter API

状态：已接受。

Core 输出应足以被测试和临时示例消费，但 v0.1 不冻结 Adapter API。正式 Adapter API 应在 `v0.6 First Adapter` 或 `v1.0 Stable Runtime API` 前单独设计。

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
- 测试应对 tick/time/status/index/events 使用精确断言，对 speed/distance/progress 使用明确 epsilon；
- edge boundary 和 route completion 等离散行为必须通过稳定 epsilon / snap 规则转换为精确事件和状态断言。

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
  deltaTimeMs?

StepResult
  world
  events
```

`deltaTimeMs` 若存在，必须等于 `CoreWorld.fixedDeltaTimeMs`；后续实现也可以选择不在 `TickInput` 中暴露 delta，而由 `CoreWorld` 配置决定 tick 长度。

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

事件顺序必须稳定：

- vehicle 更新顺序按 `vehicleId` 升序；
- 单个 vehicle 在同一 tick 内按实际 route transition 顺序输出事件；
- v0.1 vehicle 之间不发生交互，因此更新顺序只影响可观察事件顺序，不影响位置计算结果；
- 实现不得依赖 `HashMap` 等无稳定迭代顺序集合直接决定 event order。

## 7. Validation 与错误处理

v0.1 runtime 可以假设输入已经过最小校验，但实现 issue 应至少处理以下错误路径：

- lane edge id、route id 或 vehicle id 重复；
- route 引用了不存在的 lane edge；
- route 为空；
- route edge sequence 不连通；
- lane edge length 小于或等于 0；
- vehicle 引用了不存在的 route；
- vehicle `routeEdgeIndex` 越界；
- vehicle `edgeProgress` 超出当前 edge 的 `[0, edge length]`；
- `fixedDeltaTimeMs` 小于或等于 0；
- `TickInput.deltaTimeMs` 存在但不等于 `fixedDeltaTimeMs`；
- vehicle speed 小于 0；
- 未支持的 runtime command 字段不得被接受为隐式行为。

step validation error 必须保持原子性：返回错误时不得部分推进 `tickIndex`、`timeMs`、vehicle state 或 events。实现可以通过预校验、临时结果或 compute-then-apply 达成该语义。

v0.2 应把这些规则提升为正式 validation 设计。

## 8. 测试策略

v0.1 最小测试：

- 同一 world 和 tick 序列重复运行，输出一致；
- `fixedDeltaTimeMs` 在 world 初始化后保持不变；
- fixed tick 推进后的 `timeMs` 和 `tickIndex` 正确；
- 若 `TickInput.deltaTimeMs` 与 `fixedDeltaTimeMs` 不一致，应返回 validation error；
- vehicle 按 speed 和 fixed delta 推进 progress；
- progress 跨 edge 时切换到下一 edge；
- 一个 tick 跨多个 edge 时输出完整且有序的 transition events；
- route 结束时进入 completed；
- `stopped` 和 `completed` vehicle 后续 tick 不移动；
- 多 vehicle 事件按 `vehicleId` 升序稳定输出；
- event payload 包含可定位 route transition 的最小字段；
- 非法输入失败路径明确，且错误不部分修改 world；
- tick/time/status/index/event order 使用精确断言；
- speed/distance/progress 使用明确 epsilon 断言；
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

- tick 1：`V1` 留在 `A`，`edgeProgress = 6.0`，无 route transition event；
- tick 2：`V1` 从 `A` 切到 `B`，`edgeProgress = 2.0`，emit `vehicle.changedEdge(A -> B)`；
- tick 3：`V1` 完成 `R`，`status = completed`，`routeEdgeIndex = 1`，`edgeProgress = 5.0`，emit `vehicle.completedRoute`；
- tick 4：`V1` 保持 completed，不移动且不重复发出 completed event。

补充 fixture：

- `fixedDeltaTimeMs = 1000` 但 `TickInput.deltaTimeMs = 500` 时返回 validation error，world 不变；
- `edgeProgress = 10.0 - epsilon / 2` 且 travel distance 足以过线时应 snap 到 boundary，并按 edge transition 精确发出事件；
- 多 vehicle 输入顺序不同但 id 集合相同时，events 仍按 `vehicleId` 升序输出。

## 10. 与 v0.2 的边界

v0.1 的 lane graph 和 route 结构是实现输入，不是稳定数据格式。

v0.2 必须单独稳定：

- lane graph data model；
- lane connection；
- route definition；
- route validation；
- example route data；
- 兼容性和版本策略。

任何 v0.1 实现 PR 不得把内部结构描述为长期 data spec。

## 11. ADR 判断

Runtime tick 和 determinism 属于高影响设计决策，已通过 `../adr/0003-runtime-tick-and-determinism.md` 接受。

Core implementation language 属于高影响设计决策，已通过 `../adr/0004-core-implementation-language.md` 接受。

后续 v0.1 runtime 实现 PR 必须引用 ADR 0003 和 ADR 0004；若实现需要改变 fixed-step、Rust crate 或 f64 测试口径，应先更新 ADR 或拆分新的设计 issue。

## 12. 后续实现 issue 输入

后续 v0.1 实现 issue 至少包括：

- 初始化 Cargo workspace、`crates/laneflow-core` Rust Core crate 与 `cargo test` 骨架；
- 实现 vehicle state 与显式 tick；
- 实现最小 lane graph traversal；
- 实现 simple route following；
- 补充 v0.1 最小确定性测试。

每个实现 issue 必须引用本文，并在 G2 记录中说明是否改变 Core API、数据格式假设或 Adapter API。
