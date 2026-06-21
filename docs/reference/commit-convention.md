# 提交规范

**文档状态**: Active<br>
**最后更新**: 2026-06-21<br>
**适用范围**: LaneFlow 的本地提交、AI Agent 提交说明、PR commit 审查和无需完整 PR 审查的小切片留痕

## 1. 目标

LaneFlow 的提交信息以 [Conventional Commits 1.0.0](https://www.conventionalcommits.org/zh-hans/v1.0.0/) 为标题基础，并在正文保留 LaneFlow 治理字段。

提交信息必须同时回答两类问题：

- 生态工具能否识别本次变更类型、scope 和 breaking change。
- 维护者能否快速判断本次提交对应哪个 Issue、哪个 Gate、哪个切片、影响哪些接口、实际运行了哪些验证。

PR 仍是主要合并证据，commit message 是轻量、可追溯的治理留痕。

## 2. 推荐格式

```text
<type>(<scope>): <description>

Gate: G3 Pass
Slice: docs-only / governance / core-runtime / data-spec / adapter / authoring-tool / example / cross-layer
Impact: core-api=<none|changed>; data-format=<none|changed>; adapter-api=<none|changed>
Scope: <what changed>
Validation: <commands or manual checks>
Docs: updated / not required / pending <reason>

Refs: #<id>
```

示例：

```text
feat(core): 校验 route segment 连续性

Gate: G3 Pass
Slice: core-runtime
Impact: core-api=changed; data-format=none; adapter-api=none
Scope: 增加 route edge sequence 连通性校验并返回结构化错误
Validation: cargo +1.96.0 test --workspace --locked
Docs: updated

Refs: #12
```

只在满足 G4 完成边界时使用 `Closes: #<id>`。一般实现 PR commit 使用 `Refs: #<id>`，由 G4 清场负责关闭 Issue。

## 3. Conventional Commits 标题

标题格式：

```text
<type>[optional scope][optional !]: <description>
```

LaneFlow 允许的 `type`：

- `feat`：新增用户可见能力或 Core 能力。
- `fix`：修复缺陷、错误语义或错误边界。
- `docs`：只改文档、治理说明、模板说明。
- `test`：新增或调整测试，不改变运行时行为。
- `refactor`：不改变外部行为的代码结构调整。
- `perf`：性能优化。
- `build`：构建系统、依赖锁定、工具链配置。
- `ci`：GitHub Actions 或其他 CI 配置。
- `chore`：维护性任务，不属于以上类型。
- `revert`：回滚已提交变更。

`scope` 应使用小写短标识，优先选择受影响区域：

- `core`
- `governance`
- `docs`
- `ci`
- `adapter`
- `data`
- `example`
- `release`

标题应简短描述结果，而不是过程。

推荐：

- `feat(core): 实现 fixed-step tick`
- `fix(core): 拒绝非有限 edge length`
- `docs(governance): 对齐提交规范`
- `ci(governance): 校验 PR commit 信息`

不推荐：

- `update files`
- `fix stuff`
- `first pass`
- `wip`
- `更新文件`
- `先改一版`

## 4. Slice 字段

`Slice` 表示 LaneFlow 治理切片，必须与 `docs/governance/development-gates.md` 中的切片类型一致：

- `docs-only`
- `governance`
- `core-runtime`
- `data-spec`
- `adapter`
- `authoring-tool`
- `example`
- `cross-layer`

`type` 和 `Slice` 不是一回事：

- `type` 面向 Conventional Commits、changelog 和版本工具。
- `Slice` 面向 LaneFlow 的 G0-G4 治理、验证矩阵和风险判断。

常见映射：

| `type`     | 常见 `scope`          | 常见 `Slice`               |
| ---------- | --------------------- | -------------------------- |
| `feat`     | `core`                | `core-runtime`             |
| `feat`     | `adapter`             | `adapter`                  |
| `feat`     | `data`                | `data-spec`                |
| `fix`      | `core`                | `core-runtime`             |
| `docs`     | `governance` / `docs` | `docs-only` / `governance` |
| `test`     | `core`                | `core-runtime`             |
| `ci`       | `governance`          | `governance`               |
| `refactor` | `core`                | `core-runtime`             |

跨层变更必须使用 `Slice: cross-layer`，即使标题的 `scope` 只能写一个主要区域。

## 5. Gate 字段

`Gate` 表示本次提交对应的治理判断。

常见值：

- `G3 Pass`：提交前验证已满足当前切片要求。
- `G3 Block`：提交用于记录阻断或纠偏，不应视为完成。
- `G3 Waived`：存在显式例外。
- `Docs Only`：仅文档更新，且无运行时行为变更。

如果提交只是早期探索，不应合入 `main`，应使用分支或 PR 草稿，而不是把 `Gate` 写成 `Pass`。

## 6. Impact 字段

`Impact` 用于快速判断兼容性风险。

必须覆盖：

- `core-api`
- `data-format`
- `adapter-api`

推荐值：

```text
Impact: core-api=none; data-format=none; adapter-api=none
```

如果任一项为 `changed`，PR 中必须说明变更类型、兼容性影响和对应设计依据。

## 7. Breaking Change

破坏性变更必须同时满足 Conventional Commits 和 LaneFlow 治理要求。

Conventional Commits 表达方式二选一：

```text
feat(core)!: 调整 tick API
```

或在正文 / footer 中写：

```text
BREAKING CHANGE: TickInput.delta_time_ms 从可选改为必填。
```

LaneFlow 额外要求：

- `Impact` 中对应接口必须写 `changed`。
- PR 中必须链接 ADR 或 design 文档依据。
- PR 必须说明迁移边界或当前原型阶段为何可接受。

## 8. Validation 字段

`Validation` 只记录实际运行或实际完成的检查。

示例：

- `Validation: 仅文档审阅`
- `Validation: cargo +1.96.0 test --workspace --locked`
- `Validation: cargo +1.96.0 test --workspace --locked; 示例 smoke test`
- `Validation: 未运行，仅文档变更`

不得写未运行的命令。

## 9. Docs 字段

`Docs` 用于说明长期知识是否已回写。

常见值：

- `updated`
- `not required`
- `pending #<issue-id>`

涉及 Core API、数据格式、Adapter 协议或重大设计取舍时，不能只写 `not required`，除非 PR 中解释原因。

## 10. Refs / Closes 字段

提交底部使用 GitHub footer 关联 Issue：

- `Refs: #<id>`：引用 Issue，但不自动关闭。
- `Closes: #<id>`：合并后关闭 Issue。

只有关联 Issue 满足 G4 完成边界时，才使用 `Closes: #<id>`；否则使用 `Refs: #<id>`。

早期治理 bootstrap 或仓库初始化如果没有 Issue，必须写明原因，例如：

```text
Refs: pending, initial repository governance bootstrap
```

## 11. 与 PR 的关系

PR 是主要合并证据，commit message 是轻量留痕。

以下情况应优先走 PR，而不是只依赖 commit message：

- `core-runtime` 变更。
- `data-spec` 变更。
- `adapter` 变更。
- `cross-layer` 变更。
- 任何 breaking change。
- 任何需要 reviewer 或 AI Agent 二次审查的变更。

## 12. PR 合并策略

PR 合入 `main` 默认使用 **Rebase and merge**，以便：

- 保持 `main` 线性历史。
- 保留 PR 内各 commit 的 Conventional Commits 标题和 LaneFlow 治理字段。

默认命令：

```powershell
gh pr merge <number> --rebase
```

例外：

- **Squash and merge**：PR 内多个 wip commit 且无独立留痕价值，或明确要求 1 PR = 1 commit。
- **Create a merge commit**：发布分支、长期分支合流等场景。

使用例外时，须在 PR 中说明原因。详见 `../governance/github-workflow.md` 第 7 节。

## 13. CI 校验

PR commit message 应通过仓库 CI 的提交信息检查：

- 标题符合 Conventional Commits 标题格式。
- 正文包含 `Gate`、`Slice`、`Impact`、`Scope`、`Validation`、`Docs`。
- 底部包含 `Refs` 或 `Closes`。
- `Slice` 使用 LaneFlow 支持的切片类型。
- `Impact` 同时覆盖 `core-api`、`data-format` 和 `adapter-api`。

本地可运行：

```powershell
cargo +1.96.0 run -p xtask -- check-commit-messages origin/main..HEAD
```

本地运行必须显式传入 rev-range，避免默认 `HEAD` 扩大到历史祖先；CI 会根据 `pull_request` / `push` event 自动推导检查范围。

如果确有例外，应在 PR 中说明原因，并按 `development-gates.md` 的例外治理规则记录。
