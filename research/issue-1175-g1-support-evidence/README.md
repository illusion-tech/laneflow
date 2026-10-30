# G1 支撑证据：候选终验、跨边失败范围与生产资源测量（#1175）

**结论：三块支撑证据完成——8 份 pending 候选已终验四项 8/8（依赖清单逐路径列阻断边并示证）、三例跨边范围判断均源几何边界固有（只补证不重判）、资源测量方案与首轮 12 次数据取讫（release 形态 convert ~43.7s、峰值 ~310MB）；产出按 [`g1-decision-chapter.md`](g1-decision-chapter.md) 四节格式交付（G1 决策包章节）。生产 S1 的 G1 保持 Proposed。** #1175 按 Issue 范围执行（第二批 E-2 终局支撑）：不追新恢复、不改生产、测量仅取证不调优。

## 三块执行记录

### 块① 8 份 pending 候选终验分列（`calibration/g1_final_validation.rs::g1_candidates_final_validation`）

8 份候选全部位于 `sumo:--30266_0`（C24，#1154 复跑终值入池 pending）。终验分列（lead 批准预案）：

- **已终验四项 8/8**：同表示双向拟合复核（候选折线 0.769/0.822 ≤ 原参考发射折线 0.773/0.827，修复后全 span 参考 3,073 点基准）；程序位核对（bits→程序→bits 位级回环）；edgeEnd 声明绑定（`sumo:stop:--30266_0` location=edgeEnd）；机动门绑定（`sumo:-17348:--30266_0-to---32230#6_0::-17348_6_0`、`sumo:-17348:--30266_0-to--32230#7_1::-17348_7_0` 两门→机动路径→边成员）。
- **依赖清单**（编译/LFCA/编译后长度/规范端点/联合锚点）：4 条机动路径各有且仅有 1 条其他缺边阻断——`sumo::-17348_6_0` / `sumo::-17348_7_0` / `sumo::-29244_4_0`（另含首弦全参数域排除）/ `sumo::-29244_1_0`；示证记录 `target_check_not_completed`（emission_prerequisite）+ `missing_real_programs` 7 边全列（`sumo::-17348_0_0`、`sumo::-17348_5_0`、`sumo::-17348_8_0`、`sumo::-17348_6_0`、`sumo::-17348_7_0`、`sumo::-29244_1_0`、`sumo::-29244_4_0`）。
- **检查定义修正入册**：「规范端点/程序位」分属两域——程序位可验（位级回环）；canonical f32 规范端点属发射/编译域，随联合编译终验执行、归入依赖清单。
- 单/双横线防混淆：`sumo:-30266_0`（单横线）为成功边，与缺边 `sumo:--30266_0` 不同 ID。

### 块② 三例跨边范围判断（`calibration/g1_final_validation.rs::g1_cross_edge_scope_records`）

只补证不重判。三例 `cross_edge_direction_rejected` 编译诊断（`LF-COMP-SPATIAL-GEOMETRY DirectionDiscontinuity`）逐例记录：边界两侧键与观测边 ID（规范键映射 `x` 前缀+`.`→`#`、`int:`→`sumo::`）、冻结检查值位级解码（语义：lhs=dot²、rhs=cos²θmax·‖L‖²·‖R‖²，ADR 0022 固定逐运算图）与**发射边界方向重推**（P2-1 修正：编译器比较编译后几何首/末弦 `hir/spatial.rs:1219–1235`，源切向非判据）。

| 目标 | 边界（左→右） | 编译弦角（程序弦互证） | 范围判断（P2-1 重推后） |
| --- | --- | ---: | --- |
| `sumo:-19726:--32668#0_0-to---32068#1_0::-19726_1_0.:-19726_6_0` | `sumo:--32668#0_0` → `sumo::-19726_1_0` | 2.739°（逐值相等） | 属固定曲线 |
| `sumo:-25490:-32668#0_0-to--31516_0::-25490_3_0.:-25490_6_0` | `sumo:-32668#0_0` → `sumo::-25490_3_0` | 2.641°（逐值相等） | 属固定曲线 |
| `sumo:-8018:--32568_1-to--32770#1_1::-8018_0_1` | `sumo::-8018_0_1` → `sumo:-32770#1_1` | 2.385°（逐值相等） | 属固定曲线 |

**范围结论（P2-1 重推后终值）**：三例两侧均为原成功固定曲线（编译后边界弦对为固定事实，编译弦角与程序点位弦角逐值互证）——排除**属固定曲线对的属性**，恢复侧不可修复、候选空间不存在。原「一般性质（源几何固有、任何发射均拒）」表述撤回（源切向非编译判据；源边界弦向转角 8.551°/8.420°/7.891° 仅作背景记录）。编译判定本身不重开。

### 块③ 生产资源测量方案 + 首轮数据（`calibration/g1_resource_measurement.rs::measure_production_key_paths`）

三阶段口径（本切片定义）：`parse` = pinned LuST 源校验读入后的网络/信号灯/车型解析与画像转换；`convert` = 拓扑转换 + 全网曲线发射（观察捕获）；`replay` = **真实回放核验**（第二次发射 + 两次程序输出逐程序位级比对；P2-3 修复后语义，复测值待独占窗口——原 0.02–0.08s 空转口径作废）。单进程单阶段便于内存归因；双形态（debug/release 测试二进制）× 每形态 2 轮 × 3 阶段（独占窗口执行，统筹排程）。

| 形态 | 阶段 | r1 | r2 | 峰值工作集 r1/r2 |
| --- | --- | ---: | ---: | --- |
| debug | parse | 1.637s | 1.570s | 81.7 / 83.4 MB |
| debug | convert | 128.289s | 122.937s | 314.3 / 313.2 MB |
| debug | replay | 121.363s | 122.114s | 428.4 / 428.6 MB |
| release | parse | 0.333s | 0.338s | 76.5 / 77.7 MB |
| release | convert | 43.390s | 44.042s | 310.0 / 310.7 MB |
| release | replay | 42.039s | 42.288s | 427.0 / 428.2 MB |

（`replay` 行为 P2-3 修复后**真实回放核验**口径复测终值：第二次发射 + 两次程序输出逐程序位级比对；16,265 程序位级全等断言通过；峰值 ~428MB = 双观测留存所致。）

测量纪律照实：固定输入（pinned LuST `c4bd5bd3`）、同机、**无空闲门槛**、描述性限定（不推导生产吞吐或实时收益）；计时口径对齐 #1113 账目（原始计时入 measurements、条件入 environment）。进程峰值：replay 行 ~428MB（双观测留存）高于 parse/convert 行（~310MB）。release 相对 debug：convert ~2.9x / parse ~4.7x / replay ~2.9x 加速。**Codex P2 评审修复记录**：① 块②范围判断改发射边界方向为据（源切向非编译判据）——结论由「一般性质」改判「属固定曲线」；② 块①终验结论改四项合取导出（任一失败分列记 failed）；③ replay 测量改真实回放核验（二次发射 + 位级比对，非空转；旧空转值作废、表中为新口径复测终值——复测中曾发现 release 旧二进制混测（0.02s 空转值），重建 release 后补测两点，如实记录）。测量中一轮缺陷如实记录：首测输出目录未建致末尾 panic（内存采样有效、计时未落盘），一行修复后 12 次全覆盖重测。

## 复现

- 入口（`--ignored --nocapture --test-threads=1`）：`…calibration::g1_final_validation::g1_candidates_final_validation`、`…g1_cross_edge_scope_records`、`…g1_resource_measurement::measure_production_key_paths`
- 环境变量：`LUST_SOURCE_DIR`、`DIRECT_SUCCESSOR_BASELINE_DIR`、`G1_CANDIDATES_INPUT`（P2 修复后 run-00 证据目录）、`G1_EVIDENCE_OUTPUT` / `G1_CROSS_EDGE_OUTPUT` / `G1_MEASURE_PHASE` / `G1_MEASURE_OUTPUT`（各自 fresh）
- 构建：`cargo +1.99.1 test --locked -p laneflow-lust-converter --lib`（Rust 1.99.1）；测量本机脚本驱动双形态二进制

## 结论边界

不追新恢复、不改生产、测量仅取证不调优；8 候选以 pending 入池状态保留（联合编译未证）；三例跨边编译判定不重开（范围判断为补证）；测量为单机首轮描述性数据。生产 S1 的 G1 保持 Proposed（[决策章节](g1-decision-chapter.md)）。
