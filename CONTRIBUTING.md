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

不要在父任务名义下合入只覆盖子范围的实现。PR body 使用 `Closes #<issue>`；commit
footer 使用 `Refs: #<issue>`。

## 5. PR 合并策略

LaneFlow 默认通过 **Merge Queue** 合入 `main`，队列最终使用 **Rebase** 保持线性历史。详见
`docs/governance/github-workflow.md` 第 7 节。

- 当前 exact head 的 required checks 完成后入队：`gh pr merge <number> --repo illusion-tech/laneflow --match-head-commit <H_pr>`；不得在 pending 时预先武装 auto-merge
- `H_pr` 与真实 `H_mg` 都必须完成 `Commit message`、`Rust checks`、`Dependency policy`、`Analyze (actions)`、`Analyze (rust)`；原生 CodeQL rule 不替代队列中的两个 `Analyze` checks
- `main` 前进或队列重排只重建 Merge Group；未解决的 review conversation 继续由 GitHub 原生规则阻断
- Ruleset 保留 required checks、关闭 strict up-to-date，由 Merge Group 验证最新 `main` 组合
- 禁止日常 `--admin` 绕过队列；最终 merge method 例外必须先通过治理 Issue 修改规则

## 6. Commit Message

提交信息必须遵守 `docs/reference/commit-convention.md`。运行 `git config core.hooksPath .githooks` 后，仓库内置 `commit-msg` hook 可在本地提交前复用同一校验；CI 会再次检查 PR 和推送到 `main` 的 commit message。

推荐格式：

```text
feat(runtime): 校验 route segment 连续性

Refs: #12
```

提交标题遵循 Conventional Commits。footer 使用 `Refs: #<id>`；标题带 `!` 时必须有
`BREAKING CHANGE:`。标题中的 scope 可省略；使用时表示主要受影响的组件或职责域，
具体规则以 `docs/reference/commit-convention.md` 为准。不要再写 `Gate` / `Slice` /
`Impact` 等 G3 正文字段。

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

- Commit message：Conventional Commits 标题、`Refs` / `Closes`、必要时 `BREAKING CHANGE:`；`xtask` 构建使用 `Swatinem/rust-cache`（仅 `main` 写回缓存）。
- Markdown tables：表格格式检查只警告，不阻断合并。
- Rust checks：job 始终运行以保持 required check 稳定。变更触及 `crates/`、`xtask/`、`tools/`、`examples/`、`research/`、`Cargo.toml` / `Cargo.lock`、`deny.toml`、本 workflow 或 `docs/governance/github-workflow.md` 时，安装 Rust 1.96.0 并运行 `fmt` 与 `test --workspace --locked`。走廊 catalog 与 LFCA 对拍由 `laneflow-corridor-generator` 测试覆盖，不再单独跑 generator `check`。`schemas/road-editing/` 由独立 Codegen workflow 覆盖，不因 `.fbs` 拉起整仓 Rust 测试。Bevy `runtime_min` 与 `signalized_corridor` 在 Adapter、Runtime、Spatial、scenario、format、static-contract、static-network、compiler 或 `examples/data/` 变更时编译。纯文档等非 Rust 路径跳过重型 cargo 并显式记录 skip。
- Dependency policy：cargo-deny 检查 RustSec advisories、许可证、wildcard dependency 和 crate 来源。
- Analyze (actions) / Analyze (rust)：advanced CodeQL。
- Review conversation：未解决对话由 GitHub Ruleset 原生阻断；普通 Review 不由自定义 CI Check 计数或盖章。

数据 schema、Adapter build、示例 smoke test 和 Release 检查应在对应切片落地后继续加入专用门禁。

PR 中必须记录实际运行的检查。无法运行时，应说明原因和风险。

## 9. AI Agent 开发

AI Agent 可以参与设计、实现、测试和文档维护，但应遵守 `docs/governance/agent-development-guide.md`。

Agent 不应在未读取相关设计文档的情况下修改 Traffic Runtime API、数据格式或 Adapter 协议。

通用 Agent 工作流位于 `.agents/skills/`。Cursor 的 `.cursor/skills/` 只作为薄包装入口，规范本体仍以 `.agents/` 和 `docs/` 为准。

## 10. 许可证与贡献

LaneFlow 公开仓库采用 Apache-2.0-only，完整条款见根目录 `LICENSE`。

除非你在提交前明确书面标记为“Not a Contribution”，或与维护者另有书面协议，你有意提交且被项目接收的 Contribution 将按 Apache License 2.0 许可，不附加额外条款。提交者应确认自己有权提交相关代码、文档、数据或其他材料，并披露其中不受 Apache-2.0 覆盖的第三方内容。

当前项目不要求 CLA。若未来商业授权或知识产权管理需要不同贡献安排，必须先通过独立治理 Issue 更新本指南；不能追溯性地假定现有贡献授予了 Apache-2.0 之外的权利。

许可证允许商业使用不代表自动满足具体分发物的全部义务。发布者仍须复核第三方许可证、attribution、修改说明、商标和目标市场要求。依赖与发布门禁见 `docs/governance/dependency-security.md`。
