# 路线图

**文档状态**: Draft  
**最后更新**: 2026-07-18
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

规划状态：2026-07-20，里程碑跟踪议题（Issue）为 #120。#122 的数值切片已形成独立[收口审阅基线](reference/v0.6-numeric-closure-review.md)：#140 推翻旧补偿候选和无性能收益前提后，#141/ADR 0014 接受下一 Core 数值契约与迁移/回退规则；#125 拆分 current-f64 九个领域数值 owner，#126 审计 API/Data 边界，#127 完成 target-f32 离线标定；#144 的完整原子生产候选通过正确性与内存护栏，但稳态性能只提升 `4.257%`、未达到 `5%` 门槛，已完整回退 current-f64。ADR 0012 继续描述当前 `f64` 生产状态，ADR 0014 继续保存未来目标和门槛；未来若重启数值迁移，必须新建议题并重新进入 G1。#123 已通过 G1，接受 ADR 0013、独立空间层、配套空间制品、LaneFlow 自有类型和首版无生产几何依赖；#133/ADR 0015 后续把 Spatial runtime 精度修订为每轴 `±16_384 m` 的有界 canonical `f32` frame，并保留其他分层与权威决策。数值切片完成不等于 Spatial 或 v0.6 整体完成，Spatial 生产实施继续按自己的闸口推进。

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
