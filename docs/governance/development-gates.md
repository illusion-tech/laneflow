# 开发闸口

**文档状态**: Active  
**最后更新**: 2026-07-14  
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
- G3 的完整事件证据记录在每个 PR 的 `## G3 合并判断` comment，且必须在该 PR 合并前创建；PR body 的 G3 checkbox 和 Issue body 的 G3 checkbox 都只保存直接 comment permalink。
- G4 的完整事件证据记录在 Issue 的 `## G4 完成判断` comment，且必须在所有关联 PR 合并后、Issue 关闭前创建；Issue body 的 G4 checkbox 保存直接 comment permalink。Delivery PR 的 body 只回链该 Issue G4 comment，Related PR 不承担 Issue G4。
- GitHub comment 是带时间和作者的过程证据，不是不可变审计日志；长期规则仍由仓库文档和 Git 历史保存。
- G3 / G4 前必须运行 `cargo +1.96.0 run --locked -p xtask -- check-gate-evidence <g3|g4> --repo <owner/repo> --issue <number> --delivery-pr <number> [--related-pr <number>]...`。`Gate 断言` 行必须用反引号记录与本次参数完全一致的规范命令，并在命令后明确写 `已通过`；`待运行`、缺少成功标记或参数不匹配均视为 Gate 失败。命令或远端读取失败同样是 Gate 失败。
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
- 源代码许可证、依赖许可证、RustSec advisory、crate 来源或 Dependabot 配置违反 `dependency-security.md`，或适用 cargo-deny 检查未通过。
- `security-scanning.md` 要求的适用扫描仍为 `pending`、失败、无分析、已禁用或不可用，且没有记录显式例外。

PR 合入 `main` 默认使用 **Rebase and merge**；若使用 Squash 或 Merge commit，须在 PR 中说明原因。详见 `github-workflow.md` 第 7 节。

G3 记录必须写在 PR 的 `## G3 合并判断` comment 中，至少包含 `Checks`、审阅、验证、风险、例外、合并方式和 `Gate 断言`。PR body 的 G3 checkbox、Issue body 的 G3 checkbox 必须勾选并回链同一 Delivery PR comment；Related PR 如有均必须逐条回链。`Gate 断言` 必须采用下方规范行，命令参数与实际调用完全一致；填写后立即运行该命令，若失败必须移除 `已通过` 并修复证据。运行成功前不得合并。

```text
## G3 合并判断

- Checks：
- 审阅：
- 验证：
- 风险：
- 例外：
- 合并方式：Rebase and merge / 例外原因
- Gate 断言：`cargo +1.96.0 run --locked -p xtask -- check-gate-evidence g3 --repo <owner/repo> --issue <number> --delivery-pr <number> [--related-pr <number>]...` 已通过。
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
