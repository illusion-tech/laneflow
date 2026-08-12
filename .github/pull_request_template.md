# PR 治理检查清单

## 范围

- 关联 Issue：
- PR 角色：`Delivery PR` / `Related PR`
- Development 关联：
  - Delivery PR：唯一可完成关联 Issue 验收边界的 PR，使用 `Closes #<issue>` / `Resolves #<issue>`，完整 `closingIssuesReferences` 必须与全部 `关联 Issue` 精确一致。
  - Related PR：部分交付，使用 `Refs: #<issue>`；完整 `closingIssuesReferences` 必须为空。
- 切片类型：
  - [ ] docs-only（仅文档）
  - [ ] governance（治理）
  - [ ] core-runtime（Core 运行时）
  - [ ] data-spec（数据格式）
  - [ ] adapter（引擎适配）
  - [ ] authoring-tool（编辑工具）
  - [ ] example（示例）
  - [ ] cross-layer（跨层高风险）
- 本次 PR 变更：
- 本次 PR 明确不做：

## 关联 Issue 元数据 / 依赖关系审计

- [ ] 关联 Issue 的 Project、Project status、Labels 已核验；缺失项已有显式例外。
- [ ] Milestone、Parent / sub-issues、Blocked by、Blocking 已核验；不适用项已有 `N/A` 原因。
- [ ] Delivery PR / Related PRs 已在 Issue 中准确记录；若本 PR 是 Delivery PR，完整 `closingIssuesReferences` 与全部 `关联 Issue` 精确一致；若本 PR 是 Related PR，已记录 `Refs: #<issue>` 且 closing set 为空。

## 影响

- Core API 影响：`无` / 说明：
- 数据格式影响：`无` / 说明：
- Adapter API 影响：`无` / 说明：
- 示例或演示影响：`无` / 说明：
- 依赖 / 许可证影响：`无` / 说明依赖名称、用途、来源、许可证和分发影响：
- 破坏性变更：`否` / `是`，说明：
- 数值权威：本 PR 引入的数值域（几何/桩号/偏移/字面量/来源位置）各采用什么标量？`N/A`，原因： / 逐域声明：`域：标量（f32/f64/整数/字节），依据：`；注明 `f64 → f32` 是否仅发生在 canonical 输出边界（受检且不可在 lowering 中提前量化）；若触及 Core ↔ compiler 投影，是否仅为 integration-only（`laneflow-compiler-test-support`，#294 删除）：

## 设计依据

- 相关文档 / ADR：
- 是否需要新增 ADR 或更新 design 文档？`否` / `是`，说明：
- 若需要 G1，冻结后的设计输入在哪里？

## 验证

列出实际运行的命令或手工检查。若某项相关检查未运行，请说明原因。

- Markdown / 文档检查：
- 构建：
- 单元测试：
- Schema / 数据格式校验：
- Dependency policy / cargo-deny：
- Adapter 验证：
- 示例 smoke test：

## 外部审阅

- Rollout phase：`R0` / `R1` / `R2`
- Current head：
- Reviewer provider / actor：
- Reviewed head / outcome / completion / evidence：
- Findings disposition / clean re-review：
- Review threads：`unresolved = <count>`，证据：
- External Review Gate：Check URL；R0/R1 尚未启用时写明 bootstrap 状态和缺失项：
- G3 Evidence Gate Shadow：使用唯一且非空的规范值：`Check URL：https://github.com/...` / `R1 non-required：<原因>` / `候选 workflow bootstrap：<边界>`：

## 风险与例外

- 已知风险：
- 例外：`N/A`，或填写类型、原因、证据、风险、批准人、到期时间和 Cleanup owner：
- 后续 Issue：

## Gate Ledger

- [ ] G3 合并判断已记录：[当前 head 的 PR G3 comment](...)。该 comment 必须在合并前形成；允许合并前纠错编辑，编辑后以经 REST 核对的 `updatedAt` 为生效时间并重新验证。comment 包含 current head、rollout phase、checks、External Review Gate、G3 Evidence Gate Shadow、结构化审阅证据、review threads、验证、风险、例外、合并方式和 Gate 断言。
- [ ] PR / Issue permalink 均已更新后，已新增精确 `g3-evidence: changed` 顶层 PR comment 并等待 trusted `G3 Evidence Gate Shadow` 重读；marker 必须未编辑，严格晚于 current G3 comment 的生效时间、当前 PR 与全部关联 Issue body 的最后编辑时间，并晚于关联 Issue 的 close/reopen 及 full-set 全部 PR 的 comment/review/commit/lifecycle timeline 活动。只有该新建 marker 可首次为直接目标发布 success；G3 comment 的任何后续编辑都要求新 marker。仅精确 `dependabot-cargo-lock-only-v1` 的 Dependabot 自主 body edit 可在完整 trusted replay 后重用受影响 target 各自最近的 marker，其他 conversation/review/thread/metadata/manual 活动仍要求新增 marker；候选 workflow 尚未合入 `main` 时记录 bootstrap 边界。
- G4 回写：Delivery PR 在关联 Issue 的 G4 comment 发表后填入 permalink；Related PR 填 `N/A` 并说明不承担 Issue G4。

<!--
G3 comment 模板（合并前发表）：

## G3 合并判断

- Gate 结果：`G3 Pass` / `G3 Waived` / `R0-R1 bootstrap`
- Rollout phase：`R0` / `R1` / `R2`
- Current head：
- Checks：
- External Review Gate：
- G3 Evidence Gate Shadow：`Check URL：https://github.com/...` / `R1 non-required：<原因>` / `候选 workflow bootstrap：<边界>`：
- 审阅：provider、actor、reviewed head、outcome、completion、evidence URL：
- Findings disposition / clean re-review：
- Review threads：`unresolved = <count>`，证据：
- R1 thread-state signal：每批 resolve / unresolve 后新增顶层 `external-review: thread-state-changed` comment；R2 写 GitHub App webhook receipt：
- R0 bootstrap 工具边界：Related PR B 自身不得用候选 validator 自批；Related PR C 自身不得用尚未合入 default branch 的候选 shadow workflow 自批。PR B 合入后记录 live `check-external-review` 结果；PR C 合入后才记录首次 trusted-ref Check 与 R1 起点。G3 Owner 已人工核验本阶段仍未由 required Check 覆盖的字段：
- 验证：
- 风险：
- 例外：
- 合并方式：
- Gate 断言：`<与实际运行完全一致的 check-gate-evidence g3 Related-only 或 full-set 规范命令>` 已通过。

每个 Related PR 都使用 `cargo +1.96.0 run --locked -p xtask -- check-gate-evidence g3 --repo <owner/repo> --issue <number> --related-pr <current-related-pr>` 并永久保留该 Related-only 断言；Delivery PR / full-set 使用 `--delivery-pr <number>` 并传入全部 Related PR，不要求把历史 Related comment 改写为 full-set 命令。一个 PR 关联多个 Issue 时，在同一 current G3 comment 中为每个 Issue 分别写一条精确 `Gate 断言`；完整断言命令集合必须与全部声明 Issue target 精确相等，不得夹带未验证的额外命令。填写与实际参数完全一致的命令后立即逐条运行；若失败，必须移除对应“已通过”并修复证据。Related PR B 合入前，该命令只校验 legacy comment 结构、permalink、PR 关系和时序，不能替代对 current head 与外部审阅字段的人工核验。

如 Gate 结果为 `G3 Waived`，还必须按 `docs/governance/development-gates.md` 写入 `external-review-waiver:v1` 结构化记录；单 Issue 使用一条，多 Issue 为每个关联 Issue 分别写入一条具有唯一 `followUpIssue` 的记录，完整 record set 与 `关联 Issue` 精确一致。evidence 使用可见 `- 例外：` 行和文末 reference-style 定义，不在 JSON 中直接写 URL；当前 target 或其 Delivery full-set 任一成员使用有期限 waiver 时，Shadow 只发布 non-success。
-->

## 完成边界

- [ ] 已覆盖关联 Issue 的验收标准，或剩余范围已拆成后续 Issue。
- [ ] 关联 Issue 的 GitHub 元数据 / 依赖关系审计已完成。
- [ ] 文档已更新，或本 PR 已说明为何无需更新。
- [ ] 当前 head 已有一个有效外部 reviewer 的完成态审阅；若曾有 findings，处置后已有新的 current-head clean re-review。仅当 PR 精确满足 `dependabot-cargo-lock-only-v1` 时，可改为记录 `development-gates.md` 定义的机器 completion。PR author 自审未计入外部 reviewer。
- [ ] `unresolved actionable threads = 0`；不受信任 actor 的自由文本或 thread 不获得阻断权，且未把该条件当作外部审阅完成的替代证据。
- [ ] PR commits 符合 `docs/reference/commit-convention.md`（Conventional Commits 标题 + `Gate: G3 Candidate` + 其他 LaneFlow 治理字段）；合并范围内没有 `Gate: G3 Block`。
- [ ] commit message footer 与 PR body 语义已区分：commit 通常使用 `Refs: #<issue>`，PR body 使用 `Closes/Resolves` 建立 Development 关联。
- [ ] 本 PR 未在只完成子切片的情况下声称父任务已完成。
- [ ] 合并方式：默认 **Rebase and merge**；若使用 Squash / Merge commit，已在 PR 中说明原因。
- [ ] 未把 G0-G3 首次记录推迟到 G4 清场阶段；若存在补救记录，已说明流程遗漏原因。
