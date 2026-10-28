# 四个最小多边案例的固定曲线搜索补齐与逐边必要条件诊断

**结论：研究完成，生产 S1 的 G1 仍未接受。** #1221 按[冻结计划](plan.md)完成阶段一搜索补齐、阶段二逐边局部必要条件诊断与阶段三组合验收：8 条缺程序边在原候选族（默认档全量、扩展档两条）与 #1111 校准网格下**仍为零候选**，与既有记录逐值一致；诊断取得 **1 项全参数域排除**（`sumo:--30306#2_0` 首弦，41 叶排除证书经复核器验证）、**4 个局部见证**（见证参数注入后整边搜索均为保留网格代表族耗尽）、**5 个未决**（到达 32,000,000 参数盒/累计深度 48 限额）。四个分量保持 `missing_real_programs`，本轮无新增接受目标。

本研究对应 #1121（父任务 #790），源码冻结于 `17839d0dce528d28960e518392ad534e7192b2a4`（计时口径修正 `34c0ca87e534ea51982bdd505a1382ec024896a0`）。生产采样、编译器、阈值与控制锚点均未修改；本轮只增加 Rust `cfg(test)` 研究入口与证据。Traffic Runtime API、数据格式、Adapter API、依赖与许可证均无变化。生产 S1 的 G1 保持 Proposed。

## 冻结与证据

输入仍为 LuST `c4bd5bd3751d426d42a9a1749c815e47ea188549`（`scenario/lust.net.xml` 10,940,662 字节，SHA-256 `6f5d7622…`，与 `src/source/pinned.rs` 钉版值一致；该 checkout 的 git 元数据不含提交对象，钉版以内容摘要为准）。沿用 #1105 修复后完整基线、84 条目标、53 个完整分量、183 条关联路径。样本为[冻结清单](edge-cohort.json)的四个最小多边案例：C06 / C08 / C17 / C29，8 条仍缺程序边、1 条已有候选边 `sumo::-10034_0_0`、8 条完整路径。

预算、判据、注入规则、正交记录口径与停止条件全部冻结于 [plan.md](plan.md)（冻结提交 `9788aab9fcd0817fdbf6ee78ffaea7bcd003be88`，经维护者两轮评审批准）。执行中未发生计划与冻结实现的差异（§7 未触发）。

## 阶段一：固定曲线搜索补齐（8 条边 × 三组搜索）

原候选族默认档 32,768 覆盖全部 8 条边，扩展档 262,144 仅 `sumo::-10004_3_0`、`sumo:--30452_0`；校准网格为对 8 条边的首次执行。三组搜索均零候选；默认档与扩展档逐值重现既有 `1103-base` / `1107-old` / `1103-expanded` 记录（断言通过）。计划词汇停止标签（原始标签保留在各 search 的 `stop_reason`）：

| 边 | 原族默认档 | 原族扩展档 | 校准网格 |
| --- | --- | --- | --- |
| `sumo::-10004_3_0` | partitions_budget_stopped | finite_family_exhausted | retained_family_exhausted |
| `sumo::-10004_8_0` | partitions_budget_stopped | — | retained_family_exhausted |
| `sumo:--30306#2_0` | finite_family_exhausted | — | retained_family_exhausted |
| `sumo::-8806_4_0` | partitions_budget_stopped | — | retained_family_exhausted |
| `sumo::-10564_3_0` | partitions_budget_stopped | — | retained_family_exhausted |
| `sumo::-10564_6_0` | finite_family_exhausted | — | retained_family_exhausted |
| `sumo:--30452_0` | partitions_budget_stopped | partitions_budget_stopped | work_budget_stopped |
| `sumo::-20112_0_0` | finite_family_exhausted | — | retained_family_exhausted |

`candidates_found` / `program_pool_full` 本轮未出现。状态/代表裁剪为裁剪事件单独计数，不属停止标签。

## 阶段二：逐边局部必要条件诊断

对阶段一后仍零候选的 8 条边，按冻结问题表执行 10 个局部问题（C06 两边各含首弦+末弦两问题）。排除判据仅量化弦长下限（0.099999 m 保守值）与管线实际施加的方向锥（边界钳制端弦 0.95°）；规定倒圆切向不作方向条件。生产采样质量预检的片数/转角只作假设排序，未进入任何排除叶。

| 序 | 边 / span（0 基） | 问题 | 结果 | 关键计数 |
| ---: | --- | --- | --- | --- |
| 1 | `sumo::-10564_6_0` / 0 | 首弦（起始边界钳制） | 未决 | 32,000,000 盒、深度 48、pending 23 |
| 2 | `sumo:--30452_0` / 2 | 末弦（末端边界钳制） | 未决 | 32,000,000 盒、深度 48、pending 33 |
| 3 | `sumo::-20112_0_0` / 0 | 首弦（起始边界钳制） | 局部见证 | 3 盒、深度 2；见证弦长 0.1551 m |
| 4 | `sumo::-10564_3_0` / 1 | 末弦（末端边界钳制） | 未决 | 32,000,000 盒、深度 48、pending 33 |
| 5 | `sumo:--30306#2_0` / 0 | 首弦（起始边界钳制） | **全参数域排除** | 81 盒、深度 9、41 排除叶（最短弦 4 / 起始方向 37），复核器通过 |
| 6 | `sumo::-8806_4_0` / 1 | 末弦（无方向条件） | 局部见证 | 1 盒；见证弦长 0.3255 m |
| 7 | `sumo::-10004_3_0` / 1 | 首弦（无方向条件） | 局部见证 | 2 盒；见证弦长 0.2424 m |
| 7 | `sumo::-10004_3_0` / 1 | 末弦（无方向条件） | 未决 | 32,000,000 盒、深度 48、pending 33 |
| 8 | `sumo::-10004_8_0` / 1 | 首弦（无方向条件） | 局部见证 | 3 盒；见证弦长 0.2061 m |
| 8 | `sumo::-10004_8_0` / 1 | 末弦（无方向条件） | 未决 | 32,000,000 盒、深度 48、pending 33 |

证明范围（正交记录，见 `diagnosis.json`）：

- `sumo:--30306#2_0` 的首弦问题覆盖全部首弦与单弦情形且全参数域排除，**单独**即否定该固定 cubic 的完整发射；这是本轮唯一的已证固定冲突。它不构成该边其他 span、整边、分量或通用问题的无解证明，也不禁止改控制柄/合并分段的其他曲线设计。
- 4 个局部见证只证明对应局部问题存在解。见证参数注入后的整边搜索全部 `retained_family_exhausted`（零候选）；见证弦的实际尝试/使用证据（是否被检查、是否合格、是否进入任何返回路径/所选路径）记录于注入搜索的 `witness_injection_records`，注入**不强制**使用见证弦（`graph::paths_on` 枚举全部 `i<j` 节点对）。
- 5 个未决均为诊断限额（32,000,000 参数盒、累计深度 48）到达；其中大量深度-48 叶（约 1.6×10^7）表明保守排除判据与见证筛选之间存在无法用本轮必要条件判定的区域。未决不产生排除，也不产生可行证明。
- 与旧搜索的关系分两层记录：参数族成员关系按冻结算法与参数直接计算——四个见证参数（二分中点 0.5 / 0.375）恰为 `{i/512}` 网格节点（`calibrated_grid_node: true`）；实际访问证据为 `not_recorded`（既有 `history` 仅有摘要，不能由成员关系推断实际访问）。成员关系为真也不构成整边可行：注入后整边搜索仍族耗尽。

## 阶段三：组合验收与完整回归

四个分量的联合赋值均未执行：候选组合前置条件不满足（分量内仍有缺程序边），按计划保持 `missing_real_programs`，不记组合穷尽或编译拒绝。`sumo::-10034_0_0` 的 8 份既有候选保留在合并池（池仍为 7 边 × 8 份，无新增）；C08 的 8×8×8=512 组合上限口径（256 上限 + `assignment_budget_stopped`/未访问计数）本轮不适用（未进入赋值）。

完整回归（每轮全网捕获，与 #1105 基线逐行比较）：

- 16,265 条原成功边的全部 f64 程序与规范 f32 点、全部拓扑/控制信息、8,230 条失败边原诊断均与基线一致；
- 84 条目标路径：26 accepted、3 cross_edge_direction_rejected、55 target_check_not_completed——与 #1111 / #1113 的变体口径一致（#1115 的曲线调整候选为另一变体口径，盘点 28/84 含之；本轮回池不含曲线调整候选）；
- baseline 变体的 53 分量与 84 路径结果与 #1107 `variants.combined` 逐值一致（断言通过）；combined 变体无接受回退；
- 接受分量的逐点、停止线所在边、机动门路径、转换下标、信号组绑定核对与 LFCA 后发射检查照常执行；研究输出未注入局部前缀或占位几何。

## 成本与确定性

四个独立 release 进程按计划 §5 交替顺序执行（搜索族 old→grid、grid→old、grid→old、old→grid；诊断 diagnosis-first、baseline-first、baseline-first、diagnosis-first）：

| 轮次 | 三组搜索合计（秒） | 诊断含注入搜索（秒） | 分量/路径验证（秒） |
| --- | ---: | ---: | ---: |
| 00 | 3.06 | 47.23 | 8.10 |
| 01 | 3.32 | 46.37 | 10.18 |
| 02 | 2.98 | 46.10 | 8.25 |
| 03 | 3.12 | 47.09 | 8.15 |

诊断耗时集中在五个未决问题（各约 8.8–9.6 秒，均为 32,000,000 参数盒/累计深度 48 限额到达）；四个局部见证、四次见证参数注入搜索合计均小于 0.25 秒。

四次运行的全部 **61 份非计时输出**（每轮 62 份减 `timing.json`）逐字节相等：后三轮分别与首轮以 Windows `fc.exe /b` 完整比较，共 **183 组全部通过**，文件清单与比较结果见本机 `evidence-manifest.json`。内容相等由逐字节比较确定；PowerShell `Get-FileHash` 的 SHA-256 仅作互操作核验。逐问题耗时与注入搜索耗时只写 `timing.json`（首轮 run-00/run-01 曾因诊断输出携带计时字段中止，产物归档为 `run-00-aborted-timing-fields` / `run-01-aborted-partial`，修正提交 `34c0ca87` 后四轮全部重跑；中止产物不参与比较）。

未施加主机空闲门槛；耗时为同机固定输入的描述统计，不推导普遍性能倍数、统计显著性或生产吞吐；工作计数不是实际堆峰值。

## 复现

见 [plan.md §8](plan.md)：冻结提交构建 `laneflow-lust-converter` release lib test（Rust 1.99.1），正式入口

`output::emit::s1::recovery_coverage::dependency_recovery::calibration::coverage::pinned_lust_coverage_diagnosis_experiment`

传入 `--exact --ignored --nocapture --test-threads=1`；环境变量 `LUST_SOURCE_DIR`、`DIRECT_SUCCESSOR_BASELINE_DIR`、`COVERAGE_OUTPUT_DIR`（每次必须为不存在的新目录）、`COVERAGE_ORDER`（`old-grid` / `grid-old`）、`COVERAGE_DIAGNOSIS_ORDER`（`diagnosis-first` / `baseline-first`）。

首轮 run-00/run-01 曾因诊断输出携带计时字段中止（产物归档于本机证据目录 `run-00-aborted-timing-fields` / `run-01-aborted-partial`）；计时移入 `timing.json` 后（`34c0ca87`）重新执行全部四轮。该修正只涉及输出归类，不改变任何搜索、诊断或验收结果。

## 结论边界

本轮不改变 #1113 决策材料的方向，但补充了逐边证据：8 条边的固定曲线覆盖缺口**不是**旧搜索口径不齐造成的——校准网格（含 #1111 两项校准）首次覆盖这些边仍为零候选；其中 `sumo:--30306#2_0` 已有首弦必要条件的全参数域排除。`research/issue-1113-closure-feasibility/g1-proposal.md` 的「下一设计范围」对这些边同样适用：若继续恢复，需单独研究失败边的曲线/分段设计变量（内部 Bezier 控制柄、合并内部采样分段），并保持端点、钳制方向、身份、关联路径与控制锚点不变。零恢复、族耗尽、预算停止与未决分别保存，均不作数学无解结论。生产 S1 的 G1 保持 Proposed；完成本切片不关闭 #790。
