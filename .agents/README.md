# LaneFlow Agent 工作流

`.agents/` 存放面向 LaneFlow 的、与具体工具无关的 AI 编码 Agent 工作流说明。

## 原则

采用单一事实源：

- `docs/`：长期项目事实、治理、架构、设计与决策。
- `.agents/skills/`：可复用的 Agent 执行工作流。
- `.cursor/skills/` 等工具专用入口：保持薄包装，并转读本目录。

## 可用 Skill

- `skills/laneflow-governance/SKILL.md`：GitHub Issue、PR、commit、Project、Milestone、Release 与 G0–G4 工作流。
- `skills/laneflow-development/SKILL.md`：LaneFlow 通用实现工作流。
- `skills/laneflow-core-design/SKILL.md`：Core 运行时、lane graph、route、signal、parking 与确定性行为。
- `skills/laneflow-adapter/SKILL.md`：Unity、Unreal、Godot、O3DE、Web 等 Engine Adapter 开发。

## 使用方式

Agent 应选择与当前任务最相关、范围最小的 Skill。若任务跨多个领域，先读 governance，再读领域 Skill。

语言约定：模板与治理规范中文优先；技术标识符（切片类型、Gate、commit 字段名）可保留英文。
