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
- `core-research-instrumentation-externalization.md`：#380 Accepted 的 Core 研究/测试仪器外移方案（已实施完成）；
  把 Core 生产路径内嵌的研究/测试仪器分为语义研究原型、性能归因计时、故障注入与
  保留内存账本四类分治；按访问需求冻结逐项落点（P1/P2/P3 保留 crate 内常规
  `#[cfg(test)]` 模块、P4 降级 provenance）、仪器探针边界、测试支持接缝与
  benches 解耦落点。
- `core-world-module-split.md`：#381 Accepted 的 CoreWorld 命令域拆分方案（已实施完成）；冻结
  `world.rs` → `world/mod.rs` 命令域子模块划分（state/support/parking_commands/
  signal_queries/route_queries/route_lifecycle/vehicle_lifecycle/tick 系列/tests）、
  `step()` 双分支收敛（const generic 单一 advance 循环）、测试纯搬迁与
  `CoreError` 拆分结论（拆 #389）；不改变公开 API 与数据格式。
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
  完整 Identity registry、可移植规范制品（Portable Canonical Artifact）、源映射
  （Source Map）与语义差异（Semantic Diff）。Accepted ADR 0025 / #300 G1 已冻结从受检
  LFCA 构建进程内共享静态路网、取消独立静态镜像文件/ABI 的修订。
  Accepted ADR 0024 已把 #299 收缩为共享后发射检查和最小发布闭合，不再交付独立
  validator/receipt。#300/#302 必须分别冻结共享静态路网构建
  与切换信任边界。目标态把当前
  `laneflow-core/CoreWorld` 一次性
  不兼容切换为 `laneflow-runtime/TrafficWorld`；Traffic、`SharedIdentityIndex` 与
  `PartitionPlanningHints` component 必选，Spatial component 可选，
  无图形配置不携带 geometry；facility/profile/frame-only Spatial 不冒充 lane-pose
  capability。编译器拥有 worker 数无关的静态执行约束事实，
  `laneflow-static-network` 从 LFCA 关系 versioned 派生非语义规划提示，最终分区属于每世界
  运行时执行计划；不可变路网修订通过失败关闭切换事务
  进入运行世界，并保留 runtime snapshot/replay 与 routing 接入边界。该架构服务
  Accepted ADR 0021 的中国特色城市模拟游戏交通基础长期目标；目标设计被接受不表示
  已经实现，也不把城市经济或出行需求放入 Traffic Runtime。双语术语以
  `../reference/glossary.md` 的中文定义为权威。#291 G1 前置条件已经满足；#292 已完成
  compiler foundation、Synthetic DSL frontend、集成专用 LIR→current projection 及 G4，
  #282–#285 关于 #292 的稳定开工前置已经满足。该完成事实不表示整个目标路网编译器、
  共享静态路网或 Traffic Runtime 已经实现；当前 Project 状态与原生依赖关系以 GitHub 为准。
- `shared-static-network.md`：#300 G1 Accepted 的实现级设计；从
  `laneflow-format` 受检 LFCA capability 有界构建 `SharedNetworkRevision`，冻结
  Traffic/Identity/Hints/可选 Spatial component、性能优先 SoA/CSR 默认布局、共享所有权、
  构建闭合、玩家确认建造、`CommittedNetworkSource` 与 session-only exact LFCA
  `EditableDiffBase`、只保存已提交道路状态和一次性交付测量；不定义静态镜像文件/ABI、
  mmap、cache 或第二套证明平台。#440 G1 Pass 附录冻结剩余 Runtime 关系/字段三类清单；
  G2 在 `laneflow-static-network` 闭合这些关系，不改变 #439 已交付基线。
- `portable-canonical-artifact.md`：#298 G1 已重新接受并完成 G4 的实现级格式与历史
  验证事实源；把上述长期架构收窄为
  LFCA/LFSM/LFSD/LFCP 四类对象的封闭节目录、规范记录编码、路网修订派生、
  artifact/source-map/历史 receipt 与 base/target exact digest + length 绑定、受限读取、
  `CompilationOutput` 单一输入和不可变发布提交点。附录 A 的完整 table/field registry、§9
  硬上限与 §10 known vectors 已闭合。Accepted ADR 0024 已取代其独立
  validator、receipt 和 LFCP v1 当前语义；历史证据不回写为新设计。
- `compiler-post-emission-check-and-minimal-publication-closure.md`：#299 Accepted 且 G2 已实现的
  compiler 后发射检查与最小发布闭合；扩展 `laneflow-format` 复核最终
  LFCA/LFSM/LFSD 字节与跨对象 binding，以借用型 capability 守卫发布副作用，
  LFCP v2 一次性移除 receipt 且不兼容读取 v1；不复验完整路网语义，不建设证明平台。
- `road-editing-source-and-geometry-frontend.md`：#296 已实现的内部 FlatBuffers B1
  production compiler 入口；冻结可视化编辑器为主、程序化生成器为辅、道路编辑按 A → C
  演进、有类型道路编辑模型、来源位置/协作，以及按模块 size-prefixed FlatBuffers source。
  B1 schema 仍未发布，也不承担长期存档兼容承诺。
- `compiler-budget-calibration.md`：#308 已完成 G4 的一次性非生产编译器校准研究设计；冻结
  标识、走廊关系、密集路口和研究夹具对照工作负载，以及宽星形、深链、共享扇入
  三种模块图；用机器可读清单与证据 JSON Schema、至少五级规模阶梯、成本/内存拐点、校准/压力
  规模、失败关闭停止护栏、冷实例/稳定容量复用、失败清理和平衡候选顺序形成可复现
  证据。它只为 #292 G1 提供 P100 同机 R0 研究输入，不实现生产编译器、不复用
  `LF-COMP-CURRENT-EQUIV-v1`、不表示真实城市容量，也不形成产品 SLA；P100 的
  推荐硬件角色由产品负责人独立选定，不是本研究从结果外推的结论。
- `compiler-foundation.md`：#292 已完成 G4；保存其 G1 已接受的实现级设计，并把已接受
  ADR 0020 的封闭
  契约收窄为 `laneflow-static-contract`、`laneflow-compiler`、集成专用
  `laneflow-compiler-test-support`、有类型抽象语法树、HIR、MIR、已验证规范低层
  中间表示、已验证源映射输入、合成领域专用语言前端、标识 v1 首次实现、确定性
  编译、诊断、迁移投影与编译器工作负载，并以 #308 exact Evidence 冻结 P100 首轮
  资源配置档和原 G1 性能门槛。G2 已交付生产实现与迁移等价证据，同时发现研究
  工作负载不能按原自然身份无损映射为合法生产语义；后继 append-only G1 修订已把
  #308 降为容量估算输入，并用真实合法产品场景形成 P100 首轮生产编译基线。不得把
  #308 研究替身记录冒充产品通过（Product Pass）。#315 已实现官方前端模块的共同
  受检接入与来源记录生命周期；#297 不再建立 current JSON 编译器前端。
- `current-package-import.md`：#297 调整后的 Accepted 设计；确认 current Traffic v0.10、
  SpatialPackage v0.1 与 ScenarioManifest v0.1 仅为未发布的内部加载格式，取消 compiler
  导入特性、迁移包、严格资源/位置能力和资产报告，改用编译器原生有类型夹具验证
  Canonical LIR→当前 Core/Spatial 投影，并由 #294 删除旧加载路径。

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
