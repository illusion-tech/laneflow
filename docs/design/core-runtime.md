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

状态：建议接受。

Core API 不读取 `Date.now()`、系统时钟、引擎帧时间或全局 clock。调用方必须显式传入 tick 输入。

原因：

- 保持 Core 可测试；
- 避免不同引擎生命周期污染 Core；
- 允许 Adapter 自行处理 variable frame time、暂停、快进和固定步长累积。

### D2. v0.1 使用固定步长语义

状态：建议接受。

v0.1 推荐 Adapter 以固定步长调用 Core。Core 的 tick 输入包含正整数 `deltaTimeMs`，实现不得依赖浮动 wall-clock delta。

约束：

- `deltaTimeMs` 必须大于 0；
- 同一仿真序列内推荐固定值，例如 `16`、`20` 或 `33`；
- variable frame time 应由 Adapter 侧累积并拆成多个固定 tick；
- Core 可以拒绝或警告异常 delta，但 v0.1 不要求复杂时间缩放策略。

### D3. 确定性目标限定在同一实现版本和运行环境

状态：建议接受。

v0.1 的确定性目标是：同一 Core 版本、同一运行环境、同一初始状态和同一 tick/input 序列，输出一致。

v0.1 不承诺跨语言、跨 CPU、跨浮点实现的 bit-level determinism。

原因：

- Core 初始实现目标已收敛为 Rust，但数值策略仍处于 v0.1 最小约束阶段；
- bit-level deterministic math 会显著增加早期成本；
- v0.1 需要先建立可验证 runtime 闭环。

### D4. Runtime step 语义应显式输入输出

状态：建议接受。

Core public API 应接近以下语义：

```text
step(world, input) -> stepResult
```

其中：

- `world` 是当前 Core runtime state；
- `input` 包含 `deltaTimeMs` 和可选 command；
- `stepResult` 至少包含事件列表和本次 step 的可观察结果；纯函数实现可以返回更新后的 world，Rust mutable API 可以通过 `&mut world` 写回状态；
- 实现可以内部优化 mutation，但对调用方应保持无隐藏全局状态的语义。

Rust API 方向可接近：

```rust
fn step(world: &mut CoreWorld, input: TickInput) -> Result<StepResult, CoreError>
```

具体类型命名和错误模型由后续实现 issue 固化，但不得引入隐藏 clock、随机数或引擎全局状态。

### D5. v0.1 只定义最小内部 lane graph / route 输入

状态：建议接受。

v0.1 可以使用最小内部结构表达 lane graph 和 route，但不得把它声明为稳定数据格式。

最小输入：

- lane edge id；
- edge length；
- edge connection；
- route edge id sequence。

v0.2 负责稳定正式 lane graph / route 数据模型、版本、校验和示例数据。

### D6. Simple route following 使用 edge progress

状态：建议接受。

v0.1 车辆沿 route 前进时使用 route edge sequence 和 edge-local progress。

车辆每次 tick：

1. 根据当前速度和 `deltaTimeMs` 计算 travel distance；
2. 增加当前 edge progress；
3. progress 超过当前 edge length 时切到下一 edge；
4. route 结束时进入 `completed` 或 `stopped` 状态。

v0.1 不处理：

- 前车；
- 红绿灯；
- 路口让行；
- 加减速曲线；
- 车道变换；
- 曲线插值和真实世界坐标几何。

### D7. v0.1 vehicle state 保持小集合

状态：建议接受。

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

- `active`
- `stopped`
- `completed`

后续 milestone 可以扩展 acceleration、leader、signal、parking、reservation 等状态，但不得在 v0.1 隐式加入。

### D8. v0.1 不稳定 Adapter API

状态：建议接受。

Core 输出应足以被测试和临时示例消费，但 v0.1 不冻结 Adapter API。正式 Adapter API 应在 `v0.6 First Adapter` 或 `v1.0 Stable Runtime API` 前单独设计。

### D9. Core 初始实现目标为 Rust crate

状态：建议接受。

v0.1 Core 的初始实现目标是 Rust library crate。Rust API 是 v0.1 的首要实现边界；C ABI、WASM、Unity / Unreal / Godot / O3DE Adapter 绑定和稳定 Adapter API 不在本设计中冻结。

原因：

- Rust 适合作为小而可测、可嵌入的 engine-agnostic runtime；
- Rust ownership 和显式 mutation 能清楚表达 Core state 推进；
- Rust 可以通过后续绑定支持 native engine adapter 和 WASM；
- 相比把 Core 写进某个引擎语言，Rust 更能降低引擎依赖反向污染 Core 的风险。

Rust 实现约束：

- Core crate 不得依赖 Unity、Unreal、Godot、O3DE、DOM、WebGL 或 presentation / engine API；
- tick 与时间累计字段优先使用整数类型，例如 `delta_time_ms: u64`、`tick_index: u64`、`time_ms: u64`；
- v0.1 可以用 `f64` 表达 speed / distance，但这不构成跨平台 bit-level determinism 承诺；
- deterministic tests 不得依赖 `HashMap` 等无稳定迭代顺序集合的输出顺序；
- 事件、车辆更新和 edge traversal 输出应有稳定顺序。

## 5. 最小概念模型

```text
CoreWorld
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
  commands?

StepResult
  world
  events
```

事件用于测试和 Adapter 观察，不应成为隐藏控制流。

v0.1 最小事件：

- `vehicle.completedRoute`
- `vehicle.changedEdge`
- `vehicle.stopped`

## 7. Validation 与错误处理

v0.1 runtime 可以假设输入已经过最小校验，但实现 issue 应至少处理以下错误路径：

- route 引用了不存在的 lane edge；
- lane edge length 小于或等于 0；
- vehicle 引用了不存在的 route；
- `deltaTimeMs` 小于或等于 0；
- vehicle speed 小于 0。

v0.2 应把这些规则提升为正式 validation 设计。

## 8. 测试策略

v0.1 最小测试：

- 同一 world 和 tick 序列重复运行，输出一致；
- `deltaTimeMs` 推进后的 `timeMs` 和 `tickIndex` 正确；
- vehicle 按 speed 和 delta 推进 progress；
- progress 跨 edge 时切换到下一 edge；
- route 结束时进入 completed；
- 非法输入失败路径明确。

如果实现时测试框架尚未落地，Rust Core crate 骨架 issue 必须先补最小单元测试能力。

## 9. 与 v0.2 的边界

v0.1 的 lane graph 和 route 结构是实现输入，不是稳定数据格式。

v0.2 必须单独稳定：

- lane graph data model；
- lane connection；
- route definition；
- route validation；
- example route data；
- 兼容性和版本策略。

任何 v0.1 实现 PR 不得把内部结构描述为长期 data spec。

## 10. ADR 判断

Runtime tick 和 determinism 属于高影响设计决策，已新增 `../adr/0003-runtime-tick-and-determinism.md` 作为 Proposed ADR。

Core implementation language 属于高影响设计决策，已新增 `../adr/0004-core-implementation-language.md` 作为 Proposed ADR。

在 v0.1 runtime 实现 PR 合并前，ADR 0003 和 ADR 0004 应进入 `Accepted`，或在 PR 中说明为何推迟冻结。

## 11. 后续实现 issue 输入

后续 v0.1 实现 issue 至少包括：

- 初始化 Rust Core crate 与 `cargo test` 骨架；
- 实现 vehicle state 与显式 tick；
- 实现最小 lane graph traversal；
- 实现 simple route following；
- 补充 v0.1 最小确定性测试。

每个实现 issue 必须引用本文，并在 G2 记录中说明是否改变 Core API、数据格式假设或 Adapter API。
