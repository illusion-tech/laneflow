# ADR 0010：停车设施、资源绑定与车辆生命周期权威

**状态**: Accepted（#540 G1）<br>
**日期**: 2026-08-30<br>
**适用范围**: 停车静态实体、显式泊位与虚拟容量、Runtime binding/lifecycle、
快照/修订切换、Spatial/Adapter 表现边界<br>
**取代**: 本 ADR 的 v0.5 历史版本中“`ParkingArea` 只分组显式
`ParkingSpace`、拒绝匿名容量”的决定。pre-1.0 不保留旧/新两套权威；历史实现和判断
可从 git 历史追溯。<br>
**关联文档**:

- `0003-runtime-tick-and-determinism.md`
- `0005-core-identity-and-handle-model.md`
- `0008-pre-1.0-data-format-version-policy.md`
- `0025-checked-canonical-network-and-shared-static-network.md`
- `0028-integer-millimeter-traffic-geometry.md`
- `0029-retire-precompiled-static-route.md`
- `../design/parking-system.md`
- `../design/traffic-runtime-shared-consumption.md`
- `../design/traffic-runtime-snapshot.md`
- `../design/traffic-runtime-revision-cutover.md`
- `../design/adapter-api.md`
- `../design/chinese-style-city-workload.md`

## 背景

在 #540 决策时，旧实现把 `ParkingArea` 作为显式 `ParkingSpace` 的组织对象，容量由
成员泊位数派生；当时 `TrafficWorld` 也只能把车辆直接占到具体停车位。这适合能被玩家
看到的路侧或地面泊位，但不适合人车分流地下车库、建筑内部车库等不可观察设施。

对不可见设施逐容量物化泊位和内部 LaneGraph，会让“十万个容量单位”变成十万条静态
泊位记录、几何和关系，却没有对应的玩法或表现收益。反过来，如果简单把停车车辆销毁、
只保留一个聚合计数，车辆 identity、路线、存档、确定性和离场安全又会丢失。

#540 因此需要在 pre-1.0 阶段重新冻结唯一停车模型，并与 #541 的实现、#543 的真实
编译容量核算分开。用户已选择：

1. 以 `ParkingFacility` 完整取代 `ParkingArea`；
2. 同一设施可以同时拥有显式泊位和虚拟容量；
3. 虚拟设施支持多个入口和多个出口。

## 决策

### 1. `ParkingFacility` 是唯一设施实体

- `ParkingFacility` 完整取代 `ParkingArea`。生产 schema、公开 API、LFCA、共享静态
  路网、Runtime 和文档中不得长期并存两个设施种类，也不提供旧名 alias、兼容 façade
  或双写路径。
- `ParkingSpace` 继续表示有排他占用、可见静态几何和确定 parked pose 的具体泊位。
  它可以归属于一个 `ParkingFacility`，也可以作为独立路侧/专用泊位存在。
- `ParkingSpace` 的可选设施关系不参与泊位稳定身份；设施成员关系变化不应无谓改变
  泊位 `StableId128`。
- `ParkingFacility` 的稳定身份、公共 registry、静态格式版本轴、关系角色与来源/差异
  投影全部沿用 `portable-canonical-artifact.md` 及关联 ADR 的当前权威。本 ADR 不再复制
  编号、版本值或容量上限，避免停车语义反向成为第二套格式登记。

这不是兼容层：旧模型仍在 #541 一次性移除；同一现实设施能否跨修订重绑只按当前
StableId 与本 ADR §8 的活动 binding 兼容性判断。

### 2. 一个设施可以同时承载显式泊位与虚拟容量

`ParkingFacility` 的规范静态事实为：

```text
ParkingFacility
  StableId128
  explicit_spaces: canonical reverse membership
  virtual_capacity: u32
  virtual_entries: canonical non-empty set when virtual_capacity > 0
  virtual_exits: canonical non-empty set when virtual_capacity > 0
```

约束如下：

- `explicit_spaces.len + virtual_capacity > 0`；空壳设施拒绝编译。
- `virtual_capacity == 0` 时，不得声明 virtual entry/exit。
- `virtual_capacity > 0` 时，entry 和 exit 都至少一个；重复 anchor 拒绝。
- 设施总容量只用于查询、报表和工作负载计数：
  `total_capacity = explicit_spaces.len + virtual_capacity`。
- 显式泊位资源与虚拟池资源是两个 admission pool。占用显式泊位不消耗
  `virtual_capacity`；虚拟池满也不影响仍空闲的显式泊位。
- 显式泊位继续拥有自己的 entry、exit 和 geometry；设施的 virtual anchors 只服务
  虚拟池，不覆盖成员泊位的几何和接入事实。
- 不为虚拟容量生成内部 LaneEdge、Route、伪 `ParkingSpace` 或容量等长 slot 数组。

典型混合设施是：一个工厂共用同一外部道路入口，院内有可见地面泊位，建筑内另有
不可见停车容量。地面车辆必须选择具体 `ParkingSpace`；进入建筑内的车辆选择同一
`ParkingFacility` 的 virtual pool。两者可以引用同一 LaneEdge/progress 作为接入位置，
但资源与 parked pose 语义不同。

### 3. 虚拟入口/出口是多值语义 anchor

虚拟 anchor 由 `(LaneEdge StableId128, progress_mm)` 稳定表达；编译后使用 typed edge
ordinal 和 owner-local typed anchor ordinal。规范顺序按 edge stable identity、再按
`progress_mm` 排序。顺序与来源声明顺序无关；相同二元组是重复声明。

Runtime 热状态可以保存同修订 typed ordinal，快照和跨修订迁移必须保存/解析语义
anchor，不能持久化 owner-local ordinal。anchor 必须位于对应边界内并满足现行整数毫米
几何约束。

对 virtual pool，公开 reserve/rebind 命令必须显式携带 owner-local typed entry anchor
selector，leave 命令必须显式携带 exit anchor selector；selector 在安装修订中解析为
exact `(LaneEdgeOrdinal, progress_mm)`。动态 route occurrence 只说明 route 第几次经过
该 LaneEdge，不能替代 selector，因为同一 LaneEdge 上可以存在多个不同 progress 的
合法 anchor。显式泊位的 entry/exit 已由泊位静态事实唯一确定，因此禁止再携带 virtual
selector。

本切片不新增停车设施专属收费、营业时间、住户/访客权限或车辆尺寸策略。首版接入合法性
来自所选 anchor 的 LaneEdge/Route 和现行静态准入事实；若产品需要设施级 policy，另开
G1，不把未冻结运营规则塞进 `virtual_capacity`。

### 4. caller 精确选择停车目标，Runtime 不自动选地面或室内

公开语义使用 tagged target（Rust 拼写不冻结）：

```rust
enum ParkingTarget {
    ExplicitSpace(ParkingSpaceOrdinal),
    VirtualPool(ParkingFacilityOrdinal),
}
```

- caller 必须明确选择具体显式泊位或某设施的虚拟池。Runtime 不在二者之间自动选择，
  也不做最近泊位、随机分配、收费比较、满位 reroute 或排队调度。
- 对显式目标，每个 `ParkingSpace` 最多绑定一辆 live vehicle。
- 对虚拟目标，同一 `ParkingFacility` 可以绑定多辆 live vehicle，但
  `reserved_virtual + occupied_virtual <= virtual_capacity`。
- 每辆 live vehicle 最多有一个停车 binding；设施统计是同一私有 aggregate 的派生
  缓存，不是第二权威。

### 5. Runtime 私有 aggregate 拥有唯一资源权威

概念状态为：

```text
vehicle -> None
         | Reserved(ExplicitSpace, route, entry_route_occurrence)
         | Reserved(VirtualPool, route, entry_anchor_selector, entry_route_occurrence)
         | Occupied(ExplicitSpace | VirtualPool)

explicit space -> Vacant | Reserved(vehicle) | Occupied(vehicle)
virtual facility -> reserved_count + occupied_count + sparse vehicle membership
```

`ParkingRuntimeState` 是一个私有 aggregate；以上方向是同一提交事实的索引/缓存视图，
不能独立写入。实现不得为虚拟容量预分配 N 个空槽。虚拟成员只随实际 reservation/
parked vehicle 增长。

`VehicleState`、lane occupancy、静态路网、Adapter 和查询 snapshot 均不能直接修改
binding。公开 API 不提供 raw mutable map、`force_occupy` 或通用 setter。

`VehicleStatus` 与 binding 是两条正交状态轴。合法组合只有 `Active + None/Reserved`、
`Parked + Occupied`、`Completed + None`；Reserved binding 的 route 与车辆 route 必须相同。
`Reserved` / `Occupied` 不是 VehicleStatus，任何其他组合都必须失败关闭。

### 6. 生命周期、位置权威和失败原子性

正常生命周期是：

```text
Vacant capacity -> Reserved(vehicle) -> Occupied/Parked(vehicle) -> Vacant capacity
```

专用初始/恢复入口可以一次构造完整的 parked invariant，但不能绕开普通运行中的预约
规则。reservation 当即占用对应资源；无 TTL、隐式过期或自动替换。

- **reserve**：验证 live vehicle、精确 target、容量/排他性、路线 occurrence 和 entry。
  显式泊位的 entry 由泊位静态事实给出；虚拟池由 caller 通过显式 entry anchor selector
  在设施 entry 集合中选定。只有 facility + route occurrence 的输入不完整，必须拒绝。
  entry 还必须从车辆当前 exact route cursor 前向可达：晚 occurrence 合法；同 occurrence
  时 entry progress 必须更大，或 progress 相同且 `carry_um == 0`。同一整数毫米但
  `carry_um > 0` 已越过 anchor，必须拒绝。
- **arrival**：只在 exact Reserved pair、相同 route handle、exact entry occurrence、
  `progress_mm` 精确等于所选整数毫米 anchor、`speed_mm_s == 0` 且 `carry_um == 0` 时
  成立。不得用容差、同边任意位置或越过 anchor 近似到达。ParkingStop 提交到达时一次
  规范化 occurrence/progress 并清零 speed/carry；约束数值完全相同时继续按
  `SignalStop -> ParkingStop -> RouteEnd` 归因，同值顺序不改变实际运动。
  ParkingStop 与 RouteEnd 同值时保留 `Active + Reserved + Arrived`，不得同时提交
  `Completed`。合法 reservation 不存在 route-completion 自动释放；推导出
  `Completed + Reserved` 属内部不变量失败。
- **park**：只允许 exact reserved pair。在车辆已经合法到达所选 entry 后，一次提交
  `Active/lane authority -> Parked/parking binding`。Parked 保留 handle、external ID、
  profile、route 和 lifecycle identity，但不参与 lane occupancy、leader、motion、
  constraint 或 route traversal；`speed_mm_s`、`carry_um` 与 acceleration 归零。
- **leave**：显式目标使用泊位 exit；虚拟目标由 caller 通过显式 exit anchor selector
  在设施 exit 集合中选择。
  caller 必须同时给出恢复 route 与 exact occurrence。Runtime 在原
  `Parked + Occupied` 提交态上暂存 exact exit 的 zero-speed/zero-carry Active candidate，
  完整验证 route-aware physical overlap，以及所有会把 candidate 视为 stationary direct
  leader 的 Active follower 在下一 tick 不依赖 geometry hard projection 的 emergency
  可行性。静止 follower 只检查几何，不要求 comfort `min_gap`，也不借用当前信号/停车/
  waiting/conflict stop 放宽间隙。成功后一次提交 `Parked -> Active/lane authority` 并释放
  资源。精确 predicate 见停车设计 §5.5：除 stationary-leader safe-speed envelope 外，还
  必须按 Following 的 `preserved_gap = min(g0, follower.min_gap)` 与 1 mm tolerance 扣出
  `available_gap`，并证明下一 tick `emergency_min_travel <= available_gap`；否则返回
  unsafe-follower，不能让 geometry hard projection 补救。
- **cancel/rebind/route removal**：必须通过同一 aggregate 更新所有关联状态；virtual
  rebind 必须显式携带新的 entry selector；rebind 还必须给出新 route、车辆当前
  physical edge 在新 route 上的 exact current occurrence，以及新 entry occurrence。
  Runtime 保留车辆 progress/carry/speed，并要求旧/新 route 下展开的完整车身
  `(physical edge, lo_mm, hi_mm)` footprint 逐项相同；只匹配前缘 edge 不能防止跨
  predecessor 的车尾 teleport。通过后以映射 cursor 重做前向可达/准入验证，再一次替换
  车辆与 binding 的 route/occurrence。reserved vehicle 仍持有精确 route/entry 依赖，不能
  静默删除被引用 route。
- **despawn**：保留独立原子 `despawn_vehicle` 作为真正移除能力。它接受全部 live
  `VehicleStatus`；对带 Reserved binding 的 Active 或带 Occupied binding 的 Parked
  vehicle，在一次提交中释放资源/count、反向 binding、route 引用与 live identity；
  unbound Active/Completed 走同一移除边界。失败零副作用。它不是人口回流的“先删后建”，
  回流仍使用 ADR 0016 的原子 replace。

全部命令遵循 validate/compute 后一次提交。容量不足、目标冲突、stale handle、entry/exit
不匹配、路线不可达或出口不安全都必须零副作用失败；leave 失败不得先释放容量或生成
半提交位姿。

只允许能由当前 committed state 完整证明原命令 payload 的窄幂等：reserve 必须逐字段
匹配 vehicle/target/route/entry occurrence/virtual selector 与反向 binding；park 必须是
同一 exact `Parked + Occupied` pair；rebind 必须逐字段匹配新 route、current/entry
occurrence 与 selector。cancel/leave 成功后 binding 已不存在，不能猜测历史成功；parked
spawn/despawn 也无 no-op。不同车辆、target 或任一 payload 字段不同均不能被幂等吞掉。

### 7. 确定性与观察顺序

- 命令只在 step 边界同步执行，caller 调用/提交顺序是线性化和 replay 顺序；不恢复全局
  延迟 command queue 或事件 backlog。
- 成员查询、快照和观察按 Runtime 已冻结的 live/stable order 输出；批量清理按一次
  canonical scan 完成，不能依赖 hash iteration。
- `reserved_count` / `occupied_count` 是同一 aggregate 提交时维护并可由稀疏成员重算的
  受检缓存。它们不是与成员集合并行的第二 authority。
- 命令结果、已提交查询和事件/观测必须能区分 explicit/virtual target；Adapter 不从
  “有没有 pose”反推停车状态。

### 8. 快照、回放与在线修订切换

Runtime Snapshot 保存 tagged parking target 和 binding state：

- 显式目标保存 `ParkingSpace StableId128`；
- 虚拟目标保存 `ParkingFacility StableId128`；
- 所有 Reserved binding 保存其所属动态 route 内的精确 entry edge occurrence；
- 虚拟 reservation 另存所选 entry 的 semantic edge StableId/progress；显式 reservation
  的 semantic entry 从 `ParkingSpace` 静态事实解析；
- 车辆、route、状态和 command/event cursor 继续遵守快照总合同。

设施总数、容量和 counts 从目标修订与 binding 重建并闭合，不保存为独立 authority。
route 由同一 vehicle snapshot record 的 `snapshot_route_id` 承载，不在 parking binding
重复编码。恢复必须验证每个 target 存在、target kind 匹配、状态矩阵、Reserved route
ownership/前向可达、排他性和虚拟容量；任何失败整次恢复失败关闭。

跨修订切换按稳定 target identity 重绑：

- 显式泊位缺失或被多个车辆绑定：失败。
- 虚拟设施缺失：失败。
- `virtual_capacity` 增大：允许。
- `virtual_capacity` 减小：仅当现有 `reserved + occupied <= new capacity` 时允许。
- 已预约显式车辆从目标 `ParkingSpace` 当前静态 entry 重新解析 anchor；若它相对车辆
  committed cursor 仍前向可达则允许，若 entry 移到 cursor 后方则整次失败。
- 已预约虚拟车辆的 selected entry 必须在目标修订中仍有完全相同的 semantic anchor；
  否则失败。
- 已停入虚拟池的车辆不绑定旧 entry，但目标设施必须仍有合法 exit；离场使用目标修订
  当前 exit 集合。
- 移除有 binding 的显式泊位、把设施虚拟容量降为零但仍有虚拟 binding、或改变 target
  kind 都失败。

切换继续遵守 #302 的 prepare/validate/commit、zero-publish 和整事务失败；不得先迁一部分
车辆再回滚。

### 9. Spatial 与 Adapter 只消费已提交权威

- 显式 `Parked` 车辆继续产生 `PoseSource::Parking { space }`。
- 虚拟 `Parked` 车辆不进入 committed pose source 集合；Spatial 不为设施或容量生成
  伪 pose。
- 车辆仍是 live Runtime identity。Adapter 在 park 成功后才隐藏/回收表现对象，在 leave
  成功并获得 Active lane pose 后才重建/显示。失败命令不改变表现生命周期。
- pose 缺席不是 Runtime despawn，也不授予 Adapter 修改停车状态的权力。Adapter 必须
  结合 typed command/vehicle status 或 committed observation 区分 Parked、Completed、
  不可表现和真正移除。
- `despawn_vehicle` 成功是独立 typed removal：Adapter 必须原子删除 Runtime handle 与
  可见、隐藏或已池化宿主对象之间的映射；不得把 virtual Parked 的无 pose 当成 removal。

### 10. 复杂度和现实规模边界

设设施数 `F`、显式泊位数 `S`、虚拟 anchor 数 `A`、声明虚拟总容量 `C`、实际停车
binding 数 `B`：

- 静态 retained 上界是 `O(F + S + A)`，不是 `O(C)`；
- 每世界停车增量状态是 `O(F + S + B)`，不是 `O(C)`；
- tick 热路径不得扫描全部设施、泊位或声明容量；
- 10k/100k 容量但稀疏占用的设施不能出现容量等长分配；10k/100k 实际 live parked
  vehicle 的车辆和 binding 状态则是不可避免的 `O(B)`。

这里不把理论上的 `u32::MAX` 容量当产品验收重点。#541 应覆盖 0、1、exact、exact+1、
混合工厂、多入口/出口、冲突和稀疏 10k/100k；#543 只按实际 topology 逐层计数。格式
容量上限由其现有 SSOT 独立裁决，本 ADR 不复制或改写。

### 11. pre-1.0 API 与实现原子切换

#541 必须以一次 clean-break 交付唯一现行模型：

- 实现并消费当前静态制品/公共 registry 权威，不在停车实现里重新定义版本轴；
- Runtime Snapshot 原子切换到本文和快照设计冻结的停车 binding 形状；
- `ParkingArea` public symbol、旧停车入口和旧 snapshot reader/writer/schema 同切片移除；
- Runtime、Spatial 与 Adapter 同步切换，不留双 API、双读、双写或 alias。

仓库不发布迁移工具或双读期；测试 fixture 直接再生为新权威。

## 后果

正向后果：

- 可见泊位与不可见车库共享一套设施和 lifecycle authority，却不把容量物化为 N 个对象。
- 混合设施能直接表达现实常见的“地面可见 + 室内不可见 + 共用道路入口”。
- parked vehicle identity、存档、回放和离场安全不因表现隐藏而丢失。
- 资源、位置和 Adapter 表现的提交边界清晰，可对失败原子性和确定性做 directed tests。

代价与风险：

- `ParkingArea -> ParkingFacility`、tagged target、快照和 Runtime/Adapter API 都是
  breaking change；#541 必须同时更新 compiler、format、shared static、Runtime、
  Spatial/Adapter 和 fixtures，但静态格式登记只引用其 SSOT。
- 混合设施有两个独立资源池，查询必须同时给出 explicit/virtual/total，不能只暴露一个
  容易误解的 available 数。
- 多入口/出口使 reservation 和 cutover 必须保存 semantic anchor，并增加 route occurrence
  与错误面的验证。
- 实际 parked 车辆仍需逐车状态；本设计只消除不可观察的空容量物化，不把 live vehicle
  聚合成一个计数。

## 拒绝的替代方案

### 保留 `ParkingArea` 并新增平行 `ParkingFacility`

会产生两个设施身份、两套 membership/API/wire 以及长期转换规则。pre-1.0 没有为历史弯路
付兼容成本的理由，因此拒绝。

### 设施只能二选一：显式泊位或虚拟容量

会把共用地址、出入口和报表身份的地面/室内停车人为拆成两个设施，或迫使 caller 再建
上层聚合。现实中混合停车普遍且实现成本可控，因此拒绝。

### 虚拟容量展开成伪 `ParkingSpace`

保留了旧代码形状，却让静态行、关系、几何占位和每世界数组继续与声明容量线性增长，
也伪造不可见 pose，违背本议题目的。

### 为不可见车库生成内部 LaneGraph

只有内部驾驶、寻位、排队或可见表现成为产品需求时才有收益。当前玩家不可观察内部过程，
因此拒绝把完整道路模拟成本预付给没有玩法价值的区域。

### park 时销毁车辆，只保留设施计数

会打断 handle/external identity、route、存档和 Adapter 连续性，也无法精确处理离场、despawn
和修订切换，因此拒绝。

### Runtime 自动决定显式泊位或虚拟池

选择涉及玩法、收费、任务、车辆类型和 UI 意图，不是 Runtime 的通用交通权威。caller
必须传入 exact target；以后若需要调度器，应作为 Runtime 之外的策略层独立设计。

## 实施与复核

- #540：本 ADR、停车设计、跨层合同、术语和工作负载停车切片；只形成 G1 合同，
  不声称 schema/Runtime 已实现。
- #541：在 #540 G1 Accepted 后完成 clean-break production 实现与验证。
- #543：按虚拟停车后的 exact topology 记录 10k/100k 各层计数；不得从车辆数或停车
  容量直接推导某张静态表行数，也不得在本 ADR 旁路改写格式上限。
- #304：消费本模型形成中国特色城市 workload；其他交通域仍按各自 G1/实现状态分层。

如果以后要模拟设施内部道路、排队、分层、收费、充电、步行换乘或设施级准入，应按真实
产品闭环新增 G1。不得用这些尚未发生的扩展推翻当前稀疏虚拟池，也不得提前为理论极限
引入容量等长结构。
