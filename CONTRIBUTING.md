# 贡献指南

感谢你参与 LaneFlow。

LaneFlow 采用 GitHub-first 治理：Issue 管任务，PR 管合并证据，仓库文档管长期设计事实。

## 1. 开始之前

参与开发前建议先阅读：

1. `README.md`
2. `docs/README.md`
3. `AGENTS.md`
4. `.agents/README.md`
5. `docs/governance/documentation-policy.md`
6. `docs/governance/github-workflow.md`
7. `docs/governance/development-gates.md`
8. `docs/governance/agent-development-guide.md`
9. `docs/reference/rust-code-style.md`

## 2. 提 Issue

所有非平凡任务应先创建 Issue。

Issue 应说明：

- 背景
- 目标
- 非目标
- 验收标准
- 影响范围
- 相关文档或 ADR

如果任务涉及 Core API、数据格式或 Adapter 协议，可能需要先补设计文档或 ADR。

## 3. 分支

推荐分支命名：

- `feature/<issue-id>-<short-name>`
- `fix/<issue-id>-<short-name>`
- `docs/<issue-id>-<short-name>`
- `design/<issue-id>-<short-name>`
- `adapter/<issue-id>-<engine-or-topic>`

`main` 应保持可发布或至少可演示状态。

## 4. Pull Request

PR 应使用仓库 PR 模板，并至少说明：

- 关联 Issue
- 本次变更范围
- 本次明确不做范围
- Core API 影响
- 数据格式影响
- Adapter API 影响
- 文档更新情况
- 测试与验证结果
- 已知风险和例外

不要在父任务名义下合入只覆盖子范围的实现。

## 5. PR 合并策略

LaneFlow 默认使用 **Rebase and merge** 合入 `main`，详见 `docs/governance/github-workflow.md` 第 7 节。

- 默认：`gh pr merge <number> --rebase`
- 例外使用 Squash 或 Merge commit 时，须在 PR 中说明原因

## 6. Commit Message

提交信息必须遵守 `docs/reference/commit-convention.md`。运行 `git config core.hooksPath .githooks` 后，仓库内置 `commit-msg` hook 可在本地提交前复用同一校验；CI 会再次检查 PR 和推送到 `main` 的 commit message。

推荐格式：

```text
feat(core): 校验 route segment 连续性

Gate: G3 Pass
Slice: core-runtime
Impact: core-api=changed; data-format=none; adapter-api=none
Scope: 增加 route edge sequence 连通性校验
Validation: cargo +1.96.0 test --workspace --locked
Docs: updated

Refs: #12
```

提交标题遵循 Conventional Commits，正文保留 LaneFlow 治理字段。只有满足 G4 完成边界时，才使用 `Closes: #<id>`；否则使用 `Refs: #<id>`。

## 7. 文档要求

长期结论应进入仓库文档：

- 架构决策进入 `docs/adr/`。
- 具体设计进入 `docs/design/`。
- 流程和治理进入 `docs/governance/`。
- 术语、模板和通用约定进入 `docs/reference/`。

GitHub Issue、PR 和 Discussion 中形成的稳定结论，应回写到仓库文档。

## 8. 测试与验证

Rust 代码除通过 `rustfmt` 和 Clippy 外，还应遵守 `docs/reference/rust-code-style.md` 中的仓库级可读性约定。数字字面量格式审阅只应覆盖当前变更范围；历史不一致应通过独立 Issue 有界清理。

当前 CI 包含：

- Governance checks：必需治理文件存在、Markdown 文件非空、commit message 符合提交规范。
- Rust checks：Rust 1.96.0 工具链确认、workspace 格式检查和 workspace 测试。
- Dependency policy：cargo-deny 检查 RustSec advisories、许可证、wildcard dependency 和 crate 来源。

数据 schema、Adapter build、示例 smoke test 和 Release 检查应在对应切片落地后继续加入专用门禁。

PR 中必须记录实际运行的检查。无法运行时，应说明原因和风险。

## 9. AI Agent 开发

AI Agent 可以参与设计、实现、测试和文档维护，但应遵守 `docs/governance/agent-development-guide.md`。

Agent 不应在未读取相关设计文档的情况下修改 Core API、数据格式或 Adapter 协议。

通用 Agent 工作流位于 `.agents/skills/`。Cursor 的 `.cursor/skills/` 只作为薄包装入口，规范本体仍以 `.agents/` 和 `docs/` 为准。

## 10. 许可证与贡献

LaneFlow 公开仓库采用 Apache-2.0-only，完整条款见根目录 `LICENSE`。

除非你在提交前明确书面标记为“Not a Contribution”，或与维护者另有书面协议，你有意提交且被项目接收的 Contribution 将按 Apache License 2.0 许可，不附加额外条款。提交者应确认自己有权提交相关代码、文档、数据或其他材料，并披露其中不受 Apache-2.0 覆盖的第三方内容。

当前项目不要求 CLA。若未来商业授权或知识产权管理需要不同贡献安排，必须先通过独立治理 Issue 更新本指南；不能追溯性地假定现有贡献授予了 Apache-2.0 之外的权利。

许可证允许商业使用不代表自动满足具体分发物的全部义务。发布者仍须复核第三方许可证、attribution、修改说明、商标和目标市场要求。依赖与发布门禁见 `docs/governance/dependency-security.md`。
