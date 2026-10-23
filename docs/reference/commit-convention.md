# 提交规范

**文档状态**: Active
**最后更新**: 2026-10-13
**适用范围**: LaneFlow 的本地提交、AI Agent 提交说明、PR commit 审查

## 1. 目标

LaneFlow 的提交信息以 [Conventional Commits 1.0.0](https://www.conventionalcommits.org/zh-hans/v1.0.0/)
为标题基础。机器检查标题、`Refs` / `Closes` footer、破坏性变更的
`BREAKING CHANGE:`，以及第 6 节的 AI 署名。切片、影响和验证写在 PR 模板里，
不进 commit 门禁。

## 2. 推荐格式

```text
<type>[optional scope][optional !]: <description>

Refs: #<id>
```

示例：

```text
feat(runtime): 校验 route segment 连续性

Refs: #12
```

PR body 使用 `Closes #<id>` 在合并后关闭 Issue。commit footer 默认仍用
`Refs: #<id>`，避免每个 commit 关单。若确需让单个 commit 关闭 Issue，可以使用
`Closes: #<id>`。

## 3. Conventional Commits 标题

标题格式：

```text
<type>[optional scope][optional !]: <description>
```

允许的 `type`：

- `feat`：新增用户可见能力或 Traffic Runtime 等产品能力。
- `fix`：修复缺陷、错误语义或错误边界。
- `docs`：只改文档、治理说明、模板说明。
- `test`：新增或调整测试，不改变运行时行为。
- `refactor`：不改变外部行为的代码结构调整。
- `perf`：性能优化。
- `build`：构建系统、依赖锁定、工具链配置。
- `ci`：GitHub Actions 或其他 CI 配置。
- `chore`：维护性任务，不属于以上类型。
- `revert`：回滚已提交变更。

`scope` 可省略。`type` 表示变更性质，`scope` 表示本提交主要影响的组件或职责域；
scope 不等同于 PR 的切片类型，也不要求把跨层变更硬塞进 `cross-layer`。没有单一主要
范围时可以省略 scope。

scope 出现时必须是小写短标识：首字符为小写字母或数字，后续可以使用小写字母、
数字、`.`、`_`、`-`。当前优先按实际组件或职责选择：

- 产品组件：`runtime`、`compiler`、`static-network`、`format`、`spatial`、
  `scenario`、`adapter`、`data`、`example`；
- 治理与支持：`governance`、`design`、`architecture`、`docs`、`ci`、`release`、
  `deps`、`research`。

上述列表是当前命名建议，不是封闭白名单。`Commit message` 只校验 scope 语法，新增
组件不需要先修改 CI 枚举。`laneflow-core` 已拆除，新提交不要再以 `core` 指代当前
Traffic Runtime；对应范围使用 `runtime`。历史提交保持原样，不追溯改写。

标题应简短描述结果，而不是过程。

推荐：

- `feat(runtime): 实现 fixed-step tick`
- `fix(runtime): 拒绝非有限 edge length`
- `refactor(static-network): 统一共享路网索引`
- `feat(compiler): 接入新的编译遍`
- `docs(governance): 对齐提交规范`
- `ci(governance): 校验 PR commit 信息`

不推荐：`update files`、`fix stuff`、`first pass`、`wip`、`更新文件`、`先改一版`。

## 4. Breaking Change

破坏性变更必须同时使用标题 `!` 和单行 `BREAKING CHANGE:` footer。

```text
feat(runtime)!: 调整 tick API

BREAKING CHANGE: TickInput.delta_time_ms 从可选改为必填。

Refs: #12
```

- 标题必须使用 `!`。
- `BREAKING CHANGE:` 必须提供单行非空说明，放在 trailer 段之前，并与该段空一行。它不是 Git trailer 键，贴着 `Refs` / `Closes` 写会让 Git 认不出 footer。
- 若同时写 AI 署名，`BREAKING CHANGE:` 留在 trailer 段之前，并空一行。`BREAKING CHANGE:` 的键名含空格，写进 trailer 段会让 Git 整段都解析不到。见第 6 节。
- `Refs` / `Closes` 必须是最后一个非空 footer 行。
- 复杂迁移说明应写入 PR、design 或 ADR。

## 5. Refs / Closes

- `Refs: #<id>`：引用 Issue，不关闭。
- `Closes: #<id>`：该提交完成 Issue。
- 早期 bootstrap 如果没有 Issue：`Refs: pending, <reason>`。

历史提交上残留的 `Gate` / `Slice` / `Impact` / `Scope` / `Validation` / `Docs`
字段可以忽略，不强制改写。新提交不必再写这些字段。

## 6. AI 署名

`Co-authored-by` 只用于人类合作者。不得用它标注 AI 执行框架或模型。本地
`commit-msg` hook 与 CI 的 `Commit message` 使用同一套校验：先用
`git interpret-trailers --parse --no-divider` 取得 Git 认定的 trailer。该调用固定 `trailer.separators=:`，并忽略宿主机的 Git 配置。然后再检查 AI 署名和
`Harness:` / `Model:` 成对。原文只用来拒绝冒号前空格、行首缩进、取值折行和其他不规范写法。

`Co-authored-by` 键名大小写不敏感。取值满足任一条件即拒绝，并提示改写为
`Harness:` / `Model:`：

- 显示名按 ASCII 词边界包含 `cursor`、`claude`、`codex` 或 `copilot`
  （大小写不敏感）。没有尖括号时，整段取值按显示名处理。
- 尖括号内邮箱的 local-part 按同样的词边界命中上述标识。
- 邮箱精确等于 `noreply@anthropic.com`、`noreply@openai.com` 或
  `cursoragent@cursor.com`（大小写不敏感）。

不检查邮箱域名，因此 `researcher@openai.com` 不会仅因域名被拒绝。词边界
也不会把 `Claudette` 当成 `claude`。

AI 参与的提交改用下面两行，纯人工提交可以不写。出现时必须同时写，且各写一次：

```text
Harness: <执行框架>
Model: <实际使用的模型标识>
```

- 键名大小写必须精确，冒号必须紧跟键名。`harness:`、`MODEL:` 和 `Harness :` 都会拒绝，而不是当成普通正文忽略。冒号后有一个空格，取值去掉首尾空白后不得为空。
- `Harness:` 的下一行必须是 `Model:`，中间不能空行。
- 这两行和 `Refs:` / `Closes:` 属于提交末尾的同一段 Git trailer，前面必须有空行，段内不能再有空行。
- `BREAKING CHANGE:` 不是 Git trailer 键。有 AI 署名时把它写在这段之前，不要写进 trailer 段。
- `Refs:` / `Closes:` 仍是最后一个非空行。人类 `Co-authored-by` 也不得写在它后面。键名和冒号之间的空格仍视为同一个 `Co-authored-by`。
- 也拒绝 `Made-with:` 标注 AI 工具，例如 Cursor 默认的 `Made-with: Cursor`。
- 不校验取值是否属于白名单。`Model` 必须填写提交时实际使用的模型，不得照抄工具
  自动生成、且可能过期的模型名。

执行框架可以写成 `Codex CLI`、`Claude Code` 或 `Cursor`，也可以是其他非空名称。
下面只说明格式，校验不核对这些取值：

```text
feat(runtime): 校验 route segment 连续性

Harness: Claude Code
Model: claude-opus-4.7
Refs: #12
```

带人类合作者和破坏性变更时：

```text
feat(runtime)!: 调整 tick API

BREAKING CHANGE: TickInput.delta_time_ms 从可选改为必填。

Co-authored-by: Ada Lovelace <ada@example.com>
Harness: Claude Code
Model: claude-opus-4.7
Refs: #12
```

关闭各工具自动追加的 `Co-authored-by` 见 `CONTRIBUTING.md` 第 6 节。hook
仍会拒绝漏网的 AI 工具署名。

## 7. 与 PR 的关系

PR 是主要合并证据。Core、数据格式、Adapter、跨层变更和任何 breaking change
都应走 PR。当前 Ruleset 不要求固定批准数或 CODEOWNERS review；已有 review
conversation 必须在当前 patchset 上解决后才能入队。若以后要强制独立批准，必须先经
治理 Issue 选择 GitHub 原生 required approvals / CODEOWNERS。

## 8. PR 合并策略

PR 默认通过 Merge Queue 合入 `main`，队列最终 **Rebase**。详见
`../governance/github-workflow.md` 第 7 节。

```powershell
gh pr merge <number> --repo illusion-tech/laneflow --match-head-commit <H_pr>
```

## 9. CI 校验

`Commit message` job 检查：

- 标题符合 Conventional Commits。
- scope 省略或符合小写短标识语法；不检查封闭枚举。
- 底部包含 `Refs` 或 `Closes`，且它是最后一个非空行。
- 破坏性变更同时包含标题 `!` 和单行 `BREAKING CHANGE:` footer。
- 不以 `Co-authored-by` 标注 AI 工具或模型；人类合作者除外。
- `Harness:` / `Model:` 要么都不写，要么成对、非空，和 `Refs:` / `Closes:` 组成末尾 trailer 段。段前有空行，段内没有 `BREAKING CHANGE:`。

Dependabot 无法生成 Issue footer。range 校验器只对同时满足以下条件的机器提交提供
窄例外：

- Git author name 精确为 `dependabot[bot]`。
- Git author email 精确为 `49699333+dependabot[bot]@users.noreply.github.com`。
- 标题为非 breaking 的 `build(deps): <description>`。

该例外只豁免单个 bot commit 缺少的 `Refs:` / `Closes:` footer。footer 格式错误，或它不是最后一个非空行，仍然拒绝。例外也不豁免 AI 署名、标题、`BREAKING CHANGE:`、PR 审阅、cargo-deny 或 CodeQL。
本地 `commit-msg` hook 不应用 Dependabot 例外。

本地可运行：

```powershell
cargo +1.99.1 run --locked -p xtask -- check-commit-messages origin/main..HEAD
```

启用仓库内置 hook：

```powershell
git config core.hooksPath .githooks
```
