---
name: laneflow-adapter
description: 处理 LaneFlow Engine Adapter 工作。适用于 Unity、Unreal、Godot、O3DE、Web、transform 同步、引擎生命周期集成、车辆表现、调试可视化、LOD 与 Adapter API 变更。
---

# LaneFlow Adapter

## 先读这些

1. `docs/architecture.md`
2. `docs/adr/0001-project-scope.md`
3. `docs/governance/development-gates.md`
4. `docs/governance/agent-development-guide.md`
5. `docs/design/adapter-api.md`（若已存在）

若 Adapter API 设计尚不存在，且任务会改变 Core 与 Adapter 的契约，应先提出 G1 设计缺口或创建最小设计基线。

## Adapter 边界

Adapter 负责：

- 引擎生命周期集成
- 调用 Core tick
- actor、entity、prefab 或 scene object 绑定
- transform 同步
- 车辆模型与动画绑定
- 调试可视化
- LOD 与渲染集成
- 示例场景集成

Adapter 不得：

- 把 Core 交通规则搬进引擎专用代码。
- 把引擎依赖引入 Core。
- 定义未文档化的数据格式语义。
- 在不更新 design 文档的情况下改变 Adapter API。

## 验证

Adapter 变更应记录：

- 目标引擎或运行时
- 构建结果（若可运行）
- 手工场景或示例验证
- transform 同步验证
- 调试可视化验证（若相关）
- Core API 与 Adapter API 影响

## 交付说明

Adapter 相关工作应汇报：

- 影响的引擎
- Adapter 行为变更
- Core API 或 Adapter API 影响
- 已运行的验证
- 文档是否更新或后续待办
