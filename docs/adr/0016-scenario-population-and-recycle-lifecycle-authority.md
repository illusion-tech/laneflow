# 0016 Scenario Population 与车辆回流生命周期权威

**状态**: Accepted<br>
**日期**: 2026-07-22<br>
**适用范围**: v0.8 Signalized Corridor MVP 的 engine-neutral population、Core recycle command、seeded 回流、Bevy proxy 复用与启动配置边界

## 背景

v0.7 已交付固定步长 Core、动态 vehicle handle、Traffic/Spatial/Scenario 制品、Signals、Vehicle Following 与 Bevy Reference Adapter，但持续运行的参考场景还缺少统一的车辆人口和回流协议：

- `CoreWorld::spawn_vehicle` 与 `despawn_vehicle` 分别原子，但不能把已完成车辆替换为新 identity；
- `VehicleHandle` 必须在 despawn 后失效，不能把同一 handle 重置到另一条 route；
- `LaneFlowSession` 只公开 Core 只读视图以及 Vehicle/Entity 映射，没有 raw Core 可变入口；
- Completed vehicle 不再进入 pose batch，因此已绑定 proxy 会保留最后一次合法 Transform；
- Traffic Data 不持久化 initial vehicles、spawn schedule、runtime handles 或 Adapter metadata；
- Core fixed step 不读取 wall clock、全局随机数或引擎状态。

v0.8 要求 50–200 辆车持续运行。车辆驶出 route 后必须从另一入口确定性随机回流，继续复用同一 Bevy proxy/model，同时获得新的 Core identity。若把这些职责分别塞进 Core step、Bevy example 或数据文件，会破坏现有确定性、引擎无关性和失败原子性。

## 决策

### 1. Population policy 是 caller-owned engine-neutral 组件

`laneflow-core` 拥有 engine-neutral population 模块及其公共领域类型，并提供 caller-owned 的 `PopulationController` 或等价命名组件。该 controller 不是 `CoreWorld` 字段，不得成为 `CoreWorld::step` 的隐藏状态；v0.8 不为 population 新增同层 crate，也不把它下放给任何 Engine Adapter。

该组件拥有：

- 目标人口、seed、portal catalog、lane route catalog 与 pending-recycle 队列；
- 初始人口计划；
- route completion 到新 portal/lane/route 的确定性决策；
- blocked-entry 的稳定 retry 计划；
- 显式 PRNG state 和同版本 golden sequence。

它不拥有：

- Core vehicle state、occupancy、speed limit、Signals 或 route invariant；
- Bevy `Entity`、Transform、prefab/model 或 schedule 类型；
- Traffic/Spatial JSON 解析或文件系统路径。

### 2. Core 提供原子 replace/recycle command

Core 增加通用 typed command，语义接近：

```text
replace_vehicle(old_handle, replacement_input)
  -> VehicleReplaceRecord { old, new }
```

命令只允许在 `step` 之间执行，并遵循 compute-then-apply：

1. 验证 old handle 当前 live 且状态为 Completed；
2. 验证 replacement external ID、profile、route、初始位置、速度和 overlap；
3. 预留全部 registry、update-order、route-reference 与 command-spatial 变更；
4. 一次提交 old despawn 和 new spawn；
5. 返回足以让调用方更新绑定和诊断的 old/new record。

失败时 `CoreWorld` 完全不变。成功后旧 `VehicleHandle` 立即 stale，新 vehicle 获得不同 generation 的 handle；public contract 不保证复用相同 slot index。v0.8 的 logical population slot 可以复用相同 external ID，但只能由 replace command 在同一事务内完成 duplicate-ID 预检和替换。

Core command 不选择随机入口、lane 或 route，也不接触 Bevy Entity。

### 3. Adapter 承担 Core command 与宿主 binding 的组合事务

Engine Adapter 暴露 typed lifecycle 入口，不公开 `&mut CoreWorld`。调用前先验证：

- old handle 与 proxy Entity 存在且互相绑定；
- Entity 仍存活、没有被另一 vehicle 占用；
- replacement command 输入可在 Core 侧完整预检；
- old/new 映射切换所需容量已准备。

全部预检成功后，Session 先提交 Core replace，再以不可失败的已预留路径把同一 Entity 从 old handle 切换到 new handle。任一预检失败时 Core 与映射均不变。实现不得暴露一个可在 Core 成功后任意失败、从而留下 stale mapping 的公共两步调用协议。

Completed vehicle 不产生 pose record；pending 期间 proxy 保留最后一次合法 Transform。成功 replace 后，下一次 presentation batch 使用 new handle 的入口 pose 更新同一 Entity。Adapter 不 despawn/respawn proxy 或 model。

### 4. Lifecycle 决策绑定 fixed-step 边界

Population 决策按 fixed-step input sequence 运行，不按 outer-frame 次数运行：

```text
apply pending lifecycle commands
  -> Core fixed step
  -> consume ordered completion events
  -> enqueue pending plans for the next lifecycle boundary
```

初始人口在第一个 Core step 前建立。若一个 Bevy outer frame 运行多个 catch-up step，每个 step 之间仍使用相同顺序，因此 outer-frame 分块不会改变 Core/population 决策序列。Presentation 继续每个 outer frame 最多提交一次。

同一 boundary 按 pending insertion order 各尝试一次；入口阻塞只保留该计划到下一 boundary，不阻塞其他 pending plan，也不重新抽签。

### 5. Seeded 随机性是显式输入

v0.8 使用项目自有的 `SplitMix64` 序列，不新增 runtime RNG 依赖。state 由 caller 提供的 `u64 seed` 直接初始化，零 seed 合法；`next_u64` 使用 SplitMix64 标准 state increment、xor-shift 和乘法常量。

有界抽样的 `bound` 与 draw `r` 都是 `u64`。先以 unsigned wrapping 语义计算 `threshold = bound.wrapping_neg() % bound`（等价于 `2^64 mod bound`），拒绝 `r < threshold` 的值，接受后返回 `r % bound`；不得使用有偏的直接 modulo，也不得依赖集合迭代顺序。

每个首次进入 pending 的 logical slot 固定消耗两次有界决策：

1. 从除刚驶出 portal 外的其余 5 个 portal 中均匀选择；
2. 从目标 portal 的 2 或 3 条 lane routes 中均匀选择。

blocked retry 不再消耗随机数。初始人口使用同一个 PRNG 对 stable spawn-slot catalog 执行确定性 Fisher–Yates permutation；同版本实现必须用 golden sequence 锁定 seed、draw order 与结果。

### 6. Runtime population 不进入 Traffic 或 ScenarioManifest

Traffic v0.7 继续只承载 immutable lane graph、routes、profiles、Signals、Parking 与 per-edge speed limit。SpatialPackage v0.1 继续承载中心线；ScenarioManifest v0.1 继续只配对 Traffic/Spatial bytes、size 和 digest。

目标人口、seed、portal catalog、initial spawn slots、pending queue、VehicleHandle 和 Entity 不写入这些制品。v0.8 authoring/startup config 可以生成 artifacts 与 engine-neutral runtime plan，但它不是新的 production Traffic family，也不能绕过 production loader。

### 7. 权威职责

| 关注点                                   | 权威层                        |
| ---------------------------------------- | ----------------------------- |
| vehicle state、identity、overlap、route  | Core                          |
| 目标人口、seed、portal/lane/route 决策   | engine-neutral population     |
| lane graph、限速、Signals 静态输入       | Traffic Data / Core normalize |
| 中心线和 pose sampling                   | Spatial                       |
| VehicleHandle/Entity 部分双射与 schedule | Adapter                       |
| proxy、model、Transform、灯具            | Adapter / Presentation        |
| 场景拓扑和无冲突 signal program          | Authoring/generator           |

## 后果

### 正面影响

- Core 继续没有隐藏随机数、wall clock 或引擎类型；
- 同一 population policy 可用于 headless、Bevy 和未来 Adapter；
- same proxy/new Core identity 与 stale-handle 语义同时成立；
- blocked entry 不会降低目标 logical population，也不会产生部分事务；
- outer-frame chunking 不改变 fixed-step population 决策；
- Traffic/Spatial/Manifest 保持静态制品职责，不混入 runtime snapshot。

### 成本与限制

- Core 和 Adapter 都增加 public typed lifecycle API；
- v0.8 必须维护 PRNG golden sequence，算法变更会改变同版本 replay；
- Adapter 需要在一个 owner 内完成 Core/mapping 预检和提交，不能由 example 拼接松散调用；
- pending Completed vehicle 会暂时保留 Core slot 和 proxy；
- 本 ADR 不提供保存/恢复完整 population controller state 的序列化格式。

## 被拒绝的方案

### 在 Core step 内自动 despawn/spawn

拒绝。它把需求策略和随机状态藏进交通 hot path，破坏显式 input sequence，并使多个 Adapter 难以共享生命周期控制。

### 原地重置同一 VehicleHandle

拒绝。它会让缓存的旧 handle 静默指向新的旅程，违反 ADR 0005 的 stale-handle/generation 契约。

### 由 Bevy example 直接调用 raw CoreWorld

拒绝。它让 Bevy 成为回流规则 owner，无法用于 headless/其他引擎，也无法原子维护 Session 映射。

### 先 despawn，再尝试 spawn

拒绝。入口阻塞或 spawn validation 失败会丢失 vehicle、降低人口，并留下 proxy 与 Core 不一致。

### 把人口、seed 或 Entity 写入 Traffic/Manifest

拒绝。Traffic/Spatial/Manifest 是 immutable source artifacts，不是 runtime snapshot 或引擎 asset binding。

### 复用 Traffic v0.6

拒绝。Accepted Data design 已把 0.6 保留给曾经 no-go 的 f32 数值迁移；v0.8 的 per-edge speed limit 使用新的 0.7 target，避免同一版本号表达两种不兼容 shape。

## 兼容性

- Core API：新增 typed replace/recycle command，属于 pre-1.0 public API change。
- Adapter API：新增 typed lifecycle/binding transaction，属于 pre-1.0 public API change。
- Traffic：v0.8 目标由 current v0.5 breaking 迁移到 v0.7；本 ADR 本身不切换 production loader。
- SpatialPackage/ScenarioManifest：继续使用 v0.1 shape。
- Determinism：承诺范围继续是同一实现版本和运行环境；seeded population 成为显式 input/state。

## 验证要求

- old handle stale、new handle live、logical external ID 可复用；
- Core replace 的所有 validation failure 都保持 world 不变；
- Adapter 预检/提交失败不留下 stale 或双重映射；
- pending proxy 保持最后 pose，成功回流复用同一 Entity；
- 相同 seed 和 fixed-step input sequence 得到相同 initial/recycle decisions；
- 不同 outer-frame chunking 得到相同 Core/population state；
- 50/100/200、全部入口阻塞、部分恢复和多个 simultaneous completion；
- 稳定容量下 lifecycle command 不产生与全体 vehicle 数量成正比的临时分配。

## 关联

- G1 冻结：https://github.com/illusion-tech/laneflow/issues/184#issuecomment-5041612599
- 场景设计：[`../design/example-scenarios.md`](../design/example-scenarios.md)
- 下游实施：#185、#186、#187、#188、#189；v0.8 收口 #195 / Parent #193。
