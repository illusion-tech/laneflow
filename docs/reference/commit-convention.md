# 提交规范

**文档状态**: Active
**最后更新**: 2026-10-28
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
`Harness:` / `Model:` 成对，且两者均使用已登记的规范 ID。原文还用于拒绝冒号前空格、
行首缩进、取值折行和其他不规范写法。

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
Harness: <执行框架与入口形态的已登记规范 ID>
Model: <实际使用模型的已登记规范 ID>
```

- 键名大小写必须精确，冒号必须紧跟键名。`harness:`、`MODEL:` 和 `Harness :` 都会拒绝，而不是当成普通正文忽略。冒号后有一个空格，取值去掉首尾空白后不得为空。
- `Harness:` 的下一行必须是 `Model:`，中间不能空行。
- 这两行和 `Refs:` / `Closes:` 属于提交末尾的同一段 Git trailer，前面必须有空行，段内不能再有空行。
- `BREAKING CHANGE:` 不是 Git trailer 键。有 AI 署名时把它写在这段之前，不要写进 trailer 段。
- `Refs:` / `Closes:` 仍是最后一个非空行。人类 `Co-authored-by` 也不得写在它后面。键名和冒号之间的空格仍视为同一个 `Co-authored-by`。
- 也拒绝 `Made-with:` 标注 AI 工具，例如 Cursor 默认的 `Made-with: Cursor`。
- `Model` 只接受第 6.1 节目录中已登记的规范 ID，大小写与标点必须精确一致。别名、
  展示名、缺少实验室前缀的短名、模糊名称和未登记 ID 都拒绝，不会静默改写。
- `Harness` 只接受第 6.2 节目录中已登记的 `<发布者>/<产品>/<入口形态>` 精确 ID。
  展示名、模糊产品名、大小写变体和未登记的产品/入口组合均拒绝；候选只用于提示。
- 提交者须核对当前会话实际模型后选择对应规范 ID，不得照抄工具自动生成、且可能
  过期的模型名。目录只校验名称，不证明会话实际使用了该模型。

下面的示例记录通过 Claude Code 终端入口使用已登记模型的提交：

```text
feat(runtime): 校验 route segment 连续性

Harness: anthropic/claude-code/cli
Model: anthropic/claude-opus-5-5
Refs: #12
```

带人类合作者和破坏性变更时：

```text
feat(runtime)!: 调整 tick API

BREAKING CHANGE: TickInput.delta_time_ms 从可选改为必填。

Co-authored-by: Ada Lovelace <ada@example.com>
Harness: anthropic/claude-code/cli
Model: anthropic/claude-opus-5-5
Refs: #12
```

关闭各工具自动追加的 `Co-authored-by` 见 `CONTRIBUTING.md` 第 6 节。hook
仍会拒绝漏网的 AI 工具署名。

### 6.1 模型目录与登记

[`xtask/data/model-registry.json`](../../xtask/data/model-registry.json) 是本仓库接受的
模型 ID 目录，由 Git 保存版本，Rust 校验器在编译时嵌入。同一版本的本地 hook 和 CI
使用同一份数据，校验期间不联网，不读取用户自行配置的外部目录。

- `canonical_ids`：从 [models.dev 模型目录](https://models.dev/models.json?type=all)
  提取的完整规范 ID，例如 `openai/gpt-6-astra`、`anthropic/claude-opus-5-5`。
  `source` 和 `retrieved_at` 记录上游地址及该快照的 UTC 获取时间。
- `local_models`：上游尚未收录时的显式本地登记。每项必须提供完整的 `id` 和非空
  `source`，来源应指向能核对模型身份的官方文档或维护者接受的证据。不得只为了
  通过校验而登记已有模型的另一种写法。
- `aliases`：仅用于报错提示，每项记录 `alias`、`canonical_id` 和 `source`。
  [models.dev 服务商目录](https://models.dev/catalog.json?type=all) 中的
  `canonical_model_id` 可作为映射依据；工具特有前缀需明确登记，不能全局删除前缀。
  别名从不作为合法 `Model` 值。

规范 ID 采用 `<实验室>/<模型标识>`。这里的实验室指模型来源，不是执行框架或中转
服务商。保留目录中原有大小写、版本和变体，不把 `pro` / `pro-ultraspeed` 等合并，
也不由 `GPT-6` 猜测具体型号。诊断可以对大小写或短名给出唯一候选，但仍要求提交者
核对实际模型后修改；有多个候选时不替提交者选择。

使用新模型前，先更新目录或完成本地登记：

1. 获取上述 `models.json?type=all`，核对模型的完整顶层 ID；刷新快照时按 ID 排序
   更新 `canonical_ids`，同时更新 `retrieved_at`。目录是显式审阅的数据，不随网络
   响应自动改变。
2. 上游尚未收录时，在 `local_models` 加入带来源的规范 ID。上游后来收录同一 ID，
   则在刷新时移除重复的本地登记。目录中的 ID 必须唯一，大小写变体也不得重复登记；
   别名目标必须已经登记。
3. 若上游移除模型，核对原因；仍有使用依据的模型可转为带来源的本地登记，不能
   直接把它映射到另一型号。别名目标消失时同步审阅对应别名。
4. 运行 `cargo +1.99.1 test --locked -p xtask`。登记变更可与首次使用该规范 ID 的
   提交放在同一 PR；hook / CI 校验的是当前检出版本所携带的目录。

目录格式错误、重复 ID、缺少本地登记来源或别名冲突会使含 `Model` 的提交校验失败。
纯人工提交仍可省略整个 `Harness` / `Model` 对。新规则应用于 hook 文件或 CI 指定
范围中的提交；不改写已经合入的历史，尚未合入且进入检查范围的旧写法须修正。

数据来源：[models.dev](https://github.com/anomalyco/models.dev)（MIT）。

### 6.2 执行框架目录与入口登记

[`xtask/data/harness-registry.json`](../../xtask/data/harness-registry.json) 是本仓库接受的
执行框架 ID 目录，由 Git 保存版本，Rust 校验器在编译时嵌入。本地 hook 与 CI 使用
当前检出版本的同一份目录，校验时不联网，也不读取用户配置的替代目录。

规范 ID 采用 `<发布者>/<产品>/<入口形态>`，例如 `openai/codex/desktop`。这些 ID 是
LaneFlow 的登记约定，不宣称是外部目录或厂商的官方 ID。发布者与产品使用小写 ASCII
字母、数字及连接非空片段的单个连字符；不得使用 `unknown` 或 `other` 占位。命名空间
表示产品发布者，不是本次模型的实验室。显示名称或普通软件版本变化不另建 ID。

入口形态固定如下，每个产品与入口组合必须分别登记：

| 入口值          | 中文定义                                      | 判定示例                                                     |
| --------------- | --------------------------------------------- | ------------------------------------------------------------ |
| `cli`           | 终端入口，包含交互式 TUI 和非交互命令行运行   | 在 IDE 终端中直接运行 Codex CLI，仍记 `openai/codex/cli`     |
| `web`           | 浏览器中的任务交互入口，包括本机提供的 Web UI | 通过浏览器使用 `kimi web` 记 `moonshotai/kimi-code/web`      |
| `desktop`       | 独立桌面应用中的 Agent 入口                   | Codex 桌面会话记 `openai/codex/desktop`                      |
| `ide-extension` | 通过 IDE 插件/扩展接入的 Agent 工作流         | 在 Cursor 内使用 Codex 扩展，记 `openai/codex/ide-extension` |
| `ide`           | 完整 IDE 自带的 Agent 工作流                  | 使用 Cursor 编辑器内置 Agent，记 `anysphere/cursor/ide`      |

按为本次改动提供任务交互和上下文集成的主会话入口填写，不按底层执行命令的进程
或最后运行 `git commit` 的终端填写。IDE 扩展提供上下文集成、会话控制等功能时，即使
内部显示 TUI，也属于 `ide-extension`；仅把 IDE 终端当终端使用仍属于 `cli`。
桌面 Agent 调用浏览器工具不会变成 `web`；桌面 App 内部使用 CLI 服务也不会变成
`cli`。本地/远程/云端是执行位置，与入口形态独立。

每条提交仍只有一个 `Harness` / `Model` 对，记录本次提交归属的主会话。跨入口或
多工具协作时由提交者核对归属，不能在单个值中拼接多个 ID；该字段不是全部参与者
的执行轨迹。目录只验证名称与登记关系，不证明真实会话、模型或工具的使用情况。

登记表字段：

- `entries`：每项保存唯一 `id`、非空 `display_name` 和非空 `sources`。每个来源
  保存 HTTPS `url` 和有效日历日期 `checked_at`（`YYYY-MM-DD`），由登记者核实来源
  能支持对应产品及入口；机器只检查结构，不联网判定来源内容。
- `hints`：每项保存 `input` 和非空 `candidates`。所有候选必须已登记且不能重复；
  输入按 ASCII 大小写不敏感判定唯一，也不能与规范 ID 冲突。提示从不是合法别名。
  `Codex` 等模糊输入可对应多个入口，校验器列出所有候选，不替提交者选择。

新工具或新入口的登记流程：

1. 核对产品官方文档/仓库及实际入口。外部应用目录、排行、ACP Registry 可以提供
   发现线索，但不直接拥有本仓库的 ID；ACP 适配器、模型服务和产品入口不能互换。
   私有工具也须明确发布者、产品、入口，并提供维护者可审查的身份依据。
2. 在 `entries` 增加完整组合，写明来源和核实日期，按 ID 排序。已登记 CLI 不会自动
   允许同产品的 Web、Desktop 或 IDE 扩展；先核实再逐项登记。
3. 在 `hints` 增加需要的展示名/历史写法提示，按输入排序；新增入口时复核对应模糊
   产品名的候选列表。不得为同一入口登记另一种规范拼写，也不得凭旧提交名称猜入口。
4. 运行 `cargo +1.99.1 test --locked -p xtask`。登记变更可与首次使用该规范 ID 的提交
   放在同一 PR，hook / CI 使用当前检出版本携带的目录。
5. 产品改名或来源失效时通过 PR 核实并更新来源；保留稳定 ID。确需删除某组合时，
   同时处理提示引用并说明对尚未合入提交的影响，不把它静默映射成其他产品或入口。

目录格式错误、重复 ID、非法入口、缺失来源、无效日期或提示冲突会导致含 `Harness`
的提交校验失败。纯人工提交仍可省略整个 `Harness` / `Model` 对。规则作用于 hook
文件或 CI 选定范围中的提交；已经合入的历史不改写，进入检查范围的旧写法须修正。

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
- `Harness:` / `Model:` 要么都不写，要么成对、非空，和 `Refs:` / `Closes:` 组成末尾 trailer 段。段前有空行，段内没有 `BREAKING CHANGE:`；`Model` 与 `Harness` 分别必须是第 6.1、6.2 节已登记的精确规范 ID。

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
