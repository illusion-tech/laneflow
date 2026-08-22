# 验证矩阵

**文档状态**: Active
**最后更新**: 2026-08-21

**适用范围**: LaneFlow 各切片类型在合并前的最小验证要求
**关联文档**:

- 上游治理:
  - `../governance/development-gates.md`
  - `../governance/dependency-security.md`
  - `../governance/security-scanning.md`
  - `commit-convention.md`
  - `../adr/0026-merge-governance-rebuild.md`
- 模板:
  - `../../.github/pull_request_template.md`

## 1. 目标

本文把 `development-gates.md` 中“按切片类型验证”的要求收敛为一张可执行矩阵，回答每种切片：

- 哪些检查必须做。
- 哪些检查通常不需要。
- 无法运行时如何记录。

矩阵不要求所有 PR 跑同一组重复检查，但要求每次变更显式说明验证结论。Rust Core workspace
落地后，`core-runtime` 切片默认应运行 `cargo fmt --all -- --check` 与
`cargo test --workspace --locked`。仓库 CI 的 `Rust checks` job 按路径分流：非 Rust /
非 `schemas/` / 非 external-review 契约输入路径跳过重型 cargo。本地仍应按本矩阵主动运行
与切片相关的命令，不能只依赖 CI skip。

## 2. 切片类型到验证矩阵

| 切片类型         | 必须的验证                                                                                         | 通常不需要                                  |
| ---------------- | -------------------------------------------------------------------------------------------------- | ------------------------------------------- |
| `docs-only`      | 文档可读性检查、链接有效、无行为变更声明                                                           | build、单元测试、schema 校验                |
| `governance`     | 模板/路径/引用一致性、受影响流程说明；涉及 workflow/ruleset 时复核权限与 GitHub 实际状态           | 运行时测试                                  |
| `core-runtime`   | `cargo fmt --all -- --check`、`cargo test --workspace --locked`、确定性行为说明、Core API 影响说明 | adapter build、示例 smoke（除非影响主路径） |
| `data-spec`      | schema/格式校验、兼容性与版本影响、示例数据影响                                                    | adapter build（除非协议联动）               |
| `adapter`        | adapter build、手工场景验证、transform 同步验证、Core 依赖方向检查                                 | 跨引擎全量测试（除非显式要求）              |
| `authoring-tool` | 工具运行验证、输出数据可被 Core 消费、格式一致性                                                   | 引擎端 build                                |
| `example`        | 示例可运行说明、覆盖能力说明、所依赖数据格式版本                                                   | 完整单元测试套件                            |
| `cross-layer`    | 以上相关项全部适用、端到端路径验证、是否需要示例 smoke 的显式判断                                  | 无默认豁免                                  |

## 3. Markdown 表格格式

凡新增或修改含 GFM 表格的 Markdown，使用仓库内同一实现格式化：

```powershell
cargo +1.96.0 run --locked -p xtask -- format-md-tables <path...>
```

只读检查：

```powershell
cargo +1.96.0 run --locked -p xtask -- format-md-tables --check <path...>
```

CI 的 `Markdown tables` job 对协作范围内的 Markdown 执行相同检查，但只警告，不阻断合并。

## 4. 外部审阅

所有切片默认需要受信非作者 Approve/Comment，或受信非作者对 PR 正文点赞。名单见
`.github/trusted-reviewers.json`。未解决对话由 GitHub 原生规则拦截。

| 场景                                                     | 结果                           |
| -------------------------------------------------------- | ------------------------------ |
| 无原生 Review、无正文 👍                                 | Fail                           |
| 只有 PR author self-review 或作者点赞                    | Fail                           |
| 受信 reviewer 提交 `APPROVED` 或 `COMMENTED`（任意 head） | Pass                           |
| 受信 reviewer 对 PR 正文点 👍                            | Pass                           |
| Review 绑在旧 head 且结论仍为 Approve/Comment            | Pass                           |
| 最新 review 为 `CHANGES_REQUESTED` 或 `DISMISSED`        | Fail                           |
| 未列入名单的 bot（例如当前的 Codex connector）即使有评论 | Fail                           |
| `main` 前进但 `H_pr` 未变                                | 人审保留；重跑 `H_mg` 机器检查 |
| 新 push 产生新 `H_pr`                                    | 旧审阅作废                     |
| fork / cross-repository PR                               | 必须迁到同仓 PR                |

本地对照：

```powershell
cargo +1.96.0 run --locked -p xtask -- check-external-review --repo <owner/repo> --pr <number>
```

`publish-external-review-check` 只能在 trusted workflow 中使用。

## 5. 默认阻断条件

以下情况默认不得合入：

1. Adapter 代码把引擎依赖泄漏进 Core。
2. 数据格式变化没有文档或版本说明。
3. Core API 破坏性变化没有 ADR 或 design 依据。
4. 新增或更新依赖违反 ADR 0002 或 `dependency-security.md`，或 cargo-deny 未通过。
5. 必需验证未运行且没有原因说明。
6. PR 声称完成父任务，但证据只覆盖子切片。
7. `security-scanning.md` 要求的适用扫描仍为 pending、失败、无分析、已禁用或不可用。
8. 当前 head 缺少非作者受信原生 Review。
9. required checks 未在 `H_pr` 与真实 `H_mg` 上成功。

## 6. 无法运行时的记录方式

当某项必需检查当前无法运行：

- 在 PR 的「验证」区写明「未运行」及原因。
- 不得把未运行的检查写成已通过。

## 7. 与提交规范的关系

本矩阵定义“做什么检查”，`commit-convention.md` 定义 commit 标题和 footer。切片类型写在
PR 模板里，不进 commit 门禁。
