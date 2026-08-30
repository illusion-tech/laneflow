# 中国特色城市工作负载 v1

**文档状态**: Draft（#304）；其中 §4 停车切片已 Accepted（#540 G1）<br>
**最后更新**: 2026-08-30<br>
**适用范围**: `LF-CN-URBAN-v1` 的 topology / demand / runtime 分层、计数口径、
首批场景边界与阶段依赖<br>
**实现状态**: #304 尚未完成 G1；本文不声称已有 production generator、完整行为域或
Product Pass。#540 只冻结停车输入，#541 负责实现，#543 负责 exact topology/容量核算。<br>
**关联文档**:

- `parking-system.md`
- `core-runtime-performance-baseline.md`
- `waiting-zone-conflict-right-of-way.md`
- `cross-section-access.md`
- `signalized-corridor-protected-turning.md`
- `real-road-workloads.md`
- `../adr/0010-parking-binding-and-vehicle-lifecycle-authority.md`
- `../adr/0021-city-simulation-game-traffic-foundation.md`

## 1. 目的和边界

`LF-CN-URBAN-v1` 用于证明 LaneFlow 能支撑中国特色城市模拟游戏中真实会发生的首批
交通闭环，而不是用一个抽象车辆数或全国法规口号替代可复核场景。

工作负载分三层：

1. **topology artifact**：道路、路口、信号、准入、停车等不可变静态事实；
2. **demand artifact**：可重放的出行请求、精确停车目标、生成/离开序列与 provenance；
3. **runtime artifact**：实际 live/active/parked/presented 状态、命令序列、摘要和性能证据。

三层必须独立计数和版本化。城市人口、停车容量、静态表行数、道路 Active vehicle 和
引擎 presented entity 不是同一个数字，不能相互代替。

本文当前只把 #540 已决策的停车切片写成候选实现输入。多阶段信号、左转待转区、干支路、
小区出口、方向性高峰、公交/出租/上下客/路侧摩擦等仍由 #304 及其依赖逐项冻结；本次
停车文档变更不代表这些切片已完成。

## 2. 规模和计数口径

每个交通执行域 `d` 至少报告：

- `N_individual[d]`：仍保留完整 identity 和 committed state 的个体；
- `N_active[d]`：当前 tick 参与该执行域运动/约束的个体；
- `N_intent[d]`：等待进入或改变执行状态的意图；
- `N_presented[d]`：当前交给引擎表现的个体；
- `N_aggregate_records[d]`：真实存在的聚合记录；
- `N_aggregate_equivalent[d]`：聚合记录代表的等价规模，仅在定义了转换口径时报告。

停车另报告：

```text
N_parking_facility
N_parking_space_explicit
N_parking_virtual_anchor
C_parking_virtual_declared
B_parking_explicit_reserved / occupied
B_parking_virtual_reserved / occupied
```

必须满足：

```text
N_individual[road_motor_vehicle]
  = N_active[road_motor_vehicle]
  + N_parked_explicit
  + N_parked_virtual
  + 其他已登记 live 非 Active 状态

N_presented[road_motor_vehicle]
  不包含 virtual Parked，但 virtual Parked 仍计入 N_individual
```

`C_parking_virtual_declared = 100_000` 不意味着有 100,000 个 `ParkingSpace`、LFCA 行、
Runtime slot 或 presented entity。

## 3. 两档产品规模

`LF-CN-URBAN-v1` 保留 10k 与 100k 两档声明规模，用于现实的城市游戏预算和稀疏/高占用
行为验证。这里的数字首先描述 runtime 个体/停车容量目标，不提前决定 topology 的道路、
泊位或 relation 行数。

精确设施分布、各表行数、各 IR 节点数、artifact bytes 和 build/load 峰值由 #543 的
counting spike 冻结。#540 只提供现实 workload，不复制或改写 compiler profile、公共
registry、静态格式版本轴或容量上限；测量结果必须回到各自 SSOT/独立 G1 裁决，也不能
为了迎合既定结论删除真实拓扑语义。

## 4. 停车切片（#540 G1 合同）

### 4.1 代表性场景

首批 topology 必须至少含以下可复核场景：

| 场景              | 静态表达                                                                                                        | 产品意义                                                      |
| ----------------- | --------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------- |
| 工厂混合停车      | 一个 `ParkingFacility`；多个可见地面 `ParkingSpace`；非零 virtual capacity；地面与室内可共用同一外部道路 anchor | 验证用户确认的“同一工厂、同一外部入口、地面可见 + 室内不可见” |
| 住宅/商业地下车库 | virtual-only `ParkingFacility`，至少一个入口和一个出口                                                          | 验证不可见容量不生成内部路网/泊位                             |
| 多门设施          | virtual facility 有多个 entry 和/或 exit                                                                        | 验证 caller 精确选门、路线 occurrence 与离场安全              |
| 路侧显式泊位      | 独立或设施内 `ParkingSpace`                                                                                     | 保留玩家可观察的 exact space/pose 语义                        |

多门不是要求每个设施都必须有多个门；单入口/单出口仍是常见合法实例。混合工厂中，
显式泊位与 virtual pool 是两个独立 admission pool，设施总容量仅作报表。

### 4.2 Topology artifact

停车 topology 只物化：

- `F` 个 `ParkingFacility`；
- `S` 个真实可见 `ParkingSpace`；
- `A` 个 virtual entry/exit anchor；
- 每个设施一个 `u32 virtual_capacity`。

不得物化：

- `C_parking_virtual_declared` 个伪泊位；
- 不可观察车库内部 LaneEdge/Route；
- virtual parked pose 或空容量 slot；
- 与车辆数等长的静态停车对象。

### 4.3 Demand artifact

每个停车意图必须指定：

```text
vehicle / trip identity
exact ParkingTarget = ExplicitSpace | VirtualPool
entry route occurrence
virtual target 时显式携带 exact facility entry anchor selector
计划离场时的 route occurrence
virtual target 时显式携带 exact facility exit anchor selector
```

anchor selector 与 route occurrence 是两个正交输入：occurrence 选择动态 route 第几次
经过某条 LaneEdge，selector 选择该设施在这条 LaneEdge 上的哪个整数毫米 anchor。同一
LaneEdge 上存在多个入口/出口时，只有 facility + occurrence 的意图必须视为不完整。

demand/orchestration 层负责基于玩法、任务、收费或偏好选择 target。Runtime 只验证并执行
exact intent，不自动决定“停地面还是进室内”。满位后的换目标、等待或 reroute 也是
demand/Routing 策略，不在 #540 Runtime authority 内。

### 4.4 Runtime artifact

运行时必须分别记录：

- explicit/virtual reserved、occupied、vacant；
- 每个 live vehicle 的 exact tagged binding；
- virtual reservation 的 selected entry；
- park/leave/cancel/despawn 的 command result；
- exact arrival 的 occurrence、整数毫米 anchor、`speed_mm_s=0`、`carry_um=0` 以及
  `SignalStop -> ParkingStop -> RouteEnd` 同值归因；
- `N_active`、`N_individual` 和 `N_presented` 的状态变化；
- stable digest/order 和失败零副作用证据。

成功 virtual park 后：

```text
N_individual 不变
N_active -= 1
N_presented -= 1 当且仅当该车辆提交前属于 presented set；否则不变
B_parking_virtual_occupied += 1
```

成功 virtual leave 后 `N_individual` 不变、`N_active += 1`；只有新的 committed lane pose
被 Adapter presentation/LOD 策略实际纳入 presented set 时，`N_presented` 才增加 1，否则
保持不变。unsafe exit 时以上计数和表现都不变。验收必须比较提交前后真实集合成员关系和
完整状态分解，不能把 park/leave 当成无条件 `N_presented -1/+1`。

成功 `despawn_vehicle` 才使 `N_individual` 减一，并在同一提交释放 parking binding 与
route reference；virtual Parked 无 pose 或 Adapter 隐藏不改变 `N_individual`，也不能
替代 removal observation。

### 4.5 规模验证

10k/100k 至少区分两种压力：

1. **大容量、稀疏占用**：证明 static/Runtime retained 与 `F+S+A+B` 相关，不与空容量
   线性相关；
2. **高实际停驻**：证明 live vehicle/binding 按 `B` 线性增长，Parked 不进入 tick 的
   lane/following/motion 热路径。

必须记录 declaration/IR/LFCA/shared-static/runtime/Adapter 各层计数，不能只报进程总内存
或帧率。理论 `u32` 极限不属于首批 Product Gate；exact/exact+1 和现实 10k/100k 才是。

## 5. 其他首批场景的当前状态

| 切片                      | 当前状态                                                | 本文允许的声明                                |
| ------------------------- | ------------------------------------------------------- | --------------------------------------------- |
| 多阶段信号/左转待转区     | 依赖现有 signal 与 waiting/conflict 设计，#304 尚需整合 | 只登记输入，不声称 workload 已通过            |
| 干支路与小区出口          | topology/法规 fixture 需与 #238 对齐                    | 不用普通十字路口替代                          |
| 方向性高峰                | demand 规则、seed 和 oracle 待冻结                      | 不用均匀随机人口替代                          |
| 公交/出租/上下客/路侧摩擦 | 首批闭环尚待 #304 决定                                  | 未实现域必须标为 unsupported/future           |
| 非机动车/步行/轨道        | 不在当前道路机动车 Runtime 生产能力内                   | 不以机动车伪装支持                            |
| 停车                      | #540 G1 Accepted；#541 未实现                           | 只可声称设计合同，不可声称 production support |

## 6. 制品与可重放要求

最终 #304 G1 必须为三层制品冻结：

- workload ID/version；
- seed 与来源 provenance；
- topology、demand、runtime 各自 digest；
- hardware role、worker/tick/fidelity 配置；
- per-domain counts 和 accepted unsupported list；
- functional oracle、state digest、失败面和性能 Gate；
- current/target 比较的合法范围，不把旧 `CoreWorld` 当永久预言机。

#540 只提供停车字段和 oracle 输入；不替 #304 选择最终城市地图、需求分布、硬件门槛或
总体性能阈值。

## 7. 阶段和完成边界

1. #540 G1 Accepted：停车设施和虚拟停驻合同可作为实现输入。
2. #541 完成：production compiler/Runtime/Adapter 能跑停车切片。
3. #543 完成：10k/100k exact topology 容量判断可复核。
4. #304 G1 Accepted：其余首批场景、三层制品和 Product Gate 一并冻结。
5. 后续 generator/harness/certification 交付可运行制品和证据。

任一子切片完成都不能把 #304 父任务标为完成。报告必须明确区分：已满足、依赖绑定、
显式延后和 unsupported；不得以“停车 100k 容量可表达”推导“100k 城市交通已通过”。
