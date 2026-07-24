# 路线图

**文档状态**: Draft  
**最后更新**: 2026-07-24
**适用范围**: LaneFlow 初始版本路线图

本文记录 LaneFlow 的稳定路线图。GitHub Project 负责当前执行状态，本文负责长期版本边界。

## v0.1 Core Prototype

目标：建立最小 Core runtime。

范围：

- vehicle state
- fixed or explicit tick API
- basic lane graph traversal
- simple route following
- minimal tests

不覆盖：

- 完整路口规则
- 停车系统
- 多引擎 Adapter

## v0.2 Lane Graph + Route

目标：稳定车道图和路线系统。

完成状态：2026-07-12 已完成。设计、实现、数据契约、测试与剩余风险的收口依据见[收口审阅基线](reference/v0.2-closure-review.md)。

范围：

- lane graph data model
- lane connection
- route definition
- route validation
- example route data

## v0.3 Vehicle Following

目标：支持可信的前车避让和速度控制。

完成状态：2026-07-14 已完成。设计、实现、当前数据契约、确定性、不变量、性能与剩余风险的收口依据见[收口审阅基线](reference/v0.3-closure-review.md)。

设计输入：[`design/vehicle-following.md`](design/vehicle-following.md)、[`design/data-loading.md`](design/data-loading.md)、[`design/data-format.md`](design/data-format.md)、[`adr/0006-vehicle-following-control-and-safety.md`](adr/0006-vehicle-following-control-and-safety.md)、[`adr/0007-traffic-data-crate-and-loader-boundary.md`](adr/0007-traffic-data-crate-and-loader-boundary.md) 与 [`adr/0008-pre-1.0-data-format-version-policy.md`](adr/0008-pre-1.0-data-format-version-policy.md)。

范围：

- v0.3 schema、production loader 与 Vehicle Profile（历史收口事实；active current 已由 v0.4 替换）
- 纵向 VehicleState、occupancy index 与 leader detection
- IIDM comfort control、emergency safe-speed 与 no-overlap projection
- 平滑跟驰、停止与恢复
- 确定性、不变量、10k 性能与 100k 扩展性验证

## v0.4 Signals

目标：支持基础红绿灯和路口通行规则。

完成状态：2026-07-15 已完成。设计、current 0.4 数据契约、runtime、车辆合规、确定性、10k/100k 性能、安全与剩余风险的收口依据见 [`v0.4 收口审阅基线`](reference/v0.4-closure-review.md)；详细测量见 [`Signals 验证基线`](reference/v0.4-signals-validation.md)。

范围：

- current 0.4 static StopLine、MovementGate、SignalGroup、fixed-time Controller/Phase 与 strict loader；
- absolute integer-time phase/aspect snapshot、只读 query 与稀疏事件；
- protected-entry green、restrictive yellow/red、SignalStop 与 hard projection；
- permission-aware route-occurrence traversal、排队、放行、确定性与失败原子性；
- canonical fixtures、schema/loader/Core scenarios、10k/100k 性能与验证基线。

实施链：#93 design/ADR → #94 static/data → #95 runtime/query/events → #96 compliance/traversal → #97 validation/performance → #18 closure。

不覆盖：permissive movement、红灯右转/掉头、无保护左转、待行区专用语义、无信号优先级、conflict/reservation、actuated/adaptive controller 或 Adapter ABI；这些在 1.0 后按 versioned policy 与 maneuver/conflict domain 另行设计。

## v0.5 Parking

目标：支持基础停车位进出和占用状态。

完成状态：2026-07-17 已完成。#105-#110 的设计、substrate、current 0.5 data、runtime、activation 与全面验证均已交付；最终设计、实现、数据契约、性能、安全、治理和剩余边界见 [`v0.5 收口审阅基线`](reference/v0.5-closure-review.md)，详细测量见 [`Parking 验证基线`](reference/v0.5-parking-validation.md)。

范围：

- 停车场泊位与专用路边泊位/停车带的 individual ParkingSpace，以及 optional ParkingArea grouping；
- entry/exit lane anchors、edge-relative parked geometry 与 immutable Core registry；
- `Vacant -> Reserved -> Occupied -> Vacant` 一对一 binding authority；
- caller-order reserve/cancel/commit/leave/rebind/parked-spawn lifecycle；
- live `VehicleStatus::Parked`、位置 authority transfer 与 route/despawn cleanup；
- ParkingStop 与 Vehicle Following、Signals、RouteEnd、projection/traversal 的原子组合；
- current 0.5 static data、schema/loader、canonical fixtures 与 current-only migration；
- determinism、失败原子性、10k/100k、allocation/memory 与端到端 validation。

实施链：#105 design/ADR → (#106 lifecycle/performance，#107 static/current data) → #108 runtime/commands → #109 ParkingStop/activation → #110 validation/performance → #19 closure。

不覆盖：自动选位/调度、共享正常行车道停车、自由空间/倒车轨迹、停车场运营、Parking Adapter ABI/动画/authoring、100k realtime SLA 或跨平台 bit-level determinism。

## v0.6 数值与空间基础（Numeric & Spatial Foundation）

目标：在实现首个引擎适配器前，冻结 LaneFlow 的数值表示边界、引擎无关道路空间几何、长度与坐标权威，以及最小空间查询能力。

完成状态：2026-07-21 已完成。数值切片和 Spatial 切片均已完成独立 G4，整体设计、生产实现、数据制品、正确性、性能、安全与治理结论见 [v0.6 收口审阅基线](reference/v0.6-closure-review.md)。

当前生产边界为 Core/Data current-f64 交通权威、Traffic Data v0.7（含 per-edge 基础道路限速），以及每轴 `±16_384 m` 的 Spatial canonical `f32` 几何/位姿权威。Core/Data target-f32 完整候选因稳态收益 `4.257%` 未达到 `5%` 门槛而回退；Spatial `f32` 通过误差、零分配、内存和 10k/100k 性能 Gate。未来重启 Core/Data 数值迁移必须新建议题并重新进入 G1。

范围：

- #122：`f32`、`f64` 和 `f16` 在运行时高频状态、累计或参考计算、存储与传输，以及适配器边界中的角色；
- #122：领域误差预算、epsilon（误差阈值）、确定性、数据与接口兼容，以及代表性性能基准；
- #123：标准与局部坐标、引擎无关的中心线、弧长采样和空间绑定；
- #123：Core 边长与几何弧长的权威职责、容差，以及交通和空间制品边界；
- #141：10 km 产品上界、补偿残差感知 `f32` 目标权威、公共 API/Data 迁移、route 距离候选与生产收益闸口；
- 适配器所需的最小只读空间查询或面向批量的位姿提取输入；
- G1 后拆分 Core、Data 和 Spatial 实施、验证与性能，以及独立收口审阅。

不覆盖：

- Bevy 插件、实体与变换同步、Gizmos 调试图形或示例场景；
- 引擎专用的样条曲线、网格、材质、地形或创作图形界面；
- #72 的活动车辆分区、并行、多频率、中观仿真或分布式运行时；
- 未经 #122 G1 证据验证的统一 f32、f64 或 f16 结论。

## v0.7 Bevy Reference Adapter

目标：以 Rust/Bevy 作为首个 Reference Adapter，完成可运行的引擎集成闭环，并用真实宿主验证 Adapter API；Bevy 不是跨 ABI、跨语言稳定性的唯一证明。

完成状态：Milestone tracker 为 #121。v0.6 前置与 Adapter API 已完成；#169-#173 已分别交付 Bevy 0.19.x 最小 production graph、fixed schedule、Entity/Transform 同步、headless/performance Gate、debug Gizmos 与 native reference example，#174 负责最终集成收口。长期设计见 `design/bevy-reference-adapter.md`，最终生产事实、机器证据、安全与兼容边界见 `reference/v0.7-bevy-closure-review.md`。

范围：

- 固定并审计受支持的 Bevy 版本与最小 feature 集；
- Core fixed tick 与 Bevy schedule ownership；
- vehicle/entity lifecycle 与稳定映射；
- batch pose/transform synchronization；
- headless deterministic integration tests；
- optional debug visualization 与最小 native example；
- f32 presentation boundary、坐标转换和 presentation LOD authority 边界；LOD/pooling 算法本身不作为 v0.7 完成条件。

不覆盖：

- 让 Bevy ECS 成为交通状态 authority，或把 Bevy/glam 类型暴露为 Core/Spatial 公共 API；
- 把 WASM、第二个 Engine Adapter 或 foreign-host boundary proof 设为完成条件；
- #72 的 Core simulation fidelity 分层。

## v0.8 Signalized Corridor MVP

目标：交付首个可调、可持续运行的直行信号化走廊示例，把既有 Core、Signals、Spatial 与 Bevy Reference Adapter 串成一条可验证的产品路径。Milestone tracker 为 #193。

完成状态：2026-07-24 已完成。#184–#189 与 #203 已分别完成 Accepted 设计、道路限速、Core atomic replace、Bevy lifecycle transaction、场景制品、确定性人口/回流策略和 native example；最终生产事实、机器/可视证据、安全状态、兼容边界与剩余风险见 [`v0.8 收口审阅基线`](reference/v0.8-signalized-corridor-closure-review.md)。

范围：

- 一条双向六车道主干道与两条双向四车道次干道；两条次干道分别与主干道垂直，形成两个平面交叉口；
- 道路总长按三条物理道路轴线计，默认 `800 + 300 + 300 = 1.4 km` 且不超过 2 km；主干道限速 60 km/h，次干道限速 40 km/h；
- 车辆数量可在 50–200 之间配置；
- 6 个 portal-level 直行 movement 展开为 14 条 lane-level explicit routes；
- 两个交叉口采用可配置主/次绿灯、黄灯、全红和 offset 的 fixed-time 信号控制，红灯时长由完整 phase program 推导；
- 车辆驶出道路后，先在其他 5 个 portal 间均匀选择，再在目标 portal 的 lane route 间均匀选择；blocked retry 不重抽，使场景持续运行且固定 seed 可复现；
- 首版车辆仅直行，提供可运行的 Bevy native reference example、headless 集成验证与独立 closure review。

设计 SSOT 为 `design/example-scenarios.md` 与 ADR 0016；Traffic 目标版本为 v0.7，SpatialPackage/ScenarioManifest 保持 v0.1。实施链：#184 直行基线设计 → #185/#186/#188 分别交付道路限速、Core replace 与场景制品 → #187/#203 交付 Adapter lifecycle 与人口回流 policy → #189 native example 集成 → #195 closure。

不覆盖：左转、右转、受保护转向相位、permissive movement、复杂车道选择或城市级扩展。

## v0.9 Complete Signalized Corridor Example

目标：在 v0.8 直行走廊之上，先建立显式 Junction/Movement/ManeuverPath/
ManeuverGate 静态身份，再交付支持受保护左转、直行和右转的完整信号化走廊示例。
Milestone tracker 为 #194；v0.8 已完成前置收口。

范围：

- #228/ADR 0017 冻结长期 Road/Junction/Maneuver 分层、Route occurrence、一等
  ManeuverGate、authority、determinism 与 performance target；
- #196 在该通用模型上冻结转向 movement/path、route、lane connection、
  signal group/phase、兼容矩阵与车辆选择规则；
- #229 以 clean break 原子实现 Junction/Movement/ManeuverPath/ManeuverGate
  Core/Data static model、Traffic v0.8、fixtures、generator 和 generated artifacts；
- #190–#192 实现并验证两个交叉口的受保护左转、直行和右转，以及对应的 Core/Data/Adapter 行为；
- 保留 v0.8 的道路尺度、限速、50–200 车辆调节、信号时长配置和确定性出口回流能力；
- 完成端到端安全、确定性、可配置性、native 可视化和独立 closure review。

实施顺序为 `#228 -> #196 -> #229 -> #190 -> #191 -> #192`。RoadSection、
LaneGroup 与 JunctionGroup 在 v0.9 只冻结长期语义，不生产化。

不覆盖：无保护左转、红灯右转、感应式或自适应信号、掉头、lane change、
ConflictZone/right-of-way solver、RoadSection/JunctionGroup runtime，以及 #72 的
城市级扩展。

## 城市级扩展研究（Milestone N/A）

#72 保持独立 Backlog 研究入口，不属于 v0.6–v0.9 的完成边界。v0.6 的 geometry 与 #72 的 active-agent spatial partition 是不同层次；v0.7 的 presentation LOD 与 #72 的 Core simulation fidelity 也不得混同。

#72 何时进入版本范围仍留待对应 Milestone 规划时决策；但在未来 Stable Runtime API Milestone 的 G1 前，必须完成 #199 对 Core API、partition、multi-rate、batch access、commands 和 deterministic event merge 的可扩展性审计，并关闭或显式接受其待决项。该审计不阻塞 v0.8/v0.9，也不代表已选择生产架构；完整并行、多层级或分布式实施只有在证据和产品目标明确后才建立 Milestone。

## v1.0 Scope TBD

状态：待规划。产品目标、交付范围、完成定义，以及与 #72、foreign-host boundary proof 和稳定性承诺的关系均未冻结。不得因为 `v1.0 Scope TBD` Milestone 已存在，就默认把未决 Issue 绑定到该 Milestone；其范围必须通过后续治理决策与 G1 重新建立。
