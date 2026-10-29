# C17 双边恢复：新变量族执行与两程序齐备验收（B 路）

**结论：有限新族探索完成，C17 两程序不齐备——如实记不齐备（零恢复为有效结论）。** 新族（S1 内部结点切向扰动 + S2 pair-L/pair-R 逐组独立柄因子）对两条失败边**零候选**：`sumo::-10564_3_0` 5 配置、`sumo::-10564_6_0` 542 配置全部访问、**无预算停止**；负对照（`-10564_6_0` S1-keep 25 + S2-pair-R 256 共 281 配置）**零合法程序**、最早拒绝阶段如实记录，与短首段硬障碍证书（保守下界 2.7804511 mm）一致。C17 保持 `missing_real_programs`，联合赋值 0 次（未进入），完整路径 `target_check_not_completed`。基线重新验证与 E0 重锚（92 池行代际）逐值一致（**28/84** 无回退）。**这是测试研究的有限探索记录，不是生产转换接入或生产 G1 接受**；本族未通过不作无解证明。

本研究对应 #1137（六路协调任务书 v2 的 B 路，设计前置闸口全程执行）。方案经维护者三轮审阅冻结于 `432fc097`（[design.md](design.md) v3 + [short-span-certificate.json](short-span-certificate.json) + 复核/参数化负验证测试），实现冻结于 `3849d1ec`（含 E 驱动 `0bbcfa7a`、实测证据修复 `57eeb02f`、K1–K4 校准 `9d0477dc`）。生产采样、compiler、阈值、原成功几何、拓扑与控制锚点均未修改；工作预算零提高。生产 S1 的 G1 保持 Proposed（[决策材料](g1-proposal.md)）。

## 冻结与样本

C17 单条完整路径 `sumo:-10564:--32172#4_0-to--31194_0::-10564_3_0.:-10564_6_0`，两程序齐备才可验收（[edges.json](edges.json)）：

| 边 | s | 点上限 96s+1 | 首末钳制（冻结原 f64 位） | 既有状态 |
| --- | ---: | ---: | --- | --- |
| `sumo::-10564_3_0` | 2 | 193 | 末端 finish boundary clamp（起始 interior CR） | #1131 既有 1 候选（几何 80，随 E0 基线入池） |
| `sumo::-10564_6_0` | 3 | 289 | 起始 start boundary clamp（末端 interior CR） | #1131 旧族 148 配置零候选 |

共同基线（E0 冻结交付 `research-evidence-1140-shared-baseline`，`SIX_STREAMS_BASELINE_DIR` + `shared_baseline::shared_pools()` 三资产 SHA 钉版/顺序/份数断言读取）：11 边 92 份；结果对照锚 = `evidence/run-03/results.json#variants.new`（**92 池行**代际，`comparison_only`，三元组代码内钉版）。

**生成物外置**：搜索/运行生成物压缩挂 [Research Evidence: #1137](https://github.com/illusion-tech/laneflow/releases/tag/research-evidence-1137-c17-bilateral) 预发布（`c17-bilateral-evidence.zip`，424,659 字节，SHA-256 `88a6a9640d2bd4833132577f10aaea11bb0b1e51d68af5394e0c3a3f4bed9b8c`，内含候选冻结产物、四轮 `c17-searches`/`c17-results`/对照列/冻结输入、189 组比较记录与 `MANIFEST.json` 来源链（逐文件 SHA-256 / 冻结 HEAD `fdca716e` / 基线代际）；全量回归快照与接受分量 LFCA 本地留存，与 #1105 基线逐字节断言，未入包）；仓库只留计划、入口与本摘要。

## 逐配置执行记录（新族 547 列明 + 旧族 148 对照列）

每配置内：回折拒绝 → 双向来源拟合筛选（64 区间/弦，不劣于原参考两向估计）→ 划分族外层（等 t 先、512 步累计切向角后）n=2..96 内层、每族首程序 → sink（0.1 m/1.95° 量化预检）/存在钳制端 0.95° 端弦 / 0.01 m 逐区间。负对照照常走管线（证书预判不提前截断）：

| 边 | 族 | 列明 | 来源拟合拒绝 | 两族无完整程序 | 旧族重叠跳过 | 成功 | 保留候选 |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| `-10564_3_0` | S1-keep | 5 | 4 | 1 | 0 | 0 | 0 |
| `-10564_6_0` | S1-pair-L（探索） | 5 | 5 | 0 | 0 | 0 | 0 |
| `-10564_6_0` | S2-pair-L（探索） | 256 | 240 | 0 | 16 | 0 | 0 |
| `-10564_6_0` | S1-keep（负对照） | 25 | 21 | 4 | 0 | 0 | 0 |
| `-10564_6_0` | S2-pair-R（负对照） | 256 | 176 | 80 | 0 | 0 | 0 |

- **负对照 vs 证书**：281 负对照配置零合法程序（逐配置预期 vs 实测一致）；最早拒绝阶段如实记录（以拟合拒绝为主、84 配置到达划分层仍无完整程序）；短首段证书的 sink 层障碍在到达划分层的配置上与 K4 R1 交叉一致（因子≠1 的配置先被双向拟合的弦近似误差项拒绝，为既有口径保守方向行为）。
- 尝试数与停止原因（无预算停止，有限族完成）：`-10564_3_0` 划分 190 / 采样 7,025 / 拟合 202,752；`-10564_6_0` 划分 15,960 / 采样 201,069 / 拟合 27,371,520（< 32M 顶）。成功 n：无。
- 旧族 148 配置对照列（`c17-old-family-column.json`）：仅对照，不作候选点位来源。

## 长度与拟合（design.md §4 清单；未测项如实）

| 边 | `@length` | shape 折线 | 原参考弦/多边形估计（4096） | 原参考双向拟合（正/反） | 新参考 | 规范折线→来源 | 规范 f32 弦长累计 | compiler `length_mm` |
| --- | ---: | ---: | ---: | ---: | --- | --- | --- | --- |
| `sumo::-10564_3_0` | 4.600000 | 4.603308 | 4.616505 / 4.616505 | 0.084968 / 0.085066 | 未测（零候选） | 未测 | 未测 | 未测 |
| `sumo::-10564_6_0` | 4.990000 | 4.991606 | 5.008796 / 5.008796 | 0.094903 / 0.094986 | 未测（零候选） | 未测 | 未测 | 未测 |

零候选 ⇒ 无新规范长度；失败边新规范长度不声称保持不变；禁止比例缩放或重写源长度。

## C17 双边联合验收结论

- **不齐备**：两边缘均零候选 → 分量保持 `missing_real_programs`、联合赋值 0 次（`total_assignments` 空、`unvisited_assignments: null`、接缝全角记录 0 条——未进入枚举）；单条完整路径不接受（基线/新增两组一致）。
- **两口径收益**（章程 §1 分列）：新增候选 0、新接受分量 0；**完整关联路径收益 0 / 1 ↔ 原目标收益 0 / 1**（全网 84 分母不适用本行）；路径接受无变化（28/84 → 28/84）。
- 既有候选（#1131 几何 80）随基线入池于 `-10564_3_0` 池索引 0（固定索引口径满足）；本轮新增分列 0。

## 完整回归与结果对照

每轮全网捕获与 #1105 基线逐行比较：16,265 条成功边 f64 程序与规范 f32 点、全部拓扑/控制信息、8,230 条失败诊断均一致（三文件逐字节断言）；53 分量、183 关联路径、84 目标齐备；接受分量逐点、停止线所在边、机动门路径、转换下标、信号组绑定核对与 LFCA 后发射检查照常执行（本轮无新接受分量；基线 28 接受分量制品照常生成核对）。**结果对照（仅对照）**：基线变体的 53 分量结果、84 路径结果与 `path_counts` 与重锚 `#variants.new` **逐值一致**（含 `unvisited_assignments` 字段形态；28/84 = 28+3+53）。

## 成本与确定性

四个独立 release 进程按 plan §7 ABBA（baseline-new、new-baseline、new-baseline、baseline-new；候选冻结步先行产出 `c17-searches.json` 0.15 秒）：

| 轮次 | 顺序 | 整进程墙钟（秒） | 新族搜索（秒） | 捕获解析/全网转换（秒） | 基线/新增验证（秒） |
| ---: | --- | ---: | ---: | ---: | ---: |
| 00 | baseline-new | 52.56 | 0.122 | 0.51 / 41.86 | 4.57 / 4.60 |
| 01 | new-baseline | 52.96 | 0.123 | 0.52 / 42.38 | 4.53 / 4.59 |
| 02 | new-baseline | 52.94 | 0.124 | 0.52 / 42.06 | 4.43 / 4.97 |
| 03 | baseline-new | 52.35 | 0.131 | 0.51 / 41.09 | 4.81 / 4.88 |

四轮全部非计时输出确定性：每轮 63 份输出（62 份非计时 + `c17-timing.json`），后三轮与首轮 `fc.exe /b` 全量比较共 **189 组全部逐字节相等**；**结果 JSON 禁计时校验 clean**（按键名精确判定；`durationMs` 为信号相位时长域数据，非性能计时，注记不改）。旧族对照列与新族搜索合计计时（对照列未单独计时，如实注明）。文件清单与 SHA-256（仅互操作）见外置 `evidence-manifest.json`；环境见 `environment.json`；耗时见 `measurements.json`（AMD Ryzen 9 9955HX / Windows 11 build 29695 / rustc 1.99.1）。未施加主机空闲门槛；耗时为描述统计，不推导生产吞吐；工作计数不是堆峰值。

**过程记录（如实，均不改研究口径）**：四次中止批次归档 `run-00-aborted-*`（基线包返工前代 MANIFEST → 改指钉版 export-run-a；PowerShell 插值致脚本替换静默未生效 → 改编辑工具；84 目标行源改 issue-1105 `reference()`；`unvisited_assignments` 字段形态补 null 增强——c12 同款）。

## 复现

- 候选冻结入口（先跑，`c17-searches.json` 作 `CURVE_DESIGN_FROZEN_RESULT`）：`output::emit::s1::recovery_coverage::dependency_recovery::c17_bilateral_recovery::fixed_c17_bilateral_design`
- 正式实验入口（四轮 ABBA，`--exact --ignored --nocapture --test-threads=1`）：`…::pinned_lust_c17_bilateral_experiment`
- 校准（第一闸）：`…::k1_known_candidates_replay_with_frozen_inputs`、`…::k2_old_family_replay_matches_frozen_records`（k2-old-family-frozen.json 等值锚）、`…::k3_tangent_rotation_boundary_self_checks`、`…::k4_synthetic_positive_and_negative_cases`；证书 `…::short_first_span_obstruction_certificate_recheck`、`…::certificate_tampering_is_rejected`
- 常规单测：`…::c17_cohort_and_capacities_match_the_frozen_sample`、`…::c17_family_config_counts_match_the_frozen_plan`
- 环境变量：`LUST_SOURCE_DIR`、`DIRECT_SUCCESSOR_BASELINE_DIR`、`SIX_STREAMS_BASELINE_DIR`（E0 钉版基线包解压目录）、`CURVE_DESIGN_OUTPUT_DIR`（每次不存在的新目录）、`CURVE_DESIGN_ORDER`、`CURVE_DESIGN_FROZEN_RESULT`。本机脚本 `run-c17-abba.ps1` / `collect-c17-evidence.ps1`（不入库）。

## 结论边界与差异记录

- **有限族未通过不作无解证明**：S1/S2 新族 547 列明配置零候选，只表示本族未通过；短首段证书只覆盖列明配置（keep 首 cubic 与 pair-R 首组），不外推整条边或分量。不预设必须恢复。
- 新变量覆盖事实：切向扰动与逐组独立柄因子均未跨越既有阻点（拟合弦近似误差项对参数化敏感为既有保守方向行为，未改共享代码）；旧族对照列不作候选来源。
- 差异记录：R1 曾误钉 sink 标签（改回冻结「不钉特定标签」口径）；合成输入 s 由倒圆构造决定（L 形→s=3）；serde_json 1 ULP 跨片事实（双侧同源解析语义有效，未装位级硬门）；`unvisited_assignments` 字段形态按锚增强后比较；E 驱动 `attempt_reject_detail.measured` 实测证据通道由 `57eeb02f` 落地（R2/R3 直调证据齐）。
- 完成本切片不关闭 #790；下一步依据见[决策材料](g1-proposal.md)，生产 S1 的 G1 保持 Proposed。
