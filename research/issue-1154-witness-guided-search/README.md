# 见证引导整边搜索族：候选资格验证（sumo:--30266_0 线索）

**结论：研究完成——同表示 B 口径（维护者裁定）下 8/8 候选双向拟合资格通过并入本轮新增池（依赖不齐、终验记「未执行」）；零恢复不变（有效结论、分列保存）；生产 S1 的 G1 仍未接受。** #1154 按[冻结计划](plan.md)完成见证清单一次冻结（47 项 = 1 true / 7 false / 39 null；D 路 15 项 = 1/2/12 并列）、注入变体整边搜索、候选资格 (a)(b)(c) 分层验证与四轮 ABBA（含 Codex 评审 P2 三项修复后复跑）。84 例注入执行产出 8 份候选（1 次 `program_pool_full`）；(a) 资格：**同表示双向拟合 8/8 通过**（候选折线 0.769/0.822 vs 原参考发射折线 0.773/0.827 m）→ 入池；编译/LFCA/锚点因分量依赖不齐（C24 尚缺 7 边）记 `not_executed`（不混入几何拒绝）；(b) 停止线三情形 26 行（原成功 22 逐字节保持、本轮恢复 0、未恢复 4）；(c) 19 分量零新增接受。两口径收益持平：84 原目标 28/84 → 28/84、见证涉及关联路径 131 条 0 新增。

本研究对应 #1154（父任务 #790），基线 `main` = `90ada2d1`。执行口径冻结于[计划](plan.md)（统筹裁决入档 + 口径修订 B），见证全集 = [`witness-inventory.json`](witness-inventory.json)（一次冻结）。不改生产采样、compiler、阈值与原成功几何；见证变体候选不回写标准族池；生产 S1 的 G1 保持 Proposed（[决策材料](g1-proposal.md)）。

## 冻结与证据

输入为 pinned LuST `c4bd5bd3751d426d42a9a1749c815e47ea188549`；完整回归沿用 #1105 修复后基线（84/53/183/16,265/8,230 逐行一致）；共同基线经 E0 冻结门面读取（pool `31fca696…`、锚 `d025c4bf…`、MANIFEST `656fb97e…`）。预算三计数器：每边单 `WorkBudget` 句柄共享 32M 网格工作池（注入先行 → 标准网格对照得剩余量）、原族对照重放独立记账、验收层双向拟合**复核域独立计费**（lead 裁决 B 案：只记 consumed、不占搜索池、不截断验证算量）。

四独立 release 进程 ABBA（轴一 池消费序 × 轴二 验证对照序；修复后复跑）：

| 轮次 | 消费序 / 验证序 | 墙钟（秒） | 注入执行/跳过 | 候选 |
| ---: | --- | ---: | --- | ---: |
| 00 | injection-first / baseline-new | 94.7 | 84 / 10 | 8 |
| 01 | grid-first / new-baseline | 75.6 | 同序对（执行面见敏感性） | 8 |
| 02 | grid-first / new-baseline | 74.8 | 同序对 | 8 |
| 03 | injection-first / baseline-new | 95.1 | 同序对 | 8 |

**确定性（同序对判据，统筹裁决）**：同序对 run-00≡run-03、run-01≡run-02 非计时输出 **126 组 `fc.exe /b` 全部逐字节相等**；跨序 126 组中 6 行差异（`qualification.json`/`results.json`/`witness-guided-searches.json`）记**预算顺序敏感性**证据（注入先行 vs 网格先行的执行面差异：前者标准网格池尽跳过、后者注入例饿死）；evidence-manifest 按对分组记录。生成物外置 Release [`research-evidence-1154-witness-guided`](https://github.com/illusion-tech/laneflow/releases/tag/research-evidence-1154-witness-guided)（最终复跑输出；评审前版本归档 `*-pre-review-fix*` 留本机）。

## (a) 候选资格验证（口径修订 B 后终值）

84 例注入执行（上限 94）产出 8 份候选（1 次 `program_pool_full` 满池、73 次族耗尽、10 次预算停止），全部来自 `sumo:--30266_0` 首弦见证（t≈0.0688）：

| 判定 | 数量 | 说明 |
| --- | ---: | --- |
| 同表示拟合**通过** | **8** | 候选折线 (0.769, 0.822) ≤ 原参考发射折线 (0.773, 0.827)（`within` 成立；折线对折线、同一估计函数） |
| 编译/LFCA/锚点 | 8 `not_executed` | 分量依赖不齐（C24 尚缺 7 边，`target_check_not_completed`）——完整验证随 (c) 联合编译执行，**不混入几何拒绝统计** |
| 最终 outcome | 8 `not_executed`（入池） | 拟合合格 + 终验待执行 → 入本轮新增池（`witness_pool`：`sumo:--30266_0` ×8），不回写标准族池 |
| 跨表示（伪影证据列） | 8 `pass=false` | 原跨表示 8/8 拒绝 = **截断参考伪影**（P2-1：`replay_span_controls` 遇发射失败即截断，参考仅覆盖失败 span 0/6、前向 0.027 系局部值、反向 125.19 系覆盖缺口）——随 `cross_representation_artifact` 列保留（不删） |

**机理（修正后）**：原修复参考曲线自身在弯折处离源折线约 0.77–0.83 m；候选弦链与参考曲线同源同形，拟合量同级（0.769/0.822 vs 0.773/0.827）——「见证弦跨弯离面」并非候选特有缺陷，跨表示拒判为伪影。复核域独立计费：fit 评估 102 次、实耗 5,828,385,024 单位（计数不占搜索池）、编译/LFCA 各 8。

## (b)(c) 停止线三情形与分量接受

- **(b)** 沿 D 冻结 26 行口径逐边产出：原成功 22 行程序逐字节保持 22/22；本轮恢复 0 行（4 条缺程序停止线边未达分量齐备，情形 2 如实记未触发）；未恢复 4 行来源绑定核对一致 4/4。「未执行」与几何拒绝分列。
- **(c)** 见证涉及 19 分量（按冻结清单 47 边归属，P2-2 修复后口径）在基线/见证两组执行联合赋值检查；C24 依赖不齐 → 全部保持 `missing_real_programs`、组合赋值 0 次；`sumo::-29244_4_0` 全参数域排除等历史结论不变。

## 成本与确定性

四轮墙钟 94.7 / 75.6 / 74.8 / 95.1 秒（同机固定输入描述统计；未施加主机空闲门槛）；结果 JSON 无计时字段（耗时只写 `timing.json`）。这些操作在路网生成研究中执行，不在交通仿真 tick 内执行，不能推算产品吞吐或实时收益。

## 复现

- 正式实验入口（四轮 ABBA，`--exact --ignored --nocapture --test-threads=1`）：`…calibration::witness_guided_search::pinned_lust_witness_guided_search_experiment`
- 同表示复算入口（只重跑 (a) 列、不重跑搜索）：`…witness_guided_search::recompute_witness_qualification_same_representation`
- 常规单测：`witness_inventory_stays_frozen`、`injection_variants_follow_frozen_order_and_domains`、`edge_budget_handle_shares_per_edge_without_refill`；共享入口可达性契约单测随 #1164/#1171。
- 环境变量：`LUST_SOURCE_DIR`、`DIRECT_SUCCESSOR_BASELINE_DIR`、`SIX_STREAMS_BASELINE_DIR`、`WITNESS_GUIDED_OUTPUT_DIR` / `WITNESS_GUIDED_RECOMPUTE_INPUT` / `WITNESS_GUIDED_RECOMPUTE_OUTPUT`、`WITNESS_GUIDED_ORDER`、`WITNESS_GUIDED_VALIDATION_ORDER`。
- 构建：`cargo +1.99.1 test --locked --release -p laneflow-lust-converter --lib --no-run`（Rust 1.99.1）；本地脚本 `run-witness-guided.ps1` / `collect-witness-guided.ps1`。

## 结论边界与差异记录

**结论边界**：8/8 同表示拟合通过仅证明候选折线在双向拟合层「不劣于原参考发射折线」；编译/LFCA/锚点终验因分量依赖不齐**未执行**，候选以 pending 状态入池、不构成恢复；零恢复为本轮有效结论。资格结论随口径而变——跨表示拒判是伪影、同表示通过是终值，两者均如实分列。生产 S1 G1 保持 Proposed。

**差异记录（含 Codex 评审 P2 三项修复，全部接受并复跑验证）**：

1. **P2-1（致命）截断参考**：`replay_span_controls` 遇发射失败即截断（`--30266_0` 失败于 span 0/6 只捕获 span 0），原参考仅覆盖 1/6 边——125.19 m 反向值与跨表示 8/8 拒判皆由此解释。修法：全 span 参考重建（`reference_with_curve_cap` 观察模式入口，独立于发射成败 + 覆盖三断言；段容量不设约束——修复构造 cubic 数可超源段数）；(a) 列与跨表示对照重算、四轮整体重跑（旧输出归档 `*-pre-review-fix*`）。**结论翻转**：同表示 8/8 通过。复核域计数器同步按 lead B 案改「只记 consumed 不占池」（固定验证负载不设池帽，实耗 5.8G 单列）。
2. **P2-2 统计队列**：见证涉及口径改**冻结见证清单**边集合（47 边、执行前确定），跳过执行单独报出（`skipped_execution_edges`；饿死/域外边仍在统计集）。
3. **P2-3 有损解析域**：基线对照改锚**原文文本**切片（钉版 SHA-256 断言）+ `json_leaf_mismatch` 文本 token 域比较（数字叶 `f64::from_str` 位级），不双侧 normalize 进解析域（serde 1 ULP 坑）；组件行比 `result` 成员原文（行级 `cohort` 标记随各路分量不同不入对照）。
4. **执行中止归档**：`*-pre-review-fix*`（评审前四轮与复算）、`run-00-aborted-review-budget`（复核域池帽截断）、`run-00-aborted-reference-cap`/`-cap2`（段容量断言两次）——均为口径修复驱动的迭代，产物归档不删、不参与比较。
5. **共享接口协作**：E 的 `WorkBudget`/`quality::fit_with_handle`/`reference_with_curve_cap`（#1164 + 补遗 #1171）以 merge 接入；PR 时 rebase 去重保持本路 diff 纯净。

下一步依据见[决策材料](g1-proposal.md)，生产 S1 的 G1 保持 Proposed。
