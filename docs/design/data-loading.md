# Data Loading 设计

**文档状态**: Accepted  
**最后更新**: 2026-07-22
**适用范围**: Traffic v0.7、SpatialPackage/ScenarioManifest v0.1 loader 与保留的 Data v0.6 数值迁移边界

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
- `data-format.md`
- `spatial-geometry.md`
- `vehicle-following.md`
- `signal-system.md`
- `parking-system.md`

## 1. 目标与非目标

目标：

- 为当前 `formatVersion: "0.7"` 提供唯一 production JSON loader。
- 在 strict current DTO 前拒绝旧版、未来版和缺失版本。
- 保持 `laneflow-data -> laneflow-core`，让 Core constructors 拥有全部 domain invariant。
- 原子 normalization lane graph、route、Vehicle Profile、static Signals 与 static Parking，并保留稳定输入路径。
- 让 external ID resolution 和 handle allocation 停留在加载/初始化阶段。

非目标：

- 不兼容加载 v0.5、v0.6 及更早版本，不自动迁移旧字段或补默认限速。
- 不读取文件、不创建 CoreWorld、不接收 initial vehicles 或 fixed tick。
- 不公开 wire DTO/serialization API，不一次收集全部 authoring errors。
- 不实现 controller runtime、Parking reservation/occupancy/commands、车辆合规或 hostile oversized JSON 完整防护。

## 2. Crate 与 API 边界

```text
crates/laneflow-data
  private current wire DTO
  version / units / JSON path
  current schema contract tests
  Traffic / Spatial artifact identity
  Core / Spatial normalization orchestration

crates/laneflow-core
  LaneGraph / Route / VehicleProfile
  StopLine / MovementGate / SignalGroup / SignalController / Phase
  ParkingArea / ParkingSpace / ParkingRegistry
  typed handles / immutable registries / resolvers
  InitialTrafficData / CoreWorld compatibility

crates/laneflow-spatial
  canonical frame ID
  bounded canonical f32 point
```

依赖方向固定：

```text
laneflow-data -> laneflow-core
laneflow-data -> laneflow-spatial
```

Public loader：

```text
CURRENT_FORMAT_VERSION: &str = "0.7"

from_json_slice(&[u8]) -> Result<LoadedPackage, DataError>
from_json_str(&str) -> Result<LoadedPackage, DataError>

LoadedPackage
  initial_traffic_data: InitialTrafficData

from_scenario_json_slice(manifest, named_artifacts) -> Result<LoadedScenario, ScenarioError>
from_scenario_json_str(manifest, named_artifacts) -> Result<LoadedScenario, ScenarioError>
```

`LoadedPackage` 字段私有，只提供只读 accessor 与 `into_initial_traffic_data`。不公开历史 version enum、raw DTO、file/Read/async overload。

## 3. 版本闸口与 Current Shape

```text
minimal version header
  -> exact "0.7" check
  -> strict current DTO
  -> normalization
```

- 缺失、`null` 或非字符串 `formatVersion` 返回 JSON shape error。
- `"0.4"`、未来版或其他字符串返回 `UnsupportedFormatVersion { expected, actual }`。
- Unsupported version 不进入 current units、unknown field 或 domain validation。
- current root 必填 `units`、`laneGraph`、`routes`、`vehicleProfiles`、`signals` 和 `parking`。
- `signals.stopLines/movementGates/groups/controllers` 四数组均必填，可以为空。
- `parking.areas/spaces` 两数组均必填，可以为空；space `areaId` 只能省略，explicit `null` 非法。
- private DTO 全面 `deny_unknown_fields`；`signalControl` 使用 closed group/none union。

## 4. Core Normalization

Core public static input：

```text
InitialTrafficData
  lane_graph
  routes
  vehicle_profiles
  signals: SignalRegistry
  parking: ParkingRegistry
```

Programmatic callers 可使用 `InitialTrafficData::try_new` 构造显式无 Signals/Parking 数据，使用 `try_new_with_signals` 只提供 Signals，或使用 `try_new_with_signals_and_parking` 提供两个已 normalization registry。Final assembly 会消费 registries，并按 `InitialTrafficData` 自身的 `LaneGraph` 重新解析和复验全部 graph-dependent handles，禁止把另一张 graph 的预解析索引静默带入 world。

Canonical loader 顺序：

1. JSON syntax 与 version header。
2. exact current version。
3. strict v0.7 wire shape。
4. distance/time units。
5. Vehicle Profiles。
6. lane graph edge length、required `speedLimit` 与 connections；`speedLimit` 错误收窄到具体 edge field。
7. StopLines。
8. SignalGroups。
9. Controllers / Phases / States。
10. MovementGates。
11. signal global coverage / ownership / usage。
12. Parking areas identity。
13. Parking spaces identity / optional membership。
14. entry/exit anchors。
15. geometry。
16. orphan areas / ordered reverse indexes。
17. routes 与 final-StopLine rule。
18. `InitialTrafficData` final assembly/rebind。

`SignalRegistry` 预解析：

- StopLine ID -> `StopLineHandle` 与 edge handle。
- Group ID -> `SignalGroupHandle` 与 owner controller。
- Controller ID -> `SignalControllerHandle`。
- controller-local phase ID -> `SignalPhaseRef`。
- `(fromEdge, toEdge)` -> `MovementGateKey`、StopLine 与 normalized `SignalControl`。
- phase states -> controller group input order 的 compact aspect vector，并预计算 cycle 内 exclusive phase end offsets。

`ParkingRegistry` 预解析：

- area/space external ID -> dense opaque handle；
- optional `areaId` -> `ParkingAreaHandle` 与按 space input order 建立的 reverse members；
- entry/exit edge ID -> 当前 graph 的 `EdgeHandle` + validated progress；
- geometry -> canonical edge-relative rectangle value。

输入定义在 world 生命周期内 immutable。runtime handles 不持久化到 external package。

## 5. Validation Ownership

Serde / Schema 负责：

- required/type/closed shape；
- `id` / `xxxId` / `xxxIds` wire names；
- enum/tagged union；
- 单字段 integer/numeric bounds。

Core 负责：

- external ID、duplicate、unknown reference；
- StopLine one-per-edge、Gate pair/connection/StopLine ownership；
- outgoing Gate coverage、StopLine non-orphan；
- Group owner/usage；
- controller/phase cardinality、complete states、cycle checked sum、canonical offset；
- route continuity 与 final-StopLine；
- Parking identity/membership、anchor edge/progress、geometry、orphan area 与 ordered reverse index；
- world fixed delta compatibility；vehicle/signal runtime compliance 由 Core 承担。

Data crate 可以附加输入 index/path，不得复制 Core regex、graph、ownership 或 cycle algorithm。

## 6. Error Model

```text
DataError #[non_exhaustive]
  JsonSyntax
  JsonShape
  UnsupportedFormatVersion { expected, actual }
  InvalidUnit
  UnsupportedVehicleProfileModel
  CoreDomain { path, source: CoreError }
```

- syntax/shape 保留 path、line、column 与 serde source。
- domain error 保留 v0.7 field path 与结构化 `CoreError` source。
- state record 的 unknown/duplicate 错误定位到 `signals.controllers[i].phases[j].states[k].groupId`。
- missing/coverage 等无直接记录的全局错误定位到拥有该 invariant 的 phase/StopLine/Group。
- Parking duplicate 指向第二个 `.id`，unknown membership 指向 `.areaId`，anchor/geometry 指向具体 field，orphan 指向 area record。
- `DataError` 与 `CoreError` 实现 `Error + Send + Sync`；机器匹配 enum，不解析 Display。

## 7. World Compatibility 与 Vehicle Activation

Loader 不创建 world。`CoreWorld::with_traffic_data` 在 loader 成功后按顺序：

1. fixed delta 必须为正。
2. 每个 phase `durationMs >= fixedDeltaTimeMs`。
3. 构造 time-0 signal authority snapshot。
4. 注册 initial routes（复用 final-StopLine rule，并预解析 route-occurrence Gate metadata）。
5. 初始化 vehicles 并验证 overlap；empty/non-empty Signals 均走同一 vehicle activation path。

#96 已用 SignalStop、hard projection 与 permission-aware traversal 的完整车辆合规替代 legacy capability guard；signal world 中后续 `spawn_vehicle` 复用相同 activation 与 overlap validation。

## 8. Schema、Fixtures 与 Tests

Active schema：

```text
schemas/laneflow-data-v0.7.schema.json
```

该文件是 current active source schema。`$id` 按 ADR 0011 也是目标 public retrieval URL；合并后以固定 `main` revision/blob 登记 immutable publication provenance，production loader 仍不执行网络 I/O。v0.2-v0.5 schema 作为 immutable publication artifact 保留，但 current loader、fixture 与 normalization tests 只消费 v0.7。消费者入口见 [`schemas/README.md`](../../schemas/README.md)。

Canonical fixtures：

- `examples/data/v0.7-parking-signals-baseline.laneflow.json`
- `examples/data/v0.7-empty-signals-and-parking.laneflow.json`

Contract tests要求：

- schema 满足 Draft 2020-12 meta-schema；
- 两个 fixtures 同时通过 schema、production loader 与 Core normalization；
- v0.4、未来版、旧字段、JSON-LD、unknown field 与 open signal union 被拒绝；
- portable integer、identity/reference/ownership/coverage/complete-state/route-final-StopLine 有结构化错误；
- signal-only world compatibility 与失败原子性有 Core tests；
- omitted `areaId` 成功、explicit `null` 失败；Parking validation order、最窄 path、foreign-graph rebind 与 resolvers 有 Core/data tests；
- empty Signals/Parking fixture继续驱动现有 route/profile/vehicle regression。

旧 schema/fixture 不留在 active tree；历史证据由 Git history、`../reference/v0.3-closure-review.md` 与 `../reference/v0.4-closure-review.md` 保存。

## 9. Input Safety 与 Performance

v0.7 不设置固定 edge/route/profile/signal/parking count limit。调用方负责在进入 loader 前限制 input byte size；不可信网络/mod 输入需要后续 `LoadLimits`、隔离 validator 或流式 ingestion 设计。

JSON parsing、external ID lookup、Core normalization 和 handle allocation只发生在 load/world initialization。Static registries 使用 dense/indexed resolver，为 runtime 提供 O(1) lookup；10k all-vacant Parking 与 empty registry 的 warm step 均为 0 allocation，Parking registry 不进入 #107 的 per-tick/per-vehicle hot path。

## 10. v0.5 Parking loader 历史边界

#107 已在单一交付中完成 exact 0.5 version gate、strict private Parking DTO、Signals -> Parking -> Routes normalization、Core registry/rebind、paths、fixtures、tests 与 current docs 原子切换。v0.4 package 现在直接返回 `UnsupportedFormatVersion`，不自动补 empty Parking，也不提供 compatibility shim。

`areaId` 使用 custom wire presence decode 区分 omitted 与 explicit `null`；普通 `Option<String>` 的 null-to-None 行为未进入 production DTO。Loader 继续只接收调用方提供的内存 bytes/string，不创建 `CoreWorld`、不公开 DTO、不持久化 runtime parking state，也不联网解析 schema `$id`。调用方主动下载的 schema/package 属于调用方网络与不可信输入边界。完整 static/runtime 分层见 [`parking-system.md`](parking-system.md)。

## 11. Data v0.6 原子切换输入

#126 曾把目标数值迁移分配给 `formatVersion: "0.6"`。#144 因性能门槛失败而形成不迁移（no-go）结论，v0.6 从未成为 production current/published format。#185 后当前唯一生产加载器为 v0.7，并继续使用原有 `f64` 数值域；v0.5 只保留 immutable published schema，不建立 runtime 兼容责任。

v0.6 仍使用相同的先验版本闸口顺序：

```text
版本头
  -> 精确匹配 "0.6"
  -> 严格的 v0.6 私有线格式
  -> 高保真数值解析
  -> 受检 Core 转换
  -> 交叉引用和全局校验
  -> 一次性提交 LoadedPackage
```

迁移边界固定为：

- 私有线格式的 JSON 数值使用 `f64` 或等价高保真值暂存，只用于转换前校验和原始输入诊断；成功转换后不形成第二权威；
- `EdgeLength`、`Speed`、`Acceleration`、Profile 和 Parking 单值域通过受检转换进入 `f32`；禁止未检查的 `as f32`；
- `EdgeProgress` 以 `f64` 构造并观察有效值，高位/残差（high/residual）分量不进入数据传输对象（DTO）；Parking 入口/出口锚点直接规范化为 `EdgeProgress`；
- 原始转换错误可以保留 `f64` 输入；规范化单值域错误使用 `f32`，有效进度和实际采用 `f64` 的路线（route）派生值使用 `f64`；
- 模式（schema）必须与 Core 同向执行 ADR 0014 产品范围；Core 构造器仍是最终裁决者，Data 不复制 #127 的运行时判定实现；
- 任一转换、引用或全局不变量失败都在提交前返回，不留下部分 `LoadedPackage`、注册表、世界状态或事件。

#144 曾在同一候选边界内原子切换 `CURRENT_FORMAT_VERSION`、`laneflow-data-v0.6.schema.json` 及其 `$id`、发布当前指针、私有 DTO、规范化、样例和测试，但 no-go 后全部回退。未来若重启数值迁移，必须分配新的 format version 并在同一交付 PR 中完成这些切换；不保留 v0.7 分派、弃用别名、双精度开关、自动拆边（edge）、静默截断或迁移工具。

#127 已冻结九个目标 `f32` 固定绝对阈值、`EdgeProgress` 运算链和路线距离（route-distance）目标布局。#144 的回退不废除这些研究输入，但未来迁移仍不得用当前 `f64` 的 `1.0e-9`、动态相对误差、运行时末位单位（ULP）或通用近似比较辅助函数代替它们。

## 12. ScenarioManifest v0.1 原子加载

Scenario loader 不替代现有 Traffic loader，而是在它外层建立精确制品配对和 Spatial 规范化：

```text
manifest JSON syntax
  -> minimal manifest formatVersion
  -> exact "0.1" check
  -> strict manifest DTO / descriptor / ref uniqueness
  -> caller artifact ref duplicate / missing
  -> raw byte size
  -> raw byte SHA-256
  -> existing exact-current Traffic v0.7 loader
  -> Spatial JSON syntax / minimal version / exact "0.1" / strict DTO
  -> f64 coordinate finite + canonical range check
  -> checked canonical f32 points
  -> unknown / duplicate Traffic edge in Spatial input order
  -> missing Traffic edge in LaneGraph order
  -> one atomic LoadedScenario
```

`NamedArtifact` 只借用调用方已经读取的原始 bytes；loader 不解释 `artifactRef` 为路径或 URI，不读取文件，也不联网。descriptor media type 固定为 `application/vnd.laneflow.traffic+json` 与 `application/vnd.laneflow.spatial+json`。SHA-256 由 `sha2` 0.11 在 Data crate 内部计算，不新增 hex 依赖。

`LoadedSpatialPackage` 保存 `CanonicalFrameId` 与按 Traffic lane graph 稳定顺序排列的 `LoadedSpatialEdge`；每条 edge 已解析为 `EdgeHandle` 并只含受检 `CanonicalPoint3F32`。它是 #135 几何构建的输入，不是完整 `SpatialRegistry`，也不在 #134 中计算弧长、绑定长度、检查连接端点或生成采样基底。

失败 API 只返回 `ScenarioError`，不会返回 Traffic-only 或部分 Spatial 结果。JSON syntax/shape 保留 document、path、line/column；制品、坐标和 edge coverage 使用结构化 variant，嵌套 Traffic 错误保留原始 `DataError` source。
