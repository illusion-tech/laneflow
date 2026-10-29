# 第 2 批队列执行前方案（冻结计划）：11 个非信号分量的固定曲线覆盖补齐与逐边必要条件诊断

> 冻结状态：2026-10-29 维护者评审通过并冻结（Issue #1134）；执行前不得修改，差异停止并重新授权（沿第 1 轮 §7 口径）。基线 `main` = `62a7641525c1d419dfe306e590dd90dbd2741911`（#1130/#1131 合入后）；LuST 输入 `c4bd5bd3751d426d42a9a1749c815e47ea188549`。
>
> **方法事实源**：`research/issue-1121-coverage-diagnosis/plan.md`（第 1 轮冻结计划，已入库）。本批**逐值沿用**其 §2（搜索族、预算、筛选与停止语义）、§3（诊断判据、数值包围、限额、见证参数注入、正交记录）、§4（组合验收语义）、§5（回归与确定性）、§7（停止条件）；本计划只冻结本批样本、逐边问题、执行顺序与特有口径，不重复定义方法。

## 1. 样本（`edges.json`，自盘点派生并冻结）

11 个分量：C07、C09、C11、C12、C16、C20、C21、C28、C37、C40、C41；22 条缺边、25 条完整路径、1 条既有候选边 `sumo::-29388_3_0`（8 份）；0 控制器、0 停止线边。其中 **8 个单缺边分量**（C09、C11、C12、C20、C21、C28、C37、C41）；多缺边分量 C07（4）、C16（4）、C40（6）。9 条边无任何历史搜索记录（首次搜索），13 条有历史记录（含 `1103-base`/`1107-old`/`1107-grid` 等，逐值对照见 §2）。

## 2. 阶段一搜索（逐值沿用第 1 轮 §2）

- 原族默认档 32,768：22 边全量；与既有历史记录（`edges.json` `history` 所列 `1103-base`/`1107-old` 等）逐值比较，无记录边为首次执行。
- 原族扩展档 262,144：**仅** `sumo::-1000_2_0`、`sumo::-10046_0_0` 两条（`1103-expanded` 历史存在），对照该记录逐值比较；其余 20 边不跑扩展档（新增搜索量须另授权）。
- 校准网格（#1111 两项校准）：22 边首次执行，无历史对照。
- 停止标签映射、裁剪事件与预算/筛选口径（0.01 m 仅网格族、0.099999 m 仅排除余量、原族不加筛选）逐值沿用。

## 3. 阶段二诊断（逐值沿用第 1 轮 §3；问题表冻结于 `edges.json`）

- 问题生成规则（冻结）：取原诊断报告 span（0 基下标）；span **起点**切向 = 边界钳制 → 首弦（方向锥 = 该边 `clamps[0]` 原 f64 位）／规定倒圆 → 首弦（无方向锥）／interior CR → 跳过；span **终点**对称取末弦（方向锥 = `clamps[1]`）。
- 两端均 interior CR 的边（`sumo::-10046_0_0`、`sumo::-8018_4_0`、`sumo::-25490_4_0`）记 **`endpoint_diagnosis_not_selected`（本轮规则未选取端点诊断）**——CR 切向不使弦长必要条件失效，**不是**「端点必要条件不适用」；内部弦链/末两弦/接缝问题不在本轮冻结范围（另片）。三边照常执行两族搜索：出完整候选即入分量验收；零候选**只记搜索结果，不作诊断闭合**。
- 排除/见证/未决正交记录、见证参数注入（不强制使用见证弦、记录尝试/使用）、成员关系与访问证据分层、限额（32,000,000 盒 / 深度 48 / 65,536 叶）逐值沿用。诊断执行顺序 = `edges.json` 问题表顺序。

## 4. 阶段三组合验收（逐值沿用第 1 轮 §4）

- 候选齐备即分量级联合赋值（每分量 256 上限，`assignment_budget_stopped` + 未访问数）+ 真实联合编译 + LFCA + 控制锚点。
- 8 个单缺边分量：任一搜索族出 ≥1 候选即候选齐备，直接验收（本轮主要收益路径）。
- C07：待赋值边 = 4 条缺边 + `sumo::-29388_3_0` 既有候选边，各池 8 份时总组合 = **8⁵ = 32,768 > 256**；总组合按**全部待赋值边的去重后池大小连乘**计算，未访问数 = 总组合 − 已试次数，256 上限不变。C16（4 待赋值边）、C40（6 待赋值边）同理。不齐备保持 `missing_real_programs`。
- C20/C21 互为 fan-out 钳制邻居：钳制方向冻结；两分量各自验收，结果解释注明邻居关系。

## 5. 基线池与结果对照（冻结）

- 基线候选池 = 四来源按序合并（#1103：6 边 48 → +#1107：7 边 56 → +#1111：7 边 72 → +#1115 `searches.json`：9 边 75）+ #1131 曲线候选 1 份 → **10 边 76 份**（规范 f32 点位去重，顺序不得更换；候选来源与结果对照分离的口径同第 2 轮）。
- 结果对照与新增候选的读取入口**冻结为 Release 资产**（tag `research-evidence-1134-cohort2-baseline`，资产 `cohort2-baseline-inputs.zip`，内含两文件与 `MANIFEST.json`）：`stream-a-searches.json`（1,204,062 字节，SHA-256 `1c6dd16244481d8fa757865305b2f05287ed74624dde5770998e752d18e26d7b`；提供新增 1 份候选）与 `stream-a-results.json`（584,379 字节，SHA-256 `17d6bdce48e63cb26116ea6fc9882225a1aa95d7f67bb248bacfa15ac3b37786`；其 `variants.new` 作结果对照：84 路径计数 28/84、53 分量结果逐值）。两文件由 #1131 证据 `run-00` 原样外置（维护者裁定：搜索/运行生成物不入代码仓库），经环境变量 `COHORT2_BASELINE_DIR` 指向解压目录只读读取，代码内断言 SHA-256 与本节钉版值一致（身份不依赖仓库内文件）；结果对照仍仅对照、不作候选点位来源。
- 验证按基线/新增两组；本轮新增收益（新候选、新接受分量、组合赋值）单独统计，不并入基线。任一来源缺失或逐值不符停止并重新授权。
- **收益统计口径（冻结）**：本批 25 条完整关联路径对应 **14 条原目标**；剩余 7 个信号分量对应 **31 条原目标**（14 + 31 = 45，为 #790 剩余全部原目标）。接受变化与新增收益按「84 原目标」与「完整关联路径」两个口径分列报告。

## 6. 执行顺序（冻结）

1. 阶段一搜索：`edges.json` 边序（C07 → C41）。
2. 阶段二诊断：`edges.json` 问题表顺序（仅零候选边）。
3. 阶段三组合：分量码序 C07、C09、C11、C12、C16、C20、C21、C28、C37、C40、C41。
4. 四独立 release 进程：搜索族 old→grid、grid→old、grid→old、old→grid；诊断 diagnosis-first、baseline-first、baseline-first、diagnosis-first。

## 7. 验证与证据（逐值沿用第 1 轮 §5 与第 2 轮操作手册）

- 四轮非计时输出逐字节比较（`fc.exe /b`）；evidence-manifest/environment/measurements；结果 JSON 禁计时字段；输出目录全新；中止产物 `*-aborted-*` 归档。
- 完整回归 84/53/183/16,265/8,230 与 #1105 基线逐行一致；debug/release 回归、check、Clippy、rustfmt、JSON 检查；九项必需检查走 Merge Queue，trailer `Harness: deepseek/harness/web` / `Model: xiaomi/mimo-v2.6-pro`；GitHub 文本不硬折行。
- 本批为单一队列：一个工作树/分支、一个 teammate 执行、lead 监督收口；与已合入各轮无共享改动面（仅新增研究入口与文档）。

## 8. 复现（实现定稿后补精确测试名与环境变量，本计划 §8 自授权）

- 环境变量沿第 1 轮口径：`LUST_SOURCE_DIR`、`DIRECT_SUCCESSOR_BASELINE_DIR`、`COVERAGE_OUTPUT_DIR`（全新目录）、`COVERAGE_ORDER`（`old-grid` / `grid-old`）、`COVERAGE_DIAGNOSIS_ORDER`（`diagnosis-first` / `baseline-first`）；基线输入经 `COHORT2_BASELINE_DIR`（Release 资产 `cohort2-baseline-inputs.zip` 解压目录）读取，代码内断言 §5 钉版 SHA-256。
- 正式实验入口（阶段一搜索、阶段二诊断、阶段三组合验收与完整回归在同一入口内按 §6 顺序执行）：`output::emit::s1::recovery_coverage::dependency_recovery::calibration::cohort2_coverage::pinned_lust_cohort2_coverage_diagnosis_experiment`。
- 常规（非 ignore）测试入口：`…calibration::cohort2_coverage::cohort2_frozen_sample_and_problem_table_stay_consistent`、`…calibration::cohort2_coverage::cohort2_baseline_pool_merges_frozen_sources_with_pinned_input_bytes`、`…calibration::cohort2_coverage::cohort2_pinned_baseline_loader_rejects_unpinned_bytes`、`…calibration::cohort2_coverage::cohort2_span_control_fallback_preserves_existing_carriers`。
- 冻结提交构建 `laneflow-lust-converter` release lib test（Rust 1.99.1），`--exact --ignored --nocapture --test-threads=1`；输入、预算与样本均由冻结源码与 `edges.json` 固定，不得换样后沿用结论。
