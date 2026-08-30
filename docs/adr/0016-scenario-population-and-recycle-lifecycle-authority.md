# 0016 Scenario Population 与车辆回流生命周期权威

**状态**: Accepted<br>
**日期**: 2026-07-22<br>
**最后更新**: 2026-08-30（#540 原子移除与停车生命周期修订）<br>
**适用范围**: Signalized Corridor 的 caller-owned population policy、TrafficWorld 原子替换/移除命令、seeded 回流、Bevy proxy 复用与启动配置边界

## 背景

v0.7 已交付固定步长 Core、动态 vehicle handle、Traffic/Spatial/Scenario 制品、Signals、Vehicle Following 与 Bevy Reference Adapter，但持续运行的参考场景还缺少统一的车辆人口和回流协议：

- `TrafficWorld::spawn_vehicle` 是独立原子命令，但不能把已完成车辆替换为新 identity；
- `VehicleHandle` 必须在替换后失效，不能把同一 handle 重置到另一条 route；
- `LaneFlowSession::world_mut()` 只适合不会改变已绑定 handle 身份的 Runtime 命令；已绑定
  车辆的原子替换或移除必须走 typed Session 入口，不得经 `&mut TrafficWorld` 绕过映射
  轮换/清理；
- Completed vehicle 不再进入 pose batch，因此已绑定 proxy 会保留最后一次合法 Transform；
- 共享静态路网与 catalog 不持久化 initial vehicles、spawn schedule、runtime handles 或 Adapter metadata；
- TrafficWorld fixed step 不读取 wall clock、全局随机数或引擎状态。

走廊要求 50–200 辆车持续运行。车辆驶出 route 后必须从另一入口确定性随机回流，继续复用同一 Bevy proxy/model，同时获得新的 Runtime identity。若把这些职责分别塞进 `TrafficWorld::step`、Bevy example 或数据文件，会破坏现有确定性、引擎无关性和失败原子性。

## 决策

### 1. Population policy 完全由 caller 拥有

`laneflow-runtime` 不提供人口 controller，也不把目标数量、seed、portal/route catalog、pending recycle 或抽样策略纳入 TrafficWorld public API。Runtime 只提供第 2 节的通用原子 replace/remove commands。城市模拟游戏、headless host 或 reference scenario 可以各自拥有生命周期策略，并在两个 `step` 之间显式调用这些命令。

#203 / #475 只实现走廊 reference scenario 所需的 caller-owned 确定性策略；它不是 `TrafficWorld` 字段，不得成为 `TrafficWorld::step` 的隐藏状态，也不构成所有 LaneFlow 集成必须采用的默认人口模型。未来城市游戏可以完全替换该 policy，而无需修改 Runtime。

caller policy 拥有：

- 目标人口、seed、portal catalog、lane route catalog 与 pending-recycle 队列；
- 初始人口计划；
- route completion 到新 portal/lane/route 的确定性决策；
- blocked-entry 的稳定 retry 计划；
- 显式 PRNG state 和同版本 golden sequence。

caller policy 不拥有：

- TrafficWorld 车辆状态、occupancy、speed limit、Signals 或 route invariant；
- Bevy `Entity`、Transform、prefab/model 或 schedule 类型；
- LFCA / catalog 解析或文件系统路径。

### 2. TrafficWorld 提供原子 replace/recycle 与 remove commands

Runtime 提供通用 typed command：

```text
replace_completed_vehicle(old: VehicleHandle, input: VehicleSpawnInput)
  -> Result<VehicleReplaceRecord { old, new }, ReplaceError>
```

`ReplaceError::Blocked` 仅表示入口占用/重叠，可原样重放同一 `VehicleSpawnInput`。命令只允许在 `step` 之间执行，并遵循 compute-then-apply：

1. 验证 old handle 当前 live、状态为 Completed 且没有 Parking binding；
2. 验证 caller 提供的 profile、route handle、`route_edge_index`、初始位置、速度和 overlap；
3. 预检全部容量、准入与占用；
4. 一次提交旧结束与新开始；
5. 返回足以让调用方更新绑定和诊断的 old/new record。

到达路线终点时写成 `Completed`，**保留槽位与句柄**；不进 pose 批次、不再步进、不占车道占用，但占车辆容量。#301 的立刻退役不是现行回流路径。Runtime 没有 Core 那套 external ID 字符串，因此不把 `Preserve | ReplaceWith` 搬进 `TrafficWorld`。

#540 修订后，Runtime 同时保留真正移除车辆的独立原子命令（Rust 最终拼写由 #541
落定）：

```text
despawn_vehicle(vehicle: VehicleHandle)
  -> Result<VehicleDespawnRecord { vehicle, parking_release? }, DespawnError>
```

该命令接受所有 live lifecycle states。对 Reserved、Occupied/Parked vehicle，它在一次
compute-then-apply 中清除停车资源/count、反向 binding、route 引用、live order 与
vehicle identity；Active/Completed 走同一原子移除边界。任一 stale handle、状态不变量、
预留或算术失败都保持 Runtime 完全不变。成功后旧 handle 立即 stale。

`despawn_vehicle` 只表达“这个车辆现在确实不存在了”，用于设施拆除前清场、交通回收、
长期 Parked 清理和宿主明确删除。它不选择替代车辆、入口或 route，也不是 population
recycle 的第一步；需要保持人口并尝试新入口时，仍必须使用
`replace_completed_vehicle`，不能拼成可失败的 despawn + spawn 两步事务。

物理 overlap 是可重试的 typed `Blocked`，携带 old/blocker handle、前后关系和 bumper gap；其他 validation/invariant failure 返回致命 `ReplaceError`。任一失败结果都保持 `TrafficWorld` committed authority 不变。

成功后旧 `VehicleHandle` 立即 stale，新 vehicle 获得不同 generation 的 handle；public contract 不保证复用相同 slot index。replace 尽量保留旧车在 `live_order` 中的位置，不产生 tombstone；独立 spawn 仍追加到尾部。

Runtime command 不选择随机入口、lane 或 route，也不接触 Bevy Entity。

### 3. Adapter 承担 Runtime command 与宿主 binding 的组合事务

Engine Adapter 暴露 typed lifecycle 入口。调用前先验证：

- old handle 与 proxy Entity 存在且互相绑定（若已绑定）；
- Entity 仍存活、没有被另一 vehicle 占用；
- replacement command 输入可在 Runtime 侧完整预检；
- old/new 映射切换所需容量已准备。

全部预检成功后，Session 先提交 Runtime replace，再以不可失败的已预留路径把同一 Entity 从 old handle 切换到 new handle。未绑定车辆只提交 Runtime replacement。任一预检失败时 Runtime 与映射均不变。实现不得暴露一个可在 Runtime 成功后任意失败、从而留下 stale mapping 的公共两步调用协议。禁止公开「先 `despawn` 再 `spawn`」两步回流协议。

Completed vehicle 不产生 pose record；pending 期间 proxy 保留最后一次合法 Transform。成功 replace 后，下一次 presentation batch 使用 new handle 的入口 pose 更新同一 Entity。Adapter 不 despawn/respawn proxy 或 model。

对真正移除，Session 提供 typed despawn-and-unbind 组合事务：预检 handle ↔ Entity/池槽
映射和清理路径，提交 Runtime `despawn_vehicle` 后，以不可失败路径恰好一次移除映射并
销毁或回收宿主对象。未绑定车辆只提交 Runtime removal。virtual Parked 车辆可能已经无
pose、隐藏或在表现池中，但仍必须以 typed removal 清除映射；“本帧没有 pose”绝不是
despawn 信号。禁止对已绑定 handle 经 `world_mut()` 调 raw despawn 后再补删映射。

### 4. Lifecycle 决策绑定 fixed-step 边界

Population 决策按 fixed-step input sequence 运行，不按 outer-frame 次数运行：

```text
apply pending lifecycle commands
  -> TrafficWorld fixed step
  -> consume ordered Completed vehicles
  -> enqueue pending plans for the next lifecycle boundary
```

初始人口在第一个 TrafficWorld step 前建立。若一个 Bevy outer frame 运行多个 catch-up step，每个 step 之间仍使用相同顺序，因此 outer-frame 分块不会改变 Runtime/population 决策序列。Presentation 继续每个 outer frame 最多提交一次。

同一 boundary 按 pending insertion order 各尝试一次；入口阻塞只保留该计划到下一 boundary，不阻塞其他 pending plan，也不重新抽签。

### 5. v0.8 reference policy 的 seeded 随机性是显式输入

走廊 reference policy 使用项目自有的 `SplitMix64` 序列，不把 RNG 或 seed 带入 TrafficWorld。state 由 caller 提供的 `u64 seed` 直接初始化，零 seed 合法；`next_u64` 使用 SplitMix64 标准 state increment、xor-shift 和乘法常量。

有界抽样的 `bound` 与 draw `r` 都是 `u64`。先以 unsigned wrapping 语义计算 `threshold = bound.wrapping_neg() % bound`（等价于 `2^64 mod bound`），拒绝 `r < threshold` 的值，接受后返回 `r % bound`；不得使用有偏的直接 modulo，也不得依赖集合迭代顺序。

每个首次进入 pending 的 logical slot 固定消耗三个有界决策：

1. 从除刚驶出 portal 外的其余 5 个 portal 中均匀选择；
2. 从目标 portal 的 2 或 3 条 PortalLane 中均匀选择；
3. 对该 lane 的完整正整数 raw weights 执行 cumulative RouteChoice；单 choice 也不得跳过 draw。

blocked retry 不再消耗随机数。初始人口使用同一个 PRNG 对 stable spawn-slot catalog 执行确定性 Fisher–Yates permutation；同版本实现必须用 golden sequence 锁定 seed、draw order 与结果。

### 6. Runtime population 不进入 Traffic 或 ScenarioManifest

Traffic v0.8 只承载 immutable lane graph、Junction/Movement/ManeuverPath、routes、profiles、Signals、Parking 与 per-edge speed limit。SpatialPackage v0.1 继续承载中心线；ScenarioManifest v0.1 继续只配对 Traffic/Spatial bytes、size 和 digest。

目标人口、seed、portal catalog、initial spawn slots、pending queue、VehicleHandle 和 Entity 不写入这些制品。v0.8 authoring/startup config 可以生成 artifacts 与 engine-neutral runtime plan，但它不是新的 production Traffic family，也不能绕过 production loader。

### 7. 权威职责

| 关注点                                   | 权威层                       |
| ---------------------------------------- | ---------------------------- |
| vehicle state、identity、overlap、route  | TrafficWorld                 |
| 目标人口、seed、portal/lane/route 决策   | caller policy（走廊为 #475） |
| lane graph、限速、Signals 静态输入       | 共享静态路网修订             |
| 中心线和 pose sampling                   | Spatial                      |
| VehicleHandle/Entity 部分双射与 schedule | Adapter                      |
| proxy、model、Transform、灯具            | Adapter / Presentation       |
| 场景拓扑和无冲突 signal program          | Authoring/generator          |

## 后果

### 正面影响

- TrafficWorld 继续没有隐藏随机数、wall clock 或引擎类型；
- 走廊 reference policy 可用于 headless/Bevy 验证，同时未来游戏可完全替换；
- same proxy/new Runtime identity 与 stale-handle 语义同时成立；
- blocked entry 不会降低目标 logical population，也不会产生部分事务；
- outer-frame chunking 不改变 fixed-step population 决策；
- 共享静态路网与 catalog 保持静态制品职责，不混入 runtime snapshot。

### 成本与限制

- Runtime 和 Adapter 都增加 public typed lifecycle API；走廊 policy 不成为 TrafficWorld API；
- 走廊必须维护 PRNG golden sequence，算法变更会改变同版本 reference replay；
- Adapter 需要在一个 owner 内完成 Runtime/mapping 预检和提交，不能由 example 拼接松散调用；
- Runtime 与 Adapter 需要额外的 typed atomic removal record/transaction，停车释放信息
  不能靠调用后反查已经 stale 的 handle；
- pending Completed vehicle 会暂时保留 Runtime slot 和 proxy；
- 本 ADR 不提供保存/恢复完整 population controller state 的序列化格式。

## 被拒绝的方案

### 在 TrafficWorld::step 内自动 despawn/spawn

拒绝。它把需求策略和随机状态藏进交通 hot path，破坏显式 input sequence，并使多个 Adapter 难以共享生命周期控制。

### 原地重置同一 VehicleHandle

拒绝。它会让缓存的旧 handle 静默指向新的旅程，违反 ADR 0005 的 stale-handle/generation 契约。

### 由 Bevy example 直接调用 raw despawn 再 spawn 做回流

拒绝。它让 Bevy 成为回流规则 owner，无法用于 headless/其他引擎，也无法原子维护 Session 映射。该拒绝不禁止用于真正移除的 typed `despawn_vehicle`；它禁止的是用 raw 两步协议冒充原子回流。

### 先 despawn，再尝试 spawn

拒绝。入口阻塞或 spawn validation 失败会丢失 vehicle、降低人口，并留下 proxy 与 Runtime/Session 映射不一致。

### 把人口、seed 或 Entity 写入 Traffic/Manifest

拒绝。Traffic/Spatial/Manifest 是 immutable source artifacts，不是 runtime snapshot 或引擎 asset binding。

### 复用 Traffic v0.6

拒绝。Accepted Data design 已把 0.6 保留给曾经 no-go 的 f32 数值迁移；v0.8 的 per-edge speed limit 使用新的 0.7 target，避免同一版本号表达两种不兼容 shape。

## 兼容性

- TrafficWorld API：保留 caller-driven typed atomic replace，并新增/保留真正移除用的
  typed atomic `despawn_vehicle`；两者都属于 pre-1.0 public API change，不新增人口
  controller、车辆数量限制或 RNG。
- Adapter API：replace-and-rebind 与 despawn-and-unbind 都是 typed lifecycle/binding
  transaction，属于 pre-1.0 public API change。
- 静态路网：继续由共享修订安装；本 ADR 不切换 LFCA 形状。
- Determinism：Runtime replacement 保留 stable update order；走廊 seeded policy 的承诺范围继续是同一实现版本和运行环境，seed 是 caller policy 的显式 input/state。

## 验证要求

- old handle stale、new handle live、logical slot 人口不变；
- Runtime replace 的所有 validation failure 都保持 world 不变；
- Adapter 预检/提交失败不留下 stale 或双重映射；
- Active、Completed、Reserved、Occupied/Parked 的 atomic despawn；停车资源/count、route
  引用、live order 与 handle generation 一次闭合，所有失败零副作用；
- Adapter 对可见、隐藏和已池化 virtual Parked proxy 的 removal 都恰好一次清映射；无
  pose、park 或 leave 失败不能误触发 removal；
- pending proxy 保持最后 pose，成功回流复用同一 Entity；
- 走廊 policy 在相同 seed 和 fixed-step input sequence 下得到相同 initial/recycle decisions；
- 不同 outer-frame chunking 得到相同 Runtime/population state；
- 50/100/200、全部入口阻塞、部分恢复和多个 simultaneous completion；
- 稳定容量下 lifecycle command 不产生与全体 vehicle 数量成正比的临时分配。

## 关联

- G1 冻结：https://github.com/illusion-tech/laneflow/issues/184#issuecomment-5041612599
- 场景设计：[`../design/example-scenarios.md`](../design/example-scenarios.md)
- 下游实施：历史 #185、#186、#187、#188、#189、#203；现行 TrafficWorld 交付 #475。
