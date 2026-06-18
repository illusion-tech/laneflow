# 提交规范

**文档状态**: Active  
**最后更新**: 2026-06-17  
**适用范围**: LaneFlow 的本地提交、AI Agent 提交说明和无需完整 PR 审查的小切片留痕

## 1. 目标

本文定义 LaneFlow 的提交信息规范。

提交信息的目标不是替代 PR，也不是复制 Issue 全文，而是让未来维护者能快速判断：

- 本次提交属于哪个 Issue 或切片。
- 本次改动的影响范围是什么。
- 是否通过了对应开发闸口。
- 实际运行了哪些验证。
- 文档和后续任务是否已经处理。

## 2. 推荐格式

```text
<issue-or-slice-id>: <short outcome>

Gate: G3 Pass
Type: docs-only / governance / core-runtime / data-spec / adapter / authoring-tool / example / cross-layer
Impact: core-api=<none|changed>; data-format=<none|changed>; adapter-api=<none|changed>
Scope: <what changed>
Validation: <commands or manual checks>
Docs: updated / not required / pending <reason>
Issue: closes #<id> / refs #<id> / pending <reason>
```

示例：

```text
LF-12: 补充 lane graph 设计基线

Gate: G3 Pass
Type: data-spec
Impact: core-api=none; data-format=changed; adapter-api=none
Scope: 文档化 lane node、lane edge、连接关系与校验规则
Validation: 仅文档审阅
Docs: updated
Issue: refs #12
```

## 3. 标题规则

标题应简短描述结果，而不是过程。

推荐：

- `LF-12: 补充 lane graph 设计基线`
- `LF-18: 校验 route segment 连续性`
- `LF-22: 文档化 adapter transform 同步契约`

不推荐：

- `update files`
- `fix stuff`
- `first pass`
- `wip`
- `更新文件`
- `先改一版`

## 4. Gate 字段

`Gate` 表示本次提交对应的治理判断。

常见值：

- `G3 Pass`：提交前验证已满足当前切片要求。
- `G3 Block`：提交用于记录阻断或纠偏，不应视为完成。
- `G3 Waived`：存在显式例外。
- `Docs Only`：仅文档更新，且无运行时行为变更。

如果提交只是早期探索，不应合入 `main`，应使用分支或 PR 草稿，而不是把 `Gate` 写成 `Pass`。

## 5. Type 字段

`Type` 应与 `docs/governance/development-gates.md` 中的切片类型一致：

- `docs-only`
- `governance`
- `core-runtime`
- `data-spec`
- `adapter`
- `authoring-tool`
- `example`
- `cross-layer`

## 6. Impact 字段

`Impact` 用于快速判断兼容性风险。

必须覆盖：

- `core-api`
- `data-format`
- `adapter-api`

如果有破坏性变更，应在提交正文或 PR 中明确写出 `breaking change`，并链接对应 ADR 或 design 文档。

## 7. Validation 字段

`Validation` 只记录实际运行或实际完成的检查。

示例：

- `Validation: 仅文档审阅`
- `Validation: cargo test`
- `Validation: npm test; 示例 smoke test`
- `Validation: 未运行，仅文档变更`

不得写未运行的命令。

## 8. Docs 字段

`Docs` 用于说明长期知识是否已回写。

常见值：

- `updated`
- `not required`
- `pending #<issue-id>`

涉及 Core API、数据格式、Adapter 协议或重大设计取舍时，不能只写 `not required`，除非 PR 中解释原因。

## 9. Issue 字段

`Issue` 用于和 GitHub 治理连接。

常见值：

- `closes #12`
- `refs #12`
- `pending, initial repository governance bootstrap`

只有满足 G4 完成边界时，才使用 `closes`。

## 10. 与 PR 的关系

PR 是主要合并证据，commit message 是轻量留痕。

以下情况应优先走 PR，而不是只依赖 commit message：

- `core-runtime` 变更。
- `data-spec` 变更。
- `adapter` 变更。
- `cross-layer` 变更。
- 任何 breaking change。
- 任何需要 reviewer 或 AI Agent 二次审查的变更。

