# 0006 Vehicle Following Control and Safety Architecture

**状态**: Accepted  
**日期**: 2026-07-12  
**适用范围**: LaneFlow Core 的纵向决策分层、Vehicle Following 控制模型、安全所有权与扩展边界  
**后续修订**: ADR 0008 已替代本文第 5 节中保留 v0.2 schema 的版本兼容策略；Vehicle Profile 与安全架构决策继续有效  
**关联文档**:

- 上游决策:
  - `0001-project-scope.md`
  - `0002-dependency-and-licensing-constraints.md`
  - `0003-runtime-tick-and-determinism.md`
  - `0004-core-implementation-language.md`
  - `0005-core-identity-and-handle-model.md`
  - `0008-pre-1.0-data-format-version-policy.md`
- 详细设计:
  - `../design/vehicle-following.md`
  - `../design/lane-graph.md`
  - `../design/route-system.md`
  - `../design/data-format.md`

## 背景

LaneFlow v0.2 已稳定 fixed tick、typed handle、lane graph、route 和外部数据格式，但车辆仍按固定速度独立推进。v0.3 需要支持前车检测、平滑加减速、拥堵停止与恢复，同时保持 Core 引擎无关、确定性和失败原子性。

单独采用连续时间 IDM/IIDM 不能在离散 fixed tick、极端初始间距或异常制动条件下构成严格的 no-overlap 保证。另一方面，只采用保守 safe-speed 会损失正常驾驶的平滑性。若将控制器作为 public trait 暴露，还会过早冻结 observation、状态所有权、线程约束、错误隔离和跨语言扩展协议。

LaneFlow 的长期目标还包括 signals、intersection、parking 和更大规模城市运行时。Vehicle Following 不能直接拥有这些领域规则，也不能让未来约束绕过 Core 的安全层。

## 决策

### 1. 纵向决策采用分层架构

LaneFlow 将交通决策分为 route、maneuver/lane、longitudinal、conflict 和 presentation 层。

- route 与 maneuver 可以低频或事件驱动更新。
- longitudinal controller 在每个 fixed tick 更新。
- signals、intersection、parking 和 incident 通过约束或独立状态进入，不复制 IIDM。
- presentation 和 render interpolation 继续属于 Adapter。

v0.3 只实现 longitudinal Vehicle Following，不实现 lane changing、intersection conflict 或事故物理。

### 2. IIDM 只负责 comfort acceleration

v0.3 使用 Improved Intelligent Driver Model（IIDM）作为正常驾驶的 comfort controller。它根据不可变 Vehicle Profile、当前状态和 leader observation 计算期望加速度。

IIDM 输出限制在 comfortable deceleration 与 max acceleration 之间。它不直接修改 world，不负责最终 no-overlap，也不拥有 signals、merge 或 parking 语义。

### 3. Core 拥有不可绕过的安全层

Core 在 IIDM 之后依次应用：

1. Gipps-style emergency safe-speed envelope；
2. 基于 leader final travel 的 deterministic no-overlap projection。

safe-speed 最多使用 profile 的 emergency deceleration。若 emergency deceleration 仍不足，Core 可以进一步限制本 tick travel 和 final speed，并产生明确的 safety projection event。

正常受控模式下，车辆几何不得重叠。`min_gap` 是强行为目标，不是物理碰撞边界；极端情况下可以侵入 `min_gap`，但不能产生负 bumper gap。事故必须由未来显式 out-of-control/incident 模式表达，不能由 Vehicle Following 数值失败隐式产生。

### 4. v0.3 不公开 controller trait

IIDM evaluator、leader observation、constraint set、safe-speed solver 和 projection graph 都是 Core 私有实现边界。v0.3 不公开 controller trait、callback、第三方 registry 或 Adapter 注入点。

若后续需要第二种内置模型，优先采用内部 enum、static dispatch 和按模型批处理。只有出现真实第三方或跨语言扩展需求后，才单独设计稳定数据/批处理 API、C ABI 或 WASM 边界，并新增 ADR。

未来自定义 comfort controller 也不得绕过 Core 的 safe-speed 与 no-overlap 层。

### 5. Vehicle Profile 进入 v0.3 数据契约

v0.3 新增不可变 Vehicle Profile 和 opaque `VehicleProfileHandle`。外部数据使用 profile external ID，runtime hot path 使用 handle。

数据格式新增独立 0.3 schema，不静默修改 0.2。profile 行为字段全部必填，不使用隐藏默认值。v0.3 不持久化 initial vehicles、spawn schedule 或 runtime handles。

### 6. 确定性保持同版本、同环境边界

Vehicle Following 沿用 ADR 0003：同一 Core 版本、同一运行环境、同一初始状态和输入序列必须得到相同状态与事件。v0.3 不承诺跨 CPU 或跨浮点实现的 bit-level determinism。

Occupancy、controller 和 projection 只读取 tick snapshot，结果一次性原子提交。稳定 update sequence 负责 tie-break 和事件归并，不依赖 external ID 热路径排序或无稳定顺序容器。

## 后果

正向后果：

- 正常驾驶获得 IIDM 的平滑行为，同时保留离散 tick 下的严格 no-overlap。
- signals、intersection 和 parking 可以通过通用 longitudinal constraint 接入，而不修改 IIDM 公式。
- Core 保留安全所有权，Adapter 或未来 controller 扩展不能绕过安全不变量。
- 私有数据导向实现允许后续批处理、并行和空间分区演进。
- Vehicle Profile 把 desired speed、length 和制动参数从 mutable state 中移出，减少语义歧义。

成本与风险：

- v0.2 `VehicleState.speed` 和 spawn API 需要破坏性迁移。
- data format 需要新增 0.3 schema 和显式迁移路径。
- ballistic integration、safe-speed 和 functional-graph projection 比单一 IDM 公式更复杂。
- `f64` 仍不提供跨平台 bit-level determinism。
- 百万级城市运行时仍需要后续 partition、parallel 和多层级模型，不能由本 ADR 自动获得。

## 替代方案

### 只使用 IDM/IIDM

实现简单且行为平滑，但离散 tick、极端初始条件和 leader 急停时不能单独保证 no-overlap，因此拒绝。

### 只使用 safe-speed 模型

安全边界清晰，但正常加减速容易过于保守，难以提供可信、可调的舒适驾驶行为，因此拒绝。

### 公开 `LongitudinalController` trait

短期看似便于替换模型，但会提前冻结 observation、状态、错误、线程、panic 和 SemVer 边界，而且 Rust trait 不能直接成为 Unity、Unreal、Godot 或 Web 的跨语言协议，因此 v0.3 拒绝。

### 引入 SUMO、CARLA 或 rustsim 作为 runtime 依赖

这些实现可用于公式和测试对照，但与 LaneFlow 的 handle、route、许可、部署和引擎嵌入边界不一致。根据 ADR 0002，不作为客户端 Core runtime 依赖。

### 用通用碰撞物理替代正常 Vehicle Following

物理碰撞适合显式事故和 out-of-control 状态，不适合正常交通的确定性纵向控制、拥堵和性能目标，因此拒绝。

## 实施与复核

- #73：Vehicle Profile schema、loader、registry/resolver。
- #74：VehicleState、spawn input 和 profile handle 迁移。
- #75：Occupancy index、leader detection 和 overlap validation。
- #76：IIDM、safe-speed、ballistic integration 和 no-overlap projection。
- #77：确定性、不变量、10k 性能和 100k 扩展性验证。
- #72：百万级城市运行时 partition/parallel/multi-rate/mesoscopic 研究，不阻塞 v0.3。

若未来公开 controller 扩展、引入跨平台 bit-level deterministic math，或改变 Core 安全所有权，应新增 ADR，不得静默改写本决策。
