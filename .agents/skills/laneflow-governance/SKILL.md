---
name: laneflow-governance
description: 应用 LaneFlow 项目治理（GitHub Issue、PR、commit、Project、Milestone、Release、文档边界、G0-G4）。适用于 governance、issue、PR、commit、workflow、milestone、project board、release、development gates 等任务。
---

# LaneFlow 治理

## 先读这些

1. `docs/governance/documentation-policy.md`
2. `docs/governance/github-workflow.md`
3. `docs/governance/development-gates.md`
4. `docs/reference/commit-convention.md`
5. `.github/pull_request_template.md`

## 工作流

1. 将任务归类为 LaneFlow 切片类型之一：
   - `docs-only`
   - `governance`
   - `core-runtime`
   - `data-spec`
   - `adapter`
   - `authoring-tool`
   - `example`
   - `cross-layer`
2. 判断当前闸口：
   - `G0`：立项与范围
   - `G1`：设计冻结
   - `G2`：可开工
   - `G3`：可合并
   - `G4`：可完成、下游可依赖
3. 区分 GitHub 与仓库文档：
   - GitHub 记录当前状态与评审证据。
   - 仓库文档保存长期事实与决策。
4. 若 Issue、PR、Discussion 或对话中形成稳定结论，应回写到 `docs/adr/`、`docs/design/` 或 `docs/governance/`。

## Gate Ledger 硬性规则

Gate Ledger 必须按任务阶段增量记录，不得等到 G4 清场时一次性补 G0-G3。

执行规则：

- 新建或接手 Issue 时，先检查 Gate Ledger 和 GitHub 元数据 / 依赖关系审计。
- 开始实现、文档修改或开 PR 前，Issue 必须已有 G0/G1/G2 记录；小型 `docs-only` 或 `governance` 任务可用一条开工记录覆盖 G0-G2，但必须发生在实现前。
- 任务不需要 G1 时，也必须记录不适用原因。
- 准备合并 PR 前，PR 必须已有 `## G3 合并判断` comment，包含 Checks、审阅、验证、风险、例外、合并方式和 Gate 断言；PR body 与 Issue body 的 G3 checkbox 必须回链该 comment，且 `check-gate-evidence g3` 必须成功。
- 清场时只补 G4；如果发现 G0-G3 缺失，必须标记为补救记录，并说明这是流程遗漏，不能当作标准流程。
- 任一 Gate 记录缺失且没有显式例外时，不得声称任务完成。

Issue Gate Ledger 模板：

```text
- [ ] G0 立项已记录：
- [ ] G1 设计判断已记录：
- [ ] G2 开工判断已记录：
- [ ] G3 合并判断已记录：链接 Delivery PR 的 G3 comment；Related PR 如有均逐条链接
- [ ] G4 完成判断已记录：链接本 Issue 的 G4 comment
```

## Issue 元数据 / 依赖关系硬性规则

新建或接手 Issue 后，必须审计 GitHub 侧边栏和关系字段，而不是只看 Issue 正文。

必查字段：

- Project 与 Project status。
- Milestone；不适用时必须写明 `N/A` 原因。
- Labels。
- Parent / sub-issues；不适用时必须写明 `N/A` 原因。
- Blocked by；不适用时必须写明 `N/A` 原因。
- Blocking；不适用时必须写明 `N/A` 原因。
- Delivery PR；PR 创建前可写 `pending`，创建后记录唯一 `PR-number`，进入 G3 前必须确认其 `closingIssuesReferences` 覆盖目标 Issue，或说明不适用原因 / 显式例外。
- Related PRs；列出零到多个部分交付 PR；它们使用 `Refs: #<issue>`，没有时写 `N/A` 原因。

执行规则：

- 必需字段缺失且没有显式例外时，不得推进下一 Gate；不适用项缺少 `N/A` 原因时，不得推进下一 Gate。
- G2 开工前必须复核 Issue 元数据 / 依赖关系，并让 Project status 与当前 Gate 一致。
- 开 PR 前必须复核关联 Issue 的 G0/G1/G2 与元数据审计状态。
- 创建 Delivery PR 后，PR body 应使用 `Closes #<issue>` / `Resolves #<issue>` 建立 GitHub Development 关联；Related PR 使用 `Refs: #<issue>` 且不得误用 closing keyword。仓库关闭了 linked PR 自动关闭 Issue，Issue 仍由 G4 手动关闭。
- 常规 PR commit message 仍使用 `Refs: #<issue>`；不要为了 Development 关联把 commit footer 改成 `Closes`。
- G3 前默认必须用 `gh pr view <delivery-pr> --json closingIssuesReferences` 复核目标 Issue 是否被覆盖；发表 PR G3 comment 后，必须运行 `cargo +1.96.0 run --locked -p xtask -- check-gate-evidence g3 --repo <owner/repo> --issue <number> --delivery-pr <number> [--related-pr <number>]...`。GitHub Development 面板只作人工辅助证据。若只能手动关联 Development 面板，必须记录显式例外原因、风险、后续收口方式和 Cleanup owner；缺失且无显式例外时不得进入 `G3 = Pass`。
- 清场时只补 G4：在 Issue 发表 G4 comment，body 回链 permalink，Delivery PR 回链该 Issue G4；运行 `check-gate-evidence g4` 成功后才可关闭 Issue。若发现元数据或依赖关系漏项，必须标记为补救记录并说明流程遗漏原因。
- 本地分支不是长期 Development 关系证据；实施 PR 创建后必须关联 PR，或记录不适用原因。
- 若遇到非模板创建的 Issue，不得默认接受；必须先补齐模板中的元数据审计和 Gate Ledger，再推进 G0。

## 提交说明

遵循 `docs/reference/commit-convention.md`。

提交标题使用 Conventional Commits：

```text
<type>[optional scope][optional !]: <description>
```

提交正文保留 LaneFlow 治理字段：

```text
Gate: G3 Pass
Slice: governance
Impact: core-api=none; data-format=none; adapter-api=none
Scope: <what changed>
Validation: <commands or manual checks>
Docs: updated

Refs: #<id>
```

只有关联 Issue 满足 G4 完成边界时，才在 commit message footer 使用 `Closes: #<id>`；否则使用 `Refs: #<id>`。PR body 的 `Closes #<id>` / `Resolves #<id>` 用于 GitHub Development 关联，不改变常规 commit footer 规则。

PR commit message 必须符合 `docs/reference/commit-convention.md`；若存在例外，必须在 PR 中记录原因、风险和 Cleanup owner。

破坏性变更提交必须同时使用标题 `!` 和单行 `BREAKING CHANGE:` footer，并让 `Impact` 至少一项为 `changed`；`Refs` / `Closes` 仍保持最后一个非空 footer 行。

本地建议启用仓库内置 `commit-msg` hook，在提交前复用 `xtask` 校验：

```powershell
git config core.hooksPath .githooks
```

## PR 合并

默认使用 **Rebase and merge** 合入 `main`：

```powershell
gh pr merge <number> --rebase
```

例外使用 Squash 或 Merge commit 时，须在 PR 中说明原因。详见 `docs/governance/github-workflow.md` 第 7 节。

## 交付说明

汇报治理类工作时，应包含：

- 改了什么
- 支持哪个闸口或工作流
- 更新了哪些文档或 GitHub 模板
- PR 合并方式（默认 Rebase and merge）
- Gate Ledger 当前状态和缺失项
- 还有哪些必须在 GitHub 上手动完成的设置
