# 验证矩阵

**文档状态**: Active  
**最后更新**: 2026-07-24

**适用范围**: LaneFlow 各切片类型在 `G3` 合并和 `G4` 收口闸口前的最小验证要求  
**关联文档**:

- 上游治理:
  - `../governance/development-gates.md`
  - `../governance/dependency-security.md`
  - `../governance/security-scanning.md`
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

| 切片类型         | 必须的验证                                                                                                                                                                                                             | 通常不需要                                  |
| ---------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------- |
| `docs-only`      | 文档可读性检查、链接有效、Markdown 表格格式检查、无行为变更声明                                                                                                                                                        | build、单元测试、schema 校验                |
| `governance`     | 模板/路径/引用一致性、Issue 元数据 / 依赖关系审计一致性、受影响流程说明、commit Gate 与 external-review fixtures；涉及 workflow/ruleset 时复核 trusted-ref、权限、head binding、GitHub 实际状态、cargo-deny 与扫描结果 | 运行时测试                                  |
| `core-runtime`   | `cargo fmt --all -- --check`、`cargo test --workspace --locked`、确定性行为说明、Core API 影响说明                                                                                                                     | adapter build、示例 smoke（除非影响主路径） |
| `data-spec`      | schema/格式校验、兼容性与版本影响、示例数据影响                                                                                                                                                                        | adapter build（除非协议联动）               |
| `adapter`        | adapter build、手工场景验证、transform 同步验证、Core 依赖方向检查                                                                                                                                                     | 跨引擎全量测试（除非显式要求）              |
| `authoring-tool` | 工具运行验证、输出数据可被 Core 消费、格式一致性                                                                                                                                                                       | 引擎端 build                                |
| `example`        | 示例可运行说明、覆盖能力说明、所依赖数据格式版本                                                                                                                                                                       | 完整单元测试套件                            |
| `cross-layer`    | 以上相关项全部适用、端到端路径验证、是否需要示例 smoke 的显式判断                                                                                                                                                      | 无默认豁免                                  |

## 3. Markdown 表格格式门禁

凡新增或修改含 GFM 表格的 Markdown，必须使用仓库内同一实现完成格式化：

```powershell
cargo +1.96.0 run --locked -p xtask -- format-md-tables <path...>
```

提交前必须对本次涉及的 Markdown 运行只读检查：

```powershell
cargo +1.96.0 run --locked -p xtask -- format-md-tables --check <path...>
```

命令接受一个或多个文件或目录；目录会递归处理 Markdown。默认模式只重写识别出的表格布局，`--check` 不修改文件，发现未格式化表格时返回失败。CI 对仓库协作范围内的 Markdown 执行相同检查，因此本规则适用于所有切片，而不只适用于 `docs-only`。

## 4. 外部审阅 Gate 回归矩阵

所有切片默认需要一个有效 external reviewer。首版标准路径只接受 exact-head review；`unresolved=0` 是必要非充分条件。

| 场景                                                                              | 预期状态 / 结果                                                                               |
| --------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------- |
| 无 review；仅 request、reaction、任务启动或无 reviewed SHA 摘要                   | `awaiting_review` / `review_pending`，Fail                                                    |
| 只有 PR author self-review                                                        | `awaiting_review`，Fail                                                                       |
| 受信任 reviewer 在 current head 完成 clean review                                 | `pass` 候选；仍需 threads、Checks 与 G3 comment                                               |
| finding 未处置或仍有 unresolved actionable thread                                 | `findings_open`，Fail                                                                         |
| finding 已回复/resolve，但没有新的 clean re-review                                | `awaiting_rereview`，Fail                                                                     |
| finding 处置后，受信任 reviewer 在 current head clean re-review                   | `pass` 候选                                                                                   |
| clean completion 后 new push 或 review dismissed                                  | `stale`，Fail 并重新请求 review                                                               |
| clean review 绑定旧 head，且没有已批准的等价例外                                  | `stale`，Fail                                                                                 |
| provider 文案正确但 actor 不在 allowlist                                          | Fail                                                                                          |
| author 转贴 Cursor / 本地 Agent 输出                                              | Fail                                                                                          |
| content-equivalent rebase 具备全部附加证据                                        | `waived`；不得自动转成标准 `pass`                                                             |
| 预创建或编辑旧 G3 comment 回填新证据                                              | Fail；必须新增 superseding comment                                                            |
| Related PR B 自身仍由 main 上的旧 validator 判断                                  | 按 R0 bootstrap 人工核验 exact-head review；不得用候选 validator 自批                         |
| Related PR C 自身尚未把 shadow workflow 合入 main                                 | 使用 main 上的 live validator；候选 Check 不得自批，Check 缺失按 R0 bootstrap 记录            |
| PR B 合入后的 R0/R1 PR 尚无 required External Review Gate                         | `G3 Pass` / bootstrap 要求 live `pass`；结构化 `G3 Waived` 保持 `waived`，并记录 Check 缺失   |
| fork / cross-repository PR 的 head 不属于 base repository                         | R1 不发布 Check 且不计 eligible sample；R2 前迁移到 same-repository PR 并重新 exact-head 审阅 |
| R2 PR 缺少 current-head External Review Gate success                              | Fail                                                                                          |
| Check success 与 G3 comment 绑定不同 head，或 comment 早于最终 completion / Check | Fail                                                                                          |

Provider fixtures 至少覆盖 Copilot clean/findings、Codex clean/findings、人工 `APPROVED`、无 SHA、错误 actor、new-push stale、finding 后无复审、重复 thread 与 provider outage。历史事件 replay 和人工审计必须与机器最终分类一致。

离线 fixture / replay 使用 `check-external-review --input <snapshot.json> --format json --expect <state>`；live 对照使用 `check-external-review --repo <owner/repo> --pr <number> --format json`。snapshot、结果 schema、固定状态枚举和 fail-closed 退出语义必须保持向后可识别；无法完整分页或二次读取 head/base 不一致时不得降级为 `awaiting_review` 或 `pass`。current G3 comment 缺少显式 `Gate 结果` 时必须失败；`G3 Waived` 还必须覆盖结构化 record、current head/base、reference-style evidence、授权人、当前 follow-up Issue、24 小时上限与过期回归。

workflow 安全检查至少验证：

- review / inline comment 事件只进入空权限、无 checkout 的 signal；trusted `workflow_run` 不读取 signal artifact / output；
- validator 显式从 `refs/heads/main` checkout，关闭 credential persistence，不 checkout、下载或执行 PR head，不执行 comment body，不读取 repository secret；
- token 权限固定为 `contents: read`、`pull-requests: read`、`issues: read`、`checks: write`，第三方 Action 完整 SHA pin；
- R1 Check 固定为 `External Review Gate Shadow`，绑定 API 最终确认的 current head/base，并复核 telemetry source App=`github-actions`；external ID 同时绑定 PR/head/trusted-ref/run；
- publisher 二次确认 `isCrossRepository=false`；fork / cross-repository PR 不尝试向 base repository 写入无法关联的 head Check；
- 只有 `pass -> success`；`waived -> action_required`，确保 required check 不会把 waiver 当作成功；其他状态均为 failure；
- PR concurrency 取消旧运行；identity race 不向新 head 发布旧结果；
- external ID 显式包含 evaluator state 和稳定 fingerprint；只有同 head/source App/trusted-ref/state 下的等价完成态 Check 才可复用，state 变化必须产生新 Check；
- Draft、非 `main` base、非 open PR 不计入 R1 sample；10 分钟 schedule 只作 review signal 缺失时的补偿；
- R1 sample 同时绑定 trusted default-branch workflow run、external ID 与 Check receipt；PR 自定义的同名 GitHub Actions job 不得计入；
- R2 publisher 使用独立专用 GitHub App，ruleset 绑定正式 Check name 与 expected source App；恶意 PR 新增同名 Actions job 的 canary 仍必须阻断合并；
- API/provider/解析歧义 fail closed。

publisher 的本地接口为 `publish-external-review-check --repo <owner/repo> --pr <number> --details-url <workflow-run-url> --run-id <id> --run-attempt <number> --trusted-ref-oid <oid>`。该命令会产生外部 Check 写操作，只能在 trusted workflow 中使用；本地 / PR head 验证只运行 payload、state mapping、identity race 与 workflow 静态安全单元测试，不得向真实 PR 发布候选 Check。

## 5. 默认阻断条件

以下情况默认阻断 `G3 = Pass`：

1. Adapter 代码把引擎依赖泄漏进 Core。
2. 数据格式变化没有文档或版本说明。
3. Core API 破坏性变化没有 ADR 或 design 依据。
4. 新增或更新依赖违反 `../adr/0002-dependency-and-licensing-constraints.md` 或 `../governance/dependency-security.md`，或 cargo-deny 未通过。
5. 必需验证未运行且没有原因说明。
6. PR 声称完成父任务，但证据只覆盖子切片。
7. 例外缺少原因、清理责任或后续 Issue。
8. 关联 Issue 缺少必需 GitHub 元数据 / 依赖关系审计且没有显式例外，或不适用项缺少 `N/A` 原因。
9. Delivery PR 的 `closingIssuesReferences` 未覆盖对应 Issue，或 Related PR 误用 closing keyword，且没有显式例外。
10. G3 comment / Issue G3 permalink 不完整或 reference-style 定义未由对应 Gate 行实际引用，Related-only 阶段提前勾选 Issue G3，Related PR 独立 G3 未永久保留单一 `--related-pr <current-related-pr>` 断言，full-set 未使用 `--delivery-pr` 加全部 Related PR，或错误要求改写历史 Related comment，`Gate 断言` 未记录与实际调用完全一致的规范命令和 `已通过` 结果，或 `cargo +1.96.0 run --locked -p xtask -- check-gate-evidence g3 ...` 失败。
11. `../governance/security-scanning.md` 要求的适用扫描仍为 `pending`、失败、无分析、已禁用或不可用，且没有显式例外；或把 API / 命令失败误写成零开放告警。
12. external review 缺失、pending、stale、actor/provider 不可信、finding 未完成 clean re-review，或只用 `reviewThreads=0` 证明 clean。
13. R2 激活后 current-head `External Review Gate` 未成功、source 不是 ruleset 绑定的专用 GitHub App，spoof canary 可由 PR 自定义同名 Actions job 满足，或 G3 comment 不是 Check 后新增的 append-only Owner 判断。
14. PR / push range 包含 `G3 Block`，或新 commit 使用 legacy `G3 Pass` / `G3 Waived` / `Docs Only` 且不满足 `commit-convention.md` 的 cutoff 兼容条件。

G4 清场前还必须运行 `check-gate-evidence g4`；它验证 Issue G4 permalink、关联 PR 合并状态、Gate Ledger、Project `Done`，以及 `Gate 断言` 的规范命令和 `已通过` 结果，但不替代 G4 comment 中的分支清理与权限撤回证据。

## 6. 无法运行时的记录方式

当某项必需检查当前无法运行（例如运行时代码尚未存在、工具链未就绪）：

- 在 PR 的「验证」区写明「未运行」及原因。
- 在 commit message 的 `Validation` 字段同步记录，例如 `Validation: 未运行，运行时代码尚未落地`。
- 不得把未运行的检查写成已通过。

## 7. 与提交规范的关系

本矩阵定义“做什么检查”，`commit-convention.md` 定义“如何记录结果”。

两者必须一致：commit message 的 `Slice` 与本矩阵的切片类型一致，`Validation` 字段只记录实际执行或确认的检查。提交标题的 `type(scope)` 遵循 Conventional Commits，不替代 LaneFlow 的 `Slice` 判断。
