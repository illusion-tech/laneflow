# 0009 Signal Indication, Movement Gate, and Policy Separation

**状态**: Accepted  
**日期**: 2026-07-15  
**适用范围**: LaneFlow Signals 的 indication、空间准入边界、法规策略、冲突仲裁与 Core safety 所有权  
**关联文档**:

- 上游决策:
  - `0001-project-scope.md`
  - `0003-runtime-tick-and-determinism.md`
  - `0005-core-identity-and-handle-model.md`
  - `0006-vehicle-following-control-and-safety.md`
  - `0007-traffic-data-crate-and-loader-boundary.md`
  - `0008-pre-1.0-data-format-version-policy.md`
- 详细设计:
  - `../design/signal-system.md`
  - `../design/vehicle-following.md`
  - `../design/data-format.md`
  - `../design/data-loading.md`

## 背景

v0.4 需要让车辆按固定配时信号在 StopLine 前停车和放行。若把 red/yellow/green 直接定义成永久、全球统一的最终通行权，后续红灯右转、红灯掉头、无保护左转、待行区、无信号优先级和不同地区法规会被迫成为 SignalController 内的特例。

同时，SignalStop 必须接入 v0.3 已冻结的 longitudinal safety pipeline。Adapter、法规策略或未来 controller 扩展都不能绕过 leader、safe-speed 与 no-overlap。

LaneFlow 因此需要区分“灯显示什么”“空间边界在哪里”“当前 policy 如何解释”“物理安全是否允许”，并避免把 v0.4 的 protected-only 范围误写成 1.0 后的终局模型。

## 决策

### 1. SignalAspect 只是 indication

`red | yellow | green` 表达 SignalGroup 的当前 indication。它是 compliance/jurisdiction policy 的输入，不直接构成面向未来的最终 right-of-way。

v0.4 提供一个明确的 protected-entry profile：green 允许 PreGate 进入；red 与 restrictive yellow 拒绝 PreGate；已原子跨越 Gate 的车辆继续清空。这是 v0.4 profile，不是 aspect 的全球永久定义。

### 2. StopLine 与 MovementGate 显式分离

StopLine 表达车辆应停止的空间位置。MovementGate 表达 directed connection 的准入边界，并以 `(fromEdge, toEdge)` 作为 v0.4 value identity。

Gate 显式引用 StopLine 与 signal binding；StopLine 本身不决定许可，SignalGroup 也不保存第二份 Gate membership。`signalControl:none` 只表示没有 v0.4 signal constraint，不表示永久自由通行。

Gate pair 不冻结为未来完整 Movement identity。需要多阶段、区域或冲突语义时，使用 ManeuverPath、WaitingZone、ConflictZone 等独立 domain 扩展。

### 3. Policy 与 conflict arbitration 不进入 SignalController

SignalController 只拥有 immutable program、phase timing 与 Group aspect。它不得包含国家、地区、转向、道路标志或车辆类别特例。

未来 permissive movement、中国红灯右转/掉头、无保护左转、待行区和无信号优先级由 versioned jurisdiction/compliance policy、maneuver/conflict domain 与 gap/reservation 规则组合表达。法规来源、版本和适用地区必须可审计。

### 4. Core safety 始终不可绕过

Signal 或未来 policy 通过 longitudinal constraint 与 Gate permission 进入 Core。green/allow 只能移除对应 regulatory constraint，不能覆盖 leader、route end、safe-speed、no-overlap 或其他更严格约束。

Constraint provider、reducer、projection solver 与 final vehicle-specific decision 保持 Core 私有。Adapter 只能 query/render，不能推进 controller、覆盖 permission 或注入绕过安全层的结果。

### 5. v0.4 只交付 protected-entry 最小闭环

v0.4 固定使用 static fixed-time cyclic controller、explicit StopLine/MovementGate 和 protected-entry profile。Authoring 负责保证同 phase protected Gates 可以安全并行；Core 不在 v0.4 推导 foe stream 或 conflict-free geometry。

在 SignalStop 与 permission-aware traversal 一次性交付前，必须用 capability activation guard 阻止“非空 Signals + 移动车辆”产生静默闯灯语义。

## 后果

正向后果：

- v0.4 可以在没有 conflict solver 的前提下交付完整、可验证的基础信号闭环。
- indication、空间边界、法规解释与物理安全各自有单一职责。
- 中国与其他地区规则可以通过 versioned policy 扩展，不污染 controller timing。
- Adapter 可通过只读 query/events 表现灯态，但不能成为交通规则权威。
- Core 保留统一 constraint/safety pipeline，可继续进行数据导向和高性能实现。

成本与风险：

- `signalControl:none` 不能被简化成“自由通行”，调用方必须理解它只是 signal-layer 状态。
- v0.4 authoring 需要显式 Gate coverage，并承担 protected phases 的冲突自由责任。
- permissive movement 和真实复杂中国路口仍需要 ManeuverPath、ConflictZone、policy 与 gap acceptance 等后续设计。
- Gate pair 不是长期 Movement identity，未来扩展时可能新增更完整的 domain type。

## 替代方案

### 把 aspect 直接定义成最终通行权

实现最简单，但会把 green/yellow/red 的地区差异、让行和冲突规则永久耦合到 controller，因此拒绝。

### 把 StopLine 直接挂在 SignalGroup 上

无法清晰表达一个 StopLine 上不同 outgoing connections 的不同控制，也会混淆停车位置与准入边界，因此拒绝。

### 在 v0.4 同时实现 permissive conflict solver

会立即要求 junction geometry、conflict set、priority、gap acceptance 或 reservation，超出当前 lane graph 和 v0.4 范围，因此延后。

### 由 Adapter 决定车辆是否通过

会导致不同引擎复制并漂移交通规则，且可以绕过 Core safety，因此拒绝。

### 为中国规则在 Controller 中增加专用字段

短期可表达个别路口，但无法组合道路标志、车辆类别、时间、冲突和地区版本，长期不可维护，因此拒绝。

## 实施与复核

- #93：冻结本 ADR 与 `../design/signal-system.md`。
- #94：static signal domain、current 0.4 data contract 与 capability guard。
- #95：fixed-time runtime、query 与 events，保持 guard。
- #96：SignalStop、hard projection、permission-aware traversal 与 guard 解除。
- #97：端到端 determinism/property/performance evidence。
- #18：v0.4 最终全面审阅与收口。

若未来让 SignalController 直接决定 jurisdiction/conflict、允许 Adapter 绕过 Core safety、改变 Gate/StopLine 分层或公开可替换的 policy/controller ABI，应新增或 supersede 本 ADR，不得静默改写。
