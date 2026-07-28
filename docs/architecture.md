# 架构

**文档状态**: Accepted（current）＋ Draft（#291 compiler target）<br>
**最后更新**: 2026-07-28<br>
**适用范围**: LaneFlow 当前分层、Rust crate 依赖方向、Traffic Data、Road/Junction/Maneuver、Signals、Parking、场景人口与 Core/Adapter 边界，以及 #291/ADR 0020 的目标静态编译架构

## 1. 架构目标

LaneFlow 是一个引擎无关的轻量 NPC 车流 runtime。

核心架构目标：

- Core 与具体游戏引擎解耦。
- 数据格式可以被工具、示例和多个 Adapter 共享。
- Adapter 负责引擎集成和表现层，不复制 Core 交通规则。
- 示例场景用于验证最小可用闭环。

## 2. 分层

```text
Authoring Layer
  │
  v
Traffic Data Layer (`laneflow-data`)
  │
  v
LaneFlow Core (`laneflow-core`)
  │
  v
Engine Adapter Layer
  │
  v
Presentation Layer
```

当前 Rust crate 依赖方向固定为：

```text
laneflow-data -> laneflow-core
laneflow-core -X-> laneflow-data

Engine Adapter -> laneflow-core
Engine Adapter -> laneflow-data  (按需加载外部数据)
```

外部格式可以依赖 Core domain types 做 normalization；Core 不反向依赖 JSON、Serde、JSON Schema、文件系统或 Adapter。详细决策见 `adr/0007-traffic-data-crate-and-loader-boundary.md`。

v0.6 #123 已在 G1 接受引擎无关的空间层（Spatial Layer），#133 已建立首个生产 `laneflow-spatial` crate。当前与目标依赖方向为：

```text
laneflow-spatial -> laneflow-core
laneflow-data -> laneflow-spatial  (只在空间包加载与绑定路径)
引擎适配器 -> laneflow-core / laneflow-spatial / laneflow-data
laneflow-core -X-> laneflow-spatial
```

Core 继续拥有拓扑、长度、进度与交通行为的权威职责；Spatial 拥有有界 local canonical frame、中心线、弧长、绑定与位姿采样；Adapter 只把 LaneFlow 位姿映射为宿主变换（Transform）。当前 `laneflow-spatial` 已实现 LaneFlow-owned canonical `f32` 基础类型、每轴 `±16_384 m` 点范围、稳定 frame ID、结构化错误、按 `LaneGraph::edges()` 排序的 immutable registry、量化后折线绑定/采样，以及带 batch-level placement token、Parking pose 和失败原子性的批量提取。#134 的空间包/清单 loader 可直接构造该 registry；#137 继续负责误差、分配、内存和 10k/100k 性能基线。

### 2.1 #291 目标静态编译分层

ADR 0020 Proposed 的 target 不在上述 current 链条旁增加 L1/L2，而是把全部静态
网络编译前移：

本节术语以 [`reference/glossary.md`](reference/glossary.md) 的中文定义为权威，
英文只作辅助理解；代码和制品标识符保留精确拼写。

```text
Geometry / Synthetic DSL / imported / editor-authored source modules
  -> authoritative source module graph
  -> typed AST -> HIR -> MIR -> validated canonical LIR
  -> portable canonical artifact + target StaticNetworkImage + source map + semantic diff

StaticNetworkImage
  -> laneflow-runtime: required StaticTrafficView + per-world mutable state
  -> laneflow-spatial: optional StaticSpatialView + pose scratch/output
  -> Adapter: trusted image descriptor + committed snapshot + pose batch
```

compiler 拥有静态 identity、topology、geometry、owner/member、coverage、length、
initial/static occurrence 与 dense layout；target `LaneFlow Traffic Runtime`
（`laneflow-runtime`）继续拥有 tick、vehicle、dynamic Route 和其他可变 traffic
authority，Spatial 继续拥有 pose sampling。Production startup 只从外部 trusted
descriptor/validation receipt 绑定并结构验证 static image，不解析 JSON、按 external
ID rebind、重建 registry 或重复 Traffic/Spatial join。Traffic section 必选，Spatial
section 由 closed profile 控制，headless Runtime 不携带 geometry。目标职责和历史
ADR 的取代范围见 ADR 0020；在其 Accepted 且迁移 G4 前，本文其余 current 章节继续
有效。

## 3. Authoring Layer

Authoring Layer 负责生成或编辑交通数据：

- 道路编辑
- 车道编辑
- 路线编辑
- 红绿灯配置
- 停车位配置
- 示例数据生成

它可以是独立工具、引擎编辑器插件或离线转换脚本。

#291 target 中，唯一 authoring authority 是显式、可重放的 source module graph。
Geometry 是主要 production language，Synthetic DSL source 是测试/benchmark 的
权威 module；importer 保存原始 source digest、tool/options/provenance，Editor 默认
持久化 Geometry module。它们输出带 owning module/source span 的 typed AST，不直接
构造 current Core、target Runtime 或 Spatial 对象。

## 4. Traffic Data Layer

Traffic Data Layer 保存 Core 可消费的数据：

- lane graph
- route
- signal
- parking
- spawn rules
- vehicle profiles

数据格式应尽量保持引擎无关。

当前 Rust workspace 中，Traffic Data Layer 已由 `laneflow-data` 表达。它负责：

- 当前 v0.10 external package（横断面/准入静态模型、profile 必填 `participantClassId`）、required per-edge `speedLimit`、必填版本闸口与旧版/未来版拒绝；
- JSON syntax、wire shape、units 和字段路径诊断；
- external ID 到 Core domain input 的转换；
- 调用 Core constructors 完成 lane graph、Junction/Movement/ManeuverPath、route、
  Vehicle Profile、ParticipantClass、RoadCorridor/RoadSection/LaneGroup/FacilityBand、
  AccessRule、multi-Gate/WaitingZone、static Signals 与 static Parking normalization。

`laneflow-data` 不拥有 fixed tick、runtime entity、world lifecycle 或 Engine asset I/O。初始 loader 接收内存 bytes/string，不直接读取文件或创建 `CoreWorld`。

ADR 0020 target 中，`laneflow-data` 只作为 current JSON compatibility façade；
portable canonical artifact 由 `laneflow-format`/compiler contract 描述，生产
Runtime 由 `laneflow-static-image` 的 trusted descriptor + bounded verifier/view
挂载。静态 semantic normalization 从 Data/Core constructors 前移到 compiler，
static image 不取代 public publication/provenance/validation-receipt 契约。

current v0.10 在保持相同依赖方向的前提下包含 per-edge 基础道路限速、
Junction/Movement/ManeuverPath、StopLine、一等 ManeuverGate、SignalGroup、
fixed-time Controller/Phase、immutable ParkingArea/ParkingSpace、entry/exit
anchors 和 edge-relative geometry，以及 #262 生产化的 ParticipantClass、
RoadCorridor/RoadSection/LaneGroup/FacilityBand 横断面与 AccessRule 准入静态模型
（profile 必填 `participantClassId`），以及 #281 的 multi-Gate、
WaitingZone static registry/route occurrence/绑定期 capability guards，并由
canonical fixtures 锁定。详细契约见
`design/data-format.md` 与 `design/data-loading.md`。

Traffic Data 只承载 immutable ParkingArea/ParkingSpace、entry/exit anchors 与 edge-relative geometry，不持久化 reservation、occupancy、initial parked vehicles 或 runtime handles。#107 已原子切换 schema、private DTO、loader、fixtures 与 current docs；#108 的 runtime authority 完全保留在 CoreWorld，不回写 production data。

#229 已按 #228/ADR 0017 把 Traffic 原子切换为 v0.8：clean break 增加
Junction、Movement、ManeuverPath，并以一等 ManeuverGate 取代 pair-based Gate。
RoadSection、LaneGroup 与 JunctionGroup 只冻结长期语义，不进入该次 schema
（Traffic `0.8`，即 v0.9 产品里程碑的数据格式）。
SpatialPackage/ScenarioManifest 保持 v0.1；完整实现与边界见
`design/road-junction-model.md`。

#234 已按 ADR 0018 冻结多模式横断面与准入分层的长期语义：`RoadCorridor` 作为
横断面唯一 owner 组织方向性 `RoadSection` 与非遍历 `FacilityBand`，
`FacilityKind`（物理设施）、`ParticipantClass`（数据声明的单继承参与者分类）与
`AccessRule`（target/effect/时间窗口/法规 provenance 的准入 overlay）显式分离。
其 Core/Data 生产化与 Traffic `0.8 -> 0.9` 原子迁移已由 #234 拆出的最小
production Issue #262 交付（v0.9 schema 已发布并经 live 验证）；
SSOT 见 `design/cross-section-access.md`。

#235/ADR 0019 已接受多阶段复杂路口 G1：沿用 ManeuverPath/Route authority，把
multiple Gate、WaitingZone 与 ConflictZone occurrence 在 Route 注册期编译；
Traffic/Core 中显式声明的 ParticipantStream/ConflictZone 关系拥有行为 authority，
并复用 current Traffic v0.10 的 ParticipantClass/AccessRegistry 静态准入结果；
Spatial 只拥有 canonical 3D geometry/validation。SSOT 见
`design/waiting-zone-conflict-right-of-way.md`；其中 #281 static/Data 已生产化，
#282–#285 的 runtime/Conflict/policy/组合验证仍不构成 current API 声称。

## 5. LaneFlow Core

LaneFlow Core 负责运行时交通逻辑：

- vehicle state
- route following
- lane graph traversal
- vehicle following
- signal compliance
- intersection rules
- parking behavior

Core 不依赖具体游戏引擎 API。

Rust workspace 中，Core 由 `laneflow-core` 表达。Core 拥有 `InitialTrafficData`、lane graph、route、Vehicle Profile、typed handle、registry/resolver 和全部 domain/runtime invariant。

这句话描述 current。ADR 0020 target 把动态执行层 clean-break 重命名为
`LaneFlow Traffic Runtime` / `laneflow-runtime`，target public world 为
`TrafficWorld`。Static/shared contract 移入 `laneflow-static-contract` 与
`laneflow-static-image`；Runtime 不再从 `InitialTrafficData` 构建静态 registries，
而是共享 `StaticTrafficView`。每个 `TrafficWorld` 只拥有 vehicle、dynamic Route、
controller/reservation/parking 等可变 arrays。Initial/static occurrence 由 compiler
预编译，dynamic Route occurrence 仍由 Runtime 按 image index 编译，steady tick
继续只使用 typed dense handle。

`InitialTrafficData` 只表示可用于初始化 world 的已验证静态输入，当前包含 lane
graph、Junction registry、compiled routes、Vehicle Profiles 与 immutable
Signals/Parking registries，不拥有 tick、initial vehicles 或 runtime route
generation。初始 route validation 与 runtime route registration 复用同一 Core
compiler，包括 Maneuver occurrence、Gate 与 route-final-StopLine 约束。

Signals 在 Core 内保持四层职责：Controller 产生 indication；ManeuverGate/StopLine
表达空间准入；compliance policy 解释 signal-layer permission；纵向 constraint、
安全投影与 permission-aware traversal 保证结果不可绕过。SignalController 不硬编码
国家/转向规则，Adapter 只 query/render。长期分层见 ADR 0009、
`design/signal-system.md` 与 `reference/v0.4-closure-review.md`。

#229 在不改变上述职责分层的前提下实现了 ADR 0017：
`Junction -> Movement -> ManeuverPath` immutable owner hierarchy、derived
internal-edge Junction ownership 和一等 ManeuverGate。Route 继续拥有车辆实际
traversal；initial/dynamic Route 在注册期编译 Maneuver/Gate occurrences，vehicle
tick 不匹配 path、不查 external ID 或扫描全局 catalog。Core current API 只公开
Junction/Movement/ManeuverPath/ManeuverGate handles 和 resolvers，不保留 pair key。

#234/ADR 0018 在此之上冻结横断面与准入的长期职责边界：横断面是 LaneGraph 之上的
可选结构 overlay（RoadCorridor/RoadSection 引用 edge，不复制拓扑权威）；准入规则
作为 regulatory constraint 进入 Core constraint 管线，静态规则在 (class, Route)
绑定期原子校验，时变规则以 capability guard 拦截直到独立 G1；任何 allow 不覆盖 Core
safety。上述 ParticipantClass/CrossSection/Access registries、typed handles 与静态
route-binding 校验已由 #262 生产化；时变准入与 FacilityBand target 仍未生产化。

#235/ADR 0019 Accepted 设计在 signal/regulatory 与 Core safety 之间增加独立 conflict
domain：versioned compliance policy 只产生 protected/permissive/uncontrolled
candidate，Core ConflictArbiter 才能结合 yield/priority、gap acceptance、zone
occupancy/reservation 产生 vehicle-specific tick grant；crossing 成功后 grant
原子升级为 reservation。v1 grant 前还必须以 route-local occupancy/hard boundary
证明车辆能让车尾清空 coverage zone，并在 stable single-writer arbitration 中原子
取得 Waiting/Conflict/physical downstream claim；不能证明下游存储或与
committed/earlier-staged claim 冲突时 fail closed。candidate proposal 可以并行，
但线程调度/锁竞争不能决定 winner。
SignalController、Adapter、JunctionGroup 与二维几何均不拥有最终通行权；任何
grant 不覆盖 leader、safe-speed、RouteEnd、minimum-gap 或 no-overlap。该设计已
通过 G1，但仍须由后续独立 implementation slices 生产化并完成各自 G0-G4。

v0.5 Parking runtime 由 Core 私有 binding aggregate 持有唯一 authority；`VehicleStatus::Parked` 与 exact Occupied binding 一致，Parked vehicle 保留 live identity但不进入 travel-lane occupancy。#108 已公开 borrowed snapshot 和 caller-selected lifecycle commands；#109 已把 ParkingStop、SignalStop、RouteEnd 与 leader/no-overlap 纳入同一 fixed-tick constraint/traversal pipeline，并交付 arrival、route-completion release、step events 与 Reserved capability activation。Adapter 只消费 immutable registry、snapshot、records/events 和 position authority。详细设计见 ADR 0010 与 `design/parking-system.md`。

## 6. Engine Adapter Layer

Engine Adapter 负责把 Core 状态映射到具体引擎：

- tick 调用
- actor / entity 生命周期
- transform 同步
- mesh / prefab / scene object 绑定
- debug draw
- UI 面板
- LOD 和性能策略

Adapter 不应把引擎依赖引入 Core。

Adapter 可以按需调用 `laneflow-data` 解析自身 asset pipeline 已读取的内存数据，但不得要求 Core 理解引擎路径、asset handle 或异步加载协议。

ADR 0020 target 中，Adapter/宿主 asset pipeline 提供 static image bytes 与 image
外部的 trusted descriptor/validation receipt，经 bounded verifier 后把 Traffic
view 交给 Runtime，并在 profile 含 Spatial 时把对齐 Spatial view 交给 Spatial。
Adapter 不读取 compiler IR、portable artifact 语义，也不拥有 image 内静态规则。

ADR 0013/0015 与 #136 已冻结适配器边界。各 Adapter 不再自行定义中心线和长度采样权威；它们从已提交的 Core 快照构造稳定的 Lane/Parking 输入，消费带 frame identity 和 placement token 的 `f32` canonical 批量位姿，并只在末端处理 frame 放置、坐标轴、坐标系手性、宿主变换、插值和细节层次（LOD）。详细设计见 ADR 0013、ADR 0015、`design/spatial-geometry.md` 与 `design/adapter-api.md`。

v0.7 的首个生产 Adapter crate 为 `laneflow-bevy`。它依赖 `laneflow-core`、`laneflow-spatial` 和 Bevy 0.19 的最小 modular crates，使用一个 Bevy Resource 表达单活动 Session，并在宿主 `First` 之后运行 LaneFlow 自有 outer-frame/fixed schedules；它不修改 Bevy `Time<Fixed>`，也不把 Bevy 类型引入 Core/Data/Spatial。#169-#173 已完成 Plugin/Session、Adapter-owned Vehicle/Entity 部分双射、显式 frame root/token、transform propagation 前两阶段原子 local Transform 同步、headless/performance Gate、预算受控 Gizmos 与 native reference example。最终契约、证据与兼容边界见 `design/bevy-reference-adapter.md` 和 `reference/v0.7-bevy-closure-review.md`。

v0.8 的场景人口与回流采用 ADR 0016 的 caller-owned policy，不进入 `CoreWorld::step` 隐藏状态，也不由 Bevy ECS 选择 route。Core 不提供人口 controller、车辆数量限制、seed 或 portal/route 决策，只拥有 caller-driven 的原子 old-handle/new-handle replace 与交通 invariant；Adapter 以 typed transaction 原子切换同一 Entity 的 handle binding。`laneflow-scenario` 中的 #203 reference policy 按 fixed-step input sequence 拥有目标人口、seed、catalog normalization、portal/lane 决策和 blocked retry，依赖方向固定为 `laneflow-scenario -> laneflow-core`，未来城市游戏可以完全替换该 policy。Traffic/Spatial/Manifest 继续是静态制品，不持久化目标人口、runtime handles 或 Entity。场景目标见 `design/example-scenarios.md`，policy 实现契约见 `design/signalized-corridor-population.md`；production 其余切片由 #185–#189 承担。

## 7. Presentation Layer

Presentation Layer 负责用户可见效果：

- 车辆模型
- 道路表现
- 动画
- 灯光
- 调试可视化
- 示例场景 UI

Presentation 可以因引擎不同而完全不同。
