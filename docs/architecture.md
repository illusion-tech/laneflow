# 架构

**文档状态**: Accepted（current + #291 target design；目标实现尚未交付）<br>
**最后更新**: 2026-07-29<br>
**适用范围**: LaneFlow 当前分层、Rust crate 依赖方向、Traffic Data、Road/Junction/Maneuver、Signals、Parking、场景人口与 Core/Adapter 边界，以及 #291/ADR 0020/0021 的城市模拟游戏交通基础与目标静态编译架构

## 1. 架构目标

LaneFlow 当前是一个引擎无关、可嵌入的交通运行时。Accepted ADR 0021 把“为未来的
中国特色城市模拟游戏提供交通基础”定义为第一长期产品目标；#291 G1 已接受该目标
和下述目标态分层，但对应实现尚未交付。

当前架构与 #291 已接受目标态设计共同关注：

- Core 与具体游戏引擎解耦。
- 数据格式可以被工具、示例和多个 Adapter 共享。
- Adapter 负责引擎集成和表现层，不复制 Core 交通规则。
- 示例场景用于验证最小可用闭环。
- 城市经济、市民出行需求、土地利用和游戏规则由上层拥有，LaneFlow 通过显式
  命令、快照、事件和路径接入边界提供交通能力。
- 城市级目标必须同时保持确定性、可诊断、可存档、可修改路网和可扩展性，不能用
  多世界吞吐或表现层细节层次替代单个大型交通世界的正确性。

#291 已接受的长期权威目标分层为：

```text
城市模拟游戏层
  -> 出行与交通编排层
  -> 路径规划服务
  -> LaneFlow 编译器 / 静态镜像 + LaneFlow 交通运行时
  -> 引擎适配器 / 表现层
```

出行需求决定谁在何时为何出发；交通运行时导出已提交交通观测快照；路径规划/
出行编排层再结合静态路网、观测、收费、游戏政策和偏好构造动态成本快照并生成候选
路径；交通运行时只验证/注册由候选路径构成的动态通行定义，并负责交通参与单元如何
在所属执行域安全推进。当前 Core 只实现道路机动车车辆特化；长期通用抽象不把
非机动车、行人或轨道交通排除在目标交通运行时（Target Traffic Runtime）之外。
目标产品边界见 Accepted ADR 0021；Accepted 状态不表示对应交通运行时已经实现。

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
laneflow-data -> laneflow-current-source
laneflow-core -X-> laneflow-data

Engine Adapter -> laneflow-core
Engine Adapter -> laneflow-data  (按需加载外部数据)
```

外部格式可以依赖 Core domain types 做 normalization；Core 不反向依赖 JSON、Serde、JSON Schema、文件系统或 Adapter。详细决策见 `adr/0007-traffic-data-crate-and-loader-boundary.md`。

v0.6 #123 已在 G1 接受引擎无关的空间层（Spatial Layer），#133 已建立首个生产 `laneflow-spatial` crate。当前与目标依赖方向为：

```text
laneflow-spatial -> laneflow-core
laneflow-spatial -> laneflow-static-contract  (共享静态空间数值边界)
laneflow-data -> laneflow-spatial  (只在空间包加载与绑定路径)
引擎适配器 -> laneflow-core / laneflow-spatial / laneflow-data
laneflow-core -X-> laneflow-spatial
```

Core 继续拥有拓扑、长度、进度与交通行为的权威职责；Spatial 拥有有界 local canonical frame、中心线、弧长、绑定与位姿采样；Adapter 只把 LaneFlow 位姿映射为宿主变换（Transform）。当前 `laneflow-spatial` 已实现 LaneFlow-owned canonical `f32` 基础类型、每轴 `±16_384 m` 点范围、稳定 frame ID、结构化错误、按 `LaneGraph::edges()` 排序的 immutable registry、量化后折线绑定/采样，以及带 batch-level placement token、Parking pose 和失败原子性的批量提取。#134 的空间包/清单 loader 可直接构造该 registry；#137 继续负责误差、分配、内存和一万/十万性能基线。

### 2.1 #291 目标静态编译分层

Accepted ADR 0020 的目标态不在上述 current 链条旁增加 L1/L2，而是把全部静态
路网编译前移：

本节术语以 [`reference/glossary.md`](reference/glossary.md) 的中文定义为权威，
英文只作辅助理解；代码和制品标识符保留精确拼写。

```text
Road Editing / Synthetic DSL / imported / other checked source modules
  -> authoritative source module graph
  -> typed AST -> HIR -> MIR -> validated canonical LIR
  -> portable canonical artifact + target StaticNetworkImage + source map + semantic diff

StaticNetworkImage
  -> required cold StaticIdentityIndex for Runtime Snapshot / Image Cutover identity translation
  -> laneflow-runtime: required StaticTrafficView + per-world mutable state
     + per-world RuntimeExecutionPlan
  -> laneflow-spatial: optional StaticSpatialView + pose scratch/output
  -> Adapter: trusted image descriptor + committed snapshot + pose batch
```

这里的 `semantic diff` 是 #298 基于规范 LIR 的路网影响差异；C 阶段道路编辑控制点等
authoring-only 差异由后继 [#345](https://github.com/illusion-tech/laneflow/issues/345)
`RoadEditingSourceDiff` 负责。

compiler 拥有静态 identity、topology、geometry、owner/member、coverage、length、
initial/static occurrence 与 dense layout；target `LaneFlow Traffic Runtime`
（`laneflow-runtime`）继续拥有 tick、已实现执行域的交通参与单元、动态通行定义
（Dynamic Traversal Definition）和其他可变交通权威（Mutable Traffic
Authority），Spatial 继续拥有位姿采样（Pose Sampling）。
生产启动只从外部可信描述符（Trusted Descriptor）/验证收据（Validation Receipt）
认证版本化静态镜像完整性清单（Static Image Integrity Manifest），再对目标节完成
分块（Chunk）完整性和有界结构验证；不解析 JSON、不按外部标识（External ID）重绑定、
重建登记表或重复 Traffic/Spatial 联结。全镜像 SHA-256 保留为发布身份、独立重建与
显式完整审计（Full Audit），不强制每次启动先串行读取未消费节。交通节、冷稳定
身份索引（Static Identity Index，`StaticIdentityIndex`）与分区规划提示
（Partition Planning Hints，`PartitionPlanningHints`）必选；Spatial section 由
closed profile 控制，headless
Runtime 不携带 geometry。稳定身份索引不进入 steady tick，可由共享映射、压缩或按需
分页控制内存成本，但任何 production profile 都不得删除它。目标职责和历史 ADR 的
取代范围见 ADR 0020。
分块验证和有类型视图必须共享不可变字节背板（Immutable Byte Backing）；宿主不能
保证资产在视图生命周期内不可替换/改写时，必须复制封存已验证分块或拒绝建立可信视图。

目标静态镜像必须保存编译器派生的静态执行约束图（Static Execution Constraint
Graph）；v1 `PartitionPlanningHints` 节保存运行时可忽略或重建的分区规划提示
（Partition Planning Hints），但不得保存最终分区/工作线程分配
（Partition/Worker Assignment）。每个世界依据这些约束、硬件与动态负载建立自己的
运行时执行计划（Runtime Execution Plan）。精确执行的所有分区只读取已提交状态
`T`，在同一逻辑边界原子提交 `T + Δ`；不能因跨分区而额外延迟一 tick。互不相交的
资源依赖组件可以并行归约，但每个连接资源组件必须有唯一、规范的归约权威。该权威
定义唯一结果而非单一物理线程；生产方案必须用资源分段、强连通分量（Strongly
Connected Component，SCC）、凝聚有向无环图（Condensation Directed Acyclic
Graph，Condensation DAG）与稳定合并证明归约工作量/跨度（Reduction Work/Span），
并以集中式组件归约作为精确参考预言机。

静态也不等于城市永不变化：编译器每次产生不可变路网修订（Network Revision），
运行世界通过失败关闭的镜像切换事务（Image Cutover Transaction）迁移。默认在线
流程在旧世界继续固定步进时准备候选，以有界迁移增量日志（Migration Delta
Journal）记录已提交动态状态/生命周期变化及命令/事件游标，并让候选重解释这条
已提交变更流；最后在安全边界的静默提交窗口（Quiescent Commit Window）排空日志尾
并把新镜像/状态绑定与规范排序的切换事件批次原子地只发布一次。候选不得重新执行
输入、独立推进未来时间线或产生第二份已提交事件；失败时不发布切换事件并继续旧修订。
语义差异不能自行授予迁移权限，必须由独立验证或外部可信的
路网修订切换描述符（Network Revision Cutover Descriptor）绑定，并用切换前后的
稳定身份索引完成引用翻译。每个修订由 independent validator 从目标无关规范路网
语义载荷（Canonical Network Semantic Payload）重算的路网修订标识（Network
Revision ID）`NetworkRevisionId` 认证；验证收据、静态镜像描述符及切换描述符分别
绑定该标识，Runtime 不接受调用方或镜像头自报修订。
目标职责、上层边界与历史 ADR 的关系见 ADR 0020/0021；
在二者 Accepted 且阶段 8 生产切换 Issue #294 完成 G4 前，本文其余 current 章节
继续有效。

## 3. Authoring Layer

Authoring Layer 负责生成或编辑交通数据：

- 道路编辑
- 车道编辑
- 路线编辑
- 红绿灯配置
- 停车位配置
- 示例数据生成

它可以是独立工具、引擎编辑器插件或离线转换脚本。

#291 target 中，唯一逻辑 authoring authority 是显式、可重放的 source module graph。
ADR 0023 进一步把城市项目/存档中的道路编辑状态定义为 production 编制事实：可视化
编辑器与可发布程序化生成器共享有类型道路编辑模型；产品负责人已为 #296 选择按模块
size-prefixed FlatBuffers 作为持久化/交换编码，具体契约仍待当前 G1 exact-head 审阅通过。
Synthetic DSL source 继续只服务测试、benchmark、示例和
非发布程序化场景；importer 保存原始 source digest、tool/options/provenance。全部正式
来源输出 owning module、稳定实体/属性或画布选择位置及 typed AST，不直接构造 current
Core、target Runtime 或 Spatial 对象。

道路建成后仍可修改。近期产品通过候选道路的整体编译/验证和新路网修订替换实现；长期
在同一事务上增加直接调整与影响预览。道路编辑状态保存重建当前走向所需的编制定义，
运行时镜像只保留规范折线和静态表，不保存控制点或交互历史。

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

ADR 0020 target 中，`laneflow-data` 只作为 current JSON 临时内部加载实现；
portable canonical artifact 由 `laneflow-format`/compiler contract 描述，生产
Runtime 由 `laneflow-static-image` 的 trusted descriptor + bounded verifier/view
挂载。静态 semantic normalization 从 Data/Core constructors 前移到 compiler，
static image 不取代 public publication/provenance/validation-receipt 契约。current JSON
未曾作为外部资产发布，不接入 compiler，也不形成长期兼容或迁移工具承诺。

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
而是共享 `StaticTrafficView`。每个 `TrafficWorld` 只拥有已实现执行域的交通参与
单元、动态通行定义、控制器/预约/停驻状态（Stationary State）等可变数组，
以及世界 identity、输入命令游标、运行时执行计划和当前路网修订绑定。当前
投影仍是车辆/动态路线/控制器/预约/停车特化；人口、
Routing 和游戏规则 seed 仍由 caller/出行编排层拥有；Runtime 只有在后续 G1 显式
授予随机权威时才拥有相应随机流。
Initial/static occurrence 由 compiler
预编译，dynamic Route occurrence 仍由 Runtime 按 image index 编译，steady tick
继续只使用 typed dense handle。

运行时快照（Runtime Snapshot）是与镜像字节分离的版本化制品，必须绑定原规范制品
摘要与精确长度、版本化路网修订标识、原始静态镜像摘要与精确长度、运行时/约束
版本、world identity、tick、输入命令游标和全部每世界可变状态；同一修订可以在
runtime/snapshot 契约兼容、identity/constraint/execution-constraint versions
精确相等且 `StaticIdentityIndex` 能完整重建引用时恢复到另一个可信
target/profile image，即使后者因 compiler provenance 或 artifact envelope 重发布而
规范制品摘要不同。原规范制品/镜像摘要只作为审计绑定与同字节快速路径。dense
ordinal 不能跨路网修订直接复用。
任何保留旧状态的跨修订切换/恢复都必须消费经独立验证、由可信切换描述符绑定的语义
差异；`StaticIdentityIndex` 只复核 StableId128 ↔ typed ordinal 映射，不能证明
语义兼容，缺失该证据时迁移失败关闭。
回放使用显式输入命令流、checkpoint 与确定性状态摘要，调试构建可通过冷诊断和源映射
生成失同步诊断制品。交通运行时按观测导出节奏（Observation Export Cadence）导出
完整基线或版本化增量/分区选择的已提交交通观测；路径规划据此构造动态成本快照，
不进入交通参与单元 fixed-tick 热路径，也不要求每 tick 全量复制全网。

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
它也不得把细节层次、可见性或帧预算转换为交通 fidelity：宿主可以暂停、慢放或
统一改变模拟时间推进速度，但不能静默丢 fixed tick、丢事件或让不同分区读取不同
逻辑时点。任何多频率或 aggregate 降级必须由独立 fidelity contract 和 G1 冻结。

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
