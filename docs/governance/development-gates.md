# 开发闸口

**文档状态**: Active  
**最后更新**: 2026-08-20

**适用范围**: LaneFlow 的需求、设计、实现、评审与完成治理

## 1. 目标

本文定义 LaneFlow 的轻量开发闸口，避免 Core、数据格式、Adapter 和示例在没有统一输入的情况下各自漂移。

LaneFlow 采用五个闸口：

- `G0`：立项
- `G1`：设计冻结
- `G2`：开工
- `G3`：合并
- `G4`：完成

## 2. 切片类型

每个 Issue 或 PR 应选择最接近的切片类型：

- `docs-only`：只改文档。
- `governance`：流程、模板、CI、项目治理。
- `core-runtime`：LaneFlow Core 运行时逻辑。
- `data-spec`：lane graph、route、signal、parking 等数据格式。
- `adapter`：Unity、Unreal、Godot、O3DE、Web 等引擎适配。
- `authoring-tool`：道路、路线、停车位等编辑或转换工具。
- `example`：示例项目、示例场景或演示数据。
- `cross-layer`：同时影响 Core、数据格式、Adapter 或示例的高风险变更。

## Gate Ledger 增量记录

Gate Ledger 是 Issue 和 PR 上的增量闸口记录，用来说明任务何时通过了 G0-G4。它不是 G4 清场时的一次性补档。

通用规则：

- 每次任务跨过一个 Gate，都应在对应载体留下记录。
- G0、G1、G2 在 Issue Gate Ledger 中增量记录。
- G3 的完整事件证据记录在每个 PR 的 `## G3 合并判断` comment，且必须在该 PR 合并前创建；PR body 的 G3 checkbox 只保存当前 PR 的直接 comment permalink。Issue body 的 G3 Gate Ledger 按 Related PR 合入顺序增量追加各自 permalink，在 Delivery PR 与全部 Related PR 均完成前保持未勾选，最终再勾选并保存完整 permalink 索引。
- PR / Issue body 与 comment 中的 GitHub URL 使用文末 reference-style 定义，并在正文与引用定义之间保留空行；Gate validator 同时解析既有 inline permalink 和 reference-style permalink，引用定义存在但 Gate 行未实际引用时不得通过。
- G4 的完整事件证据记录在 Issue 的 `## G4 完成判断` comment，且必须在所有关联 PR 合并后、Issue 关闭前创建；Issue body 的 G4 checkbox 保存直接 comment permalink。Delivery PR 的 body 只回链该 Issue G4 comment，Related PR 不承担 Issue G4。
- GitHub comment 是带时间和作者的过程证据，不是不可变审计日志；长期规则仍由仓库文档和 Git 历史保存。
- 每个 Related PR 独立 G3 都必须运行 `cargo +<workspace-rust-version> run --locked -p xtask -- check-gate-evidence g3 --repo <owner/repo> --issue <number> --related-pr <current-related-pr>`；`<workspace-rust-version>` 由 `xtask` 的 `CARGO_PKG_RUST_VERSION` 生成（当前为 `1.96.0`），不得在生成器中另写常量。该 comment 永久保留 Related-only 断言，只验证当前 Related PR 的 comment、仍未勾选的 Issue G3 增量 permalink 与关系，不声明 Issue 整体 G3 已完成。若 Issue G3 已提前勾选，Related-only 校验必须失败。
- Delivery PR G3、整组关系复核与 G4 使用 `cargo +<workspace-rust-version> run --locked -p xtask -- check-gate-evidence <g3|g4> --repo <owner/repo> --issue <number> --delivery-pr <number> [--related-pr <number>]...`，并传入 Issue 已记录的全部 Related PR。整组复核按各 Related PR 原有的 Related-only 断言验证其 current G3 comment，不要求改写为 full-set 命令；Delivery PR comment 和 Issue 最终断言使用 full-set 命令。gate-command v1 对从 `1.96.0` 到当前 workspace MSRV 的 `1.<minor>.0` stable release 保持历史兼容；未显式纳入策略的补丁版本失败关闭。比较解析后的 phase、repo、Issue、角色与有序 Related 集合，并忽略空白及 Cargo/xtask 独立参数的等价顺序。未知、缺失、超出版本窗或语义重复的参数继续失败关闭。`G3 Pass` / bootstrap 的 `Gate 断言` 明确写 `已通过`；`G3 Exception` 及历史重放明确写 `未通过`，且必须有下文定义的结构化记录，绝不映射为 Pass。一个 PR 关联多个 Issue 时，同一 current G3 comment 必须为每个 Issue 分别保留一条精确断言；historical replay 按当前 Issue 只裁决匹配断言的结果，其他 Issue 仍须保留可解析命令和显式结果，并由各自 G4 记录独立裁决。命令或远端读取失败同样是 Gate 失败。
- 小型 `docs-only` 或 `governance` 任务可以把 G0-G2 合并为一条开工记录，但该记录必须发生在实现或开 PR 之前。
- 如果 G4 阶段才发现 G0-G3 缺失，只能标记为补救记录，并说明流程遗漏原因。
- Agent 不得在缺少当前 Gate 记录时继续推进下一 Gate，除非用户明确接受例外并留下原因、风险和 Cleanup owner。

推荐 Issue Gate Ledger：

```text
- [ ] G0 立项已记录：
- [ ] G1 设计判断已记录：
- [ ] G2 开工判断已记录：
- [ ] G3 合并判断已记录：链接 Delivery PR 的 G3 comment；Related PR 如有均逐条链接
- [ ] G4 完成判断已记录：链接本 Issue 的 G4 comment
```

## GitHub 元数据 / 依赖关系审计

Issue 的 GitHub 元数据和依赖关系是 Gate 判断的一部分，不得只依赖 Issue 正文中的任务描述。

每个可执行 Issue 至少应记录并在推进 Gate 时复核：

- `Project` 与 `Project status`。
- `Milestone`；不适用时必须写明 `N/A` 原因。
- `Labels`。
- `Parent / sub-issues`；不适用时必须写明 `N/A` 原因。
- `Blocked by`；不适用时必须写明 `N/A` 原因。
- `Blocking`；不适用时必须写明 `N/A` 原因。
- `Delivery PR`；PR 创建前可写 `pending`，创建后记录唯一的 `PR-number`，进入 G3 前必须确认其 `closingIssuesReferences` 覆盖目标 Issue；确实无需 PR 时才可写 `N/A` 并说明原因。
- `Related PRs`；列出零到多个部分交付 PR。它们使用 `Refs: #<issue>`，不得以 closing keyword 覆盖目标 Issue；没有时写 `N/A` 原因。

推荐记录格式：

```text
## GitHub 元数据 / 依赖关系审计

- Project：
- Project status：
- Milestone：milestone-name / N/A，原因：
- Labels：
- Parent / sub-issues：issue-links / N/A，原因：
- Blocked by：issue-links / N/A，原因：
- Blocking：issue-links / N/A，原因：
- Delivery PR：pending / PR-number / N/A，原因：
- Related PRs：PR-number, PR-number / N/A，原因：
```

必需元数据缺失且没有显式例外时，不得推进下一 Gate；不适用项缺少 `N/A` 原因时，同样不得推进。若因 GitHub 权限或平台限制无法设置某项必需元数据，必须记录原因、风险、临时接受边界、后续清理 Issue 和 Cleanup owner。

Delivery PR / Related PRs 关联规则：

- 一个 Issue 可以有多个 PR，但只能有一个 Delivery PR。Delivery PR 的 body 使用 `Closes #<issue>` / `Resolves #<issue>` 等 GitHub closing keyword 建立 Development 关联；仓库已关闭 linked PR 合并后自动关闭 Issue，Issue 仍由 G4 手动关闭。
- Related PR 的 body 使用 `Refs: #<issue>`，每个 Related PR 都应独立完成 G3。若没有单一 PR 可以代表完整验收边界，必须拆 Issue 或创建最终集成 Delivery PR，不得让多个部分 PR 同时 closing。
- commit message footer 不承担 Development 面板关联职责；常规 PR commit 仍使用 `Refs: #<issue>`。
- 父 Issue 子切片或部分交付不得误用 closing keyword；若 Delivery PR 无法让 `closingIssuesReferences` 覆盖目标 Issue，只能手动关联 Development 面板，必须在 PR 中记录显式例外原因、风险、后续收口方式和 Cleanup owner。

## 3. G0 立项闸口

目标：确认是否需要进入开发，以及最小交付边界是什么。

必须明确：

- 背景和使用场景
- 本次目标
- 本次明确不做
- 验收标准
- 影响范围
- 是否需要 ADR 或 design 文档
- 是否需要拆分子 Issue
- GitHub 元数据 / 依赖关系审计

通过标准：

- 已有 GitHub Issue。
- 任务边界足够小，可以独立评审。
- 验收标准可验证。
- Project、Project status、Labels 已记录，或已记录显式例外；Milestone、Parent / sub-issues、Blocked by、Blocking 已记录，不适用项已有 `N/A` 原因；Delivery PR 已记录为 `pending`、`PR-number` 或已有 `N/A` 原因，Related PRs 已记录，进入 G3 前还必须补齐 Delivery PR 的 Development 关联检查。
- Issue Gate Ledger 中已有 G0 记录。

## 4. G1 设计冻结闸口

目标：确认实现前的正式输入已经足够稳定。

以下变更必须先通过 G1：

- Core API 新增、删除或破坏性变更。
- 数据格式或 schema 变更。
- Adapter 协议变更。
- 运行时 tick、路线、避让、信号灯、停车系统等核心规则变更。
- 会影响多个引擎适配器的设计。

G1 证据可以是：

- ADR
- `docs/design/` 文档
- Issue 中链接到正式文档的冻结评论

通过标准：

- 设计输入清楚。
- 非目标清楚。
- 兼容性影响清楚。
- 测试要求清楚。
- Issue Gate Ledger 中已有 G1 记录；不需要 G1 时，也应记录不适用原因。

## 5. G2 开工闸口

目标：确认实现者不是边做边猜。

开工前应确认：

- Issue 状态为 `Ready` 或等价状态。
- 已阅读相关 ADR 和 design 文档。
- 已知道本次是否影响 Core、data spec、Adapter 或 example。
- 已知道需要补哪些测试和文档。
- 已复核 GitHub 元数据 / 依赖关系，Project status 与当前 Gate 一致。
- Issue Gate Ledger 中已有 G2 记录。

如果实现中发现设计输入不稳定，应暂停扩展实现，回到 G1 补设计或拆子切片。

## 6. G3 合并闸口

目标：确认变更可以合入主干。

所有 PR 必须说明：

- 切片类型
- 关联 Issue
- 本次变更范围
- 本次明确不做
- 文档更新情况
- 测试与验证结果
- 已知风险与例外
- 关联 Issue 的 GitHub 元数据 / 依赖关系审计状态，以及 Delivery PR / Related PRs 关联状态
- Delivery PR 默认要求 `closingIssuesReferences` 覆盖目标 Issue；若只能手动关联 GitHub Development 面板，必须记录显式例外

按切片类型追加要求：

- `docs-only`：说明无运行时行为变更。
- `governance`：说明影响的流程和模板。
- `core-runtime`：提供单元测试、确定性行为说明或未覆盖原因。
- `data-spec`：说明兼容性、版本影响和示例数据影响。
- `adapter`：说明引擎边界、Core 依赖方向和手工验证结果。
- `example`：说明示例运行方式和覆盖能力。
- `cross-layer`：说明端到端路径、回归风险和是否需要示例 smoke test。

安全设置、扫描 workflow、依赖策略或公开发布相关变更还必须按 `security-scanning.md` 记录适用扫描状态、最近运行和开放告警结论；涉及许可证、Cargo 依赖、cargo-deny 或 Dependabot 时还必须满足 `dependency-security.md`。

### 6.1 外部审阅硬门槛

所有 PR 在进入标准 `G3 Pass` 前至少需要一个有效外部 reviewer。`docs-only`、`governance` 和小改动不自动豁免；确需跳过时只能使用第 8 节和本节定义的显式 `G3 Waived`。

单维护者仓库采用双层责任：

- PR author 可以且应执行 author self-review，并可以同时担任 Maintainer / G3 Owner。
- author self-review 不计入外部 reviewer 数量；G3 Owner 负责最终判断，但不能自行补写缺失的外部审阅证据。
- “外部”指独立于 PR author 的受信任审阅执行主体，不要求必须是另一名自然人。受信任的 Copilot、Codex Connector 或其他 GitHub actor/provider 可以满足门槛；其他贡献者创建 PR 时，`wangzishi` 的人工 `APPROVED` 也可以满足门槛。
- 作者转贴的本地 Cursor / Agent 输出、作者本人发布的“已审阅”文字、review request、reaction、pending 状态和没有 reviewed SHA 的摘要均不计数。受信任 Codex provider 的无绑定 clean 摘要还形成失败关闭的时序歧义；只有严格晚于它、绑定 current exact head 且字段完整有效的 clean completion 才能 supersede 该歧义。

有效 completion event 必须记录：

- reviewer actor 与 provider；
- reviewed head SHA，且等于当前 PR head；
- `clean` 或 `findings` 结论；
- 完成时间；
- 可追溯的 review、comment、Check URL 或数据库 ID。

首版状态机为：

```text
AwaitingReview
  -> InReview
  -> Clean
  -> FindingsOpen
  -> AwaitingRereview
  -> Clean

任意 new push / review dismissal：当前结论 -> Stale -> AwaitingReview
```

判定规则：

- 没有有效 completion event 时，即使 `reviewThreads=0` 也保持 `AwaitingReview`。
- 当前 exact head 首次得到一个有效 reviewer 的 clean completion 后，才满足 reviewer 数量门槛。
- 出现 finding 后，仅修复代码、回复或由作者 resolve thread 不能恢复为 clean；必须形成 `finding -> disposition -> exact-head clean re-review`。
- `unresolved actionable threads == 0` 是必要条件，不是充分条件。
- 受信任人工 reviewer 只以 `APPROVED` / `CHANGES_REQUESTED` 分别形成结构化 clean / findings；body-only `COMMENTED` 且没有受信任 inline finding thread 时不构成 completion 或 finding，不对自由文本做关键词或语义阻断推断。
- 受信任 Codex provider 创建且保持未编辑、时间与 URL 有效、符合 clean marker 但缺少可解析 `Reviewed commit` 的 comment，是无绑定 clean 歧义事件。只有创建/提交时间严格晚于该事件、actor/provider 可信、绑定 current exact head 且字段完整有效的 clean completion 才能覆盖它；GitHub 秒级时间相同不能证明先后。仅有此类歧义，或最后一个有效 current-head clean completion 之后仍有此类歧义，均返回 `provider_error`。被编辑 comment、无效时间/URL/OID、分页截断、actor/provider 不可信和 head/base 竞态不适用该窄规则，继续直接失败关闭。
- 无绑定 clean 歧义的标准解除路径是受控请求 marker + 受信绑定记录：G3 Owner 在 PR conversation 新增正文精确为 `external-review: request-codex-review` 的 comment，trusted `Codex Clean Binding` workflow（`issue_comment.created` 触发、仅从 default branch 运行、显式 checkout `refs/heads/main` 且关闭 credential persistence）校验后发布含 `codex-review-request:v1` 隐藏记录的复审请求，记录 request head/base 完整 OID；Codex clean comment 到达时，同一 workflow 的 publisher 校验其 actor、clean verdict 子串形状（正文含固定 `Codex Review:` 与 clean verdict 文本；该形状只区分受信 actor 自身产物的 verdict 类别，信任根是 actor 身份与 head/base 绑定而非 body 语法封闭性）、未编辑与时间/URL 有效，为其选择创建时间严格更早（秒粒度）的最早未消费 marker——候选 head/base 不全相同时无法证明 clean 归属，歧义拒绝发布——并以含 `codex-clean-binding:v1` 隐藏记录的 comment 发布绑定。evaluator 只从快照判定 `github-actions[bot]` 发布、保持未编辑、字段完整有效的 marker 与 record（发布回读确认是 publisher 的运行时后置条件，见 `github-workflow.md`，evaluator 无法从快照重放证明其曾执行，不列为 evaluator 验收项）；record 引用的 marker/clean 不存在、marker 与 clean 同秒、字段无效、id 重复冲突或多个 record 引用同一 clean 均失败关闭。record 绑 current exact head 时其 clean 计为 completion，完成时间取 clean 创建时间；record 绑旧 head 时 clean 落 `stale`，stale record 同样消费其 marker，使跨 push 迟到的旧 head 响应不会永久卡死后续 clean。同秒发布的多条 record 以各自引用 clean 的 `(createdAt, id)` 恢复分配顺序，hash 派生的 record id 只作最终 tie-break。任何人手工伪造 marker 或 record 都因 actor 校验失败关闭。
- R1 没有原生 thread-resolution workflow event；每批 resolve / unresolve 后必须新增顶层 `external-review: thread-state-changed` comment，等待 trusted publisher 重读状态。R2 必须由专用 GitHub App 的 `pull_request_review_thread` webhook 或等价自动信号覆盖两个方向；若 unresolve 仍需人工 marker，不得进入 R2。
- 首版标准路径只接受 exact-head review。content-equivalent rebase 不自动继承 Pass，只能按显式例外处理。
- `dependabot-cargo-lock-only-v1` 是窄范围机器 completion：仅当 PR author 是 Dependabot、变更精确为一个 `MODIFIED` 的 `Cargo.lock`、精确为一个等于 current head 的 commit、该 commit 的 Git author name/email 精确为 `dependabot[bot] <49699333+dependabot[bot]@users.noreply.github.com>`，且 commit 时间与 GitHub URL 有效时适用。它只忽略已 resolved / outdated、绑定 current head、当前正文精确等于 PR #313 已记录三条 Codex bot-author 错误之一的 finding；还必须有受信任人工 `Disposition:` 回复，其当前正文记录 current head 与 Dependabot 的精确 bot identity，且回复的最新更新时间严格晚于 finding 的最新更新时间。该 thread 的其他受信任回复也必须都是同样合格的人工 disposition；任一其他受信任回复都退回标准审阅路径。comment 被编辑本身不使证据失效，判定使用当前正文与 `updatedAt` 顺序；依赖 disposition 的机器 completion 也以最后一条适用回复的 `updatedAt` 和 URL 作为完成证据。Codex provider 不可用或 Copilot 无法审阅在这条窄路径中不构成 completion；由上述 GitHub metadata 形成 completion。任一源文件、额外 commit、受信任 provider 的真实或 dismissed finding、current unresolved authorless thread、受信任 provider 的其他未解决 finding thread 或证据字段不完整都回到标准失败关闭路径；不受信任 actor 的自由文本和 thread 不获得外部审阅结论或阻断权。仅对已经通过上述完整机器条件、且当前 PR body 仍保留已勾选 G3 comment permalink 的 PR，若 Dependabot 自主改写使 `关联 Issue` / `PR 角色` 不再可解析，G3 Shadow 可从该 current G3 comment 的完整规范 `Gate 断言` 集合恢复 target；Issue Gate Ledger、closing set、current head 与其余 G3 校验不放宽，编辑后的 comment 仍按本节统一的 effective time 重新验证。

### 6.2 G3 双钥匙与时序

steady state 的正式 `G3 Pass` 必须同时满足：

1. current head 上由 ruleset 绑定的专用 GitHub App 发布的 `External Review Gate` Check 为 success；
2. Check success 后新增的 `## G3 合并判断` comment 由 G3 Owner 发布并引用同一 head。

`External Review Gate` 是机器权威；G3 comment 是 Owner 决策。PR / Issue body 只保存 permalink 索引，commit 只使用 `Gate: G3 Candidate`，三者不能互相替代。

Issue #446 为合并队列（Merge Queue）冻结三个独立身份：`H_pr` 是 current exact PR Head，承载本节 external review
与 G3 Owner 证据；`H_mg` 是 GitHub 为当前队列组合创建的合并组（Merge Group）Head，承载最新 `main` 上的集成
检查；`H_main` 是最终 rebase 进入默认分支的结果。PR Head 变化会让旧 review / Check / G3 全部 stale；
`main` 前进或队列重排只替换 `H_mg`，不得让未变化 `H_pr` 的外部审阅失效。合并组失败时保持阻断，
但只有修复导致 `H_pr` 变化时才重新进入 exact-head 审阅生命周期。G4 必须保存并复核
`H_pr → H_mg → H_main`，不能用 rebase 后 SHA 不同否定已验证的 inclusion，也不能用 patch inclusion
替代 `H_mg` 上的 required checks。

当前 `H_pr` 与真实 `H_mg` 都必须完成 `Governance checks`、`Rust checks`、`Dependency policy`、
`Analyze (actions)` 与 `Analyze (rust)`；五项 expected source 都是 GitHub Actions App
`integration_id=15368`。Ruleset 的原生 CodeQL `code_scanning` rule 不适用于 Merge Queue groups，不能替代
`H_mg` 上两个 advanced CodeQL required checks。`strict_required_status_checks_policy=false` 只取消手工
up-to-date / rebase 前置，不删除或放宽上述检查。

#446 canary 发现 default setup 没有合并前 `H_mg` 分析后已精确回滚；#451 Related PR #452 随后部署
advanced workflow，并在 default setup 关闭后完成 exact `main` 双语言 dispatch。#451 Delivery canary 只有在
current-head G3 完成后才可保存 before、事务式写入五项 required checks、启用队列并入队。真实 `H_mg` 的
五项 checks 和 CodeQL analysis 全部在合并前成功才提交 after；任一 missing、pending 或 failure 都恢复
Ruleset before、`allow_auto_merge=false` 与队列关闭。Delivery canary 完成 G4 前，不得把主线 dispatch、
shadow telemetry 或临时启用描述为队列集成已经通过。

R1 的 `External Review Gate Shadow` 由 `github-actions` 发布，只用于 non-required telemetry，不能直接升级为 required Check：GitHub required status checks 不区分 workflow、matrix 或 event，同仓 PR 可以创建同 source App 的同名 Actions job。R2 前必须改由独立、最小权限的专用 GitHub App 发布正式 `External Review Gate`，ruleset 同时绑定 Check name 与该 App；spoof canary 必须证明 PR 自定义的同名 job 不能满足 required check。

`G3 Evidence Gate Shadow` 是独立的 R1 证据闭环 telemetry。它从 `main` 上的 trusted validator 运行 `check-gate-evidence-target --repo <owner/repo> --pr <current-pr>`，通常根据 PR body 中一个或多个明确的 `关联 Issue` 和精确 `PR 角色`，为每个 Issue 独立构造 Related-only 或 Delivery full-set G3 参数并全部校验。R1 publisher 只在精确新建的 `g3-evidence: changed` marker 事件或精确 `dependabot-cargo-lock-only-v1` 的 marker 复用事件发布 `G3 Evidence Gate Shadow` Check；其他 PR / Issue / workflow 事件属于仅遥测（telemetry-only），不发布新 Check，也不得把 merged / closed / draft / 非 main base 的 head 重新染红。精确 `dependabot-cargo-lock-only-v1` 的 body 元数据恢复例外按上一节执行。一个 PR 关联多个 Issue 时，同一 current G3 comment 为每个 Issue 分别记录一条精确 `Gate 断言`；完整命令集合必须与全部解析 target 精确相等，不接受缺失、重复或未声明 Issue / 参数的额外成功断言。Delivery target 的完整 `closingIssuesReferences` 必须与全部 `关联 Issue` 精确一致；Related target 的 closing set 必须为空，不能用未声明 closing keyword 绕过自动 target 解析。Delivery 的完整 Related PR 集合只从对应 Issue `Related PRs` 按记录顺序读取；每个 current-policy Related 成员还必须在自身 body 声明 `Related PR` 角色、包含当前 Issue、保持空 closing set，并通过其全部声明 Issue 的断言集合闭包；只有上述精确 Dependabot 例外可从 current G3 comment 的规范断言恢复这些冗余 body 元数据。Related PR 必须已经列入每个对应 Issue，并且是非 Draft `OPEN` current target，或带 `mergedAt` 的 `MERGED` 历史证据；`CLOSED` / 状态与合并时间不一致均 fail closed。具体 PR 编号只能使用无残留选项的 `#<number>` 列表；`pending / #61 / N/A`、`#<number>` 占位、空原因、缺失或重复编号、角色与 Issue 元数据不一致均 fail closed。关联 Issue 必须仍为 `OPEN`；G3 查询不得读取只供 G4 `Project=Done` 使用的 `projectItems`，避免要求 trusted workflow token 拥有不必要的 Projects 权限。

三条 G3 校验路径只允许在 target 展开范围上有差异；target 元数据唯一性、PR 角色、完整且精确的 closing set、完整 Gate 断言集合、`G3 Evidence Gate Shadow` 字段与 waiver record set 必须统一经过同一个 `validate_g3_target` 契约：

| 校验模式            | 入口 / 使用位置                                                   | target 展开范围                                                                                   |
| ------------------- | ----------------------------------------------------------------- | ------------------------------------------------------------------------------------------------- |
| `related-only`      | Related PR 独立 G3                                                | 当前 Issue 与当前 Related PR                                                                      |
| `delivery-full-set` | Delivery PR 整组 G3，以及整组中的每个 Related 成员                | 当前 Issue 的 Delivery PR 与 Issue 已记录的完整 Related PR 集合；成员按自身全部 `关联 Issue` 复核 |
| `shadow-target`     | trusted validator 的 target、marker 与 Shadow success eligibility | 当前 PR body 声明的全部 `关联 Issue`，逐 Issue 解析 Related-only 或 Delivery full-set             |

任何模式发现上述共享契约不一致都必须 fail closed，并在错误中同时给出预期值与实际值；不得为 Related-only 或候选 Shadow 路径保留较弱的 target 校验分支。

完成 current G3 comment、PR body permalink 和 Issue 增量 permalink 后，操作者新增正文精确为 `g3-evidence: changed` 的顶层 PR comment，等待 trusted workflow 重读远端证据。只有该 comment 的 `created` 事件可以首次为直接目标发布 success；marker 必须保持未编辑，并严格晚于 current G3 comment 的 effective time、当前 PR body 与每个关联 Issue body 的最后编辑时间。未编辑 G3 comment 的 effective time 是 `createdAt`；合并前编辑的 comment 必须通过 REST 核对当前正文、PR 归属、`created_at` 与 `updated_at`，并以 `updatedAt` 作为新的 Owner 决策时间，重新验证 current head、外部审阅、字段、waiver 和 Gate 断言。编辑会使旧 marker 失效，必须新增严格更晚的 marker。

唯一 marker 复用例外是已经精确满足 `dependabot-cargo-lock-only-v1` 的 PR 在 marker 后发生 Dependabot 自主 body edit：trusted workflow 可为该 PR 及受其影响的 Delivery target 重用各自最近的未编辑 marker，但必须分页确认 marker 后的全部 body edit editor 均为 Dependabot、`lastEditedAt` 与 `updatedAt` 没有显示其他证据相关 PR activity，并重新运行完整 target、closing set、Issue Ledger、external review、timeline 与 identity 校验；任何人工 edit、分页截断、同秒歧义或证据相关的额外 activity（例如 base change）都仍要求新增 marker。同一 Dependabot `edited` event 中不参与 G3 证据的 title change 不单独使 marker 失效。validator 还分页读取关联 Issue / PR timeline：关联 Issue 的 close/reopen，以及 full-set 全部 PR 的 comment（含 edit 时间）、review、commit、close/reopen、Draft/ready、review request 与 head/base 生命周期活动都必须严格早于 marker；GitHub 同秒无法证明顺序、timeline 分页或事件时间缺失时继续失败关闭。其他 PR/body/Issue body、conversation comment、review/thread、marker edit/delete 与 workflow dispatch 等非 marker 事件不发布新 Check，也不复用旧 marker 恢复 success；它们如需撤销或更新结论，由后续 marker 事件、标准 `check-gate-evidence g3/g4` 命令或 G4 复核显式重新评估。publishable marker 的直接 PR 与 Dependabot reuse run 从 resolver 前按目标 PR 串行，publisher 对级联 target 继续按 PR 串行；review-signal、普通 PR/Issue event 为仅遥测，不进入 publisher。success 前再次完整运行 target 与 marker validator，并复核 PR identity/eligibility。即使旧 marker run 较晚到达 publisher，body 与 timeline freshness 也会拒绝在后发 Related 编辑或状态往返之前创建的 marker。

`G3 Evidence Gate Shadow` 字段以 PR #324 的 `mergedAt = 2026-08-06T10:49:21Z` 为激活边界；effective time 更早的 G3 comment 保留历史语义，不追溯补写该字段，effective time 在激活时点及之后的 comment 必须唯一、非空，并精确使用 `Check URL：https://github.com/...`、`R1 non-required：<原因>` 或 `候选 workflow bootstrap：<边界>` 之一。canonical 形态不包裹整个值，例如 `- G3 Evidence Gate Shadow：R1 non-required：<原因>`；validator 仅为历史证据兼容同一完整值的一层反引号包裹，拒绝只包 URL、部分包裹、空值、重复或其他第三种写法。Issue event resolver 只取 event 中变更前后 body 明确记录的 Delivery / Related PR 并取并集；完全没有治理元数据的无关 Issue 返回空目标，只有字段不完整或歧义时才保守刷新全部 open main PR。`G3 Waived` 是有期限的 action-required 证据；当前 target 或其 Delivery full-set 任一 current-policy PR 为 `G3 Waived`、`G3 Exception` 或历史 `G3 Block` 时，marker 评估发布 non-success；只有完整集合均为 `G3 Pass` / `R0-R1 bootstrap` 才可进入 marker success eligibility。review signal 与 Related PR 普通变化属于仅遥测；下一次 publishable marker 事件重读当前 evidence，并级联刷新对应 Issue 已记录的 Delivery PR，级联目标只能使用其自身最近的有效 marker。该 marker 只是唤醒信号，不能携带结论。`pull_request_target` 不使用 base branch 事件过滤器；non-marker 事件在 resolve-targets 入口跳过，retarget 离开 `main` 不再向旧 head 补发 failure，后续 marker 或显式命令复核时仍按当前 base 失败关闭。`G3 Evidence Gate Shadow` 由 `github-actions` 发布且当前不在 ruleset 中，只能证明 trusted-ref replay 的 telemetry 结果，不能声称已经阻止合并；修改该 workflow / validator 的候选 PR 也不能用尚未合入 `main` 的实现自批。

标准 `check-gate-evidence g3` 和 target 模式只接受仍为 `OPEN` 的关联 Issue，以及仍为 `OPEN`、非 Draft 且尚未合并的当前 Delivery / Related PR。target 模式在确认 PR / Issue 角色元数据稳定后必须重跑完整远端证据校验，不能只比较数字参数。Draft、closed 或其他不再 eligible 的同仓 PR 不发布新的 Check，保留其 eligible 时最后一次有效评估结论；合并后补写、编辑或重放标准 G3 在标准 `check-gate-evidence g3/g4` 与 G4 历史复核中继续失败关闭。`check-gate-evidence g4` 继续允许读取合并前 effective time 已形成的 G3 证据做历史复核。

fork / cross-repository PR 的 head commit 不保证存在于 base repository，base repository 的 `GITHUB_TOKEN` 因而不能可靠创建关联 Check。此类 PR 不计入 R1 eligible sample；R2 不把缺失 Check 当作成功，必须把最终 patchset 迁移为 same-repository PR，并在新 PR 的 exact head 重新完成外部审阅与 G3。只有 security / emergency hotfix 等既有显式例外可以使用临时 ruleset bypass，不能形成 fork 的 standing bypass。

G3 Owner 可以在 PR 合并前纠正 current G3 comment，也可以新增 superseding comment。new push、review dismissal 或 Gate 状态变化后，旧 review、旧 Check 与旧 G3 内容对新 head 全部 stale；无论选择编辑还是新增，都必须绑定 current head、重新满足 completion / Check、按新的 effective time 重验，并新增严格更晚的 marker。合并后不得编辑 G3 形成或改变历史证据。唯一可恢复的既成事实是：合并前正文语义有效，合并后只去除或增加 `G3 Evidence Gate Shadow` 完整值的一层反引号。G4 replay 必须从 GitHub `UserContentEdit.diff` 相邻快照验证 original/new SHA-256、editor、`editedAt`、原版早于 merge、新版晚于 merge，并由 trusted G3 Owner 在编辑后新增未编辑的 `g3-comment-correction:v1` appendix。记录字段固定为 `schemaVersion`、唯一 `id`、`issue`、`pullRequest`、`currentHeadOid`、`g3Comment`、`originalBodySha256`、`newBodySha256`、`editedAt`、`editor`、`reason`、`risk`、`acceptanceBoundary`、独立 `followUpIssue`、`cleanupOwner`、`authorizedBy`。该记录不是 exception，不能改变 Gate 结果、断言结果或合并合法性；与 pre-merge `confirmed_gate_defect` 组合回放时，exception 的正文哈希与 acceptedAt 必须绑定 correction 已验证恢复的 original snapshot，而不是格式编辑后的当前正文。任一其他正文差异、哈希/actor/head/时序不符、编辑历史分页或 appendix 自身被编辑都失败关闭。

`G3 Exception` 是“断言未通过但由 G3 Owner 接受风险”的一等审计状态，机器状态固定为 `accepted_exception`，绝不等于 `pass`。current 路径只允许 `confirmed_gate_defect`，必须在目标 G3 comment 之后另发未编辑的 `g3-exception:v1` appendix，并在验证/merge 时未过期（最长 24 小时）；多 Issue PR 按记录中的 `issue` 精确限定 exception 作用域，只有匹配 Issue 的 Gate 断言写“未通过”，其余 Issue 仍可独立写“已通过”并走正常 exact-head 外部审阅。historical G4 replay 只允许 `legacy_evidence_reconstruction`，用于既有 `G3 Block` 或历史 `G3 Pass + 未通过`，且接受事件必须晚于原 merge、在实际 G4 evaluation time 仍未过期，明确不追授原 merge 合规。记录字段固定为 `schemaVersion`、唯一 `id`、`exceptionType`、`issue`、`pullRequest`、`currentHeadOid`、`currentBaseOid`、`g3Comment`、`g3CommentBodySha256`、`reason`、非空 `evidenceRefs`、`risk`、`acceptanceBoundary`、`acceptedAt`、`expiresAt`、独立 `followUpIssue`、`cleanupOwner`、`authorizedBy`；current comment 还必须可见记录与 GitHub PR identity 一致的完整 current head。合并前 current 校验继续要求 live head/base 精确一致；G4 重放 pre-merge `confirmed_gate_defect` 时，head 仍精确匹配，而记录中的完整 base OID 保留为 merge 边界审计值，不与已经前移的 live post-merge base ref 比较。可见 `- 例外：` 行必须引用同一 GitHub evidence。未授权、过期、重复、未知类型、正文/identity/hash 不符均失败关闭。它与 `external-review-waiver:v1`、`g3-comment-correction:v1` 分离，不可互换。

在 #230 的 R0 / R1 bootstrap 阶段，required `External Review Gate` 尚未启用：

- governance contract、validator 和 shadow workflow 三个 Related PR 仍按当前 ruleset 完成各自 G3；
- G3 comment 必须明确 rollout phase、current head、人工复核的 exact-head 外部 review URL，以及 Check 尚未 required 的原因；
- Related PR B 自身仍由 `main` 上的旧 validator 判断；其 `check-gate-evidence g3` 只校验 legacy comment 字段、permalink、PR 关系和时序，G3 Owner 必须人工逐项核验 current head 与 external review lifecycle，并在 comment 中声明候选 validator 不能自批；
- Related PR B 合入后，后续 PR 的 `check-gate-evidence g3` 还会通过 live `check-external-review` 要求 exact-head `pass`、完整 current head 和晚于最终 completion 的 G3 comment；R0/R1 仍须声明 `External Review Gate` Check 尚未 required；
- Related PR C 自身不能用候选 shadow workflow 自批；其 G3 使用已合入 main 的 live validator、current-head 外部审阅与当前 ruleset，并把 Check 尚未发布 / required 的 bootstrap 边界写入 current G3 comment。PR C 合入 main 且首次 trusted-ref Check 验证成功后，才追加 R1 起点并开始计时 / 计样本；
- external-review G3 集成以 Issue #230 G2-B 增量记录的 `createdAt = 2026-07-24T15:16:21Z` 为迁移边界；effective time 更早的历史 G3 comment 不追溯改写，effective time 在该时点及之后的 current G3 comment 按现行字段规则完整校验；
- 该过渡记录只证明 bootstrap PR 按当时有效规则通过，不证明 steady-state External Review Gate 已生效；
- R2 激活后不再接受该 bootstrap 路径。

### 6.3 Rollout、ruleset 与 waiver

外部审阅门禁按三阶段实施：

- `R0`：provider fixtures、历史事件 replay、fail-closed、head binding 与 trusted-ref workflow 离线验证。
- `R1`：`External Review Gate Shadow` 以 non-required telemetry 运行至少 14 天且覆盖至少 10 个 eligible PR；0 false-pass、最终分类全部与人工审计一致、new push 与 re-review 语义全部正确，且 workflow 无权限/secret/untrusted-code 事件。样本必须绑定 trusted workflow run，不能只按同名 Check 计数。
- `R2`：R1 达标、专用 GitHub App publisher 就绪且同名 Actions spoof canary 失败后，才启用 required `External Review Gate`、conversation resolution，移除 `update` restriction 与 standing `always` bypass；再观察至少 7 天且 5 个 merged PR。break-glass、false-pass 或安全异常使稳定期重新计时。

`G3 Evidence Gate Shadow` 使用同样的 R1 安全与准确性口径，但其 R2 强制身份单独选择：优先使用 organization ruleset 的 required workflow，把 trusted workflow 本身作为必需来源；若当前 plan / API 不支持，则由独立 GitHub App 发布专用 Check，并让 ruleset 绑定 Check name 与 expected source App。两条路径都必须先通过同名 Actions spoof canary、对全部 open PR 重新触发校验、保存 ruleset 前后快照，并移除 standing `always` bypass；能力 API 的 `403` / `404` 只能记为不可验证或不可用，不能解释为已经启用或没有配置。

首版只允许三类 `G3 Waived`：

- content-equivalent rebase；
- 所有已配置 provider / Gate platform 不可用且存在明确时间边界；
- security / emergency hotfix；

已确认且无法及时修复的 Gate false-block 不再进入 current waiver 路径；它只能使用上文定义的
`G3 Exception` + `confirmed_gate_defect`，保持 `accepted_exception` 非成功状态。策略切换以 Issue #405 G1
决策记录的创建时间 `2026-08-18T04:20:55Z` 为明确边界；在该时点前已用旧规则合并的
`G3 Waived + confirmed_gate_defect` 只允许在 G4 以原 merge 时点重放；这项 grandfather 不接受 OPEN/current PR，也不恢复日常 bypass。

普通审阅延迟、作者不同意 finding、`docs-only`、赶进度和减少步骤均不是 waiver 理由。waiver 必须记录 exception type、PR/current head、已有证据、风险、临时接受边界、默认不超过 24 小时的到期时间、follow-up Issue、Cleanup owner，以及临时 bypass 的添加/撤回时间。Check 与 G3 comment 必须显示 `G3 Waived`，不得伪装成标准 `G3 Pass`。

`check-gate-evidence g3` 只在以下结构化边界内接受 `G3 Waived`：

- current G3 comment 的 `- Gate 结果：` 必须精确为 `G3 Waived`；合并前编辑时按 REST 核验的 `updatedAt` 重新计时并重验完整 waiver；
- 单 Issue PR 的 comment 必须包含且只包含一个 `external-review-waiver:v1` HTML comment；多 Issue PR 必须为每个关联 Issue 分别包含一个记录，并以唯一 `followUpIssue` 精确匹配；完整 record set 必须与 PR 的 `关联 Issue` 集合相等，不接受缺失、额外、重复或共享的模糊 waiver。每个 HTML comment 内都是 `schemaVersion: 1` JSON；字段固定为 `id`、`exceptionType`、`currentHeadOid`、`currentBaseOid`、`reason`、`evidenceRefs`、`risk`、`acceptanceBoundary`、`expiresAt`、`followUpIssue`、`cleanupOwner`、`authorizedBy`；
- `evidenceRefs` 只保存 Markdown reference label；每个 label 必须由可见的 `- 例外：` 行引用，并在 comment 文末解析为 GitHub HTTPS 证据，JSON 内不直接写 URL；
- current head/base 必须与 live PR 一致，`followUpIssue` 必须指向当前关联 Issue，`authorizedBy` 必须等于 current G3 comment author，且该 actor 必须在 trusted G3 Owner allowlist；
- `expiresAt` 必须晚于 comment effective time，且有效期从该时点起不超过 24 小时。当前仍为 `OPEN` 的 Delivery / Related PR 在每次 Gate 运行时都必须保持未过期；Delivery full-set 或 G4 复核带 `mergedAt` 的 `MERGED` 历史 Related PR 时，只按该 `mergedAt` 判断 waiver 在其合并时是否有效。该历史复核不延长或恢复 waiver，不适用于 Delivery PR 自身或 `OPEN` Related PR，不把 `waived` 转换为 `pass`，也不允许包含 waived member 的 full-set 取得 Shadow success；缺失或非法 `mergedAt` 继续失败关闭；
- validator 的输出保持 `waived`，只与 `G3 Waived` 配对；它不得转换成标准 `pass`，`G3 Pass` / `R0-R1 bootstrap` 仍要求 live exact-head `pass`。

content-equivalent rebase 还必须记录 reviewed/new head、old/new base、changed paths、稳定 patch fingerprint、受影响路径 blob 对照和常规 checks；workflow、Gate、权限、安全策略、依赖锁定语义变化或任何无法解释的不等价都禁止使用该例外。

默认阻断条件：

- Adapter 代码把引擎依赖泄漏进 Core。
- 数据格式变化没有文档说明。
- Core API 破坏性变化没有 ADR 或 design 依据。
- PR 声称完成父任务，但实际只完成子范围。
- 必需测试未运行且没有原因。
- 例外没有清理责任或后续 Issue。
- 缺少 G0-G2 Gate Ledger，且没有记录为显式例外或补救。
- 关联 Issue 缺少必需 GitHub 元数据 / 依赖关系审计且没有显式例外，或不适用项缺少 `N/A` 原因。
- Delivery PR 的完整 `closingIssuesReferences` 与全部 `关联 Issue` 不精确一致，或 Related PR 的 closing set 非空，且没有显式例外。
- PR commit message 不符合 `docs/reference/commit-convention.md`，且没有记录显式例外。
- 缺少有效外部 review completion、reviewed head 不是 current head、review 仍 pending/stale，或 actor/provider 不可信。
- finding 尚未处置、处置后没有 exact-head clean re-review，或仍有 unresolved actionable thread。
- R2 激活后，current-head `External Review Gate` 未成功，或 G3 comment 早于最终 Check / completion。
- G3 comment 在 PR 合并后被补写或编辑，编辑后未按新 effective time 重验 / 新增 marker，或 commit 使用 `G3 Pass` / `G3 Waived` 冒充 PR Gate 结果。
- 源代码许可证、依赖许可证、RustSec advisory、crate 来源或 Dependabot 配置违反 `dependency-security.md`，或适用 cargo-deny 检查未通过。
- `security-scanning.md` 要求的适用扫描仍为 `pending`、失败、无分析、已禁用或不可用，且没有记录显式例外。
- `H_pr` 缺少上述任一 required context，或入队后的真实 `H_mg` 没有在合并前完成同名、同 expected source
  的五项 success；PR / `main` 历史结果和原生 CodeQL rule 都不能补足 `H_mg` 缺口。

PR 默认通过合并队列（Merge Queue）合入 `main`，队列最终使用 **Rebase**；不得使用 `--admin` 绕过。
Squash 或 Merge commit 等最终方式例外必须先通过治理 Issue 修改适用规则并说明原因。详见
`github-workflow.md` 第 7 节。

G3 记录必须写在 PR 的 `## G3 合并判断` comment 中，至少包含 current head、rollout phase、`Checks`、`External Review Gate`、`G3 Evidence Gate Shadow`、审阅、验证、风险、例外、合并方式和 `Gate 断言`。PR body 的 G3 checkbox 必须勾选并回链当前 PR comment；Issue body 的 G3 Gate Ledger 必须增量回链该 comment，只有 Delivery PR 与全部 Related PR 均完成时才勾选。`Gate 断言` 必须使用当前角色对应的 Related-only 或 full-set 规范命令，一个 PR 关联多个 Issue 时分别写一条。`G3 Pass` 填写后必须逐条运行成功；`G3 Exception` 必须如实保留 `未通过` 并满足结构化记录，不得改写成成功。

```text
## G3 合并判断

- Gate 结果：G3 Pass / G3 Waived / G3 Exception / R0-R1 bootstrap
- Rollout phase：R0 / R1 / R2
- Current head：
- Checks：
- External Review Gate：Check URL / R0-R1 non-required 原因
- G3 Evidence Gate Shadow：按 Rollout phase 只保留一项且不包裹整个值：R0 = 候选 workflow bootstrap：<边界>；R1 = R1 non-required：<原因>；R2 = Check URL：https://github.com/...
- 审阅：provider、actor、reviewed head、outcome、completion time、evidence URL
- Review threads：actionable / unresolved / disposition / re-review
- 验证：
- 风险：
- 例外：N/A / exception type、风险、到期、follow-up、Cleanup owner
- 合并方式：Merge Queue（最终 Rebase）；Queue-ready H_pr： / 例外原因
- Gate 断言：`<与实际运行完全一致的 check-gate-evidence g3 Related-only 或 full-set 规范命令>` <按 Gate 结果填写：`G3 Exception` 写“未通过”，其他结果写“已通过”>。
```

`xtask scaffold-g3-comment` 目前只冻结输入/输出草案，不在本切片实现 GitHub 写入：输入为 `--repo`、一个或多个 `--issue`、当前 PR 及其 Delivery/Related role、完整 Related set 和 current head；工具读取远端 Issue/PR identity 后，输出 canonical comment、每个 Issue 的语义规范命令及本地预检结果。任一 identity、关系、head 或预检失败时不输出可发布结论；操作者审阅后自行发表 comment、更新 permalink 并运行正式远端校验。

## 7. G4 完成闸口

目标：确认 `Done` 代表后续任务可以依赖。

Issue 关闭前必须满足：

- 关联 PR 已按默认策略经 Merge Queue 最终 Rebase 合并，或说明为什么无需 PR / 为什么使用其他合并方式。
- 验收 checklist 已完成。
- 文档已回写，或说明不需要。
- 测试和验证结果已记录。
- 未完成范围已拆出后续 Issue。
- 父 Issue 只在所有子 Issue 完成后关闭。
- G4 记录已回写关联 Issue。
- Project 中关联 Issue 和 PR 均已移动到 `Done`，或说明为什么不适用。
- Delivery PR、Related PRs、Parent / sub-issues、Blocked by、Blocking 已收口，或剩余关系已拆出后续 Issue 并记录原因。
- 关联 Issue 已由 G4 清场手动关闭；不得依赖 GitHub 自动关闭 Issue 替代 G4。
- 本地和远端 PR 分支已清理，或说明保留原因。
- 临时权限、ruleset bypass 或 admin override 已撤回，或说明保留原因、风险和 Cleanup owner。
- 已在所有关联 PR 合并后、Issue 关闭前发表 `## G4 完成判断` comment；Issue body G4 checkbox 已回链该 comment，Delivery PR body 已回链该 Issue G4 comment。
- `check-gate-evidence g4` 已成功运行；正常 G4 comment 的 `Gate 断言` 行以规范格式记录语义一致的命令和 `已通过` 结果。只有合格的 `g3-exception:v1` historical replay 才保留 `未通过` 并输出 `accepted_exception`，且不得描述为 Pass。`待运行`、无结构化记录的失败、缺少成功标记或参数不匹配不得通过 G4。
- `2026-08-20T04:00:00Z` 是 Merge Queue G4 证据的固定 activation boundary；该时刻后合并的 Delivery 或 Related PR 都必须在 `merge-queue-g4-evidence:v1` 中各有一条 `merge_queue` record。validator 按 Delivery-first、随后 Issue Related PR 顺序验证完整集合，把每条 `H_pr` / `H_main` 与 GitHub `headRefOid` / `mergeCommit.oid` 对照，并要求 `checksUrl` 精确为当前 `H_mg` 的 commit checks 页面、`chain` 使用规范顺序、inclusion 方法与 `H_pr...H_mg` compare permalink 精确绑定。
- G4 live validation 还须从 GitHub API 读取 `merge_group` workflow runs、PR timeline 与目标分支全部当前生效的 rules；live `merge_queue` rule 必须仍存在，合并前最后一个 queue event 不得是 dequeue，最后一次入队到 merge 之间对应 PR 的最后一代 queue head 必须等于记录的 `H_mg`，并存在 trusted success run；每个 live required check 只按 merge 时刻前已完成的最后一个同名 check run 判定，且必须在该 `H_mg` 上 `completed/success`。两个 CodeQL checks 还须绑定 `integration_id=15368`，并存在同一 `H_mg` 的 advanced workflow analysis。该边界用于排除激活后回滚、任意自报 SHA、同次入队内被替换的旧 queue head、合并后 rerun 或旧队列结论；不要求本地抓取已删除的临时 ref 或执行逐补丁形式化重放。
- 全部关联 PR 都早于 boundary 合并的历史 G4 可继续无 record 重放；若一个 Issue 同时包含 activation 前后 PR，则 record 仍须覆盖全部成员，历史成员使用 `pre_activation`、保存 `H_pr/H_main` 与非空 reason，禁止补造 `H_mg`。boundary 后的非队列 merge 默认失败关闭；若治理决定允许例外，必须先通过独立 Issue 扩展结构化 record 与 validator，不能只改 `- 合并：` 文本。

G4 记录只负责最终闭环；不应在 G4 阶段首次补写 G0-G3。若必须补写，应标记为补救记录。

```text
## G4 完成判断

- 合并：
- Merge Queue evidence：见 `merge-queue-g4-evidence:v1`；按 Delivery-first、随后全部 Related PR 顺序。
- main CI：
- 验收：
- Project：
- 关系：
- 分支：
- 权限 / bypass：N/A，原因：/ 保留原因、风险、Cleanup owner：
- Gate 断言：`cargo +<workspace-rust-version> run --locked -p xtask -- check-gate-evidence g4 --repo <owner/repo> --issue <number> --delivery-pr <number> [--related-pr <number>]...` <正常 G4 写“已通过”；仅精确匹配 `legacy_evidence_reconstruction` 的 historical replay 写“未通过”>。

<!-- merge-queue-g4-evidence:v1
{
  "schemaVersion": 1,
  "activationBoundary": "2026-08-20T04:00:00Z",
  "pullRequests": [
    {
      "number": 450,
      "role": "delivery",
      "mode": "merge_queue",
      "hPr": "<40-hex>",
      "hMain": "<40-hex>",
      "hMg": "<40-hex>",
      "checksConclusion": "success",
      "checksUrl": "https://github.com/<owner>/<repo>/commit/<H_mg>/checks",
      "chain": "<H_pr> -> <H_mg> -> <H_main>",
      "inclusionMethod": "trusted GitHub merge_group identity + compare",
      "inclusionEvidenceUrl": "https://github.com/<owner>/<repo>/compare/<H_pr>...<H_mg>"
    },
    {
      "number": 448,
      "role": "related",
      "mode": "pre_activation",
      "hPr": "<40-hex>",
      "hMain": "<40-hex>",
      "reason": "merged before 2026-08-20T04:00:00Z"
    }
  ]
}
-->
```

### 7.1 Delivery 合并后新增 Related PR 的 G4 recovery

正常流程必须在 Delivery G3 前冻结完整 Related PR 集合，并使用 full-set 命令。只有
Delivery 已合法合并、之后才发现验收缺口且 late Related PR 也已分别完成 Related-only
G3 时，才可在最终 G4 comment 使用 `g3-full-set-recovery:v1`。该路径只恢复 G4
整组复核，不改变 `check-gate-evidence g3`，也不允许在 Delivery 合并后编辑其历史 G3 comment
或补写 G3。

recovery 必须同时满足：

- Delivery 的原 G3 comment 按其原始 `originalRelatedPrs` 命令仍可验证，且 effective time
  严格早于 Delivery merge；
- 每个 `lateRelatedPrs` 的 PR `createdAt` 严格晚于 Delivery `mergedAt`，并继续满足
  Related-only G3、G3 comment effective time 早于自身 merge、current-head external review、非 closing
  linkage、merge 和 Project `Done`；
- `originalRelatedPrs + lateRelatedPrs` 按顺序等于最终 G4 命令和 Issue 元数据记录的
  Related PR 全集；
- 最终 G4 comment 未编辑，author 与 `authorizedBy` 一致且属于 trusted G3 Owner；
  `- 关系：` 可见回链 Delivery 和每个 Related PR 的 G3 permalink；
- G4 的其余 merge timing、Gate Ledger、Project、Delivery backlink 和 exact command
  校验保持不变。

结构化记录格式：

```text
<!-- g3-full-set-recovery:v1
{
  "schemaVersion": 1,
  "exceptionType": "late_related_after_delivery_merge",
  "issue": 123,
  "deliveryPr": 124,
  "deliveryMergedAt": "2026-07-25T10:32:36Z",
  "originalRelatedPrs": [],
  "lateRelatedPrs": [125],
  "reason": "验收缺口在 Delivery merge 后才被确认",
  "evidenceRefs": ["delivery-g3", "related-125-g3"],
  "risk": "历史 Delivery G3 无法预先命名未来 PR",
  "acceptanceBoundary": "只恢复最终 G4；normal G3 不放宽",
  "followUpIssue": "#126",
  "cleanupOwner": "owner-login",
  "authorizedBy": "owner-login"
}
-->
```

`evidenceRefs` 必须由同一 G4 comment 的 `- 关系：` 行可见引用，并通过文末
reference-style 定义解析为对应的 GitHub G3 permalinks。不存在 late Related PR 时，
G4 继续使用严格 full-set G3，且 G4 comment 不得包含 recovery 标记；一旦检测到
late Related PR，缺少结构化记录，或时间、集合、授权、证据任一不匹配，均 fail
closed。GitHub 时间只有秒级；Delivery 与 original Related 的 G3 comment effective time，
或任一 Related PR `createdAt` 与 Delivery `mergedAt` 同秒时无法安全判定先后，也必须失败。

## 8. 例外治理

允许例外，但必须显式留痕。

例外记录至少包含：

- 原因
- 风险范围
- 临时接受边界
- 后续清理 Issue
- Cleanup owner

不得用“后面再补”替代例外记录。
