# ADR 0017：静态 Road/Junction/Maneuver 与 Gate 身份

**状态**: Accepted<br>
**日期**: 2026-07-24<br>
**适用范围**: LaneFlow 的 Junction、Movement、ManeuverPath、RoadSection、JunctionGroup、ManeuverGate、Route occurrence 与 v0.9 最小静态生产化边界<br>
**关联文档**:

- 上游决策:
  - `0001-project-scope.md`
  - `0003-runtime-tick-and-determinism.md`
  - `0005-core-identity-and-handle-model.md`
  - `0007-traffic-data-crate-and-loader-boundary.md`
  - `0008-pre-1.0-data-format-version-policy.md`
  - `0009-signal-indication-gate-and-policy-separation.md`
  - `0011-schema-identifier-and-publication-contract.md`
  - `0013-engine-neutral-spatial-geometry-and-length-authority.md`
- 详细设计:
  - `../design/road-junction-model.md`
  - `../design/lane-graph.md`
  - `../design/route-system.md`
  - `../design/signal-system.md`
  - `../design/data-format.md`
- GitHub:
  - #227
  - #228
  - #196
  - #229

## 背景

LaneFlow v0.8 production 以 `LaneEdge`、显式 Route、connector edge、StopLine 和
`(fromEdgeId, toEdgeId)` MovementGate 支撑直行信号化走廊。该模型足以表达已
author 的 directed connection，却不能稳定回答：

- 一个通行意图属于哪个道路连接设施；
- 同一道路级意图有哪些 lane-level 可遍历实现；
- 一条 Route 当前穿越的是哪条 ManeuverPath；
- 一个 Gate 位于 path 的哪个 transition；
- connector/internal edge 属于哪个 Junction；
- roundabout、complex junction 或 interchange 如何组合多个最小连接单元。

v0.8 corridor generator 还通过 connector external ID 中的字符串片段推断
intersection/group 归属。这让 external ID 命名同时承担 identity、membership
和行为选择，无法作为合流、分流、环岛、停车连接与互通立交的长期基础。

ADR 0009 已明确 SignalAspect 只是 indication，MovementGate/StopLine 是空间
准入边界，法规与 conflict arbitration 不进入 SignalController，并且当前 edge
pair 不冻结为长期 Movement identity。v0.9 protected turning 需要在不引入通用
conflict solver 的前提下，把这一留白收敛为正式静态身份。

产品负责人明确允许 clean-break 迁移，不要求旧 runtime/API/schema 兼容。当前
决策因此优先保证身份正确性、确定性与 steady-tick 性能，不保留会延长歧义的
兼容层。

## 决策

### 1. 采用 `Junction -> Movement -> ManeuverPath` 唯一 owner 层级

- `Junction` 是通用的最小拓扑连接/潜在冲突单元。
- `Intersection` 只作为平面交叉 profile 或 authoring 术语，不成为 Core 根实体。
- `Movement` 是 Junction 内的道路级通行意图和稳定分组 identity；Core 不内置
  `left/straight/right` 枚举。
- `ManeuverPath` 是 Movement 的 lane-level、可遍历实现。
- 每个 Movement 恰好属于一个 Junction；每个 ManeuverPath 恰好属于一个
  Movement；每个 Junction 和 Movement 都必须拥有至少一个 child。

LaneEdge 继续只由 LaneGraph 拥有。上述实体引用 edge，不复制 length、
connection、speed limit 或 Spatial geometry authority。

### 2. Junction 派生 internal-edge ownership

ManeuverPath 使用：

```text
entry edge + ordered internal edges + exit edge
```

Core 从 path membership 派生 internal edge 的 Junction owner，不在 wire 中保存
第二份 owner 字段：

- 一条 internal edge 至多属于一个 Junction；
- 同一 Junction 内的多个 ManeuverPath 可以共享 internal edge；
- entry/exit boundary edge 不归 Junction 独占，可连接相邻 Junction；
- internal edge 不得同时成为另一 Junction 的 internal 或 boundary edge。

LaneGraph 持有 LaneEdge storage，normalized topology registry 持有
Junction/Movement/ManeuverPath definition；ManeuverPath 与 Route 只引用
EdgeHandle。internal-edge ownership 是 Junction 对拓扑角色的 semantic exclusive
claim，不表示 Junction 或 ManeuverPath 复制、移动或持有 LaneEdge storage。

该规则让 Junction 具有可审计的拓扑边界，同时为未来同一 Junction 内的
merge/diverge 保留表达能力。它不表示 Core 已拥有 conflict geometry 或
merge arbitration。

### 3. 全局 path coherence，Route 继续拥有实际 traversal

同一 normalized topology registry 中，ManeuverPath 的完整、已解析 EdgeHandle
sequence 必须全局唯一，不受 parent Junction 或 Movement 边界限制。
zero-internal direct path 同样适用。external ID 仍是业务 identity；该规则只保证
一个物理 traversal signature 至多对应一个规范 ManeuverPath definition。

车辆类别、cost、priority 或其他 policy 差异必须作为该规范 path 的独立 overlay
表达，不能通过复制相同 edge sequence 的 ManeuverPath 表达。

Route 仍是有限、显式、有序的 LaneEdge sequence，并继续决定车辆实际行驶顺序。
ManeuverPath 只提供静态语义和可验证 occurrence。

一个 ManeuverPath occurrence 只有在 Route 中完整、连续匹配该 path edge
sequence 时成立。Repeated edge 由 `routeEdgeIndex` 区分。initial/dynamic Route
注册时编译 route-shared 的 Maneuver/Gate occurrence；vehicle tick 不重新匹配
path、不解析 external ID，也不扫描全局 catalog。

### 4. 以一等 `ManeuverGate` 取代 pair-based `MovementGate`

`MovementGate` 名称和 `(fromEdge, toEdge)` value key 不进入 v0.9 target。
新的一等实体为：

```text
ManeuverGate
  externalId
  maneuverPathId
  transitionIndex
  stopLineId
  signalControl
```

`transitionIndex = i` 表示跨越 `pathEdges[i] -> pathEdges[i + 1]` 的准入边界。
StopLine 必须绑定 `pathEdges[i]`。Core 新增 opaque `ManeuverGateHandle` 和
resolver；Gate identity 不再由 edge pair 或 ManeuverPathHandle 代替。

v0.9 protected profile 只允许 entry transition，即 `transitionIndex == 0`，并要求
每条受控 ManeuverPath 恰好一个 entry gate。未来 waiting zone 或多阶段准入可以
在同一 identity shape 上增加 path 内 Gate，不必再次更换 Gate 身份。

本 ADR 取代 ADR 0009 中“pair-based MovementGate 是 current Gate value identity”
作为长期 target 的部分；ADR 0009 的 indication、policy、conflict、StopLine 与
Core safety 分层继续有效。

### 5. RoadSection、LaneGroup 与 JunctionGroup 暂不生产化

- `RoadSection` 冻结为有方向的道路结构分段，用于未来表达道路级上下游、
  ordered lanes 与 lane adjacency；它不是 Route，也不替代 LaneEdge。
- `LaneGroup` 是 RoadSection 内可选的 authoring/组织概念；在 lane adjacency /
  lane-change G1 前不是独立 Core 根实体。
- `JunctionGroup` 是 roundabout、complex junction、interchange 等多个 Junction
  的非行为组合；它不拥有单一 conflict solver、controller clock 或 availability。

v0.9/#229 不引入上述三者的 handle、registry 或 wire array。RoadSection endpoint、
lane adjacency 和复杂设施组合需要独立 G1，不能以不可验证字符串提前进入 current
schema。

### 6. 保持 authority 分离

- Traffic/Core：LaneGraph topology 与 Junction/Movement/ManeuverPath/Gate identity。
- Spatial：edge centerline、canonical frame、弧长绑定与 pose。
- SignalController/Group：program、phase 与 indication。
- Compliance policy：把 indication 解释为 Gate signal-layer permission。
- Future conflict/right-of-way domain：conflict set、priority、gap 或 reservation。
- Core motion：把 regulatory constraint 与 leader、speed limit、ParkingStop、
  RouteEnd、safe-speed/no-overlap 归约。
- Adapter/Presentation：只 query、resolve 和 render，不推导 topology 或裁决通行。

v0.9 不增加 Junction polygon、ConflictZone 或通用 right-of-way solver。由于当前
Following 在车辆进入 shared downstream edge 前不处理 incoming-branch conflict，
#196 必须用显式 path/phase/exit-lane matrix 证明 protected profile 的开放集合无
未建模 crossing、merge 或出口车道抢占。

### 7. 冻结扁平 normalization 与 Route 注册期编译

不把具体 Rust collection 写入 public contract，但 production target 必须满足：

- 四类 static entity 使用 normalization-order dense storage；
- parent/member 关系使用 flat handles + ranges；
- path edge sequence 使用共享 flat `EdgeHandle` storage + range；
- borrowed slice 或零分配 exact-size iterator 提供稳定遍历；
- external-ID map 只在 normalization/resolver 使用；
- topology normalization 按 path input order 使用临时 traversal-signature lookup
  检查全局 coherence，hash/fingerprint 只缩小候选，完整 sequence equality 才能
  判定重复；
- Route 注册通过 entry-transition candidate index 编译 occurrences；
- steady tick 不做 external-ID lookup、hash、path match、全 catalog scan 或
  per-vehicle allocation。

Static handle 是 session/world-scoped caller contract。为避免只给新 handle 增加
与现有 static handle 不一致的 world nonce，v0.9 不承诺运行时识别所有 foreign
handle；跨 graph/world assembly 必须按 retained external IDs 对最终 LaneGraph
rebind/revalidate，禁止复制 dense handles。

### 8. Traffic v0.8 clean-break 原子迁移

#229 的 production target 为 exact-current Traffic `0.8`：

- 新增 `junctions`、`movements`、`maneuverPaths`；
- Signals 把 `movementGates` 替换为 `maneuverGates`；
- Core API、private DTO、loader、schema、fixtures、generator、artifacts、catalog、
  publication、docs 与 digests 同一交付切换；
- 不保留 0.7 runtime loader、dual schema、deprecated alias 或 upgrade shim；
- SpatialPackage 与 ScenarioManifest 继续保持 `0.1`。

历史已发布 schema/bytes 仍按 ADR 0011 immutable；不提供 runtime compatibility
不等于覆写发布证据。

## 后果

### 正向后果

- Junction、Movement、ManeuverPath 与 Gate 各自拥有明确、不可混淆的 identity。
- 完整物理 traversal 在 registry 中只有一个规范 ManeuverPath definition。
- connector 名称不再承担 Junction/group 行为推断。
- 同一 path 可以长期容纳多个 Gate，同一 Junction 可以长期容纳共享 internal
  edge，而无需再次推翻根 identity。
- Route 与 ManeuverPath 不形成双 traversal authority。
- static catalog 的规模不进入 steady vehicle tick，外部字符串不进入热路径。
- protected turning 可以在没有通用 conflict solver 的前提下安全交付，并为未来
  conflict/priority domain 保留明确插入点。

### 代价与风险

- Core、Data、Signals query、generator、fixtures 与当前示例需要一次破坏性迁移。
- `ManeuverGateHandle` 增加一个 static handle domain 和 resolver。
- Route 注册必须编译并保存 occurrence metadata，增加 route-shared 内存。
- Authoring 必须显式提供 membership、path 与 Gate，不能继续依赖命名约定。
- RoadSection 未生产化期间，Movement 的道路级 endpoints 仍是 authoring 语义，
  Core 只验证 child path 的拓扑一致性。

## 被拒绝的替代方案

### 继续使用 `(fromEdge, toEdge)` 作为 Movement identity

无法区分共享入口、后续分叉、lane-level path variant 或多阶段 Gate，也让 signal
binding 与道路级意图混为一体，因此拒绝。

### 用 `ManeuverPathHandle` 直接充当 Gate identity

隐含一条 path 最多一个 Gate，未来 waiting zone 会再次破坏 API，因此拒绝。

### 让 ManeuverPath 替代 Route

会引入第二套 traversal/planner authority，破坏当前显式 Route、route lifecycle、
repeated occurrence 与 caller-owned route selection，因此拒绝。

### 用 Route 字段消歧重复 ManeuverPath

让 Route 携带 Junction、ManeuverPath 或 occurrence discriminator，会把 Route
变成第二份拓扑语义声明，并允许同一物理 traversal 同时归属多个 Junction。
调用方还必须持续维护 discriminator 与 edge sequence 的一致性，因此拒绝。

### 在 vehicle tick 动态匹配 path

会让全局 catalog 规模、字符串或 sequence matching 进入热路径，并放大 10k/100k
成本，因此拒绝。

### v0.9 同时生产化 RoadSection/JunctionGroup

会提前冻结 lane adjacency、section boundary、complex facility composition 和更多
schema 语义，超出 protected-turning 最小闭环，因此拒绝。

### 保留旧 API/schema compatibility layer

项目处于 pre-1.0，产品负责人已授权 clean break。双 identity 和双 loader 会延长
歧义、扩大测试矩阵并降低长期正确性，因此拒绝。

## 后续

- #228：交付本 ADR 与 `road-junction-model.md`，完成设计 G3/G4。
- #196：冻结 v0.9 两个 protected intersection 的 path、geometry、route、
  Gate、phase 与 compatibility matrix。
- #229：在 #228/#196 G4 后原子实现 Core/Data/schema/generator target。
- #190–#192：交付示例、统一验证与 closure。

如果未来改变 owner 层级、让 Route 不再拥有 traversal、把 conflict 权威放入
SignalController/Adapter、允许 dynamic topology，或让 JunctionGroup 成为单一行为
求解器，必须新增或 supersede 本 ADR，不得通过 private 实现静默改变 Accepted
语义。
