# 路线图

**文档状态**: Draft  
**最后更新**: 2026-07-17
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

## v0.6 Numeric & Spatial Foundation

目标：在实现首个 Engine Adapter 前，冻结 LaneFlow 的数值表示边界、引擎无关道路空间几何、长度/坐标权威和最小空间查询能力。

规划状态：2026-07-17 已完成版本边界调整，Milestone tracker 为 #120；#122 与 #123 已通过 G0，但尚未通过 G1/G2，以下技术方向仍须由 ADR、design、误差/性能证据和最小原型冻结。

范围：

- #122：f32/f64/f16 在 runtime hot state、累计/reference、存储/传输与 Adapter 边界中的角色；
- #122：领域误差预算、epsilon、确定性、数据/API 兼容和代表性 benchmark；
- #123：canonical/local coordinate、engine-neutral centerline、弧长采样和 spatial binding；
- #123：Core edge length 与 geometry arc length 的 authority、容差与 traffic/spatial artifact 边界；
- Adapter 所需的最小只读空间查询或 batch-oriented pose extraction 输入；
- G1 后拆分 Core/Data/Spatial 实施、validation/performance 和独立 closure review。

不覆盖：

- Bevy plugin、entity/Transform 同步、Gizmos 或示例场景；
- engine-specific spline、mesh、material、terrain 或 authoring GUI；
- #72 的 active-agent partition、并行、多频率、mesoscopic 或分布式 runtime；
- 未经 #122 G1 证据验证的统一 f32、f64 或 f16 结论。

## v0.7 Bevy Reference Adapter

目标：以 Rust/Bevy 作为首个 Reference Adapter，完成可运行的引擎集成闭环，并用真实宿主验证 Adapter API；Bevy 不是跨 ABI、跨语言稳定性的唯一证明。

规划状态：Milestone tracker 为 #121，当前被 #120 阻塞；v0.6、Adapter API G1 与 Bevy 依赖审计完成前保持 Backlog。

范围：

- 固定并审计受支持的 Bevy 版本与最小 feature 集；
- Core fixed tick 与 Bevy schedule ownership；
- vehicle/entity lifecycle 与稳定映射；
- batch pose/transform synchronization；
- headless deterministic integration tests；
- optional debug visualization 与最小 native example；
- f32 presentation boundary、坐标转换和 presentation LOD。

不覆盖：

- 让 Bevy ECS 成为交通状态 authority，或把 Bevy/glam 类型暴露为 Core/Spatial 公共 API；
- 把 WASM、第二个 Engine Adapter 或 foreign-host boundary proof 设为完成条件；
- #72 的 Core simulation fidelity 分层。

## 城市级扩展研究（Milestone N/A）

#72 保持独立 Backlog 研究入口，不属于 v0.6/v0.7 的完成边界。v0.6 的 geometry 与 #72 的 active-agent spatial partition 是不同层次；v0.7 的 presentation LOD 与 #72 的 Core simulation fidelity 也不得混同。

不要求在 v1.0 前实现 100k/1M runtime，但必须在 `v1.0 Stable Runtime API` 的 G1 前，从 #72 拆出并完成 Core API 对 partition、multi-rate、batch access、commands 和 deterministic event merge 的可扩展性审计。完整并行、多层级或分布式实施只有在证据和产品目标明确后才建立 Milestone。

## v1.0 Stable Runtime API

目标：稳定 Core API、数据格式和 Adapter 协议。

范围：

- documented Core API
- versioned data format
- adapter compatibility rules
- partition、multi-rate 与 batch access 的 API 可扩展性审计（不等同于实现 100k/1M runtime）
- example scenario suite
- release process
