# 见证引导整边搜索族：候选资格验证（sumo:--30266_0 线索）

**结论：研究完成，本轮零入池（8/8 候选被双向拟合筛查拒绝，同表示 B 口径复算后仍 8/8——真实资格结论：本轮见证引导族产出候选不合格）、零恢复——有效结论、分列保存；生产 S1 的 G1 仍未接受。** #1154 按[冻结计划](plan.md)完成见证清单一次冻结（47 项 = 1 true / 7 false / 39 null；D 路 15 项 = 1/2/12 并列）、注入变体整边搜索（首弦 `[0,t]+>t 节点`、移除 ≤t 旧节点；末弦对称；不强制见证弦）、候选资格 (a)(b)(c) 分层验证与四轮 ABBA：84 例注入执行（上限 94）产出 8 份候选（1 次 `program_pool_full` 满池、73 次族耗尽、10 次预算停止），**(a) 资格验证 8/8 `fit_rejected`**——全部被双向拟合筛查拒绝（`quality::fit` 同算法双向上界估计未 ≤ 原参考值），零入本轮新增池；(b) 停止线三情形 26 行（原成功 22 行逐字节保持、本轮恢复 0、未恢复 4）；(c) 见证涉及 19 分量零新增接受。两口径收益持平：84 原目标 28/84 → 28/84、见证涉及关联路径 131 条口径 0 新增。

本研究对应 #1154（父任务 #790），基线 `main` = `90ada2d1`，实现冻结于 `dca5abfe41ae8d02fc4d9bca996f9502bc9d43a1`。执行口径冻结于[计划](plan.md)（含统筹裁决入档：池消费序正式语义、ABBA 双轴与同序对判据、复核域独立计费、遍历序修正版），见证全集 = [`witness-inventory.json`](witness-inventory.json)（一次冻结）。不改生产采样、compiler、阈值与原成功几何；见证变体候选不回写标准族池；生产 S1 的 G1 保持 Proposed（[决策材料](g1-proposal.md)）。

## 冻结与证据

输入为 pinned LuST `c4bd5bd3751d426d42a9a1749c815e47ea188549`；完整回归沿用 #1105 修复后基线（84/53/183/16,265/8,230 逐行一致，每轮全网捕获断言）；共同基线经 E0 冻结门面读取（pool `31fca696…`、锚 `d025c4bf…`、MANIFEST `656fb97e…` 钉版）。预算三计数器：每边单 `WorkBudget` 句柄共享 32M 网格工作池（注入先行 → 标准网格对照得剩余量，先到即停、不按见证续杯；E 预算 API 截断/实耗语义）、原族对照重放独立记账、验收层双向拟合**复核域独立计费**（lead 裁决，`quality::fit_with_handle` 独立句柄）。

四独立 release 进程 ABBA（轴一 池消费序 × 轴二 验证对照序）：run-00/03 `injection-first + baseline-new`、run-01/02 `grid-first + new-baseline`：

| 轮次 | 消费序 / 验证序 | 墙钟（秒） | 注入执行/跳过 | 候选 | 标准网格跳过 |
| ---: | --- | ---: | --- | ---: | ---: |
| 00 | injection-first / baseline-new | 82.7 | 84 / 10 | 8 | 10 |
| 01 | grid-first / new-baseline | 79.5 | 74 / 20 | 8 | 0 |
| 02 | grid-first / new-baseline | 82.5 | 同 01（同序对） | 8 | 0 |
| 03 | injection-first / baseline-new | 83.9 | 同 00（同序对） | 8 | 10 |

**确定性（同序对判据，统筹裁决）**：同序对 run-00≡run-03、run-01≡run-02 非计时输出 **126 组 `fc.exe /b` 全部逐字节相等**；跨序 126 组中 **6 行差异**（`qualification.json`、`results.json`、`witness-guided-searches.json` 各 2 组）记**预算顺序敏感性**证据——消费序真实改变执行面（注入先行时 10 个标准网格因池尽跳过；网格先行时 20 个注入例饿死），印证「主目标不得被对照饿死」的正式语义裁决；evidence-manifest 按对分组记录。搜索/运行生成物外置 Release [`research-evidence-1154-witness-guided`](https://github.com/illusion-tech/laneflow/releases/tag/research-evidence-1154-witness-guided)（`witness-guided-run-outputs.zip`，1,564,432 字节，SHA-256 `043c1ef2…a044`，MANIFEST 含逐文件哈希/来源链）；`successful-programs.jsonl`、LFCA、formal.exe、源码 tar 按 plan §6 留本机证据目录。

## (a) 候选资格验证

84 例注入执行中 1 次 `program_pool_full`（见证参数网格下满池 8 份候选，见证弦入返回路径/选中路径证据形态与 D 轮一致）、73 次 `retained_family_exhausted`、10 次 `work_budget_stopped`；8 份候选全部经资格验证：

| 判定 | 数量 | 说明 |
| --- | ---: | --- |
| `fit_rejected` | **8** | 双向拟合筛查拒绝：`quality::fit` 同算法双向上界估计（curve→source / source→curve）未同时 ≤ 原参考值（`within(reference)` = false） |
| `qualified` / `geometry_rejected` / `not_executed` | 0 | 编译/LFCA/锚点验证不达拒绝线者无（8 份均先折于拟合筛） |

**关键观察与口径修订（维护者裁定 2026-10-30，B 同表示基准）**：冻结口径「双向上界估计 ≤ 原参考同算法值」在折线原生候选下欠规定，裁定解释为**候选折线 vs 原参考的发射折线**（同一估计函数、同表示公平比较，#1107/#1111 折线对折线传统；plan §4 修订记录）。同表示复算（`recompute-same-representation/qualification-recompute.json`，只重跑 (a) 列、不重跑搜索）：8 份候选 **8/8 仍 `fit_rejected`**——**拒绝由前向分量决定**（候选折线→来源 0.769 m vs 原参考发射折线 0.027 m；见证弦 `[0,t≈0.0688]` 跨约 8.9 m 切过道路弯折、离面约 0.77–0.82 m 主导，与表示无关的真实几何偏离），**真实资格结论：本轮见证引导族产出的全部 8 份候选（同一见证弦形态）不合格**；其余注入例零候选，(c) 终验不触发（池空）。**观察项（不改裁定）**：参考反向估计 125.19 m 在曲线/折线两种表示下取值相同（fit 反向机制异常），因 `within` 由前向分量即否定，全部裁定与该异常无关，如实记录备查。原跨表示 8/8 拒绝（参考反向受曲线控制多边形项污染）作为表示伪影证据保留（`cross_representation_artifact` 列随复算输出）。复核域独立计费：原跑 55 次 fit / 27,092,736 单位、复算 16 次 fit / 229,920,768 单位、编译/LFCA 各 8——单独报出、不占搜索域池。

## (b)(c) 停止线三情形与分量接受

- **(b)** 沿 D 冻结 26 行口径逐边产出（复用 `signal_diag_2::stop_line_checklist`，仅可见性放宽）：原成功 22 行程序逐字节保持 22/22；本轮恢复 0 行（4 条缺程序停止线边均未取得入池候选，情形 2 如实记未触发）；未恢复 4 行来源绑定核对一致 4/4。「未执行」与几何拒绝分列。
- **(c)** 见证涉及 19 分量（47 缺边各自的归属分量）在基线/见证两组均执行联合赋值检查；见证池空 → 全部保持 `missing_real_programs`、组合赋值 0 次；`sumo::-29244_4_0` 全参数域排除等历史结论不变。

## 成本与确定性

四轮墙钟 82.7 / 79.5 / 82.5 / 83.9 秒（同机固定输入描述统计；未施加主机空闲门槛）；结果 JSON 无计时字段（耗时只写 `timing.json`）。这些操作在路网生成研究中执行，不在交通仿真 tick 内执行，不能推算产品吞吐或实时收益。

## 复现

- 正式实验入口（四轮 ABBA，`--exact --ignored --nocapture --test-threads=1`）：`output::emit::s1::recovery_coverage::dependency_recovery::calibration::witness_guided_search::pinned_lust_witness_guided_search_experiment`
- 常规（非 ignore）单测：`…witness_guided_search::witness_inventory_stays_frozen`（47 项分类/边序/绑定/null 原因逐值）、`…injection_variants_follow_frozen_order_and_domains`（形态序/域门控/调用上限 94）、`…edge_budget_handle_shares_per_edge_without_refill`（每边单句柄共享、先到即停）。
- 环境变量：`LUST_SOURCE_DIR`、`DIRECT_SUCCESSOR_BASELINE_DIR`、`SIX_STREAMS_BASELINE_DIR`、`WITNESS_GUIDED_OUTPUT_DIR`（每轮全新）、`WITNESS_GUIDED_ORDER`（`injection-first`/`grid-first`）、`WITNESS_GUIDED_VALIDATION_ORDER`（`baseline-new`/`new-baseline`）。
- 构建：`cargo +1.99.1 test --locked --release -p laneflow-lust-converter --lib --no-run`（Rust 1.99.1）；四轮用冻结二进制直跑（本地脚本 `run-witness-guided.ps1` / `collect-witness-guided.ps1` 存证据目录）。

## 结论边界与差异记录

**结论边界**：8/8 `fit_rejected` 只说明见证变体候选未通过**冻结口径的双向拟合筛查**（折线 vs 曲线表示差见 (a) 关键观察），不构成见证引导族的无解证明；零入池、零恢复为本轮有效结论，不回写标准族池。跨序 6 行差异为预算顺序敏感性证据（执行面与资格结论随消费序变化），同序对逐字节相等为确定性判据达成。生产 S1 G1 保持 Proposed。

**差异记录**：

1. **双向拟合筛查表示口径修订记录（维护者裁定 2026-10-30，B 同表示基准）**：冻结口径在折线原生候选下欠规定 → 同表示解释（候选折线 vs 原参考发射折线、同一估计函数）；原跨表示 8/8 拒绝作为**表示伪影证据保留**（不删，`cross_representation_artifact` 列随复算输出），同表示复算（只重跑 (a) 列、不重跑搜索）**仍 8/8 `fit_rejected`，真实资格结论：族不合格**（前向分量 0.769 vs 0.027 主导、见证弦离面真实几何偏离）。附带观察：参考反向估计 125.19 m 在两种表示下相同（fit 反向机制异常），不影响任何裁定，如实记录备查。
2. **共享接口协作**：E 的 `WorkBudget`/`quality::fit_with_handle` 专支（`research/1137-shared-judge-ext`，PR #1164）以 merge 接入（含其侧两次 E0603/警告修复跟进），PR 时按 #1145 先例 rebase 去重；复核域独立计费两条件（计数器单独报出、预算澄清句）已入 plan §2 与 `qualification.json`。
3. **同序对判据**：按统筹裁决，跨序差异（6 行）不作确定性失败处理，单独记「预算顺序敏感性」并按对分组入 evidence-manifest。

下一步依据见[决策材料](g1-proposal.md)，生产 S1 的 G1 保持 Proposed。
