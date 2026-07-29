# LaneFlow Agent 工作流

`.agents/` 存放面向 LaneFlow 的、与具体工具无关的 AI 编码 Agent 工作流说明。

## 原则

采用单一事实源：

- `docs/`：长期项目事实、治理、架构、设计与决策。
- `.agents/skills/`：可复用的 Agent 执行工作流。
- `.cursor/skills/` 等工具专用入口：保持薄包装，并转读本目录。

## 可用 Skill

- `skills/laneflow-governance/SKILL.md`：GitHub Issue、PR、commit、Project、Milestone、Release 与 G0–G4 工作流（PR 默认 Rebase and merge）。
- `skills/laneflow-development/SKILL.md`：LaneFlow 通用实现工作流。
- `skills/laneflow-core-design/SKILL.md`：当前态 LaneFlow Core、目标态 LaneFlow
  交通运行时（Traffic Runtime）、当前道路机动车的车道图（Lane Graph）/路线
  （Route）/信号（Signal）/停车（Parking），目标交通参与单元（Traffic
  Participant Unit）与交通执行域（Traffic Execution Domain）、确定性并行、路网
  修订、快照/回放、路径规划接入与城市模拟游戏上层边界。
- `skills/laneflow-adapter/SKILL.md`：Unity、Unreal、Godot、O3DE、Web 等引擎适配器
  （Engine Adapter）开发及当前态核心（Current Core）→目标态交通运行时（Target
  Traffic Runtime）迁移边界。

## 使用方式

Agent 应选择与当前任务最相关、范围最小的 Skill。若任务跨多个领域，先读 governance，再读领域 Skill。

`laneflow-core-design` Skill 标识符（Skill ID）在阶段 8 生产切换 Issue #294 完成
G4 前保留，用于
发现当前态核心（Current Core）任务；其内容已经同时覆盖目标态交通运行时（Target
Traffic Runtime）。生产切换时再与 `laneflow-core/CoreWorld` 一起原子改名，避免
工具发现、当前 API 和目标文档提前分叉。

语言约定：长期设计和 Agent 工作流以中文术语、中文定义为权威事实，英文只作辅助
理解；双语映射遵循 `docs/reference/glossary.md`。技术标识符（切片类型、闸口
（Gate）、提交字段名、包（crate）、类型与协议常量）保留精确原文。

Accepted ADR 0021 把“为未来的中国特色城市模拟游戏提供交通基础”定义为 LaneFlow
的第一长期产品目标。涉及城市级范围、出行编排、路径规划（Routing）、路网修订、
存档/回放、并行或保真度（Fidelity）的任务必须读取该 ADR。#291 G1 已接受目标
边界，但不得把它写成当前已实现能力；该边界不得把城市经济/出行需求塞入交通运行时
（Traffic Runtime），也不得用引擎适配器细节层次（Adapter LOD）或多世界
（Multi-world）吞吐替代单个大型交通世界的正确性与性能。
