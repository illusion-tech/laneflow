# Data Format 设计

**文档状态**: Accepted（current）＋ Draft（#291 target 导航）<br>
**最后更新**: 2026-07-28（#281 current；#291/ADR 0020 target）<br>
**适用范围**: 当前 Traffic v0.10、SpatialPackage v0.1、ScenarioManifest v0.1、保留的 Data v0.6 数值研究输入，以及 compiler target 的格式边界

**关联文档**:

- `../architecture.md`
- `../roadmap.md`
- `../adr/0005-core-identity-and-handle-model.md`
- `../adr/0007-traffic-data-crate-and-loader-boundary.md`
- `../adr/0008-pre-1.0-data-format-version-policy.md`
- `../adr/0010-parking-binding-and-vehicle-lifecycle-authority.md`
- `../adr/0009-signal-indication-gate-and-policy-separation.md`
- `../adr/0011-schema-identifier-and-publication-contract.md`
- `../adr/0013-engine-neutral-spatial-geometry-and-length-authority.md`
- `../adr/0014-residual-aware-f32-core-authority-and-migration-gates.md`
- `../adr/0016-scenario-population-and-recycle-lifecycle-authority.md`
- `../adr/0017-static-road-junction-maneuver-and-gate-identity.md`
- `../adr/0018-multimodal-cross-section-and-access-overlay.md`
- `../adr/0020-compiler-owned-static-network-and-static-image.md`
- `../reference/glossary.md`
- `../../schemas/laneflow-data-v0.10.schema.json`
- `../../schemas/laneflow-spatial-v0.1.schema.json`
- `../../schemas/laneflow-scenario-manifest-v0.1.schema.json`
- `../../schemas/README.md`
- `data-loading.md`
- `spatial-geometry.md`
- `lane-graph.md`
- `route-system.md`
- `vehicle-following.md`
- `signal-system.md`
- `road-junction-model.md`
- `cross-section-access.md`
- `parking-system.md`
- `example-scenarios.md`

## 1. 目标与非目标

本文定义 LaneFlow 当前唯一 active 的 v0.10 external package。它是 checked-in schema、
production loader、canonical fixtures、validator 和 Adapter/authoring tool 的数据契约。

目标：

- 固化 lane graph、Junction/Movement/ManeuverPath、route、Vehicle Profile、static
  Signals 与 static Parking 的字段、单位、引用和 closed shape。
- 统一 `id` / `xxxId` / `xxxIds` 引用命名。
- 维持单一 current version、严格版本闸口和 `laneflow-data -> laneflow-core` normalization。
- 让 Core constructors 成为跨记录 identity、reference、ownership、coverage、timing 和 route invariant 的唯一事实源。

非目标：

- 不持久化 initial vehicles、spawn schedule、runtime handles、phase snapshot、Parking reservation/occupancy 或 Adapter asset binding。
- 不表达 world-space geometry、停车 maneuver、灯具 transform、jurisdiction rules 或 runtime command/event state。
- 不兼容加载 v0.7 及更早版本，不提供 runtime migration shim。
- 不接受 JSON-LD；未来如有需要，只能通过独立离线 importer 转换为 canonical JSON。
- 不承诺 v1.0 的长期稳定格式。

### 1.1 #291 compiler target（未实现）

ADR 0020 不把 current Traffic JSON 直接改名为 compiler IR。Target 把版本与职责拆为：

- authoring source：authoritative source module graph；Geometry 是主要 production
  language，Synthetic DSL/imported/editor-authored module 可以共同组成 compilation
  unit；
- portable canonical artifact：平台无关、可发布、可独立校验；
- target static image：按 target/layout/closed profile 生成、可重建；Traffic section
  必选，Spatial/cold/debug section 可选；
- source map / semantic diff：治理与诊断制品。

编译器从 validated canonical LIR 同时生成这些产物；target Runtime/Spatial 直接
消费 static image 的对齐视图。Target dynamic layer clean-break 命名为
`laneflow-runtime`/`TrafficWorld`，并通过 `laneflow-static-image` 的 external
descriptor + bounded verifier 挂载 view。`formatVersion: "0.10"`、本章 Package
Model 和 §7–§8 的
JSON→Core normalization 在阶段 8 生产切换 Issue #294 完成 cutover G4 前继续是
current contract，但不再约束 target IR 或 image layout。Target 版本轴、publication
与迁移规则见 ADR 0020 和 `network-compiler.md`。

## 2. 当前 Package Model

```text
LaneFlowDataPackage
  formatVersion: "0.10"
  units: UnitSpec
  laneGraph: LaneGraphData
  junctions: JunctionData[]
  movements: MovementData[]
  maneuverPaths: ManeuverPathData[]
  routes: RouteData[]
  vehicleProfiles: VehicleProfileData[]
  participantClasses: ParticipantClassData[]
  facilityBands: FacilityBandData[]
  roadSections: RoadSectionData[]
  laneGroups: LaneGroupData[]
  roadCorridors: RoadCorridorData[]
  accessRules: AccessRuleData[]
  waitingZones: WaitingZoneData[]
  signals: SignalsData
  parking: ParkingData
  extensions?: object

LaneEdgeData
  id
  length
  speedLimit  // m/s，required，finite 且 > 0
  connections[]
    toEdgeId

RouteData
  id
  edgeIds[]

JunctionData
  id

MovementData
  id
  junctionId

ManeuverPathData
  id
  movementId
  entryEdgeId
  internalEdgeIds[]
  exitEdgeId

ParticipantClassData
  id
  extendsId?  // 数据声明的单继承

FacilityBandData
  id
  kindId

RoadSectionData
  id
  kindId
  lanes[]
    edgeIds[]
    laneGroupId?

LaneGroupData
  id
  roadSectionId

RoadCorridorData
  id
  referenceSectionId
  elements[]: { sectionId } | { bandId }

AccessRuleData
  id
  target: { kind, id }
  effect: allow | deny
  participantClassIds[]

WaitingZoneData
  id
  maneuverPathId
  entryGateId
  releaseGateId
  maxOccupancy  // u32，> 0

SignalsData
  stopLines[]
    id
    edgeId
    location: edgeEnd
  maneuverGates[]
    id
    maneuverPathId
    transitionIndex
    stopLineId
    signalControl: { kind: group, groupId } | { kind: none }
  groups[]
    id
  controllers[]
    id
    kind: fixedTime
    offsetMs
    groupIds[]
    phases[]
      id
      durationMs
      states[]
        groupId
        aspect: red | yellow | green

ParkingData
  areas[]
    id
  spaces[]
    id
    areaId?  // omitted 表示 standalone；null 非法
    entry { edgeId, progress }
    exit { edgeId, progress }
    geometry { lateralOffset, headingOffsetRadians, length, width }
```

三类 topology arrays、六个横断面/准入 arrays、`waitingZones`、`signals`、
`parking` 及其全部子数组均必填，可以为空。
当前 canonical fixtures：

- `examples/data/v0.10-parking-signals-baseline.laneflow.json`：完整 topology/Signals、
  area-owned 与 standalone spaces。
- `examples/data/v0.10-empty-signals-and-parking.laneflow.json`：显式空 static arrays，
  承接 route/profile/repeated-edge 行为回归。
- `examples/data/v0.10-signalized-corridor.laneflow.json`：2 Junction、24 Movement、
  32 ManeuverPath/Gate 与横断面/准入 overlay 的 generator artifact。
- `examples/data/v0.10-multi-gate-waiting-zone.laneflow.json`：同一路径三个
  ordered Gate、两个共享边界 WaitingZone 与 route occurrence 编译基线。

v0.9 及更早 fixtures 作为历史 artifact 保留在 `examples/data/`，不再驱动
current contract tests。

## 3. 通用字段规则

### 3.1 External ID

- 非空 ASCII token，长度 1 到 128。
- pattern：`^[A-Za-z0-9][A-Za-z0-9._:/-]{0,127}$`。
- 大小写敏感，不 trim、case fold 或 Unicode normalize。
- 不同 domain 可以复用相同文本；各 domain 内按相应规则唯一。
- external package 不持久化 handle/index/generation。

### 3.2 引用命名

- definition identity 使用 `id`。
- 单引用使用 `xxxId`。
- 多引用使用 `xxxIds`。
- `xxxRef` 保留给未来结构化、URI/IRI 或跨文件引用。

current 格式继续使用 `connections[].toEdgeId` 与 `routes[].edgeIds`；旧 `to` / `edges` 由 schema 和 strict DTO 拒绝。

### 3.3 单位

`units.distance = "meter"`、`units.time = "second"`。edge/profile 的距离、速度、时间和加速度继续采用 SI 语义。`durationMs` / `offsetMs` 是 controller scheduling 的显式毫秒字段，不改变 `units.time` 对物理参数的含义。

## 4. Lane Graph、Route 与 Vehicle Profile

Lane graph 与 Vehicle Profile 的 domain 语义沿用 v0.3：

- edge length 必须 finite 且严格大于 current v0.10 继承的 `1.0e-9 m` exclusive minimum；Data 不导入 Core 私有数值策略。
- edge `speedLimit` 必填，单位为 m/s；schema 要求 JSON number 且 `exclusiveMinimum: 0`，Core `SpeedLimit` 最终裁决 finite 且严格大于 0。
- connection target 必须存在；同一 source 不得重复 target。
- route 至少一个 edge；引用必须存在，相邻 pair 必须连通；允许 repeated edge/self loop。
- Vehicle Profile 全字段必填、immutable，当前 `model` 仅为 `iidm`；`participantClassId` 必填并引用已声明的 ParticipantClass；数值和 deceleration cross-field 规则由 Core 校验。

route 不得终止在声明 StopLine 的 edge 上；不能从/在 Junction internal edge
开始/结束。initial routes 与 runtime `register_route` 复用同一 Maneuver occurrence
compiler，不能借 route completion 或不完整 path 绕过 Gate。

## 5. Static Signals Contract

### 5.1 StopLine 与 ManeuverGate

- StopLine 是独立 ID domain；v0.4 只支持 `location: "edgeEnd"`。
- 每个 edge 最多一个 StopLine。
- ManeuverGate 是一等 ID domain，引用 ManeuverPath 与 path-local transition index。
- v0.4–v0.9 的 canonical protected-entry profile 只声明
  `transitionIndex: 0`；这是历史/canonical profile shape，不是 current v0.10
  的全局限制。v0.10 允许同一 ManeuverPath 在不同 path-local transition 声明
  多个 Gate，每个 path-transition 至多一个 Gate；Gate StopLine 必须属于
  `pathEdges[transitionIndex]`。
- 被 entry Gate（`transitionIndex: 0`）引用的 StopLine，其 outgoing traversal
  必须有 ManeuverPath coverage，每条 entry ManeuverPath 在 transition 0 必须
  恰好有一个 Gate；仅被非 entry Gate 引用的 StopLine 不产生 entry coverage
  义务。
- `signalControl` 是 closed tagged union；`none` 只表示 signal layer 不施加约束，不表示永久自由通行。

### 5.2 Group、Controller 与 Phase

- 每个 Group 必须且只能归属一个 Controller，并至少被一个 Gate 使用。
- Controller `kind` 当前只允许 `fixedTime`，至少一个 group 和 phase。
- Phase ID 只在所属 Controller 内唯一，数组顺序定义循环 program。
- 每个 Phase 对 Controller 的全部 groups 恰好列出一次 state，不允许 sparse/default/inheritance。
- `durationMs` 是 `1..=2^53-1` 的整数；`offsetMs` 是 `0..=2^53-1` 的整数。
- cycle checked sum 不得超过 `2^53-1`；canonical offset 满足 `offsetMs < cycleDurationMs`，loader 不隐式 modulo。

完整 indication/Gate/policy 分层见 ADR 0009；controller runtime、snapshot 与 events 属于 #95，车辆合规属于 #96。

## 6. Static Parking Contract

- `ParkingArea.id` 与 `ParkingSpace.id` 分别 domain-local unique；area 只做 optional 逻辑分组，不保存 capacity 或 `spaceIds`。
- `areaId` 省略表示 standalone space；explicit `null` 非法。已声明 area 必须至少拥有一个 member space，reverse member order 使用 space input order。
- entry/exit anchor edge 必须存在；progress 必须 finite，并严格满足 `1.0e-9 m < progress < edgeLength - 1.0e-9 m`。该值是 current v0.10 继承的 anchor 数值事实。
- geometry 以 entry edge 的正向切线为局部基准；`abs(lateralOffset) > 1.0e-9 m`，heading 位于 `[-PI, PI)`，length/width 严格大于 current v0.10 继承的 `1.0e-9 m` exclusive minimum。lateral offset 与 extent 在测试中分别拥有语义，不从 Core 公共常量导入。
- External package 不持久化 reservation、occupancy、initial parked vehicles、runtime handles、maneuver path 或 world transform。

停车场、专用路边停车区和 standalone 路边泊位复用同一 `ParkingSpace` 模型；current v0.10 static data 不加入影响 Core 行为的 lot/curbside kind。完整 runtime/lifecycle 契约见 [`parking-system.md`](parking-system.md)，已由 #108/#109 交付并由 #110 完成端到端验证。

## 7. Validation 分层与顺序

Production fail-fast 顺序：

```text
JSON syntax
  -> minimal formatVersion shape
  -> exact current-version check
  -> strict current DTO shape
  -> units
  -> ParticipantClasses
  -> Vehicle Profiles
  -> lane graph
  -> Junctions / Movements / ManeuverPaths
  -> StopLines
  -> Groups
  -> Controllers / Phases / States
  -> ManeuverGates
  -> global coverage / ownership / usage
  -> Parking areas identity
  -> Parking spaces identity / optional membership
  -> entry / exit anchors
  -> Parking geometry
  -> orphan areas / ordered reverse indexes
  -> FacilityBands / RoadSections / LaneGroups / RoadCorridors
  -> AccessRules
  -> routes + final-StopLine rule
  -> InitialTrafficData
```

| 层级                 | 负责者                                 | 典型错误                                                                                                      |
| -------------------- | -------------------------------------- | ------------------------------------------------------------------------------------------------------------- |
| syntax / shape       | JSON parser、Serde、JSON Schema        | required/type/closed shape、tagged union、enum、integer range                                                 |
| domain normalization | Core constructors，经 data loader 调用 | duplicate/unknown、ownership、coverage、complete state、Parking anchors/geometry/orphan、route final StopLine |
| world compatibility  | CoreWorld                              | positive fixed delta、phase duration >= delta、Signals vehicle activation guard                               |
| runtime              | CoreWorld / lifecycle                  | stale handle、route in use、tick mismatch                                                                     |

Schema 不重复 graph、ownership、coverage 或 complete-state 算法。Schema、private DTO、Core constructor 与本文冲突时必须在同一变更中统一。

## 8. Loader 与 Core 边界

```text
laneflow-data -> laneflow-core
laneflow-core -X-> laneflow-data
```

- `laneflow-data`：version header、private v0.10 DTO、JSON/units/path 和 external-to-Core 转换。
- `laneflow-core`：domain types、typed handles、registry/resolver、全局 invariant 与 world compatibility。
- loader 接收内存 bytes/string，不读取路径、不创建 `CoreWorld`、不公开 raw DTO。
- `LoadedPackage` 只表示 current v0.10，并持有已验证的 `InitialTrafficData`。
- normalization 预解析 topology/edge/StopLine/Group/Controller/Phase/Gate/Parking
  handles、parent/member ranges 与 reverse indexes；runtime hot path 不读取 JSON
  或 external ID。

## 9. Signals Vehicle Capability

#96 已完整交付 SignalStop、hard projection 与 permission-aware traversal。current world 可同时包含 non-empty Signals 与 vehicles；legacy capability error 仅保留诊断兼容性，在合法 production world 不再返回。Static Parking registry 同样不激活 runtime 停车行为；commands、binding 与 ParkingStop 由 #108/#109 交付。

## 10. 历史与迁移

ADR 0008 要求 active tree 只维护一个 current format。#94 直接以 v0.4 替换 v0.3：

| 历史 v0.3                | 历史 v0.4                                                       |
| ------------------------ | --------------------------------------------------------------- |
| `formatVersion: "0.3"`   | `formatVersion: "0.4"`                                          |
| `connections[].to`       | `connections[].toEdgeId`                                        |
| `routes[].edges`         | `routes[].edgeIds`                                              |
| 无 `signals`             | 必填 Signals object 与四数组                                    |
| v0.3 schema/fixture      | 从 active tree 移除，由 Git history 与 v0.3 closure review 保存 |
| production compatibility | 不提供；返回 `UnsupportedFormatVersion`                         |

若未来出现真实外部资产或支持窗口，再单独设计离线 migration tool；不得在 current loader 中静默累积历史分支。

随后 #107 依据 ADR 0008 以 v0.5 原子替换 v0.4：

| 历史 v0.4                       | 当时 v0.5                                               |
| ------------------------------- | ------------------------------------------------------- |
| `formatVersion: "0.4"`          | `formatVersion: "0.5"`                                  |
| 无 `parking`                    | 必填 closed Parking object 与 areas/spaces arrays       |
| Signals-only canonical fixtures | Parking + Signals baseline 与显式双空 fixture           |
| v0.4 schema/fixtures            | 从 active tree 移除，由 Git 与 v0.4 closure review 保存 |
| production compatibility        | 不提供；v0.4 返回 `UnsupportedFormatVersion`            |

随后 #185 以 v0.7 原子替换 v0.5；v0.6 从未成为 production current：

| 历史 v0.5                       | 当时 v0.7                                             |
| ------------------------------- | ----------------------------------------------------- |
| `formatVersion: "0.5"`          | `formatVersion: "0.7"`                                |
| edge 无法规限速                 | required `laneGraph.edges[].speedLimit`，单位 m/s     |
| `LaneEdge` 只有 length/topology | mandatory `SpeedLimit` + current/downstream Core 约束 |
| v0.5 canonical fixtures         | v0.7 fixtures 与 Scenario traffic digest 原子切换     |
| production compatibility        | 不提供；v0.5/v0.6 返回 `UnsupportedFormatVersion`     |

Schema `$id` 按 ADR 0011 同时作为 absolute versioned identifier 与 public retrieval
URL；catalog 中 published version 必须通过 HTTPS 返回与固定 source revision
逐字节一致的 schema。Loader、Core、Adapter 与 hermetic tests 仍不联网解析
`$id`/`$schema`。v0.2-v0.5、v0.7-v0.9 作为 immutable publication artifacts
保留；current v0.10 已固定 `main` revision/blob 并加入 publication catalog，
canonical URL 已通过 live availability 与 byte-equality 验证。消费者入口见
[`schemas/README.md`](../../schemas/README.md)。

## 11. v0.6 空间层配套制品设计

#123 G1 不把中心线或世界几何加入 Traffic `LaneFlowDataPackage`。#134 交付独立的 SpatialPackage v0.1 与 ScenarioManifest v0.1 source contract，由清单通过不透明制品引用、原始 byte size 和 SHA-256 摘要与 current Traffic package 精确配对。

- 当前 v0.10 继续拥有交通边外部 ID、Core 边长、基础限速、拓扑、路线、信号、横断面/准入与停车边相对数据。
- SpatialPackage v0.1 是 closed JSON object：`formatVersion`、`frameId`、`edges[]`；每条 edge 使用 `trafficEdgeId` 和 `centerline.points`，点固定编码为 `[x, y, z]` 三元数组，不建立全局 vertex pool/index。
- 每条中心线至少两个点。wire number 先以 `f64` 暂存，执行有限性和每轴 `[-16_384, 16_384] m` 检查，再受检转换为唯一 runtime `f32` canonical 点；坐标为米、右手、`+Y` 向上。
- Spatial JSON edge 顺序不具权威性；成功规范化结果按 `LaneGraph::edges()` 稳定顺序排列，并要求对 Traffic graph 的 edge 完整、唯一覆盖。
- ScenarioManifest v0.1 的 `traffic` / `spatial` descriptor 固定包含 `artifactRef`、角色专属 `mediaType`、`sha256:<64 lowercase hex>` 与 raw byte `size`；两个 ref 必须不同，调用方提供的 ref 集合也不得重复。
- digest 对调用方提供的原始 bytes 计算，不 trim、不重新序列化。size 必须是 `0..=2^53-1`，并先于 digest mismatch 报错。
- 场景清单与空间模式使用独立版本系列；pre-1.0 loader 只接受各自精确 current version，不提供历史分派或兼容 shim。
- 只使用 Core 的消费者无需空间制品；需要位姿的适配器或工具必须提供完整且通过绑定的空间包。
- #134 只交付 schema、样例、制品身份和到受检点/edge handle 的原子规范化；退化段、弧长、Traffic length binding、连接端点连续性、基底、采样与 `SpatialRegistry` 提交由 #135 负责。

## 12. Data v0.6 数值格式原子迁移边界

ADR 0014 接受了目标 Core/Data 数值契约；#126 曾把该研究候选分配给
`formatVersion: "0.6"`。#144 的首次生产迁移因性能门槛失败而形成不迁移
（no-go）结论，v0.6 从未成为 current/published production format。当前 v0.10
继续使用既有 `f64` 数值域；以下内容仍是未来数值迁移输入：

- 当前 v0.10 的线格式 DTO、模式范围、加载器诊断和 `f64` Core 规范化在未来数值迁移前保持当前实现行为；不增加逐字节、旧范围或旧诊断兼容证明；
- 下一目标格式把单 edge `<=10_000 m`、速度 `<=100 m/s`、Profile 加速度/减速度 `<=50 m/s²`、期望车头时距 `<=60 s`、尺寸/最小间距/偏移 `<=128 m` 等硬范围写入模式与 Core 构造器；最小 edge 长度目标值由 #127 离线标定，但 #144 回退后没有进入当前格式；
- JSON 词法类型继续是 `number`。Data 可以先以 `f64` 或等价高保真值解析，以便报告原始越界输入；随后必须通过显式受检转换进入单一 `f32` 数值域或补偿残差感知的 `EdgeProgress`；
- Parking 入口/出口锚点的线格式继续是单个 `progress` JSON 数值，但 Core 规范化结果直接使用 `EdgeProgress`；不保留裸 `f64` 静态位置或新增第三种边内位置类型；
- 原始 `f64` 转换错误可保留输入值；规范化单值域错误使用 `f32`，有效进度与实际采用 `f64` 的路线（route）派生值使用 `f64`。错误显示（Display）使用领域化中文范围，不引用已删除的数值常量名；
- 模式（schema）文件名/`$id`、发布目录的当前指针、私有线格式 DTO、加载器版本闸口与路径诊断、标准固定样例、Core 构造器、测试和当前文档必须由未来原子迁移在同一交付 PR（Delivery PR）中更新；
- 有效代码树仍只维护一个当前加载器。未来切换后不叠加 v0.10 运行时兼容分支，不自动拆 edge、不静默截断；仓库内资产随迁移直接更新，不实现离线迁移工具；
- 规范化和批量命令继续执行“先计算、后提交”，任一范围、转换或引用错误不得留下部分 `InitialTrafficData` 或 world 状态。

#127 拥有九个目标 `f32` 固定绝对阈值、`EdgeProgress` 运算链和路线距离（route-distance）布局证据；#144 已消费这些结果实施生产候选，但因性能 no-go 而完整回退。未来重启不得重新发明阈值；已公开且受 ADR 0011 约束的历史模式（schema）可以作为不可变静态制品保留，但不进入有效加载器、固定样例或规范化测试。

因此，#126 的文档只保留迁移输入；#144 未通过性能闸口，当前版本不会切换到 v0.6。未来只有新的原子 Data/Core 数值迁移通过正确性、内存护栏、性能与 G0-G4 后才可切换。

## 13. v0.8 Traffic v0.7 道路限速契约

#184 冻结并由 #185 实现 v0.8 Traffic `formatVersion: "0.7"`。v0.6 保留给 #144 曾经 no-go 的 f32 原子数值迁移，未被复用来表达 per-edge speed limit 的另一种不兼容 shape。

v0.7 直接以 v0.5 shape 和 `f64` Core 数值域为迁移基线，不激活或夹带 v0.6 的 target-f32 变更；它在每个 lane edge 增加 required、严格正且有限的 `speedLimit`，wire/Core 单位为 m/s。主干道及其直行 connector 为 60 km/h，次干道及其 connector 为 40 km/h。`VehicleProfile.desiredSpeed` 继续表达车辆自由流期望速度，不能替代 edge speed limit。production loader 只接受 exact `"0.7"`，v0.5/v0.6 不进入 current DTO；schema、fixtures 与 Scenario traffic digest 已原子切换。

目标人口、seed、portal catalog、initial spawn slots、pending recycle、VehicleHandle 与 Entity 不进入 Traffic/Spatial/Manifest。它们属于 authoring/startup config 或 caller-owned runtime plan；native example 仍必须让生成制品通过 production loader。详细场景和 lifecycle authority 见 `example-scenarios.md` 与 ADR 0016。

## 14. v0.9 产品里程碑采用的历史 Traffic v0.8 static-domain contract

#229 已按 #228/ADR 0017 实现以下 clean-break contract：

当时的 `formatVersion: "0.8"` 原子增加：

- top-level `junctions[]`，元素至少包含 `id`；
- top-level `movements[]`，child-owned `junctionId`；
- top-level `maneuverPaths[]`，child-owned `movementId`、`entryEdgeId`、
  ordered `internalEdgeIds[]` 与 `exitEdgeId`；
- `signals.maneuverGates[]`，包含 `id`、`maneuverPathId`、`transitionIndex`、
  `stopLineId` 与 `signalControl`；
- 删除 `signals.movementGates[]` 和 pair-based Gate identity。

Wire 不保存 parent child arrays、derived internal-edge owner、runtime handles、
Route occurrences 或 candidate indices。Core normalization 负责 owner、cardinality、
connectivity、Gate/StopLine、Route coverage、first-error 与 foreign-graph rebind。

当时的迁移规则：

- loader 只接受 exact `0.8`，不并行接受 `0.7`；
- 不提供 deprecated fields/types、dual schema 或 runtime upgrade shim；
- schema/private DTO/loader/Core/fixtures/generator/artifacts/catalog/publication/docs/
  digests 同一 production 交付切换；
- SpatialPackage/ScenarioManifest 保持 `0.1`；
- Traffic bytes 改变后 Manifest traffic size/digest 必须更新；
- 已发布 v0.7 schema/bytes 按 ADR 0011 immutable，不原地覆写。

完整字段语义、原子迁移和影响矩阵见
[`road-junction-model.md`](road-junction-model.md)。
current Traffic v0.10 完整继承这些 Junction/Movement/ManeuverPath/ManeuverGate
shape 与 route-occurrence 语义，并在下节增加横断面与准入静态模型；本节不再声称
v0.8 是 active loader contract。

## 15. Traffic 0.9 横断面与准入静态模型契约

#262 已按 #234/ADR 0018 实现以下 clean-break contract：

Current `formatVersion: "0.9"` 原子增加：

- top-level `participantClasses[]`：数据声明、单继承（`extendsId`）的参与者分类；
- top-level `facilityBands[]`：非遍历设施带（`kindId`）；
- top-level `roadSections[]`：方向性分段，ordered `lanes[].edgeIds[]` 与可选
  `laneGroupId`；
- top-level `laneGroups[]`：child-owned `roadSectionId`；
- top-level `roadCorridors[]`：横断面唯一 owner，`referenceSectionId` 与 ordered
  `elements[]`（section/band 二选一引用）；
- top-level `accessRules[]`：`target`/`effect`/`participantClassIds` 准入 overlay；
- `vehicleProfiles[].participantClassId` 必填。

`FacilityBandData` 有意不重复持久化父实体（Parent）字段：每个设施带必须被恰好
一个 `RoadCorridorData.elements[].bandId` 引用，核心规范化（Core Normalization）
从该正向成员关系建立成员到所有者的反向索引（Reverse Index），并对多所有者
（Multiple Owner）或零所有者（Unowned）失败关闭。该当前态（Current）单一事实源
也是 #291 目标态（Target）`FacilityBand` 父实体稳定标识（Parent StableId）的输入；
编译器（Compiler）必须在派生设施带标识前验证同一唯一所有者关系，不得由数组顺序
选择父实体。完整所有者 / 成员（Owner / Member）语义见
[`cross-section-access.md`](cross-section-access.md)。

迁移规则：

- loader 只接受 exact `0.9`，不并行接受 `0.8`；
- 不提供 deprecated fields/types、dual schema 或 runtime upgrade shim；
- schema/private DTO/loader/Core/fixtures/generator/artifacts/docs 同一 production
  交付切换；
- SpatialPackage/ScenarioManifest 保持 `0.1`；
- v0.9 schema 已发布（固定 `main` provenance + live availability 与 byte-equality
  验证）；已发布 v0.8 schema/bytes 按
  ADR 0011 immutable，不原地覆写；
- v0.8 fixtures 作为历史 artifact 保留，不进入 current contract tests。

静态规则在 (ParticipantClass, Route) 绑定期 fail-fast；时变规则与 FacilityBand
target 规则在 v1 由 capability guard 结构化拒绝。完整字段语义、横断面/准入分层、
组合裁决与绑定期校验见 [`cross-section-access.md`](cross-section-access.md)。

## 16. Traffic 0.10 multi-Gate 与 WaitingZone static contract

#281 按 #235/ADR 0019 从已发布且 immutable 的 Traffic 0.9 原子迁移当前
source contract：

- 同一 `ManeuverPath` 可以在不同 `transitionIndex` 声明多个 Gate；每个
  path-transition 仍至多一个 Gate，StopLine 必须属于该 transition 的 from edge；
- top-level required `waitingZones[]` 声明 `id`、`maneuverPathId`、
  `entryGateId`、`releaseGateId` 与 positive `maxOccupancy`；
- entry/release Gate 必须属于同一声明 path，entry transition 严格早于 release；
  同一路径 WaitingZone interior 不得重叠或嵌套，共享 boundary 合法；
- Route 注册期编译 ordered Gate/Waiting occurrences、next Gate/exit boundary 与
  empty-zone storage length；steady tick 不扫描 external IDs；
- `(VehicleProfile, Route, cursor)` 绑定在 Access 之后校验 pending zone 的整车长度
  可行性。stateful occurrence interior bootstrap 和 Waiting runtime 尚未实现，
  分别由显式 capability error 拒绝，禁止车辆静默穿越；
- loader 只接受 exact `0.10`，不并行接受 `0.9`，也不提供 alias 或 migration
  shim；SpatialPackage/ScenarioManifest 保持 `0.1`；
- v0.10 schema 已按固定 `main` revision/blob 公开发布，canonical URL 已通过 live
  availability 与 byte-equality 验证；已发布 v0.9 不修改。

队列、admission、capacity runtime state、constraint/events 属 #282；Conflict 与
Spatial 属 #283，不在本 static contract 中提前激活。
