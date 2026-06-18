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

## 提交说明

遵循 `docs/reference/commit-convention.md`。

只有关联 Issue 满足 G4 完成边界时，才使用 `closes #<id>`；否则使用 `refs #<id>`。

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
- 还有哪些必须在 GitHub 上手动完成的设置
