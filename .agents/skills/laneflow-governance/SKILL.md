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

- 新建或接手 Issue 时，先检查 Gate Ledger。
- 开始实现、文档修改或开 PR 前，Issue 必须已有 G0/G1/G2 记录；小型 `docs-only` 或 `governance` 任务可用一条开工记录覆盖 G0-G2，但必须发生在实现前。
- 任务不需要 G1 时，也必须记录不适用原因。
- 准备合并 PR 前，PR 必须已有 G3 记录，包含 checks、review、验证、风险、例外和合并方式。
- 清场时只补 G4；如果发现 G0-G3 缺失，必须标记为补救记录，并说明这是流程遗漏，不能当作标准流程。
- 任一 Gate 记录缺失且没有显式例外时，不得声称任务完成。

Issue Gate Ledger 模板：

```text
- [ ] G0 立项已记录：
- [ ] G1 设计判断已记录：
- [ ] G2 开工判断已记录：
- [ ] G3 合并判断已记录：链接 PR 中的 G3 判断
- [ ] G4 完成判断已记录：回写合并后收口结果
```

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

只有关联 Issue 满足 G4 完成边界时，才使用 `Closes: #<id>`；否则使用 `Refs: #<id>`。

PR commit message 必须符合 `docs/reference/commit-convention.md`；若存在例外，必须在 PR 中记录原因、风险和 Cleanup owner。

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
