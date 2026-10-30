# A-2 执行前方案（冻结计划）：单缺边分量设计变量方法复用（C09/C11/C20/C21/C28/C37/C41）

> 状态：草案，待维护者复审后冻结（Issue #1174）；执行前不得修改。执行中确需变更时先停、记录、重新授权（§7 口径），不得单向迁就。
>
> **共同口径**：六路协调章程 v2（`research/six-streams-20261029/coordination.md`）与第二批派工——共同基线 11 边 92 份、接受对照 28/84、两口径收益分列（每分量 1↔1）、共享代码只由 E 修改、实验排程（重构建/全量实验并发 ≤2）、生成物外置 Release、位级比较用文本 token 域。**方法事实源**：`research/issue-1115-failed-curve-design/design.md`（148 配置族、双向来源拟合估计、长度口径、资源与停止）+ #1154 裁定 B 同表示资格口径（候选折线 vs 原参考发射折线，同估计器同表示）+ #1136 实现模板（`research/issue-1136-c12-curve-design/`，C12 基准形态）。本计划只冻结样本、顺序与 A-2 特有约束；与事实源冲突时停止该处执行并重新授权。
>
> **样本 ID 纪律**（2026-10-30 纪律条）：派工/Issue 的样本 ID 必须程序化提取自冻结记录，禁止凭记忆书写；本计划样本表逐值取自 `research/issue-1134-cohort2-coverage-diagnosis/edges.json`，实现内以 include_str 冻结记录回读断言。派工初稿 ID 组与冻结记录不一致，经维护者裁决 (a) 以码本 7 边为准（Issue #1174 正文同步修正）。
>
> 基线 `main` = `d26abb76`（工作树切出点）；LuST 输入 `c4bd5bd3751d426d42a9a1749c815e47ea188549`。

## 1. 样本与进入依据（7 边 7 分量，单缺边单路径；逐项见 `edges.json`）

C12（`sumo::-10100_1_0`）已完成为基准形态；本批为其余 7 个单缺边分量：

| 码 | 缺边（#1134 冻结记录精确拼写） | s | 源长 (m) | 进入依据双记录（#1134 诊断；只作记录，不引用旧结论） |
| --- | --- | ---: | ---: | --- |
| C09 | `sumo::-10034_4_0` | 4 | 5.04 | span0 首弦（起始边界钳制）**局部见证** t=0.125 / 弦 0.1747 m（replay_fallback 载体）；原族 `finite_family_exhausted`、网格 `retained_family_exhausted`，历史零候选 |
| C11 | `sumo::-10046_0_0` | 4 | 9.88 | **endpoint_diagnosis_not_selected**（两端 interior CR，不诊断）；默认档 `partitions_budget_stopped`、扩展档 `finite_family_exhausted`、网格 `work_budget_stopped`，零候选 |
| C20 | `sumo::-1426_1_0` | 4 | 4.92 | span0 首弦（起始边界钳制）**局部见证** t=0.125 / 弦 0.1668 m（replay_fallback 载体）；原族 `finite_family_exhausted`、网格 `retained_family_exhausted`，零候选 |
| C21 | `sumo::-1426_2_0` | 5 | 7.57 | span2 末弦（规定倒圆，无方向条件）**局部见证** t=0.5 / 弦 0.4811 m；默认档 `partitions_budget_stopped`、网格 `work_budget_stopped`，零候选 |
| C28 | `sumo::-19726_2_0` | 5 | 5.10 | span1 末弦（规定倒圆，无方向条件）**局部见证** t=0.5 / 弦 0.3165 m；默认档 `partitions_budget_stopped`、网格 `retained_family_exhausted`，零候选 |
| C37 | `sumo::-22042_7_0` | 5 | 15.99 | span3 首弦（规定倒圆）**局部见证** t=0.25 / 弦 0.1795 m + span3 末弦（规定倒圆）**未决**（32,000,000 盒/深度 48）；默认档 `partitions_budget_stopped`、网格 `work_budget_stopped`，零候选 |
| C41 | `sumo::-25490_4_0` | 4 | 5.01 | **endpoint_diagnosis_not_selected**（两端 interior CR）；原族 `finite_family_exhausted`、网格 `retained_family_exhausted`，零候选 |

- **进入依据语义**：见证只表示对应局部问题存在解（非整边/末端可行证明）；未决不产生排除也不产生可行证明；`endpoint_diagnosis_not_selected` 不是「端点必要条件不适用」。全部为原固定曲线口径，不外推到设计族。
- **C20/C21 fan-out 钳制邻居关系（钳制冻结）**：C20（`sumo::-1426_1_0`）起始钳制 = fan-out mean over maneuver boundary pairs [`sumo:-31916_0->sumo::-1426_1_0`, `sumo:-31916_0->sumo::-1426_2_0`]——两个邻居恰为 C20 与 C21 的缺边（互为 fan-out 成员）；两条边的钳制方向为冻结值、不随曲线改变，候选仍按各自分量独立验收（两分量互不吸收对方候选）。
- **定位**：方法复用性探针——C12 基准形态上的 148 族在其余单缺边形态的复用与恢复能力检验；零恢复是有效结论。

## 2. 设计变量与不变量（design.md §设计变量 + #1136 同款；A-2 特有）

- 可变：失败边内部 Bezier 控制柄；内部分界保留/合并（三组分段）。合并可跨内部位置（含规定倒圆切点）——**跨缝标记与结点索引逐配置+逐候选记录**（#1136 定义沿用：内部接缝结点按 span 边界编号 1..s−1，`crossed_fillet_knots` 只计入被合段跨过的规定倒圆结点，非倒圆内部结点不计入）。各边规定倒圆接缝位置由原诊断 span 记录钉版（C21 span2/3 缝、C28 span1/2 缝、C37 span3/4 缝及其余以原链切向机制为准，实现内断言）。
- 固定：原 SUMO 输入、全部原成功程序/规范点、身份、拓扑、控制器成员、信号组、停止线、机动门声明；各边首末坐标与**存在端**钳制方向保持原 f64 位（含 C20/C21 fan-out 钳制冻结值）；sink 1.95°、存在钳制端端弦 0.95°、规范弦严格大于 0.1 m、0.01 m 研究筛选、compiler 配置档全部不变。
- 每边独立成组：单缺边单路径分量验收；不吸收他路/他边候选（实验独立，章程 §3）；新规范长度不等同源 `@length`；禁止比例缩放/重写源长度。

## 3. 冻结候选族与选择（design.md §冻结的候选族与选择，逐值沿用）

- 每边保留原参考链对照；三组分段按序：保留全部原分界、顺序两段合并、整边合段；组内保持首末点与外侧切向。
- 新柄 = 组端点弦长 / 3 × 因子；首柄/末柄因子各遍历 `[0.5, 0.75, 1, 1.25, 1.5, 2, 3]`（外层首柄、内层末柄，组间同一对因子）→ 每组 49 份；**148 个预定义几何配置**（原参考 1 + 3×49），不按结果追加；记录实际访问数与配置序号。
- 回折拒绝：控制多边形在组端点弦方向的三个增量均须非负。
- 共享机器内判定口径逐值沿用 `geom/s1/reshape`（回折拒绝、双向来源拟合 64 区间/弦、n∈2..96 等 t 与 512 步累计切向角双划分族每配置每族取首个完整程序、原 sink 0.1 m/1.95° 量化预检、存在钳制端 0.95° 端弦、0.01 m 逐区间筛选、规范 f32 点位去重、全局保留 8）。
- **候选资格（#1154 裁定 B，同表示）**：保留池候选逐份过资格闸——候选发射折线 vs **原参考发射折线**（逐 span cubic 以 513 个均匀参数节点采样、f32 规范化、串接；「grid-family base emission form」）双向来源拟合估计（同估计器：折线→控制表示后 64 区间/弦双向估计），候选两个估计值分别不得高于基准对应值。**同表示基准**为裁定 B 的核心：跨表示比较（分析链 vs 折线）已证产生伪影拒判，不采用。资格通过者为「合格候选」入新增池；资格与族内筛选分列记录。
- 保留：固定顺序尝试；全局 ≤8，排序与规范点去重沿 design.md；仅对保留池声称组合穷尽。

## 4. 来源拟合与长度口径（design.md §来源/参考/规范与锚点；资格侧按 §3 B 口径）

- 0.01 m 只筛新参考→量化折线逐区间（共享机器内）；来源拟合 64 区间/弦双向估计（共享机器内为分析链口径；**候选资格**侧为同表示 B 口径，见 §3）。
- 长度报告清单（#1136 同款）：来源 `@length`、来源 shape 折线长、原/新参考弧长（4096 分段弦长与控制多边形估计）、规范 f32 弦长累计、compiler `length_mm` 及差值；失败边新规范长度不声称保持不变、不等同源 `@length`。
- 最终保留代表另测规范折线→来源双向距离估计（计入来源拟合工作预算，64 段/弦）；长度估计只对原参考与保留代表执行（4096 区间/段）。资格基准（原参考发射折线）的双向估计随逐边报告保存（对照锚值）。

## 5. 组合验收与完整回归（六路共同口径）

- **共同基线**（E0 冻结交付 `research-evidence-1140-shared-baseline`，`SIX_STREAMS_BASELINE_DIR` + `shared_baseline::shared_pools()`/`results_anchor_pinned()` 哈希/顺序/份数断言）：11 边 92 份（合并顺序 1103→1107→1111→1115→1131→1134，禁用旧 9/10 边池）；接受对照 28/84；结果对照锚 `evidence/run-03/results.json#variants.new`（generation `92-池行`，来源三元组钉版，仅结果对照）。
- **实验独立**：新增 = 共同基线 + 本批 7 边合格候选（既有候选随基线入池并入列）；本轮新增收益单独统计。**两口径收益分列**（章程 §1）：每分量 1 条完整关联路径 ↔ 1 个原目标（7 分量合计 7↔7 口径分列，收益按 0/1 与 0/1 逐分量表述）。
- **任一边出合格候选即整分量验收**（单路径）：分量级联合赋值（256 上限、按边 ID 与候选索引遍历）+ 整分量一次真实联合编译；只有整个分量接受才选用其赋值；组合超上限记 `assignment_budget_stopped` 与未访问赋值数。逐边资格与分量验收分列报告。
- 完整回归：84 目标、53 分量、183 关联路径；16,265 条成功边 f64 程序与规范 f32 点、全部拓扑/控制信息、8,230 条失败诊断与 #1105 基线逐行一致；接受分量逐点、停止线所在边、机动门路径、转换下标、信号组绑定核对 + LFCA 后发射检查。

## 6. 资源与停止（design.md §资源和停止 + 共享 WorkBudget 计费口径）

- **每边单 `grid::WorkBudget` 句柄**（`per_edge_default`：划分 32,768 / 网格工作 32,000,000 / 来源拟合评估 32,000,000），共享计费不续杯、`*_consumed` 实耗记账；经 `search_with_curve_cap_budget` 接线（reshape 侧共享接线）。资格闸的来源拟合评估计入同一 `source_fit` 口径（复核性评估独立记账不占池，#1154 计费口径）。
- 段容量按 s 冻结：s=4 → 临时程序 ≤ 385 点；s=5 → ≤ 481 点（96s+1，`search_with_curve_cap` 参数化断言）。其余上限沿 design.md：保留池 ≤8、去重键 ≤262,144 点、几何报告 ≤148 行。
- 到限分别报告停止原因（缺程序/筛选拒绝/回折拒绝/有限族耗尽/预算停止/编译拒绝分列，资格拒绝单列）；记录实际访问几何数与配置序号。有限尝试无候选只表示本族未通过，不作无解证明；不追加预算、不放宽筛选、不预设必须恢复；零恢复为有效结论。

## 7. 验证与证据

- 四个独立 release 进程，基线池/新池顺序 ABBA（`baseline-new`、`new-baseline`、`new-baseline`、`baseline-new`）；全部非计时输出保存并逐字节比较（Windows `fc.exe /b`）；结果 JSON 禁计时字段；输出目录必须全新；中止产物归档 `*-aborted-*`。
- **位级比较用文本 token 域**：候选冻结重放与结果对照的逐值核对以 `shared_baseline::json_leaf_mismatch`（数字叶 `f64::from_str` 正确舍入 → `to_bits`）为准，serde 解析域位值不作比较基准（#1155 口径）。
- **生成物外置**：搜索/运行生成物压缩挂 `research-evidence-*` 预发布 + MANIFEST（SHA-256/字节数/来源链），仓库只留计划、入口与摘要；`environment.json`、`measurements.json`、`evidence-manifest.json` 本机保存。
- 常规检查：debug/release lib 回归、`cargo check --all-targets`、Clippy `-D warnings`、rustfmt、JSON 解析、diff 空白检查；入库仅 Rust `cfg(test)`、Markdown、纯 JSON。
- 合入：PR（`Closes #1174`）经九项必需检查走 Merge Queue，不入队、交统筹收口；提交 trailer `Harness: deepseek/harness/web` + `Model: xiaomi/mimo-v2.6-pro`。排程：与各路共用机器，重构建/全量实验并发 ≤2，正式运行前与统筹确认。

## 8. 复现（实现定稿后补精确测试名与环境变量）

- 冻结提交构建 `laneflow-lust-converter` release lib test（Rust 1.99.1），`--exact --ignored --nocapture --test-threads=1`。
- 环境变量（#1136 同款 + E0 基线接口）：`LUST_SOURCE_DIR`、`DIRECT_SUCCESSOR_BASELINE_DIR`、`SIX_STREAMS_BASELINE_DIR`、`CURVE_DESIGN_OUTPUT_DIR`（每次必须为不存在的新目录）、`CURVE_DESIGN_ORDER`（ABBA 序）、`CURVE_DESIGN_FROZEN_RESULT`；输出目录、结果文件与测试模块命名以本批 `single_edge_reuse` 区分（模块名待实现定稿，带 #1174 语义）。
- 输入、样本与预算由冻结源码与本目录 `edges.json` 固定（样本 ID 程序化回读 #1134 冻结记录断言），不得换样后沿用结论。
