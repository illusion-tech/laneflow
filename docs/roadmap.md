# 路线图

**文档状态**: Draft  
**最后更新**: 2026-07-15  
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

实现状态：2026-07-15 已完成 G1 冻结；#94 已落地 static Signals、current v0.4 schema/loader/Core normalization、canonical fixtures 与 capability guard；#95 已落地 absolute-time fixed-time runtime、time-0/post-step query 与 phase/aspect events；#96 已落地 restrictive yellow/red SignalStop、hard projection、permission-aware route-occurrence traversal，并以完整车辆合规替代 guard。#97 继续承担端到端验证与性能收口。Accepted 输入见 [`design/signal-system.md`](design/signal-system.md) 与 [`adr/0009-signal-indication-gate-and-policy-separation.md`](adr/0009-signal-indication-gate-and-policy-separation.md)。

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

范围：

- parking spot data
- parking occupancy
- approach and leave behavior
- simple parking route integration

## v0.6 First Adapter

目标：完成第一个可运行 Engine Adapter。

候选：

- Web Adapter
- Unity Adapter

范围：

- Core tick integration
- vehicle transform sync
- debug visualization
- minimal example scene

## v1.0 Stable Runtime API

目标：稳定 Core API、数据格式和 Adapter 协议。

范围：

- documented Core API
- versioned data format
- adapter compatibility rules
- example scenario suite
- release process
