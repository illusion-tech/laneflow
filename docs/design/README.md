# 设计文档

本目录用于保存 LaneFlow 的具体设计文档，重点回答“系统具体怎么做”。

`docs/adr/` 记录高影响决策原因；`docs/design/` 记录 Core、数据格式、Adapter 和运行时系统的可执行设计。

## 推荐设计文档

初始阶段建议逐步补齐：

- `core-runtime.md`：Core runtime、tick、vehicle state 和系统边界。
- `core-runtime-performance-baseline.md`：#215 Accepted 的 10k/100k 产品目标与 1M 研究包络；冻结 `LF-SYNTH-v1` 确定性拓扑、规模计数、W1–W4 workload、R0/P10/P100/O1 硬件角色、tick/frame budget、fidelity、benchmark protocol、TBD 与架构升级触发；真实路网来源与转换由 #224 独立跟踪。目标契约不等于 P10/P100 产品认证。
- `core-runtime-scalability-audit.md`：#72/#199 的城市级可扩展性前置审计；记录 CoreWorld、handle、batch、commands、deterministic phase/event merge 的 no-regret constraints、individual-first/exact-only/aggregate-first 候选矩阵与 Stable Runtime API G1 待决项，不实现生产 partition。
- `core-id-handles.md`：Core external ID、typed handle、registry / resolver、动态 lifecycle 和事件 payload 边界。
- `numeric-representation.md`：v0.6 数值表示、精度分层、误差预算、确定性与 Core/Data/Spatial/Adapter 转换边界。
- `spatial-geometry.md`：v0.6 引擎无关的坐标框架、折线中心线、长度绑定、采样、制品配对与批量位姿提取。
- `lane-graph.md`：车道图、连接关系、拓扑约束。
- `road-junction-model.md`：#228 Accepted 的长期 Road/Junction/Maneuver 分层与 v0.9 最小静态 profile；冻结 Junction/Movement/ManeuverPath owner、一等 ManeuverGate、Route occurrence、Traffic v0.8 target、确定性与性能边界。
- `signalized-corridor-protected-turning.md`：#196 Accepted 的 v0.9 双路口受保护转向 profile；冻结 lane assignment、32 条 ManeuverPath、28 条 Route、catalog 0.2、四组 12-phase signal program、安全矩阵与验收边界；当前 production 尚未切换。
- `route-system.md`：路线选择、路径跟随、目标点。
- `vehicle-following.md`：前车避让、速度控制和安全距离。
- `signal-system.md`：Accepted v0.4 Signals；#94-#97 已落地 static/current data、fixed-time runtime/query/events、车辆合规与端到端性能验证，收口证据见 `../reference/v0.4-closure-review.md`。
- `parking-system.md`：Accepted v0.5 Parking；#107 已落地 ParkingSpace/ParkingArea static registry 与 current 0.5 data，#108/#109 已交付占用 authority、预约/停车/离开及 route/Following/Signals 集成，#110 已完成端到端与性能验证，#19 已完成独立收口审阅。
- `adapter-api.md`：Core/Spatial 与引擎适配器之间的只读快照、批量位姿、宿主转换和权威职责边界。
- `bevy-reference-adapter.md`：v0.7 Bevy 0.19 Reference Adapter 的依赖、schedule、Entity/Transform、debug、example 与验证边界。
- `data-format.md`：lane graph、route 等外部数据格式、validation 和 loader 边界；Rust crate 所有权见 ADR 0007。
- `data-loading.md`：当前 v0.7 Rust loader、严格版本闸口、per-edge speed limit、Core Signals/Parking normalization、错误与测试边界。
- `example-scenarios.md`：Accepted v0.8 直行信号化走廊；冻结 1.4 km 默认几何、14 条 lane routes、限速、固定时制、50–200 车辆人口、seeded 出口回流与分层验收路径。
- `signalized-corridor-population.md`：Accepted v0.8 caller-owned reference policy；冻结 `laneflow-scenario` crate 边界、catalog normalization、两阶段 bootstrap、ordered completion、blocked retry、replay 与零分配基线。

## 文档状态

设计文档状态建议使用：

- `Draft`：草稿中，不能直接作为稳定实现输入。
- `Review`：已形成可审阅版本，但仍可能调整。
- `Accepted`：可作为当前阶段实现输入。
- `Active`：持续维护的治理性或索引性文档。
- `Archived`：历史保留，不再作为默认输入。

## 页头约定

正式设计文档建议包含：

```md
# Document Title

**文档状态**: Draft
**最后更新**: YYYY-MM-DD
**适用范围**:
**关联文档**:
```

## 使用规则

- 涉及 Core API、data spec 或 Adapter 协议的实现，应先有相关 design 文档或 ADR。
- PR 中发现设计与实现不一致时，应先回写设计或拆分后续 Issue。
- 设计文档不记录单次 PR 的测试结果。
