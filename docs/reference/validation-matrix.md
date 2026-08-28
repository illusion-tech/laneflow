# 验证矩阵

**文档状态**: Active
**最后更新**: 2026-08-24

**适用范围**: LaneFlow 各切片类型在合并前的最小验证要求
**关联文档**:

- 上游治理:
  - `../governance/development-gates.md`
  - `../governance/dependency-security.md`
  - `../governance/security-scanning.md`
  - `commit-convention.md`
  - `../adr/0026-merge-governance-rebuild.md`
  - `../adr/0027-retire-external-review-check.md`
- 模板:
  - `../../.github/pull_request_template.md`

## 1. 目标

本文把 `development-gates.md` 中“按切片类型验证”的要求收敛为一张可执行矩阵，回答每种切片：

- 哪些检查必须做。
- 哪些检查通常不需要。
- 无法运行时如何记录。

矩阵不要求所有 PR 跑同一组重复检查，但要求每次变更显式说明验证结论。Rust Core workspace
落地后，`core-runtime` 切片默认应运行 `cargo fmt --all -- --check` 与
`cargo test --workspace --locked`。仓库 CI 的 `Rust checks` job 按路径分流：非 Rust 契约输入
路径跳过重型 cargo。道路编辑 FlatBuffers 变更走独立 Codegen，不因 `.fbs` 拉起整仓
Rust 测试。本地仍应按本矩阵主动运行与切片相关的命令，不能只依赖 CI skip。

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
cargo +1.98.0 run --locked -p xtask -- format-md-tables <path...>
```

只读检查：

```powershell
cargo +1.98.0 run --locked -p xtask -- format-md-tables --check <path...>
```

CI 的 `Markdown tables` job 对协作范围内的 Markdown 执行相同检查，但只警告，不阻断合并。

## 4. Review conversation

普通 GitHub Review 继续用于协作，但不由自定义 Check、受信名单或 reaction 计数。
当前 Ruleset 不要求固定批准数或 CODEOWNERS review；未解决 review conversation 由
GitHub 原生 `required_review_thread_resolution: true` 阻止入队。

| 场景                            | 结果                                   |
| ------------------------------- | -------------------------------------- |
| 没有 Review 或 reaction         | 不影响五项机器 Check                   |
| 存在未解决 review conversation  | GitHub 原生规则阻止入队或合并          |
| 所有 review conversation 已解决 | 对话条件满足，仍须等待 required checks |
| `main` 前进但 `H_pr` 未变       | 重建 `H_mg` 并重跑五项机器检查         |
| 新 push 产生新 `H_pr`           | 新 head 重跑适用 PR 检查               |
| fork / cross-repository PR      | 必须迁到同仓 PR                        |

## 5. 默认阻断条件

以下情况默认不得合入：

1. Adapter 代码把引擎依赖泄漏进 Core。
2. 数据格式变化没有文档或版本说明。
3. Core API 破坏性变化没有 ADR 或 design 依据。
4. 新增或更新依赖违反 ADR 0002 或 `dependency-security.md`，或 cargo-deny 未通过。
5. 必需验证未运行且没有原因说明。
6. PR 声称完成父任务，但证据只覆盖子切片。
7. `security-scanning.md` 要求的适用扫描仍为 pending、失败、无分析、已禁用或不可用。
8. required checks 未在 `H_pr` 与真实 `H_mg` 上成功。

## 6. 无法运行时的记录方式

当某项必需检查当前无法运行：

- 在 PR 的「验证」区写明「未运行」及原因。
- 不得把未运行的检查写成已通过。

## 7. 与提交规范的关系

本矩阵定义“做什么检查”，`commit-convention.md` 定义 commit 标题和 footer。切片类型写在
PR 模板里，不进 commit 门禁。
