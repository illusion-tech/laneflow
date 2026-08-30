# 路线图

**文档状态**: Accepted（长期路线；当前执行状态以 GitHub 为准）<br>
**最后更新**: 2026-08-23
**适用范围**: LaneFlow 版本路线图与中国特色城市模拟游戏交通基础的长期演进

本文记录 LaneFlow 的稳定路线图和已接受长期目标。GitHub Project 负责当前执行
状态，本文负责已接受版本边界与仍需各后继 Issue 独立完成的实施 Gate。

Accepted ADR 0021 把“为未来的中国特色城市模拟游戏提供交通基础”定义为 LaneFlow
的第一长期产品目标。#291 G1 已接受该北极星及其城市游戏/出行编排/路径规划/交通
运行时分层。当前局部走廊、园区和背景车流版本是验证该目标的渐进路径，不反向把
长期目标缩小为中小型场景；城市经济、出行需求、土地利用与游戏规则继续属于上层。

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

完成状态：2026-07-12 已完成。当时收口流水账见 git 历史；现行车道图与路线设计见 `design/lane-graph.md`、`design/route-system.md`。

范围：

- lane graph data model
- lane connection
- route definition
- route validation
- example route data

## v0.3 Vehicle Following

目标：支持可信的前车避让和速度控制。

完成状态：2026-07-14 已完成。当时收口流水账见 git 历史。现行设计见 [`design/vehicle-following.md`](design/vehicle-following.md) 与 [`adr/0006-vehicle-following-control-and-safety.md`](adr/0006-vehicle-following-control-and-safety.md)。

范围：

- v0.3 schema、production loader 与 Vehicle Profile（历史收口事实；active current 已由 v0.4 替换）
- 纵向 VehicleState、occupancy index 与 leader detection
- IIDM comfort control、emergency safe-speed 与 no-overlap projection
- 平滑跟驰、停止与恢复
- 确定性、不变量、一万性能与十万扩展性验证

## v0.4 Signals

目标：支持基础红绿灯和路口通行规则。

完成状态：2026-07-15 已完成。当时收口流水账见 git 历史。现行设计见 [`design/signal-system.md`](design/signal-system.md) 与 [`adr/0009-signal-indication-gate-and-policy-separation.md`](adr/0009-signal-indication-gate-and-policy-separation.md)。

范围：

- current 0.4 static StopLine、MovementGate、SignalGroup、fixed-time Controller/Phase 与 strict loader；
- absolute integer-time phase/aspect snapshot、只读 query 与稀疏事件；
- protected-entry green、restrictive yellow/red、SignalStop 与 hard projection；
- permission-aware route-occurrence traversal、排队、放行、确定性与失败原子性；
- canonical fixtures、schema/loader/Core scenarios、一万/十万性能与验证基线。

实施链：#93 design/ADR → #94 static/data → #95 runtime/query/events → #96 compliance/traversal → #97 validation/performance → #18 closure。

不覆盖：permissive movement、红灯右转/掉头、无保护左转、待行区专用语义、无信号优先级、conflict/reservation、actuated/adaptive controller 或 Adapter ABI；这些在 1.0 后按 versioned policy 与 maneuver/conflict domain 另行设计。

## v0.5 Parking

目标：支持基础停车位进出和占用状态。

完成状态：2026-07-17 已完成。当时收口流水账见 git 历史。现行设计见 [`design/parking-system.md`](design/parking-system.md) 与 [`adr/0010-parking-binding-and-vehicle-lifecycle-authority.md`](adr/0010-parking-binding-and-vehicle-lifecycle-authority.md)。

范围：

- 停车场泊位与专用路边泊位/停车带的 individual ParkingSpace，以及 optional ParkingArea grouping；
- entry/exit lane anchors、edge-relative parked geometry 与 immutable Core registry；
- `Vacant -> Reserved -> Occupied -> Vacant` 一对一 binding authority；
- caller-order reserve/cancel/commit/leave/rebind/parked-spawn lifecycle；
- live `VehicleStatus::Parked`、位置 authority transfer 与 route/despawn cleanup；
- ParkingStop 与 Vehicle Following、Signals、RouteEnd、projection/traversal 的原子组合；
- current 0.5 static data、schema/loader、canonical fixtures 与 current-only migration；
- determinism、失败原子性、一万/十万、allocation/memory 与端到端 validation。

实施链：#105 design/ADR → (#106 lifecycle/performance，#107 static/current data) → #108 runtime/commands → #109 ParkingStop/activation → #110 validation/performance → #19 closure。

不覆盖：自动选位/调度、共享正常行车道停车、自由空间/倒车轨迹、停车场运营、Parking Adapter ABI/动画/authoring、十万 realtime SLA 或跨平台 bit-level determinism。

## v0.6 数值与空间基础（Numeric & Spatial Foundation）

目标：在实现首个引擎适配器前，冻结 LaneFlow 的数值表示边界、引擎无关道路空间几何、长度与坐标权威，以及最小空间查询能力。

完成状态：2026-07-21 已完成。当时收口流水账见 git 历史。现行设计见 [`design/numeric-representation.md`](design/numeric-representation.md) 与 [`design/spatial-geometry.md`](design/spatial-geometry.md)。

已接受交通权威为整数毫米（ADR 0028 / #496；边长/进度/车长/停车锚点为 `u32` mm，速度为 `mm/s`）、共享静态路网与 LFCA 对象合同 `formatVersion = 4`（含 static Junction/Movement/ManeuverPath、ConflictZone/ParticipantStream、multi-Gate/WaitingZone、per-edge 基础道路限速与 #262 横断面/准入静态模型），以及每轴 `±16_384 m` 的 Spatial canonical `f32` 几何/位姿权威。仓库夹具与读器只承认 format 4。Core/Data target-f32 完整候选因稳态收益 `4.257%` 未达到 `5%` 门槛而回退，不再构成现行权威；Spatial `f32` 通过误差、零分配、内存和一万/十万性能 Gate。未来重启三维/编制数值迁移必须新建议题并重新进入 G1。

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
- #72 的交通参与单元按执行域分区、并行、多频率、中观仿真或分布式运行时；其当前
  证据只覆盖道路机动车执行域；
- 未经 #122 G1 证据验证的统一 f32、f64 或 f16 结论。

## v0.7 Bevy Reference Adapter

目标：以 Rust/Bevy 作为首个 Reference Adapter，完成可运行的引擎集成闭环，并用真实宿主验证 Adapter API；Bevy 不是跨 ABI、跨语言稳定性的唯一证明。

完成状态：Milestone tracker 为 #121。v0.6 前置与 Adapter API 已完成；#169-#173 已分别交付 Bevy 0.19.x 最小 production graph、fixed schedule、Entity/Transform 同步、headless/performance Gate、debug Gizmos 与 native reference example，#174 负责最终集成收口。长期设计见 `design/bevy-reference-adapter.md`。当时收口流水账见 git 历史。

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

完成状态：2026-07-24 已完成。当时收口流水账见 git 历史。现行场景设计见 `design/example-scenarios.md`。

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

完成状态：2026-07-26 已完成。当时收口流水账见 git 历史。现行设计见 `design/road-junction-model.md` 与 `design/signalized-corridor-protected-turning.md`。

范围：

- #228/ADR 0017 冻结长期 Road/Junction/Maneuver 分层、Route occurrence、一等
  ManeuverGate、authority、determinism 与 performance target；
- #196 已接受具体转向 profile，冻结 lane assignment、32 条 ManeuverPath、28 条
  Route、catalog 0.2、四组 12-phase program、兼容矩阵与车辆选择规则；SSOT 见
  [`signalized-corridor-protected-turning.md`](design/signalized-corridor-protected-turning.md)；
- #229 以 clean break 原子实现 Junction/Movement/ManeuverPath/ManeuverGate
  Core/Data static model、Traffic v0.8、fixtures、generator 和 generated artifacts；
- #190 交付具体 profile artifacts、catalog 0.2、scenario policy 与 native 最小
  集成；#191 扩大 cross-layer 验证，#192 执行独立 closure review；
- 保留 v0.8 的道路尺度、限速、50–200 车辆调节、信号时长配置和确定性出口回流能力；
- 完成端到端安全、确定性、可配置性、native 可视化和独立 closure review。

实施顺序为 `#228 -> #196 -> #229 -> #190 -> #191 -> #192`。在该 v0.9
signalized-corridor 产品里程碑收口时，RoadSection、LaneGroup 与 JunctionGroup
只冻结长期语义，不生产化；其后 #262 已按下节生产化前两者及相关横断面/准入模型。

不覆盖：无保护左转、红灯右转、感应式或自适应信号、掉头、lane change、
ConflictZone/right-of-way solver、RoadSection/JunctionGroup runtime，以及 #72 的
城市级扩展。

## 多模式横断面与准入（静态模型已生产化）

#234/ADR 0018 冻结多模式道路横断面与准入分层：`RoadCorridor` 横断面 owner、
方向性 `RoadSection`/可选 `LaneGroup` 生产语义、非遍历 `FacilityBand`、
`FacilityKind`/`ParticipantClass`/`AccessRule` 显式分离与时间/地区 overlay；
SSOT 见 [`cross-section-access.md`](design/cross-section-access.md)。该设计属于
#227 复杂道路设施演进路线，不自动进入任何已冻结产品 Milestone；当前 Milestone
归属以 GitHub 为准。

#234 拆出的最小 production Issue #262 已交付静态模型、Traffic v0.9 原子迁移
（v0.9 schema 已发布并经 live 验证）与 (class, Route) 绑定期
静态准入校验。后续顺序：#237 动态车道用途/resolved lane plan G1（消费本
SSOT）→ #236 非机动车/步行产品范围。时变准入 runtime、横向几何与多法规版本
共存各自独立 G1，不属于当前完成边界。

## WaitingZone、ConflictZone 与通行权（G1 已接受，尚未生产化）

#235/ADR 0019 已冻结 multiple ManeuverGate occurrence、WaitingZone
容量/队列、ConflictZone/ParticipantStream、versioned jurisdiction/right-of-way
policy、Core ConflictArbiter 与 grant/reservation；SSOT 见
[`waiting-zone-conflict-right-of-way.md`](design/waiting-zone-conflict-right-of-way.md)。
性能优先 v1 使用 current/upcoming approach frontier、directed lower-bound ETA 与
mandatory downstream-clearance guard；每个 static zone-stream passage 的 top-two
owner frontier 支持 subject self-exclusion，不随 dynamic Route 数复制 cell；stable
single-writer claim ledger 防止 Waiting/Conflict/downstream 同 tick 重复分配。不能
证明车尾可清空 coverage zone 时不放行。
该设计属于 #227 的跨版本复杂路口演进，不自动进入任何产品 Milestone；当前
Milestone 归属以 GitHub 为准。#281 已把 current
Traffic source 从已发布且 immutable 的 v0.9 原子升级到 v0.10，交付 multi-Gate、
WaitingZone static registry/Data/route occurrence 与绑定期 capability guards；
protected-only runtime 仍未改变，Waiting runtime 由 #282 接续。v0.10 schema
已按固定 `main` provenance 公开发布，并通过 live availability 与 byte-equality
验证。

生产化规划由 #280 回写，实施 DAG 已拆为：

```text
#281 multi-Gate/Waiting static
  ├─> #282 Waiting runtime ───────────┐
  └─> #283 Conflict static/Spatial ───┴─> #284 policy/arbiter runtime
                                              └─> #285 cross-layer 一万/十万/closure
```

#281–#285 都是 #227 的独立后续切片，不是 #235 的子任务；每个切片必须自行完成
G0-G4 与元数据审计。#281 已交付 static/Data 0.10 并合入主干。#282–#285 的实现开工
前置条件是 #292（compiler foundation + Synthetic DSL frontend）完成 G4；其验收
必须包含用于等价验证的 integration-only LIR→current projection，就绪后拓扑密集
验证场景才改由编译器生成承担。这是交付顺序调整而非取消，DAG 依赖关系与已完成 Gate
记录继续有效。当前 Project 列、Milestone 和原生依赖关系以 GitHub 为准，不在路线图
镜像。通行权 runtime 能力
在本阶段停留 static 层，该代价登记于
[`design/network-compiler.md` §15](design/network-compiler.md#15-风险登记) 风险表。

#264 对 #235 的设计输入已经满足；#235 G4 后移除该原生 blocker，但 #264 仍必须
等待 #237 冻结后再拆 JunctionGroup、环岛、停车连接与互通组合。

## 编译器时代静态路网（#291 候选长期路线）

本节双语术语遵循 [`reference/glossary.md`](reference/glossary.md)，中文定义为
权威事实，英文只作辅助理解。

#291 已把目标从“L1/L2 生成器 + JSON 管线”修订为编译器拥有的静态路网
（Compiler-owned Static Network）：
道路编辑状态、Synthetic DSL、imported 与其他受检 module 共同组成唯一
authoritative source module graph，再进入
`typed AST → HIR → MIR → validated canonical LIR`。同一次成功编译以 LIR 作为唯一
静态语义输入，原子生成 portable canonical artifact、target-specific
`StaticNetworkImage` 与语义差异（Semantic Diff）；源映射（Source Map）另外消费
同次冻结、但不能补充静态语义的已验证来源伴随数据。目标
`laneflow-runtime`/Spatial 消费同一不可变镜像（Immutable Image）中的交通 /
可选空间视图（Traffic / Optional Spatial View），静态数据与每个世界可变状态
物理分离。生产启动通过
image 外部认证 descriptor/manifest 与 bounded verifier 建立 view，
不再解析 JSON、按字符串 rebind、重建 registry、重复 Traffic/Spatial join 或重编译
static occurrences。

本路线图中的 #298 “语义差异”只指规范 LIR/路网影响差异；道路编辑控制点等
authoring-only 改动由 C 阶段后继 [#345](https://github.com/illusion-tech/laneflow/issues/345)
`RoadEditingSourceDiff` 负责。

ADR 0023 已完成 #296 FlatBuffers G1 纠偏：production 道路来源首先服务可视化编辑器，并支持游戏
初始化时的程序化生成；道路编辑先交付整体替换 A 阶段，再演进到候选调整与影响预览 C
阶段。产品负责人已选择按模块 size-prefixed FlatBuffers 作为 production source；旧
Geometry JSON 尚未发布且不获得兼容承诺，分段 Protobuf 也不进入 production。借用的
原子 compiler 入口、依赖/审计边界以及 schema/reader、第一方 Rust 有类型来源构造面、
几何降阶和原子接入由 #296 交付。B1 仍不发布，也不增加旧 JSON 别名、读取兼容或迁移器；
当前执行与 Gate 状态以 GitHub 为准。v1 曲线只含 line/cubic
Bézier，常见 taper 使用线性
起止宽度；FacilityBand 不作为 v1 AccessRule target。#296 只为 #298 的 LIR 影响差异
提供输入，C 阶段的 authoring-only 来源差异已登记为后继 Delivery Issue
[#345](https://github.com/illusion-tech/laneflow/issues/345)。完整 C++/C# SDK 与引擎事务封装
属于后继交付。

#296 的资源验收只保留面向不可信输入的最小护栏：来源字节、verifier、主要规模、checked
arithmetic、保守整体工作集和失败原子性。逐 `Vec` / `Box` / `Arc` 的精确资源账本、阶段
共存峰值、失败诊断 allocator 证据与正式 P100 协议已拆至
[#374](https://github.com/illusion-tech/laneflow/issues/374)。#374 阻断 #305 及最终总体生产
切换，但不阻断 portable artifact、authoring diff 或 #351–#354 的 compiler-private 结构
收敛；后两者先完成还可以减少精确账本对即将变化布局的重复证明。

ADR 0020、ADR 0021 与 [`design/network-compiler.md`](design/network-compiler.md)
是 #291 G1 已接受的长期设计；Accepted ADR 0025 / #300 G1 已进一步以共享静态路网取代
独立静态镜像文件/ABI。Identity v1 区分 StableId128
declaration/addressable-derived、
owner-local occurrence 与全部 table row 的 typed `u32` ordinal，冻结完整
kind/tag registry、严格 field order、known vectors、BLAKE3-128 持久 identity 和
XXH3 compiler-only 加速。基础 `LaneEdge` 使用独立稳定边键；RoadSection 覆盖与
Junction internal role 不参与边身份，合法未覆盖边同样进入身份索引。共享静态路网
只保留 ID/ordinal 冷索引；完整规范身份表（Canonical Identity Table）
`CanonicalIdentityTable` 进入 portable artifact，供审计、诊断和后继工具读取；
Accepted ADR 0024 的 #299 后发射检查不重新执行逐实体身份派生。`SharedNetworkRevision`
采用 Traffic、冷稳定身份索引（Shared Identity Index，`SharedIdentityIndex`）与分区
规划提示（Partition Planning Hints，`PartitionPlanningHints`）必选，Spatial component
可选；facility/profile/frame-only Spatial 不冒充 lane-pose capability。稳定身份索引服务
快照恢复（Snapshot Restore）、dynamic Route 重建与修订切换
（Revision Cutover），不进入 steady tick，也不能被 headless/production profile
删除。target 把 current
`laneflow-core/CoreWorld` clean-break 为 `laneflow-runtime/TrafficWorld`，并通过
中立 `laneflow-static-contract`/`laneflow-static-network` 保持无环依赖。

编译器从 LIR 派生 worker 数无关的静态执行约束事实并规范发射到 LFCA 4 关系；LFCA 4
不保存提示 payload，`laneflow-static-network` 按显式非语义 derivation version 确定性派生
可丢弃的分区规划提示。每个 `TrafficWorld` 再依据硬件、容量和动态负载建立自己的运行时
执行计划。最终
partition/worker assignment 不进入共享静态路网。精确路径的所有 partition 读取同一
committed state `T` 并原子提交 `T + Δ`，不得因边界增加一 tick 延迟；连接资源
组件各有唯一规范归约权威，互不相交组件可并行归约。

共享静态路网表示不可变路网修订，不表示城市永不变化。玩家拖动/预览只修改 working
道路编辑状态；确认建造后才全量构建候选，新修订通过失败关闭切换事务进入运行世界。
只有已进入 Runtime 活动修订的 committed network source 进入存档：可编辑世界保存道路
编辑状态，runtime-only 发布世界保存已认证 LFCA asset reference；working/candidate 不保存。
发布来源转为 editable 前先用重编译 exact LFCA 原子 rebase root/source/diff-base binding；
可编辑 session 在内存保留当前 exact base LFCA 供下一 LFSD 发射，但不写入存档。
语义差异（Semantic Diff）必须由外部可信的路网修订切换描述符（Network Revision
Cutover Descriptor，`NetworkRevisionCutoverDescriptor`）绑定，不能自行授予迁移权限；稳定身份索引只
复核身份映射，不能替代语义兼容证据。每世界 identity、
调用方拥有的 seed/随机流（Caller-owned Seed / Random Stream）、每世界路线、执行计划
与运行时快照不进入共享静态路网。路径规划读取静态路网和已提交动态成本快照；出行需求
与路线选择策略仍由城市游戏/出行编排层拥有。

现行生产路径由编译器原生有类型来源进入 Typed AST / HIR / MIR / Canonical LIR，发射
LFCA/LFSM/LFSD，经后发射检查后构建共享静态路网，并由 `TrafficWorld` 与可选 Spatial
消费。旧 Traffic/Spatial JSON、`InitialTrafficData`、Core 与迁移投影桥都已删除，不建立
兼容导入或第二套运行时权威。

编译器性能工作负载及其规模计数必须依据真实编制/中间表示证据独立冻结；历史研究替身只作
资源估算和实现选择输入，不能冒充产品通过，也不能从 #72 的运行时交通参与单元规模反推。
#72 继续拥有交通参与单元按执行域
分解的保真度（Fidelity）、分区（Partition）、调度、迁移与内存证据；其既有证据只
覆盖当前道路机动车车辆特化。#236/#237 仍是独立产品 / 研究输入，不自动并入首个
前端（Frontend）。

## 城市级扩展研究

#72 是独立研究入口，不属于 v0.6–v0.9 的完成边界。v0.6 的 geometry 与
#72 的交通参与单元空间分区是不同层次；v0.7 的 presentation LOD 与 #72 的
Traffic Runtime fidelity 也不得混同。多世界共享静态路网可以验证内存复用、回放
和参数探索，但不能代替单个大型城市世界的 barrier、边界交换、负载偏斜与迁移性能。

#72 何时进入版本范围仍留待对应 Milestone 规划时决策；但在未来 Stable Runtime API Milestone 的 G1 前，必须完成 #199 对 Core API、partition、multi-rate、batch access、commands 和 deterministic event merge 的可扩展性审计，并关闭或显式接受其待决项。该审计不阻塞 v0.8/v0.9，也不代表已选择生产架构；完整并行、多层级或分布式实施只有在证据和产品目标明确后才建立 Milestone。

后继城市级工作至少拆分为四个独立 G1：

1. 保持个体身份的单世界确定性并行执行；
2. 不可变路网修订、运行时快照、存档/回放与修订切换；
3. 路径规划服务、动态成本快照和出行编排接入；
4. 中国特色城市拓扑/需求/运行时工作负载（Chinese-style City Workload），覆盖
   多阶段信号、左转待转区、干支路与小区出口、方向性高峰、公交/出租/路侧摩擦，
   并为非机动车/步行保留正式后继输入。

这些工作负载与 `LF-SYNTH-v1` 和 LuST 基线并列，不能静默替换既有 ID，也不能用
尚未支持的参与者伪装机动车。城市人口、乘客、出行需求与交通参与单元必须分层；
个体、活动、意图更新、表现、聚合记录和聚合等价计数必须按交通执行域分别报告。
“城市规模”不等于每个居民始终作为完整微观参与单元进入每个 tick，当前车辆证据也
不能代表非机动车、行人或轨道交通。

## v1.0 Scope TBD

状态：待规划。产品目标、交付范围、完成定义，以及与 #72、foreign-host boundary proof 和稳定性承诺的关系均未冻结。不得因为 `v1.0 Scope TBD` Milestone 已存在，就默认把未决 Issue 绑定到该 Milestone；其范围必须通过后续治理决策与 G1 重新建立。
