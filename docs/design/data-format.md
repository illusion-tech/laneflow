# Data Format 设计

**文档状态**: Accepted  
**最后更新**: 2026-07-16  
**适用范围**: 当前 v0.4 外部数据格式，以及 planned v0.5 Parking 的 staged ownership 边界

**关联文档**:

- `../architecture.md`
- `../roadmap.md`
- `../adr/0005-core-identity-and-handle-model.md`
- `../adr/0007-traffic-data-crate-and-loader-boundary.md`
- `../adr/0008-pre-1.0-data-format-version-policy.md`
- `../adr/0010-parking-binding-and-vehicle-lifecycle-authority.md`
- `../adr/0009-signal-indication-gate-and-policy-separation.md`
- `../../schemas/laneflow-data-v0.4.schema.json`
- `data-loading.md`
- `lane-graph.md`
- `route-system.md`
- `vehicle-following.md`
- `signal-system.md`
- `parking-system.md`

## 1. 目标与非目标

本文定义 LaneFlow 当前唯一 active 的 v0.4 external package。它是 checked-in schema、production loader、canonical fixtures、validator 和后续 Adapter / authoring tool 的数据契约。

目标：

- 固化 lane graph、route、Vehicle Profile 与 static Signals 的字段、单位、引用和 closed shape。
- 统一 `id` / `xxxId` / `xxxIds` 引用命名。
- 维持单一 current version、严格版本闸口和 `laneflow-data -> laneflow-core` normalization。
- 让 Core constructors 成为跨记录 identity、reference、ownership、coverage、timing 和 route invariant 的唯一事实源。

非目标：

- 不持久化 initial vehicles、spawn schedule、runtime handles、phase snapshot 或 Adapter asset binding。
- 不实现 geometry、灯具 transform、controller runtime、SignalStop、permission traversal 或 jurisdiction rules。
- 不兼容加载 v0.3，不提供 runtime migration shim。
- 不接受 JSON-LD；未来如有需要，只能通过独立离线 importer 转换为 canonical JSON。
- 不承诺 v1.0 的长期稳定格式。

## 2. 当前 Package Model

```text
LaneFlowDataPackage
  formatVersion: "0.4"
  units: UnitSpec
  laneGraph: LaneGraphData
  routes: RouteData[]
  vehicleProfiles: VehicleProfileData[]
  signals: SignalsData
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
```

`signals` 与四个子数组均必填，可以全部为空。当前 canonical fixtures：

- `examples/data/v0.4-signals-baseline.laneflow.json`：完整 StopLine/Gates、group/none 与 green/yellow/red program。
- `examples/data/v0.4-empty-signals.laneflow.json`：显式四个空数组，承接无信号 route/profile 行为回归。

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

v0.4 因此使用 `connections[].toEdgeId` 与 `routes[].edgeIds`；旧 `to` / `edges` 由 schema 和 strict DTO 拒绝。

### 3.3 单位

`units.distance = "meter"`、`units.time = "second"`。edge/profile 的距离、速度、时间和加速度继续采用 SI 语义。`durationMs` / `offsetMs` 是 controller scheduling 的显式毫秒字段，不改变 `units.time` 对物理参数的含义。

## 4. Lane Graph、Route 与 Vehicle Profile

Lane graph 与 Vehicle Profile 的 domain 语义沿用 v0.3：

- edge length 必须 finite 且大于 `EDGE_BOUNDARY_EPSILON`。
- connection target 必须存在；同一 source 不得重复 target。
- route 至少一个 edge；引用必须存在，相邻 pair 必须连通；允许 repeated edge/self loop。
- Vehicle Profile 全字段必填、immutable，当前 `model` 仅为 `iidm`；数值和 deceleration cross-field 规则由 Core 校验。

v0.4 新增 route 规则：route 不得终止在声明 StopLine 的 edge 上。initial routes 与 runtime `register_route` 复用同一 Core helper，不能借 route completion 绕过 Gate。

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

## 6. Validation 分层与顺序

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
  -> routes + final-StopLine rule
  -> InitialTrafficData
```

| 层级 | 负责者 | 典型错误 |
| --- | --- | --- |
| syntax / shape | JSON parser、Serde、JSON Schema | required/type/closed shape、tagged union、enum、integer range |
| domain normalization | Core constructors，经 data loader 调用 | duplicate/unknown、ownership、coverage、complete state、cycle、route final StopLine |
| world compatibility | CoreWorld | positive fixed delta、phase duration >= delta、Signals vehicle activation guard |
| runtime | CoreWorld / lifecycle | stale handle、route in use、tick mismatch |

Schema 不重复 graph、ownership、coverage 或 complete-state 算法。Schema、private DTO、Core constructor 与本文冲突时必须在同一变更中统一。

## 7. Loader 与 Core 边界

```text
laneflow-data -> laneflow-core
laneflow-core -X-> laneflow-data
```

- `laneflow-data`：version header、private v0.4 DTO、JSON/units/path 和 external-to-Core 转换。
- `laneflow-core`：domain types、typed handles、registry/resolver、全局 invariant 与 world compatibility。
- loader 接收内存 bytes/string，不读取路径、不创建 `CoreWorld`、不公开 raw DTO。
- `LoadedPackage` 只表示 current v0.4，并持有已验证的 `InitialTrafficData`。
- normalization 预解析 edge/StopLine/Group/Controller/Phase/Gate handle/index；runtime hot path 不读取 JSON 或 external ID。

## 8. Capability Activation Guard

#96 完整交付 SignalStop、hard projection 与 permission-aware traversal 前，Core 明确拒绝：

```text
non-empty Signals + any spawned vehicle
```

`CoreWorld::with_traffic_data` 拒绝带初始车辆的 non-empty Signals，`spawn_vehicle` 在 signal-only world 中返回结构化错误且保持 world 不变。显式 empty Signals 继续支持 v0.3 已接受的车辆行为。

## 9. 历史与迁移

ADR 0008 要求 active tree 只维护一个 current format。#94 直接以 v0.4 替换 v0.3：

| 历史 v0.3 | 当前 v0.4 |
| --- | --- |
| `formatVersion: "0.3"` | `formatVersion: "0.4"` |
| `connections[].to` | `connections[].toEdgeId` |
| `routes[].edges` | `routes[].edgeIds` |
| 无 `signals` | 必填 Signals object 与四数组 |
| v0.3 schema/fixture | 从 active tree 移除，由 Git history 与 v0.3 closure review 保存 |
| production compatibility | 不提供；返回 `UnsupportedFormatVersion` |

若未来出现真实外部资产或支持窗口，再单独设计离线 migration tool；不得在 current loader 中静默累积历史分支。

## 10. Planned v0.5 Parking staged truth

`parking-system.md` 已冻结 planned v0.5 external package 输入，但本文件前述 current v0.4 schema、loader 和 fixtures 仍是 production 事实。只有 #107 在同一 Delivery PR 中原子完成以下项目后，本文件才能把 current 改写为 v0.5：

- `CURRENT_FORMAT_VERSION = "0.5"` 与唯一 active v0.5 schema；
- root 必填 closed `parking { areas, spaces }`；
- ParkingArea/ParkingSpace、optional omitted-only `areaId`、entry/exit anchors 与 edge-relative rectangle；
- private DTO、loader、Core normalization/`InitialTrafficData` rebind、两个 canonical fixtures 与 active examples；
- v0.4 明确拒绝，且不提供 production compatibility shim。

Planned wire、validation order、schema `$id`、migration 和 fixtures 的权威输入见 [`parking-system.md`](parking-system.md) 第 12 节。External package 继续只承载 static traffic data，不包含 initial parked vehicles、reservation、occupancy 或 runtime handles；runtime 与 hermetic tests 不联网解析 schema identifier。
