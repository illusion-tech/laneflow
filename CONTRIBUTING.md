# 贡献指南

感谢你参与 LaneFlow。

LaneFlow 采用 GitHub-first 治理：Issue 管任务，PR 管合并证据，仓库文档管长期设计事实。

## 1. 开始之前

参与开发前建议先阅读：

1. `README.md`
2. `docs/README.md`
3. `AGENTS.md`
4. `.agents/README.md`
5. `docs/governance/documentation-policy.md`
6. `docs/governance/github-workflow.md`
7. `docs/governance/development-gates.md`
8. `docs/governance/agent-development-guide.md`
9. `docs/reference/rust-code-style.md`

## 2. 提 Issue

所有非平凡任务应先创建 Issue。

Issue 应说明：

- 背景
- 目标
- 非目标
- 验收标准
- 影响范围
- 相关文档或 ADR

创建时使用仓库 Issue Form，并按工作性质选择 GitHub 原生 Type：新增能力或有意改变
行为/契约使用 `Feature`，权威契约与实际行为不一致使用 `Bug`，设计、调研、文档、
治理、重构、验证、依赖、发布或清理使用 `Task`。标题只写简短结果，不添加 `[功能]`
等分类前缀。

提交后维护结构化元数据：至少一个 `area:*` 标签、适用的 `work:*` 标签、Project
Status，以及排期工作对应的 Milestone。parent / sub-issue 只表达任务分解，真实阻塞
使用 blocked-by / blocking。如果任务涉及 Traffic Runtime API、数据格式或 Adapter
协议，可能需要先补设计文档或 ADR。完整规则见
`docs/governance/github-workflow.md` 第 2–4 节。

## 3. 分支

推荐分支命名：

- `feature/<issue-id>-<short-name>`
- `fix/<issue-id>-<short-name>`
- `docs/<issue-id>-<short-name>`
- `design/<issue-id>-<short-name>`
- `adapter/<issue-id>-<engine-or-topic>`

`main` 应保持可发布或至少可演示状态。

## 4. Pull Request

PR 应使用仓库 PR 模板，并至少说明：

- 关联 Issue
- 本次变更范围
- 本次明确不做范围
- Traffic Runtime API 影响
- 数据格式影响
- Adapter API 影响
- 文档更新情况
- 测试与验证结果
- 已知风险和例外

不要在父任务名义下合入只覆盖子范围的实现。PR body 使用 `Closes #<issue>`；commit
footer 使用 `Refs: #<issue>`。

## 5. PR 合并策略

LaneFlow 默认通过 **Merge Queue** 合入 `main`，队列最终使用 **Rebase** 保持线性历史。详见
`docs/governance/github-workflow.md` 第 7 节。

- 当前 exact head 的 required checks 完成后入队：`gh pr merge <number> --repo illusion-tech/laneflow --match-head-commit <H_pr>`；不得在 pending 时预先武装 auto-merge
- `H_pr` 与真实 `H_mg` 都必须完成 `Commit message`、`Rust checks`、`Wire toolchain audit`、`Placement fixture tests`、`Urban harness tests`、`Dependency policy`、`Analyze (actions)`、`Analyze (rust)`；原生 CodeQL rule 不替代队列中的两个 `Analyze` checks
- `main` 前进或队列重排只重建 Merge Group；未解决的 review conversation 继续由 GitHub 原生规则阻断
- Ruleset 保留 required checks、关闭 strict up-to-date，由 Merge Group 验证最新 `main` 组合
- 禁止日常 `--admin` 绕过队列；最终 merge method 例外必须先通过治理 Issue 修改规则

## 6. Commit Message

提交信息必须遵守 `docs/reference/commit-convention.md`。运行 `git config core.hooksPath .githooks` 后，仓库内置 `commit-msg` hook 可在本地提交前复用同一校验；CI 会再次检查 PR 和推送到 `main` 的 commit message。

推荐格式：

```text
feat(runtime): 校验 route segment 连续性

Refs: #12
```

提交标题遵循 Conventional Commits。footer 使用 `Refs: #<id>`；标题带 `!` 时必须有
`BREAKING CHANGE:`。标题中的 scope 可省略；使用时表示主要受影响的组件或职责域，
具体规则以 `docs/reference/commit-convention.md` 为准。不要再写 `Gate` / `Slice` /
`Impact` 等 G3 正文字段。

不要用 `Co-authored-by` 或 `Made-with:` 标注 AI 工具或模型。AI 参与时，在末尾 trailer 段成对写
`Harness: <执行框架>` 与 `Model: <实际使用的模型标识>`，段前空一行；纯人工提交可以不写。
`BREAKING CHANGE:` 留在这段之前。人类合作者的 `Co-authored-by` 仍然允许，但必须放在 `Refs:` /
`Closes:` 之前。`Model` 填写提交时实际使用的模型，不要照抄工具自动生成的值。

使用这些工具时关闭自动署名，再按上面的 trailer 手写：

- Cursor：IDE 关闭 Commit Attribution 和 PR Attribution。CLI 在 `~/.cursor/cli-config.json`
  把 `attribution.attributeCommitsToAgent` 和 `attribution.attributePRsToAgent` 设为 `false`。
  默认写入的是 `Made-with: Cursor`，hook 会拒绝。
- Codex CLI：不要再设置已删除的 `commit_attribution`。现行署名由账号上的
  `commit_attribution_enabled` 决定，不是本地 `config.toml` 字段；请在 Codex 账号设置里关闭
  commit attribution。旧客户端若仍追加 `Co-authored-by: Codex <noreply@openai.com>`，由 hook 拒绝。
- Claude Code：将 `attribution.commit` 设为空字符串。该设置已知不一定生效；仓库
  `commit-msg` hook 会拒绝仍然写上的 AI 工具 `Co-authored-by`。

## 7. 文档要求

长期结论应进入仓库文档：

- 架构决策进入 `docs/adr/`。
- 具体设计进入 `docs/design/`。
- 流程和治理进入 `docs/governance/`。
- 术语、模板和通用约定进入 `docs/reference/`。

GitHub Issue、PR 和 Discussion 中形成的稳定结论，应回写到仓库文档。

## 8. 测试与验证

Rust 代码除通过 `rustfmt` 和 Clippy 外，还应遵守 `docs/reference/rust-code-style.md` 中的仓库级可读性约定。数字字面量格式审阅只应覆盖当前变更范围；历史不一致应通过独立 Issue 有界清理。

当前 CI 包含：

- Commit message：Conventional Commits 标题、`Refs` / `Closes`、必要时 `BREAKING CHANGE:`；`xtask` 构建使用 `Swatinem/rust-cache`（仅 `main` 写回缓存）。
- Markdown tables：表格格式检查只警告，不阻断合并。
- Rust checks：job 始终运行以保持 required check 稳定；检查内容由 `docs/governance/github-workflow.md` §8 权威定义（`fmt`、`clippy`、workspace 测试与 doctest）。纯文档等非 Rust 路径跳过重型 cargo 并显式记录 skip。
- Wire toolchain audit：wire 工具链审计与运行时架构检查（仓库 cargo config 卫生、钉版、workspace unsafe 边界 hermetic 双特性编译与注入金丝雀），以独立 job 与 `Rust checks` 并行执行；非 Rust 路径跳过并显式记录 skip。
- Placement fixture tests：`placement-fixtures` 空 feature 的全量重跑（clippy 与 nextest，覆盖运行时、情景包与 junction-generator），以独立 job 与 `Rust checks` 并行执行；非 Rust 路径跳过并显式记录 skip。
- Urban harness tests：urban harness 套件（`-p laneflow-urban-harness --features adapter`）以独立 job 与 `Rust checks` 并行执行；非 Rust 路径跳过并显式记录 skip。
- Dependency policy：cargo-deny 检查 RustSec advisories、许可证、wildcard dependency 和 crate 来源。
- Analyze (actions) / Analyze (rust)：advanced CodeQL。
- Review conversation：未解决对话由 GitHub Ruleset 原生阻断；普通 Review 不由自定义 CI Check 计数或盖章。

数据 schema、Adapter build、示例 smoke test 和 Release 检查应在对应切片落地后继续加入专用门禁。

PR 中必须记录实际运行的检查。无法运行时，应说明原因和风险。

### 8.1 本地调试与构建产物体积（#802）

根 `Cargo.toml` 的 `dev` 档（`test` 档继承它）只生成行表级调试信息，依赖不生成调试信息，并关闭 `incremental`。后果是：panic 栈与 dhat 栈仍有文件名与行号，但调试器看不到局部变量与类型。需要变量级调试时使用 `debugging` 档。构建普通目标：

```text
cargo build --profile debugging
```

缺失局部变量出现在单元/集成测试里时，测试目标也要用同一档（否则只做了普通构建、
测试二进制仍按 `test` 档生成）：

```text
cargo test --profile debugging      # 也可加 -p <crate> / --test <name> 收窄范围
```

`debugging` 档继承 `dev`，仅恢复 `debug = true` 与 `incremental = true`；产物落在 `target/debugging`，首次使用需完整重建一次。

注意 `[profile.dev.package."*"]` 的覆盖会被 `inherits` 一并继承（实测：双 crate 最小工程下，自有 crate 拿到 `-C debuginfo=2`，依赖 crate 仍无调试信息）。因此 `debugging` 档恢复的是**工作区 crate** 的变量级调试；单步进入依赖内部时仍看不到依赖的局部变量。确实需要依赖调试信息时，临时加 `[profile.debugging.package."*"] debug = true`（代价是 `debugging` 档体积回到改造前量级）。

`target/` 不会自动回收：Cargo 稳定版没有 build-dir GC。体积长期增长时用 `cargo clean` 清理，或按保留天数清扫（工具与实测数据见 #802）。

## 9. AI Agent 开发

AI Agent 可以参与设计、实现、测试和文档维护，但应遵守 `docs/governance/agent-development-guide.md`。

Agent 不应在未读取相关设计文档的情况下修改 Traffic Runtime API、数据格式或 Adapter 协议。

通用 Agent 工作流位于 `.agents/skills/`。Cursor 的 `.cursor/skills/` 只作为薄包装入口，规范本体仍以 `.agents/` 和 `docs/` 为准。

## 10. 许可证与贡献

LaneFlow 公开仓库采用 Apache-2.0-only，完整条款见根目录 `LICENSE`。

除非你在提交前明确书面标记为“Not a Contribution”，或与维护者另有书面协议，你有意提交且被项目接收的 Contribution 将按 Apache License 2.0 许可，不附加额外条款。提交者应确认自己有权提交相关代码、文档、数据或其他材料，并披露其中不受 Apache-2.0 覆盖的第三方内容。

当前项目不要求 CLA。若未来商业授权或知识产权管理需要不同贡献安排，必须先通过独立治理 Issue 更新本指南；不能追溯性地假定现有贡献授予了 Apache-2.0 之外的权利。

许可证允许商业使用不代表自动满足具体分发物的全部义务。发布者仍须复核第三方许可证、attribution、修改说明、商标和目标市场要求。依赖与发布门禁见 `docs/governance/dependency-security.md`。
