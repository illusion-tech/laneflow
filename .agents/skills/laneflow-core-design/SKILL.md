---
name: laneflow-core-design
description: 处理 LaneFlow Core 设计与实现。适用于 core runtime、确定性 tick、vehicle state、lane graph、route、vehicle following、signal、intersection rules、parking 与 Core API 变更。
---

# LaneFlow Core 设计

## 先读这些

1. `docs/architecture.md`
2. `docs/adr/0001-project-scope.md`
3. `docs/design/README.md`
4. `docs/governance/development-gates.md`
5. 已存在的相关 Core design 文档

若所需 design 文档尚不存在，在对 Core 做高影响变更前，应先创建或提出最小设计基线。

## Core 边界

LaneFlow Core 负责：

- 车辆运行时状态
- 车道图遍历
- 路线跟随
- 前车避让
- 信号遵守
- 路口规则
- 停车行为
- 引擎无关的 tick 行为

LaneFlow Core 不得依赖：

- Unity API
- Unreal API
- Godot API
- O3DE API
- DOM 或 WebGL 展示 API
- 引擎特有的 actor、entity、prefab 或 scene object 模型

## 设计检查

对 Core 变更，必须显式确认：

- 是否改变 Core API？
- 是否改变数据格式假设？
- 是否影响 Adapter API？
- 是否需要确定性行为测试？
- 是否需要 ADR？

## 实现偏好

- 优先小而可测的运行时原语。
- 优先显式输入输出，而非隐藏引擎状态。
- 优先确定性 tick 行为。
- 展示、mesh、动画、LOD、调试 UI 放在 Adapter 或 Presentation 层。

## 交付说明

Core 相关工作应汇报：

- Core 行为变更
- API 或数据格式影响
- 已运行的测试或验证
- 文档或 ADR 是否更新
- 尚未解决的设计问题
