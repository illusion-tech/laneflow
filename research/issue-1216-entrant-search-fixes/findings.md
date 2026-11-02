# Issue #1216 差异记录：S4 潜在进入者距离裁剪与循环路线出现项查询

基线：main@45263887（工作树 `research/1216-entrant-search-fixes`，未 commit）。方案：
`plan.md`（已批准）。本记录覆盖 §1.2 / §2.2 / §3 全部实施项与红线核对。

## 1. 缺陷一（P1-1）：距离裁剪 → 逐条目近端距游走

### 250m 常量删除

- `MAX_STOP_REACH_MM = 250_000`（原 conflict_tick.rs 7046）**已删除**，连同
  `while depth_mm < MAX_STOP_REACH_MM` 的单一全局 `depth_mm` 累加（原 7051、
  7062-7068）。仓库内无残留引用（rg 验证）。
- 替换为 `entrant_stop_reach_mm(traffic, delta_ms)`：`reach = max_{profile ∈ 修订}
  stop_distance_millimetres(MAX_SPEED_MM_S, 固定步长, comfort_decel·1000, min_gap)`
  ——受检 profile 与步长的最坏**具名四项**停止距离导出，不设任何新常量。
  - 速度项取 `laneflow_static_contract::MAX_SPEED_MM_S`（合同受检速度上界，
    与审计"100m/s × 0.5m/s² → 制动距离 10km 量级"同口径）。
  - `comfort_decel == 0` 的 profile 令 `d_stop = u64::MAX`（`stop_distance_millimetres`
    除零保护）→ 不可证输入自动导出 `reach = u64::MAX`：枚举整可达分支，有限图
    visited 集终止。修订内无 profile 或 profile 数据缺失同样取 `u64::MAX`
    fail-closed（与 #1199"证明前提不可得即 no-grant"同向）。

### depth 的精确定义（近端距口径）

- frontier 条目 `(edge, depth)`；`depth` = **该边近端（较近端点）到区间两端点
  （T 边起点/终点）的最短分支路线距离（毫米）**。条目互不共享；实现为每边
  两端点标签 `(起点端深, 终点端深)` 的双向 min 松弛（非负权有限图收敛到精确
  最短路），`depth = min(两端)`；visited 标签表在有限图上终止。
- 剪枝只用近端距：`depth ≤ reach` 才入列（边上存在可达点）；证明侧仍用既有
  逐进入者实际距 `distance_to_occurrence_progress`（含 progress，不放宽）。
  即**剪枝按近端距（不漏）、证明按实际距（不松）**，两口径分工不变式。

### 前驱邻居逐字公式反例与修正（lead 裁定口径，A2 钉版）

- plan §1.2 公式 `depth(n) = depth(c) + len(c)` 只对**后继**邻居成立（需跨过
  当前边全长）；**前驱**邻居连接在当前边近端一侧，逐字套用会多计整条共享边。
- 反例（测试 `a2_long_shared_edge_prune_keeps_predecessor_near_end`）：夹具
  `T → e2(10km) ← e4(8m) ← x4`，车在 x4 末端、实际距 8m（< d_stop 26.75m）。
  逐字公式 depth(x4) ≈ 5m + 10km + 8m ≫ 导出 reach（1.13km，car 单 profile）→
  误剪 → 漏证放行；近端距口径 depth(x4) = 8m ≤ reach → 必入列 → **修后拦**
  （安全断言 40 拍全程不得放行）。该例同时钉住口径本身，防止回退照抄公式。
- I-B1 不漏方向论证（一句话）：任何不可证车满足 `实际距 < d_stop ≤ reach`，
  而实际距 ≥ 车位到区间两端点的最短分支距离 ≥ 所在边近端距 = depth，故
  `depth ≤ reach` 恒成立、必入列——枚举域 ⊇ 一切证不出来的车。

### 枚举结构

- `entrant_neighborhood(traffic, interval_edge, reach_mm)` 返回
  `first_ring`（T 自身 + 直接邻居，区间边界目标候选集，语义与旧码一致）与
  `entries: Vec<(edge, depth)>`（发现序，确定性）。T 集中在区间两端点
  （depth 0）；近端距超 reach 的条目不入列、不再扩张（其可达近端必经更远
  端点，剪枝不漏——由上述 I-B1 论证覆盖）。

## 2. 缺陷二（P1-2）：循环路线出现项首现误选

### 注册期出现项表（tables.rs `compile_route`）

- `CompiledRoute.edge_occurrence_index: Box<[(LaneEdgeOrdinal, u32)]>`：`(边序数,
  路线下标)` 全对按 (边 raw, 下标) 升序排序，每边出现项成连续段；注册期一次、
  `try_route_vec` fallible 分配 + `sort_unstable`（无额外分配）+ `into_boxed_slice`。
- 查询 `first_edge_occurrence_at_or_after(edge, cursor)`：二分到该边连续段后
  `partition_point(|idx| idx < cursor)` 取**首次不早于游标**的出现项；全部早于
  游标=真在身后 → `None`；边不在路线（不在表）同 `None`，维持既有 fail-closed
  归口（state/compiled 分支 `return Some(unprovable)`；目标缺失=非潜在进入者跳过）。
- conflict_tick.rs 两处 `route_edges.iter().position(...)`（原 7080 区间目标、
  7087 边界目标）**同表同口径替换**：`boundary_targets(route, cursor)`，cursor=
  该进入者当前 `route_edge_index`。skip 分支（原 7257-7262）语义随之收紧为
  "全部目标真在身后才跳过"，与注释对齐。T 目标（interval_edge）与 first_ring
  边界目标共用同一查询（issue 要求同修 ✓）。

### 存储记账

- `retained_logical_bytes` 穷尽解构已加 `edge_occurrence_index`
  （`state::slice_bytes`，8 字节/路线边）；新增字段触发解构编译失败→强制分类
  （rust-code-style §6）。位移：`three_a_rear_boundary_is_exact_to_one_micrometre`
  的字节等式两侧同增（表为空 Box）——**无钉值位移**，无需重钉。
- 分配账：走既有 `try_route_vec`/`try_reserve_route_exact` failible 约定
  （`RouteError::AllocationFailed`），失败原子语义不变。`route_edge_occurrence_capacity`
  计数口径不变（表是逐路线边的派生存量，不占注册计数预算）。
- 连带：`CompiledRoute` 字面量构造点补 `edge_occurrence_index: Box::default(),`
  共 2 处（tables.rs 测试、tick.rs 测试）——结构体新增字段的强制同步，非范围外改动。

## 3. 反例测试 A–F（tables.rs `compile_route_tests::s3_lane_change::p4_entrant_search_fixes`）

| 例 | 测试 | 覆盖 | 结果 |
|---|---|---|---|
| A 长边 | `a_long_branch_does_not_drop_eight_metre_enterer` | 300m 干扰分支 + 8m 进入者（depth(xa)=8m 钉版）；修后拦 + 清窗后授予恢复 | 绿 |
| A2 前驱过计 | `a2_long_shared_edge_prune_keeps_predecessor_near_end` | 10km 长共享边逐字公式反例（公式值 > reach 断言 + depth=8m 断言）；修后必须拦 | 绿 |
| B 短边链 | `b_short_edge_chain_fully_enumerated_and_five_metre_enterer_blocks` | 36×6m 短边链（<250m）全量入列 + 5m 进入者拦 + 恢复 | 绿 |
| C 多分支 | `c_multi_branch_enterers_block_independently` | 300m 干扰 + 20m/10m 两目标分支独立测距（depth 20m/10m 钉版）、分世界各自拦 + 恢复 | 绿 |
| D 重复边 | `d_occurrence_index_avoids_behind_targets_on_loop_routes` | 循环路线 [A0,I1,B0_up,S00,B0_down,I0,C0,R0,A0,I1,B0_up,S00]（B0_up@2/10）与 T01 双现（@3/11）：游标定位 + 前方目标有限/身后 `None` 双形（T 目标与边界目标） | 绿 |
| E 异 profile | `e_profile_specific_stopping_proof_decides_grant` | 弱制动（comfort 0.5，d_stop≈198.5m）车驶入自身窗（含 100m 点位）按自身参数拦；强制动（comfort 40，d_stop≈7.7m）车 40m 按自身可证→授予；reach 导出随弱 profile 扩展 | 绿 |
| F 活性 | `f_grant_recovers_after_enterer_clears_and_interval_releases` | A 形世界：清窗后授予恢复（entered > cleared）+ 第一换道方释放区间后第二换道方获得授予 | 绿 |

- 安全（应拒时拒）由 A/A2/B/C/E 拦挡断言覆盖；活性（条件解除后授予恢复）由
  A/B/C 的 `entered > cleared` 尾态与 F 的双阶段覆盖。
- 夹具：`EntrantFixture::build(e2_meters, profiles)`——s4 stream-bearing v7 形制
  （合成 + road-editing 冲突声明 + policy），增 300m 干扰分支、四目标分支
  （8/20/10/5m 近路段）、36×6m 短边链、弱/强制动 profile；几何按全部
  lane-graph/maneuver 接缝连续落位（Join 容差内）。A2 用 `e2=10km`（合法上限）
  + car 单 profile 变体。
- E2E 说明：spawn 停妥约束要求起点在自身 `d_stop` 之外（如 13.9m/s 标准车
  ≥26.75m），故进入者从 28m 驶入窗内（覆盖 8m 点位），窗内逐拍断言拦挡；
  换道方从 10m 处（5m/s，d_stop 6.3m ≤ 距停止线）驶入，~10 拍跨入 T。A 的
  修前漏判别窗=进入者仍在进路段（旧码第二环入列即耗尽预算、第三环不展开）时
  换道方的进入跃迁拍。
- R 群复验随全量套件跑（R1a/R2/R3 绿；R1b `#[ignore]` 状态未动，归 #1218）。

## 4. 验证命令与结果（Rust 1.99.1；未装 cargo-nextest，CI nextest 以 cargo test 等价执行）

- `cargo check -p laneflow-runtime --locked --features placement-fixtures` → 绿。
- `cargo check -p laneflow-runtime --all-targets --locked --features placement-fixtures` → 绿（零 warning）。
- `cargo clippy -p laneflow-runtime --all-targets --locked --features placement-fixtures -- -D warnings` → 绿。
- `cargo test -p laneflow-runtime --locked --features placement-fixtures` → 全绿
  （含新增 7 例 A–F/A2 与 R 群回归；exit 0）。
- 变更未波及 `laneflow-compiler`/workspace 级（仅 `laneflow-runtime` 两文件 + 测试），
  按任务口径不补 workspace 级双跑。

## 5. 红线核对

- 合同 §5.2 判据语义不变：d_stop 具名判据、逐进入者自身 profile、链式证明
  形态均未动——仅改枚举覆盖（近端距游走 + 导出 reach）与目标定位（游标表查）。
- R1a/R1b/R2/R3 断言未弱化；R1b `#[ignore]` 未动。
- 新增分配遵守 fallible allocation 约定；出现项表按 §6 记账；无钉值位移。
- 注释只写契约（rust-code-style §5）：口径不变量、失败语义、I-B1 论证、
  治理锚点（#1216/#1199）；无变更日志式注释、无无锚 TODO。
- `MAX_STOP_REACH_MM` 删除，未以放大常量替代。

## 6. 遗留风险

1. **reach 速度项假设**：`MAX_SPEED_MM_S` 作为运行时车速上界用于 reach 导出
   （合同受检输入上界）。若存在车辆速度超过 `MAX_SPEED_MM_S` 的注入路径，
   reach 需相应重审（当前 spawn/update 均经受检输入面，未发现越界路径）。
2. **A2 尾态**：共享边 10km 时"进入者驶过区间"需数千拍，A2 只钉安全面
   （修后必须拦，40 拍）；活性尾态由 A/B/C/F 在常规夹具覆盖。
3. **性能**：近端距游走为 SPFA 型 min 松弛，规模与旧 O(V²) `contains` 游走同
   量级；`comfort_decel == 0` 分支枚举整可达分支的开销归 #1219（性能面）。
4. **E2E 判别窗**：A 的修前漏判别依赖进入者仍在进路段时换道方处于进入跃迁拍
   （夹具拍数余量 14 拍 vs 10 拍）；若未来调整夹具速度/距离需保持该余量。
5. tick.rs +1 行为 `CompiledRoute` 字面量补字段（见 §2 存储记账），属结构体
   连带同步，不构成独立行为面。
