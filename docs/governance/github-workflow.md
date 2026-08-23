# GitHub 工作流

**文档状态**: Active
**最后更新**: 2026-08-23

**适用范围**: LaneFlow 的 Issue、PR、Project、Milestone、Release 和 CI 治理

## 1. 工作流原则

LaneFlow 采用 GitHub-first 治理：

- Issue 是任务入口。
- Pull Request 是合并审查单元。
- Project 是当前进度看板。
- Milestone 是版本目标容器。
- Actions 是自动化质量门禁。
- Releases 是发布事实记录。

长期设计、架构决策和规范必须进入仓库文档，不应只留在 GitHub 页面中。合并证据模型
见 [ADR 0026](../adr/0026-merge-governance-rebuild.md)。

## 2. Issue 规则

所有可执行开发任务应先有 Issue。

Issue 应至少说明：

- 背景
- 目标
- 非目标
- 验收标准
- 影响范围
- 关联文档

不要在正文抄 Project / Milestone / parent / blocked-by。那些字段在 GitHub 侧边栏。

推荐 Issue 类型（与 `.github/ISSUE_TEMPLATE/` 对应）：

- `功能`（Feature）
- `缺陷`（Bug）
- `设计`（Design）
- `Core`
- `数据规范`（Data Spec）
- `适配器`（Adapter）
- `文档`（Docs）
- `调研`（Research）

仓库默认关闭 blank issue。高影响变更使用设计 / Core / 数据规范 / 适配器模板中的
可选设计链接。文档 / 缺陷 / 调研不强制 G1 勾选。

默认一个 Issue 一个完成它的 PR。PR body 使用 `Closes #<issue>`。仓库打开
linked PR 合并后自动关闭 Issue。未完成验收时拆 follow-up Issue。

## 3. Project 规则

GitHub Project 用于管理当前状态。推荐列：

- `Backlog`
- `Ready`
- `In Progress`
- `In Review`
- `Blocked`
- `Done`

`Done` = PR 已合且 Issue 已关。

## 4. Milestone 规则

Milestone 用于表达版本边界，而不是单个大任务。每个 Milestone 应有明确的完成定义，
并由一组 Issue 组成。

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

- 关联一个 Issue，body 使用 `Closes #<issue>`。
- 明确本次变更范围和明确不做范围。
- 说明是否影响 Core API、数据格式、Adapter 协议或依赖。
- 记录测试、构建和文档检查结果。
- 记录已知风险。

不得用父任务标题合入只覆盖部分能力的实现。commit footer 使用 `Refs: #<issue>`，
不要为了关单把每个 commit 写成 `Closes`。

外部审阅由 `External Review` Check 判定：当前 head 上是否存在非作者受信原生
`PullRequestReview`。未解决对话由 GitHub 原生规则拦截。不要在 PR 里抄 SHA、
thread 计数或 Shadow 行。

fork / cross-repository PR 必须把最终 patchset 迁到同仓 PR。

### External Review workflow 安全

- metadata-only；禁止 checkout 或执行 PR head。
- 权限：`contents: read`、`pull-requests: read`、`checks: write`。
- checkout `refs/heads/main`，关闭 credential persistence。
- Check 名固定为 `External Review`。
- expected source 暂为 GitHub Actions；同名伪造残余风险见 ADR 0026。
- `pull_request_review` 只由空权限 signal workflow 唤醒 trusted publisher。
- `merge_group` 上发布同名 Check，盖章已通过的 `H_pr`，不重做人审。

修改 Gate workflow 的 PR 由 `main` 上的旧 validator 判断，不能用候选实现自批。

### Copilot repository instructions

`.github/copilot-instructions.md` 只作为提示层，必须保持薄包装，转读 `AGENTS.md`、
`.agents/` 和 `docs/`。它不能替代 CI、Checks 或 GitHub 实时元数据。

## 7. PR 合并策略

LaneFlow 默认通过合并队列（Merge Queue）将 PR 合入 `main`，队列最终使用 **Rebase**。

原因：

- 保持 `main` 历史线性、清晰。
- 保留 PR 内各 commit 的 Conventional Commits 标题。
- 在最新 `main` 与队列前序变更的真实组合上重新运行 required checks。
- 避免只因 `main` 前进而要求其他 PR 手工 rebase 和重复外部审阅。

`main` 的队列配置：

| 参数                                | 值         | 目的                                  |
| ----------------------------------- | ---------- | ------------------------------------- |
| `merge_method`                      | `REBASE`   | 保持线性历史                          |
| `grouping_strategy`                 | `ALLGREEN` | 每个进入合并组的 PR 都必须满足 checks |
| `max_entries_to_build`              | `1`        | 首轮串行构建                          |
| `min_entries_to_merge`              | `1`        | 单个已就绪 PR 不等待额外成员          |
| `max_entries_to_merge`              | `1`        | 每次只把一个 PR 写入 `main`           |
| `min_entries_to_merge_wait_minutes` | `0`        | `min=1` 时无需额外等待                |
| `check_response_timeout_minutes`    | `60`       | required check 超时后失败关闭         |

启用队列时，`required_status_checks.strict_required_status_checks_policy` 必须为
`false`，但不得删除 required checks。

切换完成后 required status checks 固定为 `Commit message`、`Rust checks`、
`Dependency policy`、`Analyze (actions)`、`Analyze (rust)`、`External Review`，
PR 与 `merge_group` 同名。`External Review` 按 ADR 0026 启用顺序，先经真实队列
验证再 required。五项机器检查 expected source 绑定 GitHub Actions App
`integration_id=15368`。原生 CodeQL rule 不能替代 `H_mg` 上的两个 `Analyze`。

### 7.1 日常入队与失效边界

- PR Head `H_pr` 是补丁审阅身份，绑定原生外部审阅。
- 合并组 Head `H_mg` 是集成候选身份，绑定 CI、依赖政策和适用安全扫描；
  `External Review` 在此为盖章。
- Main Result `H_main` 是 GitHub 执行 rebase 后进入 `main` 的结果。

失效边界：PR 新 push、force-push 或冲突修复产生新 `H_pr` 时，入队资格与机器检查
按新 head 重跑。External Review 不因 `H_pr` 变化作废已有受信 Approve/Comment 或
PR 正文点赞。`main` 前进只废弃旧 `H_mg` 并重跑机器检查。

```powershell
gh pr merge <number> --repo illusion-tech/laneflow --match-head-commit <H_pr>
```

不得在 pending 时预先武装 auto-merge。验证通过并移除 bypass 之后，禁止日常
`--admin`。

最终 merge method 例外必须先通过治理 Issue 修改队列 / 分支规则：

- **Squash and merge**：PR 内多个无独立意义的 wip commit，或明确要求 1 PR = 1 commit。
- **Create a merge commit**：发布分支、长期分支合流等场景。

仓库 Settings → General → Pull Requests 必须保持 **Allow rebase merging** 启用。

## 8. CI 规则

当前检查：

- `Commit message`：Conventional Commits 标题、`Refs` / `Closes`、必要时
  `BREAKING CHANGE:`。
- `Markdown tables`：表格格式，只警告。
- `Rust checks`：`cargo fmt` 与 `cargo test --workspace --locked`。走廊 catalog 与
  LFCA 对拍由 `laneflow-corridor-generator` 测试覆盖，不再单独跑 generator `check`。
  `schemas/road-editing/` 由独立 Codegen workflow 覆盖，不因 `.fbs` 变更拉起整仓
  Rust 测试。Bevy native example 在 Adapter/Runtime/Spatial/scenario、format、
  static-contract、static-network 或 compiler fixture 变更时编译。
- `Dependency policy`：cargo-deny。
- `Analyze (actions)` / `Analyze (rust)`：advanced CodeQL。
- `External Review`：原生审阅 Check。

GitHub CodeQL、Secret Scanning 和 Dependabot 见 `security-scanning.md`。

## 9. Release 规则

每次 Release 应说明版本目标、新增能力、修复、breaking changes、Core API 版本、
数据格式版本、Adapter 兼容情况和示例项目状态。公开发布前按 `security-scanning.md`
与 `dependency-security.md` 重新验证。
