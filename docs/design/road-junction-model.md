# Road / Junction / Maneuver 静态模型

**文档状态**: Accepted<br>
**最后更新**: 2026-11-09<br>
**适用范围**: 长期 Road/Junction/Maneuver 分层、最小静态产品 profile、ManeuverGate、Route occurrence、确定性与性能边界<br>

**关联文档**:

- `../architecture.md`
- `../roadmap.md`
- `../adr/0003-runtime-tick-and-determinism.md`
- `../adr/0005-core-identity-and-handle-model.md`
- `../adr/0008-pre-1.0-data-format-version-policy.md`
- `../adr/0009-signal-indication-gate-and-policy-separation.md`
- `../adr/0013-engine-neutral-spatial-geometry-and-length-authority.md`
- `../adr/0017-static-road-junction-maneuver-and-gate-identity.md`
- `../adr/0018-multimodal-cross-section-and-access-overlay.md`
- `lane-graph.md`
- `cross-section-access.md`
- `route-system.md`
- `signal-system.md`
- `vehicle-following.md`
- `spatial-geometry.md`
- `example-scenarios.md`
- `signalized-corridor-protected-turning.md`

## 1. 目标、状态与非目标

### 1.1 目标

本文冻结：

- RoadSection、LaneGroup、Junction、Movement、ManeuverPath 与 JunctionGroup 的长期
  职责和关系；
- 已生产化的 `Junction -> Movement -> ManeuverPath` 最小静态层级；
- LaneGraph、Route、ManeuverPath、ManeuverGate、Signals 与 Spatial 的 authority；
- 稳定标识、修订内序号与 first-error；
- route registration-time occurrence compilation 与 steady-tick 性能约束；
- #196 已冻结的 protected-turning profile 和安全证明边界。

### 1.2 当前 production

| 范围              | Current production                                                                              |
| ----------------- | ----------------------------------------------------------------------------------------------- |
| Traffic           | LFCA 制品构建 `SharedNetworkRevision`                                                           |
| Lane topology     | LaneEdge + directed connection + Junction hierarchy                                             |
| Route             | 显式有限 edge sequence，注册期编译 occurrences                                                  |
| Gate              | 一等 `ManeuverGateOrdinal`（共享路网修订内序号）                                                |
| Junction identity | 稳定标识与修订内 typed ordinal 经冷双向索引互查（§4）                                           |
| Spatial           | 折线 edge centerline 经 `SharedSpatialNetwork` / `SpatialSession` 消费（`spatial-geometry.md`） |
| Conflict          | authoring protected phases；无 solver                                                           |

### 1.3 非目标

- 不实现 dynamic road topology、runtime Junction lifecycle 或 in-place mutation。
- 不实现 pathfinding、route planner、lane selection、lane change 或 overtaking。
- 不实现 ConflictZone、priority、gap acceptance、reservation 或 permissive turn。
- 不实现 roundabout、parking access、merge/diverge behavior 或 interchange。
- 不把 `left/straight/right`、国家法规或设施类型写进 SignalController。
- 不冻结道路 mesh、Junction polygon、lane width 或世界坐标。
- 不生产化 RoadSection、LaneGroup 或 JunctionGroup。
- 不提供 pair-based Gate 兼容、双 schema 或 deprecated API alias。

## 2. 术语与长期分层

### 2.1 LaneEdge

LaneEdge 是 LaneGraph 中有方向、可遍历的最小拓扑与纵向进度单元。它继续拥有：

- 稳定标识与 `LaneEdgeOrdinal`；
- length、speed limit；
- directed next-edge connections；
- vehicle route cursor 和 edge-local progress 的引用目标。

LaneEdge 不自动等于道路、Junction、Movement 或 ManeuverPath。

### 2.2 RoadSection

RoadSection 是有方向的道路结构分段，用于表达：

- 稳定的道路级上游/下游边界；
- 同一方向的 ordered lanes；
- 共享道路属性；
- lane adjacency / lane change 的结构 owner。

RoadSection 不是 Route，不决定车辆当前 path，也不复制 LaneEdge 的 length 或
connection。其生产语义（ordered lanes、edge 链、单 section 归属、与横断面 owner
RoadCorridor 的关系）已由 #234 在
[`cross-section-access.md`][cross-section-access] §3.2 冻结；Traffic Runtime registry、
handle 与 Traffic array 仍待其最小 production Issue 实现。

### 2.3 LaneGroup

LaneGroup 是 RoadSection 内可选的命名分组，可用于表达一组共享用途的 lanes。
它不是 Traffic Runtime root entity，但 #234 已在
[`cross-section-access.md`][cross-section-access] §3.3 冻结其生产语义：
一等 external ID/handle、child-owned 归属恰好一个 RoadSection、不影响 lane
顺序；lane adjacency 的唯一事实来源仍是 RoadSection 的 lane index。如何进入
lane-change policy 由 #237 在该 SSOT 上冻结。

### 2.4 Junction

Junction 是通用的最小拓扑连接/潜在冲突单元：

- 平面十字或丁字交叉可以是 Junction；
- merge、diverge、roundabout 的单个连接节点可以是 Junction；
- parking access 或 interchange 可以由多个 Junction 组成；
- 普通 geometry split、portal 或无连接的立体交叉不自动成为 Junction。

`Intersection` 只表示平面道路交叉 profile/authoring 语义，不成为 Traffic Runtime 根实体。

### 2.5 Movement

Movement 是一个 Junction 内的道路级通行意图和稳定分组 identity。它聚合同一
意图的一个或多个 lane-level ManeuverPath。

Traffic Runtime 不内置 `left | straight | right | uTurn` 枚举。转向标签、法规分类、route
权重或 UI 名称属于 authoring/profile metadata。RoadSection 尚未生产化期间，
Movement 的道路级 from/to endpoints 是由 authoring 保证的语义；Traffic Runtime 通过 child
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

当前不引入 JunctionGroup 序号、registry 或 schema。

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

来源使用 child-owned reference：

```text
Movement.junctionId
ManeuverPath.movementId
```

编译器构造反向 member ranges，来源不能提交第二份 child list。

### 3.2 ManeuverPath edge shape

```text
ManeuverPathInput
  externalId
  movementExternalId
  entryEdgeExternalId
  internalEdgeExternalIds[]
  exitEdgeExternalId
```

正本 edge sequence 为：

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

ManeuverPath 与 Route 只引用 `LaneEdgeOrdinal`，不取得 LaneEdge storage ownership。
ManeuverPath 也不独占 internal edge；排他声明位于 Junction 层，因此同一 Junction
内的多个 path 可以共享 internal edge。

Normalization 从所有 ManeuverPath 派生：

```text
InternalEdgeOwner: LaneEdgeOrdinal -> JunctionOrdinal
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
  `LaneEdgeOrdinal` sequence 必须不同，不论它们属于哪个 Junction 或 Movement；
- `internalEdges=[]` 的 direct path 同样受全局唯一性约束；
- 完全相同的物理 traversal 必须使用一个规范 ManeuverPath definition；车辆类别、
  cost、priority 或其他 policy 差异应由独立 overlay 表达，不能复制 path；
- 多条 path 可以共享 entry transition，并在后续 internal/exit edge 分叉。
- Route 必须通过完整连续 sequence 唯一匹配 path。
- 若同一 Route position 匹配零条或多条应当被建模为 Junction traversal 的 path，
  route registration 返回结构化错误。
- Traffic Runtime 不按 connector ID、edge prefix、转向字符串或 edge pair 猜测 path。

该全局规则是 traversal-definition coherence，不把 edge sequence 提升为 external
identity。它保证完整 sequence 到 ManeuverPath 至多一对一，同时保留同一
ManeuverPath 在有限循环 Route 的不同 `routeEdgeIndex` 上多次出现。

## 4. 稳定标识与修订内序号

路口静态对象以共享路网修订内的有类型序号标识（`JunctionOrdinal`、`MovementOrdinal`、
`ManeuverPathOrdinal`、`ManeuverGateOrdinal`），经修订的关系查询访问（如
`junction_movements`、`movement_paths`）；稳定标识与序号经修订内的冷双向索引互查。
稳定标识的派生见 `network-compiler.md`，序号的分层规则见 `core-id-handles.md`。

序号不暴露内部布局，不实现具有业务含义的排序，只在所属修订内有效；静态表不可变，
因此不需要 generation 生命周期。

## 5. Normalized storage 与 iteration

### 5.1 性能形状

不冻结 `Vec`、`IndexMap` 等具体 public implementation，但 production target 必须
保持以下等价形状：

```text
dense junction definitions
dense movement definitions
dense maneuver-path definitions
dense maneuver-gate definitions

flat movement ordinals + per-junction range
flat maneuver-path ordinals + per-movement range
flat edge ordinals + per-path range
```

实现可以采用 prefix counts/two-pass fill 构造 member ranges。要求：

- 实体序号按各自 normalization order 分配；
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

序号数值不具有跨修订意义，但同一来源字节/同一 normalization path 必须
得到相同序号分配与 iteration。

## 6. Route 与 Maneuver occurrence

### 6.1 Route authority 保持不变

Route 继续是有限显式 edge sequence：

- LaneGraph connection 验证每个相邻 pair；
- vehicle route cursor 继续由 `RouteHandle + routeEdgeIndex + edgeProgress` 表达；
- repeated edge 继续以 route occurrence/index 区分；
- Traffic Runtime 不自动插入、删除或重排 path edges；
- ManeuverPath 不承担 pathfinding 或 route selection。

### 6.2 Occurrence identity

概念 identity：

```text
(RouteHandle, entryRouteEdgeIndex, ManeuverPathOrdinal)
```

Normalized route-shared record：

```text
ManeuverOccurrence
  maneuverPath: ManeuverPathOrdinal
  entryRouteEdgeIndex: usize
  exitRouteEdgeIndex: usize
```

`exitRouteEdgeIndex` 指向完整 path sequence 中 exit edge 的 Route occurrence。

### 6.3 Route registration-time compilation

Topology registry 建立 entry-transition candidate index。唯一生产入口是 runtime
`register_route`（ADR 0029）；compiler 如下：

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
  maneuverGate: ManeuverGateOrdinal
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

来源形状：

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
  ordinal: ManeuverGateOrdinal
  maneuverPath: ManeuverPathOrdinal
  transitionIndex
  stopLine: StopLineOrdinal
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

### 7.2 protected-entry profile 与多 Gate

protected-entry profile 额外要求：

- 所有生产 ManeuverGate 的 `transitionIndex == 0`；
- Gate crossing 为 `entryEdge -> first internal`，internal 为空时为
  `entryEdge -> exitEdge`；
- 每条受信号控制的 ManeuverPath 恰好一个 entry Gate；
- `signalControl:none` 仍只表示 signal layer 无约束；
- green/allow 不绕过 leader、speed limit、ParkingStop、RouteEnd、safe-speed 或
  no-overlap；
- Restrictive yellow/red 在 entry StopLine 形成 SignalStop；
- 已原子跨越 Gate 的车辆继续清空，不倒退回 StopLine。

在 canonical protected corridor profile 之外，同一路径可在不同 transition 声明多个 Gate，
path query 与 Route occurrence 按 transition 顺序编译。internal Gate 的 StopLine
由 Gate usage 建立 coverage，不被误当作新的 Junction entry。

### 7.3 Gate identity 与 coverage

- Gate external ID 在 Gate domain 内唯一。
- 同一 `(ManeuverPath, transitionIndex)` 至多一个 Gate。
- Pair `(fromEdge, toEdge)` 不再是 Gate 或 Movement identity。
- 多条 path 可以共享 entry pair；Route 通过完整 path occurrence 选择唯一 Gate。
- 每个 StopLine 控制且可被 Route 采用的 Junction entry traversal 必须属于至少一条
  ManeuverPath。
- 每个被 Route 选择的受控 path occurrence 必须解析到唯一 Gate。
- Route 如果绕过显式 path/Gate，在 `register_route` 阶段拒绝。

### 7.4 Public observation

Signals 查询使用 `ManeuverGateOrdinal`，并可解析其
ManeuverPath、Movement 与 Junction。不得保留 pair-based public key/alias。

Adapter 可以：

- 遍历 Gate；
- query current aspect/signal-layer permission；
- 通过 path/parent 关系查询绑定 debug 或 lamp presentation；
- 使用 external ID 输出诊断。

Adapter 不得：

- 从 connector 名称推断 Junction；
- 通过 edge pair 重建 Movement identity；
- 决定 conflict/right-of-way；
- 覆盖 Traffic Runtime permission 或 motion。

## 8. Authority 矩阵

| 事实/行为                          | Authority                                       | 非 Authority                                        |
| ---------------------------------- | ----------------------------------------------- | --------------------------------------------------- |
| Edge connection/length/speed limit | LaneGraph/Runtime                               | Junction、Spatial、Adapter                          |
| Junction/Movement/Path identity    | 来源声明 + 编译器规范化登记表                   | connector naming、Adapter                           |
| Internal-edge Junction owner       | 编译器派生拓扑索引                              | duplicate wire owner、Spatial                       |
| Path traversal signature coherence | 编译器拓扑规范化                                | Route discriminator、policy overlay                 |
| Actual vehicle traversal           | Route + route cursor                            | ManeuverPath、SignalController                      |
| Canonical geometry/pose            | Spatial edge binding                            | Traffic Runtime Junction polygon、Adapter inference |
| Signal phase/aspect                | SignalController/Group                          | Movement、Adapter                                   |
| Gate signal permission             | Compliance + ManeuverGate                       | SignalAspect alone、Adapter                         |
| Conflict/right-of-way              | Future dedicated domain                         | SignalController、JunctionGroup                     |
| Final motion/safety                | Traffic Runtime longitudinal/traversal pipeline | green aspect、Adapter                               |
| Route/movement weights             | Scenario/authoring policy                       | Traffic Runtime static topology                     |
| Host transform/rendering           | Adapter/Presentation                            | Runtime/Spatial                                     |

## 9. 来源与制品边界

来源语言与 LFCA 的具体字段以各自合同为准（`road-editing-source-and-geometry-frontend.md`、
`portable-canonical-artifact.md`），语义不得偏离：

- parent reference 只在 child 保存；
- Junction 不保存 duplicate movement list；
- Movement 不保存 duplicate path list；
- internal edge owner 不进入来源；
- Gate 必须有独立标识和 transition index；
- 句柄、路线 occurrence 和派生索引不进入来源；
- path geometry 由其引用 edges 的 Spatial centerline 表达，不新增 Junction polygon、
  conflict polygon 或 turn radius。

## 10. Validation 与 first-error

### 10.1 来源、编译器与 Runtime 分层

- 来源字节、形状与类型错误由道路编辑读取器或合成领域专用语言前端报告；
- owner、edge reference、connectivity、Gate/StopLine 等静态不变量由编译器诊断报告，
  共享静态路网构建只做受检输入的结构核对，不复制编译器拓扑规则；
- Route occurrence 由 `TrafficWorld` 路线注册报告；
- 失败不得返回部分可用 registry、Route 或 Signal aggregate。

### 10.2 Topology normalization phase order

同一 phase 内按 input order 返回首错：

1. Junction ID syntax/duplicate，并分配序号；
2. Movement ID syntax/duplicate、unknown Junction 与 owner；
3. ManeuverPath ID syntax/duplicate、unknown Movement 与 owner；
4. 对每条 path 按 entry、internal input order、exit 解析首个 unknown edge；
5. 对每条 path 按 traversal order 返回首个 disconnected transition；
6. 按 ManeuverPath input order 检查完整 resolved edge sequence 全局重复；
7. 派生 internal-edge owner，并返回首个 cross-Junction/internal-boundary conflict；
8. 按 Junction/Movement input order 校验 non-empty child cardinality；
9. 构造 dense definitions、flat member ranges、flat path-edge storage 与 candidate index。

任一错误不得发布部分序号或登记表。

全局 sequence 检查保留首个 definition；后续重复项返回结构化诊断
`DuplicateManeuverPathSequence`，携带 first/duplicate path 与各自 Junction 的稳定键。它位于 ownership 派生之前，
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
9. 构造 dense Gate storage 与 per-path Gate ranges。

### 10.4 Route compile phase order

每条经 `register_route` 提交的路线：

1. Route edge/connectivity rules；
2. path candidate/full sequence match；
3. zero/multiple required Junction path match；
4. Maneuver occurrence overlap/order；
5. unique Gate occurrence 与 StopLine coverage；
6. 构造 route-shared metadata；
7. 原子发布 Route handle/definition/metadata。

实现不得用容器迭代偶然顺序改变 first-error。

## 11. Determinism

### 11.1 保证

同一：

- 受检 LFCA 字节；
- Traffic Runtime version；
- normalization path；
- `register_route` 命令序列；

必须得到相同：

- 序号与句柄分配；
- entity/member/path-edge iteration；
- first-error variant 与 attribution；
- Maneuver/Gate occurrence order；
- fixed-tick state/events。

### 11.2 输入 permutation

Input arrays 是 normalization order。Permutation 后：

- 序号数值允许改变；
- public normalization-order iterator 允许改变；
- first-error attribution 可以按新的 input order 改变；
- 按 external ID 对齐的实体关系、path edge sequence、Route occurrence 与运行结果必须
  语义等价，前提是输入本身合法且没有依赖顺序的重复/歧义。

测试不得把“语义等价”误写成“序号或 iteration 完全相同”。

### 11.3 禁止的非确定性来源

- `HashMap` 未定义 iteration 直接形成 public order；
- 序号或句柄数值排序；
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
- signal/route constraint 只访问 route-shared compact metadata 和修订内序号；
- 空/static registry 不改变无关 vehicle 的算法阶数。

## 13. v0.9 protected-turning profile

#196 已在
[`signalized-corridor-protected-turning.md`][signalized-corridor-protected-turning]
接受具体 corridor profile。该文档是场景级 SSOT；本文继续是
owner、identity、normalization、Route occurrence 和 performance contract 的通用
SSOT。具体 profile 已冻结以下输入：

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

已接受 profile 的 32 条 path 在四个 group 内通过 authoring compatibility 检查，
默认 yellow + all-red 为 4 秒。但 production 仍必须以规范人口和 seeds 证明 phase
handoff 时前一 active set 的 internal edges 已清空。若该测试失败，必须重新打开
#196 G1 并拆分 Junction entry capacity/clearance；不能让 #229 通过 private
heuristic、放宽测试或隐式 ConflictZone 补洞。

## 14. 测试矩阵

### 14.1 Identity/owner

- duplicate/invalid Junction、Movement、Path、Gate IDs；
- unknown parent；
- empty Junction/Movement；
- one parent per child；
- same-Junction shared internal edge；
- cross-Junction internal conflict；
- internal/boundary role conflict；
- zero-internal path；
- repeated internal edge 的通用静态语义与 v0.9 profile 拒绝。

### 14.2 Path/Route

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
- `register_route` 同规则；
- failed registration authority equality；
- remove Route 同时清理 shared occurrence metadata。

### 14.3 Gate/Signals

- unknown path/StopLine/group；
- transition bounds；
- StopLine from-edge mismatch；
- duplicate `(path, transitionIndex)`；
- v0.9 non-entry Gate；
- missing/duplicate protected Gate；
- restrictive stop、green release、crossed-Gate clearance；
- pair collision 不再影响 path-specific Gate；
- 查询使用序号，不携带 hot-path strings。

### 14.4 来源与修订

- 来源版本、字段闭合与诊断路径由各前端测试；
- input permutation 后按稳定标识对齐的 semantic equality；
- 同一来源字节的确定性再生。

### 14.5 Performance

- flat storage/member/path iterator 零分配；
- route compiler candidate index 无全 catalog scan；
- empty/no-occurrence fast path；
- 一万/十万 scaling；
- steady tick allocation；
- catalog-scale independence；
- input/command replay determinism。

## 15. Future extension boundary

### 15.1 RoadSection 与 lane change

RoadSection 的生产语义（section 的 ordered lane membership、LaneEdge chain 与
section 的关系、schema 版本边界）已由 #234 在
[`cross-section-access.md`][cross-section-access] 冻结，其最小 production
Issue 负责实现。以下各项仍需独立冻结：

- Movement from/to section endpoints；
- lane adjacency、lane-change plan 与 resolved lane plan（#237，消费 #234 SSOT）；
- 横向几何（宽度、偏移）与性能影响。

不得以未验证 string placeholder 代替该设计。

### 15.2 Multi-stage Gate 与 waiting zone

`ManeuverGate.transitionIndex` 已提供 identity 位置。#235 已在
[`waiting-zone-conflict-right-of-way.md`][waiting-zone-conflict-right-of-way]
完成独立 G1 接受：同一路径 Gate 按 transitionIndex 排序，Route 注册期编译
Gate/Waiting/Conflict occurrences；WaitingZone 由同一路径 entry/release Gate
有界，Traffic Runtime 拥有容量/队列与 committed/waiting/clearing 状态，Spatial
只拥有可选 3D region。Conflict approach 必须从 current route cursor 覆盖 horizon
内的 upcoming/repeated occurrence，把 Route-specific contributions 归约到每个
static `(ConflictZone, ParticipantStream)` 的 distinct-owner frontier 并排除 exact
subject；grant 前还必须证明车尾可清空 coverage，并原子取得
Waiting/Conflict/physical downstream claim。#281 已生产化 multi-Gate identity、
WaitingZone static registry、Route Gate/Waiting occurrence 与绑定期 capability
guards；Waiting capacity/queue/traversal runtime 仍由 #282 交付，Conflict/policy
部分仍由 #283/#284 交付。

### 15.3 Conflict/priority

#235 Accepted 设计把 ConflictZone/ParticipantStream 放入独立 ConflictRegistry，并由 Traffic Runtime
ConflictArbiter 产生 vehicle-specific grant/reservation。它可以引用 Junction、
ManeuverPath 与 Gate 序号，但不得：

- 把 conflict state 写入 SignalController；
- 让 Adapter 裁决；
- 绕过 Traffic Runtime safety；
- 让 JunctionGroup 自动成为全设施 solver。

### 15.4 Complex facilities

- Merge/diverge：独立 gap/priority/lane availability G1。
- Roundabout：多个 Junction + JunctionGroup + circulating priority。
- Parking access：复用 LaneGraph/Route/Parking，另加 access Movement/profile。
- Interchange：多个 Junction/RoadSection/Group，不建立单一巨大 conflict unit。
- City partition：与 #72 Related，不阻塞 v0.9，不改变 handle/owner 事实。

## 16. G1 冻结结论

本设计接受：

- `Junction -> Movement -> ManeuverPath` 唯一 owner；
- LaneGraph edge authority 与 derived internal-edge Junction ownership；
- normalized registry 内全局唯一的 ManeuverPath traversal signature；
- Route 作为实际 traversal authority；
- 一等 `ManeuverGate` 与 transition-index location；
- RoadSection/LaneGroup/JunctionGroup 的延后生产化；
- flat registry、route compile 与 steady-tick no-scan/no-allocation；
- protected-only authoring safety 与 future conflict domain 分离。

若后续发现必须改变 owner 层级、Gate identity、Route authority、数据版本、
Spatial shape、conflict owner 或 steady-tick complexity，必须回到 #228/ADR 0017 更新
G1 或拆 follow-up；不得通过 private implementation 静默偏离。

[cross-section-access]: cross-section-access.md
[signalized-corridor-protected-turning]: signalized-corridor-protected-turning.md
[waiting-zone-conflict-right-of-way]: waiting-zone-conflict-right-of-way.md
