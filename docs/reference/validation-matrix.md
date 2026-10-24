# 验证矩阵

**文档状态**: Active
**最后更新**: 2026-10-25

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
cargo +1.99.1 run --locked -p xtask -- format-md-tables <path...>
```

只读检查：

```powershell
cargo +1.99.1 run --locked -p xtask -- format-md-tables --check <path...>
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

| 签名（断言 / 错误面）                                                                                               | 出处                                                                              | 匹配谓词                                                                                          | 判读方法                                                                                                                                                                                                                            | 状态 / 追踪                            |
| ------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------- |
| `Validation { stage: "file-backed emission", detail: "Format(DigestMismatch { structure: ChunkDirectoryEntry })" }` | urban-harness → `laneflow-format` 读回校验                                        | 待归因（无谓词，不具偶发裁决资格）                                                                | 读 `[digest-probe]` 行（entry/offset/长度、期望 vs 实际摘要、内容头 32 字节）与 `[digest-probe] dump` 转储，按 §8.1 离线页指纹判读；`page-fingerprint` 即按环境对照裁决（独立写-读浸泡 A/B），`code-suspect` 转代码嫌疑             | 已归因（机器级页失真，非代码）· #794   |
| `artifact digest mismatch: <name>`（实测 `Validation("artifact digest mismatch: network.lfca")`）                   | urban-harness `Artifacts::load` 对生成器刚写出工件的读回摘要核对                  | 待归因（无谓词，不具偶发裁决资格；长度与 SHA-256 可双失配）                                       | 读 `[digest-probe]` `site=write-readback` / `site=artifact-load` 行（写入 vs 回读长度与摘要、首个分歧偏移、分歧形态、二次回读稳定性）；失真指纹命中即按环境对照裁决（独立写-读浸泡 A/B）                                            | 已归因（机器级页失真，非代码）· #794   |
| 写后读回失配（`[digest-probe] site=write-readback` 行 / `save write-readback` 错误面）                              | urban-generator / urban-harness 写后读回校验（#954）                              | 重试恢复即命中（`action=retry` 后写入成功）                                                       | **重试成功不得作为偶发放行依据**：留档 `[digest-probe]` 现场行并按环境失真指纹判读；重试耗尽（`action=fail-closed`）为真实失败，修复后方可合入                                                                                      | 已实施机制（#954）· 环境失真见指纹     |
| 写出读回失配重新写出（`[digest-probe] site=emission-retry` 行）                                                     | urban-generator 写出阶段的有界重新写出（#1036；后发射检查失配不重写）             | 重写恢复即命中（`action=retry` 后生成成功，`measurements.toml` 的 `emission_rewrites` 非零）      | **重写成功不得作为偶发放行依据**：留档现场行与同轮 `[digest-probe] dump` 转储，按 §8.1 判读；`code-suspect` 转代码嫌疑；重写耗尽（`action=fail-closed`）按上方 `file-backed emission` 行处置                                        | 已实施机制（#1036）· 环境失真见指纹    |
| 流式写后核对失配（`[digest-probe] site=stream-readback` 行 / `写后读回与写出内容不一致` 错误面）                    | urban-harness 运行产物写时摘要与写后读回核对（#1042）                             | 逐拍计时文件重写恢复即命中（`action=retry` 后运行成功）；流式日志与证据文件失配即本次运行失败     | **重写成功不得作为偶发放行依据**：留档现场行（写出 / 读回摘要与长度、首个不一致页偏移、不一致页数）；流式文件没有源字节，不写转储，失配文件留在输出目录，可与同计划另一次运行的同名文件按页对照；`reason=source-changed` 转代码嫌疑 | 已实施机制（#1042）· 环境失真见指纹    |
| `warm_boundary_queries_do_not_rescan_when_unrelated_edges_grow: assertion failed: left: N`                          | `placement_admission` warm 重扫计数                                               | —（已修，任何命中即回归）                                                                         | 病因为并发测试污染进程全局 `MANEUVER_UPSTREAM_EDGE_VISITS`                                                                                                                                                                          | 已修（#796，thread-local）· 再现即回归 |
| `spawn_overlap_allocation.rs:123/124 assertion ... failed` 同 tid 双 panic                                          | `allocation_guard_rejects_real_heap_operations` 的 `catch_unwind` 预期失败 stderr | 该二进制 `test result: ok`，且双 panic 同 tid 成对出现                                            | 误报判别：看该二进制 `test result:` 行，`ok. 2 passed` 即非失败                                                                                                                                                                     | 误报已撤销（#794 症状①）               |
| `steady Waiting ticks allocated` / `repeated admission sample N allocated`                                          | `waiting_budget_evidence` 零分配窗口                                              | 待归因（无谓词，不具偶发裁决资格；已知一次实例为 4 alloc / 0 realloc）                            | 读 `[tick-ledger]` 逐拍行与暖机余量，判别懒物化漂移 / 他面串扰 / 计量边界                                                                                                                                                           | 未归因，探针就位（#893）· #794         |
| `armed steady ticks must not allocate beyond unarmed steady ticks`                                                  | `cutover_cross_budget_evidence` 稳态账本不等                                      | 待归因（无谓词，不具偶发裁决资格；已知一次实例为 unarmed 侧孤立 4 alloc / 900 B 对 armed 侧全 0） | 同上 `[tick-ledger]`；left=unarmed、right=armed                                                                                                                                                                                     | 未归因，探针就位（#893）· #794         |
| `per-world live N must stay below M bytes`                                                                          | `shared_network_allocation_evidence` 每世界预算                                   | 分支 diff 存在对应的每世界真实增长（如 +4 B/世界）即真超限，否则待裁决                            | 先对比分支 diff 是否真实增长（实测有 +4 B/世界的研究分支实例），再判串扰                                                                                                                                                            | 多为真预算超限 · #794                  |
| `NonFiniteMotion` 家族偶发                                                                                          | `phase_equivalence` 等                                                            | —（已修，任何命中即回归）                                                                         | 故障注入跨武装世界引爆                                                                                                                                                                                                              | 已修（#795）· #792                     |
| 取样脚本报 FAILED 但目标 `test result: ok`                                                                          | 自建循环取样脚本                                                                  | 目标二进制 `test result: ok` 即脚本误报                                                           | 检查匹配是否大小写不敏感、是否误计 `catch_unwind` 预期失败 stderr                                                                                                                                                                   | 方法论教训（#794 iter 5）              |

`sustained` 测试夹具另输出 `[digest-probe] site=fixture-copy name=<文件>`（#1043）：
`phase=source` 核对复制源，按首次写出登记最多重建同份 pristine 三次；
`phase=before-edit` / `before-compare` 在解析或比较前核对写时摘要，失配即失败。
现场包含期望 / 实际摘要与长度、实际头 32 字节及路径；非预期 panic 保留临时目录。
目标复制使用 #954 的写后核对与最多三次重写，目标失败不触发源重建。
成功轮次的 `[fixture-copy]` 行分别记录复制次数、源重建次数，以及初始生成、重建、
目标复制和故意篡改写出的重写次数。重建或重写成功仍须留档；
`reason=changed-registration` 表示重建内容偏离首次登记，转代码嫌疑，不采纳新摘要。

放行裁决与收录机制：

- **裁决口径分三类**：表内匹配谓词逐项吻合 → 偶发裁决成立，留档后重跑放行；无谓词行
  （标注「待归因」）命中**保持阻塞**，由维护者依现场数据与探针输出显式裁决（记录进
  追踪 issue）后方可重跑放行，同签名重复命中不予裁决并升级真回归；谓词外命中与未入表
  签名一律真回归，**修复后方可合入**。已修行的任何命中一律是回归。
- **环境失真指纹**（#794 归档）：首个分歧偏移精确页对齐、整页替换（约 4 千字节级分歧
  达随机巧合水平）、位移量为 4 KiB 页的整数倍、伴随零页丢失，且随内存压力升高命中率
  上升——命中即先跑环境对照（独立写-读浸泡 A/B，不经 LaneFlow 代码）排除机器故障，
  对照实验自身输出须双路径读回互证后再采信。
- 收录机制：追踪 issue 完成时，把最终签名、匹配谓词与判读方法回写本表并在实现 PR 中
  索引；修复后保留行并标注「已修（`<PR>`）· 再现即回归」，不删行。新增签名未入表前
  一律按真回归处理。

### 8.1 摘要失配转储的取证与离线判读

`laneflow-format` 读回校验失配时，失配点只剩失真字节，原始字节已不在手；页指纹须以
健康的确定性产物为金样本离线对照（#794）。

1. **取证**：`cargo test` / `cargo run` 启动的进程（可执行文件位于含 `CACHEDIR.TAG`
   的 cargo target 目录下）失配时默认落盘到该 target 目录的 `digest-probe/`：
   `laneflow-digest-probe-<entry>-<offset>-<实际摘要前 16 位>.bin` 与同名 `.probe`
   旁文件（entry、offset、长度、期望与实际摘要），现场行为 `[digest-probe] dump path=...`。
   设置 `LANEFLOW_DIGEST_PROBE_DIR` 时改落该目录；安装到 target 之外的二进制不写盘。
   按实际摘要命名使确定性的负例测试（如 `laneflow-format` writer 摘要失配测试）只反复
   覆盖固定文件，不随运行次数占用名额；环境失真每次内容不同，各自留档。目录内不同
   转储达 16 份后跳过新内容并输出 `dump skipped`，判读留档后人工清空。只在失配分支
   写盘，不改变失败语义。仓库 cargo 配置禁止 `[env]` 段（`check-wire-audit`），故不以环境
   变量注入默认目录。
2. **金样本**：用失败测试所用的配置与规模重新生成健康产物，例如 urban-harness 夹具：

   ```powershell
   cargo +1.99.1 run --locked -p laneflow-urban-generator -- --config examples/config/cn-urban.toml --scale fixture --output <新目录>
   ```

   产物逐字节确定。命令按目录项中的期望摘要定位 chunk 并以摘要自校验，找不到即说明
   金样本配置不对，不得改用其他产物凑数。
3. **判读**：

   ```powershell
   cargo +1.99.1 run --locked -p xtask -- classify-digest-probe --dump <target 目录>/digest-probe/<转储>.bin --golden <金样本目录>
   ```

   命令先以旁文件的 `actual` 核对转储摘要；不一致输出 `verdict=evidence-mismatch` 并以
   失败退出，说明转储在落盘后已改变，不作判读，按无转储处理。核对通过后按 4 KiB 文件
   页输出首个分歧偏移、每页差异与页分类：完整页为 `Zero` 零页（全零，或非零字节至多
   16 个且至少 256 个原非零字节被清零的近零页）、`Shifted(k)` 与相距
   k 页的金样本页逐字节相同、`Replaced` 页均分 16 段后至少 12 段有差异、`Partial` 差异
   只落在少数分段；chunk 首尾不满一页的残片一律为 `Fragment`，不作任何一方的证据。
   结论：`code-suspect`（存在 `Partial` 或长度不符）→ 转代码嫌疑；否则
   `page-fingerprint`（至少一个完整页为前三类）→ 按环境失真裁决并跑环境对照；
   `inconclusive`（差异只落在残片上）→ 证据不足，保持待裁决；`identical` → 转储字节
   与金样本一致，失真未进入转储。
4. **留档**：把 `[digest-probe]` 现场行与判读输出原样贴回追踪 issue，再按本节放行口径
   裁决。

产物零字节较多，替换页与原页常有大量字节巧合相同：整页替换的差异字节可远低于 4 千、
首个分歧也可能落在页内前几个字节，故 `Replaced` 按差异覆盖的分段而非差异字节数判定；
页两端的零散翻转只覆盖少数分段，判 `Partial`。写丢失页常残留个别非零字节（归档与
零代码浸泡实测为 4,095 / 4,096 字节为零），原页零段多时差异同样只覆盖部分分段，故按
近零页归入 `Zero`；被清零的原内容不足 256 字节时仍判 `Partial`，零字节居多的页上的局部
清零不按写丢失认。判读以页分类为准，`page_aligned` 只作参考。
