# 验证矩阵

**文档状态**: Active  
**最后更新**: 2026-06-20  
**适用范围**: LaneFlow 各切片类型在 `G3` 合并闸口前的最小验证要求  
**关联文档**:

- 上游治理:
  - `../governance/development-gates.md`
  - `commit-convention.md`
- 模板:
  - `../../.github/pull_request_template.md`

## 1. 目标

本文把 `development-gates.md` 中“按切片类型验证”的要求收敛为一张可执行矩阵，回答每种切片：

- 哪些检查必须做。
- 哪些检查通常不需要。
- 无法运行时如何记录。

矩阵不要求所有 PR 跑同一组重复检查，但要求每次变更显式说明验证结论。Rust Core workspace 落地后，`core-runtime` 切片默认应运行 `cargo fmt --all -- --check` 与 `cargo test --workspace --locked`；其他技术栈检查在对应代码落地后逐步启用。

## 2. 切片类型到验证矩阵

| 切片类型         | 必须的验证                                                                                         | 通常不需要                                  |
| ---------------- | -------------------------------------------------------------------------------------------------- | ------------------------------------------- |
| `docs-only`      | 文档可读性检查、链接有效、无行为变更声明                                                           | build、单元测试、schema 校验                |
| `governance`     | 模板/路径/引用一致性、受影响流程说明、CI 文件存在性                                                | 运行时测试                                  |
| `core-runtime`   | `cargo fmt --all -- --check`、`cargo test --workspace --locked`、确定性行为说明、Core API 影响说明 | adapter build、示例 smoke（除非影响主路径） |
| `data-spec`      | schema/格式校验、兼容性与版本影响、示例数据影响                                                    | adapter build（除非协议联动）               |
| `adapter`        | adapter build、手工场景验证、transform 同步验证、Core 依赖方向检查                                 | 跨引擎全量测试（除非显式要求）              |
| `authoring-tool` | 工具运行验证、输出数据可被 Core 消费、格式一致性                                                   | 引擎端 build                                |
| `example`        | 示例可运行说明、覆盖能力说明、所依赖数据格式版本                                                   | 完整单元测试套件                            |
| `cross-layer`    | 以上相关项全部适用、端到端路径验证、是否需要示例 smoke 的显式判断                                  | 无默认豁免                                  |

## 3. 默认阻断条件

以下情况默认阻断 `G3 = Pass`：

1. Adapter 代码把引擎依赖泄漏进 Core。
2. 数据格式变化没有文档或版本说明。
3. Core API 破坏性变化没有 ADR 或 design 依据。
4. 新增运行时依赖违反 `../adr/0002-dependency-and-licensing-constraints.md`。
5. 必需验证未运行且没有原因说明。
6. PR 声称完成父任务，但证据只覆盖子切片。
7. 例外缺少原因、清理责任或后续 Issue。

## 4. 无法运行时的记录方式

当某项必需检查当前无法运行（例如运行时代码尚未存在、工具链未就绪）：

- 在 PR 的「验证」区写明「未运行」及原因。
- 在 commit message 的 `Validation` 字段同步记录，例如 `Validation: 未运行，运行时代码尚未落地`。
- 不得把未运行的检查写成已通过。

## 5. 与提交规范的关系

本矩阵定义“做什么检查”，`commit-convention.md` 定义“如何记录结果”。

两者必须一致：commit message 的 `Slice` 与本矩阵的切片类型一致，`Validation` 字段只记录实际执行或确认的检查。提交标题的 `type(scope)` 遵循 Conventional Commits，不替代 LaneFlow 的 `Slice` 判断。
