# LaneFlow Agent 工作流

`.agents/` 存放面向 LaneFlow 的、与具体工具无关的 AI 编码 Agent 工作流说明。

## 原则

采用单一事实源：

- `docs/`：长期项目事实、治理、架构、设计与决策。
- `.agents/skills/`：可复用的 Agent 执行工作流。
- `.cursor/skills/` 等工具专用入口：保持薄包装，并转读本目录。

## 可用 Skill

- `skills/laneflow-governance/SKILL.md`：GitHub Issue、PR、commit、Project、Milestone、Release 与 G0–G2 人流程及合并门禁（PR 默认通过 Merge Queue，队列最终 Rebase）。
- `skills/laneflow-development/SKILL.md`：LaneFlow 通用实现工作流。
- `skills/laneflow-core-design/SKILL.md`：LaneFlow 交通运行时（Traffic Runtime）
  设计。当前实现为 `TrafficWorld` 道路机动车特化（车道图（Lane Graph）/路线
  （Route）/信号（Signal）/等待与冲突裁决/停车（Parking）），覆盖确定性并行、
  路网修订与切换、快照/恢复、路径规划接入与宿主应用边界；长期方向为交通参与单元
  （Traffic Participant Unit）与交通执行域（Traffic Execution Domain）。
- `skills/laneflow-adapter/SKILL.md`：引擎适配器（Engine Adapter）开发。当前交付
  Bevy 参考适配器；Unity、Unreal、Godot、O3DE、Web 等遵循同一 Runtime/Spatial
  集成边界。
- `skills/laneflow-pre-1-0/SKILL.md`：1.0 前的开发态度。当前树只留一套权威，不为
  弯路堆兼容；G1/ADR 可改写。发布后失效。

## 使用方式

Agent 应选择与当前任务最相关、范围最小的 Skill。若任务跨多个领域，先读 governance，再读领域 Skill。

`laneflow-core-design` Skill 标识符（Skill ID）沿用历史名称，作为 Traffic Runtime
设计任务的发现入口，不表示仍存在 Core 运行时。`laneflow-core` crate 与 `CoreWorld`
已由 #301 拆除；Skill 改名不得反向保留可运行的 `CoreWorld`。

语言约定：长期设计和 Agent 工作流以中文术语、中文定义为权威事实，英文只作辅助
理解；双语映射遵循 `docs/reference/glossary.md`。技术标识符（切片类型、闸口
（Gate）、提交字段名、包（crate）、类型与协议常量）保留精确原文。

ADR 0021 定义 LaneFlow 的公开组件定位与宿主边界：可嵌入、引擎无关、确定性的
道路交通运行时与工具链。涉及地区交通场景、城市级范围、出行编排、路径规划
（Routing）、路网修订、存档/回放、并行或保真度（Fidelity）的任务必须读取该 ADR。
地区规则与城市工作负载用于通用技术验证，不指定具体产品或第一消费者；长期方向
不得写成当前已实现能力。城市经济与出行需求由宿主拥有，也不得用引擎适配器细节
层次（Adapter LOD）或多世界（Multi-world）吞吐替代单个大型交通世界的正确性与性能。
