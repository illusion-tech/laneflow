---
name: laneflow-governance
description: 应用 LaneFlow 项目治理（GitHub Issue、PR、commit、Project、Milestone、Release、双语术语、文档边界、G0-G2 人流程与合并门禁）。适用于 governance、issue、PR、commit、workflow、terminology、milestone、project board、release、development gates 等任务。
---

# LaneFlow 治理

## 先读这些

1. `docs/governance/documentation-policy.md`
2. `docs/governance/github-workflow.md`
3. `docs/governance/development-gates.md`
4. `docs/reference/commit-convention.md`
5. `docs/reference/glossary.md`
6. `.github/pull_request_template.md`
7. `docs/adr/0026-merge-governance-rebuild.md`
8. 涉及安全设置、扫描或公开发布时，额外阅读 `docs/governance/security-scanning.md`
9. 涉及许可证、Cargo 依赖、RustSec、cargo-deny 或 Dependabot 时，额外阅读 `docs/governance/dependency-security.md`
10. 涉及产品定位、城市级范围、出行编排、Routing、路网修订、存档/回放、并行或
    fidelity 时，额外阅读 `docs/adr/0021-city-simulation-game-traffic-foundation.md`

## 工作流

1. 将任务归类为切片类型之一：`docs-only`、`governance`、`core-runtime`、
   `data-spec`、`adapter`、`authoring-tool`、`example`、`cross-layer`。
2. 人流程：G0 立项、G1 设计冻结（高影响变更）、G2 开工。这些写在 Issue 里给人看，
   机器不解析正文。
3. 区分 GitHub 与仓库文档：GitHub 记录当前状态与评审；仓库文档保存长期事实。
4. 稳定结论回写到 `docs/adr/`、`docs/design/` 或 `docs/governance/`。
5. 中文术语以 `docs/reference/glossary.md` 为权威；精确代码/协议标识符保留原文。
6. 交通仿真规模不得使用 `Agent` 或“代理”计数。

## Issue

- 使用仓库模板。不要填写已删除的元数据审计或 Gate Ledger。
- 侧边栏的 Project、Milestone、parent、blocked-by 直接在 GitHub 上维护。
- 默认一个 Issue 一个完成它的 PR。PR body 使用 `Closes #<issue>`。
- commit footer 使用 `Refs: #<issue>`。
- 高影响变更在实现前确认 ADR / design；文档 / 缺陷 / 调研不强制 G1。

## 外部审阅与合并

- `External Review` Check 只接受绑定当前 `H_pr` 的原生 `PullRequestReview`。
- 普通 PR 评论一律不算。作者自审不计入。
- 受信名单是默认分支上的 `.github/trusted-reviewers.json`。无法生成原生 Review
  的机器人不得列入。
- 未解决对话由 GitHub `required_review_thread_resolution` 拦截，不要自己数 thread。
- `main` 走 Merge Queue（最终 Rebase）：

```powershell
gh pr merge <number> --repo illusion-tech/laneflow --match-head-commit <H_pr>
```

- required checks（PR 与 `merge_group` 同名）：`Commit message`、`Rust checks`、
  `Dependency policy`、`Analyze (actions)`、`Analyze (rust)`、`External Review`。
  `External Review` 按 ADR 0026 先经真实队列验证再 required。
- 禁止日常 `--admin`。验证完成并撤 bypass 之前，所有者按 ADR 0026 清单操作。

## 提交说明

标题使用 Conventional Commits。footer 使用 `Refs: #<id>`；标题带 `!` 时必须有
`BREAKING CHANGE:`。不要再写 `Gate` / `Slice` / `Impact` / `Scope` /
`Validation` / `Docs`。

```text
<type>[optional scope][optional !]: <description>

Refs: #<id>
```

本地 hook：

```powershell
git config core.hooksPath .githooks
```

## 安全扫描

安全设置、扫描 workflow、依赖策略或公开发布任务以
`docs/governance/security-scanning.md` 与 `docs/governance/dependency-security.md`
为事实源。必须通过 GitHub API / Checks 读取实际配置；404、403、disabled 都不能
记为零告警。

## 交付说明

汇报治理类工作时，应包含：

- 改了什么
- 更新了哪些文档或 GitHub 模板
- PR 合并路径（默认 Merge Queue）与 `H_pr` / `H_mg` 检查
- 还有哪些必须在 GitHub 上手动完成的设置
