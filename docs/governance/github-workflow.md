# GitHub 工作流

**文档状态**: Active
**最后更新**: 2026-08-28

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
见 [ADR 0026](../adr/0026-merge-governance-rebuild.md) 与
[ADR 0027](../adr/0027-retire-external-review-check.md)。

## 2. Issue 规则

所有可执行开发任务应先有 Issue。

Issue 的结构化字段各自只表达一个维度：

| 维度                | 权威位置                                  | 用途                                        |
| ------------------- | ----------------------------------------- | ------------------------------------------- |
| 工作性质            | GitHub 原生 Issue Type                    | 这是功能、缺陷还是任务                      |
| 影响范围 / 工作方式 | Labels                                    | 影响哪些领域、采用什么工作方式              |
| 当前阶段            | Project Status                            | 任务处于 Backlog、Ready、实施、评审还是终态 |
| 交付目标            | Milestone                                 | 任务是否属于一个已排期的版本或具名交付      |
| 任务关系            | parent / sub-issue、blocked-by / blocking | 分解层级与真实依赖                          |

标题只写简短、可验证的中文结果陈述；精确标识符保留原文。禁止用 `[功能]`、`[Core]`、
`[数据规范]` 等方括号前缀重复分类，也不得让机器解析标题、正文或评论来恢复元数据。

Issue 应至少说明：

- 背景
- 目标
- 非目标
- 验收标准
- 影响范围
- 关联文档

不要在正文抄 Project / Milestone / parent / blocked-by。那些字段在 GitHub 侧边栏。

### 2.1 原生 Issue Type

LaneFlow 使用组织已启用的三个 GitHub 原生类型；英文名称是平台精确标识符，不翻译或
复制成同义标签：

| Type      | 选择标准                                                 |
| --------- | -------------------------------------------------------- |
| `Feature` | 新增产品能力，或有意改变现行行为、API、数据契约          |
| `Bug`     | 当前权威契约与实际行为不一致                             |
| `Task`    | 设计、调研、文档、治理、重构、验证、依赖、发布或清理工作 |

判定顺序：非预期行为优先 `Bug`；设计、调研、文档、治理使用对应专用模板并归为
`Task`；新增 Traffic Runtime、数据规范或 Adapter 能力使用专用模板并归为 `Feature`；
其余使用通用功能或任务模板。不得为 LaneFlow 单独新增组织级 Issue Type，除非另开
治理 Issue 评估对组织内全部仓库的影响。

### 2.2 Issue Forms

仓库默认关闭 blank issue。`.github/ISSUE_TEMPLATE/` 提供：

| 表单       | Type      | 自动标签          |
| ---------- | --------- | ----------------- |
| 功能       | `Feature` | 无                |
| 缺陷       | `Bug`     | 无                |
| 任务       | `Task`    | 无                |
| 设计       | `Task`    | `work:design`     |
| 调研       | `Task`    | `work:research`   |
| 交通运行时 | `Feature` | `area:runtime`    |
| 数据规范   | `Feature` | `area:data-spec`  |
| 适配器     | `Feature` | `area:adapter`    |
| 文档       | `Task`    | `area:docs`       |
| 治理       | `Task`    | `area:governance` |

高影响变更在实现前冻结 ADR / design。文档、缺陷、调研不默认要求 G1，但实际影响达到
G1 条件时仍必须补齐设计判断。

### 2.3 Labels

Labels 不重复 Type 或 Project Status。Issue 进入 `Ready` 前至少有一个 `area:*`；跨层
任务同时使用多个 area 标签，不创建 `cross-layer` 标签。

领域标签：

- `area:runtime`
- `area:compiler`
- `area:static-network`
- `area:spatial`
- `area:scenario`
- `area:data-spec`
- `area:adapter`
- `area:authoring`
- `area:example`
- `area:docs`
- `area:governance`
- `area:ci`
- `area:release`

工作方式标签：`work:design`、`work:research`、`work:verification`、`work:security`。
`dependencies`、`github_actions`、`rust` 只作为 Dependabot / PR 集成元数据，不是 Issue
必填分类。完成 Type 迁移后，Issue 不再使用 `feature`、`bug`、`enhancement`、
`documentation` 表达工作性质。

由于 PR 没有原生 Issue Type，已有 `feature` / `bug` PR 标签暂时保留，但不得再添加到
Issue；本次迁移只从 Issue 移除它们。`enhancement` / `documentation` 在确认没有 PR
使用后删除。

既有标签按下列单一映射迁移：`core-runtime` → `area:runtime`、`data-spec` →
`area:data-spec`、`adapter` → `area:adapter`、`governance` → `area:governance`、
`docs` → `area:docs`、`design` → `work:design`、`research` → `work:research`。

### 2.4 关系与交付目标

- parent / sub-issue 表达任务分解；父任务本身不自动阻塞子任务。
- blocked-by / blocking 只表达阻止当前 Gate 前进的真实依赖；不要用状态标签代替。
- Milestone 表达已排期的版本或具名交付。Backlog、调研与尚未排期的工作可以没有
  Milestone；承诺进入具体交付的工作必须设置。

默认一个 Issue 一个完成它的 PR。PR body 使用 `Closes #<issue>`。仓库打开
linked PR 合并后自动关闭 Issue。未完成验收时拆 follow-up Issue。

## 3. Project 规则

GitHub Project 用于管理当前状态。推荐列：

- `Backlog`：G0 未通过，或尚未排期。
- `Ready`：G0 完成；G1 已接受或不适用；至少有一个 `area:*`；没有开放 blocker。
- `In Progress`：G2 已记录，且已有 assignee。
- `In Review`：用于关闭 Issue 的 PR 已打开。
- `Done`：Issue 以 `Completed` 关闭。
- `Cancelled`：Issue 以 `Not planned` 或 Duplicate 关闭。

Project 不设置 `Blocked` 状态；阻塞由 GitHub 原生 blocked-by 关系实时派生。内置字段
启用 `Type`，自定义单选字段使用：

- `Priority`：`P0`、`P1`、`P2`、`P3`。
- `Design gate`：`N/A`、`Required`、`Accepted`。

Priority 含义：`P0` 是需要立即处理的安全、数据损坏、主干或发布阻断；`P1` 是当前
Milestone 关键路径；`P2` 是已排期但非关键路径；`P3` 是机会性 Backlog。没有排期
判断时字段可以暂空，不能用 Priority 代替 Status 或 Milestone。

LaneFlow Project 的规范工作项是 Issue。自动添加只匹配
`repo:illusion-tech/laneflow is:issue`；PR 通过 Issue 的 Linked pull requests 关联，不再
把新 PR 作为独立状态卡。历史 PR 卡片可分批归档，不在迁移中突然删除。

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
- 说明是否影响 Traffic Runtime API、数据格式、Adapter 协议或依赖。
- 记录测试、构建和文档检查结果。
- 记录已知风险。

不得用父任务标题合入只覆盖部分能力的实现。commit footer 使用 `Refs: #<issue>`，
不要为了关单把每个 commit 写成 `Closes`。

普通 GitHub Review 继续用于协作，但不由自定义 Check、受信名单或 reaction 计数。
未解决对话由 GitHub 原生规则拦截。不要在 PR 里抄 SHA 或 thread 计数。

fork / cross-repository PR 必须把最终 patchset 迁到同仓 PR。

### Copilot repository instructions

`.github/copilot-instructions.md` 只作为提示层，必须保持薄包装，转读 `AGENTS.md`、
`.agents/` 和 `docs/`。它不能替代 CI、Checks 或 GitHub 实时元数据。

## 7. PR 合并策略

LaneFlow 默认通过合并队列（Merge Queue）将 PR 合入 `main`，队列最终使用 **Rebase**。

原因：

- 保持 `main` 历史线性、清晰。
- 保留 PR 内各 commit 的 Conventional Commits 标题。
- 在最新 `main` 与队列前序变更的真实组合上重新运行 required checks。
- 避免只因 `main` 前进而要求其他 PR 手工 rebase。

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

required status checks 固定为 `Commit message`、`Rust checks`、
`Dependency policy`、`Analyze (actions)`、`Analyze (rust)`，PR 与 `merge_group`
同名。五项机器检查 expected source 绑定 GitHub Actions App
`integration_id=15368`。原生 CodeQL rule 不能替代 `H_mg` 上的两个 `Analyze`。

### 7.1 日常入队与失效边界

- PR Head `H_pr` 是补丁身份，绑定 PR 级机器检查与 GitHub 原生协作状态。
- 合并组 Head `H_mg` 是集成候选身份，绑定 CI、依赖政策和适用安全扫描。
- Main Result `H_main` 是 GitHub 执行 rebase 后进入 `main` 的结果。

失效边界：PR 新 push、force-push 或冲突修复产生新 `H_pr` 时，入队资格与机器检查
按新 head 重跑。`main` 前进只废弃旧 `H_mg` 并重跑机器检查。未解决的 review
conversation 始终由 GitHub 原生规则判断。

```powershell
gh pr merge <number> --repo illusion-tech/laneflow --match-head-commit <H_pr>
```

不得在 pending 时预先武装 auto-merge，禁止日常 `--admin`。owner bypass 的终态由
#493 独立治理；永久 bypass 不得被解释为 checks 已通过。

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
  static-contract、static-network、compiler 或 `examples/data/` 变更时编译。
- `Dependency policy`：cargo-deny。
- `Analyze (actions)` / `Analyze (rust)`：advanced CodeQL。

GitHub CodeQL、Secret Scanning 和 Dependabot 见 `security-scanning.md`。

## 9. Release 规则

每次 Release 应说明版本目标、新增能力、修复、breaking changes、Traffic Runtime API 版本、
数据格式版本、Adapter 兼容情况和示例项目状态。公开发布前按 `security-scanning.md`
与 `dependency-security.md` 重新验证。
