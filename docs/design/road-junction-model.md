# Road / Junction / Maneuver 静态模型

**文档状态**: Accepted<br>
**最后更新**: 2026-07-24<br>
**适用范围**: #228 冻结的长期 Road/Junction/Maneuver 分层、v0.9 最小静态生产化 profile、ManeuverGate、Route occurrence、Traffic v0.8 target、确定性与性能边界<br>
**实现状态**: 本文是 #196/#229 的 G1 输入；current production 仍为 Traffic v0.7、pair-based MovementGate 和无 Junction/Movement/ManeuverPath registry，本文 target 尚未由 #229 实现

**关联文档**:

- `../architecture.md`
- `../roadmap.md`
- `../adr/0003-runtime-tick-and-determinism.md`
- `../adr/0005-core-identity-and-handle-model.md`
- `../adr/0007-traffic-data-crate-and-loader-boundary.md`
- `../adr/0008-pre-1.0-data-format-version-policy.md`
- `../adr/0009-signal-indication-gate-and-policy-separation.md`
- `../adr/0011-schema-identifier-and-publication-contract.md`
- `../adr/0013-engine-neutral-spatial-geometry-and-length-authority.md`
- `../adr/0017-static-road-junction-maneuver-and-gate-identity.md`
- `lane-graph.md`
- `route-system.md`
- `signal-system.md`
- `vehicle-following.md`
- `spatial-geometry.md`
- `data-format.md`
- `example-scenarios.md`

## 1. 目标、状态与非目标

### 1.1 目标

本文冻结：

- RoadSection、LaneGroup、Junction、Movement、ManeuverPath 与 JunctionGroup 的长期
  职责和关系；
- v0.9 实际生产化的 `Junction -> Movement -> ManeuverPath` 最小静态层级；
- LaneGraph、Route、ManeuverPath、ManeuverGate、Signals 与 Spatial 的 authority；
- external ID、typed handle、normalization、foreign rebind 与 first-error；
- Traffic v0.8 clean-break target 和 #229 的原子迁移边界；
- route registration-time occurrence compilation 与 steady-tick 性能约束；
- #196 必须进一步冻结的 protected-turning profile 和安全证明。

### 1.2 当前 production 与 target

| 范围              | Current production                   | Accepted target                     |
| ----------------- | ------------------------------------ | ----------------------------------- |
| Traffic           | exact-current `0.7`                  | #229 clean-break `0.8`              |
| Lane topology     | LaneEdge + directed connection       | 保持，并增加静态 Junction hierarchy |
| Route             | 显式有限 edge sequence               | 保持为实际 traversal authority      |
| Gate              | pair-based `MovementGateKey`         | 一等 `ManeuverGateHandle`           |
| Junction identity | connector ID 命名推断                | 显式 external ID + handle           |
| Spatial           | SpatialPackage `0.1` edge centerline | shape 保持 `0.1`                    |
| Conflict          | authoring protected phases           | 仍由 #196 authoring 证明；无 solver |

在 #229 G4 前，本文不得被描述为已交付的 Core/Data API。Current loader、schema、
fixtures、generator 和 native example 继续以 v0.7 为事实。

### 1.3 非目标

- 不实现 dynamic road topology、runtime Junction lifecycle 或 in-place mutation。
- 不实现 pathfinding、route planner、lane selection、lane change 或 overtaking。
- 不实现 ConflictZone、priority、gap acceptance、reservation 或 permissive turn。
- 不实现 roundabout、parking access、merge/diverge behavior 或 interchange。
- 不把 `left/straight/right`、国家法规或设施类型写进 SignalController。
- 不冻结道路 mesh、Junction polygon、lane width 或世界坐标。
- 不生产化 RoadSection、LaneGroup 或 JunctionGroup。
- 不提供 Traffic v0.7 runtime compatibility、双 schema 或 deprecated API alias。

## 2. 术语与长期分层

### 2.1 LaneEdge

LaneEdge 是 LaneGraph 中有方向、可遍历的最小拓扑与纵向进度单元。它继续拥有：

- external ID 与 `EdgeHandle`；
- length、speed limit；
- directed next-edge connections；
- vehicle route cursor 和 edge-local progress 的引用目标。

LaneEdge 不自动等于道路、Junction、Movement 或 ManeuverPath。

### 2.2 RoadSection

RoadSection 是未来有方向的道路结构分段，用于表达：

- 稳定的道路级上游/下游边界；
- 同一方向的 ordered lanes；
- 共享道路属性；
- lane adjacency / lane change 的结构 owner。

RoadSection 不是 Route，不决定车辆当前 path，也不复制 LaneEdge 的 length 或
connection。v0.9 只冻结术语，不引入 `RoadSectionHandle`、Core registry 或 Traffic
array。

### 2.3 LaneGroup

LaneGroup 是 RoadSection 内可选的 authoring/组织概念，可用于表达一组共享用途或
方向的 lanes。它不是 v0.9 Core root entity。未来是否需要独立 handle、是否与
RoadSection 一对多、以及如何进入 lane-change policy，必须由后续 G1 冻结。

### 2.4 Junction

Junction 是通用的最小拓扑连接/潜在冲突单元：

- 平面十字或丁字交叉可以是 Junction；
- merge、diverge、roundabout 的单个连接节点可以是 Junction；
- parking access 或 interchange 可以由多个 Junction 组成；
- 普通 geometry split、portal 或无连接的立体交叉不自动成为 Junction。

`Intersection` 只表示平面道路交叉 profile/authoring 语义，不成为 Core 根实体。

### 2.5 Movement

Movement 是一个 Junction 内的道路级通行意图和稳定分组 identity。它聚合同一
意图的一个或多个 lane-level ManeuverPath。

Core 不内置 `left | straight | right | uTurn` 枚举。转向标签、法规分类、route
权重或 UI 名称属于 authoring/profile metadata。RoadSection 尚未生产化期间，
Movement 的道路级 from/to endpoints 是由 authoring 保证的语义；Core 通过 child
ManeuverPath 的拓扑一致性验证其可遍历性，不保存 dangling section strings。

### 2.6 ManeuverPath

ManeuverPath 是 Movement 的 lane-level、有限、显式、可遍历实现。它引用
LaneGraph edges，并为 Route occurrence、Gate binding、debug 和未来 conflict
domain 提供稳定 identity。

ManeuverPath 不选择车辆路线，不拥有车辆状态，也不替代 Route。

### 2.7 JunctionGroup

JunctionGroup 是多个 Junction 的非行为组合，可供未来 roundabout、complex
junction、parking facility 或 interchange 的 authoring、presentation、partition
和管理使用。

JunctionGroup 不聚合为：

- 单一巨大 conflict solver；
- 单一 SignalController clock；
- 单一 runtime availability；
- 单一车辆 route planner。

v0.9 不引入 JunctionGroup handle、registry 或 schema。

## 3. 静态 owner 与引用关系

### 3.1 唯一 owner 层级

```text
Junction
  1..* Movement
    1..* ManeuverPath
```

规则：

- 每个 Movement 恰好属于一个 Junction。
- 每个 ManeuverPath 恰好属于一个 Movement。
- 每个 Junction 至少拥有一个 Movement。
- 每个 Movement 至少拥有一个 ManeuverPath。
- parent/child owner 不在两个方向重复持久化。
- LaneEdge 只由 LaneGraph 拥有；path 保存 edge reference。

Traffic target 使用 child-owned reference：

```text
Movement.junctionId
ManeuverPath.movementId
```

Core normalization 构造反向 member ranges，调用方不能提交第二份 child list。

### 3.2 ManeuverPath edge shape

```text
ManeuverPathInput
  externalId
  movementExternalId
  entryEdgeExternalId
  internalEdgeExternalIds[]
  exitEdgeExternalId
```

权威 edge sequence 为：

```text
[entryEdge] + internalEdges + [exitEdge]
```

因此：

- sequence 至少包含两条 edge；
- `internalEdges` 可以为空；
- 每个相邻 pair 都必须存在 LaneGraph directed connection；
- entry edge 表示进入 Junction 前的 approach；
- exit edge 表示离开 Junction 后的第一条 boundary edge；
- internal edge 表示由 Junction owner 管理的内部 traversal；
- path 不保存 centerline、world position、turn radius 或 lane width。

长期静态模型不全局禁止同一 Junction 内多个 path 共享 internal edge，也不禁止
有限 repeated edge。v0.9 protected intersection profile 由 #196 进一步限制为简单、
无 repeated internal edge、不会形成未建模 merge/conflict 的 path。

### 3.3 Derived internal-edge ownership

本文区分三种关系：

- storage ownership：LaneGraph 持有 LaneEdge，normalized topology registry 持有
  Junction、Movement 与 ManeuverPath definition；
- identity ownership：`Junction -> Movement -> ManeuverPath` parent hierarchy；
- semantic exclusive claim：Junction 对 internal edge role 的排他声明。

ManeuverPath 与 Route 只引用 EdgeHandle，不取得 LaneEdge storage ownership。
ManeuverPath 也不独占 internal edge；排他声明位于 Junction 层，因此同一 Junction
内的多个 path 可以共享 internal edge。

Normalization 从所有 ManeuverPath 派生：

```text
InternalEdgeOwner: EdgeHandle -> JunctionHandle
```

规则：

- 同一 internal edge 被同一 Junction 的多个 path 引用合法；
- 同一 internal edge 被不同 Junction 引用时返回错误；
- internal edge 不得作为任何其他 Junction 的 entry/exit boundary；
- entry/exit edge 可以同时是相邻 Junction 的 boundary，但不能在任一 Junction 中
  充当 internal edge；
- owner map 是 normalized derived index，不进入 Traffic wire。

对 `internalEdges=[]` 的 path，Junction identity 仍来自 owner hierarchy 和完整 path
definition，不要求虚构 connector edge。

### 3.4 Path sequence identity 与歧义

ManeuverPath 的业务 identity 是 external ID；edge sequence 是 traversal signature，
不是 external ID 的替代品。

- 同一 normalized topology registry 内，任意两条 ManeuverPath 的完整、已解析
  EdgeHandle sequence 必须不同，不论它们属于哪个 Junction 或 Movement；
- `internalEdges=[]` 的 direct path 同样受全局唯一性约束；
- 完全相同的物理 traversal 必须使用一个规范 ManeuverPath definition；车辆类别、
  cost、priority 或其他 policy 差异应由独立 overlay 表达，不能复制 path；
- 多条 path 可以共享 entry transition，并在后续 internal/exit edge 分叉。
- Route 必须通过完整连续 sequence 唯一匹配 path。
- 若同一 Route position 匹配零条或多条应当被建模为 Junction traversal 的 path，
  route registration 返回结构化错误。
- Core 不按 connector ID、edge prefix、转向字符串或 edge pair 猜测 path。

该全局规则是 traversal-definition coherence，不把 edge sequence 提升为 external
identity。它保证完整 sequence 到 ManeuverPath 至多一对一，同时保留同一
ManeuverPath 在有限循环 Route 的不同 `routeEdgeIndex` 上多次出现。

## 4. External ID、handle 与 registry

### 4.1 External ID

Junction、Movement、ManeuverPath 与 ManeuverGate external ID：

- 使用 current Traffic ASCII token 规则；
- case-sensitive；
- 不 trim、不 case fold、不做 Unicode normalization；
- 在各自 domain 内唯一；
- 不同 domain 可以复用同一字符串；
- 用于 JSON、authoring、validation、debug 与 resolver。

### 4.2 Typed handle

v0.9 target 新增：

```text
JunctionHandle
MovementHandle
ManeuverPathHandle
ManeuverGateHandle
```

它们：

- `Clone + Copy + Debug + Eq + Hash`；
- public 字段不暴露，调用方不能自行构造；
- 不实现具有业务含义的 `Ord`；
- 不持久化到 Traffic/Spatial/Manifest；
- 只在当前 CoreWorld/session caller contract 内使用；
- static registry immutable，因此 v0.9 不需要 generation lifecycle。

为保持与当前 Edge/StopLine/SignalGroup 等 static handle 一致，v0.9 不单独给新
handle 增加 world nonce。Foreign handle 不是合法输入；只读 query 无法解析时返回
`None`，修改/构造路径在可区分时返回结构化错误。不得声称所有 same-index
foreign handle 都可在运行时被检测。

### 4.3 Resolver

Core public target 至少提供：

```text
junction_handle(externalId) -> Option<JunctionHandle>
junction_external_id(handle) -> Option<&str>
movement_handle(externalId) -> Option<MovementHandle>
movement_external_id(handle) -> Option<&str>
maneuver_path_handle(externalId) -> Option<ManeuverPathHandle>
maneuver_path_external_id(handle) -> Option<&str>
maneuver_gate_handle(externalId) -> Option<ManeuverGateHandle>
maneuver_gate_external_id(handle) -> Option<&str>
```

并提供 normalization-order entity iteration、parent query、member iteration 和
borrowed path-edge iteration。Public API 不暴露内部 index/range。

### 4.4 Foreign-graph rebind

Normalized registry 不得把 dense EdgeHandle 或 parent handles 直接移植到另一张
LaneGraph/CoreWorld。

`InitialTrafficData` final assembly 必须：

1. 保留或恢复 external definitions；
2. 按 external ID 对最终 LaneGraph 重新解析 edge；
3. 重新分配 Junction/Movement/ManeuverPath/ManeuverGate handles；
4. 重建 owner/member/candidate indices；
5. 重做 path connectivity、StopLine/Gate 和 Route occurrence validation；
6. 只有全部成功后才提交可用 aggregate。

即使两张 graph 拥有相同 external IDs，也必须执行 rebind；handle 数值相同不能作为
跳过 validation 的证据。

## 5. Normalized storage 与 iteration

### 5.1 性能形状

不冻结 `Vec`、`IndexMap` 等具体 public implementation，但 production target 必须
保持以下等价形状：

```text
dense junction definitions
dense movement definitions
dense maneuver-path definitions
dense maneuver-gate definitions

flat movement handles + per-junction range
flat maneuver-path handles + per-movement range
flat edge handles + per-path range
```

实现可以采用 prefix counts/two-pass fill 构造 member ranges。要求：

- entity handles 按各自 input/normalization order 分配；
- parent member order 保持 child input order；
- path edge order严格保持 authoring order；
- normalization 可以使用临时 traversal-signature lookup 检查全局 path sequence
  coherence，但成功后不要求保留该 construction scratch；
- public iterator 创建不 clone/precollect；
- member/path-edge iteration 不做 heap allocation；
- hot path 不读取 external ID map。

### 5.2 Stable iteration

以下顺序可观察并必须测试：

- `junctions()`：Junction input order；
- `movements()`：Movement input order；
- `junction_movements(j)`：过滤后的 Movement input order；
- `maneuver_paths()`：ManeuverPath input order；
- `movement_maneuver_paths(m)`：过滤后的 ManeuverPath input order；
- `maneuver_path_edges(p)`：entry、internal input order、exit；
- `maneuver_gates()`：ManeuverGate input order。

Handle 数值不具有长期持久化意义，但同一 package bytes/同一 normalization path 必须
得到相同 handle allocation 与 iteration。

## 6. Route 与 Maneuver occurrence

### 6.1 Route authority 保持不变

Route 继续是有限显式 edge sequence：

- LaneGraph connection 验证每个相邻 pair；
- vehicle route cursor 继续由 `RouteHandle + routeEdgeIndex + edgeProgress` 表达；
- repeated edge 继续以 route occurrence/index 区分；
- Core 不自动插入、删除或重排 path edges；
- ManeuverPath 不承担 pathfinding 或 route selection。

### 6.2 Occurrence identity

概念 identity：

```text
(RouteHandle, entryRouteEdgeIndex, ManeuverPathHandle)
```

Normalized route-shared record：

```text
ManeuverOccurrence
  maneuverPath: ManeuverPathHandle
  entryRouteEdgeIndex: usize
  exitRouteEdgeIndex: usize
```

`exitRouteEdgeIndex` 指向完整 path sequence 中 exit edge 的 Route occurrence。

### 6.3 Route registration-time compilation

Topology registry 建立 entry-transition candidate index。Initial routes 和 runtime
`register_route` 使用相同 compiler：

1. 按 Route edge order 检查 LaneGraph connectivity；
2. 用当前 adjacent transition 查询少量 path candidates；
3. 对 candidate 比较完整 path edge sequence；
4. 选择唯一匹配并生成 `ManeuverOccurrence`；
5. 把 path 上适用的 ManeuverGate 编译为 `GateOccurrence`；
6. 校验 StopLine/Gate coverage、overlap 与 occurrence order；
7. 全部成功后原子提交 Route handle、definition 和 shared metadata。

概念 Gate record：

```text
GateOccurrence
  maneuverGate: ManeuverGateHandle
  routeTransitionIndex: usize
```

目标复杂度：

- topology normalization 与 total input size 线性；
- route compile 为 `O(route edge count + matched candidate work)`；
- 禁止 `O(route edge count * global path count)` 全 catalog scan；
- dynamic route 失败不留下部分 route/occurrence 或可观察 handle/order。

### 6.4 Steady-tick 使用

Vehicle step 只消费 route-shared compiled metadata 和当前 route cursor：

- 不重新匹配 path sequence；
- 不查 Junction/Movement/Path/Gate external ID；
- 不 hash external ID 或 path sequence；
- 不 clone path/gate vectors；
- 不为每辆车复制 occurrence catalog；
- 不因全局 Junction/path/gate 数量增加而改变无关车辆的 steady-tick 复杂度。

实现可使用 route transition-indexed metadata、sorted occurrences 或等价 compact
结构，但必须保留上述可观察复杂度和零 per-vehicle heap allocation。

## 7. ManeuverGate 与 Signals

### 7.1 一等 Gate model

Traffic/Core target：

```text
ManeuverGateInput
  externalId
  maneuverPathExternalId
  transitionIndex
  stopLineExternalId
  signalControl
```

Normalized：

```text
ManeuverGate
  handle
  maneuverPath: ManeuverPathHandle
  transitionIndex
  stopLine: StopLineHandle
  control: SignalControl
```

若 path edge sequence 长度为 `N`，合法 `transitionIndex` 范围为 `0..N-1` 的
transition index，即必须满足 `transitionIndex + 1 < N`。

Gate location：

```text
fromEdge = pathEdges[transitionIndex]
toEdge = pathEdges[transitionIndex + 1]
```

StopLine 必须绑定 `fromEdge`。Current StopLine 的 `edgeEnd` 语义继续成立。

### 7.2 v0.9 protected-entry profile

#196/#229 的最小 profile 额外要求：

- 所有生产 ManeuverGate 的 `transitionIndex == 0`；
- Gate crossing 为 `entryEdge -> first internal`，internal 为空时为
  `entryEdge -> exitEdge`；
- 每条受信号控制的 ManeuverPath 恰好一个 entry Gate；
- `signalControl:none` 仍只表示 signal layer 无约束；
- green/allow 不绕过 leader、speed limit、ParkingStop、RouteEnd、safe-speed 或
  no-overlap；
- Restrictive yellow/red 在 entry StopLine 形成 SignalStop；
- 已原子跨越 Gate 的车辆继续清空，不倒退回 StopLine。

### 7.3 Gate identity 与 coverage

- Gate external ID 在 Gate domain 内唯一。
- 同一 `(ManeuverPath, transitionIndex)` 至多一个 Gate。
- Pair `(fromEdge, toEdge)` 不再是 Gate 或 Movement identity。
- 多条 path 可以共享 entry pair；Route 通过完整 path occurrence 选择唯一 Gate。
- 每个 StopLine 控制且可被 Route 采用的 Junction entry traversal 必须属于至少一条
  ManeuverPath。
- 每个被 Route 选择的受控 path occurrence 必须解析到唯一 Gate。
- v0.9 corridor Route 如果绕过显式 path/Gate，在加载或 runtime route registration
  阶段拒绝。

### 7.4 Public observation

Signals query/state/event target 使用 `ManeuverGateHandle`，并可解析其
ManeuverPath、Movement 与 Junction。不得保留 pair-based public key/alias。

Adapter 可以：

- 遍历 Gate；
- query current aspect/signal-layer permission；
- 通过 path/parent resolver 绑定 debug 或 lamp presentation；
- 使用 external ID 输出诊断。

Adapter 不得：

- 从 connector 名称推断 Junction；
- 通过 edge pair 重建 Movement identity；
- 决定 conflict/right-of-way；
- 覆盖 Core permission 或 motion。

## 8. Authority 矩阵

| 事实/行为                          | Authority                                | 非 Authority                             |
| ---------------------------------- | ---------------------------------------- | ---------------------------------------- |
| Edge connection/length/speed limit | LaneGraph/Core                           | Junction、Spatial、Adapter               |
| Junction/Movement/Path identity    | Traffic input + Core normalized registry | connector naming、Adapter                |
| Internal-edge Junction owner       | Core derived topology index              | duplicate wire owner、Spatial            |
| Path traversal signature coherence | Core topology normalization              | Route discriminator、policy overlay      |
| Actual vehicle traversal           | Route + route cursor                     | ManeuverPath、SignalController           |
| Canonical geometry/pose            | Spatial edge binding                     | Core Junction polygon、Adapter inference |
| Signal phase/aspect                | SignalController/Group                   | Movement、Adapter                        |
| Gate signal permission             | Compliance + ManeuverGate                | SignalAspect alone、Adapter              |
| Conflict/right-of-way              | Future dedicated domain                  | SignalController、JunctionGroup          |
| Final motion/safety                | Core longitudinal/traversal pipeline     | green aspect、Adapter                    |
| Route/movement weights             | Scenario/authoring policy                | Core static topology                     |
| Host transform/rendering           | Adapter/Presentation                     | Core/Data                                |

## 9. Traffic v0.8 target

### 9.1 Wire shape

概念 JSON：

```json
{
  "formatVersion": "0.8",
  "laneGraph": {
    "edges": []
  },
  "junctions": [
    {
      "id": "junction-1"
    }
  ],
  "movements": [
    {
      "id": "movement-junction-1-west-to-south",
      "junctionId": "junction-1"
    }
  ],
  "maneuverPaths": [
    {
      "id": "path-junction-1-west-left-lane-1",
      "movementId": "movement-junction-1-west-to-south",
      "entryEdgeId": "edge-west-approach-lane-1",
      "internalEdgeIds": [
        "edge-junction-1-west-left-connector-1"
      ],
      "exitEdgeId": "edge-south-exit-lane-1"
    }
  ],
  "routes": [],
  "signals": {
    "stopLines": [],
    "maneuverGates": [
      {
        "id": "gate-junction-1-west-left-lane-1",
        "maneuverPathId": "path-junction-1-west-left-lane-1",
        "transitionIndex": 0,
        "stopLineId": "stop-line-junction-1-west-lane-1",
        "signalControl": {
          "kind": "group",
          "groupId": "group-junction-1-west-left"
        }
      }
    ],
    "groups": [],
    "controllers": []
  }
}
```

字段命名以 #229 schema 为准，但语义不得偏离：

- parent reference 只在 child 保存；
- Junction 不保存 duplicate movement list；
- Movement 不保存 duplicate path list；
- internal edge owner 不进入 wire；
- Gate 必须有独立 ID 和 transition index；
- handles、compiled occurrences 和 derived indices 不进入 wire。

### 9.2 版本与兼容

- Traffic target 直接从 current `0.7` 切到 `0.8`。
- Loader 只接受 exact `"0.8"`；不并行接受 `0.7`。
- 不提供 runtime migration、deprecated constructors 或 dual query。
- Source/schema/loader/Core/fixtures/generator/artifacts/catalog/docs/digests 同一可观察
  production 交付切换。
- 已发布 0.7 schema/bytes 继续 immutable，不得原地修改。

### 9.3 Spatial/Manifest

SpatialPackage 与 ScenarioManifest 保持 `0.1`：

- path geometry 继续由其引用 edges 的 Spatial centerline 表达；
- 不新增 Junction polygon、conflict polygon 或 turn radius；
- Spatial binding 仍以 Traffic edge ID、length 和 digest 配对；
- Traffic bytes 改变后 Manifest traffic size/digest 必须更新；
- Spatial bytes 未改变时不得为“版本对齐”伪造 shape 变更。

## 10. Validation 与 first-error

### 10.1 Data 与 Core 分层

- JSON syntax/schema/closed shape/type/range error 由 `laneflow-data` 报告。
- external ID、owner、edge reference、connectivity、Gate/StopLine、Route occurrence
  由 Core constructors/normalization 报告。
- loader 只做 wire 到 Core input 的转换，不复制 Core topology rules。
- 初始化失败不得返回部分可用 registry、Route 或 Signal aggregate。

### 10.2 Topology normalization phase order

同一 phase 内按 input order 返回首错：

1. Junction ID syntax/duplicate，并分配 handles；
2. Movement ID syntax/duplicate、unknown Junction 与 owner；
3. ManeuverPath ID syntax/duplicate、unknown Movement 与 owner；
4. 对每条 path 按 entry、internal input order、exit 解析首个 unknown edge；
5. 对每条 path 按 traversal order 返回首个 disconnected transition；
6. 按 ManeuverPath input order 检查完整 resolved EdgeHandle sequence 全局重复；
7. 派生 internal-edge owner，并返回首个 cross-Junction/internal-boundary conflict；
8. 按 Junction/Movement input order 校验 non-empty child cardinality；
9. 构造 dense definitions、flat member ranges、flat path-edge storage 与 candidate index。

任一错误不得发布部分 handles/registry。

全局 sequence 检查保留首个 definition；后续重复项返回概念结构化错误
`DuplicateManeuverPathSequence`，至少携带 first/duplicate path ID 与各自
Junction ID，具体 `CoreError` variant 由 #229 冻结。它位于 ownership 派生之前，
因此 zero-internal 与 nonzero-internal 的重复 traversal 得到相同错误，不被后者的
cross-Junction internal-edge conflict 抢先。

### 10.3 Signal/Gate phase order

在现有 StopLine、SignalGroup、Controller/Phase validation 后：

1. Gate ID syntax/duplicate；
2. unknown ManeuverPath；
3. transition index bounds；
4. duplicate `(path, transitionIndex)`；
5. unknown StopLine；
6. StopLine edge/path transition from-edge mismatch；
7. unknown SignalGroup / invalid signal control；
8. v0.9 profile 的 non-entry Gate 或 missing/duplicate protected entry Gate；
9. 构造 dense Gate storage、resolver 与 per-path Gate ranges。

### 10.4 Route compile phase order

每条 initial/dynamic Route：

1. current Route ID/edge/connectivity rules；
2. path candidate/full sequence match；
3. zero/multiple required Junction path match；
4. Maneuver occurrence overlap/order；
5. unique Gate occurrence 与 StopLine coverage；
6. 构造 route-shared metadata；
7. 原子发布 Route handle/definition/metadata。

具体 `CoreError` variant 和 data path 文案由 #229 冻结并测试；实现不得用容器迭代
偶然顺序改变 first-error。

## 11. Determinism

### 11.1 保证

同一：

- Traffic bytes；
- Core version；
- normalization path；
- dynamic Route command sequence；

必须得到相同：

- handle allocation；
- entity/member/path-edge iteration；
- first-error variant 与 attribution；
- Maneuver/Gate occurrence order；
- fixed-tick state/events。

### 11.2 输入 permutation

Input arrays 是 normalization order。Permutation 后：

- raw handle 数值允许改变；
- public normalization-order iterator 允许改变；
- first-error attribution 可以按新的 input order 改变；
- 按 external ID 对齐的实体关系、path edge sequence、Route occurrence 与运行结果必须
  语义等价，前提是输入本身合法且没有依赖顺序的重复/歧义。

测试不得把“语义等价”误写成“handle 或 iteration 完全相同”。

### 11.3 禁止的非确定性来源

- `HashMap` 未定义 iteration 直接形成 public order；
- handle 数值排序；
- 并行 normalization 的 completion order；
- connector/external ID substring 推断；
- vehicle tick 中按 external ID 临时排序；
- 多 candidate 错误使用容器偶然顺序选择。

## 12. 性能与内存

### 12.1 Static normalization

目标：

- 总体与 input entities、path edge references、Gate 数量线性；
- member/path edge 使用连续 storage；
- resolver/candidate map 只在初始化或 route command path 使用；
- 全局 traversal-signature 检查按 path input order 插入临时 lookup，使用完整
  sequence 作为 key，或使用 fingerprint 缩小候选后做完整 sequence equality；
- hash collision 不得构成相等，lookup iteration 不得形成 first-error 或 public order；
- 不为每个 parent/path 强制一个独立 heap object；
- 允许 construction scratch allocation，但成功后保持 compact immutable registry。

### 12.2 Route registration

- Candidate index 至少按 entry transition 缩小匹配范围。
- 不允许 route 的每个 edge position 扫描全部 paths。
- Compiled metadata 由 Route 共享，不按 vehicle 复制。
- Dynamic registration 允许与 remaining route length 成正比的 bounded work。
- Remove Route 时 occurrence metadata 与 Route definition 同生命周期释放。

### 12.3 Steady tick

无 Junction/Gate 或 Route 无 occurrence 时必须有 fast reject。启用时：

- 不查 external ID；
- 不匹配 path sequence；
- 不扫描全局 Junction/path/Gate registry；
- 不创建临时集合；
- 不做 per-vehicle heap allocation；
- signal/route constraint 只访问 route-shared compact metadata 和 normalized handles；
- 空/static registry 不改变无关 vehicle 的算法阶数。

### 12.4 #229 验收

#229 至少提供：

- 10k fixed-tick functional/performance smoke；
- 100k scaling observation；
- empty topology、无 occurrence、dense protected occurrences 的对照；
- allocation instrumentation，证明 steady tick 无新增 per-vehicle 临时分配；
- catalog 扩大但车辆 Route 不变时，steady tick 不随 catalog 线性增长；
- 同机受控 baseline/regression 记录，遵守 current performance design 的阻断阈值。

本文不为 docs-only #228 运行 runtime benchmark。

## 13. v0.9 protected-turning profile 输入

#196 必须在本通用模型之上冻结具体 corridor profile，至少包括：

### 13.1 Identity 与 topology

- 两个 Junction external IDs；
- 每个入口的 left/straight/right Movement IDs，不含 U-turn；
- 每个 lane-level ManeuverPath ID、parent Movement 和完整 edge sequence；
- internal connector edge 的 Junction owner；
- 每条 Route 的完整 Maneuver occurrence；
- 每个受控 path 的 ManeuverGate ID、StopLine 与 SignalGroup。

### 13.2 Geometry 与 lane assignment

- entry/internal/exit edge 的 Spatial polylines、length binding 和 pose 验证；
- 入口车道用途和目标 exit lane；
- 同时开放 paths 不发生几何 crossing；
- 同时开放 paths 不在进入 shared downstream edge 前发生未建模 merge；
- 同时开放 paths 不抢占同一 exit lane；
- current Following/no-overlap 开始生效前，authoring 已排除跨 branch 碰撞。

### 13.3 Signal matrix

- 每个 phase 完整列出全部 group aspect；
- path/Gate 到 group 的显式映射；
- compatibility matrix 覆盖全部同时 green/protected 集合；
- yellow/all-red/clearance 与 v0.8 fixed-time authority 一致；
- 不以 `signalControl:none` 表达“无条件 right-of-way”；
- 不在 SignalController 写 turn/country/facility if/else。

### 13.4 Scenario 与 Route catalog

- portal/lane/Movement/Route catalog cross-reference；
- seeded selection 权重；
- population/recycle policy 仍由 `laneflow-scenario` caller-owned；
- blocked retry 不消费新 draw；
- Route order/catalog order 的确定性影响；
- 50/100/200 三档和所有转向 coverage。

若 #196 无法用 authoring matrix 证明上述安全前提，必须回到 G1 拆分 ConflictZone /
merge arbitration，不能让 #229 通过 private heuristic 补洞。

## 14. Core/Data/Adapter 影响矩阵

| 层                 | Target 影响                                            | #228 production 变更 | 后续 owner     |
| ------------------ | ------------------------------------------------------ | -------------------- | -------------- |
| Core API           | 新增四类 handle/resolver/registry；Gate query breaking | 无                   | #229           |
| LaneGraph          | 保持 edge authority；增加 derived Junction owner index | 无                   | #229           |
| Route              | 注册期编译 Maneuver/Gate occurrences                   | 无                   | #229           |
| Signals            | `MovementGate` clean-break 为 `ManeuverGate`           | 无                   | #229           |
| Traffic Data       | exact-current `0.8` 新 arrays/fields                   | 无                   | #229           |
| Spatial            | shape 保持 `0.1`；继续 edge binding                    | 无                   | #196/#229 验证 |
| Manifest           | shape 保持 `0.1`；更新 Traffic size/digest             | 无                   | #229           |
| Scenario policy    | Route selection authority 不变                         | 无                   | #190/#191      |
| Generator          | 移除 connector-name inference，显式生成 membership     | 无                   | #229           |
| Fixtures/artifacts | 原子切换 v0.8 并更新 canonical bytes/digests           | 无                   | #229           |
| Adapter            | observation 迁移到 Gate/Path handles；authority 不变   | 无                   | #229/#190      |
| Presentation       | 可显示 Junction/Movement/Path/Gate，非规则 owner       | 无                   | #190           |

## 15. #229 最小生产实现输入

#229 必须作为单一可观察迁移交付：

1. 新 Core inputs、handles、dense registries、resolvers 与 borrowed iterators；
2. owner/cardinality/connectivity/first-error/foreign-rebind validation；
3. flat member/path-edge storage 和 entry-transition candidate index；
4. Route initial/dynamic registration occurrence compiler 与 atomic failure；
5. `ManeuverGate` static/current Signals query、constraint/traversal integration；
6. Traffic v0.8 schema/private DTO/loader/fixtures；
7. corridor generator 显式 membership，删除 connector-name inference；
8. generated Traffic artifacts、Manifest digest、catalog/publication/docs；
9. Core/Data/round-trip/property/determinism/performance tests；
10. current consumers、Bevy example 与 Adapter observation 的编译迁移。

### 15.1 不得出现的中间主干状态

- schema 已是 0.8、loader 仍只认识 0.7；
- Core 已删除 pair key、Data 仍生成 `movementGates`；
- generator 已生成新 arrays、canonical fixture 未更新；
- Traffic bytes 已变、Manifest digest 未更新；
- new Gate 已存在、Route 仍按 edge pair 绕过 path；
- old/new Gate API 同时公开；
- current docs 声称 0.8 已实现但 production loader 尚未切换。

如 schema publication 需要 main commit provenance，可按 ADR 0011 使用 Related source
PR + final Delivery PR 的两阶段流程，但 Issue/PR 必须显式记录，且 source-only main
不得被文档误写为已公开下载的 current schema。

## 16. 测试矩阵

### 16.1 Identity/owner

- duplicate/invalid Junction、Movement、Path、Gate IDs；
- unknown parent；
- empty Junction/Movement；
- one parent per child；
- same-Junction shared internal edge；
- cross-Junction internal conflict；
- internal/boundary role conflict；
- zero-internal path；
- repeated internal edge 的通用静态语义与 v0.9 profile 拒绝。

### 16.2 Path/Route

- unknown/disconnected entry/internal/exit edge；
- same-Junction identical path sequence；
- cross-Junction zero-internal identical path sequence；
- cross-Junction nonzero-internal identical path sequence，且 duplicate-sequence error
  先于 internal-edge owner conflict；
- duplicate error 稳定归因于 input order 中较后的 path，并引用首个 definition；
- 相邻 Junction 的 `[A, B]` 与 `[B, C]` sequence 合法；
- shared entry transition 后分叉；
- zero/multiple full match；
- repeated Route edge，以及同一 ManeuverPath 在不同 occurrence index 重复出现；
- finite loop；
- initial/dynamic Route 同规则；
- failed dynamic registration authority equality；
- remove Route 同时清理 shared occurrence metadata。

### 16.3 Gate/Signals

- unknown path/StopLine/group；
- transition bounds；
- StopLine from-edge mismatch；
- duplicate `(path, transitionIndex)`；
- v0.9 non-entry Gate；
- missing/duplicate protected Gate；
- restrictive stop、green release、crossed-Gate clearance；
- pair collision 不再影响 path-specific Gate；
- query/event 使用 handles，不携带 hot-path strings。

### 16.4 Data/round-trip/rebind

- exact 0.8，0.7/future version 拒绝；
- closed schema 和最窄 JSON path；
- canonical round-trip；
- foreign graph 相同/不同 external ID order 的 rebind；
- input permutation 后 external-ID-aligned semantic equality；
- historical published bytes 不变；
- generator deterministic regeneration 和 Manifest digest。

### 16.5 Performance

- flat storage/member/path iterator 零分配；
- route compiler candidate index 无全 catalog scan；
- empty/no-occurrence fast path；
- 10k/100k scaling；
- steady tick allocation；
- catalog-scale independence；
- input/command replay determinism。

## 17. Future extension boundary

### 17.1 RoadSection 与 lane change

未来 production RoadSection 必须单独冻结：

- section boundary；
- ordered lane membership；
- LaneEdge chain 与 section 的关系；
- Movement from/to section endpoints；
- lane adjacency、lane-change plan 与 resolved lane plan；
- schema migration 和性能影响。

不得在 #229 以未验证 string placeholder 代替该设计。

### 17.2 Multi-stage Gate 与 waiting zone

`ManeuverGate.transitionIndex` 已提供 identity 位置。未来仍需独立 G1 冻结：

- multiple Gate order；
- WaitingZone geometry/state；
- vehicle committed/clearing state；
- policy 与 conflict interaction；
- SignalStop attribution/event order。

### 17.3 Conflict/priority

Future ConflictZone/right-of-way domain 可以引用 Junction、Movement、ManeuverPath 与
Gate handles，但不得：

- 把 conflict state 写入 SignalController；
- 让 Adapter 裁决；
- 绕过 Core safety；
- 让 JunctionGroup 自动成为全设施 solver。

### 17.4 Complex facilities

- Merge/diverge：独立 gap/priority/lane availability G1。
- Roundabout：多个 Junction + JunctionGroup + circulating priority。
- Parking access：复用 LaneGraph/Route/Parking，另加 access Movement/profile。
- Interchange：多个 Junction/RoadSection/Group，不建立单一巨大 conflict unit。
- City partition：与 #72 Related，不阻塞 v0.9，不改变 handle/owner 事实。

## 18. G1 冻结结论

本设计接受：

- `Junction -> Movement -> ManeuverPath` 唯一 owner；
- LaneGraph edge authority 与 derived internal-edge Junction ownership；
- normalized registry 内全局唯一的 ManeuverPath traversal signature；
- Route 作为实际 traversal authority；
- 一等 `ManeuverGate` 与 transition-index location；
- RoadSection/LaneGroup/JunctionGroup 的延后生产化；
- Traffic v0.8 clean break；
- flat registry、route compile 与 steady-tick no-scan/no-allocation；
- protected-only authoring safety 与 future conflict domain 分离。

若 #196/#229 发现必须改变 owner 层级、Gate identity、Route authority、data version、
Spatial shape、conflict owner 或 steady-tick complexity，必须回到 #228/ADR 0017 更新
G1 或拆 follow-up；不得通过 private implementation 静默偏离。
