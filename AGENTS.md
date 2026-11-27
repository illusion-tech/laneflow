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

不要在工具专用说明文件中重复长期项目规则。仓库不再维护工具专用 Skill 副本：
`.agents/skills/` 是唯一正本，Cursor、Codex 等工具直接从此目录自动发现。

仓库不设研究资料专用目录。研究类任务的现行结论原位回写所属 `docs/design/`、ADR
或现行 reference，任务过程由 Issue / PR 承载，活测试数据放所属 crate 的
`tests/fixtures/`；逐区间或逐样本测量、搜索轨迹、全量结果集等研究过程数据留在
`target/` 或仓库外，一律不入仓，也不自发生产 manifest、逐字节一致性声明等归档物。
冻结研究数据不得被代码以 `include_str!` / `include_bytes!` 编译期嵌入，`#[ignore]`
重放实验经环境变量运行时读取。边界见 `docs/governance/documentation-policy.md`
研究结论与过程数据边界。

长期设计和 Agent 工作流中的以中文术语与中文定义为准，英文只作辅助理解；双语
映射统一由 `docs/reference/glossary.md` 管理。
