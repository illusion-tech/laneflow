# Data Format 设计

**文档状态**: Accepted  
**最后更新**: 2026-07-10  
**适用范围**: v0.2 Lane Graph + Route 的外部数据格式、版本策略、lane graph / route 字段语义、validation 边界和 Core 消费入口  
**关联文档**:

- `../architecture.md`
- `../roadmap.md`
- `../adr/0001-project-scope.md`
- `../adr/0003-runtime-tick-and-determinism.md`
- `../adr/0005-core-identity-and-handle-model.md`
- `../../schemas/laneflow-data-v0.2.schema.json`
- `core-id-handles.md`
- `lane-graph.md`
- `route-system.md`

## 1. 目标

本文定义 v0.2 阶段 LaneFlow lane graph 与 route 的外部数据格式，作为 #30 的 G1 冻结输入。

目标：

- 固化 lane graph 与 route 的字段名、字段语义、单位和引用关系。
- 提供 JSON Schema 结构契约，避免示例数据和后续 validator 对字段结构各自解释。
- 明确 external ID 与 Core runtime handle 的边界。
- 明确版本、兼容性和后续扩展策略。
- 明确 validator、Core loader、example data、后续 Adapter / authoring tool 的消费边界。
- 为 #31 validation、#32 Core 对齐、#33 example route data、#34 回归测试和 #39 性能基线提供可引用输入。

非目标：

- 不定义 vehicle、spawn rule、signal、parking、vehicle following 或 Adapter API 的正式数据格式。
- 不冻结完整道路几何、mesh、样条、车道宽度、坐标系或渲染 transform。
- 不实现 authoring tool、schema validator、Core loader 或 runtime route registry。
- 不要求 JSON Schema 独立完成所有 graph / route 语义校验。
- 不引入 pathfinding、route planner、partial-edge target、route policy 或交通状态数据。
- 不承诺 v1.0 稳定格式；v0.2 只稳定 lane graph + route 的最小可执行格式。

## 2. 设计决策

### D1. v0.2 使用 JSON-compatible package model

状态：已接受。

v0.2 data format 是一个 JSON-compatible package model。字段名使用 lower camel case，数值使用 JSON number 可表达的 finite `f64` 范围，字符串使用 external ID。

概念模型：

```text
LaneFlowDataPackage
  formatVersion: string
  units: UnitSpec
  laneGraph: LaneGraphData
  routes: RouteData[]
  extensions?: object

UnitSpec
  distance: string

LaneGraphData
  edges: LaneEdgeData[]

LaneEdgeData
  id: string
  length: number
  connections: LaneConnectionData[]

LaneConnectionData
  to: string

RouteData
  id: string
  edges: string[]
```

说明：

- `id` 字段表示对应 domain 的 external ID，不是 Core runtime handle。
- `routes[].edges` 是 ordered edge external ID sequence。
- `connections[].to` 是 target edge external ID。
- v0.2 不把 Rust 类型名、`IndexMap`、内部 handle index 或具体 loader 结构冻结为外部格式。
- 官方 example data 应优先使用 `.laneflow.json` 或等价 JSON 文档；若后续工具选择 YAML / TOML，必须先定义等价映射，不得改变本文字段语义。

最小示例：

```json
{
  "formatVersion": "0.2",
  "units": {
    "distance": "meter"
  },
  "laneGraph": {
    "edges": [
      {
        "id": "A",
        "length": 10.0,
        "connections": [{ "to": "B" }]
      },
      {
        "id": "B",
        "length": 5.0,
        "connections": []
      }
    ]
  },
  "routes": [
    {
      "id": "R",
      "edges": ["A", "B"]
    }
  ]
}
```

### D2. `formatVersion` 是必填兼容性闸口

状态：已接受。

`formatVersion` 必须存在，v0.2 的值固定为字符串 `"0.2"`。

兼容性规则：

- v0.2 validator / loader 必须拒绝缺失 `formatVersion` 的数据包。
- v0.2 validator / loader 默认只接受 `"0.2"`；更高版本必须由后续兼容性设计显式允许。
- v0.2 core-defined object 默认采用 closed shape：未在本文定义的字段必须放入 `extensions`，否则 validator 应拒绝。
- patch 级修订不得改变已定义字段语义，只能澄清说明、增加非 Core 必需的 `extensions` 内容或修复文档错误。
- 任何会改变 lane graph / route Core 语义的字段新增、字段删除、默认值变化或 validation 口径变化，都必须提升格式版本并更新本文或后续 superseding 文档。

### D3. external ID 使用严格 ASCII token

状态：已接受。

v0.2 external ID 是稳定、可读、domain-scoped 的字符串 token。它只在数据文件、validator、Core loader、resolver、debug、日志和 Adapter 绑定中使用，不进入 runtime hot path。

规则：

- `id` 必须是非空 ASCII 字符串，长度为 1 到 128 个字符。
- v0.2 官方 validator 应限制为 ASCII token：`^[A-Za-z0-9][A-Za-z0-9._:/-]{0,127}$`。
- ID 比较大小写敏感。
- validator / loader 不做 trim、case fold 或 Unicode normalization。
- 同一 domain 内必须唯一：edge IDs 在 `laneGraph.edges` 内唯一，route IDs 在 `routes` 内唯一。
- 不同 domain 可以复用同一文本，例如 edge `A` 与 route `A` 同时存在是合法的。
- data format 不持久化 `VehicleHandle`、`RouteHandle`、`EdgeHandle` 或任何 handle 内部数值。

原因：

- ASCII token 能避免跨语言、跨文件系统、URL、日志和 UI 工具中的 Unicode normalization 差异。
- UUID 字符串、短 slug、层级式 `map/road/lane` ID 都能被表达。
- external ID 仍保留可诊断性，Core runtime 通过 registry / resolver 映射到 typed handle。

### D4. `units.distance` 声明数据包距离单位

状态：已接受。

`units.distance` 必须存在。v0.2 固定为字符串 `"meter"`。

规则：

- v0.2 validator / loader 必须拒绝缺失 `units.distance` 或值不为 `"meter"` 的数据包。
- Core 消费的 `length` 数值按 meter 解释，但 Core runtime 内部仍只处理 engine-agnostic distance units。
- Core 不负责单位换算；需要转换时应由 authoring tool 或 importer 在进入 v0.2 validator / Core loader 前完成。
- Adapter / Presentation 若使用同一数据包中的非 Core 几何扩展，必须按 `units.distance` 解释长度。
- v0.2 不定义 speed、time、angle 或 coordinate unit 字段，因为 lane graph + route 最小格式只包含 edge length 和拓扑引用。

### D5. lane graph 是 directed edge graph

状态：已接受，依据 `lane-graph.md`。

`laneGraph.edges` 是有序 edge 列表。每个 edge 表示一个方向上的可通行行驶段。

字段规则：

- `laneGraph.edges` 必须存在，且必须是数组。
- v0.2 允许空 lane graph，但只要存在 route，route validation 就会因 unknown edge 失败。
- `edges[].id` 是 edge external ID。
- `edges[].length` 是 edge 的权威长度，单位由 `units.distance` 声明。
- v0.2 data format 将 `EDGE_BOUNDARY_EPSILON` 固定为 `1.0e-9`；`edges[].length` 必须大于该值。JSON Schema 使用 `exclusiveMinimum: 1e-9` 表达同一约束。
- `edges[].connections` 必须存在，且必须是数组；terminal edge 使用空数组 `[]`。
- `connections[].to` 必须引用同一个 `laneGraph.edges` 集合中的 edge external ID。
- self connection 合法，但必须显式写成 `{ "to": "<same-edge-id>" }`。
- 同一个 source edge 内不得重复声明同一个 `to`。
- disconnected graph component 合法；单条 route 是否连通由 route validation 判断。

`connections` 使用对象数组，而不是简单字符串数组，是为了给后续 signal、turn restriction、cost 或 debug metadata 保留扩展位置。v0.2 Core 只消费 `to` 字段。

### D6. route 是有限 ordered edge sequence

状态：已接受，依据 `route-system.md`。

`routes` 是 route definition 列表。每个 route 是有限、有序的 edge external ID sequence。

字段规则：

- `routes` 必须存在，且必须是数组。
- `routes[].id` 是 route external ID。
- `routes[].edges` 必须存在，且必须是非空数组。
- `routes[].edges[*]` 必须引用 `laneGraph.edges[*].id`。
- route 中任意相邻 edge pair 必须存在显式 connection。
- route 可以重复引用同一个 edge；重复 edge 表示有限 route 中的显式回环，由 route edge index 区分位置。
- route target 固定为最后一个 route edge 的出口边界。
- v0.2 不支持 partial-edge target、持续循环 route、route policy、planner cost 或 traffic-aware reroute 字段。

示例：

```json
{
  "id": "loop-once",
  "edges": ["A", "B", "A"]
}
```

上述 route 只有在 lane graph 中同时声明 `A -> B` 与 `B -> A` connection 时合法。车辆到达最后一个 `A` 的出口边界后完成，不会自动回到 route 起点。

### D7. `extensions` 只能承载非 Core 语义

状态：已接受。

v0.2 data package 可包含可选 `extensions` object，用于工具、debug、authoring 或实验性 metadata。

规则：

- `extensions` 不得改变 Core 对 lane graph 和 route 的解释。
- Core loader 可以完全忽略 `extensions`。
- `extensions` 中的 key 应使用明确 namespace，例如 `com.example.tool` 或 `x-tool-name`。
- 不得在 `extensions` 中定义影响 route connectivity、edge length、route target 或 handle registry 的隐藏语义。
- 如果某个 extension 需要成为 Core 或 Adapter contract，必须通过后续 issue 提升到正式字段。

v0.2 不在正式字段中冻结 geometry。若 example 或 tool 暂时需要几何，可放在 extension 中并明确“不作为 Core data-format contract”。

### D8. JSON Schema 是结构层契约

状态：已接受。

v0.2 提供 JSON Schema Draft 2020-12 schema：

```text
schemas/laneflow-data-v0.2.schema.json
```

Schema 职责：

- 固定 `$schema` dialect 为 `https://json-schema.org/draft/2020-12/schema`。
- 固定 `$id`，供 validator、example data 和后续 CI 引用。
- 校验 `formatVersion` 必须为 `"0.2"`。
- 校验 `units.distance` 必须为 `"meter"`。
- 校验 core-defined object 为 closed shape，未定义字段只能放入 `extensions`。
- 校验 external ID pattern。
- 校验 `laneGraph.edges[].length` 是 number 且 `exclusiveMinimum: 1e-9`。
- 校验 `connections[].to` 和 `routes[].edges[]` 的字符串形态。
- 校验 `routes[].edges` 是非空数组。

Schema 不负责：

- edge ID / route ID 是否重复。
- `connections[].to` 是否引用存在 edge。
- 同一 source edge 内是否重复连接同一 target。
- route 相邻 edge pair 是否连通。
- disconnected graph component 是否被 route 合法使用。
- runtime stale handle、route in use、vehicle state 或 tick delta。

如果 schema 与本文语义冲突，PR 不得合并，必须先统一文档和 schema。#31 validator 和 #33 example data 必须同时引用本文和 schema；CI 若后续加入 schema validation，也必须使用该 schema 作为结构层输入。

## 3. Validation 边界

v0.2 validation 分三层：

| 层级 | 负责者 | 输入 | 输出 |
| --- | --- | --- | --- |
| 语法 / schema validation | JSON Schema / data-format validator | JSON-compatible package | 字段缺失、类型错误、版本错误、ID token 错误、closed shape 错误 |
| topology / route validation | validator 或 Core loader | 已解析 data package | duplicate ID、unknown edge、duplicate connection、disconnected route |
| runtime validation | Core runtime | handle 化后的 world / command | stale handle、route in use、tick delta、vehicle state 不变量 |

data-format validation 必须覆盖：

- `formatVersion` 缺失或不是 `"0.2"`。
- `units.distance` 缺失或不是 `"meter"`。
- core-defined object 出现本文未定义字段。
- external ID 缺失、为空、超长或不符合 ASCII token 规则。
- duplicate edge ID。
- duplicate route ID。
- edge length 不是 finite number。
- edge length 小于或等于 `EDGE_BOUNDARY_EPSILON`。
- connection target 引用不存在的 edge。
- 同一 source edge 内重复 connection target。
- route 为空。
- route edge 引用不存在的 edge。
- route 相邻 edge pair 不连通。

data-format validation 不覆盖：

- vehicle 初始状态。
- runtime spawn / despawn 命令。
- route register / remove 的 active handle 状态。
- vehicle following、signals、parking、intersection right-of-way。
- path optimality、几何曲率、turn radius 或碰撞。

错误报告应保留 external ID，并按输入顺序稳定输出。若采用 fail-fast，错误发现顺序必须基于 package 字段顺序、edge 输入顺序、connection 输入顺序和 route 输入顺序；若采用批量收集错误，错误列表也必须稳定排序。

## 4. Core Loader 边界

Core loader 负责把 v0.2 data package 转换为 Core runtime 可消费的 normalized state。

推荐流程：

1. 校验 `formatVersion`、`units` 和字段类型。
2. 校验 external ID token 与 domain 内唯一性。
3. 为 edge external IDs 分配 `EdgeHandle`。
4. 将 `connections[].to` 解析为 `EdgeHandle`。
5. 为 route external IDs 分配 `RouteHandle`。
6. 将 `routes[].edges` 解析为 `EdgeHandle` sequence，并执行 route connectivity validation。
7. 构建 resolver，使 Adapter / debug / log 可以从 handle 查询 external ID。

失败语义：

- 初始化失败不得返回部分可用 `CoreWorld`。
- `register_route` 形式的运行时加载失败不得修改 active route registry。
- validation error 面向数据作者，应携带 external ID，不应只携带 handle。
- handle 是 world-scoped token，不得跨 data package、跨 `CoreWorld` 或跨进程持久化。

## 5. 与 v0.1 内部结构的迁移

v0.1 Rust Core 已有最小内部输入类型，但它们不是稳定 data spec。v0.2 data format 与 v0.1 内部结构的迁移关系如下：

| v0.1 内部结构 | v0.2 data-format 字段 | 说明 |
| --- | --- | --- |
| `LaneEdge::id()` | `laneGraph.edges[].id` | 从内部字符串升级为正式 edge external ID。 |
| `LaneEdge::length()` | `laneGraph.edges[].length` | 继续使用 engine-agnostic distance unit，需满足 `> EDGE_BOUNDARY_EPSILON`。 |
| `LaneEdge::next_edge_ids()` | `laneGraph.edges[].connections[].to` | 从字符串数组升级为可扩展 connection object。 |
| `Route::id()` | `routes[].id` | 从内部字符串升级为正式 route external ID。 |
| `Route::edge_ids()` | `routes[].edges` | 保留 ordered edge sequence 语义。 |
| `VehicleState` | 不属于 #30 正式格式 | 初始车辆、spawn rules 和 vehicle profiles 后续单独设计。 |
| `CoreEvent` string payload | 不属于 data format | #32 应按 ADR 0005 迁移到 handle payload + resolver。 |

#30 不要求修改 v0.1 Rust API。#32 负责把内部结构对齐 v0.2 数据模型和 handle registry。

### 已发现问题追踪

本设计形成过程中发现的 v0.1 / v0.2 衔接问题必须显式分流，避免 #30 完成后丢失：

| 发现问题 | 影响 | 归属 Issue | 是否阻断 #30 |
| --- | --- | --- | --- |
| v0.1 runtime 仍使用 `String` 表示 edge / route / vehicle 引用 | 在 10k vehicles / 60 tick/s 等规模下可能放大 clone、hash、排序和事件 payload 成本；也会混淆 external ID 与 runtime handle 边界。 | #32 | 否 |
| `CoreWorld::step` 依赖 `vehicle_id` 字符串排序稳定事件顺序 | 长期不应把 external ID 字符串排序作为 update order contract。 | #32 | 否 |
| `LaneEdge::next_edge_ids()` 是字符串数组 | 不能作为 v0.2 长期外部数据格式；缺少 connection 扩展点。 | #30 / #32 | 否；#30 已改为 `connections: [{ to }]`，#32 负责内部迁移 |
| duplicate connection、unknown reference、route continuity 等语义无法由 JSON Schema 完整表达 | 需要 schema 后的 domain validator，并保持 external ID 错误诊断和稳定错误顺序。 | #31 | 否 |
| 示例数据和回归测试若只覆盖 happy path，会遗漏重复 ID、断连、self connection 等边界 | v0.2 后续实现可能把格式边界误解为 runtime 内部结构。 | #33 / #34 | 否 |
| 10k vehicles / 60 tick/s 目标规模需要可重复性能验证 | 结构迁移完成不等于性能风险已被验证，需要独立定义 benchmark / profiling 基线。 | #39 | 否 |

## 6. 消费者边界

### Core

Core 消费：

- edge external ID。
- edge length。
- directed connection。
- route external ID。
- ordered route edge sequence。

Core 不消费：

- geometry。
- mesh / prefab / actor / entity 绑定。
- signal、parking、vehicle following 或 intersection metadata。
- authoring tool UI metadata。
- `extensions`。

### Validator

Validator 负责在 Core loader 前给数据作者提供稳定、可诊断的错误。#31 应以本文字段和 validation 列表为输入。

### Example Data

#33 example route data 应使用本文字段，并至少覆盖：

- 仓库基线示例位于 `../../examples/data/v0.2-route-baseline.laneflow.json`。它使用 meter 作为距离单位，包含 `main-route` 的 normal two-edge route、`loop-once` 的 explicit self loop / repeated edge route、terminal `exit`，以及未被 route 引用的合法 disconnected `isolated` edge。
- terminal edge。
- normal two-edge route。
- repeated edge route 或 explicit self loop route。
- 至少一个 disconnected graph component 的合法样例，证明 disconnected graph 不等于 route 断连。

### Adapter / Authoring Tool

后续 Adapter / authoring tool 可以使用 external ID 与 Core resolver 建立绑定，但不得：

- 持久化 runtime handle。
- 复制 Core topology validation 作为不同语义。
- 在 `extensions` 中隐藏影响 Core route 行为的字段。

## 7. ADR 判断

本文不新增 ADR。

原因：

- 项目范围由 ADR 0001 冻结。
- fixed tick 与 determinism 由 ADR 0003 冻结。
- external ID、typed handle、registry / resolver 和动态 route lifecycle 由 ADR 0005 冻结。
- 本文是在这些 ADR 之上固化 v0.2 lane graph + route 的外部数据格式，不新增更高层不可逆架构取舍。

若后续要引入动态 lane graph 拓扑、pathfinding、geometry-driven Core collision、跨格式兼容加载器或稳定 Adapter data contract，应重新评估是否需要新增 ADR。

## 8. 后续 Issue 输入

#31 route / lane graph validation：

- 以 `schemas/laneflow-data-v0.2.schema.json`、`formatVersion`、external ID token、edge length、connection 和 route validation 列表作为必测输入。
- 错误结果必须携带 external ID，并保持稳定顺序。
- 明确 schema validation 与 Core runtime validation 的边界。

#32 v0.1 内部结构对齐 v0.2 数据模型：

- 不再把 `next_edge_ids: Vec<String>` 留作长期 runtime hot path。
- 将 external ID 在 loader / registry 阶段归一化为 `EdgeHandle` / `RouteHandle`。
- 保留 resolver，供 event、debug 和 Adapter 回查 external ID。
- 记录 Core API breaking change 或 v0.1 prototype 迁移边界。

#33 example route data：

- 使用 `formatVersion: "0.2"`、`units.distance: "meter"` 和本文字段名。
- 示例数据必须能通过 `schemas/laneflow-data-v0.2.schema.json` 的结构校验。
- 示例不得包含未正式冻结的 signal、parking、vehicle following 或 Adapter geometry contract。

#34 lane graph + route regression tests：

- 引用本文 validation 列表和 example data。
- 覆盖 duplicate ID、unknown reference、duplicate connection、terminal edge、repeated edge route、self connection 和 disconnected graph component。

#39 Core 性能基线：

- 在 #32 对齐 typed handle / registry / resolver 后，定义 10k vehicles / 60 tick/s 的最小可重复 benchmark 或 profiling 场景。
- 验证 runtime hot path 不依赖 external ID 字符串排序、clone 或事件 payload 主路径分配。
- 记录性能结果是硬门槛、软基线还是趋势审计，不把 #30 data-format PR 作为性能验证阻断项。
