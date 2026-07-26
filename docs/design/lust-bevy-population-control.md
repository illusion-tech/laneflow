# LuST / Bevy 个体人口调节契约

**文档状态**: Accepted（#256 G1）<br>
**最后更新**: 2026-07-26<br>
**适用范围**: LuST 真实路网 Bevy native 示例（#257）的 1–10k 个体人口调节、
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

本文冻结 LuST/Bevy 示例层如何把共享的精确 10k `population_rank` 人口表调节到
`target_N∈[1,10000]`，以及 HUD/证据必须暴露哪些正交计数。

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
- 改写 `LF-REAL-LUST-TOPO-v1` / `LF-REAL-LUST-DEMAND-v1` 的固定 10k 语义；
- 宣称 10k@100% presentation 的 Product Pass / realtime SLA（见 #215；P10 未认证）。

走廊场景继续以 [`signalized-corridor-population.md`](signalized-corridor-population.md)
的 `50..=200` 为权威；本文只服务 LuST 示例路径（Parent #252 / #257）。

## 2. 正交计数

HUD 与 headless 证据必须同时可读：

| 符号               | 含义                                                  |
| ------------------ | ----------------------------------------------------- |
| `target_N`         | 示例 policy 当前目标 logical population，`[1, 10000]` |
| `N_individual`     | Core 中仍保留完整 identity 的个体数                   |
| `N_traffic_active` | 参与道路交通权威的个体数                              |
| `N_presented`      | 本 outer frame 被 materialize / 提交展示的个体数      |

默认稳态满足 `N_presented = N_individual`。不得用 `N_presented` 冒充
`N_individual`。

## 3. 调节组合（H1 / H2 / H3）

### 3.1 H1 启动档位

- 启动必须显式提供 `target_N∈[1,10000]` 与 `u64 seed`（零 seed 合法）。
- 禁止隐式默认静默升到 10k。
- bootstrap 在第一个 Core step 之前完成；失败则整体不进入 Running。

### 3.2 受约束 H2（运行中改目标）

- 运行中允许修改 `target_N`。
- 收敛只发生在 **fixed-step lifecycle boundary**（ADR 0016 顺序：pending
  commands → Core step → completions → 下一 boundary 的计划），不得按
  outer-frame 次数偷改。
- **扩编**：对尚未入选的 `population_rank` 继续 seeded 无放回抽样，补足差额；
  仅使用现有 Core spawn / Adapter bind 路径。
- **缩编**：见第 5 节。

### 3.3 可选手动 H3（展示覆盖）

- 用户可手动设置 `N_presented ≤ N_individual` 作为性能覆盖。
- **不是**启动默认，也 **不是** 缩编时的自动策略。
- 演示默认：**100% presentation**，即 `N_presented = N_individual`。

## 4. Seeded 无放回抽样

共享人口表含精确 10,000 条记录，`population_rank = 0..9999`
（[`real-road-workloads.md`](real-road-workloads.md) §4）。`target_N < 10000`
时必须规定选中哪一子集。

冻结规则：

1. PRNG 使用与走廊 reference 相同的 **SplitMix64 + rejection sampling** 契约
   （见 `example-scenarios.md` / ADR 0016）；state 由 caller `seed` 初始化，并由
   example policy 独占，不进入 Core。
2. **启动**：对索引数组 `0..9999` 做一次 Fisher–Yates shuffle，取前 `target_N`
   个 ranks 作为初始入选集合（顺序保留为 logical slot 顺序）。
3. **扩编**：在同一 PRNG 流上，仅对 **尚未入选** 的 ranks 继续无放回抽取差额；
   不得重洗已入选集合。
4. **同** `seed` + **同** `target_N` + **同** 算法版本 ⇒ **同** 初始入选集合；
   #257 必须提供 golden。
5. 拒绝「稳定前缀 `0..N-1`」作为本示例默认；前缀规则不属于本契约。

## 5. 缩编分层

| 策略                | 行为                                                                  | 本契约                               |
| ------------------- | --------------------------------------------------------------------- | ------------------------------------ |
| S1 自然缩编         | 停止对超出目标的 slot 做 replace/回流；等 route completion 后不再补位 | **默认权威路径**                     |
| S3 大降幅重启       | 降幅 ≥ 50% 或绝对下降 ≥ 1000 时 **推荐** H1 重建 session              | 推荐，不强制                         |
| S2 自动立刻降展示   | 缩编时自动把 `N_presented` 打到新目标                                 | **不做默认**（与 100% 展示默认冲突） |
| S4 超时主动 despawn | 超时后 despawn live 车以加速收敛                                      | **本契约不做**                       |

说明：

- LuST 路网尺度大，S1 的 wall-clock 收敛可能很长；产品应通过 HUD 暴露真实
  `N_individual`，并用 S3 处理大跳档预期。
- 缩编期间默认 `N_presented` 跟随当前 `N_individual`（仍为 100% of remaining）。
- 手动 H3 仍可临时降低展示，但不得静默修改 Core 人口。

## 6. 与 A/B/C 制品的关系

| 制品               | 关系                                                                    |
| ------------------ | ----------------------------------------------------------------------- |
| #253 static bundle | 示例加载的路网/信号权威                                                 |
| 共享 10k rank 表   | 抽样宇宙；由 converter/population 表提供                                |
| #254 TOPO plan     | 可选「满载 10k」布局源；`target_N<10000` 时仍按本文抽样，不得改 TOPO ID |
| #255 DEMAND        | 不作为可滑杆人口模型；若演示 departure，须独立模式且不与 H2 热改混用    |

## 7. 验证要求（由 #257 交付）

- 同 seed / `target_N` / fixed-step 输入 ⇒ 同抽样与回流决策序列。
- H2 扩缩编不破坏 hard invariants；失败 fail-closed 或保持上一合法目标。
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
