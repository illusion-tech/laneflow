# Signalized Corridor Population

**文档状态**: Accepted（#203 G1）<br>
**最后更新**: 2026-07-23<br>
**适用范围**: v0.8 signalized-corridor 的目标人口、确定性初始分布、完成事件消费与出口回流 reference policy

**关联文档**:

- [`example-scenarios.md`](example-scenarios.md)
- [`../adr/0016-scenario-population-and-recycle-lifecycle-authority.md`](../adr/0016-scenario-population-and-recycle-lifecycle-authority.md)
- [`core-runtime.md`](core-runtime.md)
- [`bevy-reference-adapter.md`](bevy-reference-adapter.md)

## 1. 边界与依赖

`laneflow-scenario` 是可选、引擎无关的 reference policy crate。依赖方向固定为：

```text
laneflow-corridor-generator -> laneflow-scenario -> laneflow-core
```

generator 只复用 scenario crate 公开的 catalog wire DTO；scenario crate 不读取文件系统，不依赖 Data、Spatial、Bevy 或其他 Engine Adapter。Core、Adapter 和宿主游戏都不反向依赖 scenario crate。城市游戏可以用自己的 policy 完全替代本实现。

本 crate 拥有：

- `50..=200` 目标 logical population 与默认值 `100`；
- caller 提供的 `u64 seed` 与默认值 `0`；
- corridor catalog 的 closed TOML shape、语义校验和规范顺序；
- 初始 slot permutation、completion 顺序消费、portal/lane 抽样和 blocked retry；
- logical slot 到当前 Core vehicle identity 的 caller-owned 映射。

本 crate 不拥有：

- `CoreWorld`、交通状态、overlap 或 replacement transaction；
- Entity、Transform、模型、UI 或 outer-frame 时间；
- Traffic/Spatial/Manifest 的加载路径和持久化格式；
- 通用人口 controller 或面向任意路网的路径搜索。

## 2. 两阶段启动

启动使用 `CorridorPopulationPrepare::prepare` 与 `bind` 两阶段协议：

1. caller 用 production loader 取得 `InitialTrafficData`，并在内存中解析 catalog；
2. `CorridorCatalog::normalize` 对 production Traffic 完成 cross-reference validation；
3. `prepare` 校验 config/profile，执行一次确定性 Fisher–Yates，返回完整 `VehicleSpawnInput` batch；
4. caller 只调用一次 `CoreWorld::with_traffic_data` 提交完整 batch；
5. `bind` 必须发生在 tick 0，并按 external ID 回查所有 vehicle、route 和 profile identity；
6. 全部 identity 一致后，controller 才进入 `Running = target, Pending = 0`。

`take_initial_vehicles` 是一次性转移。Core batch 创建失败或 bind 发现任一缺失、stale、route/profile/status/progress 不一致时，启动整体失败，不进入首个 step。

## 3. Catalog 契约

catalog version 固定为 `0.1`，必须精确包含：

- 文档化顺序中的 6 个 portal；
- 14 条 lane route，主干道 portal 各 3 条、次干道 portal 各 2 条；
- 至少 200 个 stable spawn slot；
- 每条 route 的 entry spawn slot。

normalize 必须拒绝未知或重复 portal/route/slot、portal route set 不一致、重复 portal/lane、相同 entry/exit portal、dangling Traffic route、slot portal/route/edge occurrence 不一致、非有限或越界 progress、重复物理位置及非法 entry slot。

原始 TOML 中 portal、route、slot 和 `entry_route_ids` 的排列不是 runtime authority。normalize 后顺序固定为 portal 文档顺序、lane index、route edge occurrence、edge progress、slot ID；同一语义 catalog 的原始重排必须得到相同结果。

## 4. Replay 与初始人口

PRNG 使用 `example-scenarios.md` 冻结的 SplitMix64 和 rejection sampling。初始 permutation 与所有回流 draw 共享一个 controller-owned state；不使用 thread RNG、hash iteration、文件系统顺序或 ECS iteration。

每个 logical slot 使用 `corridor-vehicle-{index:03}` external ID。`prepare` 对完整规范
slot catalog 执行从末尾到开头的 Fisher–Yates 后取前 N 个 slot。每个 initial slot
与每条 route 的 entry slot 都派生
`min(VehicleProfile.desiredSpeed, spawn edge speedLimit)` 作为正常行驶初速度；没有
speed-limit authority 时启动失败。50、100、200 三种目标人口都必须通过同 seed
整批 golden、初速度上限/正值、Core batch no-overlap 和 tick-0 bind 验证。

## 5. Fixed-step lifecycle

controller 只消费 caller 传入的 ordered `StepResult`，不主动驱动 Core：

```text
apply pending lifecycle commands
  -> Core fixed step
  -> consume ordered completion events
  -> enqueue frozen plans for next lifecycle boundary
```

`consume_step_result` 要求 tick 严格递增，并以先验证、后提交的方式处理整个 completion batch。每个 completion 必须满足：

- event tick 等于 `StepResult.tick_index`；
- vehicle 属于一个 `Running` logical slot，且同一 batch 不重复；
- route handle 等于该 logical slot 当前 route；
- edge handle 与 route edge occurrence 精确等于该 route 末端。

任一校验失败时，batch 不更新 logical state、PRNG、pending queue 或 last consumed tick。

验证通过后按 event 原始顺序处理。每个完成车辆先从排除原出口的 5 个 portal 中均匀抽取一个 portal，再从目标 portal 的 2 或 3 条 lane route 中均匀抽取一条；不使用 movement weight，也不对 14 条 route 直接均匀抽样。

pending plan 冻结目标 route 及其 entry edge 正常行驶初速度。入口 overlap 返回
`Blocked` 时不改 plan、不消耗 PRNG、不降速重试；成功 replacement 后仍由 Core 在首个
fixed tick 合并 leader、SignalStop、speed limit 与 no-overlap 约束。

## 6. Pending 与 host transaction

每个 logical slot 只有两种状态：

```text
Running(vehicle, route)
Pending(old, frozen route plan)
```

`apply_pending` 是 transport-neutral lifecycle API。caller 可把同一 `VehicleReplaceInput` 交给 Core 或 Adapter 的 typed transaction，并将结果映射为：

- `Replaced(old, new)`：controller 以 new handle 原子轮换 logical identity，回到 Running；
- `Blocked(old, blocker, ...)`：保留 old 与 frozen plan，移动到 FIFO 队尾；
- fatal host error：恢复当前 slot 到 FIFO 队首并返回 host error；
- identity 不一致或 new handle 已被跟踪：返回 policy contract error。

一个 lifecycle boundary 只尝试进入 boundary 时已存在的 pending 数量，因此每个 plan 最多尝试一次；blocked retry 不 draw、不改 plan，且不会阻止其他 pending plan。

## 7. 有界状态与分配

controller 在 bind 时按目标人口预留所有 steady containers。completion validation 使用复用的 slot-index/seen scratch；pending 使用有界 FIFO；logical state、plan 和 PRNG state 都是定长数据。

下列已预热 steady path 必须保持零分配：

- 无 completion 的 ordered step；
- completion batch 校验与提交；
- blocked retry；
- Preserve external ID 的成功 logical identity rotation。

200 车持续运行不得产生无界 queue、history 或 retained capacity 增长。测试基线至少覆盖 10,000 次 completion/replacement 轮换，以及不同 outer-frame catch-up chunking 下相同 fixed-step input 的 replay 一致性。

## 8. 兼容性

catalog 顺序、SplitMix64 算法、draw order、portal-first/lane-second 规则、initial ID 和 batch permutation 都属于 v0.8 replay contract。修改其中任一项必须通过新的设计/迁移决策，不能作为无说明的内部重构。

本实现不改变 Core API、Traffic/Spatial/Manifest 格式或 Adapter API；共享 catalog DTO 从 generator 移至 scenario crate 只消除 authoring/runtime shape 漂移，checked-in generator bytes 必须保持不变。
