# Copilot 仓库说明

本文件是 Copilot on GitHub 的仓库级 custom instructions，用于给 review 和代码建议提供最小 LaneFlow 上下文。它是提示层，不是事实源或硬闸口；正式规则以仓库文档、PR 模板、CI、`gh` / GraphQL 复核和 Issue Gate Ledger 为准。

官方能力说明见 GitHub Docs: <https://docs.github.com/en/copilot/how-tos/copilot-on-github/customize-copilot/add-custom-instructions/add-repository-instructions>。

## 先读事实源

处理 LaneFlow PR 时，优先参考：

1. `AGENTS.md`
2. `README.md`
3. `docs/README.md`
4. `.agents/README.md`
5. `docs/governance/agent-development-guide.md`
6. `docs/governance/development-gates.md`
7. `docs/governance/github-workflow.md`
8. `docs/reference/commit-convention.md`
9. `.github/pull_request_template.md`

任务涉及 Core、数据格式或 Adapter 时，还应参考对应 `.agents/skills/*/SKILL.md`、`docs/design/` 和 `docs/adr/`。

## Review 语言与优先级

- 默认使用中文反馈，技术标识符、命令、文件路径和 API 名称保留原文。
- 优先指出会阻断 G3/G4 的问题：行为错误、治理字段缺失、测试缺口、设计依据缺失、Core / Adapter 边界错误、数据格式与 schema 不一致。
- 避免只给风格偏好；若建议调整，说明它影响哪条治理规则、验证矩阵或长期设计事实。

## 代码质量与设计原则 review

代码风格、架构、抽象和设计模式建议应有可执行依据，不能只表达个人偏好。Rust 代码质量 review 可参考 Rust API Guidelines 与 Rust Design Patterns，但不得机械套用面向对象设计模式。

Review 时关注：

- 风格问题优先依据 `rustfmt`、`clippy`、仓库既有命名、模块组织和中文优先注释约定。
- 架构问题必须对照 LaneFlow 分层、design / ADR 和切片范围，尤其检查 Core / data-format / Adapter 职责是否混杂。
- 抽象建议必须说明它减少了什么重复、隔离了什么变化，或避免了什么长期耦合；不要建议无 Issue / design 依据的过早泛化。
- Rust pattern 建议必须说明具体收益或风险，例如 ownership 语义、newtype / typed handle、trait 边界、`Result` / error 类型、模块可见性、失败原子性、API 可预测性或测试可验证性。
- 设计原则可作为 review 判断框架：SRP、DRY、KISS、YAGNI、composition over inheritance、Design by Contract、encapsulation、CQS、POLA、Single Choice。
- OCP、LSP、DIP 等 OOP 语境原则只能作为参考；不要据此强制抽 trait、加继承式层级或引入不必要的动态分发。
- 每条代码质量评论应说明严重程度、影响范围和可执行修复方向；没有明确影响的 nit 应避免提出。

参考资料：

- Rust API Guidelines: <https://rust-lang.github.io/api-guidelines/>
- Rust Design Patterns: <https://rust-unofficial.github.io/patterns/intro.html>
- Rust Design Patterns - Design principles: <https://rust-unofficial.github.io/patterns/additional_resources/design-principles.html>

## 通用治理检查

Review 时重点检查：

- PR body 是否按模板说明关联 Issue、Delivery PR / Related PR 角色、切片类型、范围、非目标、影响、验证、风险和 Gate Ledger；G3 checkbox 是否回链 PR G3 comment，Delivery PR 的 G4 回写是否指向 Issue G4 comment。
- Issue 是否已有 G0/G1/G2 记录；不得把 G0-G3 首次记录推迟到 G4。
- Delivery PR 是否在 G3 前确认 `closingIssuesReferences` 覆盖目标 Issue；Related PR 是否避免 closing keyword；部分交付或无法机器关联时是否记录显式例外。
- PR body 使用 `Closes #<issue>` / `Resolves #<issue>` 建立 Development 关联；commit footer 通常继续使用 `Refs: #<issue>`。
- commit message 是否符合 Conventional Commits 标题和 LaneFlow 治理字段：`Gate`、`Slice`、`Impact`、`Scope`、`Validation`、`Docs`、`Refs` / `Closes`。
- 必需元数据缺失时是否记录显式例外；不适用项是否有 `N/A` 原因。
- ruleset bypass、admin override 或其他例外是否记录原因、风险、接受边界和 Cleanup owner。

## 切片重点

### `governance`

- 检查治理文档、Agent skill、Issue / PR 模板、CI 或脚本之间的字段名、大小写、占位符和语义是否一致。
- 检查新规则是否写入长期事实源，而不是只留在 PR 评论或聊天记录里。
- 检查提示层文件是否保持薄包装，避免复制完整治理规范。
- 涉及安全设置、扫描或公开发布时，转读 `docs/governance/security-scanning.md`，检查实际配置、最近分析、开放告警、状态语义和 `#56` 职责边界是否一致。

### `data-spec`

- 检查 `docs/design/data-format.md`、相关 `docs/design/*` 和 `schemas/*.schema.json` 是否一致。
- 关注 `formatVersion`、单位、external ID、epsilon、closed shape、schema validation 与 domain validation 的边界。
- 明确示例数据、validator、Core loader 和 Adapter / authoring tool 的责任划分。

### `core-runtime`

- 检查 Rust 实现是否保持 Core engine-agnostic，不依赖 Unity、Unreal、Godot、O3DE、WebGL、DOM 或具体引擎 API。
- 关注 deterministic tick、失败原子性、稳定事件顺序、typed handle / external ID 边界和中文优先错误信息。
- 默认要求 `cargo +1.96.0 fmt --all -- --check` 与 `cargo +1.96.0 test --workspace --locked`，未运行必须说明原因。

### `adapter`

- 检查 Adapter 是否只负责引擎集成、表现、模型、动画、LOD、调试可视化和生命周期接入。
- Adapter 不应复制 Core 交通规则，也不应把引擎依赖引入 Core。

## 限制

- 不要把本文件当作 GitHub 元数据事实源；Project status、Labels、Milestone、Parent / sub-issues、Blocked by、Blocking、review threads 和 `closingIssuesReferences` 必须由 GitHub UI、`gh` 或 GraphQL 复核。
- 不要把 Copilot review 当作 CI 或 Gate Ledger 的替代品。
- Copilot review 对 custom instructions 的生效以 base branch 中的 instructions 为准；修改本文件的 PR 不一定影响该 PR 当前这轮 review。
