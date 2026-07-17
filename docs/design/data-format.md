# Data Format 设计

**文档状态**: Accepted  
**最后更新**: 2026-07-17
**适用范围**: 当前 v0.5 外部数据格式与 static Parking ownership 边界

**关联文档**:

- `../architecture.md`
- `../roadmap.md`
- `../adr/0005-core-identity-and-handle-model.md`
- `../adr/0007-traffic-data-crate-and-loader-boundary.md`
- `../adr/0008-pre-1.0-data-format-version-policy.md`
- `../adr/0010-parking-binding-and-vehicle-lifecycle-authority.md`
- `../adr/0009-signal-indication-gate-and-policy-separation.md`
- `../../schemas/laneflow-data-v0.5.schema.json`
- `data-loading.md`
- `lane-graph.md`
- `route-system.md`
- `vehicle-following.md`
- `signal-system.md`
- `parking-system.md`

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

- edge length 必须 finite 且大于 `EDGE_BOUNDARY_EPSILON`。
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
- entry/exit anchor edge 必须存在；progress 必须 finite，并严格满足 `EDGE_BOUNDARY_EPSILON < progress < edgeLength - EDGE_BOUNDARY_EPSILON`。
- geometry 以 entry edge 的正向切线为局部基准；`abs(lateralOffset) > GEOMETRY_GAP_EPSILON`，heading 位于 `[-PI, PI)`，length/width 大于 geometry epsilon。
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

| 层级 | 负责者 | 典型错误 |
| --- | --- | --- |
| syntax / shape | JSON parser、Serde、JSON Schema | required/type/closed shape、tagged union、enum、integer range |
| domain normalization | Core constructors，经 data loader 调用 | duplicate/unknown、ownership、coverage、complete state、Parking anchors/geometry/orphan、route final StopLine |
| world compatibility | CoreWorld | positive fixed delta、phase duration >= delta、Signals vehicle activation guard |
| runtime | CoreWorld / lifecycle | stale handle、route in use、tick mismatch |

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

| 历史 v0.3 | 历史 v0.4 |
| --- | --- |
| `formatVersion: "0.3"` | `formatVersion: "0.4"` |
| `connections[].to` | `connections[].toEdgeId` |
| `routes[].edges` | `routes[].edgeIds` |
| 无 `signals` | 必填 Signals object 与四数组 |
| v0.3 schema/fixture | 从 active tree 移除，由 Git history 与 v0.3 closure review 保存 |
| production compatibility | 不提供；返回 `UnsupportedFormatVersion` |

若未来出现真实外部资产或支持窗口，再单独设计离线 migration tool；不得在 current loader 中静默累积历史分支。

随后 #107 依据 ADR 0008 以 v0.5 原子替换 v0.4：

| 历史 v0.4 | 当前 v0.5 |
| --- | --- |
| `formatVersion: "0.4"` | `formatVersion: "0.5"` |
| 无 `parking` | 必填 closed Parking object 与 areas/spaces arrays |
| Signals-only canonical fixtures | Parking + Signals baseline 与显式双空 fixture |
| v0.4 schema/fixtures | 从 active tree 移除，由 Git 与 v0.4 closure review 保存 |
| production compatibility | 不提供；v0.4 返回 `UnsupportedFormatVersion` |

Schema `$id` 是 absolute versioned identifier，不是 runtime 网络承诺。Loader、Core、Adapter 与 hermetic tests 不联网解析 `$id`/`$schema`；是否发布该 URL 由独立 #103 决定。
