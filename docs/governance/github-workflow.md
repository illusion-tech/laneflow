# GitHub 工作流

**文档状态**: Active  
**最后更新**: 2026-07-14  
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
- `v0.6 First Adapter`
- `v1.0 Stable Runtime API`

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
- 在 PR comment 记录 `## G3 合并判断`：checks、审阅、验证、风险、例外、合并方式和 Gate 断言；PR body 与 Issue body 的 G3 checkbox 必须回链该 comment。

不得用父任务标题合入只覆盖部分能力的实现。部分交付必须明确子切片边界。

分支不是长期 Development 关系证据。PR 创建后，应在 Issue 的 Delivery PR 或 Related PRs 字段记录 `PR-number`。唯一的 Delivery PR 通过 PR body 的 GitHub closing keyword 建立 Development 关联；Related PR 使用 `Refs: #<issue>`。若 Delivery PR 无法关联，必须在 PR 中说明原因并保留可追踪链接。

Delivery PR / Related PRs 关联规则：

- 仓库设置 `Auto-close issues with merged linked pull requests` 应保持关闭；Issue 关闭仍由 G4 清场手动完成。
- 当 PR 预期覆盖关联 Issue 的完成边界时，它是唯一 Delivery PR，body 应使用 `Closes #<issue>`、`Resolves #<issue>` 或等价 GitHub closing keyword 建立 Development 关联。
- 当 PR 只是父 Issue 的子切片或部分交付时，它是 Related PR，不得误用 closing keyword；应使用 `Refs: #<issue>`，并在 Issue 中列出该 PR。
- commit message footer 与 PR body 语义分开：commit message 通常继续使用 `Refs: #<issue>`，不得为了建立 Development 关联而把提交 footer 改成 `Closes`。
- G3 前默认必须通过 `gh pr view <delivery-pr> --json closingIssuesReferences` 确认 Delivery PR 覆盖目标 Issue；GitHub Development 面板只作为人工辅助证据。必须在合并前发表 PR G3 comment，并运行 `cargo +1.96.0 run --locked -p xtask -- check-gate-evidence g3 ...` 验证 permalink、comment 字段、Delivery / Related 关系与时序。G3 comment 的 `Gate 断言` 行必须包含与实际调用参数完全一致的反引号命令，并在命令后写 `已通过`；pending、缺少结果或参数不匹配均不能进入 `G3 = Pass`。若 Delivery PR、父 Issue 子切片、权限或平台限制导致只能手动关联 Development 面板，必须记录显式例外，说明原因、风险、后续收口方式和 Cleanup owner；否则不能进入 `G3 = Pass`。

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

例外（须在 PR 或 Issue 中说明原因）：

- **Squash and merge**：PR 内含多个无独立意义的 wip commit，或明确要求 `main` 上 1 个 PR 对应 1 个 commit。
- **Create a merge commit**：发布分支、长期分支合流等需要保留 merge 节点的场景。

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

GitHub CodeQL、Secret Scanning 和 Dependabot 属于平台安全检查，其配置、状态语义和阻断规则见 `security-scanning.md`。GitHub 为当前 PR 产生的适用 CodeQL check 必须在 G3 前完成；缺失预期分析、失败或平台不可用不能解释为通过。

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

如果 G4 阶段发现 G0-G3 没有按时记录，只能追加“补救记录”，并说明流程遗漏原因。补救记录不能作为后续任务的标准流程。
