---
name: laneflow-governance
description: 应用 LaneFlow 项目治理（GitHub Issue、PR、commit、Project、Milestone、Release、双语术语、文档边界、G0-G4）。适用于 governance、issue、PR、commit、workflow、terminology、milestone、project board、release、development gates 等任务。
---

# LaneFlow 治理

## 先读这些

1. `docs/governance/documentation-policy.md`
2. `docs/governance/github-workflow.md`
3. `docs/governance/development-gates.md`
4. `docs/reference/commit-convention.md`
5. `docs/reference/glossary.md`
6. `.github/pull_request_template.md`
7. 涉及安全设置、扫描或公开发布时，额外阅读 `docs/governance/security-scanning.md`
8. 涉及许可证、Cargo 依赖、RustSec、cargo-deny 或 Dependabot 时，额外阅读 `docs/governance/dependency-security.md`
9. 涉及产品定位、城市级范围、出行编排、Routing、路网修订、存档/回放、并行或
   fidelity 时，额外阅读 `docs/adr/0021-city-simulation-game-traffic-foundation.md`
10. 准备 #292 G3/G4、推进 #315/#296/#297、审阅官方前端共同接入、current JSON
    退役或编译器性能验收、
    或同步这些议题的 Gate Ledger 时，额外阅读 `docs/design/compiler-foundation.md` 与
    `docs/reference/v0.10-compiler-foundation-validation.md`；推进 #296 FlatBuffers G2/G3 时还须读取
    `docs/adr/0023-road-editing-state-and-phased-network-replacement.md` 与
    `docs/design/road-editing-source-and-geometry-frontend.md`
11. 推进 #298 Gate、审阅 portable canonical artifact、source map、semantic diff、
    canonical publication descriptor 或原子发布治理时，额外读取
    `docs/design/portable-canonical-artifact.md` 与
    `docs/reference/v0.10-portable-artifact-validation.md`；G1 内容阻断项未闭合前不得
    记录 G1 Pass，正式 G2 开工判断前不得启动实现

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
5. 审阅 ADR、design、architecture、Issue 设计说明和 Agent Skill 时，检查领域术语
   是否已有中文规范名与英文辅助名；中文定义以 `docs/reference/glossary.md` 为
   权威，精确代码/协议标识符保留原文。
6. 审阅面向读者的数量时，以数量级、单位和计数对象清楚且无歧义为必需条件；零较多
   的中文正文建议使用规范中文数量，表格、公式和机器输入建议使用完整十进制数字。
   `k`/`M` 等缩写和 `1 万` 等写法在上下文明确时可以保留，仅作可读性与一致性建议，
   不设自动门禁；小写 `m` 应特别避免与米混淆。交通仿真规模不得使用 `Agent` 或
   “代理”计数。#215 Accepted 的 current 五项车辆计数继续约束现有实现、工作负载与
   历史证据；#291 已接受的长期通用规模使用交通参与单元，并按交通执行域报告六类
   目标计数。目标计数不得暗示目标态已经实现。当前车辆 workload 应明确标注车辆
   特化，不得代表非机动车、行人或轨道交通；不可改标识符按原文保留。
7. 审阅历史 closure、验证与基准记录时，按 `documentation-policy.md` 保留当时的
   证据语义、研究归因和能力边界；术语等价更新不得用后续 Proposed 架构重写历史。
8. 产品北极星或城市游戏/交通职责边界发生实质变化时，必须回写 ADR、architecture、
   roadmap、glossary 和相关 Skills，并对当前 exact head 重新取得 G1 clean review；
   不得沿用旧 head 的 G1/G3 结论。

## Gate Ledger 硬性规则

Gate Ledger 必须按任务阶段增量记录，不得等到 G4 清场时一次性补 G0-G3。

执行规则：

- 新建或接手 Issue 时，先检查 Gate Ledger 和 GitHub 元数据 / 依赖关系审计。
- 开始实现、文档修改或开 PR 前，Issue 必须已有 G0/G1/G2 记录；小型 `docs-only` 或 `governance` 任务可用一条开工记录覆盖 G0-G2，但必须发生在实现前。
- 任务不需要 G1 时，也必须记录不适用原因。
- 准备合并 PR 前，必须取得当前 head 上一个有效外部 reviewer 的完成态审阅；有 findings 时，完成处置后还必须取得新的当前 head clean re-review。PR author 的自审是 G3 owner 职责，但不计入外部 reviewer。
- PR 必须有一条 current `## G3 合并判断` comment，包含当前 head、rollout phase、Checks、External Review Gate、结构化审阅证据、review threads、验证、风险、例外、合并方式和 Gate 断言。canonical shadow 行按 rollout phase 从下列 R0/R1/R2 三种值中只保留一项，且不包裹整个值；历史完整值的一层反引号只作兼容。允许在 PR 合并前纠错编辑：未编辑时以 `createdAt`、编辑后以 REST 核验的 `updatedAt` 作为生效时间，重新验证全部当前证据并新增严格更晚的 marker。合并后仅可用 append-only `g3-comment-correction:v1` 恢复经 UserContentEdit.diff 证明的完整 shadow 包裹格式差异；它不能改变 Gate 结果。PR body 与 Issue Ledger 必须回链 current comment。

```text
- G3 Evidence Gate Shadow：候选 workflow bootstrap：<边界>
- G3 Evidence Gate Shadow：R1 non-required：<原因>
- G3 Evidence Gate Shadow：Check URL：https://github.com/...
```
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
- G3 前默认必须用 `gh pr view <delivery-pr> --json closingIssuesReferences` 复核目标 Issue 是否被覆盖。每个 Related PR 都用 `cargo +<workspace-rust-version> run --locked -p xtask -- check-gate-evidence g3 --repo <owner/repo> --issue <number> --related-pr <current-related-pr>` 独立验证；命令版本由 `xtask` package `rust-version` 生成，gate-command v1 语义兼容从 `1.96.0` 到当前 MSRV 的 `1.<minor>.0` stable release；未显式纳入策略的补丁版本失败关闭。Delivery PR 用 `--delivery-pr` 加完整 Related set。普通 `G3 Pass` 断言必须写 `已通过`；确认 gate 缺陷只能写 `G3 Exception` + `未通过`，另发未编辑的 `g3-exception:v1`，机器状态保持 `accepted_exception`、最长 24 小时、Shadow non-success；多 Issue PR 只把匹配 exception 记录的 Issue 断言保留为 `未通过`，其余 Issue 可独立 `已通过`。历史 `G3 Block` / `Pass + 未通过` 只能由 Issue G4 comment 的 `legacy_evidence_reconstruction` 事件重放，不能放在 PR comment，也不能追授原 merge 合规，且在实际 G4 evaluation time 必须仍未过期。waiver、correction 与 exception 三种记录不可互换。
- 清场时只补 G4：在 Issue 发表 G4 comment，body 回链 permalink，Delivery PR 回链该 Issue G4；G4 `Gate 断言` 必须记录完整命令，普通 target 写 `已通过`，仅有精确匹配 `legacy_evidence_reconstruction` 的历史 target 保留 `未通过`，运行 `check-gate-evidence g4` 成功后才可关闭 Issue。若发现元数据或依赖关系漏项，必须标记为补救记录并说明流程遗漏原因。
- 若 Delivery 合并后才创建 late Related PR，禁止编辑历史 Delivery G3 或补发 post-merge G3；只有在 `development-gates.md` 的严格条件全部满足时，才可在新的 append-only G4 comment 使用 `g3-full-set-recovery:v1`。该结构化记录必须绑定原/新增 Related 集合、Delivery merge 时间、逐 PR G3 permalinks、风险、接受边界、follow-up Issue、Cleanup owner 和 trusted G3 Owner 授权；normal G3 行为不变。
- 本地分支不是长期 Development 关系证据；实施 PR 创建后必须关联 PR，或记录不适用原因。
- 若遇到非模板创建的 Issue，不得默认接受；必须先补齐模板中的元数据审计和 Gate Ledger，再推进 G0。

## 外部审阅门禁

完整契约以 `docs/governance/development-gates.md` 和 `docs/governance/github-workflow.md` 为准，本 Skill 只保留执行入口：

- 标准路径只接受 trusted reviewer 对 PR 当前 exact head 的完成态审阅；`unresolved review threads = 0` 只是必要条件，不能替代外部审阅证据。
- reviewer 报告 findings 后，author 必须记录每项 disposition，并在修复后的当前 head 请求新的 clean re-review；旧 head 的 approval、无新评论或仅解决线程都不能沿用。
- 受信任 Codex provider 的 clean completion 缺少可解析 `Reviewed commit` SHA 时走受控绑定路径：G3 Owner 在 PR 新增正文精确为 `external-review: request-codex-review` 的 comment，由 trusted `Codex Clean Binding` workflow 发布受控请求 marker 并在 clean 到达后发布绑定记录；任何人不得手工伪造 marker 或绑定记录，缺失或歧义均失败关闭。
- 单维护者场景不降低门槛：维护者可以且应当自审、处置 findings 并发表 G3 comment，但必须另有一个有效外部 reviewer。
- R0/R1 尚未具备 required check 时，按文档中的 bootstrap 规则显式记录阶段和缺失项。Related PR B 自身不能用候选 validator 自批，仍由 G3 Owner 人工核验新增外部审阅字段；PR B 合入后，后续 PR 的 `check-gate-evidence g3` 还必须取得 live `check-external-review` exact-head `pass`。进入 R2 后，`External Review Gate` Check success 与 current G3 Owner comment 构成双钥匙；comment 编辑后以 `updatedAt` 重新生效并要求新 marker。
- Related PR C 自身不能使用尚未合入 default branch 的候选 shadow workflow 自批；使用 main 上的 live validator 完成 exact-head 判断，并在 G3 comment 记录 Check 尚未发布 / required 的 R0 bootstrap 边界。PR C 合入、首次 trusted-ref Check 验证与 R1 起点 comment 完成前，不开始 14 天 / 10 eligible PR 计时。
- content-equivalent rebase、provider / platform outage、security / emergency hotfix 只能走文档定义的显式 waiver；current comment 必须写 `- Gate 结果：G3 Waived` 并提供 `external-review-waiver:v1` 结构化、未过期证据，validator 保持 `waived` 而不映射成 `pass`。confirmed gate defect 不使用 current waiver，只能写 `G3 Exception` + `confirmed_gate_defect` 并保持 `accepted_exception` 非成功状态；策略切换前已合并的旧 `G3 Waived + confirmed_gate_defect` 仅 grandfather G4 merge-time replay。两类路径都不得扩展成日常 bypass。
- fork / cross-repository PR 不计入 R1 eligible sample，也不能在 R2 以缺失 `External Review Gate` Check 合并；必须把最终 patchset 迁移到 same-repository PR，并对新 PR exact head 重新完成外部审阅与 G3。
- R1 的 `External Review Gate Shadow` / `github-actions` 只用于 non-required telemetry，绝不能直接成为 required Check。R2 必须先改由独立专用 GitHub App 发布正式 `External Review Gate`，在 ruleset 绑定 expected source App，并用同名 Actions spoof canary 证明 PR 代码无法伪造通过。
- R1 每批 resolve / unresolve review threads 后必须新增顶层 `external-review: thread-state-changed` comment 并等待 shadow publisher 重读；R2 必须由专用 GitHub App 的 `pull_request_review_thread` webhook 或等价自动信号实测覆盖两个方向。

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

`Gate: G3 Candidate` 只表示该 commit 可进入 PR 级 G3 判断；正式 `G3 Pass` 只存在于当前 head 的 Check 和 current G3 Owner comment，合并前编辑按最新 `updatedAt` 重验。阻断中的本地提交可使用 `Gate: G3 Block`，但它不得进入 PR / push 合并范围。

只有关联 Issue 满足 G4 完成边界时，才在 commit message footer 使用 `Closes: #<id>`；否则使用 `Refs: #<id>`。PR body 的 `Closes #<id>` / `Resolves #<id>` 用于 GitHub Development 关联，不改变常规 commit footer 规则。

PR commit message 必须符合 `docs/reference/commit-convention.md`；若存在例外，必须在 PR 中记录原因、风险和 Cleanup owner。

破坏性变更提交必须同时使用标题 `!` 和单行 `BREAKING CHANGE:` footer，并让 `Impact` 至少一项为 `changed`；`Refs` / `Closes` 仍保持最后一个非空 footer 行。

本地建议启用仓库内置 `commit-msg` hook，在提交前复用 `xtask` 校验：

```powershell
git config core.hooksPath .githooks
```

## PR 合并

`main` 要求使用合并队列（Merge Queue）；队列规则控制最终使用 **Rebase**，操作者不再为
日常 PR 选择 merge strategy。入队前冻结 current exact head `H_pr`，确认 current-head 外部审阅、
finding disposition、G3 Owner comment、marker 与 PR 级 required checks 仍有效，然后使用 head guard：

```powershell
gh pr merge <number> --repo illusion-tech/laneflow --match-head-commit <H_pr>
```

队列启用时必须保留 required checks，并把 `required_status_checks.strict_required_status_checks_policy`
设为 `false`；最新 `main` 组合由 `H_mg` 检查，不得再以 strict up-to-date 要求未变化 `H_pr` 手工 rebase。

只有 current exact `H_pr` 的 required checks、适用 CodeQL、外部审阅、finding disposition、G3 Owner
comment 与 marker 全部完成且有效后才能运行命令；不得在 pending 时预先武装 auto-merge，
`--match-head-commit` 不能替代 maintainer 后续 push 后的新 head 授权。`main` 前进或队列顺序变化只替换
`H_mg` 并重跑队列级机器检查，不使未变化 `H_pr` 上的人审失效。任何 push、
force-push 或冲突修复改变 `H_pr` 后，旧审阅、G3 与入队资格全部 stale，必须对新 head 重走生命周期。

禁止使用 `--admin` 绕过队列。若 live Ruleset 未要求队列、队列未生成真实 `H_mg`，或 required checks /
CodeQL 缺失或失败，停止合并并按关联 Issue 的 activation / rollback 契约处置；不得回退为无记录的直接合并。
最终 merge method、例外和 G4 的 `H_pr → H_mg → H_main` 证据以
`docs/governance/github-workflow.md` 第 7 节为准。Merge Queue G4 comment 必须用
`merge-queue-g4-evidence:v1` 按 Delivery-first、随后全部 Related PR 顺序逐项记录；activation 后成员保存
三项完整 OID、精确绑定 `H_mg` 的 commit-checks success URL、规范 chain 与 inclusion/replay，activation 前
成员保存 `pre_activation` identity 和原因。运行 `check-gate-evidence g4` 对照每个 PR 的
`headRefOid/mergeCommit.oid`；不得只写 Delivery 或泛化的“已合并 / CI 通过”。

## 交付说明

汇报治理类工作时，应包含：

- 改了什么
- 支持哪个闸口或工作流
- 更新了哪些文档或 GitHub 模板
- PR 合并路径（默认 Merge Queue，队列最终 Rebase）与 `H_pr/H_mg/H_main` 证据
- Gate Ledger 当前状态和缺失项
- 还有哪些必须在 GitHub 上手动完成的设置
