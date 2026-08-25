# Signalized Corridor Population

**文档状态**: Accepted（#203 G1；#475 Runtime 回流）<br>
**最后更新**: 2026-08-24<br>
**适用范围**: current v0.10 signalized-corridor catalog 0.3 人口/回流 policy；
caller-owned authority 继续继承 ADR 0016。catalog 字符串在 prepare 绑到共享路网修订
（#472）；50–200 原子替换由 #475 交付。

**关联文档**:

- [`example-scenarios.md`](example-scenarios.md)
- [`signalized-corridor-protected-turning.md`](signalized-corridor-protected-turning.md)
- [`../adr/0016-scenario-population-and-recycle-lifecycle-authority.md`](../adr/0016-scenario-population-and-recycle-lifecycle-authority.md)
- [`core-runtime.md`](core-runtime.md)
- [`bevy-reference-adapter.md`](bevy-reference-adapter.md)

## 1. 边界与依赖

`laneflow-scenario` 是可选、引擎无关的 reference policy crate。依赖方向固定为：

```text
laneflow-corridor-generator -> laneflow-scenario
  -> laneflow-compiler（Identity v1 派生）
  -> laneflow-static-network（已安装共享路网修订）
  -> laneflow-runtime（TrafficWorld 生命周期命令）
```

generator 只复用 scenario crate 公开的 catalog wire DTO；scenario crate 不读取文件系统，不依赖 Spatial、Bevy 或其他 Engine Adapter。Runtime、Adapter 和宿主游戏都不反向依赖 scenario crate。城市游戏可以用自己的 policy 完全替代本实现。

本 crate 拥有：

- `50..=200` 目标 logical population 与默认值 `100`；
- caller 提供的 `u64 seed` 与默认值 `0`；
- corridor catalog 的 closed TOML shape、语义校验和规范顺序；
- 初始 slot permutation、completion 顺序消费、portal/lane 抽样和 blocked retry；
- logical slot 到当前 `VehicleHandle` 的 caller-owned 映射。

本 crate 不拥有：

- `TrafficWorld` 占用/跟车实现或 replacement transaction 本体；
- Entity、Transform、模型、UI 或 outer-frame 时间；
- Traffic/Spatial/Manifest 的加载路径和持久化格式；
- 通用人口 controller 或面向任意路网的路径搜索。

## 2. 两阶段启动

#472 现行 prepare 绑定是 `validate(catalog)` 之后
`bind(catalog, &SharedNetworkRevision)`：编制字符串经 Identity v1 派生
`StableId128`，再查已安装修订的 `SharedIdentityIndex`。可运行世界由 LFCA 安装，不再经过已拆除的 JSON/Core
运行时入口。

#475 现行启动协议：

启动使用 catalog `bind`、`CorridorPopulationPrepare::prepare` 与 controller `bind`：

1. caller 安装共享路网修订并在内存中解析 catalog；
2. `validate` 对 catalog 0.3 完成交叉引用与边键校验，`bind` 解析边序号、`register_route`，并把句柄钉到该 `NetworkRevisionId`；
3. `prepare` 校验 config/profile，执行一次确定性 Fisher–Yates，返回完整 `CorridorVehiclePlan` batch；
4. caller 在 `TrafficWorld` 上按计划逐辆 `spawn_vehicle`；
5. population bind 必须发生在 tick 0，并按已绑定序号回查所有 vehicle、route 和 profile identity；
6. 全部 identity 一致后，controller 才进入 `Running = target, Pending = 0`。

`take_initial_vehicles` 是一次性转移。Runtime spawn 失败或 bind 发现任一缺失、stale、route/profile/status/progress 不一致时，启动整体失败，不进入首个 step。

## 3. Catalog 契约

catalog version 固定为 `0.2`，必须精确包含：

- 文档化顺序中的 6 个 portal；
- 每个 portal 的 ordered PortalLane：主干道 portal 各 3 条、次干道 portal 各 2 条；
- 每条 PortalLane 的共享 entry SpawnSlot 与至少一个正权重 RouteChoice；
- 全部 28 条 Traffic Route 到 exit portal 的唯一 cross-reference；
- 至少 200 个 route-independent physical SpawnSlot。

normalize 必须拒绝未知或重复 portal/route/slot、lane count/index 不一致、空 route
choice、零权重或 weight sum overflow、重复 choice、dangling Traffic route、相同
entry/exit portal、slot portal/lane/edge occurrence 不一致、非有限或越界 progress、
重复物理位置及非法共享 entry slot。

原始 TOML 中 portal、lane、route choice、route cross-reference 和 slot 的排列不是
runtime authority。normalize 后 portal 使用文档顺序，lane 使用 lane index，route
choice 使用 Traffic Route 输入顺序，physical slot 使用
`(portal, lane, route edge occurrence, edge progress, slot ID)`；同一语义 catalog 的
原始重排必须得到相同结果。

## 4. Replay 与初始人口

PRNG 使用 `example-scenarios.md` 冻结的 SplitMix64 和 rejection sampling。初始 permutation 与所有回流 draw 共享一个 controller-owned state；不使用 thread RNG、hash iteration、文件系统顺序或 ECS iteration。

Runtime 没有 external ID 字符串。`prepare` 对完整规范 physical slot catalog 执行从末尾
到开头的 Fisher–Yates 后取前 N 个 slot，再按 logical slot 顺序对其 PortalLane 执行一次
weighted RouteChoice draw。单 choice 也必须使用原始正整数 weight 作为 `uniform` bound，
不能跳过 draw。每个 initial slot 与每条 route 的共享 entry slot 都派生
`min(VehicleProfile.desiredSpeed, spawn edge speedLimit)` 作为正常行驶初速度；没有
speed-limit authority 时启动失败。50、100、200 三种目标人口都必须通过同 seed
整批 golden、初速度上限/正值、no-overlap spawn 和 tick-0 bind 验证。

## 5. Fixed-step lifecycle

controller 只消费 caller 传入的已提交 `TrafficWorld`，不主动驱动 step：

```text
apply pending lifecycle commands
  -> TrafficWorld fixed step
  -> consume ordered Completed vehicles
  -> enqueue frozen plans for next lifecycle boundary
```

`consume_world` 与 `pending_spawn_input` 先校验传入 `TrafficWorld` 的 `NetworkRevisionId` 与 catalog bind 一致；`apply_pending` 先校验调用方提供的 `NetworkRevisionId`（通常来自即将提交 replace 的那份 `TrafficWorld`）。`consume_world` 再要求恰好消费上一拍之后的那一拍（`last_consumed_tick + 1`），并以先验证、后提交的方式处理整个 completion batch。跳过中间 tick 或同一拍重复消费都失败。Running 句柄若已从 world 消失，视为「先消失再生成」契约失败。每个 completion 必须满足：

- vehicle 属于一个 `Running` logical slot，状态为 `Completed`，且同一 batch 不重复；world 中未跟踪的 Completed 车辆同样使整个 batch 失败；
- route handle 等于该 logical slot 当前 route；
- route edge occurrence 精确等于该 route 末端。

任一校验失败时，batch 不更新 logical state、PRNG、pending queue 或 last consumed tick。

验证通过后按 event 原始顺序处理。每个完成车辆固定消费三个 logical draw site：
先从排除原出口的 5 个 portal 中均匀抽取目标 portal，再从目标 portal 的 2 或 3 条
PortalLane 中均匀抽取 lane，最后按该 lane 完整、规范化的正整数权重 cumulative
选择 Route。它不对全部 28 条 Route 直接均匀抽样。

pending plan 冻结抽中的 portal lane、目标 route 及其 entry edge 正常行驶初速度。入口 overlap 返回
`Blocked` 时不改 plan、不消耗 PRNG、不降速重试；成功 replacement 后仍由 `TrafficWorld::step`
在首个 fixed tick 合并 leader、SignalStop、speed limit 与 no-overlap 约束。

## 6. Pending 与 host transaction

每个 logical slot 只有两种状态：

```text
Running(vehicle, route)
Pending(old, frozen route plan)
```

`apply_pending` 先校验 `NetworkRevisionId`；host callback 仍是 transport-neutral，caller 可把同一 `VehicleSpawnInput` 交给 `TrafficWorld` 或 Adapter 的 typed transaction，并将结果映射为：

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
- 成功 logical identity rotation（新 `VehicleHandle`）。

200 车持续运行不得产生无界 queue、history 或 retained capacity 增长。一轮轮换计一次成功 `Replaced`；`Blocked` 不计入。默认 CI 覆盖 50 车与 200 车的短容量 soak，以及独立 headless world 上同一 per-tick 链（`apply_pending` → `step` → `consume_world`）的确定性对拍。这不替代 Bevy outer-frame accumulator / `max_catch_up_steps` 调度测试；把多拍 `step` 叠在一次 `consume_world` 之前必须失败。完整 10,000 次成功 replace 由 `#[ignore]` 测试承担，复现：

```powershell
cargo +1.96.0 test --offline -p laneflow-scenario --test signalized_corridor_population soak_50_cars_10000_replacements -- --ignored --exact
```

## 8. Replay 与兼容性

catalog 0.2 规范顺序、SplitMix64 算法、physical-slot Fisher–Yates、initial
weighted route draw、completion 的 portal/lane/route draw order、raw weights、
initial ID 和 batch permutation 共同构成 current replay contract。blocked retry
不 draw、不改变 frozen plan。修改其中任一项必须通过新的设计/迁移决策，不能作为
无说明的内部重构。

catalog `0.1 -> 0.2` 是无兼容 clean break，不提供旧 DTO、dual parser、alias 或
migration shim。本实现通过 `TrafficWorld::replace_completed_vehicle` 与 Adapter
typed 换绑交付回流；不恢复 current JSON / `laneflow-data`，也不把人口政策写入 `step`。

## 9. v0.9 catalog 0.2 current 实现

#190 在不改变 caller-owned lifecycle、ordered completion、bounded state 与 blocked
retry authority 的前提下，已按 #196 clean-break 实现：

- Portal 拥有 ordered PortalLane；
- PortalLane 引用共享 entry SpawnSlot，并拥有 weighted full RouteChoice；
- completion 固定调用 portal、lane、route 三个 logical bounded draw site；
  每个 site 保留既有 rejection sampling，固定的是调用点与顺序；
- route site 对所有 PortalLane 统一调用 `uniform(totalPositiveWeight)`；单一
  RouteChoice 也以其正整数 raw weight 为 bound，必然选中唯一 Route，但不得改用
  `uniform(1)`、跳过 draw 或预先约分权重；
- 初始化先做 physical-slot Fisher–Yates，再按 logical slot order 为每个 slot
  消费一次 Route draw；
- SpawnSlot 不再拥有单一 Route，多条 Route 可共享同一 PortalLane entry slot；
- 默认 pitch 为 10 m，只在真实 portal approach edges 上生成 physical slots。

完整 28 Route、weights、catalog 0.2 ownership 与 golden draw order 见
[`signalized-corridor-protected-turning.md`](signalized-corridor-protected-turning.md)。
