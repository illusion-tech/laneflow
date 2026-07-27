# ADR 0019：WaitingZone、ConflictZone 与车辆级通行权 authority

**状态**: Accepted（#235 G1）<br>
**日期**: 2026-07-27<br>
**适用范围**: LaneFlow 的多阶段 ManeuverGate、WaitingZone、ConflictZone、jurisdiction/right-of-way policy、车辆级 conflict grant 与 Core safety 集成<br>
**关联文档**:

- 上游决策:
  - `0003-runtime-tick-and-determinism.md`
  - `0005-core-identity-and-handle-model.md`
  - `0006-vehicle-following-control-and-safety.md`
  - `0007-traffic-data-crate-and-loader-boundary.md`
  - `0008-pre-1.0-data-format-version-policy.md`
  - `0009-signal-indication-gate-and-policy-separation.md`
  - `0012-core-numeric-authority-and-presentation-precision.md`
  - `0013-engine-neutral-spatial-geometry-and-length-authority.md`
  - `0015-bounded-f32-canonical-spatial-frames.md`
  - `0017-static-road-junction-maneuver-and-gate-identity.md`
  - `0018-multimodal-cross-section-and-access-overlay.md`
- 详细设计:
  - `../design/waiting-zone-conflict-right-of-way.md`
  - `../design/road-junction-model.md`
  - `../design/signal-system.md`
  - `../design/vehicle-following.md`
  - `../design/numeric-representation.md`
  - `../design/spatial-geometry.md`
- GitHub:
  - #227
  - #228
  - #229
  - #234
  - #235
  - #264

## 背景

ADR 0009 已把 signal indication、Gate permission、法规解释、conflict arbitration 与
Core safety 分层；ADR 0017 又冻结了
`Junction -> Movement -> ManeuverPath`、带 `transitionIndex` 的一等
`ManeuverGate`，以及 Route registration-time occurrence compilation。当前
production 仍只接受每条 ManeuverPath 的 entry Gate（`transitionIndex = 0`），
没有待行区、多 Gate 路径内状态、冲突区域或车辆级通行权裁决。

中国路口中的左转待转区、多阶段信号、无保护转向、条件通行与复杂让行需要组合
上述能力。若把规则写进 SignalController、Adapter、二维几何相交推导或
JunctionGroup 级大 solver，会破坏既有 authority、确定性与安全边界。

ADR 0018 还冻结了 `AccessRule.regulation` 的
`(jurisdiction, version, source?)` provenance 与“allow 不解除其他约束”原则。
Traffic v0.9 已将 ParticipantClass、CrossSection 与 AccessRule 静态模型生产化：
VehicleProfile 必填 participant class，AccessRule 在 normalization 期消解为
edge/path `AccessCell`，static deny 在 `(ParticipantClass, Route)` 绑定期原子拒绝。
新的 right-of-way policy 必须复用该审计口径和 resolved cells，不能形成第二套
互不相容的法规身份或准入求值器。

## 决策

### 1. ManeuverPath 与 Route authority 不变，多 Gate 按 occurrence 预编译

`ManeuverPath` 继续拥有规范 lane-level traversal；`Route` 继续拥有车辆实际
edge occurrence。一个 `ManeuverGate` 精确覆盖其
`(maneuverPathId, transitionIndex)`，同一路径按 `transitionIndex` 严格递增排序，
同一 transition 最多一个 Gate。Gate 不拥有路径区间、车辆队列或 conflict solver。

Route 注册时把每个 `ManeuverOccurrence` 展开为有序 `GateOccurrence`，每项保存
route transition index、Gate/StopLine handle 及其后续 WaitingZone/ConflictZone
覆盖范围。steady tick 不匹配 external ID、不重新扫描 ManeuverPath，也不通过几何
寻找 Gate。

### 2. WaitingZone 是 Gate 有界的一等静态实体，运行时队列由 Core 拥有

`WaitingZone` 引用同一 ManeuverPath 上严格有序的 `entryGateId` 与
`releaseGateId`，两者之间的 path interval 是其规范拓扑范围；parent Junction 从
ManeuverPath 唯一派生，不重复存储可漂移的 Junction 引用。

WaitingZone 声明非零 `maxOccupancy`，它是运营容量上限，不替代车辆长度、
minimum gap 与 no-overlap。进入条件同时要求：

1. entry Gate 的 signal/regulatory permission 不为 deny；
2. 当前 occupancy 小于 `maxOccupancy`；
3. zone 内有足够物理存储，不会越过 release Gate 或与队尾重叠；
4. Core 更严格的 leader、safe-speed、RouteEnd 等约束允许运动。

车辆跨 entry Gate 时，Core 从 WaitingZone 全局 counter 分配单调 admission
sequence 并增加 occupancy；同 tick 多个 successful crossings 必须按 post-step
physical queue front-to-back 顺序统一分配，不按 reducer、vehicle update 或 raw
handle 顺序。未 crossing 的 staged claim 不消费 sequence，counter overflow 使 step
原子失败。跨 release Gate 时退出 WaitingZone。队列顺序由 admission sequence
冻结，实际纵向位置继续由 Route progress、车辆长度、minimum gap 和 no-overlap
决定，不在 data 中预存等距 parking-like slot。

WaitingZone 的行为 identity/范围归 Traffic + Core；可选 3D region/marking/pose
归 Spatial。缺少 Spatial geometry 不改变 Core 行为，Adapter 不从画面反推 zone。

### 3. ConflictZone 与 ParticipantStream 构成独立 conflict domain

新增静态 immutable `ConflictRegistry`，包含一等 `ConflictZone` 与
`ParticipantStream`：

- `ConflictZone` 归属于一个 Junction；参与 stream 集合只从
  `ParticipantStream` 的 conflict passage 引用派生，不在 zone 上保存反向列表；
- v1 road-vehicle `ParticipantStream` 引用一个 ManeuverPath 与按该 path 规范进度
  定义的 entry/exit anchors；admission ManeuverGate 从 passage entry 之前最近的
  Gate 唯一派生，不在 wire 重复保存；
- 同一 stream 可依次穿过多个 ConflictZone；Route 注册时编译为
  `ConflictPassageOccurrence`；
- Spatial 可用单向 `trafficConflictZoneId` / `trafficWaitingZoneId` 引用绑定可选
  geometry；Traffic entity 不保存 Spatial artifact 内部 ID；
- pedestrian/cyclist crossing 的非 ManeuverPath traversal 由 #236 或后续独立
  G1 扩展，不在本 ADR 伪装成 LaneEdge/ManeuverPath。

PathAnchor crossing、distance-to-anchor 归零、zone enter/clear 与 boundary event
统一由新的 Core 私有
`CONFLICT_ANCHOR_CROSSING_TOLERANCE_METERS` 拥有。它不与 edge-boundary、
longitudinal-constraint 或 physical-gap tolerance 互相别名；authoring canonical
endpoint 仍使用精确结构规则。future current-f64 implementation 的冻结值为
`1.0e-9 m`；front enter、tail clear、distance-to-anchor 归零、one-shot event 与
downstream proof 使用详细设计中同一组 inclusive predicate。

Traffic/Core 中显式声明的 zone-stream 关系是行为 authority。Spatial 拥有 canonical
3D geometry、pose 与 authoring validation；二维投影相交、mesh overlap 或
Adapter collider 只能提出 authoring 候选，不能自动创建或删除 conflict 语义。

### 4. 最终车辆级通行权由 Core ConflictArbiter 决定

SignalController 只产生 indication。每个 `[T, T + D)` step 的 policy 与
ConflictArbiter 使用 tick-start committed signal snapshot(T)；预先计算的
snapshot(T + D) 只在 successful step 末尾提交给下一 tick。版本化 Gate compliance
policy 把 indication、车辆类别、标志标线事实与既有 traversal state 解释为
`deny / protected candidate / permissive candidate / uncontrolled candidate`。
AccessRule、时变法规等 regulatory overlay 只能追加 deny。

Core `ConflictArbiter` 消费候选、yield/priority relation、gap-acceptance profile、
现有 zone occupancy/reservation 与 tick-local candidate set，按稳定总序产生
vehicle-specific `ConflictGrant`。执行顺序固定为：

```text
signal snapshot
  -> jurisdiction/compliance interpretation
  -> AccessRule / other regulatory deny
  -> right-of-way relation
  -> gap acceptance
  -> downstream-clearance guard
  -> ConflictGrant / reservation
  -> longitudinal stop constraints
  -> leader + safe-speed + no-overlap
  -> permission-aware traversal hard guard
```

protected 只表示无需向配置的相交 approach 让行，不表示可以进入仍被占用的 zone；
任何 allow/grant 都不能覆盖 leader、safe-speed、RouteEnd、minimum-gap 或
no-overlap。

#235 选择性能优先的保守确定性 v1 gap solver。它不做随机 critical-gap 分布、
driver aggressiveness、长时域 trajectory search 或每路口可替换算法；只使用
pre-step committed state、route-relative distance、VehicleProfile 最大加速度、
整数毫秒 policy gap 与已编译 conflict approach frontier。frontier 必须覆盖车辆
current cursor 之后 horizon 内的 current/upcoming/repeated passage occurrence，
不能只扫描当前 Maneuver occurrence。每个静态
`(ConflictZone, ParticipantStream)` passage cell 保留按保守程度排序的 top-two
distinct vehicle contributors；Route-specific occurrences 只产生并归约
contribution，不复制 frontier cell。candidate 查询排除 exact subject 后取最严格
剩余值，避免 looping/repeated Route 的车辆用自己的 future target-stream passage
阻塞自己，同时保持 O(1) lookup。normalization 中 static AccessRule deny 的合成
stream/profile approach 不贡献 frontier；public route binding 不会创建该 active
vehicle，已 occupied/reserved 的状态仍由 zone authority fail closed。对每个
subject passage cell 在同一 ConflictZone 中编译出的 exact yield target cell，Core 先以
segmented directed-bound route-distance query 得到冲突车辆到 passage entry 的
距离下界，再使用 fixed-order directed-bound 运算得到 ETA 下界，并要求：

```text
foeEarliestEntryMs
  > fixedDeltaTimeMs + minimumLeadGapMs + clearanceBufferMs
```

同时，最近 incompatible passage clear 后必须已过去
`minimumLagGapMs + clearanceBufferMs`。任一 zone 已占用/预留、时间窗无法有限且
确定地求值时都 no-grant/fail-closed；空 approach 或已证明超出 horizon 的
`OutsideHorizon` 等价于正无穷并通过，不能与 `Unprovable` 混淆。gap 与 fixed
delta 的和必须在 world 初始化时受检且不超过 portable `2^53-1 ms`。具体公式、
directed rounding、fixed operation order、high-precision interval oracle、
first-error 与性能结构见详细设计。该选择优先 tick 成本与 replay，接受无保护转向
吞吐偏保守；可校准概率/预测 solver 必须走 #72 或后续独立 G1，不得静默替换 v1。

v1 还强制执行 downstream-clearance guard：grant 前必须以 tick-start route-local
occupancy、车辆长度/minimum gap、最远 conflict exit、下一 Gate、RouteEnd 与其他
hard stop boundary 证明车尾存在可清空全部 coverage zone 的安全存储，并把最远
exit 之后的 storage proof 扩展成 admission Gate crossed side 至 clearance target
的 physical edge/progress claim span；storage proof 使用与 tail-clear 相同的
`storageUpperBound + tolerance >= exit + vehicle.length` inclusive predicate。只
claim 最终 footprint 会漏掉 coverage 内的跨 Route merge。stable-order arbiter
必须针对 committed 与 earlier staged claim 原子取得该 span；不同 Route/Junction
汇入同一 physical downstream edge 时不能重复分配。同 edge 相邻 claim 按 progress
找到实际 follower，并使用该 follower 的 minimum gap，不能始终使用当前 candidate
profile。未落位 claim 不作为可依赖继续移动的虚拟 leader，因此 v1 不做同 tick
convoy/packing。不能证明或 claim 冲突时 normal no-grant；该 guard 不能按 Junction、
Adapter LOD 或 solver tier 关闭。

Route registration 必须为每个 resource-bearing Gate occurrence 编译
empty-occupancy Waiting storage 与 conflict-clearance boundary metadata，但
`register_route` 继续保持 class/profile-agnostic，不能因为某个无关长车型不适配就
拒绝整条 Route。initial/spawn/replacement/runtime route assignment 在既有
`(ParticipantClass, Route)` static access validation 之后，必须按实际
VehicleProfile 对 cursor 后缀逐 occurrence 执行 static feasibility check：空
WaitingZone 连车身都容纳不下，或最远 conflict exit 到下一 Gate/RouteEnd 连车尾都
无法清空时，原子拒绝该 `(VehicleProfile, Route, cursor)` 绑定。initial Routes 与
dynamic Routes 使用同一 compiler；永久 impossible 的 profile-route 组合不得留成
每 tick no-grant，也不得扩大成 route-global rejection。

### 5. Grant 是 tick-local，跨 Gate 后原子升级为 reservation

stable arbitration 中的 `ConflictGrant` 必须原子取得一个 resource bundle：
coverage ConflictZones、optional Waiting admission capacity/storage 与 optional
downstream clearance span。candidate 只产生 request；唯一
`ConflictArbiter` 按规范 total order 更新 committed + earlier-staged 可见的
single-writer ledger。任何一个资源失败都 no-grant 且不得留下部分 claim；同 tick
staged leave/clear 不返还 capacity，避免 winner 依赖尚未提交的运动。

无 Waiting/Conflict resource coverage 的 pure signal/regulatory Gate 沿用既有
permission path，不创建空 grant；latest decision 以 `NotRequired` 区分。regulatory
deny 以 `NotEvaluated` 区分，resource-bearing evaluation 才产生
`Granted | NoGrant(reason)`。records 和 multi-reason attribution 必须使用规范顺序，
不能由 proposal completion 或 raw handle 决定。

未跨 admission Gate 的 `ConflictGrant` 只对当前 tick 有效；车辆因更严格约束没有
跨 Gate 时，grant 及全部 staged claim 自动失效，不长期占用 zone。车辆成功跨 Gate
时，grant 与 Maneuver traversal state 按 bundle 内容分别原子提交 Waiting
membership、`ConflictReservation` 和/或 committed downstream claim；只有非空
ConflictZone coverage 才创建 reservation，纯 Waiting admission 不伪造空
reservation。作用域包含 vehicle、Route occurrence、Gate 与覆盖的
ConflictZone/physical span。

reservation 在车辆清除全部对应 exit anchor 后释放；失败 step 不改变 grant、
reservation、WaitingZone occupancy、downstream claim、vehicle state、events、
tick/time。claim 只在同一 successful step 的 post-step actual occupancy 已成为下一
tick leader authority 后随 final tail-clear 释放。任何 route 替换或 completion 都
不得遗留 reservation/claim；不能证明等价迁移时必须原子拒绝命令。
若 passage 在 `[T, T + D)` 内 clear，`lastClearTimeMs` 只在 successful commit
记录 post-step `T + D`；同 step 较早的 arbitration 不观察该 staged clear，下一
tick 的 lag elapsed 从零开始。

Rust 实现应把 staged grant 表达为 Core 私有、不可 `Copy`/`Clone` 的消费式
capability，并由 crossing 显式转换为 committed state；权威 release 不依赖
`Drop`/panic unwinding。proposal 计算可以并行，但锁竞争、CAS 或 worker 完成顺序
不能决定 winner；所有 proposal 必须 stable-sort 后进入单一 reducer。

### 6. 车辆状态属于 CoreWorld，不写回静态数据

Core 保存可观察但不可由 Adapter 修改的 committed
`ManeuverTraversalState`：

```text
PreGate -> Committed <-> Waiting
             |             |
             +--> Clearing <-+
                    |
                    +--> Committed / PreGate / complete
```

多个 WaitingZone 可使 `Committed <-> Waiting` 重复。release/admission grant
属于当前 step 的候选决策，不是 committed vehicle state：未跨 Gate 时 vehicle state
保持不变且 grant 失效；跨 Gate 时直接原子提交下一 `Committed`/`Clearing` 状态与
必要 reservation。CoreWorld 级稀疏 latest-decision batch 只记录刚完成 tick 中
evaluated Gate frontier 的归因，不能在下一 tick 复用为 permission。黄灯或 signal
切换只影响尚未跨越的 Gate，不能撤销已提交的 Gate crossing；后续 Gate 仍按各自
当前 snapshot 独立裁决。

Waiting membership 与车辆当前是 `Committed` 还是已停住的 `Waiting` phase 正交：
车辆跨 entry Gate 后即持有 membership，即使同 tick 继续行驶也不能丢失；stop/resume
只切换 phase，successful release crossing 或显式原子 removal 才移除 membership。
per-zone queue index 必须与该车辆侧 semantic authority 同事务维护。

既有 spawn/initial/Completed replacement 可以指定非零 route cursor，但 v1 不根据
cursor 猜测历史 Gate、Waiting 或 Conflict authority。对含 stateful occurrence 的
Route，车辆只可在 first stateful Gate 未跨越侧或 occurrence exit 之后
materialize；cursor 位于 crossed side 到 exit 的 interior 时 capability-unavailable
并原子拒绝。未来 interior bootstrap 必须独立 G1，一次验证 maneuver state、
Waiting membership/sequence、reservation/downstream claim 与 occupancy。

### 7. jurisdiction/right-of-way policy 复用统一 provenance

right-of-way policy header 使用 ADR 0018 已冻结的
`(jurisdiction, version, source?)` 口径，并增加半开
`[effectiveFrom, effectiveUntil)` 审计范围。Scenario/World 初始化必须显式 pin
policy set 与 regulation date；不得读取宿主墙钟，运行中不得隐式切换法规版本。

Traffic v0.9 AccessRegistry normalization 已保证同一 package 中所有显式
AccessRule regulation 共享一个 `(jurisdiction, version)`。#235 只把这一个
optional normalized identity 与 right-of-way policy 比较：两者必须一致；未声明
provenance 的 AccessRule 继续沿用 ADR 0018 的合法“未指定”语义。sign、road
marking、vehicle class 等输入必须先规范化为 Traffic/Core 事实并保留 provenance；
SignalController 与 Adapter 不解析国家代码或视觉资产来决定通行权。

“可合法进入”统一由 Traffic v0.9 `AccessRegistry::path_access/edge_access`
resolved `AccessCell` 派生：profile 对 ParticipantStream admission Gate 至 last
passage exit 的 path 平面和全部 physical edge 平面都不是
`Decided { effect: Deny, .. }` 时为 `conflictEligible`；`Allow` 与
`Unconstrained` 都 eligible。current public binding 已拒绝 route suffix 上的
static deny，因此该静态定义用于 policy totality/authoring coherence，不重复执行
route access。current production 仍 capability-reject `timeWindows`；未来时变
runtime 必须对任一规范 segment 曾 eligible 的 `everConflictEligible` union 建总表，
不能让 policy table 随 tick 缺项。

World 初始化必须为每个 `everConflictEligible` registered VehicleProfile 编译恰好
一个 resolved stream rule；缺失/歧义 rule、非空 yield set 缺少 gap profile、或
target stream 任一 eligible profile 不具备严格更高 priority 时拒绝，tick 不得猜测
默认值。pinned policy 还必须与 signal phase/ConflictZone 做 protected-coherence
validation：同 phase 不能让 incompatible PreGate streams 同时得到 Protected；
runtime reservation 只是 invariant 防线，不是错误 authoring 的静默降级。
一个 Gate coverage 含多个 subject streams 时，每个 exact subject cell 仍使用自己的
resolved rule/yield range，candidate effective priority 取全部 distinct subject
rules 的最小值；不得任选一条 rule。纯 Waiting admission 不虚构 stream priority，
以显式 absent-priority rank 进入 arrival/waiting-ticket 排序。

多法规版本在同一 world 热切换、实时政策下载与城市级预约系统不属于本 ADR。

### 8. 稠密存储、确定性与性能边界

Conflict/Waiting static entities 使用 external ID + dense typed handle + flat
storage/range；Route registration 编译 Gate/Waiting/Conflict occurrences。运行时
使用 dense occupancy/reservation tables、owner-attributed top-two approach
frontier、physical downstream/Waiting claim ledger、route-local clearance-boundary
index 与预分配 scratch；禁止每 vehicle 热路径分配、字符串比较、hash iteration、
全 catalog 扫描或每 tick 几何相交。

static entity handle 继续按 normalization order 分配，但 raw handle 数值不参与业务/
事件 tie-break；normalization 从 external ID ASCII byte order 一次编译 dense
`canonicalRank`。input permutation 后按 external ID 对齐的 grant、diagnostic 与
same-anchor event 必须语义等价，steady tick 不做字符串排序。

同一 tick 的候选按 protected rank、policy-priority presence rank、coverage-min
right-of-way priority、首次到达 tick、带显式 presence rank 的 Waiting admission
sequence、唯一 vehicle update sequence 组成的规范键排序；pure Waiting 的 absent
numeric slot 规范为 `0` 且不参与有/无 priority 的相对顺序，不能冒充 policy default。
raw vehicle handle 不参与 tie-break。同输入、同 fixed delta、同命令序列必须得到
逐位一致的 state/event order。生产实施必须维持现有 10k product budget 与 100k
research scaling guard，并报告 top-two frontier bytes、claim count/collision 与
physical-span visits；具体协议见详细设计。

## 与既有 ADR 的关系

- 本 ADR **扩展但不 supersede** ADR 0009：Controller/indication、policy、
  conflict、Core safety 的既有职责分离保持不变。
- 本 ADR **扩展但不 supersede** ADR 0017：ManeuverPath/Route occurrence/
  ManeuverGate identity 保持不变，只解除 v0.9 protected-turning 产品 profile 的
  `transitionIndex = 0` 限制并增加 occurrence metadata；该产品版本与 Traffic
  format v0.9 是不同版本轴。
- 本 ADR **复用而不替换** ADR 0018 及其 Traffic v0.9 实现：AccessRule 的
  target/effect/组合、resolved AccessCell 与 regulation provenance 不变；
  right-of-way policy 不能跨平面解除 AccessRule deny。
- 本 ADR **扩展但不改变 current production** ADR 0012：新增独立 conflict-anchor
  tolerance owner 与 ETA directed-bound helper；只有后续 implementation G1 才能
  原子进入 Core numeric policy。
- 本 ADR 不改变 ADR 0013/0015 的 Spatial authority：几何可验证/表现，不拥有行为。

## 后果

### 正向后果

- 待转区、多阶段信号与无保护转向拥有同一套可组合 identity/authority。
- signal allow 不再被误当作最终通行权，冲突裁决可以给出 vehicle-specific、
  可审计结果。
- Route occurrence、WaitingZone queue 与 ConflictZone reservation 均可 replay，
  且不把字符串或几何扫描带入 steady tick。
- frontier retained state 由 static passage cell 数量决定，dynamic Route/repeated
  occurrence 只增加有界 contribution visits，不复制 cell。
- 中国法规差异通过 versioned policy 表达，而不是 Controller/Adapter 私有分支。

### 代价与风险

- Core 增加静态 registry、车辆 maneuver state、zone occupancy/reservation 与新的
  constraint/event 面；Traffic 必须从届时 current version（当前为已发布 v0.9）
  原子升级，Spatial schema 是否升级由 geometry implementation G1 决定。
- gap acceptance 与多 zone grant 会扩大 tick 热路径，必须通过 active frontier、
  稠密索引与 10k/100k 证据约束。
- mandatory downstream-clearance guard 会增加每 candidate 的 route-local
  storage/boundary/claim query，并在出口拥堵或同 tick claim 冲突时更早拒绝进入；
  v1 不在未落位 claim 后做 convoy packing，吞吐偏保守，但避免车辆持有
  reservation 却无法独立清空 zone 的网络级阻塞。
- frontier cell 保存两个 distinct-owner contribution，retained bytes 高于单一
  tri-state cell；它以常数内存换取 O(1) exact-subject exclusion，避免
  candidate × vehicle 回扫。
- WaitingZone 同 tick successful admissions 需要在 commit 前按 physical queue
  order 做有界 stable ordering 与 counter preflight；canonical-rank arrays 也增加少量
  static retained memory。
- arbitrary-progress spawn/replace 仍可用于普通 route segment，但 stateful
  occurrence interior 在 v1 被 capability guard 拒绝；需要从路口内部 materialize
  的产品必须先设计完整 bootstrap transaction。
- 性能优先 v1 会低估部分 permissive movement 的可接受 gap，可能增加等待和队列；
  它不是专业交通工程容量校准模型。
- 只使用 path anchors 的 Core 行为可能与精细 Spatial region 有近似差异；authoring
  validation 必须检查绑定一致性，不能悄悄让 geometry 取代 path progress。
- starvation/fairness 依赖稳定 arrival/admission ticket；implementation 若改用
  hash/map iteration 会破坏 replay。

## 被拒绝的替代方案

### SignalController 直接决定最终通行权

会把国家法规、车辆类别、gap acceptance 与 zone occupancy 写进 phase clock，违反
ADR 0009，且无法处理 signal 已放行但 conflict 仍被占用的情形。

### Adapter collider 或二维折线相交自动产生 ConflictZone

mesh/collider 属表现层，二维相交不能表达高架分离、错层、共享边界或 authoring
意图；不同 Adapter 还会得到不同结果，因而拒绝。

### WaitingZone 使用固定等距 slot

车辆长度与 minimum gap 不同，固定 slot 会与 Core no-overlap 产生第二套物理
authority。采用 maxOccupancy + route-relative physical storage。

### grant 一经产生即长期 reservation

车辆可能被 leader、RouteEnd 或安全投影挡住；未跨 Gate 就持有长期 reservation 会
制造无谓阻塞和死锁。grant 只在 crossing 成功时升级。

### 只检查 zone 空闲，不检查下游可清空存储

leader 可以位于 exit anchor 之后、因而不算当前 ConflictZone occupant，却仍让
subject 无法把车尾移出 zone。若只依赖 crossing 后的 no-overlap 与 reservation，
车辆会安全但长期占住路口，并可能形成相邻 Junction 的循环死锁。v1 因此在 grant
前强制 downstream-clearance guard；模拟违规 block-the-box 行为必须另立
versioned policy。

### clearance 只读取 tick-start occupancy，不取得排他 claim

两个不共享 ConflictZone、甚至属于不同 Junction 的 candidate 可能汇入同一
physical downstream edge；若都只看 tick-start free space，它们会同时通过 proof，
随后由 no-overlap 把其中一辆安全地停在 conflict 内。v1 因此把 Waiting/
Conflict/downstream 资源组成 stable-order atomic grant bundle，并让后续 candidate
观察 earlier staged claim。

### 把未落位 claim 当作 virtual leader 允许同 tick packing

后车若按前一 grant 的未来目标位置计算 storage，就依赖该 owner 继续移动；而 owner
仍可能被更严格 leader/safety constraint 延迟，违反 mandatory keep-clear 的独立
清空证明。v1 对重叠未落位 span fail closed；预测 convoy/conditional multi-commit
属于后续独立 solver G1。

### 每 tick 对 Route 重新匹配 ManeuverPath/Gate/zone

这会重复 ADR 0017 已消除的路径匹配成本并引入歧义；所有 occurrence metadata 必须
在 Route 注册时编译。

### 以 deny-overrides 代替 right-of-way arbitration

AccessRule 的 allow/deny 是准入 overlay，不表达同时到达车辆之间的时序与资源互斥。
把二者混为一层既无法给出 gap acceptance，也会破坏 ADR 0018 的组合语义。

### 首版直接采用可校准概率/长时域预测 gap solver

这会把 driver distribution、场景校准、trajectory prediction、额外状态和更大的
hot-path 矩阵一次引入 #235，超出“非完整专业交通工程 solver”的范围，也无法在当前
证据下证明 10k/100k 成本。保留为 #72 或后续独立 G1；v1 使用规范的保守确定性
critical-gap filter。

## 后续

- #235：完成本 ADR 与 `waiting-zone-conflict-right-of-way.md` 的 G1/G3/G4。
- 后续最小 production 切片应至少分为：
  1. 从届时 current Traffic（当前为 v0.9）原子升级的
     multi-Gate/WaitingZone static + Route occurrence compilation + profile-route
     static feasibility binding；
  2. WaitingZone runtime state/queue/constraints；
  3. ConflictZone/ParticipantStream static + Spatial binding；
  4. right-of-way policy normalization + ConflictArbiter/grant/reservation；
  5. cross-layer fixtures、10k/100k 与 Adapter observation。
- #264：消费本 ADR 与 #237 的冻结结论，拆分 JunctionGroup、环岛、停车连接与互通
  组合设施 G1。

如果未来改变最终 conflict decision owner、让 Spatial/Adapter/SignalController
拥有行为、让 grant 绕过 Core safety、或放弃 registration-time occurrence
compilation，必须新增或 supersede 本 ADR，不得通过 private 实现静默偏离。
