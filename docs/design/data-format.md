# Data Format 设计

**文档状态**: Accepted  
**最后更新**: 2026-07-13  
**适用范围**: 当前 v0.3 外部数据格式、版本策略、lane graph / route / Vehicle Profile 字段语义、validation 与消费边界  
**关联文档**:

- `../architecture.md`
- `../roadmap.md`
- `../adr/0001-project-scope.md`
- `../adr/0003-runtime-tick-and-determinism.md`
- `../adr/0005-core-identity-and-handle-model.md`
- `../adr/0007-traffic-data-crate-and-loader-boundary.md`
- `../adr/0008-pre-1.0-data-format-version-policy.md`
- `../../schemas/laneflow-data-v0.3.schema.json`
- `data-loading.md`
- `lane-graph.md`
- `route-system.md`
- `vehicle-following.md`

## 1. 目标与非目标

本文定义 LaneFlow 当前 v0.3 external package，是 schema、production loader、validator、example 和后续 Adapter / authoring tool 的数据契约。

目标：

- 固化 lane graph、route 与 Vehicle Profile 的字段名、语义、单位和引用关系。
- 提供 Draft 2020-12 JSON Schema 结构契约。
- 明确 external ID 与 Core runtime handle 的边界。
- 明确单一当前版本、validation 分层和 Core normalization 边界。
- 保持格式引擎无关，并为后续工具提供一致输入。

非目标：

- 不持久化 initial vehicles、spawn schedule、demand、runtime handle 或 Adapter asset binding。
- 不冻结道路 geometry、mesh、样条、坐标系或 presentation transform。
- 不实现 authoring tool、批量 validator、pathfinding 或 route planner。
- 不要求 JSON Schema 独立完成 graph、route 或 profile 的全部 domain validation。
- 不兼容加载 v0.2，也不提供自动迁移；版本政策见 ADR 0008。
- 不承诺 v1.0 的长期稳定格式。

## 2. 当前 Package Model

当前格式使用 JSON-compatible package model。字段名使用 lower camel case，数值必须能表示为 finite `f64`，标识符使用 external ID。

```text
LaneFlowDataPackage
  formatVersion: "0.3"
  units: UnitSpec
  laneGraph: LaneGraphData
  routes: RouteData[]
  vehicleProfiles: VehicleProfileData[]
  extensions?: object

UnitSpec
  distance: "meter"
  time: "second"

LaneGraphData
  edges: LaneEdgeData[]

LaneEdgeData
  id: external ID
  length: number
  connections: LaneConnectionData[]

LaneConnectionData
  to: external edge ID

RouteData
  id: external ID
  edges: external edge ID[]

VehicleProfileData
  id: external ID
  length: number
  model: "iidm"
  desiredSpeed: number
  minGap: number
  timeHeadway: number
  maxAcceleration: number
  comfortableDeceleration: number
  emergencyDeceleration: number
```

最小可执行示例位于 `examples/data/v0.3-profile-baseline.laneflow.json`。它同时覆盖 normal route、terminal edge、explicit self loop / repeated edge route、合法 disconnected component 和一个 IIDM profile。

## 3. 设计决策

### D1. 只维护一个当前格式

状态：已接受，依据 ADR 0008。

`formatVersion` 必填，当前值固定为字符串 `"0.3"`。production loader 和当前 schema 只接受该值：

- 缺失、`null` 或类型错误返回 JSON shape error。
- 旧版、未来版或其他字符串返回包含 expected/actual 的 version error。
- 旧版不会隐式升级，也不会获得默认 profile。
- 1.0 前发生破坏性格式调整时，提升版本并替换当前 schema、fixture、private DTO 和 loader，不在 active runtime 中累积历史分支。
- 历史证据由 Git history、里程碑收口文档和 immutable permalink 保存。

Core-defined object 使用 closed shape。未定义字段只能放入允许的 `extensions`，否则 schema/loader 拒绝。

### D2. External ID 使用严格 ASCII token

状态：已接受，沿用 ADR 0005。

External ID 规则：

- 非空 ASCII 字符串，长度 1 到 128。
- pattern 为 `^[A-Za-z0-9][A-Za-z0-9._:/-]{0,127}$`。
- 比较大小写敏感，不 trim、case fold 或 Unicode normalize。
- edge、route、profile 分别在自身 domain 内唯一；不同 domain 可以复用相同文本。
- 数据格式不持久化任何 handle 或 handle 内部数值。

External ID 用于文件、validation、resolver、debug、日志和 Adapter binding；runtime hot path 使用 typed handle。

### D3. 单位显式固定

状态：已接受。

`units.distance` 必须为 `"meter"`，`units.time` 必须为 `"second"`。由此：

- edge/profile length 和 min gap 使用 meter。
- desired speed 使用 meter/second。
- time headway 使用 second。
- acceleration/deceleration 使用 meter/second^2。
- Core 不负责单位换算；authoring/importer 必须在进入当前 loader 前完成转换。
- 缺失或其他单位必须被拒绝，不做猜测或隐式换算。

### D4. Lane graph 是 directed edge graph

状态：已接受，依据 `lane-graph.md`。

- `laneGraph.edges` 必填，可以为空；存在 route 时仍须通过 route reference validation。
- `edges[].id` 是 edge external ID。
- `edges[].length > EDGE_BOUNDARY_EPSILON`，当前 epsilon 为 `1.0e-9 meter`。
- `edges[].connections` 必填；terminal edge 使用空数组。
- `connections[].to` 必须引用当前 graph 中存在的 edge。
- self connection 合法，但必须显式声明。
- 同一 source edge 不得重复连接同一 target。
- disconnected component 合法；单条 route 的连续性由 route validation 判断。

Connection 使用对象而不是字符串，是为了以后提升 turn restriction、cost 或 debug metadata 时保留扩展位置。当前 Core 只消费 `to`。

### D5. Route 是有限 ordered edge sequence

状态：已接受，依据 `route-system.md`。

- `routes` 必填，可以为空。
- `routes[].id` 在 route domain 内唯一。
- `routes[].edges` 必填且至少包含一个 edge ID。
- 每个引用必须存在，相邻 edge pair 必须在 graph 中显式连通。
- route 可以重复 edge，也可以使用 explicit self loop。
- route target 固定为最后一个 route edge 的出口边界。
- 当前格式不表达无限循环 policy、partial-edge target、planner cost、traffic-aware reroute 或 runtime route generation。

### D6. Vehicle Profile 是显式不可变输入

状态：已接受，依据 ADR 0006 与 `vehicle-following.md`。

`vehicleProfiles` 顶层字段必填，可以为空。每个 profile 的所有字段均必填，不使用 loader 默认值：

- `id`：profile external ID，domain 内唯一。
- `model`：当前固定为 `"iidm"`。
- `length > GEOMETRY_GAP_EPSILON`。
- `desiredSpeed > 0`。
- `minGap >= 0`。
- `timeHeadway > 0`。
- `maxAcceleration > 0`。
- `comfortableDeceleration > 0`。
- `emergencyDeceleration > 0` 且不小于 comfortable deceleration。
- 所有数值必须 finite。

Schema 负责字段 shape 和单字段下界；finite、external ID uniqueness 和 deceleration cross-field ordering 由 Core constructor 校验。

### D7. `extensions` 不承载 Core 语义

状态：已接受。

顶层可包含可选 `extensions` object，用于工具、debug 或实验 metadata。Core 不消费它。任何影响 Core、Adapter 稳定契约或运行行为的字段都必须通过后续 Issue 提升为正式字段，不能长期隐藏在 extension 中。

### D8. JSON Schema 是结构层契约

状态：已接受。

当前 schema：

```text
schemas/laneflow-data-v0.3.schema.json
```

Schema 负责：

- Draft 2020-12 dialect 与稳定 `$id`。
- 当前 `formatVersion`、distance/time units。
- required fields、类型、closed shape 和 external ID pattern。
- edge/profile 单字段 numeric bounds。
- connection/route/profile 数组 shape。

Schema 不负责：

- duplicate external ID。
- unknown connection/route reference。
- duplicate connection target。
- route continuity。
- profile deceleration cross-field ordering。
- runtime stale handle、route in use、vehicle state 或 tick delta。

Schema、private DTO 和本文语义冲突时，PR 不得合并，必须在同一变更中统一。

## 4. Validation 分层

| 层级 | 负责者 | 典型错误 |
| --- | --- | --- |
| syntax / shape | JSON parser、Serde、JSON Schema | JSON 无效、required/type/closed shape/version const/ID pattern/numeric shape |
| domain normalization | Core constructors，经 data loader 调用 | duplicate ID、unknown reference、duplicate connection、route discontinuity、non-finite、profile cross-field |
| runtime | CoreWorld / lifecycle command | stale handle、route in use、tick mismatch、vehicle state invariant |

Production loader 使用 fail-fast 和稳定输入顺序。错误保留 JSON path 或 external ID，并以 Core constructor 作为 domain invariant 的唯一事实源。批量 authoring diagnostics 属于未来 validator/tooling，不进入当前 runtime loader。

## 5. Loader 与 Core Normalization

具体 Rust 所有权由 ADR 0007 和 `data-loading.md` 固化：

```text
laneflow-data -> laneflow-core
laneflow-core -X-> laneflow-data
```

- `laneflow-data`：version header、当前 private DTO、JSON syntax/shape、units、路径诊断和转换。
- `laneflow-core`：lane graph、route、Vehicle Profile、registry/resolver 和全部 domain/runtime invariant。
- loader 接收内存 bytes/string，不读取文件，不创建 `CoreWorld`。
- public 结果是单一当前 `LoadedPackage`，包含已验证的 `InitialTrafficData`。
- 初始化失败不返回部分数据；handle 不跨 package/world/process 持久化。

概念流程：

1. 解析最小 `formatVersion` header。
2. 拒绝非当前版本。
3. 严格解析当前 DTO 和 units。
4. 通过 Core constructors 构造 edge、connection、route 与 profile。
5. 构造 `InitialTrafficData` 并完成组合 validation。
6. 返回 resolver-ready 的 Core input。

## 6. 消费者边界

### Core

Core 消费 edge/route/profile external ID、edge length、directed connection、ordered route sequence 和 profile 参数。Core 不消费 JSON、geometry、presentation binding、authoring metadata 或 `extensions`。

### Validator / Authoring

Validator 可以执行 schema 和 Core-equivalent domain validation，为作者提供批量错误；不得形成与 Core 不同的 domain 语义。Importer 必须产出当前格式，不能依赖 production loader 自动迁移旧数据。

### Example

当前 fixture 必须同时通过 schema 与 production loader，并驱动 Core route behavior regression。示例不包含未冻结的 signal、parking、initial vehicle、spawn schedule 或 Adapter geometry contract。

### Adapter

Adapter 可以使用 external ID 与 resolver 建立 engine asset binding，但不得持久化 handle、复制 topology/profile validation 为不同规则，或把影响 Core 的字段藏在 `extensions` 中。

## 7. 历史与迁移边界

v0.2 Lane Graph + Route 的设计、验证和收口结论保存在 `docs/reference/v0.2-closure-review.md` 与 Git history。v0.3 沿用其 lane graph/route 领域语义，并加入时间单位与 Vehicle Profile；它不保持 v0.2 wire compatibility。

pre-1.0 仓库内资产采用直接迁移：

| 历史 v0.2 | 当前 v0.3 |
| --- | --- |
| `formatVersion: "0.2"` | `formatVersion: "0.3"` |
| 只有 `units.distance` | 增加必填 `units.time: "second"` |
| 无 `vehicleProfiles` | 增加必填数组，可为空 |
| v0.2 schema/fixture | 从 active tree 移除，由 immutable Git history 保存 |
| production loader 兼容 | 不提供；明确返回 unsupported version |

若出现真实外部发布资产、集成承诺或大规模不可同步迁移的数据，再按 ADR 0008 单独设计离线 migration tool 或限期 compatibility layer。

## 8. 后续实现约束

- #73 交付当前 v0.3 schema、fixture、production loader、Profile registry 和 `InitialTrafficData`。
- #74 及后续 Vehicle Following 实现只依赖当前 domain contract，不增加 v0.2 shim。
- 新增 signals、parking、initial vehicles、spawn schedule、dynamic topology 或 stable Adapter data contract 时，必须先完成对应 G1 设计。
- 首次对外发布稳定资产或接近 1.0 时，必须重新评估长期版本、deprecation、迁移和支持窗口政策。
