# PR 治理检查清单

## 范围

- 关联 Issue：
- Development 关联（用于 GitHub Development 面板）：
  - 完成交付：`Closes #<issue>` / `Resolves #<issue>`
  - 部分交付：`Refs #<issue>`，例外原因：
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
- [ ] Development PR 已在 Issue 中记录为本 PR，且 GitHub Development 面板 / `closingIssuesReferences` 已关联；若缺失，已记录显式例外。

## 影响

- Core API 影响：`无` / 说明：
- 数据格式影响：`无` / 说明：
- Adapter API 影响：`无` / 说明：
- 示例或演示影响：`无` / 说明：
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
- Adapter 验证：
- 示例 smoke test：

## 风险与例外

- 已知风险：
- 例外：
- 后续 Issue：

## Gate Ledger

- [ ] G3 合并判断已记录（checks、review、验证、风险、例外和合并方式）。
- [ ] G4 完成判断将在合并后回写关联 Issue（验收 checklist、Project 状态移至 `Done`、分支清理、临时权限撤回）。

## 完成边界

- [ ] 已覆盖关联 Issue 的验收标准，或剩余范围已拆成后续 Issue。
- [ ] 关联 Issue 的 GitHub 元数据 / 依赖关系审计已完成。
- [ ] 文档已更新，或本 PR 已说明为何无需更新。
- [ ] PR commits 符合 `docs/reference/commit-convention.md`（Conventional Commits 标题 + LaneFlow 治理字段），或已记录显式例外。
- [ ] commit message footer 与 PR body 语义已区分：commit 通常使用 `Refs: #<issue>`，PR body 使用 `Closes/Resolves` 建立 Development 关联。
- [ ] 本 PR 未在只完成子切片的情况下声称父任务已完成。
- [ ] 合并方式：默认 **Rebase and merge**；若使用 Squash / Merge commit，已在 PR 中说明原因。
- [ ] 未把 G0-G3 首次记录推迟到 G4 清场阶段；若存在补救记录，已说明流程遗漏原因。
