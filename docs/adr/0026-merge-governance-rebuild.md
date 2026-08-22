# 0026 合并治理重建：原生审阅 Check、Merge Queue 与 Schema Publication 退役

**状态**: Accepted
**日期**: 2026-08-21
**适用范围**: LaneFlow 合并门禁、External Review Check、commit message 校验、Issue/PR
模板、GitHub Ruleset / Merge Queue、JSON Schema 公共发布义务
**取代**: ADR 0011 的公共发布、永久 URL 与历史保留现行义务
**关联 Issue**: [#468](https://github.com/illusion-tech/laneflow/issues/468)

**关联文档**:

- `../governance/development-gates.md`
- `../governance/github-workflow.md`
- `../governance/documentation-policy.md`
- `../reference/commit-convention.md`
- `../../schemas/README.md`
- `../../.github/trusted-reviewers.json`

## 背景

旧 G3 / G4 与 `xtask` 的 `gate_evidence` 把可变的 GitHub Markdown 评论当成不可变审计
日志，再用自然语言句式、墙钟时序和「评论必须复述 xtask 命令」做 fail-closed 校验。
产品 CI 已绿时，合并仍会被 Shadow 行、反引号、Related 集合和 comment 时序挡住。

LaneFlow 当前未发布 1.0，不存在生产部署，也没有任何外部消费者依赖历史 JSON Schema
或公开 URL。ADR 0011 的永久发布承诺是内部自设约束，不构成实际兼容义务。

项目所有者授权以一次性直接合入 `main` 的方式部署本次治理重构。授权范围仅限治理
基础设施、文档、模板、Ruleset、旧门禁删除，以及 `schemas/` 中历史发布物与 ADR 0011
退役。产品 crate 不在范围内。

## 决策

### 1. 机器不解析自然语言

Issue / PR 正文、勾选框、评论表格、permalink 句式一律不是合并门禁输入。人审只读
GitHub Reviews API 与 Check Run。正文和评论允许编辑；不把评论当审计日志。

### 2. 用 commit OID 绑定身份

人审批当前 PR head `H_pr`。CI / CodeQL 批 Merge Queue 的合并组 head `H_mg`。`H_pr`
变了人审作废；`main` 前进只重跑机器检查。墙钟先后、同秒顺序、comment 必须晚于
review 等规则废止。

### 3. 删除 G3 / G4 手续

删除：G3 Owner comment、`check-gate-evidence`、Evidence Gate Shadow、Gate Ledger
勾选与 permalink、自指 cargo 断言、「维护者点头」第二钥匙、G4
`merge-queue-g4-evidence:v1`。不删除「当前 head 上要有非作者受信审阅」和
「CI / CodeQL 必须绿才能进 main」。

合入事实以 GitHub `mergeCommit` 与 merge group `head_sha` 为准。

### 4. 人流程保留在模板，零解析

Issue 模板保留目标、非目标、验收。高影响变更（设计 / Core / 数据规范 / 适配器）
可填相关 ADR 和「实现前是否需要先冻结设计」。侧边栏元数据从 GitHub 读取，禁止再
抄进 body。Project 列：`Backlog` / `Ready` / `In Progress` / `In Review` / `Done`。
Done = PR 已合且 Issue 已关。

默认一个 Issue 一个完成它的 PR，body 使用 `Closes #n`。仓库打开
`Auto-close issues with merged linked pull requests`。commit footer 继续 `Refs:`。
删除 Delivery / Related 双轨和 `g3-full-set-recovery`。

G0 / G1 / G2 仍作为人可读的立项、设计冻结、开工意图，不进 xtask。

### 5. Required checks 冻结为六个同名

切换完成后，PR 与 `merge_group` 阶段必须使用相同名称：

1. `Commit message`
2. `Rust checks`
3. `Dependency policy`
4. `Analyze (actions)`
5. `Analyze (rust)`
6. `External Review`

删除 `Governance checks`。不新增 `Schema publication` required check。
`Markdown tables` 继续跑 `format-md-tables`，只警告，不阻断合并。

`Commit message` 只检查 Conventional Commits 标题、footer 的 `Refs: #<n>` 或
`Closes: #<n>`（及已有的 `Refs: pending, <reason>`）、以及标题带 `!` 时的
`BREAKING CHANGE:`。不检查 `Gate` / `Slice` / `Impact` / `Scope` / `Validation` /
`Docs`。历史提交上残留的旧 G3 字段忽略。

### 6. External Review 只认原生 PullRequestReview

Check 名固定为 `External Review`。

通过条件：当前 `H_pr` 上存在至少一条非作者、列入 `.github/trusted-reviewers.json`
的已提交原生 `PullRequestReview`，且其 `commit_id` 等于当前 head，状态为
`APPROVED` 或 `COMMENTED`。每位 reviewer 只看其最新一条已提交 review。
`CHANGES_REQUESTED`、`DISMISSED`、`PENDING`、旧 head、作者自审均不算。

普通 Issue / PR 评论一律不算。无法生成原生 Review 的机器人不得列入受信名单。
初始名单包含 `wangzishi` 与 `copilot-pull-request-reviewer`。后继已把
`chatgpt-codex-connector`（ChatGPT Codex Connector）与 `kody-ai`（Kodus Kody AI）
列入同一 JSON；二者均对 PR 提交原生 `PullRequestReview`。名单仍不把普通
Issue/PR 评论算作证据。`qodo-code-review` 是另一个产品，未列入。

未解决对话由 Ruleset `required_review_thread_resolution: true` 拦截。Check 不
维护 thread 是否已解决，也不再使用 `thread-state-changed` marker。

### 7. Merge Queue 盖章 `H_mg`

人审只针对 `H_pr`，不在 `H_mg` 上重做人审。workflow 监听 `merge_group` /
`checks_requested`。入队后在 `H_mg` 上发布同名 `External Review`：确认组内 PR
的 live `headRefOid` 仍等于排队时的 `H_pr`（`gh-readonly-queue` 引用中的 40 位
OID 若存在则必须一致），且该 `H_pr` 仍满足第 6 节，然后把结论盖章到 `H_mg`。
当前队列为单项 REBASE / ALLGREEN；若以后改成多 PR 一组，必须对组内每个 PR 的
`H_pr` 都成立。

### 8. 本次由 GitHub Actions trusted-ref 发布，不上独立 App

workflow 从 default branch 运行：`pull_request_target`、`merge_group`、
`workflow_dispatch`，以及仅作唤醒的空权限 `pull_request_review` signal。
checkout `refs/heads/main`，不执行 PR head。权限：`contents: read`、
`pull-requests: read`、`checks: write`。`contents: read` 用于读取默认分支上的
`.github/trusted-reviewers.json`。

expected source 暂为 GitHub Actions。存在同名 Check 伪造的残余风险，尤其
`merge_group` 的 workflow YAML 来自 `H_mg`（含当前 PR）。若以后接受外部贡献或
要把 Check 做成不可伪造，另开独立 App Issue。删除 R1 Shadow telemetry，不保留
「非 required 的假门禁」。

### 9. 启用顺序（防死锁）

1. 本 ADR 对应的切换提交进入 `main`。该提交不受旧 G3 / G4 约束，也不受新规则
   回溯。
2. 所有者立即把 Ruleset required checks 从 `Governance checks` 换成
   `Commit message`，保留 `Rust checks`、`Dependency policy`、`Analyze (actions)`、
   `Analyze (rust)`；**此时还不把 `External Review` 设为 required**；**bypass 仍保留**；
   打开 `required_review_thread_resolution`。
3. 用真实未合并 PR（例如 #278）走 Merge Queue，确认 `H_pr` 与 `H_mg` 上都出现
   同名 `External Review`。
4. **仅当第 3 步通过后**：把 `External Review` 加入 required，并同时移除 bypass。
   禁止把「设为 required」和「撤 bypass」做在 workflow 尚未验证之前。
5. 此后禁止日常 `--admin` / bypass。

切换时尚未合并的 PR（含 #278 与草稿 #261）留在原号、当时的 current head 上执行
新规则，不必关闭重开。旧 G3 评论可留作历史，机器不再读。

### 10. Schema Publication 退役

- 删除 `xtask` 的 `check-schema-publication-contract`、`build-schema-publication`。
- 删除 `schemas/publication.json`、schema Pages / monitor workflow。
- 删除仅用于历史发布的 Traffic schema `laneflow-data-v0.2` … `v0.9`。
- 暂时保留当前内部 loader/test 输入：`laneflow-data-v0.10`、
  `laneflow-spatial-v0.1`、`laneflow-scenario-manifest-v0.1` 与
  `schemas/road-editing/`，在 #294 删除旧 JSON 生产路径时一并删除。
- ADR 0011 被本文取代。历史 closure review 按当时语义保留，不回写、不当现行义务。
- 所有者停止 Schema Pages 发布；若 Pages 仅服务该功能则关闭对应仓库设置。

### 11. 不追补旧证据

已合并 PR / 已关闭或仍打开的历史 Issue 不必补 G3/G4 档案。#400、#406、#460、
#230、#319 关闭为被 #468 取代。

## 后果

- 合并门禁 = 受信原生审阅 Check + 五个机器 Check + Merge Queue + 未解决对话由
  GitHub 原生拦截。
- xtask 删除 `gate_evidence` 与 schema publication，External Review 按本 ADR 重写。
- 独立 GitHub App 是后续加固，不是本次切换前置。
- 所有者必须按第 9 节完成仓库设置；这些设置不进 git。

## 所有者当天操作清单

GitHub 设置不进 git，切换当天由所有者执行：

1. 以 bypass 或直推把本切换提交送入 `main`。
2. Ruleset：删除 required `Governance checks`，新增 required `Commit message`；
   暂不 required `External Review`；保留 bypass；
   `required_review_thread_resolution: true`。
3. 打开 linked PR 合并后自动关闭 Issue。
4. 停止 Schema Pages；若 Pages 仅此用途则关闭 Pages。
5. 真实队列 canary 通过后：required `External Review`，并移除 bypass。
