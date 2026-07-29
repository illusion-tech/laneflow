# LuST / Bevy 个体人口调节契约

**文档状态**: Accepted（#256 G1）<br>
**最后更新**: 2026-07-26<br>
**适用范围**: LuST 真实路网 Bevy native 示例（#257）的 1–一万个体人口调节、
展示计数与确定性抽样；不覆盖走廊 `50..=200` reference policy，也不改写
TOPO/DEMAND workload ID

**关联文档**:

- [`real-road-workloads.md`](real-road-workloads.md)
- [`example-scenarios.md`](example-scenarios.md)
- [`signalized-corridor-population.md`](signalized-corridor-population.md)
- [`core-runtime-performance-baseline.md`](core-runtime-performance-baseline.md)
- [`../adr/0016-scenario-population-and-recycle-lifecycle-authority.md`](../adr/0016-scenario-population-and-recycle-lifecycle-authority.md)
- [#256 G1 冻结判断](https://github.com/illusion-tech/laneflow/issues/256#issuecomment-5082584634)

## 1. 目的与边界

本文冻结 LuST/Bevy 示例层如何把 TOPO plan 的一万 `logical_rank` placement
调节到 `target_N∈[1,10000]`，以及 HUD/证据必须暴露哪些正交计数。共享
`population_rank` 表仍是 provenance 输入，但不是 placement 权威。

权威分层继承 ADR 0016：

| 关注点                                  | 权威                                     |
| --------------------------------------- | ---------------------------------------- |
| vehicle identity、replace/spawn/despawn | Core                                     |
| `target_N`、seed、抽样集合、回流/停补位 | caller-owned LuST example policy（#257） |
| handle↔Entity 映射                      | Adapter                                  |
| `N_presented` 选中与 UI                 | Presentation                             |

本文 **不**：

- 把人口 controller 放进 Core 或新增 batch lifecycle public API；
- 把 `target_N` / seed / 抽样集合写入 Traffic、Spatial 或 ScenarioManifest；
- 改写 `LF-REAL-LUST-TOPO-v1` / `LF-REAL-LUST-DEMAND-v1` 的固定一万语义；
- 宣称一万@100% presentation 的 Product Pass / realtime SLA（见 #215；P10 未认证）。

走廊场景继续以 [`signalized-corridor-population.md`](signalized-corridor-population.md)
的 `50..=200` 为权威；本文只服务 LuST 示例路径（Parent #252 / #257）。

## 2. 正交计数

HUD 与 headless 证据必须同时可读：

| 符号               | 含义                                                     |
| ------------------ | -------------------------------------------------------- |
| `target_N`         | 示例 policy 当前目标 logical population，`[1, 10000]`    |
| `N_individual`     | Core 中仍保留完整 identity 的道路机动车参与单元数        |
| `N_traffic_active` | 参与道路机动车执行域交通权威的个体参与单元数             |
| `N_presented`      | 本 outer frame 被 materialize / 提交展示的个体参与单元数 |

本文是 current `CoreWorld`/LuST 车辆特化，所有无下标计数均绑定
`execution_domain=road_motor_vehicle`；它不定义目标 Traffic Runtime 的非机动车、
步行或轨道交通计数原子。

默认演示目标是 **100% of presentable vehicles**：在无 pending `Completed`、未启用
手动 H3、且 **没有** S1-retained Completed identity（excess slot 已 Completed、已
unbind/移出 proxy、仍占 Core identity 等待 H1）的 outer frame，应有
`N_presented = N_individual`。不得用 `N_presented` 冒充 `N_individual`。

瞬时例外：Bevy presentation 不提交 `Completed` pose，而 `N_individual` 在
despawn/replace 前仍计入该 handle。因此 route completion 之后、下一 lifecycle
boundary 的 replace 生效之前，允许短暂 `N_presented < N_individual`。证据与 HUD
必须同时暴露四计数，不得把该瞬时差解释为已降档。

S1 例外：excess slot 按 §3.2.1 保留 Core identity 但已移出场景后，允许持续
`N_presented < N_individual`（差额等于仍 live 的 S1-retained Completed 数）。
此时不得再要求无条件 `N_presented = N_individual`；HUD 须能区分
`target_N` / `N_individual` / presentable 计数。精确恢复四计数对齐仍靠 S3/H1。

## 3. 调节组合（H1 / H2 / H3）

### 3.1 H1 启动档位

- 启动必须显式提供 `target_N∈[1,10000]` 与 `u64 seed`（零 seed 合法）。
- 禁止隐式默认静默升到一万。
- bootstrap 在第一个 Core step 之前完成；失败则整体不进入 Running。

### 3.2 受约束 H2（运行中改目标）

- 运行中允许改写 `target_N` 意图，但 **精确人口收敛**（`N_individual == target_N`）
  的升档与降档都走 **H1 重建 session**；见第 5 节。Running 内仅允许 S1 停补位
  （不保证 identity 计数收敛）以及目标内的
  `replace_completed_vehicle` 回流。
- 生命周期决策只发生在 **fixed-step lifecycle boundary**（ADR 0016 顺序：pending
  commands → Core step → completions → 下一 boundary 的计划），不得按
  outer-frame 次数偷改。
- **禁止 Running Session 内 live-spawn / live-despawn 扩缩编**。现有 Bevy public
  surface：`LaneFlowSession::core()` 只暴露 `&CoreWorld`；`bind_vehicle_entity`
  只绑定已存在车辆；Adapter lifecycle 仅有 completed-vehicle replace。H1 重建 =
  销毁当前 Session，按第 4/6 节用初始 batch 重建 `CoreWorld` 后再挂 Session（与
  走廊两阶段 bootstrap 同构）。若需要 Running 内 typed spawn/despawn，必须另立
  G1/ADR，不得在本契约下静默扩展 public API。
- 同 `seed` 下重建到更大 `target_N` 时，新入选集合必须是第 4 节全量 shuffle 的更长
  前缀（旧集合为其真前缀）；重建到更小 `target_N` 时取同一 shuffle 的更短前缀。

### 3.2.1 Running 回流（selected slot replace）

对仍属于当前目标入选集合、且未处于 S1 停补位的 logical slot：

1. vehicle 进入 `Completed` 后，按 ADR 0016 在下一 fixed-step lifecycle boundary
   调用 `replace_completed_vehicle`。
2. **replacement 输入冻结为该 slot 的原始 TOPO placement**：同一 `logical_rank`
   对应的 profile、route、`route_edge_index`、edge/progress spawn cursor；
   `initialSpeed` 遵循 #257 演示模式（与 H1 bootstrap 该 slot 的取值规则一致）；
   `VehicleReplaceExternalId` 固定为 **`Preserve`**（同一 logical slot 的 identity
   连续性，对齐 ADR 0016 slot 复用）。**禁止**用可能重复的
   `source_population_rank` 推导 `ReplaceWith` ID。
3. **不**为回流改写 placement 再抽签，**不**换到其他 `logical_rank`，**不**从
   static bundle 发明新 placement。
4. 入口 `Blocked`：保留该 pending plan 到后续 boundary 重试，**不**消耗额外 PRNG，
   **不**改 placement（ADR 0016 blocked-retry 语义）。
5. S1 标记为 excess 的 slot：completion 后 **不** replace；identity 保留至 H1 重建
   或未来另立的 typed despawn G1。若该 slot 在变为 excess **之前**已有 Blocked
   pending replace，必须在下一次 lifecycle 决策前 **确定性取消/作废**该 pending
   plan，且不得再重试；取消不消耗 PRNG。此外，excess slot 一旦 `Completed`，
   #257 必须在同一 lifecycle boundary（或紧随其后的 presentation 提交前）对绑定
   proxy 执行 **unbind + 移出场景**（`unbind_vehicle` 后 despawn Entity，或等价
   隐藏且不再参与 picking/HUD 计数）。不得留下“最后合法 Transform 冻结在终点”
   的可见幽灵车；该清理是 Presentation 义务，**不是** Core despawn（仍非 S4），
   也 **不是** S2（不改 `target_N` / 不自动降展示目标）。

同 `seed` / 同入选集合 / 同 fixed-step completion 序 ⇒ 同回流决策序列；#257 golden
须覆盖至少一次 selected-slot replace 与一次 Blocked 重试。

### 3.3 可选手动 H3（展示覆盖）

- 用户可手动设置 `N_presented ≤ N_individual` 作为性能覆盖。
- **不是**启动默认，也 **不是** 缩编时的自动策略。
- 演示默认：**100% of presentable**（见第 2 节）；不得把 completion→replace 间隙的
  瞬时差，或 S1-retained Completed 造成的持续差，当成 H3。

## 4. Seeded 无放回抽样

共享人口表含精确 10,000 条记录，`population_rank = 0..9999`
（[`real-road-workloads.md`](real-road-workloads.md) §4）。但共享表与 static
bundle **不**提供 tick-0 无碰撞 placement（§3.6 明确排除初始车辆）。#257
交互示例的抽样宇宙因此是 **TOPO plan 的唯一 `logical_rank ∈ 0..9999`**
（[`real-road-workloads.md`](real-road-workloads.md) §5.2）；每条 logical slot
已绑定唯一 `(route_edge_index, edge, progress)` placement 与
`source_population_rank` provenance。不得按可能重复的
`source_population_rank` 反查 placement。

冻结规则：

1. PRNG 使用与走廊 reference 相同的 **SplitMix64 + rejection sampling** 契约
   （见 `example-scenarios.md` / ADR 0016）；state 由 caller `seed` 初始化，并由
   example policy 独占，不进入 Core。
2. **算法（必须逐字节可复现）**：令 `arr = [0, 1, …, 9999]`。对
   `index` 从 `9999` 递减到 `1`：
   `swap_index = uniform(index + 1)`（即 `0..=index`），然后
   `swap(arr[index], arr[swap_index])`。这与走廊
   `signalized_corridor` 初始 slot permutation 同构。整次 shuffle 固定消耗
   9999 次有界 `uniform`（含其内部 rejection `next_u64`），与随后取用的
   `target_N` 无关。
3. **入选集合**：取 shuffle 后 `arr[0..target_N)` 作为入选 `logical_rank`
   序列；该顺序即为示例 logical slot 顺序。每个 selected rank 直接使用 TOPO
   plan 中对应 logical slot 的 spawn cursor / profile / route 构造
   `VehicleSpawnInput`（`initialSpeed` 遵循 #257 所选演示模式；TOPO 满载
   harness 语义仍为 0，交互示例可另定但必须写入 #257 证据）。初始
   `vehicle` external ID 冻结为 `lust-logical-{logical_rank}`（十进制、无前导零
   padding 以外的填充）；**禁止**用 `source_population_rank` 当唯一 external ID。
4. **同** `seed` + **同** `target_N` + **同** 算法版本 ⇒ **同** 入选序列；
   同 `seed` 下更大的 `target_N'` 的入选序列必须以前一 `target_N` 序列为真前缀。
   #257 必须提供 golden。
5. 拒绝「稳定前缀 `logical_rank = 0..N-1`」作为本示例默认。
6. Running 内不存在“继续抽剩余 pool”的扩编路径；提高人口只能 H1 重建并按本
   节对同一 `seed` 取更长前缀。

## 5. 缩编分层

| 策略                | 行为                                                                                            | 本契约                                          |
| ------------------- | ----------------------------------------------------------------------------------------------- | ----------------------------------------------- |
| S1 停补位           | 超出目标的 logical slot 停止 replace/回流；Completed 后 unbind/移出 proxy；Core identity 仍保留 | **允许**，但 **不**保证 `N_individual→target_N` |
| S3 H1 重建缩编      | 降低 `target_N` 后按第 4 节前缀重建 Session，使 `N_individual = target_N`                       | **精确缩编的权威路径**                          |
| S2 自动立刻降展示   | 缩编时自动把 `N_presented` 打到新目标                                                           | **不做默认**（与 100% 展示默认冲突）            |
| S4 超时主动 despawn | 超时后 despawn 仍 Active 的 live 车以加速收敛                                                   | **本契约不做**                                  |

说明：

- ADR 0016：Completed 车辆在 despawn / atomic replace 前仍占 identity；replace 保持
  人口基数。现有 Bevy Session 无 typed despawn（`core()` 只读），因此 **S1 单独无法**
  把 `N_individual` 降到新 `target_N`，也不能兑现“稳态 `N_presented = target_N`”。
- 凡需要 `N_individual == target_N`（含演示默认 100% presentation 对目标人口）的降档，
  **必须 S3 / H1 重建**。S1 只用于明确接受“identity 暂高于目标、仅停止补位”的观测模式，
  HUD 必须继续暴露真实 `N_individual`。
- 若 #257 需要 Running 内对 excess Completed 做 typed despawn 而不重建 Session，停止并
  另立 Adapter/G1；不得把 S4（对 Active live 车的超时驱逐）偷换为该能力。
- 手动 H3 仍可临时降低展示，但不得静默修改 Core 人口。

## 6. 与 A/B/C 制品的关系

| 制品               | 关系                                                                                              |
| ------------------ | ------------------------------------------------------------------------------------------------- |
| #253 static bundle | 示例加载的路网/信号权威；**不含**初始车辆 placement                                               |
| 共享一万 rank 表   | TOPO/DEMAND 共享 provenance；**不是** #257 的 placement 权威                                      |
| #254 TOPO plan     | **#257 交互示例必需**的 rank→placement 权威；抽样作用于其 `logical_rank`；不得改 TOPO workload ID |
| #255 DEMAND        | 不作为可滑杆人口模型；若演示 departure，须独立模式且不与 H2 热改 / TOPO 子集抽样混用              |

无 TOPO plan 时，#257 不得声称已满足本契约的 H1 bootstrap；不得用 static
bundle + 人口表临时发明 collision-free `route_edge_index`/progress。

## 7. 验证要求（由 #257 交付）

- 同 seed / `target_N` / fixed-step 输入 ⇒ 同抽样、同 selected-slot 回流与 Blocked
  重试决策序列。
- H1 重建升/降档与 S1 停补位不破坏 hard invariants；非法请求 fail-closed 或保持上一合法目标。
- 证据须区分：S1 后 `N_individual` 可仍高于 `target_N`；S3/H1 后必须相等。
- 证据矩阵至少覆盖 `target_N ∈ {1, 100, 1000}`；`10000` 按机器能力记录，未认证前
  不写 Product Pass。
- GUI smoke 必须能读出第 2 节四个计数。
- 稳定容量下，lifecycle 路径不得引入与全体车辆数成正比的 steady-tick 临时分配
  （与 ADR 0016 / 走廊基线同口径）。

## 8. 下游

- 实现：#257 Bevy LuST native 示例与调节 UI。
- 设计 Issue：#256；Parent tracker：#252。
- 若实现需要新的 Core/Adapter public API、通用 scenario controller 或 schema，
  停止并另立 G1/ADR，不得在本契约下静默扩展。
