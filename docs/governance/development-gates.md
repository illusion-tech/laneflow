# 开发闸口

**文档状态**: Active  
**最后更新**: 2026-07-24

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
- 每个 Related PR 独立 G3 都必须运行 `cargo +1.96.0 run --locked -p xtask -- check-gate-evidence g3 --repo <owner/repo> --issue <number> --related-pr <current-related-pr>`；该 comment 永久保留 Related-only 断言，只验证当前 Related PR 的 comment、仍未勾选的 Issue G3 增量 permalink 与关系，不声明 Issue 整体 G3 已完成。若 Issue G3 已提前勾选，Related-only 校验必须失败。
- Delivery PR G3、整组关系复核与 G4 使用 `cargo +1.96.0 run --locked -p xtask -- check-gate-evidence <g3|g4> --repo <owner/repo> --issue <number> --delivery-pr <number> [--related-pr <number>]...`，并传入 Issue 已记录的全部 Related PR。整组复核按各 Related PR 原有的 Related-only 断言验证其 append-only comment，不要求改写为 full-set 命令；Delivery PR comment 和 Issue 最终断言使用 full-set 命令。`Gate 断言` 行必须用反引号记录与本次参数完全一致的规范命令并明确写 `已通过`；`待运行`、缺少成功标记或参数不匹配均视为 Gate 失败。命令或远端读取失败同样是 Gate 失败。
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
- 作者转贴的本地 Cursor / Agent 输出、作者本人发布的“已审阅”文字、review request、reaction、pending 状态和没有 reviewed SHA 的摘要均不计数。

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
- 首版标准路径只接受 exact-head review。content-equivalent rebase 不自动继承 Pass，只能按显式例外处理。

### 6.2 G3 双钥匙与时序

steady state 的正式 `G3 Pass` 必须同时满足：

1. current head 上的 `External Review Gate` Check 为 success；
2. Check success 后新增的 `## G3 合并判断` comment 由 G3 Owner 发布并引用同一 head。

`External Review Gate` 是机器权威；G3 comment 是 Owner 决策。PR / Issue body 只保存 permalink 索引，commit 只使用 `Gate: G3 Candidate`，三者不能互相替代。

G3 comment 采用 append-only。new push、review dismissal 或 Gate 状态变化后，旧 review、旧 Check 和旧 G3 comment 对新 head 全部 stale；必须产生新的 completion、Check 与 superseding G3 comment，不得编辑旧评论冒充新时点。

在 #230 的 R0 / R1 bootstrap 阶段，required `External Review Gate` 尚未启用：

- governance contract、validator 和 shadow workflow 三个 Related PR 仍按当前 ruleset 完成各自 G3；
- G3 comment 必须明确 rollout phase、current head、人工复核的 exact-head 外部 review URL，以及 Check 尚未 required 的原因；
- Related PR B 自身仍由 `main` 上的旧 validator 判断；其 `check-gate-evidence g3` 只校验 legacy comment 字段、permalink、PR 关系和时序，G3 Owner 必须人工逐项核验 current head 与 external review lifecycle，并在 comment 中声明候选 validator 不能自批；
- Related PR B 合入后，后续 PR 的 `check-gate-evidence g3` 还会通过 live `check-external-review` 要求 exact-head `pass`、完整 current head 和晚于最终 completion 的 G3 comment；R0/R1 仍须声明 `External Review Gate` Check 尚未 required；
- external-review G3 集成以 Issue #230 G2-B 增量记录的 `createdAt = 2026-07-24T15:16:21Z` 为迁移边界；更早的历史 G3 comment 不追溯改写，该时点及之后的 current G3 comment 若发生创建后编辑则 fail closed；
- 该过渡记录只证明 bootstrap PR 按当时有效规则通过，不证明 steady-state External Review Gate 已生效；
- R2 激活后不再接受该 bootstrap 路径。

### 6.3 Rollout、ruleset 与 waiver

外部审阅门禁按三阶段实施：

- `R0`：provider fixtures、历史事件 replay、fail-closed、head binding 与 trusted-ref workflow 离线验证。
- `R1`：Check 以 non-required shadow 运行至少 14 天且覆盖至少 10 个 eligible PR；0 false-pass、最终分类全部与人工审计一致、new push 与 re-review 语义全部正确，且 workflow 无权限/secret/untrusted-code 事件。
- `R2`：R1 达标后启用 required Check、conversation resolution，移除 `update` restriction 与 standing `always` bypass；再观察至少 7 天且 5 个 merged PR。break-glass、false-pass 或安全异常使稳定期重新计时。

首版只允许四类 `G3 Waived`：

- content-equivalent rebase；
- 所有已配置 provider / Gate platform 不可用且存在明确时间边界；
- security / emergency hotfix；
- 已确认且无法及时修复的 Gate false-block。

普通审阅延迟、作者不同意 finding、`docs-only`、赶进度和减少步骤均不是 waiver 理由。waiver 必须记录 exception type、PR/current head、已有证据、风险、临时接受边界、默认不超过 24 小时的到期时间、follow-up Issue、Cleanup owner，以及临时 bypass 的添加/撤回时间。Check 与 G3 comment 必须显示 `G3 Waived`，不得伪装成标准 `G3 Pass`。

`check-gate-evidence g3` 只在以下结构化边界内接受 `G3 Waived`：

- current G3 comment 的 `- Gate 结果：` 必须精确为 `G3 Waived`，且 comment 未编辑；
- comment 必须包含且只包含一个 `external-review-waiver:v1` HTML comment，其中是 `schemaVersion: 1` JSON；字段固定为 `id`、`exceptionType`、`currentHeadOid`、`currentBaseOid`、`reason`、`evidenceRefs`、`risk`、`acceptanceBoundary`、`expiresAt`、`followUpIssue`、`cleanupOwner`、`authorizedBy`；
- `evidenceRefs` 只保存 Markdown reference label；每个 label 必须由可见的 `- 例外：` 行引用，并在 comment 文末解析为 GitHub HTTPS 证据，JSON 内不直接写 URL；
- current head/base 必须与 live PR 一致，`followUpIssue` 必须指向当前关联 Issue，`authorizedBy` 必须等于 append-only G3 comment author，且该 actor 必须在 trusted G3 Owner allowlist；
- `expiresAt` 必须晚于 comment 创建时间、有效期不超过 24 小时，并在每次 Gate 运行时仍未过期；
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
- Delivery PR 的 `closingIssuesReferences` 未覆盖对应 Issue，或 Related PR 误用 closing keyword，且没有显式例外。
- PR commit message 不符合 `docs/reference/commit-convention.md`，且没有记录显式例外。
- 缺少有效外部 review completion、reviewed head 不是 current head、review 仍 pending/stale，或 actor/provider 不可信。
- finding 尚未处置、处置后没有 exact-head clean re-review，或仍有 unresolved actionable thread。
- R2 激活后，current-head `External Review Gate` 未成功，或 G3 comment 早于最终 Check / completion。
- G3 comment 通过编辑旧记录回填，或 commit 使用 `G3 Pass` / `G3 Waived` 冒充 PR Gate 结果。
- 源代码许可证、依赖许可证、RustSec advisory、crate 来源或 Dependabot 配置违反 `dependency-security.md`，或适用 cargo-deny 检查未通过。
- `security-scanning.md` 要求的适用扫描仍为 `pending`、失败、无分析、已禁用或不可用，且没有记录显式例外。

PR 合入 `main` 默认使用 **Rebase and merge**；若使用 Squash 或 Merge commit，须在 PR 中说明原因。详见 `github-workflow.md` 第 7 节。

G3 记录必须写在 PR 的 `## G3 合并判断` comment 中，至少包含 `Checks`、审阅、验证、风险、例外、合并方式和 `Gate 断言`。PR body 的 G3 checkbox 必须勾选并回链当前 PR comment；Issue body 的 G3 Gate Ledger 必须增量回链该 comment，只有 Delivery PR 与全部 Related PR 均完成时才勾选。`Gate 断言` 必须使用当前角色对应的 Related-only 或 full-set 规范命令，参数与实际调用完全一致；填写后立即运行该命令，若失败必须移除 `已通过` 并修复证据。运行成功前不得合并。

```text
## G3 合并判断

- Gate 结果：G3 Pass / G3 Waived / R0-R1 bootstrap
- Rollout phase：R0 / R1 / R2
- Current head：
- Checks：
- External Review Gate：Check URL / R0-R1 non-required 原因
- 审阅：provider、actor、reviewed head、outcome、completion time、evidence URL
- Review threads：actionable / unresolved / disposition / re-review
- 验证：
- 风险：
- 例外：N/A / exception type、风险、到期、follow-up、Cleanup owner
- 合并方式：Rebase and merge / 例外原因
- Gate 断言：`<与实际运行完全一致的 check-gate-evidence g3 Related-only 或 full-set 规范命令>` 已通过。
```

## 7. G4 完成闸口

目标：确认 `Done` 代表后续任务可以依赖。

Issue 关闭前必须满足：

- 关联 PR 已按默认策略（Rebase and merge）合并，或说明为什么无需 PR / 为什么使用其他合并方式。
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
- `check-gate-evidence g4` 已成功运行；G4 comment 的 `Gate 断言` 行以规范格式记录与实际调用完全一致的命令和 `已通过` 结果。`待运行`、缺少成功标记或参数不匹配不得通过 G4。

G4 记录只负责最终闭环；不应在 G4 阶段首次补写 G0-G3。若必须补写，应标记为补救记录。

```text
## G4 完成判断

- 合并：
- main CI：
- 验收：
- Project：
- 关系：
- 分支：
- 权限 / bypass：N/A，原因：/ 保留原因、风险、Cleanup owner：
- Gate 断言：`cargo +1.96.0 run --locked -p xtask -- check-gate-evidence g4 --repo <owner/repo> --issue <number> --delivery-pr <number> [--related-pr <number>]...` 已通过。
```

## 8. 例外治理

允许例外，但必须显式留痕。

例外记录至少包含：

- 原因
- 风险范围
- 临时接受边界
- 后续清理 Issue
- Cleanup owner

不得用“后面再补”替代例外记录。
