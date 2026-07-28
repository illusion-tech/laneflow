---
name: laneflow-core-design
description: 处理 LaneFlow 当前态核心运行时（Current Core Runtime）与目标态交通运行时（Target Traffic Runtime）设计。适用于确定性固定步进（Tick）、车辆状态（Vehicle State）、车道图（Lane Graph）、路线（Route）、跟车（Vehicle Following）、信号（Signal）、路口规则（Intersection Rules）、停车（Parking），以及 Core/Runtime API 变更。
---

# LaneFlow 交通运行时设计（Traffic Runtime Design）

Skill 标识符（Skill ID）`laneflow-core-design` 在 #291 生产切换 G4 前保留，作为
当前态核心（Current Core）与目标态交通运行时（Target Traffic Runtime）的兼容发现
入口；它不表示目标态继续命名为 Core。

## 先读这些

1. `docs/architecture.md`
2. `docs/adr/0001-project-scope.md`
3. `docs/design/README.md`
4. `docs/governance/development-gates.md`
5. `docs/reference/glossary.md`
6. 已存在的相关 Core / Traffic Runtime design 文档
7. 涉及 #291 静态路网编译、静态镜像（Static Image）或 core→runtime 切换时，读取
   `docs/adr/0020-compiler-owned-static-network-and-static-image.md` 与
   `docs/design/network-compiler.md`

若所需 design 文档尚不存在，在对当前 Core 或目标 Traffic Runtime 做高影响变更
前，应先创建或提出最小设计基线。

## 当前态与目标态命名边界

- 当前态：中文规范名“LaneFlow 核心（LaneFlow Core）”，精确标识符
  `laneflow-core` / `CoreWorld`。
- #291 目标态：中文规范名“LaneFlow 交通运行时（LaneFlow Traffic Runtime）”，
  精确标识符 `laneflow-runtime` / `TrafficWorld`。
- ADR 0020 Accepted 且生产切换 G4 前，代码与现有 API 继续使用当前态名称；目标
  设计不得把 `Core` 当成终态名称。
- 中文术语和中文定义以 `docs/reference/glossary.md` 为权威，英文只作辅助理解。

## 动态执行层边界

当前 Core / 目标 Traffic Runtime 负责：

- 车辆运行时状态
- 车道图遍历
- 路线跟随
- 前车避让
- 信号遵守
- 路口规则
- 停车行为
- 引擎无关的固定步进（Fixed Tick）行为

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
