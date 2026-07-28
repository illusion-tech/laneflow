# Copilot 仓库说明

本文件只作为 Copilot on GitHub 的仓库级薄包装，不保存 LaneFlow 的长期架构、领域或
治理规则。正式事实以仓库通用入口、`.agents/`、`docs/`、PR 模板、CI 与 GitHub
实时元数据为准。

官方能力说明见 GitHub Docs:
<https://docs.github.com/en/copilot/how-tos/copilot-on-github/customize-copilot/add-custom-instructions/add-repository-instructions>。

## 必读入口

1. 先读取并遵循 `AGENTS.md`；由它转读 `README.md`、`docs/README.md`、
   `.agents/README.md`、治理文档和与任务匹配的 `.agents/skills/*/SKILL.md`。
2. 审阅 PR 时同时读取 `.github/pull_request_template.md`；需要判断当前 GitHub
   状态时，通过 GitHub UI、API、`gh` 或 GraphQL 核验，不从本文件推断。
3. 任务涉及具体领域时，由通用 Skill 和文档导航选择相关 ADR、design、reference
   与 validation 文档；不得把本提示层当作这些事实源的摘要替代品。

## Review 输出

- 默认使用中文反馈；技术标识符、命令、文件路径和 API 名称保留精确原文。
- 只提出有明确影响和可执行修复方向的问题，并说明严重程度、影响范围及对应事实源
  或验证证据；避免只表达个人风格偏好。
- 严格区分 current、Proposed target 与 Accepted decision。不得仅凭本文件把提案
  写成当前实现或已接受事实。
- GitHub 元数据、外部审阅、Gate Ledger、commit 规范、代码风格与各切片边界均按
  必读入口指向的实时状态和事实源检查，不在这里复制其规则正文。

## 限制

- 本文件与通用事实源冲突时，以通用事实源为准，并修复本薄包装中的错误引用。
- Copilot review 对 custom instructions 的生效以 base branch 中的 instructions
  为准；修改本文件的 PR 不一定影响该 PR 当前这轮 review。
