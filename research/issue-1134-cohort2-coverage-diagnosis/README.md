# 第 2 批队列：11 个非信号分量的固定曲线覆盖补齐与逐边必要条件诊断

**结论：研究完成，本轮零恢复（有效结论、分列保存），生产 S1 的 G1 仍未接受。** #1134 按[冻结计划](plan.md)完成阶段一搜索补齐、阶段二逐边局部必要条件诊断与阶段三组合验收：22 条缺边的原族默认档与历史记录逐值一致、扩展档两条与 `1103-expanded` 一致、校准网格 22 边首次执行——仅 `sumo::-25332_7_0` 出候选（两族各 8 份、规范 f32 点位互异共 16 份，为本批唯一新候选边），其余 21 边零候选。诊断取得 **1 项全参数域排除**（`sumo::-10100_1_0` 首弦，20 叶排除证书经复核器验证）、**13 个局部见证**、**6 个未决**（到达 32,000,000 参数盒/累计深度 48 限额）、1 项不适用（`sumo::-25332_7_0` 有候选）。11 个分量全部保持 `missing_real_programs`（8 个单缺边分量的缺边均零候选；C40 仅 1/6 待赋值边有候选，不齐备），无新增接受分量与接受目标：「84 原目标」口径 28/84 → 28/84，「完整关联路径」口径 0/25 → 0/25。

本研究对应 #1134（父任务 #790），基线 `main` = `62a7641525c1d419dfe306e590dd90dbd2741911`，实现冻结于 `7891569da61b4c2150c7d537af2a72fccebd64ad`。执行口径冻结于[计划](plan.md)（冻结提交 `5c124e96`），方法事实源为[第 1 轮计划](../issue-1121-coverage-diagnosis/plan.md)（§2/§3/§4/§5/§7 逐值沿用）。执行中发生 2 处共享测试助手改动与 1 处 §7 重新授权，均经 lead 裁决（见「结论边界与差异记录」）。生产采样、compiler 配置档、阈值、原成功几何、拓扑与控制锚点均未修改；本轮只增加 Rust `cfg(test)` 研究入口、测试助手可见性放宽、`span_control` 载体链的授权扩展与本报告。Traffic Runtime API、数据格式、Adapter API、依赖与许可证均无变化。生产 S1 的 G1 保持 Proposed（[决策材料](g1-proposal.md)）。

## 冻结与证据

输入仍为 pinned LuST `c4bd5bd3751d426d42a9a1749c815e47ea188549`；完整回归沿用 #1105 修复后基线：84 条目标、53 个完整分量、183 条关联路径、16,265 条成功边、8,230 条失败诊断，每轮全网捕获与基线逐字节断言一致。样本为[冻结清单](edges.json)的 11 个非信号分量（C07、C09、C11、C12、C16、C20、C21、C28、C37、C40、C41）：22 条缺边、25 条完整关联路径（↔ 14 条原目标）、1 条既有候选边 `sumo::-29388_3_0`、0 控制器、0 停止线边；8 个单缺边分量，多缺边分量 C07（4 缺边）、C16（4）、C40（6）。

基线候选池按计划 §5 五来源固定顺序合并、规范 f32 点位去重，逐级断言通过：① #1103 原候选池 6 边 48 份 → ② #1107 补搜原族 7 边 56 份 → ③ #1111 校准网格 7 边 72 份 → ④ #1115 曲线候选 9 边 75 份 → ⑤ Release 资产 [research-evidence-1134-cohort2-baseline](https://github.com/illusion-tech/laneflow/releases/tag/research-evidence-1134-cohort2-baseline)（`cohort2-baseline-inputs.zip`，含 `MANIFEST.json`）中 `stream-a-searches.json`（1,204,062 字节，SHA-256 `1c6dd162…`）的 1 份候选 **10 边 76 份**（`sumo::-10004_5_1` 16、`sumo::-7348_3_0` 16、`sumo::-10034_0_0` 8、`sumo::-10130_8_0` 8、`sumo::-10130_16_0` 8、`sumo::-10130_18_0` 8、`sumo::-29388_3_0` 8、`sumo::-28860_3_0` 2、`sumo::-16502_0_0` 1、`sumo::-10564_3_0` 1）。两份基线输入经 `COHORT2_BASELINE_DIR` 指向解压目录只读读取，代码内断言 SHA-256 与计划 §5 钉版值一致（身份不依赖仓库内文件）。结果对照为同一资产中 `stream-a-results.json`（584,379 字节，SHA-256 `17d6bdce…`）的 `variants.new`：基线重新验证的 53 分量结果、84 路径结果与路径计数**逐值一致**（28/84），该文件仅作结果对照、不作候选点位来源。结果制品内 `baseline_pool.sources` 与 `result_comparison.target` 字段保留 #1131 原制品标识名（返工前后结果 JSON 逐字节一致，身份由 SHA-256 与 `MANIFEST.json` 钉版）。

## 阶段一：固定曲线搜索补齐（22 边 × 三组）

原候选族默认档 32,768 覆盖全部 22 条边；扩展档 262,144 仅 `sumo::-1000_2_0`、`sumo::-10046_0_0` 两条；校准网格（#1111 两项校准）为 22 边首次执行。13 条边的历史记录逐值对照全部通过（`1103-base` 11 条、`1107-old` 8 条、`1103-expanded` 2 条的报告与候选逐值一致；`1107-grid` 8 条仅作版本定位与控制点兜底载体），9 条边无任何历史记录、属首次执行（与四份记录文件双向核对）。计划词汇停止标签（原始标签保留在各 search 的 `stop_reason`）：

| 边 | 原族默认档 | 原族扩展档 | 校准网格 | 候选（默认/扩展/网格） |
| --- | --- | --- | --- | --- |
| `sumo:-32592#0_0` | partitions_budget_stopped | — | work_budget_stopped | 0 / — / 0 |
| `sumo::-1000_2_0` | partitions_budget_stopped | finite_family_exhausted | retained_family_exhausted | 0 / 0 / 0 |
| `sumo::-1000_5_0` | finite_family_exhausted | — | retained_family_exhausted | 0 / — / 0 |
| `sumo::-1000_7_0` | finite_family_exhausted | — | retained_family_exhausted | 0 / — / 0 |
| `sumo::-10034_4_0` | finite_family_exhausted | — | retained_family_exhausted | 0 / — / 0 |
| `sumo::-10046_0_0` | partitions_budget_stopped | finite_family_exhausted | work_budget_stopped | 0 / 0 / 0 |
| `sumo::-10100_1_0` | finite_family_exhausted | — | retained_family_exhausted | 0 / — / 0 |
| `sumo:--31624_0` | partitions_budget_stopped | — | work_budget_stopped | 0 / — / 0 |
| `sumo:-30976#0_0` | partitions_budget_stopped | — | work_budget_stopped | 0 / — / 0 |
| `sumo::-10414_4_0` | partitions_budget_stopped | — | retained_family_exhausted | 0 / — / 0 |
| `sumo::-22822_2_0` | partitions_budget_stopped | — | retained_family_exhausted | 0 / — / 0 |
| `sumo::-1426_1_0` | finite_family_exhausted | — | retained_family_exhausted | 0 / — / 0 |
| `sumo::-1426_2_0` | partitions_budget_stopped | — | work_budget_stopped | 0 / — / 0 |
| `sumo::-19726_2_0` | partitions_budget_stopped | — | retained_family_exhausted | 0 / — / 0 |
| `sumo::-22042_7_0` | partitions_budget_stopped | — | work_budget_stopped | 0 / — / 0 |
| `sumo:-32770#6_1` | partitions_budget_stopped | — | work_budget_stopped | 0 / — / 0 |
| `sumo:-32770#7_1` | partitions_budget_stopped | — | work_budget_stopped | 0 / — / 0 |
| `sumo::-25332_7_0` | **program_pool_full** | — | **program_pool_full** | **8 / — / 8** |
| `sumo::-25332_7_1` | partitions_budget_stopped | — | work_budget_stopped | 0 / — / 0 |
| `sumo::-8018_4_0` | partitions_budget_stopped | — | work_budget_stopped | 0 / — / 0 |
| `sumo::-8018_7_0` | finite_family_exhausted | — | retained_family_exhausted | 0 / — / 0 |
| `sumo::-25490_4_0` | finite_family_exhausted | — | retained_family_exhausted | 0 / — / 0 |

`candidates_found` 本轮未出现（`program_pool_full` 为 `sumo::-25332_7_0` 两族各自的池满停止）；状态/代表裁剪为裁剪事件单独计数，不属停止标签。`sumo::-25332_7_0` 的 16 份候选为**首次搜索**即得（该边无历史记录），两族各 8 份、规范 f32 点位全量互异，去重后 16 份并入新增池（11 边 92 份）。

## 阶段二：逐边局部必要条件诊断

对冻结问题表的 21 个局部问题执行诊断：20 个属阶段一后仍零候选的边，1 个属有候选边 `sumo::-25332_7_0`（记 `not_applicable_edge_has_candidates`）。排除判据仅量化弦长下限（0.099999 m 保守值）与管线实际施加的方向锥（边界钳制端弦 0.95°）；规定倒圆切向不作方向条件。三条 `endpoint_diagnosis_not_selected` 边（`sumo::-10046_0_0`、`sumo::-8018_4_0`、`sumo::-25490_4_0`，两端均 interior CR）不诊断、零候选只记搜索结果——CR 切向不使弦长必要条件失效，**不是**「端点必要条件不适用」；内部弦链/末两弦/接缝问题不在本轮冻结范围（另片）。

| 序 | 边 / span（0 基） | 问题 | 结果 | 关键计数 |
| ---: | --- | --- | --- | --- |
| 1 | `sumo:-32592#0_0` / 3 | 末弦（规定倒圆，无方向条件） | 未决 | 32,000,000 盒、深度 48、pending 33 |
| 2 | `sumo::-1000_2_0` / 1 | 末弦（末端边界钳制） | 未决 | 32,000,000 盒、深度 48、pending 33 |
| 3 | `sumo::-1000_5_0` / 0 | 首弦（起始边界钳制） | 局部见证 | t=0.5；见证弦长 0.6945 m |
| 4 | `sumo::-1000_7_0` / 0 | 首弦（起始边界钳制） | 未决 | 32,000,000 盒、深度 48、pending 21 |
| 5 | `sumo::-10034_4_0` / 0 | 首弦（起始边界钳制） | 局部见证 | t=0.125；见证弦长 0.1747 m（重放回退载体） |
| 6 | `sumo::-10100_1_0` / 0 | 首弦（起始边界钳制） | **全参数域排除** | 39 盒、深度 7、20 排除叶（最短弦 2 / 起始方向 18），复核器通过 |
| 7 | `sumo::-10100_1_0` / 0 | 末弦（规定倒圆，无方向条件） | 局部见证 | t=0.5；见证弦长 0.3821 m |
| 8 | `sumo:--31624_0` / 3 | 首弦（规定倒圆，无方向条件） | 局部见证 | t=0.125；见证弦长 0.5223 m |
| 9 | `sumo:-30976#0_0` / 2 | 末弦（末端边界钳制） | 未决 | 32,000,000 盒、深度 48、pending 27 |
| 10 | `sumo::-10414_4_0` / 1 | 末弦（规定倒圆，无方向条件） | 局部见证 | t=0.5；见证弦长 0.3148 m |
| 11 | `sumo::-22822_2_0` / 1 | 末弦（规定倒圆，无方向条件） | 局部见证 | t=0.5；见证弦长 0.3150 m |
| 12 | `sumo::-1426_1_0` / 0 | 首弦（起始边界钳制） | 局部见证 | t=0.125；见证弦长 0.1668 m（重放回退载体） |
| 13 | `sumo::-1426_2_0` / 2 | 末弦（规定倒圆，无方向条件） | 局部见证 | t=0.5；见证弦长 0.4811 m |
| 14 | `sumo::-19726_2_0` / 1 | 末弦（规定倒圆，无方向条件） | 局部见证 | t=0.5；见证弦长 0.3165 m |
| 15 | `sumo::-22042_7_0` / 3 | 首弦（规定倒圆，无方向条件） | 局部见证 | t=0.25；见证弦长 0.1795 m |
| 16 | `sumo::-22042_7_0` / 3 | 末弦（规定倒圆，无方向条件） | 未决 | 32,000,000 盒、深度 48、pending 33 |
| 17 | `sumo:-32770#6_1` / 4 | 末弦（末端边界钳制） | 局部见证 | t=0.939285…（非网格节点）；见证弦长 0.1097 m |
| 18 | `sumo:-32770#7_1` / 3 | 末弦（末端边界钳制） | 局部见证 | t=0.944962…（非网格节点）；见证弦长 0.1045 m |
| 19 | `sumo::-25332_7_0` / 3 | 末弦 | 不适用（边有候选） | 阶段一两族各 8 份候选 |
| 20 | `sumo::-25332_7_1` / 2 | 末弦（规定倒圆，无方向条件） | 未决 | 32,000,000 盒、深度 48、pending 33 |
| 21 | `sumo::-8018_7_0` / 0 | 首弦（起始边界钳制） | 局部见证 | t=0.0625；见证弦长 0.1324 m |

证明范围与正交记录（见 `diagnosis.json`）：

- `sumo::-10100_1_0` 的首弦问题覆盖全部首弦与单弦情形且全参数域排除，**单独**即否定该固定 cubic 的完整发射（量化弦长下限与起始边界钳制方向锥的已证固定冲突）——这是继第 1 轮 `sumo:--30306#2_0` 之后的**第二例已证固定冲突**，所在 C12 为单缺边分量；同边的末弦局部见证与之正交并存（沿第 1 轮 §3.6 口径，见证与冲突不作整边互斥分类）。它不构成该边其他 span、整边、分量或通用问题的无解证明，也不禁止改控制柄/合并分段的其他曲线设计。
- 13 个局部见证只证明对应局部问题存在解。见证参数注入后的整边搜索 13 次全部零候选：9 次 `retained_family_exhausted`、4 次 `work_budget_stopped`；`witness_injection_records` 共 1,577 条（1,572 条 `witness_chord_attempted=true`，5 条为图搜索预算停止的 not_recorded 口径），全部 `witness_chord_qualified=true` 但 `witness_chord_in_any_returned_path=false`——见证弦合格，注入 cubic 的弦图没有从它延续到终点的完整保留路径。注入**不强制**使用见证弦（`graph::paths_on` 枚举全部 `i<j` 节点对）；13 次注入与 13 个见证一一对应，形态与第 1 轮一致。
- 6 个未决均为诊断限额（32,000,000 参数盒、累计深度 48）到达；未决不产生排除，也不产生可行证明。
- 与旧搜索的关系分两层记录：参数族成员关系按冻结算法直接计算——13 个见证参数中 11 个恰为 `{i/512}` 网格节点（t ∈ {0.0625, 0.125, 0.25, 0.5}），`sumo:-32770#6_1` / `sumo:-32770#7_1` 的 t 非网格节点（`calibrated_grid_node: false`）；实际访问证据为 `not_recorded`（既有 `history` 仅有摘要，不能由成员关系推断实际访问）。成员关系为真也不构成整边可行：注入后整边搜索仍族耗尽或预算停止。
- **控制点来源与交叉核对**（lead §7 重新授权的三层载体链）：16 条边由 fresh 搜索报告 `failures` 行解析（`fresh_failures`），0 条命中 1107 历史层（fresh 层优先），2 条由 `repair_curve` 只读重放回退解析（`replay_fallback`：`sumo::-10034_4_0`、`sumo::-1426_1_0`，均 span 0——其网格走查通过该 span、报告不留行）。全部 20 条已诊断问题的 `replay_agrees_with_control=true`（重放六点与主载体前四点逐位一致）。其中 fresh 载体专属 12 边（`sumo:-32592#0_0`、`sumo::-1000_2_0`、`sumo::-1000_5_0`、`sumo::-1000_7_0`、`sumo:--31624_0`、`sumo:-30976#0_0`、`sumo::-10414_4_0`、`sumo::-22822_2_0`、`sumo:-32770#6_1`、`sumo:-32770#7_1`、`sumo::-25332_7_1`、`sumo::-8018_7_0`）的等值断言在四轮正式实验全部为 true，构成「回退加入前后逐值不变」的运行时闭环（静态单测覆盖历史载体 4 边 + 回退 2 边，见复现）。

## 阶段三：组合验收与两口径收益

11 个分量在基线/新增两组均执行联合赋值检查；候选齐备前置条件不满足，全部保持 `missing_real_programs`、组合赋值 0 次（不记组合穷尽或编译拒绝）：

| 分量 | 待赋值边（去重池） | 有候选边 | 结果 |
| --- | ---: | --- | --- |
| C07 | 5（含既有候选边 `sumo::-29388_3_0`） | 1/5（仅既有 8 份） | 不齐备，`missing_real_programs` |
| C09 | 1 | 0/1 | 不齐备（`sumo::-10034_4_0` 零候选） |
| C11 | 1 | 0/1 | 不齐备（`sumo::-10046_0_0` 零候选） |
| C12 | 1 | 0/1 | 不齐备（`sumo::-10100_1_0` 零候选） |
| C16 | 4 | 0/4 | 不齐备 |
| C20 | 1 | 0/1 | 不齐备（`sumo::-1426_1_0` 零候选） |
| C21 | 1 | 0/1 | 不齐备（`sumo::-1426_2_0` 零候选） |
| C28 | 1 | 0/1 | 不齐备（`sumo::-19726_2_0` 零候选） |
| C37 | 1 | 0/1 | 不齐备（`sumo::-22042_7_0` 零候选） |
| C40 | 6 | 1/6（`sumo::-25332_7_0` 16 份） | 不齐备 |
| C41 | 1 | 0/1 | 不齐备（`sumo::-25490_4_0` 零候选） |

计划 §4 的组合口径已实现并在证据中留痕但本轮未触发：C07 含既有候选边的总组合按待赋值边去重池连乘为 8⁵ = 32,768 量级（>256 即 `assignment_budget_stopped` + 未访问赋值数），C16/C40 同理；因候选不齐备未进入赋值循环，无 `assignment_budget_stopped`（未访问赋值数不适用）。8 个单缺边分量按「出候选即验收」口径本轮无候选可验。全 53 分量的组合赋值合计 62 次（与基线相同，来自其余分量的盘点验证）。

C20/C21 互为 fan-out 钳制邻居（`sumo:-31916_0` 出口扇出，钳制方向为冻结值）：两分量本轮各自独立保持 `missing_real_programs`，邻居关系仅在后续任一方出候选时需要协调解释，钳制方向本轮未改动。

**收益统计（两口径分列，新增收益不并入基线）**：

| 新增收益 | 「84 原目标」口径 | 「完整关联路径」口径（本批 25 条 ↔ 14 条原目标） |
| --- | --- | --- |
| 基线接受 | 28/84 | 0/25 |
| 新增组接受 | 28/84 | 0/25 |
| 新接受分量 | 0（53 分量中） | 0（11 分量中） |
| 新候选 | 16 份（`sumo::-25332_7_0`，两族各 8，去重后并存） | 同左（C40 待赋值边 1/6 齐备度） |
| 组合赋值 | 62 次（与基线同） | 0 次（11 分量均未进入赋值） |

7 个信号分量另对应 31 条原目标（14 + 31 = 45，为 #790 剩余全部原目标），不在本批样本内。

## 成本与确定性

四个独立 release 进程按计划 §6 顺序 ABBA 执行（搜索族 old→grid、grid→old、grid→old、old→grid；诊断 diagnosis-first、baseline-first、baseline-first、diagnosis-first）：

| 轮次 | 顺序（搜索族 / 诊断） | 整进程墙钟（秒） | 22 边搜索合计（秒） | 诊断含注入搜索（秒） | 基线 / 新增验证（秒） |
| ---: | --- | ---: | ---: | ---: | ---: |
| 00 | old-grid / diagnosis-first | 153.6 | 13.5 | 70.5 | 5.3 / 5.4 |
| 01 | grid-old / baseline-first | 154.8 | 15.4 | 67.1 | 5.0 / 4.8 |
| 02 | grid-old / baseline-first | 150.9 | 12.7 | 67.5 | 4.9 / 4.6 |
| 03 | old-grid / diagnosis-first | 157.6 | 14.3 | 71.2 | 5.3 / 5.2 |

诊断耗时集中在 6 个未决问题（各约 8.8–9.6 秒，均为 32,000,000 参数盒/累计深度 48 限额到达）；13 个局部见证与对应的 13 次见证参数注入搜索合计均小于 2 秒。

四轮全部非计时输出确定性：每轮 66 份输出（65 份非计时 + `timing.json`），后三轮与首轮以 Windows `fc.exe /b` 完整比较，共 **195 组全部逐字节相等**（文件名集合也完全相同）。基线生成物外置 Release 的返工（见差异记录）后以最终冻结源重跑四轮 ABBA，返工前四轮归档 `run-0X-pre-baseline-release`：返工前后逐轮非计时输出 `fc.exe /b` 共 **260 组全部逐字节相等**（比较记录见本机 `pre-baseline-release-comparison.json`）——返工只改打包与读取入口，研究结果不变。文件清单、大小与 SHA-256（仅互操作）见本机 `evidence-manifest.json`；机器与工具链见 `environment.json`（AMD Ryzen 9 9955HX / Windows 11 build 29695 / rustc 1.99.1）；逐轮计时明细见 `measurements.json`，耗时只写 `timing.json`，结果 JSON 无任何计时字段。中止产物不参与比较、按规归档于本机证据目录：`run-00-aborted-span-field`（实现 bug：`span_index_zero_based` 误按问题行读取，实为边级字段，修复后重跑）、`run-00-aborted-span-carrier`（触发 `span_control` §7 停止，经 lead 重新授权后加三层载体链重跑）与 `run-00-aborted-missing-baseline-env`（返工后脚本漏设 `COHORT2_BASELINE_DIR` 的启动中止，补设后重跑）；各批修正均不改变任何搜索、诊断或验收结果口径。

未施加主机空闲门槛；耗时为同机固定输入的描述统计，不推导普遍性能倍数、统计显著性或生产吞吐；工作计数不是实际堆峰值。这些操作在路网生成研究中执行，不在交通仿真 tick 内执行，未进入生产转换，不能推算产品吞吐或实时收益。

## 复现

精确测试名与环境变量（plan §8 已同步补录）：

- 正式实验入口（四轮 ABBA，`--exact --ignored --nocapture --test-threads=1`）：
  `output::emit::s1::recovery_coverage::dependency_recovery::calibration::cohort2_coverage::pinned_lust_cohort2_coverage_diagnosis_experiment`
- 常规（非 ignore）单测：`…cohort2_coverage::cohort2_frozen_sample_and_problem_table_stay_consistent`（问题规则逐值复现 21 问题表、9 首次/13 有历史双向核对）、`…cohort2_coverage::cohort2_baseline_pool_merges_frozen_sources_with_pinned_input_bytes`（来源①→④逐级断言 + 临时目录同字节内容的钉版读取与合并）、`…cohort2_coverage::cohort2_pinned_baseline_loader_rejects_unpinned_bytes`（非钉版字节拒读）、`…cohort2_coverage::cohort2_span_control_fallback_preserves_existing_carriers`（历史载体 4 边回退加入前后逐值不变 + 回退 2 边解析与完整六点）。
- 环境变量（沿第 1 轮口径）：`LUST_SOURCE_DIR`（pinned LuST checkout）、`DIRECT_SUCCESSOR_BASELINE_DIR`（#1105 修复后完整基线）、`COHORT2_BASELINE_DIR`（Release 资产 `cohort2-baseline-inputs.zip` 解压目录，两份基线输入经 SHA-256 钉版断言读取）、`COVERAGE_OUTPUT_DIR`（每次必须为不存在的新目录）、`COVERAGE_ORDER`（`old-grid` / `grid-old`）、`COVERAGE_DIAGNOSIS_ORDER`（`diagnosis-first` / `baseline-first`）。
- 构建：`cargo +1.99.1 test --locked --release -p laneflow-lust-converter --lib --no-run`（Rust 1.99.1）；正式四轮用该冻结二进制直跑（本地脚本 `run-cohort2.ps1` / `collect-evidence.ps1` 存证据目录）。输入、预算与样本均由冻结源码与 `edges.json` 固定，不得换样后沿用结论。

## 结论边界与差异记录

**结论边界**：本轮零恢复不写任何可行或不可行结论——21 边零候选只表示三档有限搜索族未产出完整候选；6 个未决不产生排除也不产生可行证明；13 个局部见证与 1 项全参数域排除（`sumo::-10100_1_0` 首弦，第二例已证固定冲突）都只对**原固定曲线**的对应局部问题成立；`sumo::-25332_7_0` 的 16 份候选是局部候选，C40 不齐备故不记分量恢复。8 个单缺边分量的「出候选即验收」路径本轮未触发。完成本切片不关闭 #790，也不构成 LuST 全网转换成功或生产设计冻结。

**差异记录（lead 裁决并授权，plan §7 口径）**：

1. **共享测试助手可见性放宽（第 1 次授权）**。计划 §7 曾写「与已合入各轮无共享改动面（仅新增研究入口与文档）」；本片对第 1/2 轮共享助手（`calibration::coverage` 的 `coverage_output`/`plan_stop_label`/`grid_node_member`/`edge_input`/`find_span_control`/`span_control` 与 `stream_a_curve_design_c29_c17` 的 `pool_counts`/`per_edge_counts`）作了 `pub(super)` 可见性放宽以复用冻结实现、避免词汇映射分叉。lead 裁决授权，条件为仅可见性、语义逐字不变——§7 表述据此修正为「共享改动仅限测试助手可见性放宽（及第 2 条授权的载体扩展），行为不变」。
2. **`span_control` 载体链扩展（第 2 次授权，§7 重新授权）**。执行中触发第 1 轮实现预置的 §7 停止（`span 0 control missing for sumo::-10034_4_0`）：网格搜索报告的 `spans` 行逐次重放暂态、零候选边只在 `failures` 留失败 cubic 行，成功走过的诊断 span 不留痕。经停止、归档与报批，lead 授权在共享载体链**末尾**追加 `repair_curve` 重放回退（`sampling_observation` 只读采集，逐 cubic 六点控制；geom 侧仅可见性暴露既有采集入口）：既有两层（fresh 两族 → 1107 history）优先级与解析结果不变，仅缺口边走回退，§7 panic 语义扩为「三层载体全失才停」。每条诊断记录 `span_control_source` 与重放交叉核对列；`span_control` 回退使用边数记于 `diagnosis.json`（2 条）。
3. **见证注入匹配改用完整六点（随第 2 次授权的行为差异）**。第 1 轮 `Injection.control` 对 `spans` 行载体零填充后四位，注入命中依赖 `failures` 行的完整六点；本轮起注入匹配统一取完整六点（`span_control` 返回 `injection_control`，含 `dir_a`/`dir_b`），不再零填充。该差异只影响注入命中率，不影响诊断判据与排除/见证结论；13 次注入全部命中注入网格并留痕（第 1 轮口径的 `witness_injection_records` 语义不变）。
4. **实现 bug 修正（不涉计划口径）**。首轮中止 `run-00-aborted-span-field`：`span_index_zero_based` 误按问题行读取（实为 `edges.json` 边级字段），修复后全部结果按冻结问题表执行；纯字段读取修正，不改任何搜索、诊断或验收口径。
5. **基线生成物外置 Release（维护者裁定），仓库不留长 JSON**。`baseline/stream-a-searches.json` 与 `baseline/stream-a-results.json` 属搜索/运行生成物，按维护者裁定从分支历史清除，改由 Release 资产 `research-evidence-1134-cohort2-baseline`（`cohort2-baseline-inputs.zip`，含 `MANIFEST.json`）承载；plan §5 读取入口改写为 `COHORT2_BASELINE_DIR` 解压目录 + 代码内 SHA-256 钉版断言（钉版值与字节数不变，身份不依赖仓库内文件），计划 §7 表述沿前述修正口径。返工后以最终冻结源重跑四轮 ABBA，与返工前四轮非计时输出逐字节相等（260 组，见「成本与确定性」），研究结果不变。

下一步依据见[决策材料](g1-proposal.md)，生产 S1 的 G1 保持 Proposed。
