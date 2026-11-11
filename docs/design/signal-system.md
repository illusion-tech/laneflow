# Signal System 设计

**文档状态**: Accepted（固定时制与 Gate 合规仍有效；相位时长必须是世界步长的正整数倍）<br>
**最后更新**: 2026-11-09<br>
**适用范围**: Signals 静态领域、fixed-time runtime、车辆合规与性能边界。静态信号数据由编译器 / 共享静态路网承载。<br>

**关联文档**:

- `../architecture.md`
- `../roadmap.md`
- `../adr/0003-runtime-tick-and-determinism.md`
- `../adr/0005-core-identity-and-handle-model.md`
- `../adr/0006-vehicle-following-control-and-safety.md`
- `../adr/0008-pre-1.0-data-format-version-policy.md`
- `../adr/0009-signal-indication-gate-and-policy-separation.md`
- `../adr/0028-integer-millimeter-traffic-geometry.md`
- `traffic-runtime-integer-geometry.md`
- `../adr/0017-static-road-junction-maneuver-and-gate-identity.md`
- `road-junction-model.md`
- `signalized-corridor-protected-turning.md`
- `route-system.md`
- `vehicle-following.md`
- `parking-system.md`

## 1. 目标、状态与非目标

本文定义 fixed tick、route occurrence、Vehicle Following 和不可绕过的纵向安全层之上的基础信号灯闭环：

- static、fixed-time、cyclic SignalController；
- ManeuverGate 与 StopLine；
- red / yellow / green indication 和 protected-entry profile；
- 红灯/限制性黄灯停车、排队、绿灯放行；
- permission-aware route traversal；
- 可供 Adapter/debug 使用的只读查询。

本文不定义下列能力；已交付的路权、等待与冲突行为以 `waiting-zone-conflict-right-of-way.md` 与路权策略实施合同为准：

- permissive green、无保护左转、红灯右转、红灯掉头；
- 左转/直行待行区的专用语义；
- 无信号路口优先级、gap acceptance、conflict graph/zone、reservation；
- 行人/非机动车冲突仲裁、环岛或 merge priority；
- actuated/adaptive controller、detector、program switching、off/flashing；
- Engine Adapter ABI、灯具 geometry/transform、渲染动画或 authoring UI；
- 专业交通工程配时优化或外部仿真器兼容层。

## 2. 长期分层与 protected-entry profile

SignalAspect 只是 indication，不是面向未来的最终通行权。完整决策链分为：

```text
SignalController
  -> SignalAspect
  -> versioned compliance / jurisdiction policy
  -> ManeuverGate signal-layer permission
  -> future conflict / priority arbitration
  -> Traffic Runtime longitudinal safety and traversal
```

固定时制信号层提供一个 protected-entry compliance profile：

- green 对 PreGate 产生 `ProtectedAllow`；
- red 与 restrictive yellow 对 PreGate 产生 `DenyAndStop`；
- `signalControl:none` 只表示没有 signal constraint；
- green 或 none 都不得绕过 leader、route end、safe-speed、no-overlap 或未来 policy。

SignalController 不得硬编码国家、转向或道路设施特例。ADR 0009 固定这一分层，未来中国交通规则通过 versioned policy 和更完整的 maneuver/conflict domain 扩展。

## 3. 静态领域模型与 identity

### 3.1 StopLine

`StopLine` 是独立的标识域，现行只支持 `edgeEnd`：

```text
StopLineInput
  id
  edgeId
  location: edgeEnd
```

- 逻辑位置是 `edge_progress = edge.length`。
- 来源中的 `id` 遵循编译器的 ASCII token 规则；不同标识域可以复用相同文本。
- 每个 edge 最多一个 StopLine。
- 真实中段停止线由 authoring/importer 在该位置拆 edge。
- StopLine 只定义停车位置，本身不决定许可或直接修改车辆。
- geometry、线宽、颜色、材质与世界坐标属于 Adapter/Presentation。

### 3.2 ManeuverGate

`ManeuverGate` 是机动路径某一相邻边转移上的准入边界：

```text
ManeuverGate
  stableIdentity
  maneuverPath
  transitionIndex
  stopLine
  signalControl:
    group(group)
    | none
```

- `transitionIndex=i` 表示 `pathEdges[i] -> pathEdges[i+1]`，StopLine 必须绑定 `pathEdges[i]`。
- 同一 ManeuverPath 的不同 transition 可以各有独立 ManeuverGate，用于多阶段准入。
- Gate 在共享路网修订内以 `ManeuverGateOrdinal` 标识；edge pair 不是 Gate 或 Movement identity，不保留 pair-based key、别名或双重查询。
- `register_route` 把 path/Gate 编译为路线共享的 occurrences，车辆 tick 不用 edge pair 推导完整行为 identity。
- Gate 单向持有 signal binding；SignalGroup 不保存第二份 membership。
- `none` 不是 `free`、红灯右转或永久优先权，只表示 signal layer 不施加约束。

### 3.3 SignalGroup、Controller 与 Phase

```text
SignalGroupInput
  id

SignalControllerInput
  id
  kind: fixedTime
  offsetMs
  groupIds[]
  phases[]

SignalPhaseInput
  id                       // controller-local
  durationMs
  states[]

SignalGroupStateInput
  groupId
  aspect: red | yellow | green
```

- 每个 Group 必须且只能归属一个 Controller，并至少被一个 Gate 使用。
- Controller 至少有一个 Group 和一个 Phase。
- Phase ID 只在所属 Controller 内唯一；数组顺序定义循环 program。
- 每个 Phase 对 Controller 的全部 Groups 恰好列出一次 state；不允许 sparse、继承或默认值。
- 允许 all-red phase、全 red program，以及 aspect vector 相同但 ID/duration 不同的相邻 phases。
- Traffic Runtime 不强制 green/yellow/red 的交通工程 transition pattern；authoring 对 program 安全负责。
- `durationMs` 为正整数，`offsetMs` 为必填非负整数；cycle 使用 checked sum。
- `durationMs`、`offsetMs` 与 cycle 累加和都以 `2^53 - 1` 为上界。`Ms` 字段显式以毫秒调度。
- canonical offset 满足 `0 <= offsetMs < cycleDurationMs`，编译器不隐式取模。

StopLine、Gate、Group、Controller、Phase definition 和 program 在共享路网修订内不可变，不提供 signal mutation command。

## 4. Validation

静态信号数据由编译器校验，不变量以编译诊断报告，经 LFCA 与共享静态路网进入 `TrafficWorld`。

关键 domain invariants：

- StopLine ID、edge reference、one-per-edge 与 `edgeEnd` 合法；
- 同一 `(ManeuverPath, transitionIndex)` 至多一个 Gate，transition 合法，StopLine 属于该 transition 的起始边；
- terminal edge 上不得声明 StopLine；非 terminal StopLine 必须至少被一个
  ManeuverGate 引用；
- 若 StopLine 至少拥有一个 entry Gate，则其 edge 的每条 outgoing connection
  必须有 ManeuverPath coverage，且每条候选 path 必须有 entry Gate；仅拥有
  non-entry Gate 的内部 StopLine 不承担 entry coverage；
- Group ID 唯一、恰好一个 Controller owner、至少一个 Gate usage；
- Controller groups/phases 非空，Phase ID controller-local 唯一；
- state record 对 group unknown/duplicate/missing 可稳定诊断；
- duration、cycle sum、offset 满足 safe-integer 与 canonical range；
- route 不得终止在声明 StopLine 的 edge 上，由 `register_route` 校验。

`TrafficWorld::install` 按以下顺序关闭：步长必须落在 `4..=1000` ms；按
Controller/Phase 顺序检查 `durationMs >= dt` 且 `durationMs % dt == 0`；再构造
time-0 signal snapshot。短相位不得靠 tick 跳过。

## 5. Fixed-tick phase timing

已提交信号状态的时序：安装时直接解析 time 0，成功 step 先计算 `T + D` candidate，并与车辆状态一起原子提交；失败 step 不改变当前查询。

Controller 的 effective state 由 immutable program、world integer `timeMs` 和 canonical offset 直接推导：

- 使用 overflow-safe `timeMs + offset` modulo cycle；不累计浮点 timer；
- Phase interval 是 half-open `[start, end)`，恰好命中 boundary 选择后一个 Phase；
- time 0 已有有效 phase/aspect snapshot；
- Phase duration：每个 `durationMs` 必须 `>= fixedDeltaTimeMs` 且为该世界步长的
  正整数倍，否则 `install` 失败。检入走廊使用 `yellow_ms = 3008`、
  `all_red_ms = 1008`（16 ms 的正整数倍）。
- 因此每个 Controller 每 tick 最多一次 observable Phase change。

Tick 时序固定为：

```text
pre-step time T        -> vehicles 使用 signal snapshot(T)
step interval          -> [T, T + D)
post-step time T + D   -> 原子提交 signal snapshot(T + D)
next tick              -> vehicles 使用新 snapshot
```

Boundary 在 tick 内发生时，车辆观察延迟严格小于一个 fixed tick；Controller 仍按绝对时间运行，不产生 drift。

## 6. Gate compliance 与 crossing

行为表：

| Aspect / state       | PreGate 行为                           |
| -------------------- | -------------------------------------- |
| green                | `ProtectedAllow`，但物理安全约束仍生效 |
| yellow               | `DenyAndStop`                          |
| red                  | `DenyAndStop`                          |
| `signalControl:none` | 不产生 signal constraint               |

Restrictive yellow 的判断读取 tick-start snapshot。只有原子完成对应 route occurrence 的 `fromEdge -> toEdge` transition 才算 `CrossedGate`；仍在 fromEdge，包括 front bumper 精确位于 edge end，均是 PreGate。不保存 `YellowCommitted` 状态。

Red/restrictive-yellow denial 是 hard entry denial。极端不可停车输入只能通过显式 projection 和 attribution 保证合规，不得静默转换成合法 permission；不模拟概率违章或“不可避免越线”结果。

Denied Gate 允许车辆 front bumper 精确到达 StopLine，但禁止 transition。信号停车不改变 vehicle status；车辆保持 Active，速度可以为零。green 后只移除 signal constraint，车辆通过既有 controller/leader pipeline 自然恢复，不增加 startup delay。

StopLine clearance 与车队跟驰 clearance 是不同约束：首车相对 denied StopLine 的 desired/hard clearance 继续为 `0`，后车相对前车则使用 follower profile 的 `min_gap`。从可行 snapshot 出发时，红灯排队不得把后车 bumper gap 压到 `min_gap` 以下；既有 sub-min-gap 只能保持或改善。

若业务目的地位于某个 Gate 之前，authoring 必须使用一个独立、没有 StopLine 的 upstream terminal edge 表达。Route 不得终止在带 StopLine 的 edge 上，也不能借 completion 绕过 Gate。

## 7. Longitudinal constraints 与 traversal

Signal 只提供 regulatory spatial target，不直接修改 VehicleState 或 IIDM，也不创建独立 signal speed controller：

```text
tick-start snapshot
  -> occupancy / route-aware leader
  -> nearest denied Gate / SignalStop constraint
  -> IIDM comfort + safe-speed
  -> constraint reducer
  -> signal hard projection / minimum-gap-preserving geometry projection
  -> permission-aware traversal
  -> atomic commit
```

- 每辆 Active vehicle 沿确定 route occurrences，在安全/舒适 lookahead 内查找最近 denied Gate；允许跳过 permitted Gate。
- SignalStop 使用 route distance、target speed 0、front-bumper desired/hard clearance 0。
- SignalStop 的 clearance 只约束首车与 StopLine，不替代车辆间 minimum gap；后车 final geometry cap 使用 follower `min_gap`。
- 多约束按共同允许的最严格 motion 归约；numeric priority 不表达业务优先级，tie-break 只影响 attribution。
- signal hard projection 先确定 leader final travel，随后 minimum-gap-preserving geometry projection 沿 functional leader graph 从前向后传播；两者只在实际进一步收紧 final motion 时生效。
- 纵向约束集合、provider、reducer、projection solver 与 scratch 均保持 private concrete implementation。

Traversal 按 route occurrence 顺序逐个检查 Gate。单 tick 可以连续穿越多个 permitted Gates；遇到第一个 denied Gate 就停止。精确到达 denied boundary 时保留在 fromEdge occurrence，不更新 route index。

Parking 不改变 signal permission authority。ParkingStop 与 SignalStop/RouteEnd 从同一 tick-start snapshot 产生并按最严格 admissible motion 归约；exact numeric tie 仅按 Signal -> Parking -> RouteEnd 稳定归因。Parking/Signals/Following 的组合与 atomic commit 见 [`parking-system.md`][parking-system]。

## 8. 观察边界

静态信号对象是共享路网修订内的有类型序号：`StopLineOrdinal`、`SignalGroupOrdinal`、`SignalControllerOrdinal`、`SignalPhaseOrdinal` 与 `ManeuverGateOrdinal`。它们经共享静态路网的关系查询读取，无法解析时返回 `None`；序号只在所属修订内有效，没有业务排序含义。

动态状态：

- `TrafficWorld::committed_signal_groups()` 按组序号给出当前 aspect，由已提交时间推导，车辆直接消费同一 tick-start snapshot，不另建 controller clock 或第二份状态；
- `TrafficWorld::route_gate` 观察路线 hop 上的 Gate。

Gate 的 `signalControl:none` 只表示没有信号绑定；permission 也只是 signal-layer decision，不是 vehicle-specific 最终 right-of-way。`TrafficWorld` 不提供任意绝对时间查询；该能力只作为私有参考预言机用于 property tests。

## 9. 事件

信号相位与灯色变化不发布事件，调用方读取 `committed_signal_groups()`。车辆越过 Gate 由 `TrafficWorld::latest_transition_events()` 的 `GateCrossed` 观察；转移事件的语义见 [`junction-observation-and-validation.md`][junction-observation]。

## 10. Determinism 与失败原子性

Signals 沿用 ADR 0003：同一 Traffic Runtime 版本、同一运行环境、同一 normalized initial state 和同一 lifecycle/tick input sequence，逐 tick committed state、query snapshot 与 ordered events 一致；不承诺跨 CPU/编译器/浮点实现的 bit-level determinism。

不得依赖 HashMap iteration、handle 数值排序、Adapter 调用顺序或每 tick external-ID sort。Phase/route sequence 保持行为顺序。Gate 输入顺序不能成为交通优先级。

Step 的事务语义：

```text
validate tick/time
  -> read snapshot(T)
  -> derive candidate signal snapshot(T + D)
  -> compute candidate vehicle states
  -> atomically commit vehicles + signal state + tick/time
```

任一正常 `Err` 都不得改变 tick/time、vehicle 或 signal 查询结果。Candidate/scratch 可以复用 capacity，但不是 authority state。语义要求 compute-then-apply，不要求 clone 完整 world。

`TrafficWorld::install`、单条 lifecycle command 与 `register_route` 也保持 whole-operation atomic。Panic、abort、OOM 或进程终止不承诺 rollback。

## 11. Error 与 normal outcome

StopLine 的失败原因保持结构化区分：terminal edge 使用
诊断 `OrphanStopLine`；非 terminal edge 上未被任何 ManeuverGate 引用使用
`UnreferencedStopLine`；已有 entry Gate 后缺少 outgoing path/Gate
coverage 才分别使用 `MissingManeuverPathCoverage` /
`MissingManeuverGateCoverage`。编译诊断均定位到拥有该 invariant 的 StopLine，
但不得用同一误导性文案合并不同原因。

正常红/黄停车、排队、green release、有限 emergency braking、finite signal/no-overlap projection、phase/aspect change 和 `signalControl:none` 均不是 error。非法输入/引用/invariant、tick/time overflow、非法 command handle 或 non-finite runtime calculation 才返回 error。

## 12. 性能与 private implementation

标识解析、cross-reference 和序号分配只发生在编译与共享静态路网构建。`register_route` 与 `install` 必须预解析 route transition/Gate lookup 与 compact signal snapshots。

Tick hot path 要求：

- Gate lookup、Group aspect 与 current Controller state 使用 dense/indexed O(1) 路径；
- 每车不扫描全部 Gates/Controllers/Groups；
- traversal 为 `O(T_actual)`，只与实际经过的 route transitions 成正比；
- 不做 per-tick external-ID lookup/sort 或 per-vehicle allocation；
- private scratch/candidate buffer 可复用，具体容器不构成 public contract。

Signals 服从完整 Runtime step 总预算，不叠加专项预算：

- 一万 common target `<= 1 ms/tick`，hard limit `<= 4 ms/tick`，连续 60 ticks `<= 240 ms`；
- no-Signals legacy 场景相对同机 v0.3 等价基线回归 `>20%` 必须分析，`>30%` 默认阻断；
- matched all-green controlled 与 all-none 场景必须报告增量，`>20%` 必须 profile；
- 十万同时把车辆和 signal topology 放大 10 倍，一万→十万耗时比目标 `<= 20x`；
- 发现 vehicle × all-Gates/Controllers/Groups、全表扫描、per-vehicle allocation 或接近二次增长时直接阻断。

当前道路机动车执行域（`execution_domain=road_motor_vehicle`）的一万 common
workload 固定为 `N_traffic_active=10000`、100 Controllers、每 Controller 4 Groups/Gates
和 4 Phases、400 controlled routes、16 ms fixed tick、60 ticks；至少覆盖
all-green、all-none、red queue、stop/release、mixed offsets 与现有 legacy
scenarios。十万 observation 同时放大到 `N_traffic_active=100000`、1,000 Controllers、
4,000 Groups/Gates 和 matched route horizon。该车辆特化 workload 不代表其他
执行域的规模或能力。

Reference desktop 使用 optimized Criterion step benchmark；setup/parse/reset 不计入。一万使用 20 samples、十万使用 10 samples，warm-up 1 s、measurement 5 s，连续三轮读取 median point estimate 后再取三轮 median。必须记录 CPU、OS、rustc、target、power mode、commit 和 profile；环境变化时同机重跑 baseline/candidate。

十万只用于 scaling observation，不构成实时承诺。CI 只做 functional smoke 与 benchmark compile，不用共享 runner wall-clock 阻断。当时 workload 与逐轮结果见 git 历史，不把单次测量写入本文。

## 13. Tests 与现行 fixture

Signals 行为由 `TrafficWorld` 与编译器原生有类型模块覆盖。

现行必须覆盖：

- 固定时制 snapshot(T) 与成功 step 后 `committed_signal_groups` 为 T+D；
- 限制性灯色的停车距离与许可通行；
- install 时 `durationMs >= dt && durationMs % dt == 0`，短于一步与不能整除为
  可区分失败；走廊相位保持 16 ms 的正整数倍；
- 无信号编制仍可安装并步进。

## 14. 中国场景组合边界

中国场景使用 `ManeuverPath + ManeuverGate + WaitingZone + ConflictZone + versioned rule policy`
组合，不向 SignalController 增加国家/转向 if/else。以下是领域表达与扩展方向；当前
支持范围以路权策略实施合同为准，不表示所有场景均已交付：

- 左转/直行待行区：`Gate A -> waiting edge/zone -> Gate B` 的多阶段准入；
- 红灯右转：SignalAspect 输入 + jurisdiction policy + 冲突/让行判断；
- 红灯掉头：在 StopLine 前的拓扑分叉，或由 policy 判断专用 ManeuverPath；
- 右转专用通道：独立 edge/path，绑定独立 group、`none` 或显式让行策略；
- 无保护左转/让行绿灯：permissive indication + conflict set + gap acceptance；
- 无信号优先级：独立 priority/sign/jurisdiction policy，不伪装成 SignalGroup；
- 中段 StopLine：未来扩展 `edgeProgress`，或继续通过拆 edge authoring。

#235 的 Accepted 设计
[`waiting-zone-conflict-right-of-way.md`][waiting-zone]
已把上述组合细化为 registration-time Gate/Waiting/Conflict occurrence、车辆级
Gate decision、Traffic Runtime ConflictArbiter 与 tick-local grant/reservation。该设计保持
本文 Controller -> indication -> policy -> conflict -> safety 顺序，并要求 pinned
policy/profile 在初始化时拒绝同 phase simultaneous Protected 的 incompatible
Gate coverage；runtime reservation 不是错误 signal authoring 的降级机制。正式生产路径
已共同实现 Waiting、策略绑定与 Conflict 组合仲裁；#285 的最终跨层场景和 Adapter
debug surface 仍独立交付，不能由运行时实现替代。

法规行为必须由明确版本、适用地区与可审计依据驱动。#284 的中国机动车红灯右转
策略、法规来源与支持边界以
[`waiting-zone-conflict-right-of-way.md` §6.2.1][waiting-zone-red-turn]
为准；法规版本通过 policy pin 固定，不把“最新法规”或页面链接硬编码为 runtime 规则。

## 15. 走廊 protected-turning profile

#196 已在
[`signalized-corridor-protected-turning.md`][protected-turning]
冻结每个 Junction 的 `main-left`、`main-through-right`、`secondary-left`、
`secondary-through-right` 四个 group、完整 12-phase/84 秒 program、32 个
group-controlled entry ManeuverGate 和 authoring compatibility matrix。该 profile
不扩展 SignalController runtime authority。

[parking-system]: parking-system.md
[junction-observation]: junction-observation-and-validation.md
[waiting-zone]: waiting-zone-conflict-right-of-way.md
[waiting-zone-red-turn]: waiting-zone-conflict-right-of-way.md#621-中国机动车红灯右转的最小策略
[protected-turning]: signalized-corridor-protected-turning.md
