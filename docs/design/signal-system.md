# Signal System 设计

**文档状态**: Accepted<br>
**最后更新**: 2026-07-17<br>
**适用范围**: v0.4 Signals 的静态领域、fixed-time runtime、车辆合规、Core API、数据契约、验证与性能边界，以及 current v0.5 package embedding<br>
**实现状态**: #94-#97 已完成 v0.4 Signals 全链路与收口；#107 已将保持相同 Signals shape/behavior 的 package 原子迁移到 current v0.5，并由新的 Parking+Signals fixtures 承接 active contract

**关联文档**:

- `../architecture.md`
- `../roadmap.md`
- `../adr/0003-runtime-tick-and-determinism.md`
- `../adr/0005-core-identity-and-handle-model.md`
- `../adr/0006-vehicle-following-control-and-safety.md`
- `../adr/0007-traffic-data-crate-and-loader-boundary.md`
- `../adr/0008-pre-1.0-data-format-version-policy.md`
- `../adr/0009-signal-indication-gate-and-policy-separation.md`
- `data-format.md`
- `data-loading.md`
- `route-system.md`
- `vehicle-following.md`
- `parking-system.md`

## 1. 目标、状态与非目标

v0.4 的目标是在 v0.3 fixed tick、route occurrence、Vehicle Following 和不可绕过的纵向安全层上，交付可验证的基础信号灯闭环：

- static、fixed-time、cyclic SignalController；
- directed connection 上的显式 MovementGate 与 StopLine；
- red / yellow / green indication 和 protected-entry profile；
- 红灯/限制性黄灯停车、排队、绿灯放行；
- permission-aware route traversal；
- 可供 Adapter/debug 使用的只读 query 与稀疏事件；
- current 0.4 schema/loader/Core normalization、确定性、失败原子性与性能证据。

本文是 #93 的 Accepted 设计输入，不是实现完成声明。实施切片为 #94、#95、#96、#97；只有它们分别完成 Gate 后，相应能力才成为 production 事实。

v0.4 明确不实现：

- permissive green、无保护左转、红灯右转、红灯掉头；
- 左转/直行待行区的专用语义；
- 无信号路口优先级、gap acceptance、conflict graph/zone、reservation；
- 行人/非机动车冲突仲裁、环岛或 merge priority；
- actuated/adaptive controller、detector、program switching、off/flashing；
- Engine Adapter ABI、灯具 geometry/transform、渲染动画或 authoring UI；
- 专业交通工程配时优化或外部仿真器兼容层。

## 2. 长期分层与 v0.4 profile

SignalAspect 只是 indication，不是面向未来的最终通行权。完整决策链分为：

```text
SignalController
  -> SignalAspect
  -> versioned compliance / jurisdiction policy
  -> MovementGate signal-layer permission
  -> future conflict / priority arbitration
  -> Core longitudinal safety and traversal
```

v0.4 只提供一个 protected-entry compliance profile：

- green 对 PreGate 产生 `ProtectedAllow`；
- red 与 restrictive yellow 对 PreGate 产生 `DenyAndStop`；
- `signalControl:none` 只表示没有 v0.4 signal constraint；
- green 或 none 都不得绕过 leader、route end、safe-speed、no-overlap 或未来 policy。

SignalController 不得硬编码国家、转向或道路设施特例。ADR 0009 固定这一分层，未来中国交通规则通过 versioned policy 和更完整的 maneuver/conflict domain 扩展。

## 3. 静态领域模型与 identity

### 3.1 StopLine

`StopLine` 是独立 external-ID domain。v0.4 只支持：

```text
StopLineInput
  id
  edgeId
  location: edgeEnd
```

- 逻辑位置是 `edge_progress = edge.length`。
- `id` 沿用 Core external ID 的严格 ASCII token 规则；不同 ID domain 可以复用相同文本。
- 每个 edge 最多一个 StopLine。
- 真实中段停止线由 authoring/importer 在该位置拆 edge。
- StopLine 只定义停车位置，本身不决定许可或直接修改车辆。
- geometry、线宽、颜色、材质与世界坐标属于 Adapter/Presentation。

### 3.2 MovementGate

`MovementGate` 是 directed connection 上的准入边界：

```text
MovementGateInput
  fromEdgeId
  toEdgeId
  stopLineId
  signalControl:
    group(groupId)
    | none
```

- value identity 是 `(fromEdgeId, toEdgeId)`；Core normalization 后使用 `(EdgeHandle, EdgeHandle)`。
- 该 pair 只是 Gate identity，不是 1.0 后完整 Movement identity。
- Gate 必须引用属于 `fromEdgeId` 的 StopLine，且 pair 必须是合法 connection。
- 声明 StopLine 的 edge，其每个 outgoing connection 必须恰好有一个 Gate。
- Gate 单向持有 signal binding；SignalGroup 不保存第二份 membership。
- `none` 不是 `free`、红灯右转或永久优先权，只表示 v0.4 signal layer 不施加约束。

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
- Core 不强制 green/yellow/red 的交通工程 transition pattern；authoring 对 program 安全负责。
- `durationMs` 为正整数，`offsetMs` 为必填非负整数；cycle 使用 checked sum。
- canonical offset 满足 `0 <= offsetMs < cycleDurationMs`，loader 不隐式 modulo。

StopLine、Gate、Group、Controller、Phase definition 和 program 在 world 生命周期内不可变。v0.4 不提供 signal mutation command。

## 4. Historical v0.4 contract 与 current v0.5 embedding

### 4.1 Current data facts

ADR 0008 要求 active tree 只维护一个 current format。#94 曾原子交付 v0.4；#107 已在保持 Signals static/runtime semantics 不变的前提下迁移到 v0.5：

- production current 是 `formatVersion: "0.5"`；
- `schemas/laneflow-data-v0.5.schema.json` 是唯一 active schema；
- production loader 明确拒绝 v0.4 及更早版本、未来版、旧字段与 JSON-LD；
- static Signals、fixed-time runtime 与完整车辆合规仍是 production 行为；v0.4 收口证据继续作为历史行为/性能基线。

### 4.2 ID 与引用命名

0.4 全包统一采用：

- 实体自身标识：`id`；
- 单个实体引用：`xxxId`；
- 多个实体引用：`xxxIds`；
- `xxxRef` 保留给未来结构化、URI/IRI 或跨文件引用。

因此 0.4 直接迁移同时包含 `connections[].to -> toEdgeId` 与 `routes[].edges -> edgeIds`。不同 ID domain 仍可复用相同 token，不建立单一全局命名空间。

### 4.3 Canonical JSON shape

`signals` 与四个子数组在 current 0.5 中继续必填，数组允许为空。概念 shape：

```json
{
  "formatVersion": "0.5",
  "units": { "distance": "meter", "time": "second" },
  "laneGraph": {
    "edges": [
      {
        "id": "edge-a",
        "length": 100.0,
        "connections": [{ "toEdgeId": "edge-b" }]
      },
      {
        "id": "edge-b",
        "length": 50.0,
        "connections": []
      }
    ]
  },
  "routes": [{ "id": "route-main", "edgeIds": ["edge-a", "edge-b"] }],
  "vehicleProfiles": [],
  "signals": {
    "stopLines": [
      { "id": "stop-a", "edgeId": "edge-a", "location": "edgeEnd" }
    ],
    "movementGates": [
      {
        "fromEdgeId": "edge-a",
        "toEdgeId": "edge-b",
        "stopLineId": "stop-a",
        "signalControl": { "kind": "group", "groupId": "group-main" }
      }
    ],
    "groups": [{ "id": "group-main" }],
    "controllers": [
      {
        "id": "controller-main",
        "kind": "fixedTime",
        "offsetMs": 0,
        "groupIds": ["group-main"],
        "phases": [
          {
            "id": "phase-green",
            "durationMs": 30000,
            "states": [{ "groupId": "group-main", "aspect": "green" }]
          }
        ]
      }
    ]
  },
  "parking": {
    "areas": [],
    "spaces": []
  }
}
```

无信号 current package 仍必须显式提供四个空数组。`signalControl` 是 closed tagged union：`{ "kind": "group", "groupId": "..." }` 或 `{ "kind": "none" }`。

`durationMs` / `offsetMs` 使用 JSON integer，并以 `2^53 - 1` 为 portable safe-integer 上界；cycle checked sum也不得超过该上界。该 invariant 由 Core construction 定义并由 data layer 复用，不能形成两套上界规则。`Ms` 字段显式以毫秒调度，不改变 `units.time = "second"` 对物理时间参数的语义。

顶层 `extensions` 可以继续存在，但不得承载 StopLine、Gate、Controller 或其他影响 Core 行为的 Signals 数据。

Current canonical/runtime 输入保持严格普通 JSON，不接受 `@context` 或通用 JSON-LD。未来若有跨文件/ontology 需求，JSON-LD 只能作为独立、离线、版本化的 importer/exporter profile，转换后仍须通过 canonical JSON schema 与 Core normalization。

## 5. Normalization 与 validation

依赖方向保持：

```text
laneflow-data -> laneflow-core
laneflow-core -X-> laneflow-data
```

Production fail-fast 顺序为：

```text
JSON syntax
  -> minimal formatVersion shape
  -> exact current-version check
  -> strict current DTO shape
  -> units
  -> Core domain normalization
  -> CoreWorld fixed-delta/runtime compatibility
```

旧版/未来版必须在 0.5 shape、unknown-field、units 和 domain validation 前返回 `UnsupportedFormatVersion`。Schema/Serde 负责 required/type/closed shape、tagged union、enum 与单字段 integer range；Core constructors 是引用、唯一性、ownership、coverage、cycle、route 和 runtime invariant 的唯一事实源。

Core normalization 的 canonical 顺序：

```text
vehicleProfiles
laneGraph edges/connections
signals.stopLines
signals.groups
signals.controllers/phases/states
signals.movementGates
signal global coverage/ownership/usage
routes + final-StopLine rule
InitialTrafficData final assembly
```

关键 domain invariants：

- StopLine ID、edge reference、one-per-edge 与 `edgeEnd` 合法；
- Gate pair 唯一、是合法 connection、StopLine 属于 fromEdge；
- StopLine edge 的 outgoing Gates 覆盖完整，StopLine 不 orphan；
- Group ID 唯一、恰好一个 Controller owner、至少一个 Gate usage；
- Controller groups/phases 非空，Phase ID controller-local 唯一；
- state record 对 group unknown/duplicate/missing 可稳定诊断；
- duration、cycle sum、offset 满足 safe-integer 与 canonical range；
- route 不得终止在声明 StopLine 的 edge 上，initial route 与 runtime `register_route` 复用同一规则。

First-error 顺序同样是 contract：array domain error 按输入顺序；duplicate 锚定第二个 occurrence；Phase state 先按 record 顺序报告 unknown/duplicate group，再按 `groupIds` 顺序报告第一个 missing group；global coverage/usage 按 StopLine、Group、Controller normalization order；Route 按 route/`edgeIds` 顺序。

当前 world compatibility 按以下顺序执行：验证 positive fixed delta；按 Controller/Phase normalization order 检查 `durationMs >= fixedDeltaTimeMs`；构造 time-0 signal snapshot；注册 initial routes；按既有 overlap 规则校验并创建 initial vehicles；最后发布 world。#96 已用 SignalStop、hard projection 与 permission-aware traversal 的完整车辆合规替代 capability guard，non-empty Signals 可与 initial/runtime vehicles 组合。

`InitialTrafficData` 已包含 immutable signal registry，并在组装时按自身 `LaneGraph` 重绑定和复验 graph-dependent handles。Core 保留不经 JSON 的 programmatic construction path；runtime handles 永不持久化到 external package。

## 6. Fixed-tick phase timing

Issue #95 已按本节交付 committed authority snapshot：初始化直接解析 time 0，成功 step 先计算 `T + D` candidate，再在所有 vehicle events 之后产生 signal events 并原子提交；失败 step 不改变当前 query。

Controller 的 effective state 由 immutable program、world integer `timeMs` 和 canonical offset 直接推导：

- 使用 overflow-safe `timeMs + offset` modulo cycle；不累计浮点 timer；
- Phase interval 是 half-open `[start, end)`，恰好命中 boundary 选择后一个 Phase；
- time 0 已有有效 phase/aspect snapshot，初始化不发 change event；
- Phase duration 不要求整除 fixed delta；
- world 初始化要求每个 `durationMs >= fixedDeltaTimeMs`；
- 因此每个 Controller 每 tick 最多一次 observable Phase change；
- Phase identity 改变即产生 Phase event，即使 aspect vector 相同；single-phase wrap 不产生。

Tick 时序固定为：

```text
pre-step time T        -> vehicles 使用 signal snapshot(T)
step interval          -> [T, T + D)
post-step time T + D   -> 原子提交 signal snapshot(T + D)
next tick              -> vehicles 使用新 snapshot
```

Boundary 在 tick 内发生时，车辆观察延迟严格小于一个 fixed tick；Controller 仍按绝对时间运行，不产生 drift。

## 7. Gate compliance 与 crossing

v0.4 行为表：

| Aspect / state       | PreGate 行为                           |
| -------------------- | -------------------------------------- |
| green                | `ProtectedAllow`，但物理安全约束仍生效 |
| yellow               | `DenyAndStop`                          |
| red                  | `DenyAndStop`                          |
| `signalControl:none` | 不产生 signal constraint               |

Restrictive yellow 的判断读取 tick-start snapshot。只有原子完成对应 route occurrence 的 `fromEdge -> toEdge` transition 才算 `CrossedGate`；仍在 fromEdge，包括 front bumper 精确位于 edge end，均是 PreGate。不保存 `YellowCommitted` 状态。

Red/restrictive-yellow denial 是 hard entry denial。极端不可停车输入只能通过显式 projection 和 attribution 保证合规，不得静默转换成合法 permission；v0.4 不模拟概率违章或“不可避免越线”结果。

Denied Gate 允许车辆 front bumper 精确到达 StopLine，但禁止 transition。信号停车不把 vehicle status 改为 `Stopped`；车辆保持 Active，速度可以为零。green 后只移除 signal constraint，车辆通过既有 controller/leader pipeline 自然恢复，不增加 startup delay。

若业务目的地位于某个 Gate 之前，authoring 必须使用一个独立、没有 StopLine 的 upstream terminal edge 表达。Route 不得终止在带 StopLine 的 edge 上，也不能借 completion 绕过 Gate。

## 8. Longitudinal constraints 与 traversal

Signal 只提供 regulatory spatial target，不直接修改 VehicleState 或 IIDM，也不创建独立 signal speed controller：

```text
tick-start snapshot
  -> occupancy / route-aware leader
  -> nearest denied Gate / SignalStop constraint
  -> IIDM comfort + safe-speed
  -> constraint reducer
  -> signal hard projection / no-overlap
  -> permission-aware traversal
  -> atomic commit
```

- 每辆 Active vehicle 沿确定 route occurrences，在安全/舒适 lookahead 内查找最近 denied Gate；允许跳过 permitted Gate。
- SignalStop 使用 route distance、target speed 0、front-bumper desired/hard clearance 0。
- 多约束按共同允许的最严格 motion 归约；numeric priority 不表达业务优先级，tie-break 只影响 attribution。
- signal hard projection 先于 no-overlap geometry projection；两者只有实际进一步收紧 final motion 才产生各自事件。
- `LongitudinalConstraintSet`、provider、reducer、projection solver 与 scratch 均保持 private concrete implementation。

Traversal 按 route occurrence 顺序逐个检查 Gate。单 tick 可以连续穿越多个 permitted Gates；遇到第一个 denied Gate 就停止。精确到达 denied boundary 时保留在 fromEdge occurrence，不更新 route index，不产生 `VehicleChangedEdge`。

Current v0.5 Parking 不改变 signal permission authority。#109 已让 ParkingStop 与 SignalStop/RouteEnd 从同一 tick-start snapshot 产生并按最严格 admissible motion 归约；exact numeric tie 仅按 Signal -> Parking -> RouteEnd 稳定归因。Parking/Signals/Following 共同 event order 与 atomic commit 见 [`parking-system.md`](parking-system.md)；该 runtime composition 已激活。

### 8.1 Capability activation

#96 已原子交付 SignalStop、hard projection 与 permission-aware traversal，并在专项行为、事件顺序和失败原子性测试通过后解除以下 capability guard：

```text
non-empty Signals + spawned/moving vehicles
```

public runtime 现在允许 Signals 与 initial/runtime vehicles 组合；车辆始终消费同一 tick-start authority snapshot，不存在“灯正常运行但车辆静默忽略信号”的公开中间能力。

## 9. Public Core observation boundary

#94 已公开 static Signals 的 world-scoped opaque types：

- `StopLineHandle`；
- `SignalGroupHandle`；
- `SignalControllerHandle`；
- controller-local `SignalPhaseRef`；
- value identity `MovementGateKey { from_edge, to_edge }`。

Handle/ref 至少满足 `Clone + Copy + Debug + Eq + Hash`，不公开 index/generation、没有 `Ord`，不得跨 CoreWorld 持久化或混用。无法解析的只读 handle/ref query 返回 `None`。

当前 static query 提供：

- StopLine/Group/Controller external-ID resolver 与稳定 definition iteration；
- Phase external-ID resolver；
- Gate 的 StopLine 与 control binding。

#95 已在已提交 snapshot 上增加 current Controller state（phase、cycle position、elapsed、remaining）、current Group aspect，以及 Gate aspect / signal-layer permission，并提供 normalization-order 的无分配只读遍历。#96 直接消费同一 tick-start snapshot，不另建 controller clock 或第二份 authority state。

Gate `Uncontrolled` 只表示 `signalControl:none`；permission 也只是 signal-layer decision，不是未来 vehicle-specific 最终 right-of-way。Public CoreWorld 不提供 arbitrary absolute-time query；该能力仅作为 private/reference oracle 用于 property tests。

## 10. Events 与全局总序

稀疏事件边界如下；前两项已由 #95 交付，第三项由 #96 交付：

- `SignalPhaseChanged { tickIndex, controller, fromPhase, toPhase }`；
- `SignalGroupAspectChanged { tickIndex, group, fromAspect, toAspect }`；
- `VehicleSignalStopProjectionApplied { tickIndex, vehicle, route, fromRouteEdgeIndex, toRouteEdgeIndex, gate, stopLine, group, aspect }`。

Payload 只使用 handles、route occurrence indices、enum 与整数，不携带 external strings 或 `f64`。Projection event 只在 signal hard boundary 把 travel 进一步压到 emergency envelope 以下时产生；正常停车、排队和释放不产生。

不新增 GateCrossed、每 tick GateDenied、VehicleStoppedAtSignal 或 SignalReleased。Gate crossing 已由 `VehicleChangedEdge` 表达；连续状态由 query 获取。

成功 step 的全局事件总序：

```text
for vehicle in stable vehicleUpdateOrder:
  VehicleSignalStopProjectionApplied?
  -> VehicleFollowingSafetyProjectionApplied?
  -> VehicleChangedEdge*
  -> VehicleCompletedRoute?

for controller in signal normalization order:
  SignalPhaseChanged?
  -> SignalGroupAspectChanged*       // controller.groupIds 输入顺序
```

Events 使用 post-step tick index；time 由 `StepResult` 提供。Events 只是 transition notification，CoreWorld post-step query 才是权威当前状态。Adapter 使用“time-0 query 初始化 -> events 增量更新 -> query resync”模式。

## 11. Determinism 与失败原子性

Signals 沿用 ADR 0003：同一 Core 版本、同一运行环境、同一 normalized initial state 和同一 lifecycle/tick input sequence，逐 tick committed state、query snapshot 与 ordered events 一致；不承诺跨 CPU/编译器/浮点实现的 bit-level determinism。

不得依赖 HashMap iteration、handle 数值排序、Adapter 调用顺序或每 tick external-ID sort。Controller event order 使用 controller normalization order；Group event order使用 `groupIds`；Phase/route sequence 保持行为顺序。Gate 输入顺序不能成为交通优先级。

Step 的事务语义：

```text
validate tick/time
  -> read snapshot(T)
  -> derive candidate signal snapshot(T + D)
  -> compute candidate vehicle states/events
  -> build phase/aspect events
  -> atomically commit vehicles + signal state + tick/time
```

任一正常 `Err` 都不得改变 tick/time、vehicle、signal query cache、resolver 或 events。Candidate/scratch 可以复用 capacity，但不是 authority state。语义要求 compute-then-apply，不要求 clone 完整 world。

Loader、InitialTrafficData、CoreWorld initialization、单条 lifecycle command 与 `register_route` 也保持 whole-operation atomic。Panic、abort、OOM 或进程终止不承诺 rollback。

## 12. Error 与 normal outcome

Error ownership 保持：

```text
DataError
  JSON/version/unit/path
  CoreDomain { path, source: CoreError }

CoreError
  construction/domain/world-compatibility/runtime invariant
```

Data path 采用 `$` 根 + dot/bracket 风格，并继续使用 `xxxId/xxxIds` 字段，例如 `signals.controllers[0].phases[1].states[2].groupId`。Machine matching 使用 enum variant/字段，不解析 Display 文案。

正常红/黄停车、排队、green release、有限 emergency braking、finite signal/no-overlap projection、phase/aspect change 和 `signalControl:none` 均不是 error。非法输入/引用/invariant、tick/time overflow、非法 command handle 或 non-finite runtime calculation 才返回 error。

## 13. 性能与 private implementation

所有 external ID、JSON、cross-reference 和 handle allocation 只发生在 load/world initialization。Normalization 必须预解析 route transition/Gate lookup 与 compact signal snapshots。

Tick hot path 要求：

- Gate lookup、Group aspect 与 current Controller state 使用 dense/indexed O(1) 路径；
- 每车不扫描全部 Gates/Controllers/Groups；
- traversal 为 `O(T_actual)`，只与实际经过的 route transitions 成正比；
- 不做 per-tick external-ID lookup/sort 或 per-vehicle allocation；
- private scratch/candidate buffer 可复用，具体容器不构成 public contract。

Signals 服从完整 Core step 总预算，不叠加专项预算：

- 10k common target `<= 1 ms/tick`，G3 hard limit `<= 4 ms/tick`，连续 60 ticks `<= 240 ms`；
- no-Signals legacy 场景相对同机 v0.3 等价基线回归 `>20%` 必须分析，`>30%` 默认阻断；
- matched all-green controlled 与 all-none 场景必须报告增量，`>20%` 必须 profile；
- 100k 同时把车辆和 signal topology 放大 10 倍，10k→100k 耗时比目标 `<= 20x`；
- 发现 vehicle × all-Gates/Controllers/Groups、全表扫描、per-vehicle allocation 或接近二次增长时直接阻断。

10k common workload 固定为 10,000 Active vehicles、100 Controllers、每 Controller 4 Groups/Gates 和 4 Phases、400 controlled routes、16 ms fixed tick、60 ticks；至少覆盖 all-green、all-none、red queue、stop/release、mixed offsets 与现有 legacy scenarios。100k observation 同时放大到 100,000 vehicles、1,000 Controllers、4,000 Groups/Gates 和 matched route horizon。

Reference desktop 使用 optimized Criterion step benchmark；setup/parse/reset 不计入。10k 使用 20 samples、100k 使用 10 samples，warm-up 1 s、measurement 5 s，连续三轮读取 median point estimate 后再取三轮 median。必须记录 CPU、OS、rustc、target、power mode、commit 和 profile；环境变化时同机重跑 baseline/candidate。

100k 只用于 scaling observation，不构成实时承诺。CI 只做 functional smoke 与 benchmark compile，不用共享 runner wall-clock 阻断。具体 workload、环境和结果见 [`v0.4-signals-validation.md`](../reference/v0.4-signals-validation.md)，不把单次测量写入本文。

## 14. Tests 与 canonical fixtures

测试矩阵必须覆盖：

- schema/DTO/loader：current version、closed shape、tagged union、safe integer、旧字段/JSON-LD 拒绝、path/source；
- Core domain：identity/reference/coverage/ownership/complete state/cycle/route-final-StopLine；
- timing/query：time 0、boundary、offset、non-divisible delta、single-phase wrap、overflow 与失败原子性；
- vehicle behavior：green/red/yellow、exact boundary、nearest denied Gate、多 Gate、repeated edge、queue/release、shared StopLine；
- events/determinism：全局总序、dual projection、multi-controller、replay 与 fresh-world retry；
- property：1-8 groups/phases、boundary/wrap/long-time/near-overflow，对照独立 `u128` reference resolver；
- performance：10k common/stress、matched all-green/none/no-signals、legacy regression 与 100k scaling。

#107 拥有两个 current-only fixtures；Signals 端到端测试直接消费，不复制：

1. `v0.5-parking-signals-baseline.laneflow.json`：完整 StopLine/Gates、group/none、green/yellow/red program 与 static Parking；route 在无 StopLine downstream edge 终止。其 `none` Gate 只验证 signal-layer 无约束语义，不表达红灯右转法规。
2. `v0.5-empty-signals-and-parking.laneflow.json`：Signals 四数组和 Parking 两数组显式为空，证明无信号数据仍是 current 0.5 的合法输入。

## 15. 实施切片与退出边界

```text
#93 design/ADR
  -> #94 static domain + current 0.4 data + fixtures
  -> #95 fixed-time runtime + query/events
  -> #96 SignalStop + projection + permission traversal
  -> #97 end-to-end validation + performance
  -> #18 milestone closure
```

- #94 原子完成 current-format 切换并建立 capability guard。
- #95 交付 runtime/query/events，但不解除 guard。
- #96 在完整车辆合规闭环通过后解除 guard。
- #97 固化验证与性能证据。
- #18 在所有子 Issue 完成后进行最终全面审阅和收口。

## 16. 1.0 后中国场景扩展

未来扩展使用 `ManeuverPath + MovementGate + WaitingZone + ConflictZone + versioned rule policy`，而不是向 SignalController 增加国家/转向 if/else：

- 左转/直行待行区：`Gate A -> waiting edge/zone -> Gate B` 的多阶段准入；
- 红灯右转：SignalAspect 输入 + jurisdiction policy + 冲突/让行判断；
- 红灯掉头：在 StopLine 前的拓扑分叉，或由 policy 判断专用 ManeuverPath；
- 右转专用通道：独立 edge/path，绑定独立 group、`none` 或 future yield policy；
- 无保护左转/让行绿灯：permissive indication + conflict set + gap acceptance；
- 无信号优先级：独立 priority/sign/jurisdiction policy，不伪装成 SignalGroup；
- 中段 StopLine：未来扩展 `edgeProgress`，或继续通过拆 edge authoring。

法规行为必须由明确版本、适用地区与可审计依据驱动。中国现行信号通行语义的正式来源之一是[《中华人民共和国道路交通安全法实施条例》](https://www.samr.gov.cn/zljds/zcfg/art/2023/art_5c212e15369443b3b2bea4e17a1c565b.html)；未来实现仍需在对应版本立项时重新核验，不把当前链接永久硬编码为 runtime 规则。
