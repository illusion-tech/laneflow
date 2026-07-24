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
6. 涉及安全设置、扫描或公开发布时，额外阅读 `docs/governance/security-scanning.md`
7. 涉及许可证、Cargo 依赖、RustSec、cargo-deny 或 Dependabot 时，额外阅读 `docs/governance/dependency-security.md`

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
- 准备合并 PR 前，必须取得当前 head 上一个有效外部 reviewer 的完成态审阅；有 findings 时，完成处置后还必须取得新的当前 head clean re-review。PR author 的自审是 G3 owner 职责，但不计入外部 reviewer。
- PR 必须新增一条 append-only 的 `## G3 合并判断` comment，包含当前 head、rollout phase、Checks、External Review Gate、结构化审阅证据、review threads、验证、风险、例外、合并方式和 Gate 断言；不得编辑旧 head 的 G3 comment 冒充当前结论。PR body 的 G3 checkbox 必须回链当前 comment；Issue body 的 G3 Gate Ledger 对 Related PR 增量回链但保持未勾选，直到 Delivery PR 与全部 Related PR 均完成。`Gate 断言` 行必须包含与实际参数完全一致的反引号命令并明确写 `已通过`，且 `check-gate-evidence g3` 必须成功。
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
- G3 前默认必须用 `gh pr view <delivery-pr> --json closingIssuesReferences` 复核目标 Issue 是否被覆盖。每个 Related PR 都用 `cargo +1.96.0 run --locked -p xtask -- check-gate-evidence g3 --repo <owner/repo> --issue <number> --related-pr <current-related-pr>` 独立验证，把 comment permalink 增量写入仍未勾选的 Issue G3 Gate Ledger，并永久保留 Related-only 断言；Delivery PR 用 `--delivery-pr <number>` 加 Issue 已记录的全部 `--related-pr` 做整组复核，不改写历史 Related comment。`Gate 断言` 使用与实际调用完全一致的完整命令并在反引号后写 `已通过`；若运行失败，立即移除成功标记并修复证据。GitHub Development 面板只作人工辅助证据。若只能手动关联 Development 面板，必须记录显式例外原因、风险、后续收口方式和 Cleanup owner；缺失且无显式例外时不得进入 `G3 = Pass`。
- 清场时只补 G4：在 Issue 发表 G4 comment，body 回链 permalink，Delivery PR 回链该 Issue G4；G4 `Gate 断言` 同样必须记录完整命令和 `已通过` 结果，运行 `check-gate-evidence g4` 成功后才可关闭 Issue。若发现元数据或依赖关系漏项，必须标记为补救记录并说明流程遗漏原因。
- 本地分支不是长期 Development 关系证据；实施 PR 创建后必须关联 PR，或记录不适用原因。
- 若遇到非模板创建的 Issue，不得默认接受；必须先补齐模板中的元数据审计和 Gate Ledger，再推进 G0。

## 外部审阅门禁

完整契约以 `docs/governance/development-gates.md` 和 `docs/governance/github-workflow.md` 为准，本 Skill 只保留执行入口：

- 标准路径只接受 trusted reviewer 对 PR 当前 exact head 的完成态审阅；`unresolved review threads = 0` 只是必要条件，不能替代外部审阅证据。
- reviewer 报告 findings 后，author 必须记录每项 disposition，并在修复后的当前 head 请求新的 clean re-review；旧 head 的 approval、无新评论或仅解决线程都不能沿用。
- 单维护者场景不降低门槛：维护者可以且应当自审、处置 findings 并发表 G3 comment，但必须另有一个有效外部 reviewer。
- R0/R1 尚未具备 required check 时，按文档中的 bootstrap 规则显式记录阶段和缺失项。Related PR B 自身不能用候选 validator 自批，仍由 G3 Owner 人工核验新增外部审阅字段；PR B 合入后，后续 PR 的 `check-gate-evidence g3` 还必须取得 live `check-external-review` exact-head `pass`。进入 R2 后，`External Review Gate` Check success 与当前 head 的 append-only G3 comment 构成双钥匙。
- Related PR C 自身不能使用尚未合入 default branch 的候选 shadow workflow 自批；使用 main 上的 live validator 完成 exact-head 判断，并在 G3 comment 记录 Check 尚未发布 / required 的 R0 bootstrap 边界。PR C 合入、首次 trusted-ref Check 验证与 R1 起点 comment 完成前，不开始 14 天 / 10 eligible PR 计时。
- content-equivalent rebase、provider / platform outage、security / emergency hotfix、confirmed gate defect 只能走文档定义的显式例外；current comment 必须写 `- Gate 结果：G3 Waived` 并提供 `external-review-waiver:v1` 结构化、未过期证据，validator 保持 `waived` 而不映射成 `pass`；不得扩展成日常 bypass。
- fork / cross-repository PR 不计入 R1 eligible sample，也不能在 R2 以缺失 `External Review Gate` Check 合并；必须把最终 patchset 迁移到 same-repository PR，并对新 PR exact head 重新完成外部审阅与 G3。

## 安全扫描

- 安全设置、扫描 workflow、依赖策略或公开发布任务必须以 `docs/governance/security-scanning.md` 为长期事实源。
- 必须通过 GitHub API / Checks 读取实际配置、最近适用分析和开放告警；404、403、disabled、not-configured、无分析或命令失败都不能记为零告警。
- 修改 CodeQL、Secret Scanning、push protection 或 ruleset 时，先记录设计与开工 Gate，操作后保存设置前后和首次适用分析证据。
- ruleset bypass 或 push protection bypass 不改变扫描结论；使用 bypass 时仍按例外规则记录原因、风险、接受边界和 Cleanup owner。
- 源代码许可证、第三方许可证、cargo-deny 与 Dependabot 更新策略以 `docs/governance/dependency-security.md` 为事实源；新增或更新依赖的 PR 必须记录许可证、来源、漏洞和分发影响。

## 提交说明

遵循 `docs/reference/commit-convention.md`。

提交标题使用 Conventional Commits：

```text
<type>[optional scope][optional !]: <description>
```

提交正文保留 LaneFlow 治理字段：

```text
Gate: G3 Candidate
Slice: governance
Impact: core-api=none; data-format=none; adapter-api=none
Scope: <what changed>
Validation: <commands or manual checks>
Docs: updated

Refs: #<id>
```

`Gate: G3 Candidate` 只表示该 commit 可进入 PR 级 G3 判断；正式 `G3 Pass` 只存在于当前 head 的 Check 和 append-only G3 comment。阻断中的本地提交可使用 `Gate: G3 Block`，但它不得进入 PR / push 合并范围。

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
