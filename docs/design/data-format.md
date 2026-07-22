# Data Format 设计

**文档状态**: Accepted  
**最后更新**: 2026-07-22
**适用范围**: 当前 Traffic v0.5、SpatialPackage v0.1、ScenarioManifest v0.1、保留的 Data v0.6 数值迁移与 v0.8 Traffic v0.7 目标

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
- `../../schemas/laneflow-data-v0.5.schema.json`
- `../../schemas/laneflow-spatial-v0.1.schema.json`
- `../../schemas/laneflow-scenario-manifest-v0.1.schema.json`
- `../../schemas/README.md`
- `data-loading.md`
- `spatial-geometry.md`
- `lane-graph.md`
- `route-system.md`
- `vehicle-following.md`
- `signal-system.md`
- `parking-system.md`
- `example-scenarios.md`

## 1. 目标与非目标

本文定义 LaneFlow 当前唯一 active 的 v0.5 external package。它是 checked-in schema、production loader、canonical fixtures、validator 和后续 Adapter / authoring tool 的数据契约。

目标：

- 固化 lane graph、route、Vehicle Profile、static Signals 与 static Parking 的字段、单位、引用和 closed shape。
- 统一 `id` / `xxxId` / `xxxIds` 引用命名。
- 维持单一 current version、严格版本闸口和 `laneflow-data -> laneflow-core` normalization。
- 让 Core constructors 成为跨记录 identity、reference、ownership、coverage、timing 和 route invariant 的唯一事实源。

非目标：

- 不持久化 initial vehicles、spawn schedule、runtime handles、phase snapshot、Parking reservation/occupancy 或 Adapter asset binding。
- 不表达 world-space geometry、停车 maneuver、灯具 transform、jurisdiction rules 或 runtime command/event state。
- 不兼容加载 v0.4 及更早版本，不提供 runtime migration shim。
- 不接受 JSON-LD；未来如有需要，只能通过独立离线 importer 转换为 canonical JSON。
- 不承诺 v1.0 的长期稳定格式。

## 2. 当前 Package Model

```text
LaneFlowDataPackage
  formatVersion: "0.5"
  units: UnitSpec
  laneGraph: LaneGraphData
  routes: RouteData[]
  vehicleProfiles: VehicleProfileData[]
  signals: SignalsData
  parking: ParkingData
  extensions?: object

LaneEdgeData
  id
  length
  connections[]
    toEdgeId

RouteData
  id
  edgeIds[]

SignalsData
  stopLines[]
    id
    edgeId
    location: edgeEnd
  movementGates[]
    fromEdgeId
    toEdgeId
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

`signals`、`parking` 及其全部子数组均必填，可以为空。当前 canonical fixtures：

- `examples/data/v0.5-parking-signals-baseline.laneflow.json`：完整 Signals、area-owned 与 standalone spaces、same/distinct anchors、正负 lateral 和 zero/angled heading。
- `examples/data/v0.5-empty-signals-and-parking.laneflow.json`：显式空 Signals/Parking 数组，承接 route/profile/repeated-edge 行为回归。

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

- edge length 必须 finite 且严格大于 current v0.5 自有的 `1.0e-9 m` exclusive minimum；Data 不导入 Core 私有数值策略。
- connection target 必须存在；同一 source 不得重复 target。
- route 至少一个 edge；引用必须存在，相邻 pair 必须连通；允许 repeated edge/self loop。
- Vehicle Profile 全字段必填、immutable，当前 `model` 仅为 `iidm`；数值和 deceleration cross-field 规则由 Core 校验。

v0.4 引入且 v0.5 保留的 route 规则：route 不得终止在声明 StopLine 的 edge 上。initial routes 与 runtime `register_route` 复用同一 Core helper，不能借 route completion 绕过 Gate。

## 5. Static Signals Contract

### 5.1 StopLine 与 MovementGate

- StopLine 是独立 ID domain；v0.4 只支持 `location: "edgeEnd"`。
- 每个 edge 最多一个 StopLine。
- MovementGate identity 是 `(fromEdgeId, toEdgeId)`，且该 pair 必须是合法 connection。
- Gate 引用的 StopLine 必须属于 `fromEdgeId`。
- 声明 StopLine 的 edge，其每个 outgoing connection 必须恰好有一个 Gate。
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
- entry/exit anchor edge 必须存在；progress 必须 finite，并严格满足 `1.0e-9 m < progress < edgeLength - 1.0e-9 m`。该值是 current v0.5 的 anchor 数值事实。
- geometry 以 entry edge 的正向切线为局部基准；`abs(lateralOffset) > 1.0e-9 m`，heading 位于 `[-PI, PI)`，length/width 严格大于 current v0.5 自有的 `1.0e-9 m` exclusive minimum。lateral offset 与 extent 在测试中分别拥有语义，不从 Core 公共常量导入。
- External package 不持久化 reservation、occupancy、initial parked vehicles、runtime handles、maneuver path 或 world transform。

停车场、专用路边停车区和 standalone 路边泊位复用同一 `ParkingSpace` 模型；v0.5 static data 不加入影响 Core 行为的 lot/curbside kind。完整 runtime/lifecycle 契约见 [`parking-system.md`](parking-system.md)，已由 #108/#109 交付并由 #110 完成端到端验证。

## 7. Validation 分层与顺序

Production fail-fast 顺序：

```text
JSON syntax
  -> minimal formatVersion shape
  -> exact current-version check
  -> strict current DTO shape
  -> units
  -> Vehicle Profiles
  -> lane graph
  -> StopLines
  -> Groups
  -> Controllers / Phases / States
  -> MovementGates
  -> global coverage / ownership / usage
  -> Parking areas identity
  -> Parking spaces identity / optional membership
  -> entry / exit anchors
  -> Parking geometry
  -> orphan areas / ordered reverse indexes
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

- `laneflow-data`：version header、private v0.5 DTO、JSON/units/path 和 external-to-Core 转换。
- `laneflow-core`：domain types、typed handles、registry/resolver、全局 invariant 与 world compatibility。
- loader 接收内存 bytes/string，不读取路径、不创建 `CoreWorld`、不公开 raw DTO。
- `LoadedPackage` 只表示 current v0.5，并持有已验证的 `InitialTrafficData`。
- normalization 预解析 edge/StopLine/Group/Controller/Phase/Gate/Parking handles 与 reverse indexes；runtime hot path 不读取 JSON 或 external ID。

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

| 历史 v0.4                       | 当前 v0.5                                               |
| ------------------------------- | ------------------------------------------------------- |
| `formatVersion: "0.4"`          | `formatVersion: "0.5"`                                  |
| 无 `parking`                    | 必填 closed Parking object 与 areas/spaces arrays       |
| Signals-only canonical fixtures | Parking + Signals baseline 与显式双空 fixture           |
| v0.4 schema/fixtures            | 从 active tree 移除，由 Git 与 v0.4 closure review 保存 |
| production compatibility        | 不提供；v0.4 返回 `UnsupportedFormatVersion`            |

Schema `$id` 按 ADR 0011 同时作为 absolute versioned identifier 与 public retrieval URL；catalog 中全部版本必须通过 HTTPS 返回与固定 source revision 逐字节一致的 schema。Loader、Core、Adapter 与 hermetic tests 仍不联网解析 `$id`/`$schema`。v0.2-v0.4 只作为 immutable publication artifacts 保留，不改变当前唯一 active v0.5 contract；消费者入口见 [`schemas/README.md`](../../schemas/README.md)。

## 11. v0.6 空间层配套制品设计

#123 G1 不把中心线或世界几何加入当前 v0.5 `LaneFlowDataPackage`，也不提升其 `formatVersion`。#134 交付独立的 SpatialPackage v0.1 与 ScenarioManifest v0.1 source contract，由清单通过不透明制品引用、原始 byte size 和 SHA-256 摘要与 Traffic package 精确配对。

- 当前 v0.5 继续拥有交通边外部 ID、Core 边长、拓扑、路线、信号与停车边相对数据。
- SpatialPackage v0.1 是 closed JSON object：`formatVersion`、`frameId`、`edges[]`；每条 edge 使用 `trafficEdgeId` 和 `centerline.points`，点固定编码为 `[x, y, z]` 三元数组，不建立全局 vertex pool/index。
- 每条中心线至少两个点。wire number 先以 `f64` 暂存，执行有限性和每轴 `[-16_384, 16_384] m` 检查，再受检转换为唯一 runtime `f32` canonical 点；坐标为米、右手、`+Y` 向上。
- Spatial JSON edge 顺序不具权威性；成功规范化结果按 `LaneGraph::edges()` 稳定顺序排列，并要求对 Traffic graph 的 edge 完整、唯一覆盖。
- ScenarioManifest v0.1 的 `traffic` / `spatial` descriptor 固定包含 `artifactRef`、角色专属 `mediaType`、`sha256:<64 lowercase hex>` 与 raw byte `size`；两个 ref 必须不同，调用方提供的 ref 集合也不得重复。
- digest 对调用方提供的原始 bytes 计算，不 trim、不重新序列化。size 必须是 `0..=2^53-1`，并先于 digest mismatch 报错。
- 场景清单与空间模式使用独立版本系列；pre-1.0 loader 只接受各自精确 current version，不提供历史分派或兼容 shim。
- 只使用 Core 的消费者无需空间制品；需要位姿的适配器或工具必须提供完整且通过绑定的空间包。
- #134 只交付 schema、样例、制品身份和到受检点/edge handle 的原子规范化；退化段、弧长、Traffic length binding、连接端点连续性、基底、采样与 `SpatialRegistry` 提交由 #135 负责。

## 12. Data v0.6 数值格式原子迁移边界

ADR 0014 接受了下一 Core/Data 数值契约；#126 进一步冻结目标交通数据（Traffic Data）版本为 `formatVersion: "0.6"`。#144 的首次生产迁移因性能门槛失败而形成不迁移（no-go）结论，因此当前唯一有效格式继续是 v0.5；v0.6 仍只是未来原子迁移输入：

- 当前 v0.5 的线格式 DTO、模式范围、加载器诊断和 `f64` Core 规范化在原子迁移前保持当前实现行为；不增加逐字节、旧范围或旧诊断兼容证明；
- 下一目标格式把单 edge `<=10_000 m`、速度 `<=100 m/s`、Profile 加速度/减速度 `<=50 m/s²`、期望车头时距 `<=60 s`、尺寸/最小间距/偏移 `<=128 m` 等硬范围写入模式与 Core 构造器；最小 edge 长度目标值由 #127 离线标定，但 #144 回退后没有进入当前格式；
- JSON 词法类型继续是 `number`。Data 可以先以 `f64` 或等价高保真值解析，以便报告原始越界输入；随后必须通过显式受检转换进入单一 `f32` 数值域或补偿残差感知的 `EdgeProgress`；
- Parking 入口/出口锚点的线格式继续是单个 `progress` JSON 数值，但 Core 规范化结果直接使用 `EdgeProgress`；不保留裸 `f64` 静态位置或新增第三种边内位置类型；
- 原始 `f64` 转换错误可保留输入值；规范化单值域错误使用 `f32`，有效进度与实际采用 `f64` 的路线（route）派生值使用 `f64`。错误显示（Display）使用领域化中文范围，不引用已删除的数值常量名；
- 模式（schema）文件名/`$id`、发布目录的当前指针、私有线格式 DTO、加载器版本闸口与路径诊断、标准固定样例、Core 构造器、测试和当前文档必须由未来原子迁移在同一交付 PR（Delivery PR）中更新；
- 有效代码树仍只维护一个当前加载器。未来切换后不叠加 v0.5 运行时兼容分支，不自动拆 edge、不静默截断；仓库内资产随迁移直接更新，不实现离线迁移工具；
- 规范化和批量命令继续执行“先计算、后提交”，任一范围、转换或引用错误不得留下部分 `InitialTrafficData` 或 world 状态。

#127 拥有九个目标 `f32` 固定绝对阈值、`EdgeProgress` 运算链和路线距离（route-distance）布局证据；#144 已消费这些结果实施生产候选，但因性能 no-go 而完整回退。未来重启不得重新发明阈值；已公开且受 ADR 0011 约束的历史模式（schema）可以作为不可变静态制品保留，但不进入有效加载器、固定样例或规范化测试。

因此，#126 的文档合入只建立直接迁移输入；#144 未通过性能闸口，本文件的“当前版本”标题和模式链接不切换到 v0.6。未来只有新的原子 Data/Core 迁移通过正确性、内存护栏、性能与 G0-G4 后才可切换。

## 13. v0.8 Traffic v0.7 target

#184 冻结 v0.8 的目标 Traffic 版本为 `formatVersion: "0.7"`。v0.6 已被 Accepted Data design 保留给 #144 曾经 no-go 的 f32 原子数值迁移，不能复用同一版本号表达 per-edge speed limit 的另一种不兼容 shape。

v0.7 target 直接以 current v0.5 shape 和 `f64` Core 数值域为迁移基线，不激活或夹带 v0.6 的 target-f32 变更；它在每个 lane edge 增加 required、严格正且有限的 `speedLimit`，wire/Core 单位为 m/s。主干道及其直行 connector 为 60 km/h，次干道及其 connector 为 40 km/h。`VehicleProfile.desiredSpeed` 继续表达车辆自由流期望速度，不能替代 edge speed limit。具体 schema shape、版本发布与 loader 原子切换由 #186 交付；本设计 PR 不修改 active schema 或 production loader，当前唯一 active Traffic 仍为 v0.5。

目标人口、seed、portal catalog、initial spawn slots、pending recycle、VehicleHandle 与 Entity 不进入 Traffic/Spatial/Manifest。它们属于 authoring/startup config 或 caller-owned runtime plan；native example 仍必须让生成制品通过 production loader。详细场景和 lifecycle authority 见 `example-scenarios.md` 与 ADR 0016。
