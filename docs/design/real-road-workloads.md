# 真实路网 Workload

**文档状态**: Accepted（#224 G1）<br>
**最后更新**: 2026-07-27<br>
**适用范围**: LuST Scenario v2.0 的可复现获取、LaneFlow 静态转换、10k
真实路网性能补充 workload 与需求代表性 observation

**关联文档**:

- [`core-runtime-performance-baseline.md`](core-runtime-performance-baseline.md)
- [`data-format.md`](data-format.md)
- [`data-loading.md`](data-loading.md)
- [`spatial-geometry.md`](spatial-geometry.md)
- [`../governance/dependency-security.md`](../governance/dependency-security.md)
- [#224 当前 G1 纠偏冻结判断](https://github.com/illusion-tech/laneflow/issues/224#issuecomment-5078718717)

## 1. 目的、状态与声明边界

本文是 LaneFlow 真实路网 workload 的长期设计事实源。首个来源固定为 LuST
Scenario v2.0，并拆成两个不能互相冒充的稳定 workload：

- `LF-REAL-LUST-TOPO-v1`：真实静态拓扑、Signals、lane/connection 密度和
  demand-derived 长路线条件分布下的 10k active 性能补充。
- `LF-REAL-LUST-DEMAND-v1`：固定 DUE 样本的 departure schedule、route 和自然
  lifecycle observation。

旧的 `LF-REAL-LUST-v1` 在任何 artifact 或 benchmark result 产生前退役。不得用
旧 ID 生成新结果，也不得把两个新 workload 的结果合并回旧 ID。

本文当前只冻结设计契约。converter、Release assets、TOPO/DEMAND plan、benchmark
harness 和运行结果均由 #224 G4 后的下游 Issue 交付。在这些实现和证据完成前：

- `LF-SYNTH-v1` 继续是 10k/100k 唯一 canonical product Gate baseline；
- TOPO 只能形成真实拓扑压力的补充结果，不能单独形成 Product Pass；
- DEMAND 全部结果都是 observation，不适用 #215 的 performance budget；
- 不得宣称真实城市 workload SLA、LuST travel-time fidelity、真实停车代表性或中国
  交通代表性。

v0.8 Signalized Corridor 的 50–200 车辆 native reference 场景仍由
[`example-scenarios.md`](example-scenarios.md) 和
[`signalized-corridor-population.md`](signalized-corridor-population.md) 管理。它
验证 production loader、Signals、人口与回流闭环，不承担本文 10k 真实路网
workload 的规模或代表性职责。

## 2. 来源选择

### 2.1 候选结论

候选评估以可公开再获取、许可和署名可执行、lane-level 数据、10k/100k 规模适配、
Signals/route 完整度及长期可复现性为主要条件：

| 候选                                       | G1 记录的许可/数据边界                  | 结论                                                              |
| ------------------------------------------ | --------------------------------------- | ----------------------------------------------------------------- |
| LuST Scenario v2.0                         | LuST MIT；底层 OSM 派生数据 ODbL 1.0    | 选择为首个 10k 来源；使用固定 commit 和消费子集                   |
| BeST Berlin                                | CC BY 4.0；约 800 km²、71,651 edges     | 100k 首选候选，由下游 G 独立 G1，不在当前 ID 中预留               |
| Alicante–Murcia                            | MIT；高速 corridor                      | 可作吞吐专项，但缺少当前需要的信号和停车语义                      |
| InTAS / MoST / TuST                        | GPLv3 场景数据叠加 OSM 派生义务         | 当前工程分发边界不采用；MoST 规模也不匹配 10k 目标                |
| TAPAS Cologne / Bologna                    | 分别为 CC-NC / 未找到可执行的明确许可   | 排除                                                              |
| nuScenes、nuPlan、Argoverse 2 等 HD 地图集 | NC 或 research-gated                    | 排除                                                              |
| MATSim open scenarios                      | link-level，且来源/标注不能闭合 lane 级 | 排除                                                              |
| 中国城市公开/商业地图与路口数据            | lane 级开放来源不足或存储/派生受限      | 不进入当前真实路网计划；由手工 authoring 样本独立覆盖中国特色语义 |

上表是 #224 G1 的工程适用性结论，不是针对任一司法辖区的法律意见。

### 2.2 固定来源

唯一允许的 LuST v1 source revision 为：

```text
repository = https://github.com/lcodeca/LuSTScenario
tag        = v2.0
commit     = c4bd5bd3751d426d42a9a1749c815e47ea188549
```

tag 只用于人类识别，commit SHA 才是 revision authority。获取工具必须请求精确
commit，不得解析默认分支、浮动 tag 或 latest release。

固定消费输入及已复核 digest 为：

| 文件                                        |      bytes | SHA-256                                                            |
| ------------------------------------------- | ---------: | ------------------------------------------------------------------ |
| `scenario/lust.net.xml`                     | 10,940,662 | `6f5d76223cf14b797ae6267f13b23eb6c872d76adec1fb22a8569a806dc09341` |
| `scenario/lust.poly.xml`                    |  2,743,451 | `abb519b3e12e0392111e9d0c3517e8f54ec424dedbb54dd9ceaf31a9eb3fcc8e` |
| `scenario/tll.static.xml`                   |     83,530 | `893f4e0e9ffb8e8eed67caeafada8fec958fb92f437aa41f69783c059c474102` |
| `scenario/vtypes.add.xml`                   |      1,707 | `8bfa1f4f2c51f4f15066e5d799027572f3482aaaa67fb37ee10a75cecf42ee92` |
| `scenario/DUERoutes/local.static.0.rou.xml` | 41,634,764 | `08a76518941bcb56ee42a50e34a360efade103a21a6ea3d7d19f5a66765e6503` |
| `scenario/DUERoutes/local.static.1.rou.xml` | 42,862,343 | `1e657f82da83d43869d9bf91d7f20c997bc9b2d031cd4f08ccdc3ccd84836ccf` |
| `scenario/DUERoutes/local.static.2.rou.xml` | 44,934,267 | `bdd93df275c624f44db4d3bbdc659cd91f237a5951b1d09fa0be105264ecdd35` |
| upstream `LICENSE.md`                       |      1,134 | `9a16c8681095f730e72ad6efe2b56c30a924552d4b6b6326868309749d64837b` |

`buslines.rou.xml`、DUA routes、公交、检测器和其他 upstream tree 内容不进入 v1。
converter 不能因缺少某个固定文件而从完整 tree 中寻找替代输入。

已复核的 source health anchors 为：

- 5,779 个 external edges、8,622 条 external lanes、30,051 条 connections；
- network `<location>` 精确为
  `netOffset="-285448.66,-5492398.13"`、`convBoundary="0.00,0.00,13613.76,11455.04"`；
- external lane 的 `length`、`speed`、`shape` 均完整；
- network 与 `tll.static.xml` 各有 201 个 controller，ID 集合精确相等；
- 1,298 个 static phases，零时长 phase 为 0；
- 三个 DUE static 文件共 215,526 个 vehicles/routes 和 8,771,017 个 edge
  references，引用 5,777 个 external edge IDs，dangling reference 为 0；
- 14,170 个 polygons，其中 `parking` 为 175 个；
- 7 个 vehicle types，其中 6 个 passenger type、1 个 bus type。

任一 source bytes、计数或 closure 与上述事实不一致都属于 source mismatch，必须
fail closed 回到 #224 G1。

## 3. 共享静态转换

### 3.1 Lane graph、ID 与 route

- 每个 SUMO external `<lane>` 与 junction internal / `via` `<lane>` 都映射为一个
  LaneFlow `LaneEdge`；`length` 和 `speedLimit` 分别取 lane 的 `length` 与
  `speed`，单位保持 m 和 m/s。不得丢弃 internal lane 或把路口两侧 external lane
  直接连接。
- 只有拥有至少一条 normalized external-entry → external-exit traversal 的 SUMO
  road junction 才映射为 current Traffic v0.10 `Junction`。该 identity/owner shape
  由 v0.8 引入并被 v0.9 继承；owner 由 external from-edge 的 `to`、external
  to-edge 的 `from`、`junction@intLanes` 显式 membership 与完整 connection chain
  共同确定；四者不能唯一闭合时 fail closed。
- `type="internal"` helper node、dead-end node，以及 normalization 后没有合法
  traversal 的 source junction 不生成 LaneFlow `Junction`。helper/internal node
  只通过上述显式 topology membership 折叠进所属 traversal，不允许用 `:j` 等 ID
  substring 推断 owner。最终每个 emitted Junction 至少一个 Movement，每个
  Movement 至少一个 ManeuverPath。
- 同一 emitted Junction 内按 `(from road edge ID, to road edge ID)` 冻结道路级
  转向意图并映射为 `Movement`；相同意图的多条 lane-level traversal 仍是不同
  `ManeuverPath`。
- 每条 external-lane connection 从 entry external lane 开始，按 SUMO `via` 和
  internal connection 链顺序遍历零到多条 internal lane，最后到达 exit external
  lane；该完整序列映射为
  `ManeuverPath(entryEdgeId, ordered internalEdgeIds, exitEdgeId)`。空 `via` 只在
  source 确实没有 internal lane 时允许。
- source connection 与 `via` chain normalization 使用
  `(junction ID, from road edge ID, to road edge ID, from lane index, to lane index,
  ordered internal lane IDs)` 的稳定顺序。unknown/dangling `via`、cycle、跨
  Junction owner、非连通 sequence 或重复 traversal signature 都是 fatal error。
- 所有 external ID 使用 `sumo:` namespace，不能按 ID 首字符选择性加前缀。
- 每个道路级 source route 枚举起始道路的全部 external lane，并只保留能沿
  connection graph 完成整个道路级 route 的 lane-level path；按完整 external
  lane-index occurrence sequence 的 lexicographic order 选择唯一最小路径。因此先
  选择能完成全程的最低起始 lane index，再对每个后续道路 edge 选择能完成余下
  route 的最低 lane index；不得硬编码从 lane index 0 开始。选中
  `ManeuverPath` 的完整 internal-edge sequence 展开到 LaneFlow Route。这样
  registration-time compiler 必须在每个 junction occurrence 唯一匹配同一
  ManeuverPath。无完整路径、零/多 path 匹配、incomplete/overlap、未知 edge 或
  ambiguous normalization 都是 fatal error；不能跳过已选 record。
- 相同 lane occurrence sequence 可归一化为同一个 route catalog entry，但 source
  record 仍保存自己的 stable `population_rank`、profile 和 route reference。

### 3.2 Spatial geometry

- external 与 internal lane 的 `shape` 都生成对应 LaneEdge 的 Spatial centerline；
- source shape 坐标、`netOffset` 与 `convBoundary` 的 lexical decimal 都按精确
  十进制有理数解析。对 source point `(sx, sy)`，先用
  `projected = (sx - netOffset.x, sy - netOffset.y)` 撤销 SUMO offset；canonical
  origin 固定为 projected `convBoundary` 的精确中点：
  `(292255.54, 5498125.65)`；随后必须计算
  `(x, z) = (projected.x - origin.x, projected.y - origin.y)`。完整三步公式精确化简为
  `(x, z) = (sx - 6806.88, sy - 5727.52)`；不得省略 origin subtraction，也不得改用
  geometry centroid、首点、bounding box of selected lanes 或实现自选 origin；
- canonical frame 为右手 Y-up，`x=easting`、`z=northing`，可用 elevation 写入
  `y`；二维 source 没有 elevation 时 `y=0`。SpatialPackage `frameId` 固定为
  `lust-v2.0-c4bd5bd3-convboundary-center`；
- midpoint 与 subtraction 全部在精确十进制有理数域完成，然后只进行一次
  IEEE-754 binary64 round-to-nearest, ties-to-even 转换；`-0.0` 规范化为 `0`，
  JSON number 使用能 round-trip 到相同 binary64 的最短十进制表示。重复转换必须
  byte-identical；
- Traffic edge length 与 Spatial quantized polyline arc length 必须通过现有
  binding validation；ManeuverPath 中每一对相邻 external/internal edge 也必须通过
  current Spatial endpoint 连续性验证。不能用丢弃 internal geometry 或放宽
  tolerance 规避验证。

LuST 约 13.6 × 11.5 km 的范围在重定中心后位于现有每轴
`[-16_384, 16_384] m` canonical frame 边界内。超界不能通过缩放、分片或改变
Spatial contract 在本 ID 下解决。

### 3.3 Static Signals

v1 只接受已复核的 201 个 static controllers：

- network controller ID 与 `tll.static.xml` 必须精确闭合；
- 每个 phase duration 必须严格正，并按精确 source decimal 转为整数毫秒；
- `G/g -> Green`、`y/u -> Yellow`、`r/o/O -> Red`；SUMO major/minor green 的
  让行差异不能由 current LaneFlow static indication 表达，必须在转换报告中记录；
- group 由“全部 phases 中状态向量相同的受控 connection 等价类”确定，成员
  connection 的字典序决定稳定 group ID；
- 同一 from edge 只生成一个 edge-end StopLine，相关 gates 共用该 StopLine；
- 每个受控 lane-level connection 必须解析到唯一 `ManeuverPath`，并生成
  `ManeuverGate(maneuverPathId, transitionIndex=0)`；Gate crossing 是
  `entryEdge -> first internalEdge`，无 internal edge 时才是
  `entryEdge -> exitEdge`。不得生成 pair-based `MovementGate` 或从 connector ID
  推断 Junction/Movement owner；
- controller、phase、group、gate 与 StopLine 的对象计数和 stable order 写入
  转换报告。

不得固化 actuated program，也不存在“缺失 controller 时降级为 unsignalized”的
路径。任一 controller 缺失、损坏、需要丢 phase 或需要降级时，转换立即失败。

### 3.4 Vehicle Profile

只允许六个 passenger vtypes。Traffic v0.10 输出固定声明
`motorVehicle -> car` 两级 ParticipantClass，全部六个 profile 必填
`participantClassId: "car"`；bus class/profile 不生成。Traffic v0.10 wire
normalization 使用：

| SUMO vtype 输入 | Traffic v0.10 wire field  |
| --------------- | ------------------------- |
| `accel`         | `maxAcceleration`         |
| `decel`         | `comfortableDeceleration` |
| `length`        | `length`                  |
| `minGap`        | `minGap`                  |
| `maxSpeed`      | `desiredSpeed`            |

`emergencyDeceleration = 8 m/s²`、`timeHeadway = 1.0 s` 是 v1 常量；
`speedDev` 不展开为随机 profile。未知 vtype 和 bus type 都是 source selection
error。

### 3.5 Parking

LuST v1 不把 polygons 合成为 ParkingArea 或 ParkingSpace。175 个 `parking`
polygons 只作为 source health 和转换报告事实；Traffic Parking registry 必须为空。
TOPO 和 DEMAND 均不得声明停车代表性。未来引入真实或合成 Parking 会改变 state 和
artifact digest，必须使用新 workload ID。

### 3.6 共享输出

converter 分别生成：

- Traffic v0.10 package，包含 `junctions[]`、`movements[]`、
  `maneuverPaths[]`、`signals.maneuverGates[]`、上述 ParticipantClass/profile
  binding，以及显式空 `facilityBands[]`、`roadSections[]`、`laneGroups[]`、
  `roadCorridors[]`、`accessRules[]`；LuST v1 不伪造尚未设计的横断面或准入语义；
- SpatialPackage v0.1；
- ScenarioManifest v0.1，用 size 和 SHA-256 配对 Traffic/Spatial；
- conversion report，记录 source health、normalization object counts、
  warning/loss boundaries，以及 Traffic/Spatial/ScenarioManifest 三个 direct payload
  的 raw digests；report 不记录自身、shared static archive 或外部 manifest 的
  digest；
- semantic provenance manifest，记录第 2 节 source chain、config digest、licenses、
  Release assets 和 normalized semantic output digests；其内容和 digest 不包含
  converter commit、toolchain、build timestamp 或 host；
- build provenance record，记录 converter commit、锁定 toolchain/依赖、调用参数、
  semantic provenance digest 和本次生成的 raw output digests；它使用 canonical
  serialization，且不得写入 wall-clock timestamp、host name、绝对路径或未冻结的
  environment value，也不记录自身 digest。

Traffic/Spatial/ScenarioManifest 与 conversion report 组成 shared static bundle。
semantic provenance manifest 是 bundle 外的 versioned index，记录该 bundle 和其他
Release assets 的 URL/size/digest；它不得嵌入任何由自己索引的 asset，否则会形成
self-digest cycle。build provenance record 同样位于 bundle 外，作为逐次生成审计
证据，不进入 semantic bundle 或 cross-build comparison digest。初始车辆、release
schedule、runtime handles、Parking binding、lifecycle call log 和 presentation
selection 不写入 Traffic/Spatial/ScenarioManifest。

## 4. 共享精确 10k source population

候选记录只来自 `.0`、`.1`、`.2` 三个 DUE static 文件，并按该文件顺序解析：

1. 只保留 `depart` 位于精确秒区间 `[28800, 30600)` 且引用六个 passenger
   vtypes 之一的 `<vehicle>`。
2. vehicle ID 使用原始 UTF-8 bytes；禁止 trim、case folding 或 Unicode
   normalization。空 ID、重复 ID、未知 vtype/route 或 dangling edge 立即失败。
3. 主排序键是 `SHA-256(UTF-8 vehicle ID)` 的 32 bytes，按 unsigned byte
   lexicographic order；碰撞时以原始 vehicle-ID UTF-8 bytes 为第二键。
4. 候选数必须精确为 10,592。按上述顺序取前 10,000，并依次赋
   `population_rank = 0..9999`。

source-file ordinal 与 XML vehicle ordinal 进入转换报告，但不参与唯一 vehicle ID 的
排序。不得扩窗、换 seed、跳过无法转换的已选 record 或用第 10,001 个候选补位。

完整 10k record table、selection config 和各自 digest 是 TOPO/DEMAND 的共享输入。

## 5. `LF-REAL-LUST-TOPO-v1`

### 5.1 用途与时间协议

TOPO 是真实拓扑压力的 10k active supplement。它不忠实回放 LuST departure
timing，也不代表 LuST 出行需求或旅行时间。

fixed delta 固定为 `16 ms`。从转换后的最长 discrete static-signal cycle 得到
`C_signal_ticks`，并使用：

```text
warm_up_ticks     = max(512, 4 * C_signal_ticks)
observation_ticks = max(4096, 8 * C_signal_ticks)
T_case_us          = (warm_up_ticks + observation_ticks + 1) * 16_000
T_case_s           = T_case_us / 1_000_000
```

额外的一个 tick 是严格的 remaining-free-flow-time 安全边界。
`remaining_free_flow_time_s` 由 source `length` / `speed` lexical decimal
作为精确十进制有理数求和；实现用交叉相乘比较
`remaining_free_flow_time_s * 1_000_000 > T_case_us`，不得先转浮点、舍入到
整数毫秒，或把秒值直接与毫秒值比较。

### 5.2 Template 与 physical slots

对共享 10k records 的每条 converted lane route，任一候选 progress 的最快剩余自由流
时间为：

```text
current edge remaining length / speedLimit
  + sum(all later edge lengths / their speedLimits)
```

“all later edge”包含 Route 中的完整 internal-edge sequence。只有存在
`remaining_free_flow_time_s > T_case_s` 合法位置的 record 才进入 template list；
template 按 `population_rank` 升序。

令六个 passenger profiles 的 envelope 为：

```text
Lmax  = max(profile.length)
Gmax  = max(profile.minimumGap)
pitch = Lmax + Gmax
```

只有 external route edge 可以提供 tick-0 physical slot；internal edge 参与 Route、
ManeuverPath、剩余自由流时间和 digest，但不作为初始车辆承载位置。每个 external
physical lane edge 的候选 front-bumper progress 为
`Lmax + k * pitch`，其中 `k = 0, 1, ...`，且
`progress <= edgeLength - Gmax`。同一 `(edge, progress)` 只有一个 physical
capacity slot，不能因 Route 重复经过该 edge 而复制物理容量。

logical slot `i` 的唯一 `logical_rank = i`，范围精确为 `0..9999`；它使用
`template[i mod template_count]` 的 route/profile，并把该 template record 的
`population_rank` 单独保存为 `source_population_rank` provenance。template
重复不能复制 source rank 成为 logical rank。其可选 placement identity 为
`(route_edge_index, edge, progress)`；必须位于该 route occurrence 的 external
edge 上，并同时满足 slot envelope 和从该 occurrence 开始计算的
`remaining_free_flow_time_s > T_case_s`。同一 Route 重复经过同一 edge 时，各
occurrence 是不同 placement candidate，但继续竞争同一个 `(edge, progress)`
physical capacity slot。

车辆按 logical index 升序，placement candidate 按
`(sumo edge ID UTF-8 bytes, route_edge_index, finite progress total order)`
排序。最终布局是在“每个 logical slot 恰好一个 placement、每个 physical capacity
slot 至多一次”的约束下，对 ordered assignment tuple 求
**lexicographically smallest perfect matching**；plan 必须保存
`route_edge_index` 作为唯一 Core spawn cursor。无 template、重复 physical
capacity、非唯一 occurrence、无 10k perfect matching 或 Core tick-0 batch
validation 失败均使 workload 构造失败；不能修改 pitch、时间窗、人口或匹配规则后
沿用本 ID。

### 5.3 Runtime population 与 Gate

- 10,000 辆车在 tick 0 均以 `VehicleStatus::Active`、`initialSpeed=0` 建立；
- Parking registry 为空；
- warm-up 和 observation 内不执行 spawn、despawn、replace 或 Parking command；
- remaining-time 条件必须保证完整 protocol 内不产生 Completed；
- 每个 successful pre/post-step 精确满足
  `N_individual=10000`、`N_traffic_active=10000`、`N_aggregate=0`；
- 任一 Completed、人口变化、caller mutation、overlap 或 unexpected status change
  都使 round 无效。

presentation 主行为 100%；1%/10%/50% 是 observation。选择以第 5.2 节唯一
`logical_rank=i`、canonical seed 0 沿用 #215 的
`SplitMix64(logical_rank xor seed)` stable-rank 算法；不得使用可能因 template
重复而重复的 `source_population_rank`，也不按 vehicle status 或 external-ID
prefix 选择。

TOPO 适用 #215 的 warm-up、fresh-process rounds、latency/tail、memory、
hard-invariant 和 exact-only oracle 纪律，但它是 supplemental row，不产生独立
Product Pass/Fail。结果只能声明真实静态拓扑/控制和“从固定 DUE 样本条件化得到的
长路线”压力。

## 6. `LF-REAL-LUST-DEMAND-v1`

### 6.1 Departure 与 pending authority

DEMAND 忠实回放共享 source records 的固定 departure schedule：

- source anchor 固定为 `t0 = 28800 s`；
- source `depart` lexical decimal 作为精确十进制有理数解析；
- `release_tick = ceil((depart - t0) * 1000 / 16)`，不得先转浮点或先舍入到毫秒；
- pre-step boundary `k` 尝试所有 `release_tick <= k` 且尚未进入 Core 的 records；
- release 前的 record 只存在于 caller-owned `DemandPending` table，不创建伪造
  Parked Core vehicle，不占 ParkingSpace，也不计入 `N_individual`；
- 每个 selected record 从 case 开始就在 `DemandPending`，直到成功插入 Core 才从
  table 原子移除。尚未到 release boundary 的 future record 与已经到期但因
  `VehiclePhysicalOverlap` blocked 的 record 都由同一 table 持有；blocked 不是第三
  个 owner，也不能从 pending 中临时移除。

同一 boundary 按 `(release_tick, population_rank)` 升序尝试。

### 6.2 确定性 `departPos`

全部已选 source records 必须具有 lexical value `departPos="random"`；任何其他值
都是 workload/source mismatch。

对 record 计算：

```text
h = SHA-256(
      UTF-8("LF-REAL-LUST-DEMAND-v1\0")
      || UTF-8(vehicle ID)
    )
u = big_endian_u64(h[0..8])
q = high_53_bits(u) / 2^53
```

首 route edge 的合法 front-bumper 区间为：

```text
low  = vehicle.length
high = edgeLength - vehicle.minimumGap
progress = low + q * (high - low)
```

`high < low` 或任一非有限值立即失败。input 使用 source record 的 route/profile 和
`initialSpeed=0`。

### 6.3 Blocked、Completed 与结束条件

只有 Core `VehiclePhysicalOverlap` 是可恢复的 blocked 结果：

- blocked record 保留完全相同的 route/profile/progress/input；
- 每个后续 boundary 最多重试一次；
- retry 不重新 hash、不换 lane、不降速、不消耗随机状态；
- `N_demand_pending` 是 `DemandPending` 的当前总 cardinality；
  `N_spawn_blocked` 是其中 `release_tick <= current boundary` 且最近一次 insertion
  attempt 返回 `VehiclePhysicalOverlap` 的子集 cardinality，不是累计失败次数；
- 其他 validation 或 invariant error 均为 fatal。

route completion 自然进入 `Completed`，并保留 live identity 到 case 结束；不
recycle、不 despawn、不补位。

release phase 固定为 112,500 ticks（1,800 s）。随后 drain phase 最多运行
112,500 ticks，或在
`N_demand_pending=0 && N_spawn_blocked=0 && N_traffic_active=0`
时提前结束。到达 drain 上限时仍 blocked 的 records 继续保留在
`DemandPending`，其完整 caller state 必须进入 final replay digest；不得把它们当作
已回放或静默丢弃。每 tick 必须报告：

- `N_source_total=10000`；
- `N_demand_pending`；
- `N_spawn_blocked`；
- `N_individual`；
- `N_traffic_active`；
- `N_completed`；
- `N_presented`；
- `N_aggregate=0`。

每个 pre/post-step boundary 还必须满足
`N_source_total = N_demand_pending + N_individual = 10000`、
`N_spawn_blocked <= N_demand_pending` 和
`N_individual = N_traffic_active + N_completed`；任何 record 无 owner、重复 owner
或跨 owner 重复计数都是 fatal。

DEMAND 不设置 active lower bound。1%/10%/50%/100% presentation rows 全部只是
observation；selection 沿用稳定 `population_rank`，但不能据此形成 performance
Gate。结果只能声明固定 LuST DUE 样本的 departure/route/lifecycle 分布。
blocked insertion 和 LaneFlow longitudinal behavior 不能作为 SUMO travel-time
fidelity 证据。

## 7. Oracle、digest 与可比性

converter 必须分别生成 shared static bundle、TOPO plan 和 DEMAND plan。plan 是
caller/harness 输入，不进入 production Traffic/Spatial/ScenarioManifest schema。

两个 workload 都必须在相同 converter commit、锁定 toolchain/config 下，于 fresh
process 中从 pinned source 重建两次，并得到相同的：

- semantic provenance manifest digest；
- build provenance record digest；
- conversion config digest；
- static bundle 和 conversion report digest；
- workload plan digest；
- topology digest；
- initial/state digest；
- fixed-input-sequence digest。

TOPO 还要以 exact-only Core 验证完整 protocol 每个 pre/post-step 的 10k active
invariant。DEMAND 比较 ordered command outcomes、ordered events、per-tick count
digest 和 final state。exact-only Core 是 oracle；本文不新增候选 runtime 语义。

其中 build provenance record digest 只证明同一 build identity 的重复生成；它不是
跨 converter build 的 semantic comparison key。可比性规则固定为：

- 同 workload ID、同 scale 只有在全部 applicable **semantic** digest 相同时才能
  直接比较；两次运行都必须保留自己的 build provenance record，但 record digest
  不要求跨 build 相同；
- source snapshot、转换配置、normalization algorithm 或任一 semantic output
  digest 改变时，必须提升受影响 workload ID；
- converter 纯重构且所有 semantic bytes/digests 不变时可以保留 ID，但运行记录必须
  保存新的 converter commit 和 build provenance record；
- TOPO、DEMAND 与 `LF-SYNTH-v1` 不直接比较绝对值；同硬件、同 build、同协议的
  跨 ID 差异只能标为相对 observation；
- production runtime 不得识别 workload ID、source、seed、population rank 或
  external-ID prefix 走专用路径。

## 8. 许可、制品与离线消费

LaneFlow converter、config、manifest schema 和项目自有文档仍按仓库
Apache-2.0 分发。LuST/source/static/plan 数据保留自己的第三方许可边界：

- source bundle 原样包含 upstream `LICENSE.md`；
- bundle 包含 ODbL 1.0 全文；
- bundle 包含 NOTICE，至少保留
  `Road network data © OpenStreetMap contributors`、ODbL 1.0 链接、LuST
  Scenario v2.0 来源、MIT 和 LuST 作者 attribution；
- semantic provenance manifest 逐项记录上述 license/NOTICE bytes 和 SHA-256；
- LuST 作者请求的论文 citation 进入 NOTICE，但不改写为项目自有许可证要求。

所有 raw snapshot、Traffic/Spatial static package、TOPO plan 和 DEMAND plan，无论
大小，一律作为 `illusion-tech/laneflow` GitHub Release assets 保存，不提交 Git。
Git 只保存 converter、config、semantic provenance manifest、build provenance
record、digests、转换报告摘要、license/NOTICE 文本和长期设计文档。

四类 asset bundle 固定为 source、static、TOPO、DEMAND。每个 Release asset 的
pinned URL、byte size 和 SHA-256 写入 versioned semantic provenance manifest。archive
必须是 deterministic uncompressed POSIX tar：

- path 按原始 UTF-8 bytes 升序；
- `mtime=0`；
- `uid=0`、`gid=0`；
- owner/group 为空；
- regular-file mode 为 `0644`；
- 不调用会引入版本相关 bytes 的压缩器。

regeneration/preflight 可以显式下载 manifest 中的 pinned Release URL，但下载完成后
必须先核对 byte size，再核对 SHA-256，之后才能解包。benchmark/runtime 禁止联网，
只接受 caller 显式提供且 digest 已验证的本地 artifact directory。离线 cache 以
asset SHA-256 为 key，不以 tag、latest、文件名或 URL basename 作为 authority。

上述内容是 LaneFlow 的工程合规边界，不替代具体分发场景的法律审查。

## 9. Fail-closed 条件

以下任一条件失败都停止转换或运行，并回到 #224 G1：

- 固定 source URL/commit/file bytes、对象计数或 201-controller closure 不匹配；
- XML shape、current Traffic v0.10 schema/full-reference、ParticipantClass/profile
  binding、显式 CrossSection/Access arrays、Junction/Movement/ManeuverPath
  ownership/path connectivity、ManeuverGate binding 或 Traffic/Spatial
  length/endpoint binding 失败；
- SUMO internal / `via` chain unknown、dangling、cyclic、跨 Junction，或任何
  internal lane 的 Traffic/Spatial geometry 被丢弃；
- source junction owner 不能由 edge endpoint、`junction@intLanes` 和 connection
  chain 唯一闭合，或 emitted Junction / Movement 为空；
- selected candidate 数不等于 10,592 或精确 10k table digest 不匹配；
- selected record 无法转换，或 profile/route/source reference 非法；
- repeated conversion 不是 byte-identical；
- TOPO 秒/微秒 exact-unit comparison 不成立，或没有 template、10k perfect
  matching、重复 edge 的 occurrence/capacity identity 不唯一、tick-0 Core
  validation、`logical_rank=0..9999` 唯一性或完整 10k-active oracle；
- DEMAND departure/departPos、ordered outcomes/events/counts 或 final replay
  digest 不一致；
- license/NOTICE、Release URL、size 或 SHA-256 preflight 不完整。

失败后不得扩窗、替换 record、调整 pitch/人口/时间、降级 Signal、合成 Parking、
跳过 source file 或静默切换来源。若未来从 Luxembourg OSM 自建 snapshot，必须
独立来源 G1 并使用新 ID，例如 `LF-REAL-OSM-LUX-v1`；不得复用任何 LuST ID。

## 10. 下游切片与治理边界

#224 G4 后分别创建以下 G0 Issue：

| 切片 | 交付                                                                     | 依赖/边界                      |
| ---- | ------------------------------------------------------------------------ | ------------------------------ |
| A    | source/static converter、Release assets、provenance 与 conversion report | 其他 workload 实施的共同前置   |
| B    | TOPO plan、harness 与 evidence                                           | 依赖 A                         |
| C    | DEMAND caller policy、plan、harness 与 evidence                          | 依赖 A                         |
| D    | DUA rerouting                                                            | 独立 G1/ADR 判断               |
| E    | bus/stop semantics                                                       | 独立 G1                        |
| F    | 中国特色手工 authoring 样本和独立 workload ID                            | 独立排期                       |
| G    | BeST 100k 来源、裁剪和 workload                                          | 10k 获取/转换链路稳定后独立 G1 |

产品路径上另有示例层切片（Parent #252），**不**改变上表 A–G 的 workload 语义：

| 切片     | 交付                             | 依赖/边界                                                                                |
| -------- | -------------------------------- | ---------------------------------------------------------------------------------------- |
| H (#256) | LuST/Bevy 1–10k 个体人口调节契约 | 设计已 Accepted；见 [`lust-bevy-population-control.md`](lust-bevy-population-control.md) |
| I (#257) | Bevy LuST native 示例与调节 UI   | 依赖 A 的 static、B 的 TOPO plan（placement 权威）与 H 的契约；C/DEMAND 仍可选           |

本文不改变 Core API、Data format、Spatial API、Adapter API、production runtime
behavior 或 crate dependency direction，也不新增 ADR。

若任一实现要求新的通用 scenario controller、batch lifecycle API、public schema、
Parking 合成、感应信号、rerouting、bus stop、partition/scheduler 或新的 Spatial
frame contract，必须停止对应切片并完成独立 G1；满足 ADR 触发条件时先写 ADR，再
进入实现。
