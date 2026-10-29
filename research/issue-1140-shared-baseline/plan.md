# E0 冻结计划：六路共同基线（11 边 92 份候选池）与最小公共测试助手

> 状态：执行计划（Issue #1140，G0 已由维护者 2026-10-29 冻结六路任务书 v2 记录；G2 在本路设计/依赖就绪后另行记录）。工作树 `research/1140-shared-baseline`，起点 `origin/main` = `8336eb87`。共同口径沿[六路协调章程 v2](../six-streams-20261029/coordination.md)与[任务书 E0](../six-streams-20261029/lane-briefs.md)：生成物外置 Release、仓库只留计划/入口/摘要、共享代码本路单点修改（使用方复核）、结果 JSON 禁计时、行为等值仅限旧入口/旧输入/既有结论。方法事实源：[第 1 轮计划](../issue-1121-coverage-diagnosis/plan.md)、[第 2 批计划 §5](../issue-1134-cohort2-coverage-diagnosis/plan.md)。执行中与本计划不一致即停止并报 lead，不得单向迁就。

## 1. 基线冻结对象（候选池六来源合并链，顺序不得更换）

11 边 92 份候选，规范 f32 点位（整程序 `program_points` 逐点 `(x as f32).to_bits()` 位串）去重、先到先留、每边上限 16（沿 `dependency_recovery::experiment::merge` 逐字语义）。合并链与逐级断言（边数/份数）：

| 来源 | 输入（钉版载体） | 逐级断言 |
| --- | --- | --- |
| ① #1103 原候选池 | `research/issue-1103-s1-recovery-budget/results.json`（`tiers.32768.searches`，仓库内） | 6 边 48 份 |
| ② #1107 补搜原族 | `research/issue-1107-dependency-recovery/old-searches.json`（仓库内） | 7 边 56 份 |
| ③ #1111 校准网格 | `research/issue-1111-grid-calibration/positive-calibration.json#edges`（仓库内） | 7 边 72 份 |
| ④ #1115 曲线候选 | `research/issue-1115-failed-curve-design/searches.json`（仓库内） | 9 边 75 份 |
| ⑤ #1131 曲线候选 1 份 | Release 资产 `research-evidence-1134-cohort2-baseline` → `cohort2-baseline-inputs.zip::stream-a-searches.json`（1,204,062 字节，SHA-256 `1c6dd162…e26d7b`，经 `COHORT2_BASELINE_DIR` 读取） | 10 边 76 份 |
| ⑥ #1134 `-25332_7_0` 16 份 | #1134 冻结四轮 ABBA 逐字节一致的 `run-03` 输出 `old-searches.json` + `grid-searches.json`（经 `COHORT2_RUN_DIR` 读取；仅 `sumo::-25332_7_0` 有候选，原族 8 份在前、网格 8 份在后，规范 f32 点位 16 份全互异） | 11 边 92 份 |

最终逐边份数（11 边 92 份逐值，加载断言常量）：`sumo::-10004_5_1` 16、`sumo::-7348_3_0` 16、`sumo::-25332_7_0` 16、`sumo::-10034_0_0` 8、`sumo::-10130_8_0` 8、`sumo::-10130_16_0` 8、`sumo::-10130_18_0` 8、`sumo::-29388_3_0` 8、`sumo::-28860_3_0` 2、`sumo::-16502_0_0` 1、`sumo::-10564_3_0` 1。共同对照（接受 28/84、53 分量/84 路径逐值）由**结果对照锚**承载——#1134 run-03 `results.json#variants.new` 整行 = **92 池行**（锚代际错位事故返工后取材，见 §2 与 README 差异记录）；基线只冻结候选池，A–D 各自实验不吸收其他流新候选（章程 §3）。

## 2. 打包与外置（Release 资产 + MANIFEST，按基线包合并需求单）

- 资产：**`shared-baseline-inputs.zip`**，tag **`research-evidence-1140-shared-baseline`**（`research-evidence-*` 预发布），内含 `pool.json`、`results-anchor.json`、`MANIFEST.json`。三成员钉版：`pool.json`（1,299,422 字节，SHA-256 `31fca69617e2fd5d818aafc64b6704451475e05bf3aa3918391e1a2f9feb1bb3`）、`results-anchor.json`（278,322 字节，SHA-256 `0973dd355f18be878b977f3e2caea65ba355aeb7f1d0282e624e4b62b8b1abe5`）、`MANIFEST.json`（4,040 字节，SHA-256 `56edd1365c48ce68b39e260f5be01ecea8629ee92e424d58363bd718e2d00cf5`）。来源⑥两文件同钉版：`old-searches.json`（157,603 字节，SHA-256 `011db49fad061d945ab7d21858d73961d8234e2955450fd4f040269d04309dba`）、`grid-searches.json`（1,313,285 字节，SHA-256 `7b8b2e1f28f1d7e47aed6ff5222086971a1ae2838dad07559ff03efd94513fe9`）；锚取材源钉版：run-03 `results.json`（631,650 字节，SHA-256 `b320cfe9452edb87cd17a2186338fc0fef077ba141c937a626c56686b970c25f`）。
- `pool.json`（**形状定案 = A 路消费澄清**）：与历史候选来源同构的行形状 `{edge: {"candidates": [{"program","arc_m","spans"}]}}`（`experiment::pool` 可直接消费），11 边 92 份候选数组保持合并顺序；规范 f32 点位去重键由程序点位重算校验（包内不另存），逐份来源标记移入 MANIFEST `candidate_sources`。
- `results-anchor.json`（锚代际错位事故返工后）：#1134 run-03 `results.json#variants.new` **整行逐值**（**92 池行**：`cohort_joint_acceptance`/`cohort_path_counts`/`cohort_paths`/`components` 53/`path_counts` 28/84/`paths` 84/`pool_sizes`）。`source` 为**取材身份三件套 + 代际 + 变更记录**：`asset: evidence/run-03/results.json` + `sha256`/`bytes`、`json_pointer: #variants.new` + `generation: "92-池行"`、`release_tag: research-evidence-1140-shared-baseline`；`changed_from` 记旧取材（`#variants.new@stream-a-results.json`，`research-evidence-1134-cohort2-baseline`，76-池行）与根因（两代唯一语义差 = C40 missing_edges 6→5）。代际守卫：C40 组件行 `missing_edges`=5（76 池旧代 6 拒收，单测钉版）。**仅结果对照，不作候选点位来源**。
- `MANIFEST.json`（形状同 A 路）：`files.<name>.{sha256,bytes}`（A 路代码内哈希断言读取点）+ `merge_order: ["1103","1107","1111","1115","1131","1134"]` + `per_stage_counts` + `final`（11/92 与逐边份数）+ `sources`（六来源钉版链）+ `candidate_sources` + 生成说明。
- 打包/校验由入库 Rust `cfg(test)` 研究入口执行（§5）；本机打包脚本不入库，暂存于 `target/issue-790-806-handoff-20261026/e0-shared-baseline-20261029/`；仓库只留本计划、README 摘要与入口。

## 3. 读取口径（env + 代码内哈希/顺序断言，A–D 唯一基线入口）

- 环境变量 **`SIX_STREAMS_BASELINE_DIR`** 指向 `shared-baseline-inputs.zip` 解压目录；`COHORT2_BASELINE_DIR` / `COHORT2_RUN_DIR` 仅打包与逐值重建校验使用（来源⑤/⑥）。**旧 9/10 边池禁用**：消费方只允许经本入口取池。
- 代码内断言（`calibration::shared_baseline`，常量钉版）：① `pool.json`/`results-anchor.json`/`MANIFEST.json` 三文件 SHA-256 与本计划 §2 钉版值一致（写入什么字节即校验什么字节，身份不依赖仓库内文件）；② `merge_order` 与 `per_stage_counts` 逐值等于冻结链；③ 11 边逐边份数与总数 92 逐值；④ 每边 ≤16、规范 f32 点位键由程序点位重算且逐边唯一、`program` 位逐字可经 `frozen_candidate` 往返；⑤ 六来源重建（仓库内 ①–④ + env 钉版 ⑤/⑥）与包内候选**逐值逐序**一致（11 边 92 份）；⑥ 结果对照锚 = `run-03 results.json#variants.new` 整行逐值（接受 28/84），取材三件套（asset/json_pointer/release_tag）+ 代际标签 `92-池行` + 取材源 SHA-256 逐项断言（A 路入口同款三元组断言）。非钉版字节拒绝读取。
- 排序/去重语义不另立实现：合并沿 `dependency_recovery::experiment::{pool, merge}`；包内顺序即合并顺序，任何来源缺失、哈希不符或逐值不符 → §7 停止并报 lead。

## 4. 最小公共助手（需求单范围；共享代码本路单点修改）

需求单最小集（需求方 A(#1136)/B(#1137 方案冻结后)/C(#1138)/D(#1139) 复核）：

- **池读取/断言**：`shared_baseline::shared_pools() -> combinations::Pools`（§3 全断言后返回；旧 9/10 边池禁用）。
- **词汇映射**：`plan_stop_label`（计划词汇映射，原始标签保留在各 `stop_reason`）。
- **载体三层链**：`span_control`（#1134 §7 授权三层：fresh 两族 → 1107 history → `repair_curve` 只读重放回退）及其 `find_span_control`/`replay_span_controls`/`SpanControl` 构件。
- **网格节点**：`grid_node_member`。
- 上述助手自 `calibration::coverage` 以再导出暴露到 `dependency_recovery` 作用域（仅可见性、语义逐字不变），A–D 复用冻结实现、不建平行分叉；`edge_input`/`coverage_output` 为内部依赖不对外（需求单增补再扩）。**行为等值范围**：仅限旧入口、旧输入与既有结论——既有 `cohort2_*`/`coverage_*`/`stream_*` 测试不经修改全部通过即为等值证明；新增能力（本基线加载器与 E1 新诊断）另行证明正确性（E1 归 E1 方案）。各路增量需求汇入 Issue #1140 评论，另行增补并重新等值复核，不并入本次冻结。

## 5. 入口（`cfg(test)`，tools/laneflow-lust-converter）

- `calibration::shared_baseline::shared_baseline_export`（`#[ignore]`，env：`SHARED_BASELINE_EXPORT_DIR` 全新目录 + `COHORT2_BASELINE_DIR` + `COHORT2_RUN_DIR`）：按 §1 合并链重建 92 份池（逐级断言 + 逐份来源标记），写 `pool.json` + `results-anchor.json` + `MANIFEST.json`（§2 形态；本计划 §2 钉版 SHA-256 于打包后回填代码常量）。
- `calibration::shared_baseline::shared_baseline_package_matches_frozen_chain`（`#[ignore]`，env：同上 + `SIX_STREAMS_BASELINE_DIR`）：§3 全断言 + 六来源重建逐值逐序 + 锚整行逐值对照。
- 常规（非 ignore）单测（合成输入/临时目录，不依赖外部资产）：`shared_baseline_merge_dedups_canonical_points_in_frozen_order`（去重键=规范 f32 点位、先到先留、合并顺序保留、来源标记）、`shared_baseline_facade_exposes_required_helper_set`（需求单最小集契约签名钉版）、`shared_baseline_loader_rejects_unpinned_bytes`（非钉版字节拒读）、`shared_baseline_anchor_rejects_old_generation_row`（锚代际守卫：76 池行拒收）。

## 6. 验证与证据

- 行为等值：改动前后各跑 `cargo +1.99.1 test --locked -p laneflow-lust-converter --lib`，既有测试名集合与结论不变（旧入口/旧输入/既有结论）；共享改动仅限 §4 再导出可见性。使用方（A–D）复核记录由 lead 汇总。
- 打包确定性：export 两次（不同全新目录）非计时输出逐字节一致（`fc.exe /b`）；MANIFEST 哈希与 `Get-FileHash` 实测一致；Release 资产下载回读 SHA-256 一致。
- 结果 JSON（包与 MANIFEST）无任何计时字段；耗时只写本机暂存 `timing.json`（不入库、不入资产）。九项必需检查与 trailer（按实际执行的 Harness/Model）在 PR 阶段执行；README/g1-proposal 随交付。
- 打包/核验证据暂存 `target/issue-790-806-handoff-20261026/e0-shared-baseline-20261029/`（不入库）。

## 7. 停止条件

任一来源缺失、SHA-256/字节数与钉版不符、逐级计数或逐边份数不符、规范 f32 点位重复、往返重建不一致、既有测试结论变化、或与本计划口径冲突 → 停止执行、归档中止产物（`*-aborted-*`）、报 lead 裁决，不得单向迁就。E0 交付后由 lead **独立验收冻结**，A–D 实验前置才解除；本计划不改生产采样、compiler、数据格式、阈值、原成功几何/拓扑/控制锚点/端点钳制方向，不提高预算。
