# 架构

**文档状态**: Accepted（current + #291 target design + ADR 0025 / #300 G1 修订；#301 后 Runtime 为当前可运行世界）<br>
**最后更新**: 2026-08-23<br>
**适用范围**: LaneFlow 当前分层、Rust crate 依赖方向、Traffic Data、Road/Junction/Maneuver、Signals、Parking、场景人口与 Runtime/Adapter 边界，以及 #291/ADR 0020/0021 和 Accepted ADR 0025 的城市模拟游戏交通基础与目标静态编译架构

## 1. 架构目标

LaneFlow 当前是一个引擎无关、可嵌入的交通运行时。Accepted ADR 0021 把“为未来的
中国特色城市模拟游戏提供交通基础”定义为第一长期产品目标；#291 G1 已接受该目标
和下述目标态分层。#300 / #301 已交付共享静态路网与 `TrafficWorld` 当前可运行路径；
出行编排、Routing 与多执行域尚未交付。

当前架构与 #291 已接受目标态设计共同关注：

- 交通运行时与具体游戏引擎解耦。
- 数据格式可以被工具、示例和多个 Adapter 共享。
- Adapter 负责引擎集成和表现层，不复制交通规则。
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
  -> LaneFlow 编译器 / 共享静态路网 + LaneFlow 交通运行时
  -> 引擎适配器 / 表现层
```

出行需求决定谁在何时为何出发；交通运行时导出已提交交通观测快照；路径规划/
出行编排层再结合静态路网、观测、收费、游戏政策和偏好构造动态成本快照并生成候选
路径；交通运行时只验证/注册由候选路径构成的动态通行定义，并负责交通参与单元如何
在所属执行域安全推进。当前 `TrafficWorld` 只实现道路机动车车辆特化；长期通用抽象不把
非机动车、行人或轨道交通排除在目标交通运行时（Target Traffic Runtime）之外。
目标产品边界见 Accepted ADR 0021。#301 已使 `TrafficWorld` 成为唯一可运行交通世界。

## 2. 分层

```text
Authoring / Compiler Layer
  │
  v
Shared Static Network (`laneflow-static-network`)
  │
  v
LaneFlow Traffic Runtime (`laneflow-runtime`)
  │
  v
Engine Adapter Layer
  │
  v
Presentation Layer
```

当前 Rust crate 依赖方向固定为：

```text
laneflow-runtime -> laneflow-static-network
laneflow-runtime -> laneflow-static-contract
laneflow-spatial -> laneflow-static-network
laneflow-spatial -> laneflow-static-contract
laneflow-runtime -X-> laneflow-spatial
laneflow-spatial -X-> laneflow-runtime

Engine Adapter -> laneflow-runtime
Engine Adapter -> laneflow-spatial
```

Runtime 不依赖 JSON、Serde、JSON Schema、文件系统、compiler 或 Adapter。current JSON 不再安装可运行世界。详细决策见 `design/traffic-runtime-shared-consumption.md`。

v0.6 #123 已在 G1 接受引擎无关的空间层（Spatial Layer），#133 已建立首个生产 `laneflow-spatial` crate。当前与目标依赖方向为：

```text
laneflow-spatial -> laneflow-static-network
laneflow-spatial -> laneflow-static-contract  (共享静态空间数值边界)
引擎适配器 -> laneflow-runtime / laneflow-spatial
laneflow-runtime -X-> laneflow-spatial
```

#301 后 Spatial 绑定共享根 `Arc`，不再依赖已拆除的 `laneflow-core`。Spatial 拥有有界 local canonical frame、中心线、弧长与位姿采样；Adapter 只把 LaneFlow 位姿映射为宿主变换（Transform）。当前 `laneflow-spatial` 已实现 LaneFlow-owned canonical `f32` 基础类型、每轴 `±16_384 m` 点范围、稳定 frame ID、结构化错误，以及绑定 `SharedNetworkRevision` 的 `SpatialSession` 批量提取。

### 2.1 #291 目标静态编译分层

Accepted ADR 0020 的目标态不在上述 current 链条旁增加 L1/L2，而是把全部静态
路网编译前移。Accepted ADR 0025（#300 G1 Pass）进一步取消独立静态镜像文件/ABI，
改为以受检 LFCA 构建进程内共享静态路网：

本节术语以 [`reference/glossary.md`](reference/glossary.md) 的中文定义为权威，
英文只作辅助理解；代码和制品标识符保留精确拼写。

```text
Road Editing / Synthetic DSL / imported / other checked source modules
  -> authoritative source module graph
  -> typed AST -> HIR -> MIR -> validated canonical LIR
  -> LFCA + LFSM + LFSD
  -> laneflow-format checked LFCA capability
  -> laneflow-static-network -> SharedNetworkRevision

SharedNetworkRevision
  -> required SharedTrafficNetwork + SharedIdentityIndex + PartitionPlanningHints
  -> laneflow-runtime: shared immutable Traffic data + per-world mutable state
     + per-world RuntimeExecutionPlan
  -> laneflow-spatial: optional SharedSpatialNetwork; lane_pose capability + pose scratch/output
  -> Adapter: Runtime/Spatial public API + committed snapshot + pose batch
```

这里的 `semantic diff` 是 #298 基于规范 LIR 的路网影响差异；C 阶段道路编辑控制点等
authoring-only 差异由后继 [#345](https://github.com/illusion-tech/laneflow/issues/345)
`RoadEditingSourceDiff` 负责。

compiler 拥有静态 identity、topology、geometry、owner/member、coverage、length、
initial/static occurrence 与 dense layout；target `LaneFlow Traffic Runtime`
（`laneflow-runtime`）继续拥有 tick、已实现执行域的交通参与单元、动态通行定义
（Dynamic Traversal Definition）和其他可变交通权威（Mutable Traffic
Authority），Spatial 继续拥有位姿采样（Pose Sampling）。
Accepted ADR 0024 已冻结 LFCA 后发射检查和最小发布闭合。Accepted ADR 0025 让发布加载和
玩家确认建造在 admission 之后共用同一条构建路径：宿主发布加载先认证 LFCP v2/
manifest 绑定，本地玩家编辑则直接消费 `PostEmissionCheckedBundleV1`；两者都由
`laneflow-format` 提供受检 LFCA capability，再由 `laneflow-static-network` 完成
跨表引用、身份双射、Traffic/Spatial 对齐和执行约束闭合。目标 Runtime 不解析 JSON、
LFCA 或 compiler-private LIR，也不按 external ID 重建登记表。

`SharedNetworkRevision` 的 Traffic、冷稳定身份索引和分区规划提示必选，Spatial
component 可选。headless 构建不保留 geometry；稳定身份索引不进入 steady tick，
但任何构建模式都不得删除它。Spatial component 可以是 facility/profile/frame-only；只有
非空 LaneEdge geometry 才形成完整 lane-pose sampling capability。共享结果拥有自己的连续
数组且不借用 LFCA backing。runtime-only 世界可以在构建后释放 LFCA；可编辑 session 为
后续 LFSD 保留的 exact base LFCA 由 editor/#302 独立拥有，不进入共享根。#300 v1 不定义
静态镜像 descriptor、integrity manifest、chunk、mmap、磁盘 cache 或 target/profile 文件变体。

LFCA 的规范关系保存编译器派生的静态执行约束事实（Static Execution Constraint
Facts）；LFCA v1 不保存提示 payload。`laneflow-static-network` 按显式非语义 derivation
version 确定性派生 `PartitionPlanningHints` component，Runtime 可以忽略或重建，但不得保存
最终分区/工作线程分配（Partition/Worker Assignment）。每个世界依据这些约束、硬件与
动态负载建立自己的
运行时执行计划（Runtime Execution Plan）。精确执行的所有分区只读取已提交状态
`T`，在同一逻辑边界原子提交 `T + Δ`；不能因跨分区而额外延迟一 tick。互不相交的
资源依赖组件可以并行归约，但每个连接资源组件必须有唯一、规范的归约权威。该权威
定义唯一结果而非单一物理线程；生产方案必须用资源分段、强连通分量（Strongly
Connected Component，SCC）、凝聚有向无环图（Condensation Directed Acyclic
Graph，Condensation DAG）与稳定合并证明归约工作量/跨度（Reduction Work/Span），
并以集中式组件归约作为精确参考预言机。

静态也不等于城市永不变化：编译器每次产生不可变路网修订（Network Revision），
运行世界通过失败关闭的路网修订切换事务（Network Revision Cutover Transaction）迁移。默认在线
流程在旧世界继续固定步进时准备候选，以有界迁移增量日志（Migration Delta
Journal）记录已提交动态状态/生命周期变化及命令/事件游标，并让候选重解释这条
已提交变更流；最后在安全边界的静默提交窗口（Quiescent Commit Window）排空日志尾
并把新共享修订/状态绑定与规范排序的切换事件批次原子地只发布一次。候选不得重新执行
输入、独立推进未来时间线或产生第二份已提交事件；失败时不发布切换事件并继续旧修订。
语义差异不能自行授予迁移决定，必须由对象外可信的
路网修订切换描述符（Network Revision Cutover Descriptor）绑定，并用切换前后的
稳定身份索引完成引用翻译。Accepted ADR 0024 的 compiler 后发射检查从目标无关规范路网语义载荷
（Canonical Network Semantic Payload）重算路网修订标识（Network Revision ID）
`NetworkRevisionId`；LFCP/manifest、共享修订 origin 及切换描述符按各自职责绑定该
标识，Runtime 不接受调用方或 LFSD 自报修订。#300/#302 分别冻结共享静态路网和切换
输入。当前 production descriptor 是不含 receipt 的 LFCP v2；LFCP v1 只保留为
#298 已实现的历史契约。目标职责、上层
边界与历史 ADR 的关系见 ADR 0020/0021 及 Accepted ADR 0024。#301 完成前，本文其余
current 章节描述仓库内仍可运行的 Core 路径；#301 完成后这些章节须改为历史描述或删除，
以 `docs/design/traffic-runtime-shared-consumption.md` 为准。

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
编辑器与可发布程序化生成器共享有类型道路编辑模型；#296 已实现按模块
size-prefixed FlatBuffers 的内部 B1 production compiler 入口。该格式尚未发布，也不承担
长期存档兼容承诺。
Synthetic DSL source 继续只服务测试、benchmark、示例和
非发布程序化场景；importer 保存原始 source digest、tool/options/provenance。全部正式
来源输出 owning module、稳定实体/属性或画布选择位置及 typed AST，不直接构造 current
Core、target Runtime 或 Spatial 对象。

道路建成后仍可修改。近期产品通过候选道路的整体编译/验证和新路网修订替换实现；长期
在同一事务上增加直接调整与影响预览。道路编辑状态保存重建当前走向所需的编制定义，
共享静态路网只保留规范折线和静态表，不保存控制点或交互历史。鼠标拖动与预览不触发
完整编译；用户确认建造后才构建候选。城市存档只保存活动 Runtime 修订的 committed
network source：可编辑世界保存道路编辑状态，只从发布 LFCA 启动的 runtime-only 世界
保存绑定 LFCA digest/length/revision 的持久 asset reference；working/candidate 不进入
存档。发布资产要进入道路编辑流程，必须同时提供可重编译并核对到同一修订的 committed
道路编辑状态，不能从 LFCA 逆向恢复 authoring-only 曲线；#302 先用重编译 exact LFCA 原子
rebase root/source/diff-base binding，再启用编辑。可编辑 session 内存保留当前 exact base
LFCA 供下一次 `PortableDiffBase::Artifact`，但存档仍只保存 committed 道路状态。

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

ADR 0020/0025 target 中，`laneflow-data` 只作为 current JSON 临时内部加载实现；
portable canonical artifact 由 `laneflow-format`/compiler contract 描述，生产
Runtime 由 `laneflow-static-network` 从受检 LFCA 构建并挂载
`SharedNetworkRevision`。静态 semantic normalization 从 Data/Core constructors 前移到
compiler；共享静态路网不取代 LFCA publication/provenance 与宿主 admission 契约。current JSON
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

## 5. LaneFlow 交通运行时

LaneFlow 交通运行时负责运行时交通逻辑：

- vehicle state
- route following
- lane graph traversal
- vehicle following
- signal compliance
- intersection rules
- parking behavior

Runtime 不依赖具体游戏引擎 API。

Rust workspace 中，当前可运行世界由 `laneflow-runtime` 的 `TrafficWorld` 表达。
`laneflow-core` / `CoreWorld` 已拆除。Static/shared contract 在
`laneflow-static-contract` 与 `laneflow-static-network`；Runtime 共享
`SharedTrafficNetwork`，不再从 `InitialTrafficData` 构建静态 registries。
每个 `TrafficWorld` 只拥有已实现执行域的交通参与
单元、动态通行定义、控制器/预约/停驻状态（Stationary State）等可变数组，
以及世界 identity、输入命令游标、运行时执行计划和当前路网修订绑定。当前
投影仍是车辆/动态路线/停车占用特化；人口、
Routing 和游戏规则 seed 仍由 caller/出行编排层拥有；Runtime 只有在后续 G1 显式
授予随机权威时才拥有相应随机流。
Initial/static occurrence 由 compiler
预编译，dynamic Route occurrence 仍由 Runtime 按 typed dense handle 编译，steady tick
继续只使用 typed dense handle。

运行时快照（Runtime Snapshot）是与共享静态数组分离的版本化制品。Accepted ADR 0025 要求
#302 使用版本化路网修订标识、LFCA origin digest/length、静态契约版本、world identity、
tick、输入命令游标和全部每世界可变状态完成绑定；内部 dense ordinal、地址、布局和
partition plan 不进入快照。只要 snapshot/runtime contract 与身份/执行约束版本兼容，
加载时可以从已提交道路编辑状态重新编译 LFCA、构建同一修订并借助
`SharedIdentityIndex` 完整重建引用。dense ordinal 不能跨路网修订直接复用。
任何保留旧状态的跨修订切换/恢复都必须消费经 #302 接受的可信切换输入绑定的语义
差异；`SharedIdentityIndex` 只复核 StableId128 ↔ typed ordinal 映射，不能证明
语义兼容，缺失该证据时迁移失败关闭。
回放使用显式输入命令流、checkpoint 与确定性状态摘要，调试构建可通过冷诊断和源映射
生成失同步诊断制品。交通运行时按观测导出节奏（Observation Export Cadence）导出
完整基线或版本化增量/分区选择的已提交交通观测；路径规划据此构造动态成本快照，
不进入交通参与单元 fixed-tick 热路径，也不要求每 tick 全量复制全网。

历史 `InitialTrafficData` 只表示 Core 时代可用于初始化 world 的已验证静态 JSON
输入。#301 后可运行世界由 `TrafficWorld::install(Arc<SharedNetworkRevision>)`
安装，不再从 current JSON / `InitialTrafficData` 构造。

Signals 分层职责：Controller 产生 indication；ManeuverGate/StopLine
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

Engine Adapter 负责把已提交交通状态映射到具体引擎：

- tick 调用
- actor / entity 生命周期
- transform 同步
- mesh / prefab / scene object 绑定
- debug draw
- UI 面板
- LOD 和性能策略

Adapter 不应把引擎依赖引入 Runtime。

Adapter 不再从 current JSON 安装可运行世界；它消费 `TrafficWorld` 与可选 `SpatialSession`。不得要求 Runtime 理解引擎路径、asset handle 或异步加载协议。

ADR 0025 target 中，Adapter/宿主 asset pipeline 负责认证发布资产或提供已提交道路编辑
状态；`laneflow-format` 与 `laneflow-static-network` 产生完整
`Arc<SharedNetworkRevision>`。Runtime 安装完整根；Spatial 只 `bind` 同一根 `Arc`，不依赖
Runtime，也不从 Runtime 发布的 snapshot/facade 借用。同时持有 world 与 session 的 Adapter
必须 `Arc::ptr_eq`。不存在独立 component 安装或重新组合 API。#302 若发布
revision-bound 只读 facade，必须内部保留同一根 `Arc`，不得把 Spatial 绑到 Runtime 类型。
Adapter 不读取 compiler IR、LFCA 表语义，也不拥有共享静态规则或单独替换 component。
它也不得把细节层次、可见性或帧预算转换为交通 fidelity：宿主可以暂停、慢放或
统一改变模拟时间推进速度，但不能静默丢 fixed tick、丢事件或让不同分区读取不同
逻辑时点。任何多频率或 aggregate 降级必须由独立 fidelity contract 和 G1 冻结。

ADR 0013/0015 与 #136 已冻结适配器边界。各 Adapter 不再自行定义中心线和长度采样权威；它们从 `TrafficWorld::committed_pose_sources()` 构造稳定的 Lane/Parking 输入，消费带 frame identity 和 placement token 的 `f32` canonical 批量位姿，并只在末端处理 frame 放置、坐标轴、坐标系手性、宿主变换、插值和细节层次（LOD）。详细设计见 ADR 0013、ADR 0015、`design/spatial-geometry.md` 与 `design/adapter-api.md`。

v0.7 的首个生产 Adapter crate 为 `laneflow-bevy`。#301 后它依赖 `laneflow-runtime`、`laneflow-spatial` 和 Bevy 0.19 的最小 modular crates，使用一个 Bevy Resource 表达单活动 Session，并在宿主 `First` 之后运行 LaneFlow 自有 outer-frame/fixed schedules；它不修改 Bevy `Time<Fixed>`，也不把 Bevy 类型引入 Runtime/Spatial。`debug-gizmos` 目前是占位 plugin。最小证据为 `runtime_min` 与无窗口 smoke。历史 v0.7 Gate 与 native reference 细节见 `design/bevy-reference-adapter.md` 和 `reference/v0.7-bevy-closure-review.md`。

v0.8 的场景人口与回流采用 ADR 0016 的 caller-owned policy，不进入 `TrafficWorld::step` 隐藏状态，也不由 Bevy ECS 选择 route。Runtime 不提供人口 controller。走廊人口迁到 Runtime 是 follow-up Issue，不是 #301 完成条件。Traffic/Spatial 继续是静态制品，不持久化目标人口、runtime handles 或 Entity。场景目标见 `design/example-scenarios.md`。

## 7. Presentation Layer

Presentation Layer 负责用户可见效果：

- 车辆模型
- 道路表现
- 动画
- 灯光
- 调试可视化
- 示例场景 UI

Presentation 可以因引擎不同而完全不同。
