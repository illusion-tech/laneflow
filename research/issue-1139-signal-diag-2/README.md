# 信号诊断组 2：C19/C22/C24 的固定曲线覆盖补齐、逐边诊断与停止线三情形核对

**结论：研究完成，本轮零分量恢复（有效结论、分列保存），生产 S1 的 G1 仍未接受。** #1139 按[冻结计划](plan.md)完成阶段一三组搜索、阶段二逐边局部必要条件诊断、阶段三组合验收与停止线三情形核对：30 条缺边中仅 3 条出候选（`sumo::-13188_4_0`、`sumo::-5126_5_0`、`sumo::-25286_17_0`，两族各 8 份、规范 f32 点位互异共 16 份/边），27 条零候选；诊断取得 **5 项全参数域排除**（含停止线边 `sumo:--30358#12_0` 末弦）、15 个局部见证、13 个未决、3 项不适用（边有候选）；停止线三情形核对表 26 行逐边产出（原成功 22 行逐字节保持、本轮恢复 0 行、未恢复 4 行来源绑定核对一致）。三个信号分量全部保持 `missing_real_programs`（候选齐备前置不满足，组合赋值 0 次）；两口径收益持平：「84 原目标」28/84 → 28/84，「完整关联路径」（47 条 ↔ 12 条原目标）0/47 → 0/47。

本研究对应 #1139（六路协调任务书 v2 的 D 路，父任务 #790），基线 `main` = `8336eb87dca2229bff0061b56389ea90ae0cab8d`，实现冻结于 `239c737f119810b381e393ad645ef8616f1d798b`。执行口径冻结于[计划](plan.md)（冻结提交 `10c35b03`），样本与问题表冻结于[`edges.json`](edges.json)，方法事实源为[第 1 轮计划](../issue-1121-coverage-diagnosis/plan.md)（§2/§3/§4/§5/§7 逐值沿用），实现模板为[第 2 批 cohort2](../issue-1134-cohort2-coverage-diagnosis/)。生产采样、compiler 配置档、阈值、原成功几何、拓扑与控制锚点均未修改；本切片只增加 Rust `cfg(test)` 研究入口（模块 `signal_diag_2`）与本报告。Traffic Runtime API、数据格式、Adapter API、依赖与许可证无变化。生产 S1 的 G1 保持 Proposed（[决策材料](g1-proposal.md)）。

## 冻结与证据

输入为 pinned LuST `c4bd5bd3751d426d42a9a1749c815e47ea188549`；完整回归沿用 #1105 修复后基线：84 条目标、53 个完整分量、183 条关联路径、16,265 条成功边、8,230 条失败诊断，每轮全网捕获与基线逐字节断言一致。共同基线为 E0（#1140）冻结交付的 11 边 92 份（Release `research-evidence-1140-shared-baseline`，pool.json SHA-256 `31fca696…1bb3`），结果对照锚 = #1134 run-03 `results.json#variants.new`（92 池行，`results-anchor.json` 312,453 字节 SHA-256 `d025c4bf…847b`，lead 位级复验重新冻结），经 `SIX_STREAMS_BASELINE_DIR` + `shared_baseline::shared_pools()`/`results_anchor_pinned()` 钉版读取。样本为[冻结清单](edges.json)的 3 个信号分量：C19/C22/C24，30 条缺边、47 条完整关联路径（↔ 12 条原目标）、4 个控制器（`sumo:-5126`、`sumo:-1542`、`sumo:-25286`、`sumo:-17348`）、4 条缺程序停止线边与 22 条原成功停止线边。

四个独立 release 进程按计划 §7 顺序 ABBA 执行（搜索族 old→grid、grid→old、grid→old、old→grid；诊断 diagnosis-first、baseline-first、baseline-first、diagnosis-first）：

| 轮次 | 顺序（搜索族 / 诊断） | 整进程墙钟（秒） |
| ---: | --- | ---: |
| 00 | old-grid / diagnosis-first | 182.4 |
| 01 | grid-old / baseline-first | 184.3 |
| 02 | grid-old / baseline-first | 190.7 |
| 03 | old-grid / diagnosis-first | 180.2 |

四轮全部非计时输出确定性：每轮 67 份输出（66 份非计时 + `timing.json`），后三轮与首轮以 Windows `fc.exe /b` 完整比较，共 **198 组全部逐字节相等**。搜索/运行生成物外置 Release [`research-evidence-1139-signal-diag-2`](https://github.com/illusion-tech/laneflow/releases/tag/research-evidence-1139-signal-diag-2)（`signal-diag-2-run-outputs.zip`，1,431,096 字节，SHA-256 `0da1b293…463d`，含 MANIFEST：逐文件 SHA-256/字节数/来源链）；`successful-programs.jsonl`（70 MB）、LFCA 几何、formal.exe、源码 tar 按 plan §5 留本机证据目录（前者与钉版基线逐字节相等由每轮 snapshot 断言）。文件清单、大小与 SHA-256 见本机 `evidence-manifest.json`；机器与工具链见 `environment.json`（AMD Ryzen 9 9955HX / Windows 11 build 29695 / rustc 1.99.1）；逐轮计时明细见 `measurements.json`，耗时只写 `timing.json`，结果 JSON 无任何计时字段。

## 阶段一：固定曲线搜索补齐（30 边 × 三组）

原候选族默认档 32,768 覆盖全部 30 条边；扩展档 262,144 仅 `sumo:--30358#12_1`、`sumo:--30316_0` 两条；校准网格（#1111 两项校准）为 30 边首次执行。3 条边的历史记录逐值对照全部通过（`1103-base` 3 条、`1103-expanded` 2 条的报告与候选逐值一致），27 条边无任何历史记录、属首次执行（与 `research/issue-1103-s1-recovery-budget/results.json` 双向核对）。计划词汇停止标签（原始标签保留在各 search 的 `stop_reason`）：

| 边 | 原族默认档 | 原族扩展档 | 校准网格 | 候选（默认/扩展/网格） |
| --- | --- | --- | --- | --- |
| `sumo:--30358#12_0` | partitions_budget_stopped | — | work_budget_stopped | 0 / — / 0 |
| `sumo:--30358#12_1` | partitions_budget_stopped | partitions_budget_stopped | work_budget_stopped | 0 / 0 / 0 |
| `sumo::-13188_4_0` | **program_pool_full** | — | **program_pool_full** | **8 / — / 8** |
| `sumo::-5126_2_0` | partitions_budget_stopped | — | retained_family_exhausted | 0 / — / 0 |
| `sumo::-5126_3_0` | finite_family_exhausted | — | retained_family_exhausted | 0 / — / 0 |
| `sumo::-5126_5_0` | **program_pool_full** | — | **program_pool_full** | **8 / — / 8** |
| `sumo::-5126_8_0` | partitions_budget_stopped | — | work_budget_stopped | 0 / — / 0 |
| `sumo:--30316_0` | partitions_budget_stopped | partitions_budget_stopped | work_budget_stopped | 0 / 0 / 0 |
| `sumo::-1542_0_0` | finite_family_exhausted | — | retained_family_exhausted | 0 / — / 0 |
| `sumo::-1542_10_0` | finite_family_exhausted | — | retained_family_exhausted | 0 / — / 0 |
| `sumo::-1542_6_0` | finite_family_exhausted | — | retained_family_exhausted | 0 / — / 0 |
| `sumo::-1542_7_0` | finite_family_exhausted | — | retained_family_exhausted | 0 / — / 0 |
| `sumo::-1542_8_0` | partitions_budget_stopped | — | work_budget_stopped | 0 / — / 0 |
| `sumo::-1542_8_1` | partitions_budget_stopped | — | work_budget_stopped | 0 / — / 0 |
| `sumo::-25286_10_0` | partitions_budget_stopped | — | retained_family_exhausted | 0 / — / 0 |
| `sumo::-25286_11_0` | finite_family_exhausted | — | retained_family_exhausted | 0 / — / 0 |
| `sumo::-25286_17_0` | **program_pool_full** | — | **program_pool_full** | **8 / — / 8** |
| `sumo::-25286_18_0` | partitions_budget_stopped | — | work_budget_stopped | 0 / — / 0 |
| `sumo::-25286_2_0` | partitions_budget_stopped | — | work_budget_stopped | 0 / — / 0 |
| `sumo::-25286_3_0` | finite_family_exhausted | — | retained_family_exhausted | 0 / — / 0 |
| `sumo::-25286_7_0` | finite_family_exhausted | — | retained_family_exhausted | 0 / — / 0 |
| `sumo::-25286_8_0` | finite_family_exhausted | — | retained_family_exhausted | 0 / — / 0 |
| `sumo:--30266_0` | finite_family_exhausted | — | retained_family_exhausted | 0 / — / 0 |
| `sumo::-17348_0_0` | finite_family_exhausted | — | retained_family_exhausted | 0 / — / 0 |
| `sumo::-17348_5_0` | partitions_budget_stopped | — | retained_family_exhausted | 0 / — / 0 |
| `sumo::-17348_6_0` | finite_family_exhausted | — | retained_family_exhausted | 0 / — / 0 |
| `sumo::-17348_7_0` | partitions_budget_stopped | — | work_budget_stopped | 0 / — / 0 |
| `sumo::-17348_8_0` | finite_family_exhausted | — | retained_family_exhausted | 0 / — / 0 |
| `sumo::-29244_1_0` | partitions_budget_stopped | — | retained_family_exhausted | 0 / — / 0 |
| `sumo::-29244_4_0` | finite_family_exhausted | — | retained_family_exhausted | 0 / — / 0 |

3 条出候选边均为两族各自池满（`program_pool_full`），去重后各 16 份并入新增池；`candidates_found` 本轮未出现。状态/代表裁剪为裁剪事件单独计数，不属停止标签。

## 阶段二：逐边局部必要条件诊断

对冻结问题表的 36 个局部问题执行诊断：33 个属阶段一后仍零候选的边，3 个属有候选边（记 `not_applicable_edge_has_candidates`）。排除判据仅量化弦长下限（0.099999 m 保守值）与管线实际施加的方向锥（边界钳制端弦 0.95°）；规定倒圆切向不作方向条件。`sumo::-17348_6_0` 两端均 interior CR，记 `endpoint_diagnosis_not_selected`（本轮规则未选取端点诊断；CR 切向不使弦长必要条件失效，不是「端点必要条件不适用」），零候选只记搜索结果。

| 序 | 边 | 问题 | 结果 | 关键计数 |
| ---: | --- | --- | --- | --- |
| 1 | `sumo:--30358#12_0` | 末弦（末端边界钳制） | **全参数域排除** | 13 盒、深度 4、7 排除叶（方向 5 / 弦长 2），复核通过 |
| 2 | `sumo:--30358#12_1` | 末弦（末端边界钳制） | 局部见证 | 注入后整边 work_budget_stopped；见证弦合格 |
| 3 | `sumo::-13188_4_0` | 末弦 | 不适用（边有候选） | — |
| 4 | `sumo::-5126_2_0` | 末弦（末端边界钳制） | 未决 | 32,000,000 盒、深度 48、pending 33 |
| 5 | `sumo::-5126_3_0` | 末弦（规定倒圆，无方向条件） | 局部见证 | 注入后整边 retained_family_exhausted |
| 6 | `sumo::-5126_5_0` | 首弦 | 不适用（边有候选） | — |
| 7 | `sumo::-5126_8_0` | 首弦（规定倒圆，无方向条件） | 局部见证 | 注入后整边 work_budget_stopped |
| 8 | `sumo:--30316_0` | 首弦（规定倒圆，无方向条件） | 局部见证 | 注入后整边 work_budget_stopped |
| 9 | `sumo:--30316_0` | 末弦（规定倒圆，无方向条件） | 未决 | 32,000,000 盒、深度 48、pending 33 |
| 10 | `sumo::-1542_0_0` | 首弦（起始边界钳制） | 局部见证 | 注入后整边 retained_family_exhausted |
| 11 | `sumo::-1542_10_0` | 首弦（起始边界钳制） | 局部见证 | 注入后整边 retained_family_exhausted |
| 12 | `sumo::-1542_6_0` | 首弦（起始边界钳制） | **全参数域排除** | 219 盒、深度 13、110 排除叶，复核通过 |
| 13 | `sumo::-1542_6_0` | 末弦（规定倒圆，无方向条件） | 未决 | 32,000,000 盒、深度 48、pending 33 |
| 14 | `sumo::-1542_7_0` | 首弦（起始边界钳制） | 局部见证 | 注入后整边 retained_family_exhausted |
| 15 | `sumo::-1542_8_0` | 首弦（规定倒圆，无方向条件） | 局部见证 | 注入后整边 work_budget_stopped |
| 16 | `sumo::-1542_8_0` | 末弦（末端边界钳制） | **全参数域排除** | 27 盒、深度 7、14 排除叶（方向 11 / 弦长 3），复核通过 |
| 17 | `sumo::-1542_8_1` | 末弦（规定倒圆，无方向条件） | 未决 | 32,000,000 盒、深度 48、pending 33 |
| 18 | `sumo::-25286_10_0` | 末弦（末端边界钳制） | 未决 | 32,000,000 盒、深度 48、pending 33 |
| 19 | `sumo::-25286_11_0` | 首弦（起始边界钳制） | 局部见证 | 注入后整边 retained_family_exhausted |
| 20 | `sumo::-25286_17_0` | 首弦 | 不适用（边有候选） | — |
| 21 | `sumo::-25286_18_0` | 末弦（规定倒圆，无方向条件） | 未决 | 32,000,000 盒、深度 48、pending 33 |
| 22 | `sumo::-25286_2_0` | 首弦（规定倒圆，无方向条件） | 局部见证 | 注入后整边 work_budget_stopped |
| 23 | `sumo::-25286_2_0` | 末弦（规定倒圆，无方向条件） | 未决 | 32,000,000 盒、深度 48、pending 33 |
| 24 | `sumo::-25286_3_0` | 末弦（规定倒圆，无方向条件） | 局部见证 | 注入后整边 retained_family_exhausted |
| 25 | `sumo::-25286_7_0` | 首弦（规定倒圆，无方向条件） | 局部见证 | 注入后整边 retained_family_exhausted |
| 26 | `sumo::-25286_7_0` | 末弦（规定倒圆，无方向条件） | 未决 | 32,000,000 盒、深度 48、pending 33 |
| 27 | `sumo::-25286_8_0` | 首弦（起始边界钳制） | **全参数域排除** | 43 盒、深度 8、22 排除叶，复核通过 |
| 28 | `sumo::-25286_8_0` | 末弦（规定倒圆，无方向条件） | 局部见证 | 注入后整边 retained_family_exhausted |
| 29 | `sumo:--30266_0` | 首弦（起始边界钳制） | 局部见证 | 注入后整边 **program_pool_full**（见证参数网格下 8 份候选；按 §3.5 不入候选池） |
| 30 | `sumo::-17348_0_0` | 首弦（起始边界钳制） | 局部见证 | 注入后整边 retained_family_exhausted |
| 31 | `sumo::-17348_5_0` | 末弦（末端边界钳制） | 未决 | 32,000,000 盒、深度 48、pending 33 |
| 32 | `sumo::-17348_7_0` | 末弦（规定倒圆，无方向条件） | 未决 | 32,000,000 盒、深度 48、pending 33 |
| 33 | `sumo::-17348_8_0` | 首弦（起始边界钳制） | 未决 | 32,000,000 盒、深度 48、pending 17 |
| 34 | `sumo::-29244_1_0` | 末弦（规定倒圆，无方向条件） | 未决 | 32,000,000 盒、深度 48、pending 33 |
| 35 | `sumo::-29244_4_0` | 首弦（起始边界钳制） | **全参数域排除** | 33 盒、深度 7、17 排除叶，复核通过 |
| 36 | `sumo::-29244_4_0` | 末弦（规定倒圆，无方向条件） | 未决 | 32,000,000 盒、深度 48、pending 33 |

证明范围与正交记录（见 `diagnosis.json`）：

- **5 项全参数域排除**（`sumo:--30358#12_0` 末弦、`sumo::-1542_6_0` 首弦、`sumo::-1542_8_0` 末弦、`sumo::-25286_8_0` 首弦、`sumo::-29244_4_0` 首弦）各**单独**否定其固定 cubic 的完整发射（量化弦长下限与适用方向锥的已证固定冲突），排除证书经复核器验证（certificate_verified=true）。其中 `sumo:--30358#12_0` 为**缺程序停止线边**——该边的恢复须改曲线设计（控制柄/分段），属另一研究口径。这些结论只对原固定曲线的对应局部问题成立，不构成整边、分量或通用问题的无解证明，也不禁止改控制柄/合并分段的其他曲线设计。同边另一端的局部见证/未决与排除正交并存（沿 §3.6 口径）。
- 15 个局部见证只证明对应局部问题存在解；见证参数注入后的整边搜索 15 次：9 次 `retained_family_exhausted`、5 次 `work_budget_stopped`、**1 次 `program_pool_full`**（`sumo:--30266_0` 首弦见证注入后整边搜索取得满池 8 份候选）。按计划 §3.5 冻结语义，见证参数注入不强制使用见证弦、**局部见证注入搜索的候选不得注入候选池**——该 8 份候选仅作证据保存，不构成本轮恢复、不参与组合验收。**`sumo:--30266_0` 线索完整证据**：见证参数 t = 0.06884765625（`parameters_f64_bits = [0, 4589625417127690240]`，非校准网格节点 `calibrated_grid_node=false`）；注入网格命中（`witness_injection_records` 1 条：`cubic_index=0`、`span_index_zero_based=0`、`witness_pair_node_indices=[0,1]`）；见证弦 `witness_chord_qualified=true`、`witness_chord_attempted=true`、`witness_chord_in_any_returned_path=true`、**`witness_chord_in_selected_path=true`**（弦偏差 0.000635 m）；注入整边搜索 `plan_stop_label=program_pool_full`（`stop_reason=program_pool`），8 份候选记录于 `diagnosis.json` `problems[]`（`edge=sumo:--30266_0`）`.injected_whole_edge.candidates`（Release 资产 `run-00/diagnosis.json` 同路径；注入命中记录同节 `.injected_whole_edge.search.witness_injection_records`）。与第 1/2 轮「见证弦合格但不在任何返回路径」的形态不同，本轮该见证弦**进入了选中路径**——「见证参数并入网格的搜索变体」属新搜索族，须按 B 路同款流程（新方案相对现行口径的增量/遍历顺序/预算 + 评审冻结）另行授权（lead 已上报维护者列入收口批裁定与第二批候选项），不得本轮临时改判。
- 13 个未决均为诊断限额（32,000,000 参数盒、累计深度 48）到达；未决不产生排除，也不产生可行证明。
- 控制点来源与交叉核对（三层载体链）：33 个已诊断问题中 30 个由 fresh 搜索报告 `failures` 行解析（`fresh_failures`），3 边由 `repair_curve` 只读重放回退解析（`replay_fallback`：`sumo::-1542_7_0`、`sumo::-17348_0_0`、`sumo::-25286_18_0`），全部重放交叉核对一致（`replay_agrees_with_control=true`）。
- 与旧搜索的关系分两层记录（参数族成员关系 / 实际访问证据 `not_recorded`），见证与冲突不作整边互斥分类。

## 阶段三：组合验收、停止线三情形与两口径收益

三个分量在基线/新增两组均执行联合赋值检查；候选齐备前置条件不满足，全部保持 `missing_real_programs`、组合赋值 0 次（不记组合穷尽或编译拒绝）：

| 分量 | 待赋值边 | 有候选边 | 结果 |
| --- | ---: | --- | --- |
| C19 | 7 | 2/7（`sumo::-13188_4_0` 16、`sumo::-5126_5_0` 16） | 不齐备，`missing_real_programs` |
| C22 | 15 | 1/15（`sumo::-25286_17_0` 16） | 不齐备，`missing_real_programs` |
| C24 | 8 | 0/8 | 不齐备，`missing_real_programs` |

**停止线三情形核对表**（逐边产出，26 行，见 `stop-line-checklist.json`）：

| 情形 | 行数 | 核对结果 |
| --- | ---: | --- |
| 原成功停止线边 | 22 | 原程序与控制语义逐字节保持：22/22 程序位级等同原成功程序（`successful-programs.jsonl` 与 #1105 基线全文件逐字节相等），控制门/组/相位/停止线声明原样保留 |
| 本轮恢复的停止线边 | 0 | 4 条缺程序停止线边本轮均零候选，情形 2 不触发（候选齐备即执行的口径如实记录为未触发） |
| 未恢复 | 4 | 程序/编译检查未执行；来源绑定核对 4/4 一致（停止线声明 `location=edgeEnd` 与来源边、全部依赖用户关系保持） |

4 条缺程序停止线边（C19 `sumo:--30358#12_0`、`sumo:--30358#12_1`；C22 `sumo:--30316_0`；C24 `sumo:--30266_0`）在情形 3 记录；其中 `sumo:--30358#12_0` 另有末弦全参数域排除（见阶段二）。单/双横线边 ID（如 `sumo:-30316_0` 与 `sumo:--30316_0`、`sumo:-31496#1_0` 与 `sumo:--31496#1_0`）为不同边，核对表按精确 ID 区分（冻结记录的关键防御，运行断言覆盖）。

**收益统计（两口径分列，新增收益不并入基线）**：

| 新增收益 | 「84 原目标」口径 | 「完整关联路径」口径（47 条 ↔ 12 条原目标） |
| --- | --- | --- |
| 基线接受 | 28/84 | 0/47 |
| 新增组接受 | 28/84 | 0/47 |
| 新接受分量 | 0（53 分量中） | 0（3 分量中） |
| 新候选 | 48 份（3 边 × 16） | 同左（C19 待赋值边 2/7、C22 1/15 齐备度） |
| 组合赋值 | 与基线同（53 分量盘点验证） | 0 次（3 分量均未进入赋值） |

## 成本与确定性

四轮墙钟 182.4 / 184.3 / 190.7 / 180.2 秒（同机固定输入的描述统计）；非计时输出 198 组 `fc.exe /b` 全部逐字节相等（文件名集合完全相同）。中止产物按规归档于本机证据目录：`aborted-wrong-build-cwd/`（运行脚本宿主 CWD 缺陷导致首启构建了错误工作树的测试体，0 tests 静默退出，脚本已加工作树内构建/产物路径断言/二进制入口预检三道防御）、`aborted-anchor-mismatch/`（结果对照锚代际错位事故的 run-00 中止产物；详见 E0 README 差异记录，本路按 plan §8 停机报统筹后经 lead 裁决重锚重启）。未施加主机空闲门槛；耗时不推导普遍性能倍数、统计显著性或生产吞吐；工作计数不是实际堆峰值。这些操作在路网生成研究中执行，不在交通仿真 tick 内执行，未进入生产转换，不能推算产品吞吐或实时收益。

## 复现

精确测试名与环境变量（plan §10 已同步补录）：

- 正式实验入口（四轮 ABBA，`--exact --ignored --nocapture --test-threads=1`）：`output::emit::s1::recovery_coverage::dependency_recovery::calibration::signal_diag_2::pinned_lust_signal_diag_2_coverage_diagnosis_experiment`
- 常规（非 ignore）单测：`…signal_diag_2::signal_diag_2_frozen_sample_and_problem_table_stay_consistent`（问题规则逐值复现 36 问题表、27 首次/3 有历史双向核对）、`…signal_diag_2::signal_diag_2_stop_line_table_pins_exact_edge_ids`（26 行停止线表 + 单/双横线关键防御）、`…signal_diag_2::signal_diag_2_stop_line_three_case_checklist_covers_each_case`（三情形分类、证据投影与绑定核对）、`…signal_diag_2::signal_diag_2_pinned_loader_parses_same_bytes_and_rejects_others`（钉版读取器同字节解析/异字节拒读）。
- 环境变量：`LUST_SOURCE_DIR`（pinned LuST checkout）、`DIRECT_SUCCESSOR_BASELINE_DIR`（#1105 修复后完整基线）、`SIX_STREAMS_BASELINE_DIR`（Release 资产 `shared-baseline-inputs.zip` 解压目录，E0 门面钉版断言读取）、`COVERAGE_OUTPUT_DIR`（每次必须为不存在的新目录）、`COVERAGE_ORDER`（`old-grid` / `grid-old`）、`COVERAGE_DIAGNOSIS_ORDER`（`diagnosis-first` / `baseline-first`）。
- 构建：`cargo +1.99.1 test --locked --release -p laneflow-lust-converter --lib --no-run`（Rust 1.99.1）；正式四轮用该冻结二进制直跑（本地脚本 `run-signal-diag-2.ps1` / `collect-signal-diag-2.ps1` 存证据目录）。输入、预算与样本均由冻结源码与 `edges.json` 固定，不得换样后沿用结论。

## 结论边界与差异记录

**结论边界**：本轮零分量恢复不写任何可行或不可行结论——27 边零候选只表示三档有限搜索族未产出完整候选；13 个未决不产生排除也不产生可行证明；15 个局部见证与 5 项全参数域排除都只对**原固定曲线**的对应局部问题成立；3 边 × 16 份候选是局部候选，C19/C22/C24 均不齐备故不记分量恢复。停止线情形 2 本轮 0 行不等于停止线核对能力缺失——4 条缺程序停止线边在情形 3 完整记录，候选齐备时情形 2 口径即刻生效。`sumo:--30266_0` 见证注入网格下的 8 份候选按 §3.5 不入候选池，仅为后续研究线索。完成本切片不关闭 #790，也不构成 LuST 全网转换成功或生产设计冻结。

**差异记录**：

1. **停止线三情形核对表 fixture 单测使用合成 evaluate 形状评价行（lead 裁决可接受，2026-10-29）**。fixture 环境中手工合成的停止线/机动门声明不被真实联合编译接受（fixture 信号语义限制），核对表单测因此以合成的 evaluate 结果形状验证分类、证据投影与绑定核对；真实证据（规范端点、编译后长度、`edgeEnd` 解析、LFCA 控制锚点）由正式实验的联合编译产出，其正确性由 `compile_part` 内部断言（完整规范程序点位、`length_mm`/`canonical_arc_m`、`compiled_control_anchors`）保证，正式实验入口另对恢复行断言 `lfca_bytes > 0`。
2. **结果对照锚代际错位事故（E0 交付缺陷，lead 验收漏检；本路按 plan §8 停机触发发现）**。首启 run-00 在基线对照断言停止：E0 原锚取材 `stream-a-results.json#variants.new`（#1131 时代 76 池行，C40 `missing_edges=6`），与章程「结果对照锚 #1134 新增变体」（#1134 run-03 `variants.new` 92 池行，C40 `missing_edges=5`）代际错位。本路以 #1134 run-03 真值逐行核对圈定差异面（53 分量行仅 C40 一行、84 路径行 0 行、28/84 计数全同）后停机报统筹；lead 裁决 E0 重锚（含位忠实导出 1 ULP 返工）、位级复验（4,554 叶 0 失配）重新冻结后重启四轮。事故与返工的完整记录见 E0 `research/issue-1140-shared-baseline/README.md` 差异记录；本路中止产物 `aborted-anchor-mismatch/` 保留归档。
3. **运行脚本首启缺陷（本路脚本，非研究口径）**。脚本宿主 CWD 未固定导致 cargo 构建了错误工作树的测试体（0 tests 静默退出），未产生任何运行输出；已归档 `aborted-wrong-build-cwd/` 并加三道防御（工作树内构建、产物路径断言、二进制入口预检）。不改任何搜索、诊断或验收口径。
4. **跨片事实备忘（C 路发现，lead 通报，仅记录）**。serde_json 1.0.154 默认解析对 17 位十进制浮点文本存在 1 ULP 误舍入；本路四轮比较为解析域双侧自洽（同解析器对称）+ 198 组文件字节级全等，**不受该缺陷影响，结论有效**。收口批若补做位级核对，须用 E 的共享文本域比较助手（`f64::from_str` 正确舍入）或 C 的 `compare-anchor-bits.ps1` 工具，**勿用「内存值 vs serde 解析值」形态**（锚代际事故复验所用 compare-anchor-bits 即文本域位级形态，4,554 叶 0 失配）。

下一步依据见[决策材料](g1-proposal.md)，生产 S1 的 G1 保持 Proposed。
