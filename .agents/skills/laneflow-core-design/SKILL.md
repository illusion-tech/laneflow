---
name: laneflow-core-design
description: 处理 LaneFlow 当前态核心运行时（Current Core Runtime）与目标态交通运行时（Target Traffic Runtime）设计。适用于当前道路机动车的车辆状态（Vehicle State）、车道图（Lane Graph）、路线（Route）、跟车（Vehicle Following）、信号（Signal）、路口规则（Intersection Rules）与停车（Parking），以及目标交通参与单元（Traffic Participant Unit）、交通执行域（Traffic Execution Domain）、确定性固定步进（Tick）/并行、路网修订、快照/回放、路径规划接入和 Core/Runtime API 变更。
---

# LaneFlow 交通运行时设计（Traffic Runtime Design）

Skill 标识符（Skill ID）`laneflow-core-design` 可在 `laneflow-core` crate 删除后
短暂保留，作为 Traffic Runtime 的发现入口；它不表示目标态继续命名为 Core。crate
拆除由 #301 完成。

## 先读这些

1. `docs/architecture.md`
2. `docs/adr/0001-project-scope.md`
3. `docs/design/README.md`
4. `docs/governance/development-gates.md`
5. `docs/reference/glossary.md`
6. 已存在的相关 Core / Traffic Runtime design 文档
7. 涉及 #291 静态路网编译、#300 共享静态路网或 #301 消费路径与 Core 入口拆除时，读取
   `docs/adr/0020-compiler-owned-static-network-and-static-image.md`、
   `docs/adr/0025-checked-canonical-network-and-shared-static-network.md`、
   `docs/design/network-compiler.md`、`docs/design/shared-static-network.md` 与
   `docs/design/traffic-runtime-shared-consumption.md`
8. 涉及 #308 编译器工作负载、资源/性能预算校准、研究停止护栏或私有容器候选时，
   读取 `docs/design/compiler-budget-calibration.md`。#308 已关闭；研究执行器、
   R0 raw/Evidence JSON 与研究报告不在当前工作区。查阅当时制品，使用 G4 精确
   证据提交 `de4cd460a96415cafbd811141568b81f74d73534` 与交付 PR #310
9. 涉及 #292/#315/#296/#297 编译器基础设施、官方前端共同受检模块接入、current JSON
   退役、已验证规范 LIR 或集成专用 LIR→当前态投影时，额外读取
   `docs/design/compiler-foundation.md`；
   #296 的道路编辑状态、候选道路替换或来源权威还须读取
   `docs/adr/0023-road-editing-state-and-phased-network-replacement.md` 与
   `docs/design/road-editing-source-and-geometry-frontend.md`；
   准备 #292 G3 或复核生产性能证据时，同时读取
   `docs/reference/v0.10-compiler-foundation-validation.md`
10. 涉及城市模拟游戏范围、出行编排、Routing、路网修订、存档/回放、并行或
   fidelity 时，读取 `docs/adr/0021-city-simulation-game-traffic-foundation.md`

若所需 design 文档尚不存在，在对当前 Core 或目标 Traffic Runtime 做高影响变更
前，应先创建或提出最小设计基线。

## 当前态与目标态命名边界

- 当前态：中文规范名“LaneFlow 核心（LaneFlow Core）”，精确标识符
  `laneflow-core` / `CoreWorld`。
- #291 目标态：中文规范名“LaneFlow 交通运行时（LaneFlow Traffic Runtime）”，
  精确标识符 `laneflow-runtime` / `TrafficWorld`。
- #301 完成前，代码与现有 API 继续使用当前态名称；#301 完成后可运行世界使用
  `laneflow-runtime` / `TrafficWorld`。目标设计不得把 `Core` 当成终态名称。
- 中文术语和中文定义以 `docs/reference/glossary.md` 为权威，英文只作辅助理解。

## 动态执行层边界

当前 Core 只实现道路机动车车辆特化，负责：

- `VehicleState`、车道图遍历、Route 跟随与前车避让
- 信号遵守、路口规则和停车行为
- 引擎无关的固定步进（Fixed Tick）行为

目标 Traffic Runtime 使用交通参与单元（Traffic Participant Unit）作为长期通用
抽象，并按交通执行域（Traffic Execution Domain）分离网络、运动/安全求解和生命
周期。它负责已实现执行域的参与单元、动态通行定义、每世界可变状态、运行时执行
计划和路网修订绑定。当前车辆能力不能被写成终态唯一参与者模型，通用术语也不能
反向声称非机动车、行人或轨道执行域已经实现。

当前 Core / 目标 Traffic Runtime 不得依赖：

- Unity API
- Unreal API
- Godot API
- O3DE API
- DOM 或 WebGL 展示 API
- 引擎特有的 actor、entity、prefab 或 scene object 模型

## 设计检查

对当前 Core / 目标 Traffic Runtime 变更，必须显式确认：

- 是否改变当前 Core API 或目标 Traffic Runtime API？
- 是否改变数据格式假设？
- 是否影响 Adapter API？
- 是否需要确定性行为测试？
- 是否需要 ADR？
- 是否错误地让静态 / 共享契约留在动态运行时包（crate），或让编译器 / 验证器
  （Compiler / Validator）依赖当前核心对象图（Current Core Object Graph）？
- 是否把城市经济、出行需求、路线选择策略或游戏规则错误放进 Traffic Runtime？
- 是否把当前 `VehicleState`、Route、道路 occupancy 或 Parking 特化错误提升为
  所有交通参与单元的终态公共基类，或把 `ParticipantClass` 当成执行域/行为能力？
- 是否把最终分区/工作线程（Partition/Worker）写入共享静态路网，或让分区切分
  （Partition Cut）增加一 tick 延迟、改变已提交状态/事件？
- 是否区分不可变路网修订、每世界 runtime snapshot 与可重建执行计划？
- 是否把 multi-world 吞吐、Presentation LOD 或未冻结 aggregate 当成单城市世界
  fidelity/性能证据？

## 实现偏好

- 优先小而可测的运行时原语。
- 优先显式输入输出，而非隐藏引擎状态。
- 优先确定性 tick 行为。
- 展示、mesh、动画、LOD、调试 UI 放在 Adapter 或 Presentation 层。

## 交付说明

当前 Core / 目标 Traffic Runtime 相关工作应汇报：

- 当前 Core 或目标 Traffic Runtime 行为变更
- API 或数据格式影响
- 已运行的测试或验证
- 文档或 ADR 是否更新
- 尚未解决的设计问题
