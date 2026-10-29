# C12 单缺边的曲线/分段设计变量有限探索

**结论：有限设计变量探索完成，C12 保持 `missing_real_programs`，本轮零恢复（有效结论）。** `sumo::-10100_1_0`（s=4，临时程序上限 385 点）的 148 个预定义几何配置**全部访问、无预算停止**（`completed_finite_geometry_family`），**0 份保留候选**；C12 单条完整路径保持缺程序、组合赋值 0 次。基线重新验证与重锚 `results-anchor.json`（92 池行代际）逐值一致（84 目标 = 28 接受 + 3 跨边方向拒绝 + 53 未完成目标检查）。首弦全参数域排除与 span 0 末弦见证均只对**原固定曲线**成立；本族无候选只表示本族未通过，**不作无解证明**。这是测试研究的有限探索记录，不是生产转换接入或生产 G1 接受。

本研究对应 #1136（六路协调任务书 v2 的 A 路），方案冻结于 `4236b685`（[plan.md](plan.md) + [edges.json](edges.json)，维护者审阅四点确认），实现冻结于 `c7449993`（含 E0 门面修复 `5beb4a84`）。执行口径冻结于[计划](plan.md)，方法事实源为[design.md](../issue-1115-failed-curve-design/design.md)，实现模板为[流 A 前作](../issue-1131-curve-design-c29-c17/README.md)。生产采样、compiler 配置档、阈值、原成功几何、拓扑与控制锚点均未修改；本轮只增加 Rust `cfg(test)` 研究入口与本报告。Traffic Runtime API、数据格式、Adapter API、依赖与许可证均无变化。生产 S1 的 G1 保持 Proposed（[决策材料](g1-proposal.md)）。

## 冻结与证据

C12 为**单缺边分量**：唯一缺边 `sumo::-10100_1_0`，单条完整路径 `sumo:-10100:--31480_0-to--30896_0::-10100_1_0.:-10100_4_0`。进入依据双记录（详见 [edges.json](edges.json)）：

- **首弦全参数域排除**（span 0，起始边界钳制）：39 盒/深度 7/20 排除叶（最短弦 2 / 起始方向 18），复核器通过——覆盖全部首弦与单弦情形，单独否定该**固定 cubic** 的完整发射（量化弦长下限与起始边界钳制方向锥的已证固定冲突，继 `sumo:--30306#2_0` 后第二例）。不禁止改控制柄/合并分段的曲线设计。
- **span 0 末弦局部见证**（规定倒圆，无方向条件）：t=0.5、见证弦长 0.3821 m——**仅为局部问题存在性记录，不是整条边末端的可行证明**；与首弦排除正交并存。

输入事实：源长 3.04 m；原诊断 span 0/4 → **s=4**、临时程序点上限 385（96s+1）；起始 fan-out 钳制与末端 pair 钳制保持原 f64 位；span 0/1 接缝为规定倒圆切点（见证对应位置）。历史三档搜索（`1103-base`/`1107-old` `finite_family_exhausted`、`1107-grid` `retained_family_exhausted`）均零候选，仅作对照、不引用旧结论。

**共同基线**（E0 冻结交付 `research-evidence-1140-shared-baseline`，经 `SIX_STREAMS_BASELINE_DIR` + `shared_baseline::shared_pools()` 哈希/顺序/份数断言读取）：11 边 92 份（合并顺序 1103→1107→1111→1115→1131→1134，规范 f32 点位去重；禁用旧 9/10 边池）；接受对照 28/84；结果对照锚 = `evidence/run-03/results.json#variants.new`（generation **92-池行**，`comparison_only`，来源三元组在代码内钉版）。

**生成物外置**：搜索/运行生成物压缩挂 [Research Evidence: #1136](https://github.com/illusion-tech/laneflow/releases/tag/research-evidence-1136-c12-curve-design) 预发布（`c12-curve-design-evidence.zip`，69,776 字节，SHA-256 `9fc5fa6f…55cfb3`，内含候选冻结产物、四轮逐字节一致的 search/results、冻结输入、186 组比较记录与 `MANIFEST.json` 来源链）；仓库只留计划、入口与本摘要。

## 逐配置记录（148 个预定义几何配置，全部访问）

因子表 `[0.5, 0.75, 1, 1.25, 1.5, 2, 3]`²、三组分段（原参考 + 保留分界/顺序两段合并/整边合段 × 49 因子对），回折拒绝 = 控制多边形在组端点弦方向出现负增量；来源拟合拒绝 = 双向相对筛选（64 区间/弦）劣于原参考对应值；「两划分族均无完整程序」= n∈2..96 等 t 与 512 步累计切向角两族均未产出过 sink/端弦/0.01 m 全通过的完整程序：

| 配置组 | 数量 | 回折拒绝 | 来源拟合拒绝 | 两划分族均无完整程序 | 成功 | 保留候选 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 不跨规定倒圆接缝（reference + keep，50） | 50 | 18 | 13 | 19 | 0 | 0 |
| 跨规定倒圆接缝（pair + whole，98） | 98 | 30 | **55** | 13 | 0 | 0 |
| **合计** | **148** | **48** | **68** | **32** | **0** | **0** |

跨缝标记（`crosses_prescribed_fillet_seam`）逐配置与逐候选记录（本轮无候选，配置级记录见 `c12-searches.json`）。**尝试数**：完整划分尝试 6,080、采样工作 98,823、来源拟合点到线段评估 5,854,464、去重键保留 0 点；`stop_reason = completed_finite_geometry_family`（无预算停止、无有限族外裁剪）。成功 n：无（本族无任何完整合法程序）。有限尝试无候选只表示本族未通过，不作无解证明。

## 长度与拟合

按 design.md 清单（正向/反向依次表示曲线到来源、来源到曲线；完整双值保存于外置 `c12-searches.json` / `c12-results.json`）：

| 项 | 值（米） |
| --- | ---: |
| 来源 `@length` | 3.040000 |
| 来源 shape 折线长 | 3.049574 |
| 原参考弧长（弦估计 / 控制多边形估计，4096 区间） | 3.015732 / 3.015732 |
| 原参考双向拟合（正/反） | 0.089324 / 0.093503 |
| 新参考弧长 / 规范 f32 弦长累计 / compiler `length_mm` | 无候选，不适用 |

失败边无新规范长度；`compiler` 列因无接受编译为 N/A。禁止比例缩放或重写源长度；长度由现行 compiler 推导和验收（本轮未达编译）。普通 f64 估计不构成向外舍入的连续认证。

## C12 完整分量验收结论

- **单路径完整分量验收**：`sumo::-10100_1_0` 零候选 → 分量保持 `missing_real_programs`，组合赋值 0 次（`selected_assignment: null`，无 `assignment_budget_stopped`、未访问数不适用）；单条完整路径不接受。两组（基线/新增）一致。
- **两口径收益**（章程 §1 分列）：新候选 0 份、新接受分量 0 个、组合赋值 62/62 次（与基线同，来源为其余分量盘点验证）；**完整关联路径 0 / 84 原目标 0**；路径接受无变化（28/84 → 28/84）。
- 实验独立（章程 §3）：新增池 = 共同基线 + 本路候选（0 份），不吸收其他路候选；既有候选（`sumo::-10564_3_0` 1 份、`sumo::-25332_7_0` 16 份等）随基线入池并入列。

## 完整回归与结果对照

每轮全网捕获与 #1105 基线逐行比较：16,265 条原成功边 f64 程序与规范 f32 点、全部拓扑/控制信息、8,230 条失败诊断均一致（`successful-programs.jsonl`/`topology.json`/`failed-edges.json` 逐字节断言）；53 分量、183 关联路径、84 目标断言齐备；接受分量逐点、停止线所在边、机动门路径、转换下标、信号组绑定核对与 LFCA 后发射检查照常执行（本轮无新接受分量；基线 28 个接受分量制品照常生成核对）。

**结果对照（仅对照、不作候选来源）**：基线重新验证的 53 分量结果、84 路径结果与 `path_counts` 与重锚 `results-anchor.json#variants.new` **逐值一致**（含 `unvisited_assignments` 字段形态；28/84 = 28 + 3 + 53）。

## 成本与确定性

四个独立 release 进程按 plan §7 顺序 ABBA 执行（候选冻结步先行产出 `c12-searches.json` 作 `CURVE_DESIGN_FROZEN_RESULT`，0.06 秒）：

| 轮次 | 顺序 | 整进程墙钟（秒） | 单边搜索（秒） | 基线 / 新增验证（秒） | 捕获解析 / 全网转换（秒） |
| ---: | --- | ---: | ---: | ---: | ---: |
| 00 | baseline-new | 57.33 | 0.032 | 3.57 / 3.65 | 0.51 / 48.73 |
| 01 | new-baseline | 58.50 | 0.032 | 3.61 / 3.62 | 0.53 / 49.84 |
| 02 | new-baseline | 57.17 | 0.032 | 3.62 / 3.56 | 0.52 / 48.61 |
| 03 | baseline-new | 59.29 | 0.032 | 3.67 / 3.62 | 0.54 / 50.57 |

四轮全部非计时输出确定性：每轮 63 份输出（62 份非计时 88,388,379 字节 + `c12-timing.json`，其中 56 份接受分量 LFCA），后三轮与首轮以 Windows `fc.exe /b` 完整比较，共 **186 组全部逐字节相等**（文件名集合完全相同）。文件清单与 SHA-256（仅互操作）见外置 `evidence-manifest.json`；机器与工具链：AMD Ryzen 9 9955HX / Windows 11 build 29695 / rustc 1.99.1。结果 JSON 无任何计时字段，耗时只写计时文件。未施加主机空闲门槛；耗时为同机固定输入的描述统计，不推导普遍性能倍数、统计显著性或生产吞吐；工作计数不是实际堆峰值。

## 复现

- 候选冻结入口（先跑，其 `c12-searches.json` 作 `CURVE_DESIGN_FROZEN_RESULT`）：`output::emit::s1::recovery_coverage::dependency_recovery::c12_curve_design::fixed_c12_curve_design`
- 正式实验入口（四轮 ABBA，`--exact --ignored --nocapture --test-threads=1`）：`output::emit::s1::recovery_coverage::dependency_recovery::c12_curve_design::pinned_lust_c12_curve_experiment`
- 常规（非 ignore）单测：`…c12_curve_design::c12_cohort_and_capacities_match_the_frozen_plan`、`…c12_curve_design::c12_fillet_seam_crossing_tracks_group_boundaries`、`…c12_curve_design::curve_capacity_parameterization_keeps_the_frozen_six_default`
- 环境变量：`LUST_SOURCE_DIR`（pinned LuST）、`DIRECT_SUCCESSOR_BASELINE_DIR`（#1105 修复后基线）、`SIX_STREAMS_BASELINE_DIR`（E0 基线包解压目录，`shared_baseline::shared_pools()`/`results_anchor_pinned()` 哈希断言）、`CURVE_DESIGN_OUTPUT_DIR`（每次必须为不存在的新目录）、`CURVE_DESIGN_ORDER`（`baseline-new` / `new-baseline`，四次按 ABBA 各设一个值）、`CURVE_DESIGN_FROZEN_RESULT`。输出目录、结果文件与测试模块命名以 `c12` 区分。
- 构建：`cargo +1.99.1 test --locked --release -p laneflow-lust-converter --lib --no-run`（Rust 1.99.1）；正式四轮用冻结二进制直跑（本机脚本 `run-formal.ps1` / `collect-evidence.ps1`，不入库）。输入、样本与预算由冻结源码与 [edges.json](edges.json) 固定，不得换样后沿用结论。

## 结论边界与差异记录

**探索性定位（plan §1/§6）**：本路是「已证冲突 + 单缺边」形态上的有限设计变量探索。首弦全参数域排除只否定**原固定 cubic** 的完整发射；span 0 末弦见证（t=0.5）只是局部问题存在性、非整条边末端可行证明；148 配置有限族完成且无候选只表示**本族未通过**。零恢复为有效结论，不预设必须恢复，不写可行或不可行结论。

**不变量确认**：原 SUMO 输入、全部原成功程序/规范点、身份、拓扑、控制器成员、信号组、停止线、机动门声明、首末坐标与钳制方向（起始 fan-out、末端 pair）保持原 f64 位；sink 1.95°、端弦 0.95°、规范弦严格大于 0.1 m、0.01 m 研究筛选、compiler 配置档全部不变；新规范长度不等同源 `@length`（本轮无候选，未产生新规范长度）。

**过程记录（实现缺陷与授权，均不改研究口径）**：

1. **中止批次一**（`candidate-run-aborted-seam-group-reference*`）：跨缝标记映射漏原参考组名 `reference`（reference/keep 均保留全部分界、不跨缝），首跑 panic 后修复并补单测；不改任何候选族、筛选或预算口径。
2. **中止批次二**（`run-00-aborted-anchor-unvisited-field*`）：结果对照断言在 `unvisited_assignments` 字段增强**前**比较，而对照锚的分量行含该字段（#1134 入口产出形态）——修正为按 plan §5 增强后再逐值比较（含字段形态全等）；对照语义不变、比较更完整。
3. **锚代际错位事故**（跨路，见六路事故记录）：初版 `results-anchor.json` 取材 #1131 代（76 池行）；经 D 路停止报告与统筹裁决，E0 重钉为 #1134 新增变体（92 池行，`generation` 标签 + `changed_from` 来源链 + MANIFEST 钉版），本路断言钉死新三元组（`research-evidence-1140-shared-baseline` / `evidence/run-03/results.json` / `#variants.new`）与 `generation: "92-池行"` 作回归闸口。
4. **E0603 使用方复核**：`calibration::shared_baseline` 门面 mod 原为私有、跨子树消费被 rustc 拒绝；按章程 §2 停手报统筹，E 以 `pub(in super) mod shared_baseline;` 修复（`5beb4a84`），rustfmt 等价归一化为 `pub(super)`（语义不变，已报备）。
5. 浮点解析跨片事实（C 路发现）：serde_json 默认解析对 17 位十进制浮点可误舍入 1 ULP；本路对照为双侧同源解析（`normalized` 形态）语义有效，未装位级硬门（如需等待 E 的文本域比较助手）。

完成本切片不关闭 #790，也不构成 LuST 全网转换成功或生产设计冻结；下一步依据见[决策材料](g1-proposal.md)，生产 S1 的 G1 保持 Proposed。
