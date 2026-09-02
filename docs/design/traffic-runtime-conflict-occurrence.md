# 路线冲突出现项与仲裁前能力保护

**文档状态**: Accepted（#283 G1；PR #546）<br>
**最后更新**: 2026-09-01<br>
**适用范围**: `TrafficWorld` 路线冲突出现项、路线坐标规范化、资源容量，以及
#284 交付冲突仲裁前的活动车辆能力保护

**关联文档**:

- [`../adr/0019-waiting-zone-conflict-right-of-way-authority.md`](../adr/0019-waiting-zone-conflict-right-of-way-authority.md)
- [`portable-canonical-artifact.md`](portable-canonical-artifact.md)
- [`shared-static-network.md`](shared-static-network.md)
- [`traffic-observation-and-routing-integration.md`](traffic-observation-and-routing-integration.md)
- [`traffic-runtime-shared-consumption.md`](traffic-runtime-shared-consumption.md)
- [`traffic-runtime-snapshot.md`](traffic-runtime-snapshot.md)
- [`traffic-runtime-revision-cutover.md`](traffic-runtime-revision-cutover.md)
- [`waiting-zone-conflict-right-of-way.md`](waiting-zone-conflict-right-of-way.md)

## 1. 结论

冲突静态领域已经由现行 official source、LFCA 4 与
`SharedNetworkRevision::conflict()` 完整拥有。本文不再登记实体、格式表、来源映射、
语义差异或空间区域，也不重述这些权威文档。

#283 尚需交付的 Runtime 合同只有：

1. 唯一 `compile_route` 为每个 `ManeuverOccurrence` 展开全部
   `ConflictPassageOccurrence`，保留同一路线中的重复出现；
2. 把 path-local anchor 映射为无二义性的 route-local 整数位置，并保留
   admission Gate coverage；
3. 用独立语义容量限制全部存活路线保留的冲突出现项；
4. #284 冲突仲裁能力不存在时，禁止创建或留下尚未由车尾清除全部冲突通行段的
   `Active` 车辆；
5. 快照恢复与路网修订切换只保存稳定路线输入并重新编译派生出现项，不持久化热表。

静态根与含冲突路线可以正常 build、install、register、保存和加载。能力保护只约束
会提交 `Active` 车辆的低频生命周期边界；fixed tick 不扫描冲突表、不临时停车，也不
伪造 grant、reservation 或通行权。

## 2. Authority 与保留形状

### 2.1 静态 passage 地址

`ConflictPassage` 是 `ParticipantStream` 所有者局部关系，没有全局 ordinal 或独立
`StableId128`。路线出现项必须使用：

```text
ConflictPassageAddress =
  (ParticipantStreamOrdinal, passageLocalIndex: u32)
```

`passageLocalIndex` 是共享根中该 stream 的规范 slice 下标。`ConflictZoneOrdinal`、
admission Gate 与 route position 可以缓存为执行操作数，但都不能成为第二套 passage
身份。#284 必须复用这一地址，不能从几何或 `(zone, path)` 重新猜测 passage。

#284 的快照与跨修订定位候选见
[`traffic-runtime-right-of-way-policy.md`](traffic-runtime-right-of-way-policy.md) §6
（Review）：使用 stream/zone 的稳定引用定位已有唯一局部关系，核验 LFSD 与路径语义后
重建本节地址；不把当前根 ordinal/local index 写成跨修订身份，也不新增 passage 实体。

### 2.2 每路线保留数据

实现目标的私有逻辑形状为：

```text
RoutePosition
  routeEdgeIndex: u32
  progressMm: u32

ConflictPassageOccurrence
  stream: ParticipantStreamOrdinal
  passageLocalIndex: u32
  zone: ConflictZoneOrdinal
  maneuverIndex: u32
  admissionHop: u32
  entry: RoutePosition
  clearance: RoutePosition

CompiledRoute additions
  conflicts[]
  conflictGateRanges[]
  finalConflictClearance?: (RoutePosition, conflictOccurrenceIndex)
```

`clearance` 就是静态 passage exit 映射到路线后的整数位置；它不提前加车型长度、
clearance buffer 或未来 policy tolerance。`conflictGateRanges` 与现有 `hop_gate` 对齐，
长度至多为 route hop 数；没有与冲突出现项数量一起增长的第二张索引表。

`conflicts` 按以下总序原地不稳定排序；该顺序同时保证同一 admission hop 的 coverage
连续：

```text
(admissionHop, entry, clearance, stream.raw(), passageLocalIndex)
```

`finalConflictClearance` 取 `(clearance, 上述出现项总序)` 的最大项。它只用于 #284
前的 O(1) 能力保护；#284 可以直接消费完整 `conflicts` 与 Gate range。

## 3. 路线位置规范化

设一个 `ManeuverOccurrence` 的 path 有 `N` 条边，path-local edge `i` 映射到 route
edge `entryRouteEdgeIndex + i`。`exitRouteEdgeIndex` 必须等于
`entryRouteEdgeIndex + N - 1`。

静态 anchor 唯一映射为：

```text
Gate(transitionIndex = t)
  -> (entryRouteEdgeIndex + t + 1, 0)

EdgeBoundary(0)
  -> (entryRouteEdgeIndex, 0)

EdgeBoundary(b), 0 < b < N
  -> (entryRouteEdgeIndex + b, 0)

EdgeBoundary(N), exitRouteEdgeIndex + 1 < routeEdgeCount
  -> (exitRouteEdgeIndex + 1, 0)

EdgeBoundary(N), exitRouteEdgeIndex is route terminal
  -> (exitRouteEdgeIndex, length(lastPathEdge))

Interior(pathEdgeIndex = i, progressMm = p)
  -> (entryRouteEdgeIndex + i, p)
```

因此所有仍有 route to-side edge 的边界都只使用下一边 `(index + 1, 0)`；路径起点
使用首边 `(index, 0)`；只有整个路线终点保留末边 `(index, edgeLength)`。Gate 与同
位置 boundary 的 source 规范编码仍由 LFCA 权威保证；路线编译器不接受第二种
route-local 表示。

车辆 route cursor 在参与比较前也执行同一规范化：非末边的
`progress_mm == edge_length_mm && carry_um == 0` 转为下一边 `(index + 1, 0, 0)`；
路线末端保留 `(last, edge_length_mm, 0)`。其它进度越界或边界上的非零微米余数是既有
车辆不变量错误，不通过饱和或截断修复。

位置比较先比较 route edge occurrence 下标，再比较 `progress_mm`，最后比较
`carry_um`。它比较的是路线 occurrence，不是 `LaneEdgeOrdinal`；循环路线和重复边不会
折叠为同一位置。

## 4. 唯一路线编译与失败原子性

direct、candidate、admitted/replay、snapshot restore 与 cutover target 都必须进入
现有唯一 `register_route_edges` / `compile_route` 路径。冲突出现项编译按以下顺序执行：

1. 完成既有非空、边、连通、机动 occurrence 与覆盖验证；
2. 对每个 `ManeuverOccurrence` 查询 path 的所有 participant stream，再以 checked
   `u64` / `usize` 汇总其 passage 数；
3. 在任何按 passage 数增长的分配前，核对 §5 的世界总容量；
4. 精确预留一条 `conflicts` vector，映射 anchor、验证 owner/path/Gate 闭合并填充；
5. 原地排序，建立按 hop 数增长的 `conflictGateRanges`，计算
   `finalConflictClearance`；
6. 路线槽、边出现项计数、冲突出现项计数和命令游标在同一提交点更新。

任何 checked arithmetic、静态闭合、容量或分配失败都只丢弃局部 staged
`CompiledRoute`。不得占用 route slot、推进命令游标、改变两个世界计数或留下半条
路线。`remove_route` 成功时按该 compiled route 的 exact `conflicts.len()` 释放计数；
失败不改变计数。

## 5. 冲突出现项容量

`route_edge_occurrence_capacity` 继续只统计全部存活动态路线的 `edges.len()` 总和，
不改变名称或既有语义。#283 为 `WorldConfig` 新增：

```text
route_conflict_occurrence_capacity: u64
```

`WorldConfig::new` 在 `route_edge_occurrence_capacity` 之后增加该必填参数，不提供会随
机器内存变化的默认值；`0` 合法，表示该 world 只能注册不含冲突出现项的路线。

独立计数解决的是实际 retained memory：一条较短的机动路线可以穿过多个冲突区，
`conflicts.len()` 并不等于 `edges.len()`。它不是针对恶意制品的任意 byte quota，也不
把 allocator、操作系统或机器总 RAM 变成 Runtime 行为输入。

它统计全部存活动态路线的 `conflicts.len()` 总和；重复路线、重复 Maneuver occurrence
和重复 passage occurrence 都逐项计数。注册预检使用：

```text
next = liveRouteConflictOccurrenceCount.checked_add(candidateConflictCount)
accept iff next <= routeConflictOccurrenceCapacity
```

超过容量返回独立的 `ConflictOccurrenceCapacityExceeded` 路线错误；不能伪装为边容量、
普通 allocation failure 或静态格式限制。

一项容量恰对应一条 `ConflictPassageOccurrence`。本切片不保留第二个按该计数线性增长
的 vector；Gate ranges、route coordinate 辅助列与既有 compiled-route 列均按 route
edge/hop 数增长，继续由 `route_edge_occurrence_capacity` 约束。未来若 #284 需要新增
每 passage 的并行 retained 列，必须返回 G1 明确其收费倍率或新容量，不能隐式扩大
本容量代表的内存。

该容量是行为语义配置：它会改变后续路线注册命令的成败，因此必须进入快照配置与
确定性摘要。现行唯一生产版本轴为：

```text
LFRS formatVersion               = 4
runtime_state_version            = 4
RUNTIME_STATE_DIGEST_VERSION     = 6
```

v4 `WorldConfigBinding` 继续包含 `route_conflict_occurrence_capacity: ulong`。旧 reader、writer
和 schema 不属于当前生产入口，不双读、不自动迁移。其它需要修改 Runtime Snapshot
的设计必须从 current 版本继续编号，不能并行占用同一版本值。
恢复容量错误新增 `RouteConflictOccurrences` dimension，与现行 routes、vehicles 和
route-edge-occurrences 一样分别报告 snapshot 配置、target 配置与实际重建计数。

## 6. #284 前的 3A 能力保护

### 6.1 不变量与判定

在冲突仲裁能力尚未安装时，每辆成功提交的 `Active` 车辆必须满足：

> 该车辆的 route-local 车尾位置已经到达或越过该路线全部
> `ConflictPassageOccurrence.clearance` 的最大值。

车尾位置由候选 front cursor、`carry_um` 与实际 `VehicleProfile.length_mm` 沿同一
compiled route occurrence sequence 向后计算。跨边时使用共享根整数边长；重复
`LaneEdgeOrdinal` 仍按各自 route occurrence 处理。若车身伸到路线起点之前，车尾为
`BeforeRouteStart`，小于任何 passage clearance；有限车尾位置也必须执行 §3 的边界
规范化，不能把前一边末端与后一边零进度当成两个位置。

精确谓词为：

```text
allowed = route.finalConflictClearance is None
       OR routeRear(candidateCursor, profile.length_mm) >= finalClearance
```

等于 clearance 表示车尾已经清除；提前一微米仍拒绝。不能只看前保险杠、当前 route
edge、第一处 passage 或“cursor 之后是否还有 entry”。这个判定自然覆盖当前、后续、
车身仍占用、循环和重复出现项。

### 6.2 必须覆盖的入口

| 入口                         | 行为                                                    |
| ---------------------------- | ------------------------------------------------------- |
| build/install 含冲突的共享根 | 允许；新 world 尚无车辆                                 |
| 任意路线注册入口             | 允许；完整编译出现项并收费                              |
| `spawn_vehicle`              | 候选 `Active` 车辆执行保护                              |
| `replace_completed_vehicle`  | 新 `Active` 车辆执行保护，失败保留旧 Completed          |
| `leave_parking`              | `Parked -> Active` 的候选 exit cursor 执行保护          |
| `rebind_parking_route`       | 对保持 `Active + Reserved` 的候选路线和完整车身执行保护 |
| `spawn_parked_vehicle`       | 允许；不创建 Active，离场时再检查                       |
| snapshot restore             | 路线重编译后检查全部 Active；任一失败零发布             |
| same/cross-revision cutover  | target 路线重编译后检查全部 Active；任一失败保留旧聚合  |
| fixed tick                   | 不查冲突表；满足不变量的车辆只会继续向前                |

`park_vehicle`、despawn 与 Completed 转换只减少 Active 集，不需要把已经安全的路线重新
判为不安全。Parked / Completed 可以保留含冲突路线；再次变为 Active 时必须经过上表
入口。

生命周期检查位于既有 handle/profile/route/cursor、静态 access 与物理 footprint
验证之后，任何 world mutation、occupancy 发布或命令游标推进之前。失败使用明确的
`ConflictRuntimeUnavailable` 错误类，并至少携带 route/staged route 标识、stream
ordinal、passage local index 与 zone ordinal；不能退化成 `UnknownRoute`、Overlap 或
普通 invariant error。

## 7. 快照、恢复与切换

路线冲突出现项完全由有序 `LaneEdge StableId128` 序列和目标
`SharedNetworkRevision` 确定，不进入 LFRS：

- capture 仍只保存路线边稳定标识序列；
- v4 保存 `WorldConfig` 容量并把它纳入 deterministic digest；
- restore 使用目标根的唯一路线编译器重建出现项，先核对 edge 与 conflict 两个总容量，
  再执行 Active 车辆 3A 保护，全部成功后才发布 world；
- 恢复目标容量可以放大，但必须容纳快照内重建出的 exact 出现项总数；精确回放对拍
  仍要求两个语义容量与原配置一致；
- cutover prepare 在 target 根上重编译全部路线并累计 exact conflict count；容量、
  allocation 或任一 Active 车辆保护失败都放弃整个 target；
- migration journal 和 cutover descriptor 继续记录稳定路线输入，不复制派生热表。

## 8. 与后续运行时的边界

- #282 只拥有 WaitingZone 本地 membership/admission/storage，不得借 #283 预置通用
  Conflict grant、reservation 或组合 ledger。
- #284 直接消费本文的 passage address、route position、occurrence 与 Gate range；其
  正式能力同切片移除 `ConflictRuntimeUnavailable` 临时保护，并建立车辆级
  grant/reservation/tail-clear 状态。不能先删除保护再分期补仲裁。
- #542 可以生成场景路线或显式 conflict authoring，但路线最终仍进入同一 Runtime
  编译器；场景 catalog 不保存另一份 occurrence 权威。
- Spatial 与 Adapter 只消费共享根的可选区域或只读 Runtime 结果，不参与路线
  occurrence 编译和 3A 判定。

## 9. 验证与现实规模

G2 必须覆盖：

- Gate、路径起点、内部 boundary、路径终点和 Interior 的 exact route mapping；
- 前一边末端与后一边零进度的规范等价，非规范 cursor 失败关闭；
- 同一 path 多 stream、多 passage、区间重叠、重复 Maneuver occurrence 与循环路线；
- `finalConflictClearance` 不是最后 entry 的反例；
- 不同车型长度及车尾在 clearance 前一微米、相等、后一微米；
- 上表全部生命周期入口、Parked/Completed 允许面和失败零副作用；
- edge 容量与 conflict 容量分别达到 `max-1 / max / max+1`，checked overflow、分配
  failpoint、route removal 释放和三个路线注册入口共用计数；
- LFRS v4 round-trip、v3/unknown version 拒绝、摘要差异、恢复容量放大、cutover
  target conflict count 增减与整事务回滚；
- 10,000 冲突路线出现项产品档与 100,000 scaling 档的注册时间、retained logical
  bytes 和近线性比例。它们是路线元数据，不等于活动车辆数。

一百万静态实体、LFCA file-backed 路径和共享根内存已由静态制品合同验证；#283 不
重复构造第二套百万静态基准。它只证明新增 Runtime 元数据与实际出现项数量近线性，
不宣称 #285 fixed-tick 产品性能已完成。

当前资源账本中 `ConflictPassageOccurrence` 为 `36 B/项`，因此 10,000 / 100,000 项
分别保留 `360,000 B` / `3,600,000 B`。release 证据命令输出两档注册时间与比例；
这是新增的 conflict-count 线性 payload，不是还包含 edge/hop 元数据的完整
`CompiledRoute` retained 总量。绝对墙钟不作为共享 CI runner 的硬门槛，近线性形状、
exact 计数与逻辑字节账本是可复现约束。具体运行结果属于 PR / Issue 验证证据，不写入
长期设计。复现命令：

```text
cargo +1.98.0 test --release --locked -p laneflow-runtime --test compiled_networks conflict_route_registration_10k_100k_wall_clock_evidence -- --exact --ignored --nocapture
```

## 10. 非目标与返回 G1 条件

本切片不实现：

- 静态 entity/field/role/format、Identity、LFSM/LFSD 或 Spatial 新版本；
- 通用几何候选、right-of-way policy、gap、approach frontier；
- `ConflictArbiter`、grant、reservation、downstream claim 或 tick 事件；
- WaitingZone 动态运行时；
- 旧 LFRS reader、迁移 shim 或旧 Core/JSON 路径。

以下变化必须返回 G1：给 passage 全局身份；修改静态 anchor 或 owner-local 地址；让
几何拥有行为；持久化派生 occurrence；在 #284 能力原子到位前放宽 3A；新增未收费的
按 conflict count 线性 retained 表；或 #284 不能无歧义消费本文操作数。

## 11. G2 入口

实现前必须把本文更新为 `Accepted`，把 #283 的 Project `Design gate` 更新为
`Accepted`、Status 更新为 `Ready`，并相对届时 current main 再冻结版本与依赖。实现
PR 是一个 Runtime completion slice：路线编译、容量、现行 LFRS、生命周期、恢复、切换、
测试和 exact-current 文档必须闭合提交；不得把只有路线编译的部分结果称为 #283 完成。
