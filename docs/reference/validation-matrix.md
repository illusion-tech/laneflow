# 验证矩阵

**文档状态**: Active
**最后更新**: 2026-10-12

**适用范围**: LaneFlow 各切片类型在合并前的最小验证要求，以及 CI 失败后的偶发签名判读
**关联文档**:

- 上游治理:
  - `../governance/development-gates.md`
  - `../governance/dependency-security.md`
  - `../governance/security-scanning.md`
  - `../governance/github-workflow.md`（偶发失败 triage 口径，第 8.2 节）
  - `commit-convention.md`
  - `../adr/0026-merge-governance-rebuild.md`
  - `../adr/0027-retire-external-review-check.md`
- 模板:
  - `../../.github/pull_request_template.md`

## 1. 目标

本文把 `development-gates.md` 中“按 PR 验证切片类型验证”的要求收敛为一张可执行矩阵；
切片类型不等同于 GitHub 原生 Issue Type 或 `area:*` 标签。本矩阵回答每种切片：

- 哪些检查必须做。
- 哪些检查通常不需要。
- 无法运行时如何记录。

矩阵不要求所有 PR 跑同一组重复检查，但要求每次变更显式说明验证结论。Rust workspace
落地后，`core-runtime` 切片默认应运行 `cargo fmt --all -- --check` 与
`cargo test --workspace --locked`。仓库 CI 的 `Rust checks` job 按路径分流：非 Rust 契约输入
路径跳过重型 cargo。道路编辑 FlatBuffers 变更走独立 Codegen，不因 `.fbs` 拉起整仓
Rust 测试。本地仍应按本矩阵主动运行与切片相关的命令，不能只依赖 CI skip。

## 2. 切片类型到验证矩阵

| 切片类型         | 必须的验证                                                                                                    | 通常不需要                                  |
| ---------------- | ------------------------------------------------------------------------------------------------------------- | ------------------------------------------- |
| `docs-only`      | 文档可读性检查、链接有效、无行为变更声明                                                                      | build、单元测试、schema 校验                |
| `governance`     | 模板/路径/引用一致性、受影响流程说明；涉及 workflow/ruleset 时复核权限与 GitHub 实际状态                      | 运行时测试                                  |
| `core-runtime`   | `cargo fmt --all -- --check`、`cargo test --workspace --locked`、确定性行为说明、Traffic Runtime API 影响说明 | adapter build、示例 smoke（除非影响主路径） |
| `data-spec`      | schema/格式校验、兼容性与版本影响、示例数据影响                                                               | adapter build（除非协议联动）               |
| `adapter`        | adapter build、手工场景验证、transform 同步验证、Traffic Runtime 依赖方向检查                                 | 跨引擎全量测试（除非显式要求）              |
| `authoring-tool` | 工具运行验证、输出数据可被 Traffic Runtime 消费、格式一致性                                                   | 引擎端 build                                |
| `example`        | 示例可运行说明、覆盖能力说明、所依赖数据格式版本                                                              | 完整单元测试套件                            |
| `cross-layer`    | 以上相关项全部适用、端到端路径验证、是否需要示例 smoke 的显式判断                                             | 无默认豁免                                  |

## 3. Markdown 表格格式

凡新增或修改含 GFM 表格的 Markdown，使用仓库内同一实现格式化：

```powershell
cargo +1.98.1 run --locked -p xtask -- format-md-tables <path...>
```

只读检查：

```powershell
cargo +1.98.1 run --locked -p xtask -- format-md-tables --check <path...>
```

CI 的 `Markdown tables` job 对协作范围内的 Markdown 执行相同检查，但只警告，不阻断合并。

## 4. Review conversation

普通 GitHub Review 继续用于协作，但不由自定义 Check、受信名单或 reaction 计数。
当前 Ruleset 不要求固定批准数或 CODEOWNERS review；未解决 review conversation 由
GitHub 原生 `required_review_thread_resolution: true` 阻止入队。

| 场景                            | 结果                                   |
| ------------------------------- | -------------------------------------- |
| 没有 Review 或 reaction         | 不影响七项机器 Check                   |
| 存在未解决 review conversation  | GitHub 原生规则阻止入队或合并          |
| 所有 review conversation 已解决 | 对话条件满足，仍须等待 required checks |
| `main` 前进但 `H_pr` 未变       | 重建 `H_mg` 并重跑七项机器检查         |
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

## 8. 偶发失败签名对照表

本表是偶发失败签名与判读方法的单一事实源；处置流程见
`../governance/github-workflow.md` 第 8.2 节。每行可反向索引追踪 issue，结论以追踪
issue 为准，本表只做判读入口，不复制诊断结论。

| 签名（断言 / 错误面）                                                                                               | 出处                                                                              | 匹配谓词                                                                                          | 判读方法                                                                                                                                                                                 | 状态 / 追踪                            |
| ------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------- |
| `Validation { stage: "file-backed emission", detail: "Format(DigestMismatch { structure: ChunkDirectoryEntry })" }` | urban-harness → `laneflow-format` 读回校验                                        | 待归因（无谓词，不具偶发裁决资格）                                                                | 读 `[digest-probe]` 行（entry/offset/长度、期望 vs 实际摘要、内容头 32 字节）；失真指纹（首坏偏移页对齐、整页替换、页整数倍位移、零页）命中即按环境对照裁决（独立写-读浸泡 A/B）         | 已归因（机器级页失真，非代码）· #794   |
| `artifact digest mismatch: <name>`（实测 `Validation("artifact digest mismatch: network.lfca")`）                   | urban-harness `Artifacts::load` 对生成器刚写出工件的读回摘要核对                  | 待归因（无谓词，不具偶发裁决资格；长度与 SHA-256 可双失配）                                       | 读 `[digest-probe]` `site=write-readback` / `site=artifact-load` 行（写入 vs 回读长度与摘要、首个分歧偏移、分歧形态、二次回读稳定性）；失真指纹命中即按环境对照裁决（独立写-读浸泡 A/B） | 已归因（机器级页失真，非代码）· #794   |
| 写后读回失配（`[digest-probe] site=write-readback` 行 / `save write-readback` 错误面）                              | urban-generator / urban-harness 写后读回校验（#954）                              | 重试恢复即命中（`action=retry` 后写入成功）                                                       | **重试成功不得作为偶发放行依据**：留档 `[digest-probe]` 现场行并按环境失真指纹判读；重试耗尽（`action=fail-closed`）为真实失败，修复后方可合入                                           | 已收口机制（#954）· 环境失真见指纹     |
| `warm_boundary_queries_do_not_rescan_when_unrelated_edges_grow: assertion failed: left: N`                          | `placement_admission` warm 重扫计数                                               | —（已修，任何命中即回归）                                                                         | 病因为并发测试污染进程全局 `MANEUVER_UPSTREAM_EDGE_VISITS`                                                                                                                               | 已修（#796，thread-local）· 再现即回归 |
| `spawn_overlap_allocation.rs:123/124 assertion ... failed` 同 tid 双 panic                                          | `allocation_guard_rejects_real_heap_operations` 的 `catch_unwind` 预期失败 stderr | 该二进制 `test result: ok`，且双 panic 同 tid 成对出现                                            | 误报判别：看该二进制 `test result:` 行，`ok. 2 passed` 即非失败                                                                                                                          | 误报已撤销（#794 症状①）               |
| `steady Waiting ticks allocated` / `repeated admission sample N allocated`                                          | `waiting_budget_evidence` 零分配窗口                                              | 待归因（无谓词，不具偶发裁决资格；已知一次实例为 4 alloc / 0 realloc）                            | 读 `[tick-ledger]` 逐拍行与暖机余量，判别懒物化漂移 / 他面串扰 / 计量边界                                                                                                                | 未归因，探针就位（#893）· #794         |
| `armed steady ticks must not allocate beyond unarmed steady ticks`                                                  | `cutover_cross_budget_evidence` 稳态账本不等                                      | 待归因（无谓词，不具偶发裁决资格；已知一次实例为 unarmed 侧孤立 4 alloc / 900 B 对 armed 侧全 0） | 同上 `[tick-ledger]`；left=unarmed、right=armed                                                                                                                                          | 未归因，探针就位（#893）· #794         |
| `per-world live N must stay below M bytes`                                                                          | `shared_network_allocation_evidence` 每世界预算                                   | 分支 diff 存在对应的每世界真实增长（如 +4 B/世界）即真超限，否则待裁决                            | 先对比分支 diff 是否真实增长（实测有 +4 B/世界的研究分支实例），再判串扰                                                                                                                 | 多为真预算超限 · #794                  |
| `NonFiniteMotion` 家族偶发                                                                                          | `phase_equivalence` 等                                                            | —（已修，任何命中即回归）                                                                         | 故障注入跨武装世界引爆                                                                                                                                                                   | 已修（#795）· #792                     |
| 取样脚本报 FAILED 但目标 `test result: ok`                                                                          | 自建循环取样脚本                                                                  | 目标二进制 `test result: ok` 即脚本误报                                                           | 检查匹配是否大小写不敏感、是否误计 `catch_unwind` 预期失败 stderr                                                                                                                        | 方法论教训（#794 iter 5）              |

放行裁决与收录机制：

- **裁决口径分三类**：表内匹配谓词逐项吻合 → 偶发裁决成立，留档后重跑放行；无谓词行
  （标注「待归因」）命中**保持阻塞**，由维护者依现场数据与探针输出显式裁决（记录进
  追踪 issue）后方可重跑放行，同签名重复命中不予裁决并升级真回归；谓词外命中与未入表
  签名一律真回归，**修复后方可合入**。已修行的任何命中一律是回归。
- **环境失真指纹**（#794 归档）：首个分歧偏移精确页对齐、整页替换（约 4 千字节级分歧
  达随机巧合水平）、位移量为 4 KiB 页的整数倍、伴随零页丢失，且随内存压力升高命中率
  上升——命中即先跑环境对照（独立写-读浸泡 A/B，不经 LaneFlow 代码）排除机器故障，
  对照实验自身输出须双路径读回互证后再采信。
- 收录机制：追踪 issue 收口时，把最终签名、匹配谓词与判读方法回写本表并在实现 PR 中
  索引；修复后保留行并标注「已修（`<PR>`）· 再现即回归」，不删行。新增签名未入表前
  一律按真回归处理。
