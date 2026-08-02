# 设计文档

本目录用于保存 LaneFlow 的具体设计文档，重点回答“系统具体怎么做”。

`docs/adr/` 记录高影响决策原因；`docs/design/` 记录 Core、数据格式、Adapter 和运行时系统的可执行设计。

## 推荐设计文档

初始阶段建议逐步补齐：

- `core-runtime.md`：Core runtime、tick、vehicle state 和系统边界。
- `core-runtime-performance-baseline.md`：#215 Accepted 的当前道路机动车一万/十万产品目标与一百万研究包络；冻结 current 五项车辆计数、`LF-SYNTH-v1` 确定性拓扑、W1–W4 workload、R0/P10/P100/O1 硬件角色、tick/frame budget、fidelity、benchmark protocol、TBD 与架构升级触发；2026-08-02 已把当前物理配置选定为 P100 目标产品推荐参考机型，但同机 R0 研究证据不自动成为 Product Pass；#291 已接受交通参与单元、交通执行域与六类分域目标计数，但在目标态实现与性能基准完成显式迁移前，它们不改写 current Product Pass、工作负载或历史证据。
- `real-road-workloads.md`：#224 Accepted 的 LuST v2.0 真实路网契约；冻结 source/provenance、共享静态转换、精确一万 selection、`LF-REAL-LUST-TOPO-v1` 与 `LF-REAL-LUST-DEMAND-v1`、oracle/digest、Release 制品与 fail-closed 边界。它只补充 `LF-SYNTH-v1`，不等于真实路网 Product Pass。
- `lust-bevy-population-control.md`：#256 Accepted 的 LuST/Bevy 示例层一至一万个体人口调节契约；冻结 H1/H2/手动 H3、连续 `target_N`、seeded 无放回抽样、缩编分层与 100% presentation 默认；由 #257 实现，不改写 TOPO/DEMAND workload ID。
- `core-runtime-scalability-audit.md`：#72/#199 的城市级可扩展性前置审计；区分当前
  道路机动车特化证据与目标 Traffic Runtime 多执行域抽象，记录 CoreWorld、句柄、
  批处理、命令、确定性阶段/事件合并的无悔约束（No-regret Constraints）、个体优先/
  仅精确/聚合优先（Individual-first/Exact-only/Aggregate-first）候选矩阵与 Stable
  Runtime API G1 待决项，不实现生产分区。
- `core-id-handles.md`：Core external ID、typed handle、registry / resolver、动态 lifecycle 和事件 payload 边界。
- `numeric-representation.md`：v0.6 数值表示、精度分层、误差预算、确定性与 Core/Data/Spatial/Adapter 转换边界。
- `spatial-geometry.md`：v0.6 引擎无关的坐标框架、折线中心线、长度绑定、采样、制品配对与批量位姿提取。
- `lane-graph.md`：车道图、连接关系、拓扑约束。
- `road-junction-model.md`：#228 Accepted 的长期 Road/Junction/Maneuver 分层与 v0.9 最小静态产品 profile；冻结 Junction/Movement/ManeuverPath owner、一等 ManeuverGate、Route occurrence、历史 Traffic v0.8 clean-break target（current v0.10 继承）、确定性与性能边界。
- `waiting-zone-conflict-right-of-way.md`：#235 Accepted 的多阶段复杂路口 G1；基于 current Traffic v0.10 AccessRegistry 冻结 multiple ManeuverGate occurrence、WaitingZone 容量/队列、ConflictZone/ParticipantStream、versioned jurisdiction/right-of-way policy、Core ConflictArbiter、top-two directed-bound approach frontier、mandatory downstream-clearance、single-writer resource claims、grant/reservation、post-v0.9 原子格式迁移、事件与一万/十万边界；#281 已交付 multi-Gate/WaitingZone static 与 Data 0.10，#282–#285 继续 runtime/Conflict/policy/closure。
- `cross-section-access.md`：#234 Accepted、#262 已生产化的多模式横断面与准入分层；RoadCorridor/RoadSection/LaneGroup/FacilityBand、ParticipantClass/AccessRule 的 Traffic v0.9 来源契约由 current v0.10 继承，静态 `(class, Route)` 准入已落地，时变规则与 FacilityBand target 仍由 capability guard 拒绝。
- `signalized-corridor-protected-turning.md`：#196 Accepted 的 v0.9 双路口受保护转向 profile；冻结 lane assignment、32 条 ManeuverPath、28 条 Route、catalog 0.2、四组 12-phase signal program、安全矩阵与验收边界；#190 当前交付 profile artifacts、scenario policy 与 native 最小集成。
- `route-system.md`：路线选择、路径跟随、目标点。
- `vehicle-following.md`：前车避让、速度控制和安全距离。
- `signal-system.md`：Accepted v0.4 Signals；#94-#97 已落地 static/current data、fixed-time runtime/query/events、车辆合规与端到端性能验证，收口证据见 `../reference/v0.4-closure-review.md`。
- `parking-system.md`：Accepted v0.5 Parking；#107 已落地 ParkingSpace/ParkingArea static registry 与 current 0.5 data，#108/#109 已交付占用 authority、预约/停车/离开及 route/Following/Signals 集成，#110 已完成端到端与性能验证，#19 已完成独立收口审阅。
- `adapter-api.md`：Core/Spatial 与引擎适配器之间的只读快照、批量位姿、宿主转换和权威职责边界。
- `bevy-reference-adapter.md`：v0.7 Bevy 0.19 Reference Adapter 的依赖、schedule、Entity/Transform、debug、example 与验证边界。
- `data-format.md`：lane graph、route 等外部数据格式、validation 和 loader 边界；Rust crate 所有权见 ADR 0007。
- `data-loading.md`：当前 v0.10 Rust loader、严格版本闸口、Junction/Movement/ManeuverPath、multi-Gate/WaitingZone、per-edge speed limit、横断面/准入（RoadCorridor/RoadSection/LaneGroup/FacilityBand/ParticipantClass/AccessRule）、Core Signals/Parking normalization、错误与测试边界。
- `example-scenarios.md`：v0.8 直行走廊基线与 current v0.10 protected-turning 增量；记录 1.4 km 几何、28 Route、限速、固定时制、50–200 车辆人口、native 入口与分层验收路径。
- `signalized-corridor-population.md`：current v0.10 caller-owned reference policy；冻结 `laneflow-scenario` crate 边界、catalog 0.2 PortalLane/weighted RouteChoice normalization、三 draw-site completion、blocked retry、replay 与零分配基线。
- `network-compiler.md`：#291 G1 综合架构修订；采用权威来源模块图
  （Authoritative Source Module Graph）、有类型抽象语法树（Typed AST）→高层中间
  表示（HIR）→中层中间表示（MIR）→已验证规范低层中间表示（Canonical LIR）、
  完整 Identity registry、可移植规范制品（Portable Canonical Artifact）、外部信任
  收据绑定的目标静态镜像（Target Static Image）、源映射（Source Map）、语义差异
  （Semantic Diff）与独立验证器。目标态把当前 `laneflow-core/CoreWorld` 一次性
  不兼容切换为 `laneflow-runtime/TrafficWorld`；Traffic、`StaticIdentityIndex` 与
  `PartitionPlanningHints` section 必选，Spatial section 由 closed profile 控制，
  无图形配置不携带 geometry；编译器拥有 worker 数无关的静态执行
  约束，最终分区属于每世界运行时执行计划；不可变路网修订通过失败关闭镜像切换事务
  进入运行世界，并保留 runtime snapshot/replay 与 routing 接入边界。该架构服务
  Accepted ADR 0021 的中国特色城市模拟游戏交通基础长期目标；目标设计被接受不表示
  已经实现，也不把城市经济或出行需求放入 Traffic Runtime。双语术语以
  `../reference/glossary.md` 的中文定义为权威。#291 G1 前置条件已经满足；#292 仍须
  按自身 Gate 推进，当前 Project 状态与原生依赖关系以 GitHub 为准。
- `compiler-budget-calibration.md`：#308 已完成 G4 的一次性非生产编译器校准研究设计；冻结
  标识、走廊关系、密集路口和研究夹具对照工作负载，以及宽星形、深链、共享扇入
  三种模块图；用机器可读清单与证据 JSON Schema、至少五级规模阶梯、成本/内存拐点、校准/压力
  规模、失败关闭停止护栏、冷实例/稳定容量复用、失败清理和平衡候选顺序形成可复现
  证据。它只为 #292 G1 提供 P100 同机 R0 研究输入，不实现生产编译器、不复用
  `LF-COMP-CURRENT-EQUIV-v1`、不表示真实城市容量，也不形成产品 SLA；P100 的
  推荐硬件角色由产品负责人独立选定，不是本研究从结果外推的结论。
- `compiler-foundation.md`：#292 G1 的实现级设计草案；把已接受 ADR 0020 的封闭
  契约收窄为 `laneflow-static-contract`、`laneflow-compiler`、集成专用
  `laneflow-compiler-test-support`、有类型抽象语法树、HIR、MIR、已验证规范低层
  中间表示、已验证源映射输入、合成领域专用语言前端、标识 v1 首次实现、确定性
  编译、诊断、迁移投影与编译器工作负载，并以 #308 exact Evidence 冻结 P100 首轮
  资源配置档和性能门槛。该文档仍是草案（Draft），不构成 G2 开工授权。

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
