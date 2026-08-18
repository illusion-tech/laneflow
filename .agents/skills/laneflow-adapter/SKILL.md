---
name: laneflow-adapter
description: 处理 LaneFlow 引擎适配器（Engine Adapter）工作。适用于 Unity、Unreal、Godot、O3DE、Web、当前 Core/目标 Traffic Runtime 集成、变换（Transform）同步、引擎生命周期、当前车辆/目标交通参与单元表现、调试可视化、细节层次（LOD）与 Adapter API 变更。
---

# LaneFlow 引擎适配器（Engine Adapter）

## 先读这些

1. `docs/architecture.md`
2. `docs/adr/0001-project-scope.md`
3. `docs/governance/development-gates.md`
4. `docs/governance/agent-development-guide.md`
5. `docs/design/adapter-api.md`（若已存在）
6. `docs/reference/glossary.md`
7. 涉及 #291/#300 共享静态路网或交通运行时（Traffic Runtime）时，读取
   `docs/adr/0020-compiler-owned-static-network-and-static-image.md`、
   `docs/adr/0025-checked-canonical-network-and-shared-static-network.md`、
   `docs/design/network-compiler.md` 与 `docs/design/shared-static-network.md`
8. 涉及城市模拟游戏集成、存档/回放、路网切换或 fidelity 时，读取
   `docs/adr/0021-city-simulation-game-traffic-foundation.md`

若 Adapter API 设计尚不存在，且任务会改变当前 Core / 目标 Traffic Runtime 与
Adapter 的契约，应先提出 G1 设计缺口或创建最小设计基线。

## 命名与术语

- 当前态动态执行层使用 `LaneFlow Core` / `laneflow-core` / `CoreWorld`。
- #291 目标态使用中文规范名“LaneFlow 交通运行时（LaneFlow Traffic Runtime）”及
  精确标识符 `laneflow-runtime` / `TrafficWorld`。
- 当前 Adapter API 是 `VehicleHandle`/Entity 车辆特化；目标通用表现使用交通参与
  单元并按交通执行域区分。不得把当前车辆映射写成终态唯一 Adapter 模型，也不得用
  通用术语声称尚未实现的非机动车、行人或轨道表现能力。
- 中文术语和中文定义以 `docs/reference/glossary.md` 为权威；英文只作辅助理解，
  类型、crate、字段和引擎 API 保留精确原文。

## Adapter 边界

Adapter 负责：

- 引擎生命周期集成
- 调用当前 Core / 目标 Traffic Runtime 的固定步进（Fixed Tick）
- actor、entity、prefab 或 scene object 绑定
- transform 同步
- 当前车辆模型与动画绑定，以及未来已实现执行域的参与单元表现绑定
- 调试可视化
- LOD 与渲染集成
- 示例场景集成

Adapter 不得：

- 把 Core / Traffic Runtime 交通规则搬进引擎专用代码。
- 把引擎依赖引入 Core / Traffic Runtime。
- 定义未文档化的数据格式语义。
- 在不更新 design 文档的情况下改变 Adapter API。
- 用 LOD、可见性或帧预算静默改变 Traffic Runtime safety、通行权、fixed tick 或
  event；宿主只能显式暂停、慢放或统一改变模拟时间推进。
- 拥有出行需求、Routing、静态规则、运行时分区（Runtime Partition），或把宿主
  Entity/seed 写入共享静态路网/运行时快照权威；也不得单独替换 Traffic/Spatial
  component 或把内部连续数组当作稳定 Adapter ABI。

## 验证

Adapter 变更应记录：

- 目标引擎或运行时
- 构建结果（若可运行）
- 手工场景或示例验证
- transform 同步验证
- 调试可视化验证（若相关）
- Core / Traffic Runtime API 与 Adapter API 影响

## 交付说明

Adapter 相关工作应汇报：

- 影响的引擎
- Adapter 行为变更
- Core / Traffic Runtime API 或 Adapter API 影响
- 已运行的验证
- 文档是否更新或后续待办
