# 提交规范

**文档状态**: Active  
**最后更新**: 2026-07-14  
**适用范围**: LaneFlow 的本地提交、AI Agent 提交说明、PR commit 审查和无需完整 PR 审查的小切片留痕

## 1. 目标

LaneFlow 的提交信息以 [Conventional Commits 1.0.0](https://www.conventionalcommits.org/zh-hans/v1.0.0/) 为标题基础，并在正文保留 LaneFlow 治理字段。

提交信息必须同时回答两类问题：

- 生态工具能否识别本次变更类型、scope 和 breaking change。
- 维护者能否快速判断本次提交对应哪个 Issue、哪个 Gate、哪个切片、影响哪些接口、实际运行了哪些验证。

PR 仍是主要合并证据，commit message 是轻量、可追溯的治理留痕。

## 2. 推荐格式

```text
<type>[optional scope][optional !]: <description>

Gate: G3 Candidate
Slice: docs-only / governance / core-runtime / data-spec / adapter / authoring-tool / example / cross-layer
Impact: core-api=<none|changed>; data-format=<none|changed>; adapter-api=<none|changed>
Scope: <what changed>
Validation: <commands or manual checks>
Docs: updated / not required / pending <reason>

Refs: #<id>
```

治理字段使用严格格式：字段名后必须是冒号和一个空格（例如 `Slice: governance`）。`Gate`、`Slice`、`Impact`、`Scope`、`Validation`、`Docs` 必须按推荐格式连续排列，中间不得插入空行；标题后保留一个空行，`Docs` 后保留一个空行并接底部 `Refs` 或 `Closes` footer。`Impact` 必须按 `core-api`、`data-format`、`adapter-api` 顺序记录，并使用 `; ` 分隔。

示例：

```text
feat(core): 校验 route segment 连续性

Gate: G3 Candidate
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

新提交只允许：

- `G3 Candidate`：实现者已完成当前提交的本地验证，可以进入 PR 外部审阅；它不声明正式 G3 通过。
- `G3 Block`：提交用于分支上的阻断调查或纠偏记录，不应合入 `main` 或视为完成。

正式 `G3 Pass` 只由绑定 current head 的 `External Review Gate` Check 与 append-only `## G3 合并判断` comment 双钥匙表达，不再通过 amend commit 回写。`G3 Waived` 同样只存在于 PR Check / comment 的显式例外证据层；`Docs Only` 不再绕过外部审阅。

迁移规则：

- 本地 `commit-msg` hook 对新提交立即要求 `G3 Candidate` 或 `G3 Block`。
- range 校验器对 committer timestamp 早于 `2026-08-07T00:00:00Z` 的既有 commit 暂时兼容 `G3 Pass`、`G3 Waived` 与 `Docs Only`，避免为开放分支重写历史。
- cutoff 只用于迁移兼容，不赋予 legacy 值正式 G3 权威，也不能替代 PR 外部审阅、Check 或 G3 comment。
- committer timestamp 不是安全身份或审阅证明；它只决定 commit message 的兼容语法。

如果提交只是早期探索，不应合入 `main`，应使用分支或 PR 草稿；需要保存阻断事实时使用 `G3 Block`。

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

破坏性变更必须同时满足 Conventional Commits 和 LaneFlow 治理要求。LaneFlow 比 Conventional Commits 1.0.0 更严格：破坏性变更必须同时使用标题 `!` 和单行 `BREAKING CHANGE:` footer，避免只靠标题短句承载迁移信息。

推荐格式：

```text
feat(core)!: 调整 tick API

Gate: G3 Candidate
Slice: core-runtime
Impact: core-api=changed; data-format=none; adapter-api=none
Scope: 将 TickInput.delta_time_ms 固化为必填字段
Validation: cargo +1.96.0 test --workspace --locked
Docs: updated

BREAKING CHANGE: TickInput.delta_time_ms 从可选改为必填。
Refs: #12
```

LaneFlow 额外要求：

- 标题必须使用 `!`。
- `BREAKING CHANGE:` 必须提供单行非空说明，并放在 `Refs` / `Closes` 之前。
- `Refs` / `Closes` 仍必须是最后一个非空 footer 行。
- `Impact` 中至少一个对应接口必须写 `changed`。
- PR 中必须链接 ADR 或 design 文档依据。
- PR 必须说明迁移边界或当前原型阶段为何可接受。
- 复杂迁移说明应写入 PR、design 或 ADR；commit message 中暂不支持多行 `BREAKING CHANGE:`。

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
- `pending <reason>`

涉及 Core API、数据格式、Adapter 协议或重大设计取舍时，不能只写 `not required`，除非 PR 中解释原因。若文档回写由后续 Issue 跟踪，应把 Issue 号写入 `<reason>`，例如 `pending 后续由 #25 跟踪补齐`。

## 10. Refs / Closes 字段

提交底部使用 GitHub footer 关联 Issue：

- `Refs: #<id>`：引用 Issue，但不自动关闭。
- `Closes: #<id>`：表达该提交完成 Issue；只有满足 G4 完成边界时才允许使用。

只有关联 Issue 满足 G4 完成边界时，才使用 `Closes: #<id>`；否则使用 `Refs: #<id>`。

commit message footer 与 PR body 的 Development 关联语义不同：

- 常规 PR commit 继续使用 `Refs: #<id>`，由 G4 清场负责关闭 Issue。
- PR body 可以使用 `Closes #<id>` / `Resolves #<id>` 建立 GitHub Development 关联；仓库设置已关闭 linked PR 合并后自动关闭 Issue，因此该写法不替代 G4 手动关闭。
- 不得为了让 `closingIssuesReferences` 出现关联 Issue，而把 commit footer 从 `Refs` 改成 `Closes`。

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
- 新提交的 `Gate` 为 `G3 Candidate` 或 `G3 Block`；legacy 值只在上述 cutoff 前按 committer timestamp 兼容。
- 治理字段块按推荐格式连续排列，字段之间没有额外空行。
- 破坏性变更同时包含标题 `!`、单行 `BREAKING CHANGE:` footer，以及至少一个 `Impact` 的 `changed`。
- 底部包含 `Refs` 或 `Closes`。
- `Slice` 使用 LaneFlow 支持的切片类型。
- `Impact` 同时覆盖 `core-api`、`data-format` 和 `adapter-api`。

Dependabot 无法生成 LaneFlow 的完整治理字段。为使依赖自动更新能够进入正常 PR 审阅，range 校验器只对以下机器提交提供窄例外：

- Git author name 精确为 `dependabot[bot]`。
- Git author email 精确为 `49699333+dependabot[bot]@users.noreply.github.com`。
- 标题为非 breaking 的 `build(deps): <description>`。

三个条件必须同时满足。人工依赖提交、其他 bot、其他 scope 和 breaking change 仍必须使用完整治理正文。该例外只豁免单个 bot commit 的正文格式，不豁免 PR 模板、测试、cargo-deny、CodeQL、review、Development 关联或 G3/G4。作者字段不是安全身份认证，因此该规则不能替代受保护分支和 required checks。

本地可运行：

```powershell
cargo +1.96.0 run --locked -p xtask -- check-commit-messages origin/main..HEAD
```

也可以启用仓库内置 `commit-msg` hook，在提交创建前校验当前 commit message：

```powershell
git config core.hooksPath .githooks
```

启用后，Git 会在每次 `git commit` 时运行：

```powershell
cargo +1.96.0 run --locked -p xtask -- check-commit-message-file .git/COMMIT_EDITMSG
```

`check-commit-message-file` 会忽略 Git 提交模板、verbose commit 或 diffstat 生成的 `#` 注释行，再按最终提交正文执行同一套治理规则。

本地 `commit-msg` hook 不应用 Dependabot 例外，因为本地提交没有可信的最终 bot author 上下文；人工提交即使使用 `build(deps)` 标题，也必须填写完整治理字段。

本地运行必须显式传入 rev-range，避免默认 `HEAD` 扩大到历史祖先；CI 会根据 `pull_request` / `push` event 自动推导检查范围。

如果确有例外，应在 PR 中说明原因，并按 `development-gates.md` 的例外治理规则记录。
