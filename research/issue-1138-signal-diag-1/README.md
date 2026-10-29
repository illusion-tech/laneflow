# 信号诊断组 1：C14/C15/C18/C46 信号分量的固定曲线覆盖补齐与逐边必要条件诊断

**结论：研究完成，本轮零恢复（有效结论、分列保存），生产 S1 的 G1 仍未接受。** #1138 按[冻结计划](plan.md)完成阶段一搜索补齐、阶段二逐边局部必要条件诊断与阶段三候选齐备分量完整控制器验收：23 条缺边的原族默认档与 `1103-base` 历史记录逐值一致、扩展档按历史清单为空（0 条）、校准网格 23 边首次执行——**23 边两族全部零候选**。诊断 30 个局部问题取得 **1 项全参数域排除**（`sumo::-11042_4_0` 首弦，91 盒/深度 9/46 叶证书经复核器验证，继 `sumo:--30306#2_0`、`sumo::-10100_1_0` 后**第三例已证固定冲突**）、**15 个局部见证**、**14 个未决**（到达 32,000,000 参数盒/累计深度 48 限额）。4 个分量全部保持 `missing_real_programs`（候选不齐备，未进入赋值循环）：两口径收益「84 原目标」28/84 → 28/84、「完整关联路径」（本组 54 条 ↔ 19 条原目标）0/54 → 0/54，无新增接受分量与接受目标。**完整控制器验收口径已全量留痕**：四分量控制器全集（`full_controller_members`）断言通过，31 条停止线全部落在成功边、原程序与控制语义逐项保持（「原成功停止线边」情形 31/31）。

本研究对应 #1138（六路第一批 C 路，父任务 #790），基线 `main` = `8336eb87`，实现冻结于 `8796e0b6`（分支含 E0 依赖链）。执行口径冻结于[计划](plan.md)（冻结提交 `41626793`），方法事实源为[第 1 轮计划](../issue-1121-coverage-diagnosis/plan.md)（§2/§3/§4/§5/§7 逐值沿用），实现模板为 [`calibration::cohort2_coverage`](../issue-1134-cohort2-coverage-diagnosis/README.md)；共同口径按六路协调章程 v2（`research/six-streams-20261029/coordination.md`）。生产采样、compiler 配置档、阈值、原成功几何、拓扑与控制锚点均未修改；本轮只增加 Rust `cfg(test)` 研究入口与本报告。Traffic Runtime API、数据格式、Adapter API、依赖与许可证均无变化。生产 S1 的 G1 保持 Proposed（[决策材料](g1-proposal.md)）。

## 冻结与证据

输入为 pinned LuST `c4bd5bd3751d426d42a9a1749c815e47ea188549`；完整回归沿用 #1105 修复后基线：84 条目标、53 个完整分量、183 条关联路径、16,265 条成功边、8,230 条失败诊断，每轮全网捕获与基线逐字节断言一致。样本为[冻结清单](edges.json)（自 missing-component-inventory-20261028 盘点派生）：C14/C15/C18/C46 四个信号分量、23 条缺边、54 条完整关联路径（↔ 19 条原目标：C14/C15/C18 各 1 + C46 16）、3 条既有候选边（C14 的 `sumo::-10130_8_0`/`sumo::-10130_16_0`/`sumo::-10130_18_0`，各 8 份在共同基线池内）、4 个控制器（每分量 1，整体验收对象）、**0 条缺程序停止线边**。15 条边有历史记录（全部 `1103-base` 锚）、8 条边（C18 全部）首次搜索（与记录文件双向核对）。

**共同基线**（E0 冻结交付 [research-evidence-1140-shared-baseline](https://github.com/illusion-tech/laneflow/releases/tag/research-evidence-1140-shared-baseline)）：`shared-baseline-inputs.zip`（`pool.json` 11 边 92 份六来源合并去重 + `results-anchor.json` 结果对照锚 + `MANIFEST.json` 钉版），经 `SIX_STREAMS_BASELINE_DIR` + `shared_baseline::shared_pools()`/`results_anchor_pinned()` 读取（代码内哈希/合并顺序/逐边份数断言）；**禁止旧 9/10 边池**。结果对照锚 = #1134 run-03 `variants.new`（92 池行代，53 分量、84 路径、接受 28/84 逐值；`C40 missing_edges` 6→5 为两代唯一语义差，锚已按 92 池行代重钉），仅作结果对照、不作候选点位来源。

## 阶段一：固定曲线搜索补齐（23 边 × 两组）

原候选族默认档 32,768 覆盖全部 23 条边，与 `1103-base` 历史记录（报告与候选）逐值对照全部一致；扩展档按历史清单**为空**（`1103-expanded` 锚只标记 #1103 六条截断样本边，均不在本组；「32768 停止边补跑 262144」属新增搜索量，统筹裁决本轮不授权、记入第二批候选项）；校准网格（#1111 两项校准）23 边首次执行、无历史对照。**23 边两族全部零候选**（每边 0/0），停止标签：

| 边 | 原族默认档 | 校准网格 | 边 | 原族默认档 | 校准网格 |
| --- | --- | --- | --- | --- | --- |
| `sumo::-10130_12_0` | partitions_budget_stopped | retained_family_exhausted | `sumo::-11042_11_0` | finite_family_exhausted | retained_family_exhausted |
| `sumo::-10130_13_0` | partitions_budget_stopped | retained_family_exhausted | `sumo::-11042_14_0` | finite_family_exhausted | retained_family_exhausted |
| `sumo::-10130_4_0` | partitions_budget_stopped | retained_family_exhausted | `sumo::-11042_15_0` | finite_family_exhausted | retained_family_exhausted |
| `sumo::-10130_5_0` | finite_family_exhausted | retained_family_exhausted | `sumo::-11042_16_0` | finite_family_exhausted | retained_family_exhausted |
| `sumo::-10130_7_0` | partitions_budget_stopped | retained_family_exhausted | `sumo::-11042_4_0` | finite_family_exhausted | retained_family_exhausted |
| `sumo::-10156_2_0` | partitions_budget_stopped | retained_family_exhausted | `sumo::-11042_6_0` | partitions_budget_stopped | retained_family_exhausted |
| `sumo::-10156_3_0` | finite_family_exhausted | retained_family_exhausted | `sumo::-29900_10_0` | partitions_budget_stopped | retained_family_exhausted |
| `sumo::-10156_4_0` | partitions_budget_stopped | work_budget_stopped | `sumo::-29900_11_0` | partitions_budget_stopped | retained_family_exhausted |
| `sumo::-10156_5_0` | finite_family_exhausted | retained_family_exhausted | `sumo::-29900_16_0` | partitions_budget_stopped | retained_family_exhausted |
| `sumo::-10156_8_0` | finite_family_exhausted | retained_family_exhausted | `sumo::-29900_3_0` | partitions_budget_stopped | retained_family_exhausted |
| `sumo::-11042_0_0` | finite_family_exhausted | retained_family_exhausted | `sumo::-29900_4_0` | partitions_budget_stopped | retained_family_exhausted |
| `sumo::-11042_10_0` | partitions_budget_stopped | retained_family_exhausted | | | |

`candidates_found`/`program_pool_full` 本轮未出现；状态/代表裁剪为裁剪事件单独计数。

## 阶段二：逐边局部必要条件诊断（30 个局部问题，逐端判定）

问题表按冻结规则逐端判定（边界钳制→首/末弦带方向锥；规定倒圆→对应端无方向锥；interior CR→**该端不取**；仅两端均不取才边级记 `endpoint_diagnosis_not_selected`——本组 23 边均至少一端可取，0 条该标记；单端不取的边如 `sumo::-10130_7_0`、`sumo::-10156_4_0`、`sumo::-29900_10_0` 等只保留另一端问题）。排除判据仅量化弦长下限（0.099999 m 保守值）与管线实际施加的方向锥（边界钳制端弦 0.95°）；规定倒圆切向不作方向条件。

**结果分层（正交记录，见 `diagnosis.json`）**：

- **1 项全参数域排除**：`sumo::-11042_4_0` 首弦（起始边界钳制）——91 盒、深度 9、46 排除叶（最短弦 3 / 起始方向 43），复核器通过。**第三例已证固定冲突**（继 `sumo:--30306#2_0`、`sumo::-10100_1_0`）：量化弦长下限与起始边界钳制方向锥的固定约束冲突，单独否定该固定 cubic 的完整发射；同边末弦局部见证（t=0.5）与之正交并存（见证与冲突不作整边互斥分类）。不构成该边其他 span、整边、分量或通用问题的无解证明。
- **15 个局部见证**（t ∈ {0.0625, 0.125, 0.25, 0.5}，全部为 `{i/512}` 网格节点）：见证参数注入后的整边搜索 15 次全部 `retained_family_exhausted` 零候选；`witness_injection_records` 共 240 条全部 `witness_chord_attempted=true`、`witness_chord_qualified=true`，但 `witness_chord_in_any_returned_path=false`——见证弦合格而弦图无延续到终点的完整保留路径。注入不强制使用见证弦；形态与第 1/3 批一致。
- **14 个未决**：全部到达 32,000,000 参数盒/累计深度 48 限额（各约 8.8–9.6 秒）；未决不产生排除，也不产生可行证明。
- **控制点来源与交叉核对**（三层载体链）：`fresh_failures` 19 边 + `replay_fallback` 4 边（`sumo::-10130_13_0`、`sumo::-10130_5_0`、`sumo::-10156_3_0`、`sumo::-11042_0_0`，均首弦/起始钳制，网格走查通过诊断 span 不留行）；全部 30 个问题的 `replay_agrees_with_control=true`（重放六点与主载体前四点逐位一致）。

## 阶段三：完整控制器验收与两口径收益

4 个分量在基线/新增两组均执行联合赋值检查；候选齐备前置条件不满足（23 边全部零候选，C14 仅 3 条既有候选边、其余待赋值边无候选），全部保持 `missing_real_programs`、组合赋值 0 次（不记组合穷尽或编译拒绝）。满池组合口径已实现留痕（C14 待赋值边 8 条 8⁸、C15/C46 5 条 8⁵、C18 8 条 8⁸——均远超 256，齐备即触发 `assignment_budget_stopped` + 未访问数）；本轮未触发。局部候选不记分量恢复（C14 的 3 条既有候选边保留在共同基线池）。

**完整控制器验收（本组核心增量，口径全量留痕）**：

- 四分量 `full_controller_members` 全集断言通过（分量 = 控制器完整闭包）；
- **原成功停止线边情形核对 31/31 全过**：本组 0 条缺程序停止线边，31 条停止线全部落在成功边——逐条核对原程序保持（固定成功边逐字节断言）、停止线与来源边关系、机动门绑定、信号组绑定、转换下标（`compiled_control_anchors` 口径）；「本轮恢复的停止线边」「未恢复」两情形不适用（已在核对表标注）；
- 接受分量的 LFCA 发射检查与 `lfca_bytes` 逐值断言照常执行（基线侧与锚逐字节一致，本轮无新接受分量）。

**收益统计（两口径分列，新增收益不并入基线）**：

| 新增收益 | 「84 原目标」口径 | 「完整关联路径」口径（本组 54 条 ↔ 19 条原目标） |
| --- | --- | --- |
| 基线接受 | 28/84 | 0/54 |
| 新增组接受 | 28/84 | 0/54 |
| 新接受分量 | 0（53 分量中） | 0（4 分量中） |
| 新候选 | 0（23 边全零候选） | 0 |
| 组合赋值 | 62 次（与基线同，来自其余分量盘点） | 0 次（4 分量均未进入赋值） |

其余信号分量（D 组 C19/C22/C24，47 路径 ↔ 12 原目标）另片。

## 成本与确定性

四个独立 release 进程按计划 §6 顺序 ABBA 执行（搜索族 old→grid、grid→old、grid→old、old→grid；诊断 diagnosis-first、baseline-first、baseline-first、diagnosis-first），单轮 152.9–164.4 秒（阶段一 23 边两族 ~13–16 秒、阶段二 30 问题 ~65–70 秒（未决 14 项各约 9 秒为主体）、阶段三双组验证 ~10 秒）。每轮 66 份输出（65 份非计时 + `timing.json`），后三轮与首轮 Windows `fc.exe /b` 完整比较共 **195 组全部逐字节相等**（文件名集合也完全相同）；耗时只写 `timing.json`，结果 JSON 无任何计时字段；输出目录全新。文件清单、大小与 SHA-256（仅互操作）见本机 `evidence-manifest.json`；机器与工具链见 `environment.json`（AMD Ryzen 9 9955HX / Windows 11 build 29695 / rustc 1.99.1）；逐轮计时明细见 `measurements.json`。共同基线资产来自 Release 钉版包（`shared-baseline-inputs.zip`，MANIFEST 三件套 SHA-256 与 `pool.json` 11 边 92 份/`results-anchor.json` 形状经预校验）。

未施加主机空闲门槛；耗时为同机固定输入的描述统计，不推导普遍性能倍数、统计显著性或生产吞吐；工作计数不是实际堆峰值。这些操作在路网生成研究中执行，不在交通仿真 tick 内执行，未进入生产转换。

## 复现

- 正式实验入口（四轮 ABBA，`--exact --ignored --nocapture --test-threads=1`）：`output::emit::s1::recovery_coverage::dependency_recovery::calibration::signal_diag_1::pinned_lust_signal_diag_1_experiment`
- 常规（非 ignore）单测：`…signal_diag_1::signal_diag_1_frozen_sample_and_problem_table_stay_consistent`（逐端问题规则逐值复现 30 问题表、15 有史/8 首次双向核对、样本形状断言）、`…signal_diag_1::signal_diag_1_shared_pool_shape_matches_the_frozen_common_baseline`（共同基线 11 边 92 份形状钉版 + 临时目录同字节读写）。
- 环境变量：`LUST_SOURCE_DIR`、`DIRECT_SUCCESSOR_BASELINE_DIR`、`SIX_STREAMS_BASELINE_DIR`（E0 资产解压目录）、`COVERAGE_OUTPUT_DIR`（每次必须为不存在的新目录）、`COVERAGE_ORDER`（`old-grid` / `grid-old`）、`COVERAGE_DIAGNOSIS_ORDER`（`diagnosis-first` / `baseline-first`）。
- 构建：`cargo +1.99.1 test --locked --release -p laneflow-lust-converter --lib --no-run`（Rust 1.99.1）；正式四轮用该冻结二进制直跑。开跑前置预校验：`compare-anchor-bits.ps1`（锚 vs 源切片 f64 位级 token 比较，正确舍入解析）+ 形状复核。输入、预算与样本均由冻结源码与 `edges.json` 固定，不得换样后沿用结论。

## 结论边界与差异记录

**结论边界**：本轮零恢复不写任何可行或不可行结论——23 边零候选只表示两档有限搜索族未产出完整候选；14 个未决不产生排除也不产生可行证明；15 个局部见证与 1 项全参数域排除（`sumo::-11042_4_0` 首弦，第三例已证固定冲突）都只对**原固定曲线**的对应局部问题成立。C14 的 3 条既有候选边是局部候选，分量不齐备故不记恢复。完成本切片不关闭 #790，也不构成 LuST 全网转换成功或生产设计冻结。

**差异记录（统筹裁决并授权，plan §6.1/§7 口径）**：

1. **比较口径（plan §6.1，统筹预授权 + 补维 + 裁决 B）**。逐值比较 = 语义字段逐值（`status`、`selected_result`/`selected_assignment`、`trials`/计数、`paths`/`path_counts`、`missing_edges` 列表、分量结果语义行、`pool_sizes`）；运行局部字段显式归一化（分量行 `artifact` 文件名前缀归一为 `component-NN.lfca`）；锚顶层 `source`（E0 资产来源注）、`cohort_*` 与行内 `cohort` 标记（源批次 #1134 自有分析列，其 11 分量队列与本组 4 分量队列不同）显式声明不入比较。**数值位级终裁在文本 token 域**：锚文件原始 token vs 本路内存值序列化 token 逐叶比较，数值叶以正确舍入 `f64::from_str` 解析后按 `to_bits` 相等。
2. **四停根因：serde_json 默认解析 1 ULP 误舍入 + 比较面清点（跨片事实）**。正式批四次中止（`run-00-aborted-anchor-source-metadata`（锚来源注误入比较）/ `-parse-rounding`（解析误舍入误报）/ `-anchor-extra-fields`（源批分析列）/ `-anchor-cohort-flag`（行内 cohort 标记））后定位：serde_json 1.0.154 默认解析（未开 `float_roundtrip` feature）对 17 位十进制浮点误舍入 1 ULP（实证 `from_str("10.220051765441895")` → bits …313，真值 …312；`DoubleToInt64Bits` 对照）。三类比较语义分析（已同步 A/D）：解析域双侧比较对折叠对（…895/…896）**漏报**（旧锚漂移类恰属此类）、内存 vs 解析域位比较对忠实锚**误报**、文本 token 域正确舍入比较两向严格。历史各轮解析域「逐值」语义仍有效（双侧同解析器 + 产物字节级一致），`float_roundtrip` 升级为独立维护者决策项（生产解析正确性潜伏问题，不混入研究切片）。`compare-anchor-bits.ps1` 保留为开工前外部校验工具。
3. **锚代际错位事故（E0 返工，统筹重冻结）**。结果对照锚初版误取 #1131 代 `stream-a-results.json`（76 池行），与章程要求的 #1134 新增变体（92 池行）存在唯一语义差（`C40 missing_edges` 6→5）；E0 重锚至 run-03 `variants.new` 并改位忠实文本拼接导出，统筹重冻结（「位级复验通过」）后本批开跑。本路预校验（位级 0 失配 + 形状 + C40=5 + pool 未变）通过。
4. **共享助手改动（统筹裁决记录）**。本路仅新增自有入口；公共助手（`shared_baseline` 读取/门面、`span_control` 三层载体等）由 E 单点修改，本路零共享代码改动（模块注册行 `mod signal_diag_1;` 按裁决留本分支字母序插入、PR 摘要列出、冲突由统筹统一处理）。
5. **中止产物归档**（不参与比较、不删除记录）：本批四停产物如上（`-anchor-source-metadata` / `-parse-rounding` / `-anchor-extra-fields` / `-anchor-cohort-flag`），均属比较接缝清点与解析边界修正，不改任何搜索、诊断或验收结果口径。

下一步依据见[决策材料](g1-proposal.md)，生产 S1 的 G1 保持 Proposed。
