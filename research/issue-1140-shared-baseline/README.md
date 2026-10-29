# 六路共同基线与最小公共测试助手（E0 交付）

**结论：E0 交付完成，等待 lead 独立验收冻结（A–D 实验前置仅此一项）。** 共同基线 = 11 边 92 份候选池（六来源按序合并、规范 f32 点位去重、顺序冻结），打包外置 Release [`research-evidence-1140-shared-baseline`](https://github.com/illusion-tech/laneflow/releases/tag/research-evidence-1140-shared-baseline)（`research-evidence-*` 预发布，资产 `shared-baseline-inputs.zip`，79,689 字节，SHA-256 `b30b5c6e751e2590f8a4e18eaa92211533ccdb429d91fe09df7536bd54b12e54`，下载回读一致）；仓库只留[计划](plan.md)、摘要与 `cfg(test)` 入口。读取经 `SIX_STREAMS_BASELINE_DIR` + 代码内 SHA-256/合并顺序/逐边份数断言，**旧 9/10 边池禁用**。包形状与 Issue #1140 需求单及 A 路消费澄清**逐项定案一致**（见「形状定案」）。结果对照锚接受 **28/84**（53 分量 / 84 路径）逐值。生产 S1 的 G1 保持 Proposed（[决策材料](g1-proposal.md)）。

本研究对应 #1140（六路协调任务书 v2 的 E0 节点，协调章程 `research/six-streams-20261029/coordination.md`），起点 `origin/main` = `8336eb87`。本切片不改生产采样、compiler 配置档、数据格式、阈值、原成功几何/拓扑/控制锚点/端点钳制方向与任何依赖；共享代码改动仅限测试助手**可见性放宽**（语义逐字不变，见「行为等值」）。

## 冻结与证据

**六来源合并链**（规范 f32 点位 = 整程序 `program_points` 逐点 `(x as f32).to_bits()` 位串；去重先到先留、每边上限 16，逐字沿 `dependency_recovery::experiment::{pool, merge}`；顺序不得更换）：

| 级 | 来源 | 钉版载体 | 逐级断言 |
| --- | --- | --- | --- |
| ① `1103` | 原候选池 | `research/issue-1103-s1-recovery-budget/results.json`（仓库内） | 6 边 48 份 |
| ② `1107` | 补搜原族 | `research/issue-1107-dependency-recovery/old-searches.json`（仓库内） | 7 边 56 份 |
| ③ `1111` | 校准网格 | `research/issue-1111-grid-calibration/positive-calibration.json#edges`（仓库内） | 7 边 72 份 |
| ④ `1115` | 曲线候选 | `research/issue-1115-failed-curve-design/searches.json`（仓库内） | 9 边 75 份 |
| ⑤ `1131` | 曲线候选 1 份 | Release `research-evidence-1134-cohort2-baseline` → `cohort2-baseline-inputs.zip::stream-a-searches.json`（1,204,062 字节，SHA-256 `1c6dd162…e26d7b`） | 10 边 76 份 |
| ⑥ `1134` | `sumo::-25332_7_0` 16 份 | #1134 冻结四轮 ABBA 逐字节一致的 run-03 `old-searches.json`（157,603 字节，SHA-256 `011db49f…09dba`）+ `grid-searches.json`（1,313,285 字节，SHA-256 `7b8b2e1f…13fe9`；原族 8 份在前、网格 8 份在后，16 份规范 f32 点位全互异） | **11 边 92 份** |

最终逐边份数（加载断言常量）：`sumo::-10004_5_1` 16、`sumo::-7348_3_0` 16、`sumo::-25332_7_0` 16、`sumo::-10034_0_0` 8、`sumo::-10130_8_0` 8、`sumo::-10130_16_0` 8、`sumo::-10130_18_0` 8、`sumo::-29388_3_0` 8、`sumo::-28860_3_0` 2、`sumo::-16502_0_0` 1、`sumo::-10564_3_0` 1。

**资产钉版**（身份不依赖仓库内文件；写入什么字节即校验什么字节）：`pool.json` 1,299,422 字节 SHA-256 `31fca696…1bb3`；`results-anchor.json` 256,071 字节 SHA-256 `ff5276fc…acfc`；`MANIFEST.json` 3,381 字节 SHA-256 `8b5d8021…be66`（完整值见 plan §2 与代码常量）。MANIFEST 含 `files.<name>.{sha256,bytes}`、`merge_order`、`per_stage_counts`、`final`、六来源钉版链 `sources`、逐份来源标记 `candidate_sources` 与生成说明。

**确定性与等值证据**：导出入口两轮（`export-run-a`/`export-run-b`）三资产 Windows `fc.exe /b` **全部逐字节相等**；`shared_baseline_package_matches_frozen_chain`（六来源重建逐值逐序 + 锚逐值）通过；全量单测 **269 passed / 0 failed**（改动前后结论不变）；结果 JSON（三资产）无任何计时字段，耗时只写本机暂存。执行环境 AMD Ryzen 9 9955HX / Windows 11 / rustc 1.99.1；打包与核验证据暂存本机 `target/issue-790-806-handoff-20261026/e0-shared-baseline-20261029/`（不入库）。导出过程中止记录（无产物落盘）：① 来源⑥两族文件一度被记为两个断言级（实际 7 级 vs 冻结 6 级链），改为按来源分组后通过；② 一次环境变量路径笔误导致读取失败。均属打包入口修正，不改任何合并、去重或断言口径。

## 形状定案（与 Issue #1140 需求单及 A 路消费澄清逐项一致）

1. `pool.json`：**与历史候选来源同构的行形状** `{edge: {"candidates": [{"program","arc_m","spans"}]}}`，`experiment::pool` 可直接消费；候选数组保持合并顺序；去重键由程序点位重算校验（包内不另存），逐份来源在 MANIFEST `candidate_sources`。
2. `results-anchor.json`：`{"source": {…}, "components": [53 分量], "paths": [84 路径], "path_counts": {accepted: 28, …}}`。**仅结果对照，不作候选点位来源**（`pool_sizes`/`joint_acceptance` 不入锚）。
3. `MANIFEST.json`：`files.<name>.{sha256,bytes}`（消费方代码内哈希断言读取点）+ `merge_order: ["1103","1107","1111","1115","1131","1134"]` + 来源链与生成说明。

## 公共助手最小集（需求单，A/B/C/D 复核）

`calibration::shared_baseline` 门面（`dependency_recovery` 作用域）：**池读取/断言** `shared_pools()`（钉版断言后返回 `combinations::Pools`）与 `results_anchor_pinned()`；**词汇映射** `plan_stop_label`；**载体三层链** `span_control`（fresh 两族 → 1107 history → `repair_curve` 只读重放回退）及 `find_span_control`/`replay_span_controls`/`SpanControl`；**网格节点** `grid_node_member`。契约签名由 `shared_baseline_facade_exposes_required_helper_set` 单测钉版。`edge_input`/`coverage_output` 为内部依赖不对外（需求单增补再扩）。

**行为等值（仅限旧入口、旧输入、既有结论）**：共享代码改动只含可见性放宽——`calibration::coverage` 六项（上列助手 + `SpanControl` 及其四字段）`pub(super)`→`pub(in super::super)`、`cohort2_coverage::read_pinned_json` `fn`→`pub(super) fn`，语义逐字不变；既有 `cohort2_*`/`coverage_*`/`stream_*`/`closure_*` 测试不经修改全部通过（269/0），即等值证明。新增能力（基线加载器、导出入口）另行证明正确性（合成单测 + 冻结链逐值校验入口）；E1 新诊断正确性归 E1 方案。使用方复核记录由 lead 汇总。

## 复现

- 构建：`cargo +1.99.1 test --locked -p laneflow-lust-converter --lib`（Rust 1.99.1）。
- 常规单测（不依赖外部资产）：`…calibration::shared_baseline::shared_baseline_merge_dedups_canonical_points_in_frozen_order`、`…shared_baseline_facade_exposes_required_helper_set`、`…shared_baseline_loader_rejects_unpinned_bytes`。
- 导出入口（`--exact --ignored --nocapture --test-threads=1`，全名 `output::emit::s1::recovery_coverage::dependency_recovery::calibration::shared_baseline::shared_baseline_export`）：env `SHARED_BASELINE_EXPORT_DIR`（全新目录）+ `COHORT2_BASELINE_DIR`（来源⑤解压目录）+ `COHORT2_RUN_DIR`（来源⑥ run-03 输出目录）。
- 校验入口（全名 `…shared_baseline::shared_baseline_package_matches_frozen_chain`）：env `SIX_STREAMS_BASELINE_DIR`（本资产解压目录）+ `COHORT2_BASELINE_DIR` + `COHORT2_RUN_DIR`。
- 消费方（A–D）：env `SIX_STREAMS_BASELINE_DIR` + `shared_baseline::shared_pools()`；输入、预算与样本由冻结源码与钉版常量固定，不得换样后沿用结论。

## 结论边界

92 份池是**共同基线输入**，不是任何边的恢复结论；`results-anchor.json` 仅作结果对照（28/84、53 分量、84 路径逐值），不作候选点位来源。A–D 各自实验不吸收其他流新候选、各报相对共同基线的增量（章程 §3）。完成本切片不关闭 #1140 之外的任何 Issue，不构成 LuST 全网转换成功或生产设计冻结；生产 S1 的 G1 保持 Proposed。
