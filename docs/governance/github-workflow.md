# GitHub 工作流

**文档状态**: Active  
**最后更新**: 2026-06-18  
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
- Gate Ledger：G0/G1/G2 在 Issue 阶段增量记录，G3/G4 后续由 PR 和收口流程回写

推荐 Issue 类型（与 `.github/ISSUE_TEMPLATE/` 对应）：

- `功能`（Feature）：新增能力
- `缺陷`（Bug）：缺陷修复
- `设计`（Design）：设计收口或架构决策准备
- `Core`：LaneFlow Core 运行时变更
- `数据规范`（Data Spec）：数据格式、schema 或序列化变更
- `适配器`（Adapter）：引擎适配层变更
- `文档`（Docs）：文档与治理变更
- `调研`（Research）：尚未确定是否实现的探索

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
- `Ready`：G0 已记录；需要 G1 的任务已完成 G1，不需要 G1 的任务已记录不适用原因。
- `In Progress`：G2 已记录，任务已经进入实现或文档修改。
- `In Review`：已有 PR 或审查材料，G3 判断应在 PR 中维护。
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
- 记录测试、构建和文档检查结果。
- 记录已知风险和例外。
- 记录或链接 G3 合并判断：checks、review、验证、风险、例外和合并方式。

不得用父任务标题合入只覆盖部分能力的实现。部分交付必须明确子切片边界。

## 7. PR 合并策略

LaneFlow 默认使用 **Rebase and merge** 将 PR 合入 `main`。

原因：

- 保持 `main` 历史线性、清晰。
- 保留 PR 内各 commit 的治理说明（`Gate`、`Type`、`Impact` 等）。
- 避免为常规功能 PR 增加多余的 merge commit 节点。

默认规则：

- 常规功能、修复、文档、治理 PR → **Rebase and merge**。
- PR 内 commit 已具备独立意义且 message 符合 `docs/reference/commit-convention.md` → **Rebase and merge**。

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

- 仓库中关键文档文件存在。
- Markdown 文件非空。
- 后续根据 CI 能力和实际技术栈增加 Markdown/YAML 语法检查、build、test、lint、schema validation 和 example smoke test。

当 Core、data spec 或 Adapter 代码出现后，应逐步增加专用门禁。

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

## 10. 合并后 G4 清场流程

PR 合并后，应回到关联 Issue 完成 G4，而不是在清场时首次补写 G0-G3。

G4 清场必须完成：

- 确认 PR 已按默认策略 Rebase and merge 合入，或记录例外原因。
- 勾选 Issue 验收 checklist。
- 在 Issue Gate Ledger 中补充 G4 记录。
- 将 Project 中关联 Issue 和 PR 移动到 `Done`。
- 删除远端 PR 分支并 prune 本地 remote-tracking 分支。
- 切回并更新本地 `main`。
- 撤回临时 ruleset bypass、admin override 或其他临时权限；若不能撤回，记录保留原因、风险和 cleanup owner。

如果 G4 阶段发现 G0-G3 没有按时记录，只能追加“补救记录”，并说明流程遗漏原因。补救记录不能作为后续任务的标准流程。
