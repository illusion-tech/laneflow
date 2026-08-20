# AI Agent 开发指南

**文档状态**: Active  
**最后更新**: 2026-08-20

**适用范围**: 使用 AI Agent 参与 LaneFlow 的设计、开发、测试、文档和治理工作

## 1. 目标

本文定义 AI Agent 在 LaneFlow 中的默认开发规则，确保 Agent 不只根据单条 prompt 编码，而是基于 GitHub Issue、仓库文档和现有代码上下文工作。

## 2. 开工前必须读取

AI Agent 开工前应读取：

1. 当前 GitHub Issue 或用户任务说明。
2. `README.md`。
3. `AGENTS.md`。
4. `.agents/README.md`。
5. 与任务匹配的 `.agents/skills/<skill-name>/SKILL.md`。
6. `docs/governance/development-gates.md`。
7. 与任务相关的 `docs/design/` 文档。
8. 与任务相关的 `docs/adr/` 文档。
9. 受影响代码区域的现有实现和测试。

如果相关 design 或 ADR 不存在，但任务涉及 Core API、数据格式或 Adapter 协议，应先补设计或提出 G1 阻断。

`.cursor/skills/` 只作为 Cursor 自动发现入口。跨 Agent 的执行规范应维护在 `.agents/skills/`。

## 3. 默认工作方式

AI Agent 应遵守以下流程：

1. 确认任务类型和影响范围。
2. 审计当前 Issue 的 GitHub 元数据 / 依赖关系，包括 Project、Project status、Milestone、Labels、Parent / sub-issues、Blocked by、Blocking、Delivery PR 和 Related PRs；Delivery PR 创建后须在 G3 前确认 `closingIssuesReferences` 覆盖目标 Issue，Related PR 不得误用 closing keyword。G3 前必须取得 trusted reviewer 对当前 exact head 的完成态外部审阅；仅当 PR 精确满足 `development-gates.md` 的 `dependabot-cargo-lock-only-v1` 时，才由其机器 completion 替代 reviewer completion。PR author 自审不计入外部 reviewer，有 findings 时必须在处置后取得新的 current-head clean re-review。G3 证据写入 PR comment；合并前允许纠错编辑，未编辑时以 `createdAt`、编辑后以经 REST 核对的 `updatedAt` 作为生效时间，并重新验证全部当前证据。G4 证据写入 Issue comment，body checkbox 只回链对应 permalink；`Gate 断言` 行必须包含与实际参数完全一致的反引号命令并明确写 `已通过`，pending、缺少结果、参数不匹配或运行失败均不得推进对应 Gate。若只能手动关联 GitHub Development 面板，必须记录显式例外；不适用项必须有 `N/A` 原因，无法设置的必需元数据必须记录显式例外。
3. 检查是否需要 ADR 或 design 文档。
4. 读取现有代码和测试。
5. 制定小范围实现计划。
6. 修改代码或文档。
7. 运行与变更匹配的检查。
8. 在 PR 或交付说明中记录测试、风险和未覆盖范围。

提交信息应遵守 `docs/reference/commit-convention.md`，标题使用 Conventional Commits，正文必须包含 `Gate`、`Slice`、`Impact`、`Scope`、`Validation`、`Docs`，底部 footer 必须包含 `Refs` 或 `Closes`。新提交的本地 `Gate` 使用 `G3 Candidate` 或 `G3 Block`，但 PR / push range 不得包含 `G3 Block`；正式 `G3 Pass` 只存在于 PR Check 和当前 head 的 G3 Owner comment。

修改 Rust 代码时还应读取 `docs/reference/rust-code-style.md`。其中无法由 `rustfmt` 表达的规则只约束本次触及范围；不得在无关功能 PR 中顺带重排历史格式。

## 4. Core 开发规则

涉及 LaneFlow Core 时，Agent 必须注意：

- Core 不应依赖 Unity、Unreal、Godot、O3DE、WebGL、DOM 或任何具体引擎 API。
- Core 应以确定性 runtime 行为为优先目标。
- Core API 变更需要同步更新 design 文档。
- 破坏性 Core API 变更需要 ADR 或明确的 design 决策。
- 路线、车道图、信号灯、避让、停车等核心规则应尽量有单元测试。

## 5. Data Spec 开发规则

涉及数据格式时，Agent 必须注意：

- 明确数据格式版本影响。
- 明确是否兼容旧数据。
- 更新 `docs/design/data-format.md` 或对应专题文档。
- 更新示例数据或记录不更新原因。
- 避免在代码中隐式定义未文档化的数据语义。

## 6. Adapter 开发规则

涉及引擎适配器时，Agent 必须注意：

- Adapter 可以依赖引擎，Core 不可以。
- Adapter 负责展示、模型、动画、LOD、调试可视化和引擎生命周期对接。
- Adapter 不应复制 Core 的交通规则。
- Adapter 协议变化需要更新 `docs/design/adapter-api.md`。
- 新增 Adapter 时应提供最小示例或手工验证说明。

## 7. 文档开发规则

Agent 修改文档时应注意：

- 长期设计结论写入 `docs/adr/` 或 `docs/design/`。
- 当前任务状态写入 GitHub Issue，不写入长期设计文档。
- PR 验证和本次风险写入 PR，不写入 ADR。
- 目录入口文档应保持简短，负责导航而不是复制全部内容。

## 8. 测试与验证规则

Agent 应根据切片类型选择验证：

- `docs-only`：Markdown 和链接基本检查。
- `governance`：模板、路径、引用和 Issue 元数据 / 依赖关系审计一致性检查；涉及许可证或依赖时运行 cargo-deny 并复核 GitHub Dependabot 状态。
- `core-runtime`：单元测试、确定性行为测试。
- `data-spec`：schema validation、示例数据检查。
- `adapter`：adapter build、手工运行说明或截图。
- `cross-layer`：相关测试加示例 smoke test。

如果检查无法运行，必须说明原因和剩余风险。

## 9. 禁止事项

AI Agent 不应：

- 未读相关文档就修改 Core API。
- 把引擎依赖引入 Core。
- 在一个 PR 中混合无关重构和功能开发。
- 修改数据格式但不更新文档。
- 声称父任务完成但只交付子范围。
- 用“测试未运行”结束而不说明原因。
- 为了兼容未发布的分支变更叠加无意义 shim。

## 10. PR 交付说明要求

Agent 完成工作后，PR 或最终说明至少应包含：

- 变更摘要
- 影响范围
- 已运行检查
- 未运行检查及原因
- 文档更新情况
- 已知风险
- 当前 head 的外部审阅、findings disposition / clean re-review 与 External Review Gate 证据
- 后续 Issue 或留白

## 11. PR 合并策略

Agent 协助合并 PR 时，先冻结 current exact head `H_pr`，确认该 head 的外部审阅、finding disposition、
G3 Owner comment、marker、PR 级 required checks 与适用 CodeQL 均已完成且有效，再把 PR 加入 `main` 的
合并队列；不得在 pending 时预先武装 auto-merge：

```powershell
gh pr merge <number> --repo illusion-tech/laneflow --match-head-commit <H_pr>
```

合并队列的 live 规则控制最终使用 Rebase；Agent 不再为日常入队选择 merge strategy。`main` 前进或队列
重排只重建 `H_mg` 并重跑机器检查，不要求对未变化 `H_pr` 重新人审；任何 push、force-push 或冲突修复
改变 `H_pr` 后，旧审阅、G3 与入队资格全部 stale。禁止使用 `--admin` 绕过队列。规则与例外详见
`docs/governance/github-workflow.md` 第 7 节。

Agent 必须按 GitHub API / Checks 精确核对 `H_pr` 与真实 `H_mg` 的 `Governance checks`、`Rust checks`、
`Dependency policy`、`Analyze (actions)`、`Analyze (rust)`；CodeQL 两项还须来自 GitHub Actions App
`integration_id=15368` 并在 `H_mg` 上形成适用 analysis。不得用 PR Head、合并后的 `main` 或原生
CodeQL rule 的旧结果补足 Merge Group。
