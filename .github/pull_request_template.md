# PR 治理检查清单

## 范围

- 关联 Issue：
- PR 角色：`Delivery PR` / `Related PR`
- Development 关联：
  - Delivery PR：唯一可完成关联 Issue 验收边界的 PR，使用 `Closes #<issue>` / `Resolves #<issue>`，并由 `closingIssuesReferences` 覆盖目标 Issue。
  - Related PR：部分交付，使用 `Refs: #<issue>`；不得以 closing keyword 覆盖目标 Issue。
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
- [ ] Delivery PR / Related PRs 已在 Issue 中准确记录；若本 PR 是 Delivery PR，`closingIssuesReferences` 已覆盖关联 Issue；若本 PR 是 Related PR，已记录 `Refs: #<issue>` 且没有误用 closing keyword。

## 影响

- Core API 影响：`无` / 说明：
- 数据格式影响：`无` / 说明：
- Adapter API 影响：`无` / 说明：
- 示例或演示影响：`无` / 说明：
- 依赖 / 许可证影响：`无` / 说明依赖名称、用途、来源、许可证和分发影响：
- 破坏性变更：`否` / `是`，说明：

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

## 风险与例外

- 已知风险：
- 例外：
- 后续 Issue：

## Gate Ledger

- [ ] G3 合并判断已记录：[PR G3 comment](...)。该 comment 必须在合并前发表，并包含 checks、审阅、验证、风险、例外、合并方式和 Gate 断言。
- G4 回写：Delivery PR 在关联 Issue 的 G4 comment 发表后填入 permalink；Related PR 填 `N/A` 并说明不承担 Issue G4。

<!--
G3 comment 模板（合并前发表）：

## G3 合并判断

- Checks：
- 审阅：
- 验证：
- 风险：
- 例外：
- 合并方式：
- Gate 断言：`cargo +1.96.0 run --locked -p xtask -- check-gate-evidence g3 --repo <owner/repo> --issue <number> --delivery-pr <number> [--related-pr <number>]...` 已通过。

填写与实际参数完全一致的命令后立即运行；若失败，必须移除“已通过”并修复证据。
-->

## 完成边界

- [ ] 已覆盖关联 Issue 的验收标准，或剩余范围已拆成后续 Issue。
- [ ] 关联 Issue 的 GitHub 元数据 / 依赖关系审计已完成。
- [ ] 文档已更新，或本 PR 已说明为何无需更新。
- [ ] PR commits 符合 `docs/reference/commit-convention.md`（Conventional Commits 标题 + LaneFlow 治理字段），或已记录显式例外。
- [ ] commit message footer 与 PR body 语义已区分：commit 通常使用 `Refs: #<issue>`，PR body 使用 `Closes/Resolves` 建立 Development 关联。
- [ ] 本 PR 未在只完成子切片的情况下声称父任务已完成。
- [ ] 合并方式：默认 **Rebase and merge**；若使用 Squash / Merge commit，已在 PR 中说明原因。
- [ ] 未把 G0-G3 首次记录推迟到 G4 清场阶段；若存在补救记录，已说明流程遗漏原因。
