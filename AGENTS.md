# Agent 说明

LaneFlow 采用唯一事实来源管理 Agent 工作流。

开工前请先阅读：

1. `README.md`
2. `docs/README.md`
3. `.agents/README.md`
4. `docs/governance/agent-development-guide.md`
5. `docs/governance/development-gates.md`
6. `docs/reference/glossary.md`

按任务类型阅读对应 Skill：

- `.agents/skills/laneflow-governance/SKILL.md`
- `.agents/skills/laneflow-development/SKILL.md`
- `.agents/skills/laneflow-core-design/SKILL.md`
- `.agents/skills/laneflow-adapter/SKILL.md`
- `.agents/skills/laneflow-pre-1-0/SKILL.md`（1.0 前：一套正本，不为弯路堆兼容）

不要在工具专用说明文件中重复长期项目规则。`.cursor/skills/` 等工具入口应保持薄包装，并转读 `.agents/` 与 `docs/`。

研究类任务只向 `research/` 提交研究结论与复现输入；逐区间或逐样本测量、搜索轨迹、
全量结果集等研究过程数据一律不入仓，也不自发生产 manifest、逐字节一致性声明等
归档物。边界见 `docs/governance/documentation-policy.md` 研究结论与过程数据边界。

长期设计和 Agent 工作流中的以中文术语与中文定义为准，英文只作辅助理解；双语
映射统一由 `docs/reference/glossary.md` 管理。
