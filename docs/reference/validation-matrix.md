# 验证矩阵

**文档状态**: Active  
**最后更新**: 2026-08-05

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

矩阵不要求所有 PR 跑同一组重复检查，但要求每次变更显式说明验证结论。Rust Core workspace 落地后，`core-runtime` 切片默认应运行 `cargo fmt --all -- --check` 与 `cargo test --workspace --locked`；其他技术栈检查在对应代码落地后逐步启用。仓库 CI 的 `Rust checks` job 按路径分流：非 Rust / 非 `schemas/` / 非 external-review 契约输入路径跳过重型 cargo；`schemas/` 与 `xtask` 嵌入的 external-review workflow / `docs/governance/github-workflow.md` 变更会跑 workspace 测试以覆盖 schema contract 与 trusted-ref 契约。Bevy `native-example` / Bevy benches / Bevy allocation 在 Bevy Adapter、其 Core/Spatial/Data/Scenario 依赖、example 制品、`Cargo.lock` / `Cargo.toml` 或 CI workflow 变更时启用。本地仍应按本矩阵主动运行与切片相关的命令，不能只依赖 CI skip。

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

| 场景                                                                                           | 预期状态 / 结果                                                                                    |
| ---------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------- |
| 无 review；仅 request、reaction 或任务启动                                                     | `awaiting_review` / `review_pending`，Fail                                                         |
| 仅有受信任 Codex provider 的无 reviewed SHA clean 摘要                                         | `provider_error`，Fail                                                                             |
| 旧无 SHA clean 摘要后有严格更晚、字段完整、绑定 current exact head 的 clean completion         | 旧歧义被 supersede，进入正常 completion/thread 状态机                                              |
| current-head clean completion 后又有无 SHA clean 摘要，或两者为 GitHub 同秒                    | `provider_error`，Fail；无法证明最终歧义已被覆盖                                                   |
| 只有 PR author self-review                                                                     | `awaiting_review`，Fail                                                                            |
| 受信任 reviewer 在 current head 完成 clean review                                              | `pass` 候选；仍需 threads、Checks 与 G3 comment                                                    |
| finding 未处置或仍有 unresolved actionable thread                                              | `findings_open`，Fail                                                                              |
| finding 已回复/resolve，但没有新的 clean re-review                                             | `awaiting_rereview`，Fail                                                                          |
| finding 处置后，受信任 reviewer 在 current head clean re-review                                | `pass` 候选                                                                                        |
| R1 resolve / unresolve 后未新增 `external-review: thread-state-changed` comment                | shadow 状态可能 stale，Fail；marker 触发 trusted metadata re-read 后再判断                         |
| clean completion 后 new push 或 review dismissed                                               | `stale`，Fail 并重新请求 review                                                                    |
| clean review 绑定旧 head，且没有已批准的等价例外                                               | `stale`，Fail                                                                                      |
| provider 文案正确但 actor 不在 allowlist                                                       | Fail                                                                                               |
| author 转贴 Cursor / 本地 Agent 输出                                                           | Fail                                                                                               |
| content-equivalent rebase 具备全部附加证据                                                     | `waived`；不得自动转成标准 `pass`                                                                  |
| 合并前纠正 current G3 comment                                                                  | 允许；以 REST `updatedAt` 重验，并新增更晚 marker                                                  |
| 合并后编辑或缺少可核验 `updatedAt`                                                             | Fail；不得改变历史证据，无法核验时失败关闭                                                         |
| Related PR B 自身仍由 main 上的旧 validator 判断                                               | 按 R0 bootstrap 人工核验 exact-head review；不得用候选 validator 自批                              |
| Related PR C 自身尚未把 shadow workflow 合入 main                                              | 使用 main 上的 live validator；候选 Check 不得自批，Check 缺失按 R0 bootstrap 记录                 |
| PR B 合入后的 R0/R1 PR 尚无 required External Review Gate                                      | `G3 Pass` / bootstrap 要求 live `pass`；结构化 `G3 Waived` 保持 `waived`，并记录 Check 缺失        |
| 多 Issue PR 使用 `G3 Waived`，但 waiver 缺少某个 Issue 或 `followUpIssue` 重复                 | Fail；每个关联 Issue 必须有一条唯一匹配的 `external-review-waiver:v1` 记录                         |
| waiver record set 含未声明 Issue，或 target / Delivery full-set 任一成员使用 waiver            | Fail；record set 必须精确匹配关联 Issue，Shadow 对任一 `G3 Waived` 永不发布 success                |
| Delivery closing set 含未声明 Issue，或 Related closing set 非空                               | Fail；target 的完整 closing set 必须与 PR 角色和全部 `关联 Issue` 精确一致                         |
| PR body 缺少明确 `关联 Issue` 列表、仍保留角色模板占位或角色与任一 Issue 元数据不一致          | `G3 Evidence Gate Shadow` Fail；多 Issue 必须逐个校验，不得猜测 target 参数                        |
| 多 Issue PR 的 G3 断言有缺失、重复或未声明 Issue / 参数的额外成功命令                          | Fail；完整断言命令集合必须与全部解析 target 精确相等                                               |
| 多 Issue PR 仅一个 Issue 使用 current `G3 Exception`                                           | 只允许该 Issue 的精确断言为“未通过”；其他 Issue 保持“已通过”并独立验证 exact-head 外部审阅         |
| historical exception 接受事件晚于 merge、但在实际 G4 evaluation time 已过期                    | Fail；merge 只判断事后接受顺序，不替代 G4 运行时的 freshness 判断                                  |
| G4 重放 pre-merge current exception 时 live base 已随 main 前移                                | 保留记录中的完整 pre-merge base OID；head 仍精确匹配，不与 post-merge live base 比较               |
| Dependabot 多 Issue metadata recovery 含一个 Exception 与其他 Pass                             | 先只恢复完整命令集；随后用 PR comment 记录按 Issue 复核各自结果                                    |
| 策略切换前已合并历史 PR 使用旧 `G3 Waived + confirmed_gate_defect`                             | 仅当 merge 早于 #405 G1 cutoff `2026-08-18T04:20:55Z` 时 grandfather G4 replay；G3/current 拒绝    |
| Delivery target 的 Related 成员角色/Issue 元数据错误、closing set 非空或断言集不闭合           | Fail；每个 current-policy Related 成员必须按自身 target 规则独立校验                               |
| Issue 的具体 PR 字段仍含 `pending / #61 / N/A`、`#<number>` 或空 `N/A` 原因                    | Fail；具体编号只接受明确 `#<number>` 列表，互斥模板选项必须清理                                    |
| G3 comment / body / Issue permalink 或关联 PR/Issue timeline 活动后未新增严格更晚的精确 marker | Fail；marker 须晚于 G3 effective time 和最终 evidence；同秒撤销 success                            |
| `G3 Evidence Gate Shadow` 字段缺失、重复、为空、嵌入无关 prose 或不符合三种规范值              | Fail；只接受 Check URL、带原因的 R1 non-required 或带边界的候选 workflow bootstrap                 |
| review/thread 状态变化或 Related 普通变化后未刷新 G3 shadow                                    | 仅遥测；non-marker 事件不发布新 Check，后续 marker 事件或显式 `check-gate-evidence g3/g4` 重新评估 |
| Delivery marker run 尚在 resolver，后发 Related body edit 或 close/reopen 往返先到 publisher   | marker 必须晚于 full-set 内全部 Related PR body 与 timeline；旧 marker 即使后到也只能 non-success  |
| success 候选首次校验后 evidence / marker 或 head/base 再变化                                   | 最终可信重验或 identity 复核失败；在原始已评估 head 发布 failure                                   |
| PR 从 `main` retarget 到其他 base，head 不变                                                   | 仅遥测；不向旧 head 补发 Check，后续 marker 或显式命令复核时按当前 base 失败关闭                   |
| Delivery full-set 含 CLOSED Related，或 state 与 `mergedAt` 不一致                             | Fail；只接受非 Draft OPEN current target 或带 `mergedAt` 的 MERGED 历史证据                        |
| 关联 Issue 已 closed，或当前 Delivery / Related PR 为 Draft、merged 或 closed                  | Fail；同仓不再 eligible 的 PR 不发布新 Check，标准 G3/G4 复核继续失败关闭，历史证据只由 G4 复核    |
| 候选 G3 validator / workflow 尚未合入 `main`                                                   | 不得用候选 `G3 Evidence Gate Shadow` 自批；使用当前 main validator 并记录 bootstrap 边界           |
| fork / cross-repository PR 的 head 不属于 base repository                                      | R1 不发布 Check 且不计 eligible sample；R2 前迁移到 same-repository PR 并重新 exact-head 审阅      |
| R2 PR 缺少 current-head External Review Gate success                                           | Fail                                                                                               |
| Check success 与 G3 comment 绑定不同 head，或 comment 早于最终 completion / Check              | Fail                                                                                               |

Provider fixtures 至少覆盖 Copilot clean/findings、Codex clean/findings、人工 `APPROVED`、仅有无 SHA、旧无 SHA 后严格更晚 current-head clean、current-head clean 后更晚无 SHA、无 SHA 与 clean 同秒、错误 actor、new-push stale、finding 后无复审、被编辑 completion、重复 thread 与 provider outage。历史事件 replay、live evaluator、trusted-ref shadow publisher 和人工审计必须与机器最终分类一致。

离线 fixture / replay 使用 `check-external-review --input <snapshot.json> --format json --expect <state>`；live 对照使用 `check-external-review --repo <owner/repo> --pr <number> --format json`。snapshot、结果 schema、固定状态枚举和 fail-closed 退出语义必须保持向后可识别；无法完整分页或二次读取 head/base 不一致时不得降级为 `awaiting_review` 或 `pass`。current G3 comment 缺少显式 `Gate 结果` 时必须失败；`G3 Waived` 还必须覆盖结构化 record、current head/base、reference-style evidence、授权人、当前 follow-up Issue、24 小时上限与过期回归。

workflow 安全检查至少验证：

- review / inline comment 事件只进入空权限、无 checkout 的 signal；trusted `workflow_run` 不读取 signal artifact / output；
- `workflow_run.pull_requests=[]` 时使用经校验的 run `head_sha` 调 commit-to-PR association API，只接受 open main PR；API 失败或 OID 非法必须 fail closed；
- validator 显式从 `refs/heads/main` checkout，关闭 credential persistence，不 checkout、下载或执行 PR head，不执行 comment body，不读取 repository secret；
- token 权限固定为 `contents: read`、`pull-requests: read`、`issues: read`、`checks: write`，第三方 Action 完整 SHA pin；
- R1 Check 固定为 `External Review Gate Shadow`，绑定 API 最终确认的 current head/base，并复核 telemetry source App=`github-actions`；external ID 同时绑定 PR/head/trusted-ref/run；
- `G3 Evidence Gate Shadow` 只解析有界 PR number，固定 checkout `refs/heads/main`，从 GitHub API 重读 PR / Issue，逐个校验全部关联 Issue，校验前后复核 head/base，确认角色参数稳定后重跑完整证据，并拒绝缺失/歧义 role、association、模板残留、closed Issue、Draft / merged current target、与角色/关联 Issue 不一致的完整 closing set，以及 Delivery full-set 中 CLOSED / 状态与 `mergedAt` 不一致的 Related PR；每个 current-policy Related 成员还必须独立验证自身 target metadata、空 closing set 与完整断言集；多 Issue comment 的断言命令集合必须与全部解析 target 精确相等；G3 查询不得请求只由 G4 使用的 `projectItems`；
- G3 evidence marker 只接受正文精确为 `g3-evidence: changed` 的新顶层 PR comment；marker 必须未编辑、属于当前 PR，并严格晚于 current G3 comment 的 effective time、当前 PR 与全部关联 Issue body 最后编辑时间，Delivery marker 还必须晚于 full-set 内全部 Related PR body；G3 comment 合并前编辑时以 REST 核验的 `updatedAt` 为 effective time 并使旧 marker 失效；只有该 created 事件且 Gate 结果为 `G3 Pass` / `R0-R1 bootstrap` 才可为直接目标发布 success，`G3 Waived` 在 marker 评估中只能发布 non-success；marker 级联 Delivery 目标发布 non-success 或最新结论；其他 conversation comment、marker edit/delete、review/thread/metadata/manual 属于仅遥测，不发布新 Check；直接事件从 resolver 前按 PR 串行，success 前完整重跑 target / marker 并再次复核 identity/eligibility；retarget 离开 `main` 属于仅遥测，不补发 Check；后续 marker 或显式命令按当前 base 失败关闭；marker 内容不进入 shell、不携带 Gate 结论；
- external-review signal、Related PR 普通事件和 Issue event 属于仅遥测，不发布新 Check；marker 事件仍从 open Issue 的 `Related PRs` 反向发现 Delivery PR 并级联复核，无治理字段返回空目标，元数据不完整或歧义时才保守刷新全部 open main PR；
- `G3 Evidence Gate Shadow` source App=`github-actions` 且 non-required；R2 必须改用 organization required workflow 或独立 App expected source，并通过同名 spoof canary、open PR 重触发和 ruleset before/after 审计；
- publisher 二次确认 `isCrossRepository=false`；fork / cross-repository PR 不尝试向 base repository 写入无法关联的 head Check；
- 只有 `pass -> success`；`waived -> action_required`，确保 required check 不会把 waiver 当作成功；其他状态均为 failure；
- PR concurrency 取消旧运行；identity race 不向新 head 发布旧结果；
- external ID 显式包含 evaluator state、稳定 fingerprint、trusted run 与 attempt；publisher 不查询或复用既有同名 Check，每个 trusted event 必须创建并验证自己的 receipt；
- Draft、非 `main` base、非 open PR 不计入 R1 sample；same-repository 的 ineligible 目标不发布新 Check，保留 eligible 时最后一次有效结论，cross-repository 不发布；标准 G3/G4 显式命令重新取得 success 仍需新 marker，不运行周期 schedule；
- R1 sample 同时绑定 trusted default-branch workflow run、external ID 与 Check receipt；PR 自定义的同名 GitHub Actions job 不得计入；
- R2 publisher 使用独立专用 GitHub App，ruleset 绑定正式 Check name 与 expected source App；恶意 PR 新增同名 Actions job 的 canary 仍必须阻断合并；
- R1 thread resolve / unresolve 批次必须以顶层 marker 唤醒 trusted `issue_comment` workflow；workflow 只接受 `created` 且正文精确匹配 marker 的 PR comment，edited / deleted 或其他 PR comment 不得刷新 shadow Check；R2 专用 App 必须实测 `pull_request_review_thread` / 等价自动信号覆盖两个方向，缺一则阻断 cutover；
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
10. G3 comment / Issue G3 permalink 不完整或 reference-style 定义未由对应 Gate 行实际引用，Related-only 阶段提前勾选 Issue G3，Related PR 独立 G3 未永久保留单一 `--related-pr <current-related-pr>` 断言，full-set 未使用 `--delivery-pr` 加全部 Related PR，或错误要求改写历史 Related comment，`Gate 断言` 未记录与实际调用完全一致的规范命令和 `已通过` 结果，标准 G3 的当前目标已经 merged / closed，或 `check-gate-evidence g3` / `check-gate-evidence-target` 失败。
11. `../governance/security-scanning.md` 要求的适用扫描仍为 `pending`、失败、无分析、已禁用或不可用，且没有显式例外；或把 API / 命令失败误写成零开放告警。
12. external review 缺失、pending、stale、actor/provider 不可信、finding 未完成 clean re-review，或只用 `reviewThreads=0` 证明 clean。
13. R2 激活后 current-head `External Review Gate` 未成功、source 不是 ruleset 绑定的专用 GitHub App，spoof canary 可由 PR 自定义同名 Actions job 满足，或 current G3 comment 的 effective time 不晚于 Check。
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
