# GitHub 工作流

**文档状态**: Active  
**最后更新**: 2026-08-07
**适用范围**: LaneFlow 的 Issue、PR、Project、Milestone、Release 和 CI 治理

## 1. 工作流原则

LaneFlow 采用 GitHub-first 治理：

- Issue 是任务入口。
- Pull Request 是合并审查单元。
- Project 是当前进度看板。
- Milestone 是版本目标容器。
- Actions 是自动化质量门禁。
- Releases 是发布事实记录。

长期设计、架构决策和规范必须进入仓库文档，不应只留在 GitHub 页面中。

## 2. Issue 规则

所有可执行开发任务应先有 Issue。

Issue 应至少说明：

- 背景
- 目标
- 非目标
- 验收标准
- 影响范围
- 关联文档
- GitHub 元数据 / 依赖关系审计：Project、Project status、Milestone、Labels、Parent / sub-issues、Blocked by、Blocking、Delivery PR、Related PRs
- Gate Ledger：G0/G1/G2 在 Issue 阶段增量记录；G3 的权威证据在 PR comment，G4 的权威证据在 Issue comment，body 只保存对应 permalink 索引

Issue 创建或接手时必须审计 GitHub 侧边栏和关系字段，而不是只读取 Issue 正文。若 Milestone、Parent / sub-issues、Blocked by 或 Blocking 暂不适用，必须在 Issue 中写明 `N/A` 原因。Delivery PR 若尚未创建但预计需要 PR，应记录为 `pending`；创建后必须记录唯一 `PR-number`（例如 `#27`），并在 G3 前确认其 `closingIssuesReferences` 覆盖目标 Issue。Related PRs 必须逐条列出；没有时写 `N/A` 原因。仅当 Issue 确实不通过 PR 交付时，Delivery PR 才可记录为 `N/A` 并说明原因。缺少必需元数据且没有显式例外、Delivery PR 或 Related PRs 记录不完整、G3 前 Delivery PR 缺少 `closingIssuesReferences` 关联、Related PR 误用 closing keyword 且没有显式例外，或不适用项没有 `N/A` 原因时，不得推进到下一 Gate。

推荐 Issue 类型（与 `.github/ISSUE_TEMPLATE/` 对应）：

- `功能`（Feature）：新增能力
- `缺陷`（Bug）：缺陷修复
- `设计`（Design）：设计收口或架构决策准备
- `Core`：LaneFlow Core 运行时变更
- `数据规范`（Data Spec）：数据格式、schema 或序列化变更
- `适配器`（Adapter）：引擎适配层变更
- `文档`（Docs）：文档与治理变更
- `调研`（Research）：尚未确定是否实现的探索

仓库默认关闭 blank issue。若必须通过非模板方式记录紧急事项，接手者必须在推进 G0 前补齐模板中的 GitHub 元数据 / 依赖关系审计和 Gate Ledger。

## 3. Project 规则

GitHub Project 用于管理当前状态。推荐列：

- `Backlog`
- `Ready`
- `In Progress`
- `In Review`
- `Blocked`
- `Done`

状态与 Gate 对应关系：

- `Backlog`：尚未通过 G0，或只记录候选想法。
- `Ready`：G0 已记录，GitHub 元数据 / 依赖关系审计已完成；需要 G1 的任务已完成 G1，不需要 G1 的任务已记录不适用原因。
- `In Progress`：G2 已记录，GitHub 元数据 / 依赖关系已复核，任务已经进入实现或文档修改。
- `In Review`：已有 PR 或审查材料，Delivery PR 已记录为 `PR-number`，其 `closingIssuesReferences` 已覆盖目标 Issue，Related PRs 已记录；若不适用或无法使用 closing keyword 建立机器可查关联，必须说明原因，G3 判断应在 PR comment 中维护。
- `Blocked`：当前 Gate 被阻断，必须记录阻断原因、风险和恢复条件。
- `Done`：G4 已完成，Issue 和 PR 的收口证据完整。

状态定义：

- `Backlog`：想法或候选任务，尚未准备开工。
- `Ready`：范围、验收标准和输入文档已经清楚。
- `In Progress`：正在实现。
- `In Review`：已有 PR 或审查材料。
- `Blocked`：设计、依赖、技术验证或权限问题阻断。
- `Done`：已完成 G4 收口。

## 4. Milestone 规则

Milestone 用于表达版本边界，而不是单个大任务。

推荐初始 Milestone：

- `v0.1 Core Prototype`
- `v0.2 Lane Graph + Route`
- `v0.3 Vehicle Following`
- `v0.4 Signals`
- `v0.5 Parking`
- `v0.6 Numeric & Spatial Foundation`
- `v0.7 Bevy Reference Adapter`
- `v0.8 Signalized Corridor MVP`
- `v0.9 Complete Signalized Corridor Example`
- `v1.0 Scope TBD`

每个 Milestone 应有明确的完成定义，并由一组 Issue 组成。

## 5. 分支规则

推荐分支命名：

- `feature/<issue-id>-<short-name>`
- `fix/<issue-id>-<short-name>`
- `docs/<issue-id>-<short-name>`
- `design/<issue-id>-<short-name>`
- `adapter/<issue-id>-<engine-or-topic>`

`main` 应保持可发布或至少可演示状态。所有非平凡变更应通过 PR 合入。

## 6. PR 规则

每个 PR 应：

- 关联一个或多个 Issue。
- 明确本次变更范围。
- 明确本次不做范围。
- 说明是否影响 Core API。
- 说明是否影响数据格式。
- 说明是否影响 Adapter 协议。
- 复核关联 Issue 的 Project、Project status、Milestone、Labels、Parent / sub-issues、Blocked by、Blocking、Delivery PR 和 Related PRs 关联状态。
- 记录测试、构建和文档检查结果。
- 记录已知风险和例外。
- 在 PR comment 记录 `## G3 合并判断`：current head、rollout phase、checks、External Review Gate、G3 Evidence Gate Shadow、审阅、验证、风险、例外、合并方式和 Gate 断言；PR body 的 G3 checkbox 回链当前 comment，Issue body 的 G3 Gate Ledger 对 Related PR 增量回链并保持未勾选，直到 Delivery PR 与全部 Related PR 均完成。
- 在标准 G3 前取得至少一个受信任 external reviewer 的 exact-head completion；finding 必须处置并由新的 exact-head clean re-review 封口。

不得用父任务标题合入只覆盖部分能力的实现。部分交付必须明确子切片边界。

分支不是长期 Development 关系证据。PR 创建后，应在 Issue 的 Delivery PR 或 Related PRs 字段记录 `PR-number`。唯一的 Delivery PR 通过 PR body 的 GitHub closing keyword 建立 Development 关联；Related PR 使用 `Refs: #<issue>`。若 Delivery PR 无法关联，必须在 PR 中说明原因并保留可追踪链接。

Delivery PR / Related PRs 关联规则：

- 仓库设置 `Auto-close issues with merged linked pull requests` 应保持关闭；Issue 关闭仍由 G4 清场手动完成。
- 当 PR 预期覆盖关联 Issue 的完成边界时，它是唯一 Delivery PR，body 应使用 `Closes #<issue>`、`Resolves #<issue>` 或等价 GitHub closing keyword 建立 Development 关联。
- 当 PR 只是父 Issue 的子切片或部分交付时，它是 Related PR，不得误用 closing keyword；应使用 `Refs: #<issue>`，并在 Issue 中列出该 PR。
- commit message footer 与 PR body 语义分开：commit message 通常继续使用 `Refs: #<issue>`，不得为了建立 Development 关联而把提交 footer 改成 `Closes`。
- G3 前默认必须通过 `gh pr view <delivery-pr> --json closingIssuesReferences` 确认 Delivery PR 的完整 closing set 与全部 `关联 Issue` 精确一致；Related PR 的 closing set 必须为空。GitHub Development 面板只作为人工辅助证据。每个 Related PR 都用 `check-gate-evidence g3 --repo <owner/repo> --issue <number> --related-pr <current-related-pr>` 独立验证并永久保留该 Related-only 断言；Delivery PR 使用 `--delivery-pr <number>` 并传入 Issue 已记录的全部 `--related-pr` 做整组复核，逐个读取历史 Related-only comment 而不改写它们。G3 comment 的 `Gate 断言` 行必须包含与实际调用参数完全一致的反引号命令，并在命令后写 `已通过`；关联多个 Issue 时为每个 Issue 分别写一条精确命令。pending、缺少结果、重复命令或参数不匹配均不能进入 `G3 = Pass`。若 Delivery PR、父 Issue 子切片、权限或平台限制导致只能手动关联 Development 面板，必须记录显式例外，说明原因、风险、后续收口方式和 Cleanup owner；否则不能进入 `G3 = Pass`。

### 外部审阅与复审

外部审阅的完整状态机和 Gate 规则以 `development-gates.md` 第 6 节为准。GitHub 工作流执行时：

- PR author 可以做 self-review 并担任 G3 Owner，但不能把自己的 review 计入外部 reviewer 数量。
- Copilot、Codex Connector 与人工 reviewer 通过 provider adapter 归一化；只信任 allowlist actor 和可追溯 GitHub event，不信任作者转贴文本。
- review request、reaction 和任务启动只表示 pending。受信任 Codex provider 的无 reviewed SHA clean 摘要本身不计为 completion，并形成失败关闭的时序歧义；只有严格晚于它且绑定 current exact head 的字段完整有效 clean completion 才能覆盖该歧义。
- `reviewThreads=0` 只表示当前没有 unresolved thread；没有有效 completion 时仍为未审阅。
- finding 被作者 resolve 后进入 `AwaitingRereview`，必须重新请求 reviewer 并取得 current-head clean completion。
- new push 或 review dismissal 使旧 completion、Check 与 G3 comment stale。G3 comment 必须新增，不得编辑旧评论回填。
- 其他贡献者创建 PR 时，`wangzishi` 的 exact-head `APPROVED` 可以计数；`wangzishi` 自己创建 PR 时，由受信任 AI / GitHub actor 提供外部审阅。
- 外部 provider 无法审阅时不自动通过。只有 trusted-ref validator 同时证明 Dependabot App / bot PR author、同 repository 的 `dependabot/cargo/*` ref、唯一 current-head GitHub `web-flow` verified Dependabot commit、完整 Dependabot force-push provenance、完整且唯一 `MODIFIED Cargo.lock`，并确认错误 PR commit/object SHA finding 与 G3 Owner `Disposition:` 均未编辑、处置时间严格更晚且线程已解决，才能由 `dependabot_lockfile_policy` 形成机器替代 completion；其他路径继续要求 exact-head reviewer 或结构化 waiver。

PR commit 使用 `Gate: G3 Candidate`，只说明实现者验证完成并准备进入 review。正式 G3 结果不写回 commit。

### External Review Gate workflow 安全

steady-state Gate 必须由 default branch 或其他 trusted ref 上的 validator 执行，并把结果发布到通过 API 二次确认的 current head：

- metadata-only；禁止 checkout 或执行 PR head，禁止执行评论正文或把未信任字段直接插入 shell。
- 最小权限只包含读取 repository/PR/Issue/Checks 与发布目标 Check Run 所需权限；不读取 repository secret。
- 监听 head、review、review comment 和 conversation comment 变化，按 PR/head 使用 concurrency 与 external ID；旧 head 的迟到任务不得覆盖新 head。
- Check Run 发布前再次读取 current head；变化时将旧运行标为 stale。
- required check 绑定预期 GitHub App/source，避免同名 status 冒充。
- workflow/action 依赖继续 pin 完整 commit SHA。修改 Gate workflow 的 PR 由 `main` 上的旧 validator 判断，不能用候选实现自批。

`check-external-review` 负责输出稳定 JSON 和人类可读摘要，状态固定为 `pass`、`awaiting_review`、`review_pending`、`findings_open`、`awaiting_rereview`、`stale`、`provider_error`、`waived`。只有 `pass` 对应标准 Check success；歧义、API/provider 错误和 head 竞态 fail closed。

live metadata 校验使用：

```text
cargo +1.96.0 run --locked -p xtask -- check-external-review --repo <owner/repo> --pr <number> --format json
```

CodeQL current-head 适用性独立校验使用：

```text
cargo +1.96.0 run --locked -p xtask -- check-codeql --repo <owner/repo> --pr <number> --format json
```

只有 PR-bound rollup 选定且 REST source App=`github-advanced-security` 的 aggregate success 所得 `pass`，以及精确 `dependabot-cargo-lock-only-v1` 对 aggregate `NEUTRAL` / no-analysis 的 `not_applicable` 可进入标准 G3；`SKIPPED` 失败关闭。OPEN target 还要求 REST `pull_requests` 精确绑定 PR number/current head/current base；MERGED 历史重放允许 association 被 GitHub 清空，但仍须由 PR-bound URL 与 REST source/head/conclusion 双重匹配。同 head 的其他 PR/base 分析或同名 Actions job 不计入；external-review waiver 不连带豁免 CodeQL；
从 `2026-08-08T00:00:00Z` 创建的 G3 comment 起必须唯一记录 `- CodeQL：` 状态、
policy（若适用）和 evidence URL；状态必须是字段后的首个且唯一 backtick 状态，URL 必须
与机器结果精确相等。fixture/replay 使用 `--input ... --expect <state>`，普通
源码/workflow PR 的 neutral/no-analysis 仍失败关闭。

R0 fixture / 历史事件 replay 使用版本化 snapshot：

```text
cargo +1.96.0 run --locked -p xtask -- check-external-review --input <snapshot.json> --format json --expect <state>
```

snapshot 与结果均使用 `schemaVersion: 1`。live evaluator 读取 author、current head/base、review requests、reviews、issue comments 和 review threads，并在计算后再次读取 head/base；任一 connection 超过 100 条、thread comment 截断、actor/timestamp 缺失、provider 文案歧义或二次读取发生竞态时返回 `provider_error`。review/review comment 的 SHA 缺失同样失败关闭；唯一窄例外是受信任 Codex provider 的无绑定 clean comment 可被创建/提交时间严格更晚、绑定 current exact head 且字段完整有效的 clean completion supersede。仅有无绑定 clean、最终有效 current-head clean 之后又出现无绑定 clean、两者同秒无法证明先后、comment 被编辑，或时间/URL/OID 无效时仍返回 `provider_error`。`--expect` 只用于 fixture/replay 断言；未提供时只有 `pass` 退出成功，`waived` 仍保持独立状态。

`check-gate-evidence g3` 的 external-review 集成以 Issue #230 的 G2-B 增量开工记录时间 `2026-07-24T15:16:21Z` 为迁移边界：更早的 G3 comment 保留 legacy 历史语义，不追溯要求新增字段；该时点及之后创建的 G3 comment 必须显式包含 `Gate 结果`，是未编辑的 append-only 记录，并包含完整 current head。`G3 Pass` / `R0-R1 bootstrap` 必须晚于 live evaluator 识别的最终 completion；`G3 Waived` 必须使用 `development-gates.md` 规定的 `external-review-waiver:v1` 结构化记录，多 Issue PR 按 `followUpIssue` 为每个关联 Issue 保留一条唯一记录，且 record set 与 `关联 Issue` 集合精确一致；live evaluator 逐 Issue 匹配并保持 `waived` 而不冒充 `pass`。waiver 路径只读取并二次确认 PR number、author、draft、current head/base identity，不依赖 provider review connection；GitHub identity API 不可读、head/base 竞态或 Draft PR 仍然 fail closed。OPEN current target 按当前时间判断 waiver 未过期；Delivery full-set / G4 重放 MERGED 历史成员时按其 `mergedAt` 判断当时有效性，不能把历史 waiver 重新用于新的 merge。G3 Evidence Shadow 不把有到期时间的 waiver 发布为 success，避免时钟推进后留下假阳性。

#### R1 shadow workflow 实现边界

R1 的 non-required shadow 由两个 workflow 分离不可信事件与可信发布权限：

- `External Review Signal` 只监听 `pull_request_review` 与 `pull_request_review_comment`，使用空权限，不 checkout、不读取 artifact、不调用 API，也不执行 PR 内容。它的完成事件只作为唤醒信号，不能携带审阅结论。
- `External Review Gate Shadow` 只从 default branch 运行：直接监听 `pull_request_target`、PR conversation comment、signal 的 `workflow_run` 和手工 dispatch。它显式 checkout `refs/heads/main`，关闭 credential persistence，并只授予 `contents: read`、`pull-requests: read`、`issues: read` 与 `checks: write`。
- trusted `workflow_run` 优先读取 GitHub payload 的 PR number；若 `workflow_run.pull_requests` 为空，则校验完整 `head_sha`，调用 commit-to-PR association API，并只保留 open、targeting `main` 的 PR。关联 API 失败时 workflow 失败，不能把缺失 target 静默当作无变化；fallback 不解析 branch name、run title、artifact 或 PR 提供的输出。
- GitHub Actions 不提供 `pull_request_review_thread` workflow trigger。R1 中每批 resolve / unresolve 后，操作者必须新增一条顶层 PR comment，正文精确为 `external-review: thread-state-changed`；trusted `issue_comment` workflow 只接受 `created` 且正文精确匹配该 marker 的 PR comment，把 PR number 当信号并通过 API 重读全部 thread 状态。正文只用于固定值入口判定，不进入 shell、不作为 evaluator 结论；edited / deleted 或其他 PR comment 均不能刷新 shadow Check。缺少 marker 时不得声称 shadow Check 已反映最新 resolution。
- trusted publisher 不读取 signal artifact 或输出，不把 event/comment 字段插入 shell。它只解析有界的数字 PR ID，再通过 GitHub API 重新读取 current identity。
- `publish-external-review-check` 在 evaluator 前后复核 PR number、open/draft、main base、same-repository、head/base OID；只有 identity 稳定时才创建完成态 Check Run。R1 Check 固定名为 `External Review Gate Shadow`，external ID 绑定 repository、PR、head、trusted validator OID 与 workflow run/attempt，并在 API 响应中复核 source App 为 `github-actions`。
- `pass` 映射为 `success`；`waived` 映射为 `action_required`，确保未来成为 required check 后仍需显式临时 bypass；其他状态均为 `failure`。Check output 保存状态、head/base、provider/actor、completion、finding/thread/re-review 统计与 reference-style evidence；完整 evaluator JSON 留在 trusted workflow log。
- publisher 按 PR 设置 concurrency 并取消旧运行。发布前 identity race 直接失败且不向新 head 写入旧结果；new head 由 `synchronize` 重新计算，漏事件只能显式 manual dispatch 补偿。evaluator result 的显式 state 与固定 FNV-1a fingerprint 会和 PR/head/trusted-ref/run/attempt 共同形成 external ID。publisher 永不查询或复用既有同名 Check，每个 trusted event 都创建并复核自己的 receipt，避免 PR 预造同 source App / external-id prefix 使 trusted run 跳过发布。
- Draft、非 `main` base、非 open 和 fork / cross-repository PR 不属于 R1 eligible sample，不发布 shadow 结论。base repository 的 `GITHUB_TOKEN` 不能保证向 fork head commit 创建可关联的 Check；R2 不为此放宽 required check，贡献者或维护者必须把最终 patchset 迁移为 same-repository PR，并在其 exact head 重新取得 external review。schedule 仍只枚举 targeting `main` 的 open PR，publisher 对其中的 fork 再 fail-safe skip。
- R1 的 `github-actions` source App 只证明 shadow telemetry 由 GitHub Actions 写入，不是防伪身份。同仓 PR 可以定义同名 Actions job；required status check 又不区分 workflow、matrix 或 event，因此 `External Review Gate Shadow` 永远不得加入 ruleset。R1 样本必须同时索引 default-branch trusted workflow run、external ID 和 Check receipt；PR 自定义的同名 Check 不计样本。

该 workflow 合入 `main` 前不能用于 Related PR C 自身的 G3；PR C 仍由已合入 main 的 validator、current-head external review 与当前 ruleset 完成 R0 bootstrap。合入后先追加 R1 起点记录，再通过手工 dispatch 和真实事件验证首次 Check；未完成该记录与首次 live 验证前不得开始计算 14 天 / 10 eligible PR 退出门槛。

#### G3 evidence shadow 与强制边界

`G3 Evidence Gate Shadow` 复用相同的 trusted-ref 安全边界，但独立评估 Gate Ledger 闭环：

- `pull_request_target`、PR conversation comment、external-review metadata-only signal、精确 thread-state marker、精确 `g3-evidence: changed` 顶层 PR comment 或显式 manual dispatch 只提供有界 PR number；Issue body 变化只提供受信任 event file，resolver 把其中的 body 当数据解析，不送入 shell。`pull_request_target` 不使用 base branch 事件过滤器，确保从 `main` retarget 到其他 base 时仍能发布 failure，main eligibility 由 trusted job 内部判断。workflow checkout `refs/heads/main`，不 checkout / 执行 PR head，也不把 comment body 送入 shell。
- `check-gate-evidence-target --repo <owner/repo> --pr <number>` 从 PR body 解析一个或多个明确 `关联 Issue` 与唯一 `PR 角色`，并逐个 Issue 校验。Delivery 自动读取每个 Issue 的完整 Related PR 集合；Related 对每个 Issue 只执行自身 Related-only G3。Delivery full-set 中每个 current-policy Related PR 还要验证自身 `关联 Issue` / `Related PR` 角色、空 closing set 与全部声明 Issue 的断言集合；状态必须是非 Draft `OPEN` current target，或具有 `mergedAt` 的 `MERGED` 历史证据，`CLOSED` / 状态与合并时间不一致均 fail closed。关联多个 Issue 时，同一 append-only G3 comment 必须为每个 Issue 分别包含一条精确断言，且完整命令集合与解析 target 精确相等。具体编号字段只接受 `#<number>` 列表或该角色允许的完整 `pending` / 带原因 `N/A` 状态，不接受互斥模板选项残留。关联 Issue 必须保持 `OPEN`；远端 metadata、permalink、append-only comment、current-head external review 或角色关系任一缺失都失败关闭；G3 API 查询省略只由 G4 使用的 `projectItems`。
- resolver 除直接事件目标外，还从 open Issue 的 `Related PRs` 反向发现受影响 Delivery PR；Related push/body/review/thread 变化必须让该 Delivery 的旧 success 被新的 non-success 或重新验证结果取代。Issue event resolver 对变更前后 body 分别执行精确元数据解析并取 PR 并集，因而删除关联也会刷新旧目标；完全无治理字段的无关 Issue 返回空集，字段不完整、互斥模板残留或其他解析歧义才保守刷新全部 open main PR。
- 直接 PR / comment / review-signal 事件在 resolver 之前按目标 PR 串行，publisher 对级联目标继续按 PR 串行。发布 success 前完整重跑 target 与 marker validator，并再次读取 open/draft/base/head repository/head/base OID；marker freshness 除 body 外还分页读取关联 Issue / PR timeline：关联 Issue 的 close/reopen，以及 full-set 全部 PR 的 comment（含 edit 时间）、review、commit、close/reopen、Draft/ready、review request 与 head/base 生命周期活动都必须早于 marker。更早但延迟的 marker run 在后发 Related 状态往返或活动后只能失败。identity race 在原始已评估 head 发布 failure，不留下旧 success。same-repository 的 Draft、closed 或其他不再 eligible 目标发布 non-success 以撤销同一 head 上的旧 success；fork / cross-repository 仍不发布。source App=`github-actions`，success / failure 只表示本次 trusted replay 结果。
- 新增或编辑 G3 comment、PR body 或 Issue body 后，在全部 permalink 就绪时新增精确 marker；只有新建、未编辑且正文精确为 `g3-evidence: changed`、并严格晚于 current G3 comment、当前 PR 与全部关联 Issue body 最后编辑时间及上述 timeline 活动的直接目标事件可以发布 success；full-set 任一 current-policy PR 为 `G3 Waived` 时仍只能 non-success，同秒无法证明顺序或 timeline 分页 / 时间缺失时失败关闭。G3 comment 的 `G3 Evidence Gate Shadow` 还必须是唯一、非空的规范值：`Check URL：https://github.com/...`、`R1 non-required：<原因>` 或 `候选 workflow bootstrap：<边界>`。其他 conversation comment、marker edit/delete、review/thread/PR/Issue metadata/manual dispatch 与级联 Delivery 刷新只能发布 non-success，防止旧 marker 被重用；Shadow success 不能替代当前 G3 comment，也不能替代逐 Issue `check-gate-evidence g3` 的显式 Gate 断言。
- 候选 validator / workflow 在合入 `main` 前不能自批。标准 G3 对 closed Issue、Draft 或已 merged / closed 的当前 PR 目标失败；target 模式确认角色参数稳定后必须重跑完整远端证据；G4 保留对合并前证据的历史复核。

R1 不修改 ruleset，`G3 Evidence Gate Shadow` 永远不能被描述为平台级 merge blocker。R2 只接受两条强制来源：

1. 当前 organization plan 与 API 支持时，在 organization ruleset 中配置 [required workflow](https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/managing-rulesets/available-rules-for-rulesets#require-workflows-to-pass-before-merging)，并锁定 trusted source workflow；
2. 否则由独立、最小权限 GitHub App 发布正式 `G3 Evidence Gate` Check，repository ruleset 同时绑定 Check name 与 expected source App。

启用任一路径前必须用 API 保存 ruleset before snapshot，验证 source identity，并让恶意 PR 自定义同名 Actions job 的 canary 保持不可合并；随后重触发全部 open PR，确认 failure 真实阻断而 success 无需 `--admin`，再保存 after snapshot。organization ruleset API 返回 plan / permission `403` 时 R2 保持 blocked，不能退化为同名 required status check。现有 standing `always` bypass 必须随 cutover 移除；紧急例外仅使用有期限的 break-glass 记录。GitHub 关于 organization ruleset 创建与权限边界的说明见 [Creating rulesets for repositories in your organization](https://docs.github.com/en/organizations/managing-organization-settings/creating-rulesets-for-repositories-in-your-organization)。

### Rollout 与 ruleset 迁移

Issue #230 采用 `R0 -> R1 -> R2`：

- R0 交付 governance contract 与 validator。
- R1 由 trusted-ref workflow 发布 non-required shadow Check，至少运行 14 天并覆盖 10 个 eligible PR；退出指标见 `development-gates.md`。
- R2 激活前必须注册并仅在 base repository 安装专用 External Review Gate GitHub App；其短期 installation token 只能提供给 trusted publisher。App repository permissions 固定为 `Checks: read and write`（创建 / 复核 Check Run）、`Commit statuses: read and write`（允许 ruleset 选择 expected source）与 `Pull requests: read`（接收 review-thread webhook），不授予 Contents、Actions 或 Issues。App 必须订阅 `pull_request_review_thread` webhook 并让 trusted publisher 重读当前 PR；PR D canary 必须实测 resolved 与 unresolve 两个方向。若平台只交付文档化的 `resolved` action，unresolve 仍需 marker 或另一条自动信号，且在该缺口关闭前不得进入 R2。PR D 把正式 Check 政名为 `External Review Gate`，验证 API 返回该专用 App 的 slug / integration ID，并在 ruleset 中同时绑定 Check 名和 expected source App。缺少该独立身份、完整 thread signal、仍由 `github-actions` 发布或 spoof canary 能满足 required check 时，R2 一律阻断。
- R2 达标且专用 App canary 通过后，把正式 `External Review Gate` 加入 required status checks，启用 conversation resolution，保持 native required approvals 为 0，移除 `update` restriction 和 `wangzishi: always` bypass。

ruleset 变更前后必须保存完整 JSON snapshot 并做字段级对比；随后用低风险 canary PR 验证全部 Gate 通过时 `gh pr merge <number> --rebase` 无需 `--admin`。R2 激活时重新评估全部 open PR，不进行无证据 grandfathering。

紧急路径不保留 standing bypass。仅在 `development-gates.md` 允许的四类事件中临时授权，记录 `G3 Waived`、风险、到期、follow-up 和 Cleanup owner，并默认在 24 小时内撤回。

### Copilot repository instructions

仓库可通过 `.github/copilot-instructions.md` 给 Copilot on GitHub 提供仓库级自定义说明。该文件只作为提示层使用，必须保持薄包装，优先转读 `AGENTS.md`、`.agents/` 和 `docs/governance/` 中的事实源，不应复制完整长期规则。

使用边界：

- Copilot instructions 不能替代 CI、`gh` / GraphQL 元数据复核、review threads 状态检查或 Gate Ledger。
- Copilot review 不能作为 Project status、Labels、Milestone、Parent / sub-issues、Blocked by、Blocking 或 `closingIssuesReferences` 的事实源。
- 修改 `.github/copilot-instructions.md` 的 PR 不应假定本轮 review 已使用新说明；对 PR review 的稳定影响以合入 `main` 后的 base branch 内容为准。

## 7. PR 合并策略

LaneFlow 默认使用 **Rebase and merge** 将 PR 合入 `main`。

原因：

- 保持 `main` 历史线性、清晰。
- 保留 PR 内各 commit 的治理说明（`Gate`、`Slice`、`Impact` 等）。
- 避免为常规功能 PR 增加多余的 merge commit 节点。

默认规则：

- 常规功能、修复、文档、治理 PR → **Rebase and merge**。
- PR 内 commit 已具备独立意义且 message 符合 `docs/reference/commit-convention.md` → **Rebase and merge**。

PR commit message 应使用 Conventional Commits 标题，并在正文保留 LaneFlow 治理字段：

- `Gate`
- `Slice`
- `Impact`
- `Scope`
- `Validation`
- `Docs`
- `Refs` 或 `Closes`

CI 会校验 PR commit 标题和必需治理字段。若确需例外，必须在 PR 中说明原因，并按 `development-gates.md` 的例外治理规则记录。

新提交的 `Gate` 使用 `G3 Candidate`；`G3 Pass` 与 `G3 Waived` 只属于 PR Check / comment 证据层。迁移 cutoff 和历史 commit 兼容规则见 `docs/reference/commit-convention.md`。

例外（须在 PR 或 Issue 中说明原因）：

- **Squash and merge**：PR 内含多个无独立意义的 wip commit，或明确要求 `main` 上 1 个 PR 对应 1 个 commit。
- **Create a merge commit**：发布分支、长期分支合流等需要保留 merge 节点的场景；以及 `docs/design/current-package-import.md` 第 13.2 节已冻结的 #297 资产审计证据 PR：`A..E` 仅新增固定报告路径的提交必须原样进入 `main`，使报告 `source.commit` 与 G3 记录的提交身份在标准 clone 中可重放。

命令示例：

```powershell
gh pr merge <number> --rebase
```

仓库设置建议：在 GitHub 仓库 Settings → General → Pull Requests 中启用 **Allow rebase merging**，并按团队习惯禁用或保留 squash / merge commit。

## 8. CI 规则

CI 的初始目标是保证基础质量，不追求一次到位。

当前最小检查：

- 仓库中关键治理文档文件存在。
- Markdown 文件非空。
- PR / push commit message 符合 `docs/reference/commit-convention.md`。
- Rust workspace 格式检查通过：`cargo fmt --all -- --check`。
- Rust workspace 测试通过：`cargo test --workspace --locked`。
- Rust 依赖政策通过：`Dependency policy` required check 中的 cargo-deny advisories、licenses、bans 和 sources 检查成功。

外部审阅检查按 #230 rollout 逐步加入：R1 以 non-required shadow 运行；R2 达标后 `External Review Gate` 成为 required check。当前阶段不得把“workflow 文件已存在”写成 ruleset 已启用。

GitHub CodeQL、Secret Scanning 和 Dependabot 属于平台安全检查，其配置、状态语义和阻断规则见 `security-scanning.md`。GitHub 为当前 PR 产生的适用 CodeQL check 必须在 G3 前完成；唯一长期 `not_applicable` 是 `check-codeql` 机器证明的 `dependabot-cargo-lock-only-v1`。其他缺失预期分析、失败或平台不可用不能解释为通过。

后续根据实际技术栈继续增加 Markdown/YAML 语法检查、lint、schema validation、adapter build 和 example smoke test。新增 data spec、Adapter 或示例代码后，应同步增加对应专用门禁。

## 9. Release 规则

每次 Release 应说明：

- 版本目标
- 新增能力
- 修复内容
- breaking changes
- Core API 版本
- 数据格式版本
- Adapter 兼容情况
- 示例项目状态

Release 说明可以引用 `docs/roadmap.md` 和相关 ADR。

公开发布或对外分发前还必须按 `security-scanning.md` 重新验证 Code Scanning、Secret Scanning 和 Dependabot，并按 `dependency-security.md` 复核源代码许可证、Cargo metadata、cargo-deny 和分发物 attribution；历史零告警不能替代本次发布审计。

## 10. 合并后 G4 清场流程

PR 合并后，应回到关联 Issue 完成 G4，而不是在清场时首次补写 G0-G3。

G4 清场必须完成：

- 确认 PR 已按默认策略 Rebase and merge 合入，或记录例外原因。
- 勾选 Issue 验收 checklist。
- 在 Issue comment 发表 `## G4 完成判断`，并在 Issue Gate Ledger 勾选 G4、回链该 comment；Delivery PR body 只回链该 Issue G4 comment。
- 将 Project 中关联 Issue 和 PR 移动到 `Done`。
- 确认 Delivery PR、Related PRs、Parent / sub-issues、Blocked by、Blocking 已收口；Issue G4 comment 的 `Gate 断言` 必须使用与实际调用参数完全一致的规范命令并明确写 `已通过`，运行 `check-gate-evidence g4` 成功后才可关闭 Issue。无法收口的剩余关系必须拆出后续 Issue，并记录原因、风险和 Cleanup owner。
- 手动关闭关联 Issue；不得依赖 GitHub 自动关闭 Issue 跳过验收 checklist、G4 记录、Project `Done` 和分支清理。
- 删除远端 PR 分支并 prune 本地 remote-tracking 分支。
- 切回并更新本地 `main`。
- 撤回临时 ruleset bypass、admin override 或其他临时权限；若不能撤回，记录保留原因、风险和 Cleanup owner。

若 Delivery 合并后才发现验收缺口并创建 late Related PR，历史 Delivery G3 既不能编辑，
也不能在 merge 后补写。此时只能按
[`development-gates.md` 的 G4 recovery](development-gates.md#71-delivery-合并后新增-related-pr-的-g4-recovery)
在新的 append-only G4 comment 记录 `g3-full-set-recovery:v1`；校验器必须证明 late
Related PR 确实在 Delivery merge 后创建、逐个 Related-only G3 已通过、最终集合和
证据回链完全一致。该路径不用于普通 G3，也不替代事前冻结完整 PR 集合。

如果 G4 阶段发现 G0-G3 没有按时记录，只能追加“补救记录”，并说明流程遗漏原因。补救记录不能作为后续任务的标准流程。
