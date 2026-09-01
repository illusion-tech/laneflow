# 停车系统设计

**文档状态**: Accepted（#540 G1）<br>
**最后更新**: 2026-08-31（#541 clean-break 实现）<br>
**适用范围**: `ParkingFacility` / `ParkingSpace` 静态模型、显式与虚拟停车资源、
Traffic Runtime 生命周期、快照/修订切换、Spatial/Adapter 和复杂度边界<br>
**实现状态**: 当前实现已以 `ParkingFacility + ParkingSpace`、tagged
`ExplicitSpace | VirtualPool` 和私有稀疏 binding aggregate 完成 clean break；旧
`ParkingArea` / `occupy_parking` 入口、Runtime Snapshot v1 reader/writer 均不保留。<br>
**关联文档**:

- `../adr/0010-parking-binding-and-vehicle-lifecycle-authority.md`
- `../adr/0025-checked-canonical-network-and-shared-static-network.md`
- `traffic-runtime-shared-consumption.md`
- `traffic-runtime-snapshot.md`
- `traffic-runtime-revision-cutover.md`
- `traffic-runtime-integer-geometry.md`
- `shared-static-network.md`
- `network-compiler.md`
- `portable-canonical-artifact.md`
- `adapter-api.md`
- `chinese-style-city-workload.md`

## 1. 结论

停车只有一套设施模型：

```text
ParkingFacility
  ├─ 0..N 个显式 ParkingSpace
  └─ 0..virtual_capacity 个不可见容量单位
```

两部分可以同时存在。`ParkingSpace` 是有排他占用和 parked pose 的具体资源；
`ParkingFacility` 的 virtual pool 是不物化具体位置的计数资源。caller 必须精确选择
`ExplicitSpace` 或 `VirtualPool`，Runtime 不负责在地面泊位、室内车库或多个设施之间
替玩家/玩法层做策略选择。

虚拟停车不是销毁车辆、聚合车辆或隐藏路网：车辆仍是 live `Parked` identity，保留
route/profile/binding；只是退出 travel-lane 运行集合，并且不产生 committed pose。

## 2. 产品边界

### 2.1 首版必须支持

- 独立显式路侧/专用泊位；
- 只含显式泊位的设施；
- 只含虚拟容量的设施；
- 同一设施同时含显式泊位和虚拟容量；
- 虚拟池多个入口、多个出口以及入口=出口的合法场景；
- caller-selected reservation、到达、显式 park、cancel、leave、despawn、route rebind、
  初始/恢复 parked spawn；
- 快照、回放、同/跨修订恢复和失败原子性；
- 显式 parked pose 与虚拟 parked 无 pose；
- 声明容量与空容量单位数无关的稀疏 Runtime 状态。

### 2.2 明确不做

- 设施内部道路、寻位轨迹、楼层、倒车、排队车道或车库交通仿真；
- Runtime 自动选设施/泊位、最近匹配、随机分配、满位 reroute 或排队调度；
- 收费、营业时间、住户/访客权限、充电、vehicle-size fit、valet 或停车运营；
- 共享正常行车道上的双排停车或动态缩窄道路；
- 以 despawn/respawn 代替正常停车；
- 把 10k/100k 声明容量直接等同于 LFCA 某张表的行数；
- 为理论极限容量预留 slot、位图或容量等长数组。

这些能力只有形成实际产品闭环时才另开 G1；首版不为“也许以后会有”的车库内部模拟
预付复杂度。

## 3. 静态领域模型

### 3.1 `ParkingFacility`

来源语义形状如下；字段名用于冻结概念，不承诺最终 Rust/FlatBuffers 拼写：

```rust
struct ParkingFacilitySource {
    parking_facility_key: String,
    virtual_capacity: u32,
    virtual_entries: Vec<ParkingLaneAnchor>,
    virtual_exits: Vec<ParkingLaneAnchor>,
    provenance: SourceProvenance,
}
```

显式成员只由 `ParkingSpace.parking_facility` 正向引用声明。compiler 在引用解析和规范排序
后生成设施的 reverse member range；source 不再保存第二份 `parking_space_keys`，避免
双向事实漂移。

编译后共享静态语义形状：

```rust
struct ParkingFacilityStatic {
    stable_id: StableId128,
    explicit_spaces: RangeU32,
    virtual_capacity: u32,
    virtual_entries: RangeU32,
    virtual_exits: RangeU32,
}

struct ParkingFacilityAnchorStatic {
    lane_edge: LaneEdgeOrdinal,
    progress_mm: u32,
}
```

不保存 `total_capacity`；公开查询以
`u64::from(explicit_space_count) + u64::from(virtual_capacity)` 派生，避免为了不会发生的
极端组合压缩两个真实分量或额外限制 authoring。

### 3.2 `ParkingSpace`

```rust
struct ParkingSpaceStatic {
    stable_id: StableId128,
    parking_facility: Option<ParkingFacilityOrdinal>,
    entry: ParkingLaneAnchorStatic,
    exit: ParkingLaneAnchorStatic,
    geometry: ParkingSpaceGeometry,
}
```

显式泊位保持现行几何和 Spatial 语义。可选设施只用于组织、反向查询和统计，不参与
`ParkingSpace` 稳定身份。没有设施的泊位仍是合法 exact target。

### 3.3 编译不变量

对每个设施：

1. `explicit_spaces.len + virtual_capacity > 0`；
2. `virtual_capacity == 0` 当且仅当 virtual entry/exit 都为空；
3. `virtual_capacity > 0` 时 entry/exit 都非空；
4. 同一 entry 集或 exit 集内 `(edge StableId, progress_mm)` 不重复；
5. anchor edge 存在，且相对提交后边长 `L` 满足 `1 <= progress_mm <= L - 1`
   （两端各留 `1 mm`），并通过现行 route/access 静态闭合；
6. 每个 `ParkingSpace` 至多归属于一个设施；reverse range 与正向引用恰好互证；
7. total capacity 以 `u64` 派生，不因整数溢出回绕。

虚拟 entry/exit 按 `(LaneEdge StableId128, progress_mm)` 排序并从零生成 owner-local
ordinal。来源 vector 顺序不是语义；LFSM 仍保存各声明的来源位置。

#540 首版不扩展 `AccessRule` 的 target kind。车辆能否到达/离开设施由所选 anchor 所在
LaneEdge、动态 Route 和现行 participant-class access 闭合；设施级住户、收费、营业时间
等 policy 需要另开 G1。这样不把尚未存在的运营系统伪装成 Runtime parking authority。

### 3.4 静态制品与公共登记边界

`ParkingFacility`、virtual anchors、来源映射、语义差异、公共 registry 和静态制品
版本轴均直接沿用 `portable-canonical-artifact.md` 及其关联 ADR 的当前权威；本文不再
复制编号、版本值、表布局或容量上限。#541 只实现并消费该权威，不得从本停车文档
反向生成另一套登记。

停车域只补充一个跨层不变量：virtual anchor 是 owner-local occurrence，不获得全局
StableId。Runtime 同修订热状态可以保存 typed selector；快照和跨修订迁移必须保存
facility StableId 与精确 `(LaneEdge StableId, progress_mm)`，不能持久化 owner-local
ordinal，也不能用“设施 StableId 未变”掩盖 anchor 位移。

## 4. 运行时资源与唯一权威

### 4.1 精确停车目标

```rust
enum ParkingTarget {
    ExplicitSpace(ParkingSpaceOrdinal),
    VirtualPool(ParkingFacilityOrdinal),
}
```

`ParkingTarget` 是命令、binding、查询、快照和观察共同使用的 tagged semantic。不能用
特殊 ordinal、空 space、负数或单独布尔值编码 virtual target。

设施 aggregate 查询同时返回：

```text
explicit: capacity / reserved / occupied / vacant
virtual:  capacity / reserved / occupied / vacant
total:    capacity / reserved / occupied / vacant (checked derived)
```

调用方做 admission 时必须使用 target 对应的 pool。`total.vacant > 0` 不代表某个指定
pool 一定有资源，因此不得只公开一个含糊的 `available` 作为命令判断依据。

### 4.2 私有 aggregate

逻辑不变量：

```text
vehicle_binding[v] = None
  | Reserved {
      target,
      route,
      entry_route_occurrence,
      virtual_entry_selector: RequiredExactlyForVirtualPool
    }
  | Occupied { target }

explicit_state[s] = Vacant | Reserved(v) | Occupied(v)

virtual_state[f] = {
  reserved_count,
  occupied_count,
  sparse reserved/occupied vehicle membership
}
```

这些结构由一个 `ParkingRuntimeState` 私有 aggregate 拥有。`VehicleState` 可以只读暴露
tagged binding/status，但不是可独立修改的第二 authority。

`VehicleStatus` 与 parking binding 是两条正交但受约束的状态轴，合法组合只有：

| `VehicleStatus` | parking binding      | 语义                         |
| --------------- | -------------------- | ---------------------------- |
| `Active`        | `None` 或 `Reserved` | 正常道路车辆或正在驶向入口   |
| `Parked`        | `Occupied`           | 已由停车资源持有             |
| `Completed`     | `None`               | 已到路线终点，等待替换或移除 |

`Reserved` / `Occupied` 不是 `VehicleStatus`。Reserved binding 的 `route` 必须与
`VehicleState.route` 相同；Occupied 后车辆仍通过 `VehicleState` 保留 live route
reference。任何其他组合都是 aggregate invariant violation，不得被 snapshot、cutover 或
despawn 当作可修复输入。

虚拟设施按 `F` 保存 counts/ranges，按实际 binding `B` 保存稀疏成员；不得按
`virtual_capacity` 建 slot vector、bitset、free list 或伪 space handles。显式泊位保持
`O(S)` 排他状态，因为它们本来就是实际静态资源。

### 4.3 资源守恒

对每个显式泊位：

```text
vacant + reserved + occupied == 1
```

对每个虚拟设施：

```text
reserved_count == exact Reserved(VirtualPool(f)) member count
occupied_count == exact Occupied(VirtualPool(f)) member count
reserved_count + occupied_count <= virtual_capacity
```

对每辆车，vehicle binding 与资源反向 binding 恰好互证。counts 可增量维护，但每个
validation/snapshot/cutover 测试必须能从 canonical sparse members 重算并比较。

## 5. 生命周期命令

### 5.1 命令集合

语义 API（不冻结最终 Rust 拼写）：

```rust
reserve_parking(vehicle, reserve_target)
cancel_parking(vehicle, target)
park_vehicle(vehicle, target)
leave_parking(vehicle, leave_target)
rebind_parking_route(vehicle, rebind_target)
spawn_parked_vehicle(parked_input, target)
despawn_vehicle(vehicle)

enum ReserveParkingTarget {
    ExplicitSpace {
        space,
        entry_route_occurrence,
    },
    VirtualPool {
        facility,
        entry_anchor: VirtualEntryAnchorSelector,
        entry_route_occurrence,
    },
}

enum LeaveParkingTarget {
    ExplicitSpace {
        space,
        route,
        exit_route_occurrence,
    },
    VirtualPool {
        facility,
        route,
        exit_anchor: VirtualExitAnchorSelector,
        exit_route_occurrence,
    },
}

enum RebindParkingTarget {
    ExplicitSpace {
        space,
        new_route,
        new_current_route_occurrence,
        new_entry_route_occurrence,
    },
    VirtualPool {
        facility,
        new_route,
        new_current_route_occurrence,
        new_entry_anchor: VirtualEntryAnchorSelector,
        new_entry_route_occurrence,
    },
}

struct ParkedVehicleSpawnInput {
    profile,
    route,
    route_occurrence,
    progress_mm,
}
```

命令仅在 step 之间同步执行。caller 顺序就是提交和 replay 顺序；不建立隐式异步队列。

`VirtualEntryAnchorSelector` / `VirtualExitAnchorSelector` 是安装修订内、设施所有者局部的
typed selector；它必须解析到该设施对应集合中的一个精确
`(LaneEdgeOrdinal, progress_mm)`。route occurrence 只选择动态 route 中第几次经过该
LaneEdge，不能代替 anchor selector：同一个 LaneEdge 上可以合法存在多个不同
`progress_mm` 的入口或出口。

presence 规则是封闭的：virtual reserve/rebind 必须携带 entry selector，virtual leave
必须携带 exit selector；显式泊位命令禁止这些字段，因为泊位静态事实已唯一给出
entry/exit。park/cancel/despawn 消费既有 exact binding，parked spawn 直接构造
Occupied 且不发生道路边界穿越，因此也不虚构 selector。

Reserve 使用车辆当前 `VehicleState.route`；成功 record 与 Reserved binding 都回显/保存
该 route。Leave 必须显式携带要恢复的 route。Rebind 必须同时给出新 route 上与车辆当前
物理边对应的 current occurrence，以及该 route 上的 entry occurrence；只给 entry
occurrence 无法在 repeated-edge route 上无损迁移当前 cursor。`ParkedVehicleSpawnInput`
携带的是 Parked 状态必须保留的 retained route cursor，不是 entry/exit selector，也不授予
lane occupancy；命令固定提交 `speed_mm_s = 0`、`carry_um = 0` 和零 acceleration。

对 Reserved entry 定义唯一的前向可达谓词。设当前或映射后的 cursor 为
`(c_occ, c_progress_mm, c_carry_um)`，entry 为 `(e_occ, e_progress_mm)`：

```text
forward_reachable :=
  e_occ > c_occ
  || (e_occ == c_occ
      && (e_progress_mm > c_progress_mm
          || (e_progress_mm == c_progress_mm && c_carry_um == 0)))
```

`carry_um > 0` 表示车辆已越过同一整数毫米位置；此时相同 `progress_mm` 的 anchor 在物理
上已经位于车辆后方，不能因整数主值相等而接受。该谓词用于 reserve、rebind、snapshot
restore 与 cutover revalidation；不得退化为只比较 occurrence 或裸 `>= progress_mm`。

### 5.2 Reserve

Reserve 依次验证：

1. vehicle live、`VehicleStatus::Active` 且未绑定，target ordinal/kind 当前有效；
2. 显式 space Vacant，或虚拟 pool 的 `reserved + occupied < capacity`；
3. entry 属于 target：显式 target 必须匹配 space entry；虚拟 target 的显式 entry
   selector 必须由该设施拥有，并解析出唯一 exact anchor；
4. caller 指定的动态 route occurrence 必须是车辆当前 route 上该 anchor 所在 LaneEdge 的 exact
   occurrence；anchor 内的 `progress_mm` 只来自 selector，不能由 occurrence 或 facility
   猜测；entry 必须按 §5.1 谓词从当前 cursor 前向可达，route/class/access 必须合法；
5. route 引用容量和全部 checked arithmetic 可提交。

成功后立即消耗资源。binding 保存当前 route 与 exact entry occurrence；对虚拟 target
另保存所选 entry ordinal。同修订内使用 typed ordinal，snapshot 使用 semantic anchor。
Runtime 不在多个 entry 之间自动选一个，也不接受已经位于 committed cursor 后方的 entry。

### 5.3 Approach 与 arrival

Reserved Active vehicle 仍由 route/lane position 提供 authority。所选 entry 进入现行
ParkingStop/constraint/traversal 边界：车辆不能越过目标后仍保持未解释 reservation。

arrival 是 committed observation/query state，不新增可任意写入的 public `Arrived`
vehicle status。它只在以下五项同时成立时为真：

1. vehicle 与 target 是 exact Reserved pair；
2. vehicle 当前 route handle 仍是 reservation 绑定的 route；
3. 当前 `route_edge_index` 等于 reservation 保存的 exact entry occurrence；
4. 当前 `progress_mm` 精确等于所选 entry anchor 的整数毫米 `progress_mm`；
5. `speed_mm_s == 0` 且 `carry_um == 0`。

不得用距离容差、同一 LaneEdge 上任意 progress、零速但非零 carry 或“已经越过”近似
arrival。到达提交若由 ParkingStop 截停，必须一次把 occurrence/progress 归一到上述 exact
值并把 speed/carry 清零。信号、leader、waiting/conflict 和 ParkingStop 从同一
tick-start snapshot 归约 final admissible motion；数值 target 完全相同时，归因顺序沿用
`SignalStop -> ParkingStop -> RouteEnd`，只改变 observation attribution，不改变运动结果。
当 ParkingStop 与 RouteEnd 同值时，ParkingStop 归因意味着车辆保持
`Active + Reserved + Arrived`，不得同时提交 `Completed`；相同的是数值运动结果，不是
生命周期结果。合法 Reserved entry 始终前向可达，因此正常步进不存在
`Completed + Reserved` 或 route-completion 自动释放分支；若实现仍推导出该组合，整拍以
内部不变量错误失败关闭。

### 5.4 Park

`park_vehicle` 只接受 exact `Reserved(vehicle,target)` 且已经 committed arrival 的 pair。
成功原子提交：

- binding `Reserved -> Occupied`；
- vehicle `Active -> Parked`；
- 从 lane occupancy/leader/motion/traversal 执行集合移除；
- `speed_mm_s`、`carry_um` 和 acceleration 归零；
- 显式 target 的 position authority 切到 ParkingSpace geometry；
- 虚拟 target 的 position authority 变为“无 committed pose”。

route/profile/identity 保留。虚拟 park 不生成内部 route progress、随机 slot 或虚拟 world
transform。

### 5.5 Leave

显式 target 的 exit 由 space 静态事实确定；virtual leave 必须由 caller 显式携带设施
exit selector。调用方同时给出恢复 route 和 occurrence。Runtime 在仍为
`Parked + Occupied` 的提交态上验证：

- exact vehicle/target binding；
- virtual exit selector 属于 target，并解析出 exact anchor；route occurrence 的 LaneEdge
  精确匹配该 anchor，插入 progress 精确使用 selector 的整数毫米值；
- route/class/access 有效；
- 车辆在该 occurrence 的前后间隙满足下述 no-overlap/safe insertion 规则；
- 所有 handle、索引和算术可提交。

全部通过后才一次提交 `Parked -> Active`、用命令 route/occurrence/exit progress 替换 retained
route cursor、清零 motion、建立 lane authority、原子轮换 route reference 并释放 parking
resource。任何失败保持 binding、容量、vehicle status、retained route cursor、lane
occupancy 和 pose source 全部不变。

安全插入必须在车辆仍为 `Parked + Occupied` 时，先暂存一个位于 exact exit anchor 的
`Active` candidate；candidate 的 route/occurrence 来自命令，progress 来自静态 space
exit 或 virtual selector，speed/carry/acceleration 均为零。提交前必须同时满足：

1. 与全部已提交 Active lane occupants 无物理 overlap，覆盖 same edge、相邻 route
   boundary、车身跨边、repeated occurrence，以及 candidate/existing 双向 route
   visibility；
2. 对所有会把 candidate 视为新 stationary direct leader 的 Active follower，复用
   `vehicle-following.md` 的 emergency safe-speed 与 projection 判据，证明下一 tick 无需
   依赖 geometry hard projection 就能保持无重叠；
3. 静止 follower 只需通过物理几何检查；不额外要求 comfort `min_gap`，小于 comfort
   目标但 emergency-feasible 的 gap 由后续 Following 自然恢复；
4. 不借用当前 SignalStop、ParkingStop、Waiting/Conflict constraint 来放宽 gap，也不在
   leave 命令中修改 follower、tick 或事件。

第 2 项的 admission predicate 不是实现自行解释的“看起来够远”。对每个 direct follower，
先按 `vehicle-following.md` §11.2 的相同整数规则计算：

```text
g0_mm = max(0, route_aware_bumper_gap_mm)
preserved_gap_mm = min(g0_mm, follower.min_gap_mm)
raw_available_gap_mm = g0_mm - preserved_gap_mm
available_gap_mm = 0                 if raw_available_gap_mm <= 1
                   raw_available_gap_mm otherwise
```

这里的 `1` 是现行 `minimum_gap_tolerance = 1 mm`，不是新的 Parking tolerance。令 `v` 为
当前速度、`b_f` 为 emergency deceleration、`dt` 为 world fixed step、`s` 为 `g0_mm`
按 Following 同一规则转换的 SI bumper gap；candidate 的 leader speed 为零。先取 emergency
braking 在下一 tick 可达到的最低非负速度：

```text
u_min = max(0, v - b_f * dt)
```

再按 `vehicle-following.md` §11.3 求下一 tick 的 emergency minimum travel：

```text
emergency_min_travel = v^2 / (2 * b_f)                 if v <= b_f * dt
                       v * dt - 0.5 * b_f * dt^2       otherwise
```

只有以下两项都按 Following 相同量化/比较规则成立，且全部中间值 finite 时才允许 leave：

```text
0.5 * (v + u_min) * dt + u_min^2 / (2 * b_f) <= s
emergency_min_travel <= available_gap_mm / 1000
```

第一项是 §10.2 在 stationary leader 下的 safe-speed envelope，保证 emergency floor 不高于
safe-speed 上界；第二项显式扣除 §11.2 承诺保留的 gap，保证下一 tick 的 geometry cap 不会
把 emergency minimum travel 再压小。任一项不成立都返回 leave-unsafe-follower，而不是
允许下一 tick 用 geometry hard projection 补救。`v == 0` 时两项左侧都为零，因此静止
follower 只剩物理几何要求；`g0_mm <= min_gap_mm` 且 `v > 0` 时 available gap 为零，必须
拒绝。

生产查询可以用 edge-local/route-aware index 缩小候选，但必须保留 full-scan reference
oracle 对拍。多个 blocker 以稳定 live/update order 选择；物理 overlap 与
unsafe-follower 必须是两个可区分错误，不能统一为含糊的 `unsafe exit`。

### 5.6 Cancel、rebind、despawn 与 parked spawn

- cancel 只接受 exact Reserved pair，释放资源并删除 reservation/entry binding；车辆仍为
  Active，`VehicleState.route/cursor` 不变并继续持有该 live route reference。Occupied
  不能 cancel。
- rebind 只接受 `Active + Reserved`。`new_current_route_occurrence` 必须在 `new_route` 上解析
  到车辆当前 physical LaneEdge；命令保留当前 `progress_mm/carry_um/speed_mm_s`、length 和
  acceleration。Runtime 必须分别按旧 route/cursor 与 candidate 新 route/cursor 展开该车
  完整车身占用区间，并要求规范序列 `(physical LaneEdge, lo_mm, hi_mm)` 逐项完全相同。
  只匹配前缘所在边不够：只要车尾仍跨在一个或多个 predecessor occurrence 上，新旧
  predecessor footprint 不同就必须以 body-footprint-mismatch 失败，不能把车尾 teleport 到
  另一条支路；车身已完全进入当前边，或新旧 predecessor 物理区间完全相同，才可继续。
  通过 footprint equality 后，再按 §5.1 从映射 cursor 验证新 entry 前向可达及
  route/class/access。成功后一次替换 `VehicleState.route/current occurrence` 与 Reserved
  binding 的 route/entry/selected entry；virtual rebind 必须显式携带新的 entry selector，
  旧 route reference 的释放与新 route reference 的取得同一提交，全程不得暂时释放容量或
  产生无 route 的中间状态。
- `despawn_vehicle` 是真正移除 live vehicle 的独立原子命令，不是回流的“先删后建”。它
  接受全部 live `VehicleStatus`（`Active | Parked | Completed`），并按合法状态矩阵同步
  释放可选 `Reserved | Occupied` binding、资源/count、route 引用和 vehicle identity；stale
  handle 失败不能释放后来占用同槽位的车辆。
- parked spawn/restore 是专用构造入口：一次验证容量、target、profile、retained route
  cursor 与 route/class/access 后建立完整 `Occupied + Parked` invariant；retained cursor
  必须在 route/edge/progress 闭包内，但不进入 lane occupancy，也不要求等于任何 entry。
  这使 only-virtual 或多入口设施无需伪造“停车时采用哪个入口”。普通 park 不能借此跳过
  reservation。

### 5.7 幂等和错误分类

窄幂等只允许当前 committed state 能完整证明同一命令载荷的情形，返回显式
`NoChange`/等价结果，不能伪装成新提交：

| 命令    | 允许 `NoChange` 的唯一条件                                                                                                             |
| ------- | -------------------------------------------------------------------------------------------------------------------------------------- |
| reserve | 已存在完全相同的 Reserved payload：vehicle、target、route、entry occurrence，以及 virtual 时的 entry selector；资源反向 binding 也一致 |
| park    | 已是同一 vehicle/target 的 `Parked + Occupied` exact pair                                                                              |
| rebind  | 当前 vehicle route/current occurrence 与完整 Reserved payload 已逐字段等于请求的新 route/current occurrence/entry occurrence/selector  |

cancel 与 leave 成功后已删除证明旧命令的 binding，不能仅凭“现在未绑定/目标空闲”猜测曾经
成功，重复调用返回 not-reserved/not-occupied。parked spawn 创建新 identity，无 no-op；
despawn 的 stale handle 是错误，也不能当 no-op。不同车辆、不同 target 或任一 payload
字段不同均不得被幂等吞掉。

`NoChange` 判定只能在 vehicle/target/route/selector/occurrence 均解析并通过 kind、ownership、
edge 与数值闭包后执行；malformed 或 stale payload 不能因当前最终状态相似而被吞掉。
reserve/rebind 的成功 record、输入命令日志与 replay payload 都保存同一完整字段集。

至少区分：

- stale/unknown vehicle、route、facility、space；
- target kind mismatch、vehicle already bound、target bound by another vehicle；
- virtual capacity exhausted；
- entry/exit selector missing/not owned by target、route occurrence/anchor mismatch；
- entry behind cursor/not forward reachable、rebind current occurrence/physical edge mismatch、
  rebind body footprint mismatch；
- not reserved/not arrived/not occupied；
- leave physical overlap、leave unsafe follower；
- route referenced/cannot remove；
- arithmetic/configuration capacity exceeded。

错误分类必须足以让 caller 决定改目标、等待、改 route 或报告坏数据；不得把所有失败压成
`Unavailable`。

## 6. Tick、确定性与观察

- Parked vehicle 不进入 lane occupancy、leader、vehicle-following、signal/waiting/conflict
  projection、longitudinal motion 或 route traversal。
- Reserved Active vehicle仍正常参与上述道路行为，并受所选 entry 的 ParkingStop 约束。
- arrival 只按 §5.3 的 exact occurrence/progress/zero-speed/zero-carry 谓词成立；
  `SignalStop -> ParkingStop -> RouteEnd` 的同值归因顺序保持不变。
- arrival query 始终从 committed state 派生；successful step 只在 tick-start 为 false、提交后
  为 true 时产生一次 arrival observation。reserve/rebind 命令若直接建立 already-arrived
  committed pair，命令 result 回显该事实，但不补造延迟 step observation；后续 step 也不
  重复发送。
- command 在 step 边界线性化；step 内只读一个 tick-start committed snapshot，并一次提交。
- 合法 `NoChange` 仍是成功消费的一条输入命令，input command cursor checked `+1`；它不改变
  parking/vehicle authority、`observationStateSequence`、tick/time 或事件游标。cursor 耗尽
  时连 no-op 也必须零副作用失败，不能返回未计数成功。
- parking member/query/observation 按既有 live/stable vehicle order；不能暴露 hash order。
- 批量 despawn/restore/cutover 可以一次 canonical scan 处理 `B` 个 binding；不得对每个
  facility 再扫描全部 vehicle 形成 `O(F*V)`。
- counts 是提交时更新、测试中可重算的缓存。摘要必须覆盖 target tag、stable identity、
  binding state、车辆所属 route group、Reserved entry route occurrence、selected entry 和
  canonical member order。
- reserve result 必须回显 target、bound route、entry occurrence 与 virtual entry selector；
  leave result 必须回显 target、恢复 route、exit occurrence 与 virtual exit selector；rebind
  result 必须回显 old/new route、current/entry occurrence 与 virtual entry selector。不能只
  回显 facility 后要求 Adapter、snapshot 或 replay 侧重新猜 route/anchor。
- 不为 command 重新引入全局 event sequence。成功 lifecycle command 的 committed record
  按 command cursor/caller order 观察；step 只产生 arrival observation，按稳定 live order
  观察。cancel/leave/despawn 的资源释放由同步 typed record 表达，不产生延迟 release
  observation。同步 typed command result 与 step/observation cursor 按各自既有合同输出，
  不能把两条顺序轴混成延迟 backlog。

## 7. Snapshot、回放与修订切换

### 7.1 Runtime Snapshot v4

每个停车 binding 保存：

```text
vehicle snapshot-local id
binding state: Reserved | Occupied
target:
  ExplicitSpace { parking_space_stable_id }
  VirtualPool { parking_facility_stable_id }
Reserved additionally:
  bound route is the vehicle record's snapshot_route_id
  entry_route_edge_index within the bound dynamic route
Reserved VirtualPool additionally:
  selected_entry { lane_edge_stable_id, progress_mm }
```

显式 target 的 entry/exit 和所有 target 的 capacity/membership counts 不重复入档；恢复从
目标共享修订解析并从 binding 重建。若存档同时保存派生 count，reader 必须拒绝该未登记
字段，不能容忍第二 authority。

route 不在 parking binding 中重复编码：Reserved binding 所属 route 必须是同一 vehicle
record 的 `snapshot_route_id`，确定性摘要中的 route-group 也覆盖该关系。恢复顺序：
解析/版本检查 → 稳定 identity 解析 → target/anchor/route 闭合 → 状态矩阵与前向可达
检查 → 资源守恒检查 → 分配新 runtime handles → 构建完整 aggregate → 原子发布。任何
一步失败都没有部分 world。

### 7.2 精确回放

精确回放要求相同语义容量和相同 command input sequence。容量不同可能改变后续 reserve
成败，因此只允许恢复，不宣称跨容量精确 replay 等价。digest 至少覆盖 parking binding
tag、target StableId、state、Reserved entry route occurrence、selected entry 与 stable
vehicle order。

### 7.3 跨修订迁移

迁移先按 StableId 解析 target：

| 变化                                               | 处理                                                     |
| -------------------------------------------------- | -------------------------------------------------------- |
| facility/space 同身份且兼容                        | 继续验证并重绑                                           |
| virtual capacity 增大                              | 允许                                                     |
| virtual capacity 减小但仍 `>= reserved+occupied`   | 允许                                                     |
| virtual capacity 小于现有 binding                  | 整体失败                                                 |
| bound facility/space 缺失或 kind 改变              | 整体失败                                                 |
| Reserved explicit entry 在目标修订中仍前向可达     | 以目标 space 当前静态 entry 继续重绑                     |
| Reserved explicit entry 移到 committed cursor 后方 | 整体失败；不倒车、不 teleport、不自动改派                |
| Reserved virtual selected entry 缺失/位移          | 整体失败                                                 |
| Occupied virtual facility 更换 entry               | 不影响该 parked binding                                  |
| Occupied virtual facility 无合法 exit              | 静态候选本身拒绝；不能发布                               |
| bound explicit space 被移出设施                    | target 仍是同一 space 时可保留；设施报表归属随新修订更新 |
| bound explicit space 被删除                        | 整体失败                                                 |

显式 reservation 不把静态 entry 重复写进快照；跨修订时从同一 `ParkingSpace` 的目标静态
事实重新解析，再用 §5.1 谓词相对车辆 committed cursor 检查前向可达。entry 向前移动可以
继续接近；移到 cursor 后方则整个事务失败。virtual reservation 保存的是 caller 已选的
semantic entry，必须 exact 重绑，不应用上述“采用目标当前 entry”规则。设施 StableId
不含 capacity、anchors 或成员集合，因此调整这些业务事实不会无谓创建新设施 identity；
迁移策略负责判断活动 binding 是否仍兼容。

## 8. Spatial 与 Adapter

Runtime 输出 committed pose source 时：

```text
Active                    -> PoseSource::Lane
Parked(ExplicitSpace)     -> PoseSource::Parking(space)
Parked(VirtualPool)       -> no committed pose source
Completed / not presented -> according to their own existing contract
```

Spatial 只采样传入的 Lane/ParkingSpace source，不接受 `ParkingFacility` pose，也不从设施
entry 推断 parked transform。

Adapter 的表现状态机：

1. park 命令成功且 committed status 为 virtual Parked 后，隐藏或回收表现实体；
2. park 失败时保持原 lane 表现；
3. leave 失败时继续隐藏；
4. leave 成功并出现 committed lane pose 后，才创建/复用并显示表现实体；
5. 车辆 identity/binding 由 Runtime typed observation/query 获取，不能由“这一帧没有 pose”
   反推 despawn 或停车。
6. `despawn_vehicle` 成功后，无论实体当前可见、隐藏还是已回收到表现池，Adapter 都必须
   原子删除 Runtime handle ↔ 宿主实体/池槽映射，并把该结果解释为真正移除；virtual
   Parked 的无 pose 本身绝不能触发这条路径。

Adapter 可以自行选择对象池和视觉过渡，但不能延迟、回滚或提前宣告 Runtime 的资源提交。

## 9. 复杂度与资源账本

定义：

- `F`：设施数；
- `S`：显式泊位数；
- `A`：全部 virtual entry/exit anchor 数；
- `C`：声明 virtual capacity 总和；
- `B`：实际 Reserved/Occupied parking binding 数；
- `V_active`：道路 Active vehicle 数。

目标边界：

| 路径                    | 时间/空间边界                                                               |
| ----------------------- | --------------------------------------------------------------------------- |
| shared static retained  | `O(F + S + A)`，与 `C` 无关                                                 |
| 每世界 parking retained | `O(F + S + B)`，与空容量无关                                                |
| reserve/park/cancel     | ordinal 已解析后摊销 `O(1)`；anchor membership 可用有序 range 查找          |
| leave safety            | 一次扫描 `V_active` 并复用 route-aware occupancy 查询；不扫描 Parked 或 `C` |
| fixed tick              | `O(V_active + active constraints)`；不扫描 Parked 或 `C`                    |
| snapshot/cutover        | `O(V + B + routes)`，无 `O(C)` 展开                                         |

“100k 容量、100 辆实际 parked”只增加 100 个稀疏 binding；“100k 辆实际 parked”则需要
100k 个 live vehicle/binding，这是产品真实状态，不能也不应该伪装成常数内存。

当前 `parking_sparse_scale_evidence` 对只改变 virtual capacity 的 10k/100k 两个根给出
相同 shared-static retained 与相同单 binding 世界分配形状；该证据只闭合 #541 的稀疏
容量轴，不替代 #543 对 #304 exact topology、制品和加载峰值的核算。

#543 负责在 #304 exact topology 上测 declarations、各 IR、静态制品、共享静态路网和
build/load 峰值；本设计不复制或改写其格式容量上限，所有判断回到对应静态格式权威。

## 10. 验证矩阵

#541 至少提供以下 directed 证据；通用 broad tests 不能替代边界反例：

### 10.1 静态/compiler/LFCA

- only-explicit、only-virtual、mixed factory、standalone space；
- capacity 0 + no anchors 合法（仅当有 explicit member）；capacity 0 + anchors 拒绝；
- capacity > 0 的 entry/exit 缺失、重复、越界、unknown edge 拒绝；
- source 顺序扰动产生相同 canonical anchors、identity 和 LFCA；
- `ParkingArea` 被唯一生产入口拒绝；格式版本、公共 registry、未知字段与 verifier
  拒绝面直接复用各自 SSOT 的既有测试，不在 #540 复制一份登记；
- clean regeneration 与静态制品 → SharedNetworkRevision round-trip；
- 10k/100k virtual capacity 不生成对应 ParkingSpace/geometry/relation 行。

### 10.2 Runtime lifecycle

- virtual capacity `1` 的 exact/exact+1、两车竞争和 caller order；
- mixed facility 中 explicit pool 与 virtual pool 独立守恒；
- 多入口 reservation 精确绑定，错误入口/route occurrence 失败零副作用；
- 同一 LaneEdge 上两个不同 progress 的 virtual anchor 必须由 selector 区分；只给
  facility + route occurrence 的输入在类型/校验边界不可表达；
- arrival 必须覆盖 exact occurrence、exact integer-mm anchor、`speed_mm_s=0`、
  `carry_um=0` 四个维度的单项反例，并覆盖
  `SignalStop -> ParkingStop -> RouteEnd` exact-tie attribution；
- reserve/rebind 的前向可达反例覆盖 occurrence 在后、同 occurrence progress 在后、同
  `progress_mm` 但 `carry_um>0`，以及 exact anchor + zero carry；
- park 前未到达、重复 park、错误 target、stale handle；
- 多出口 leave、same-edge/相邻边/车身跨边/repeated occurrence overlap、移动 direct
  follower unsafe 与静止 follower、错误出口；direct follower 覆盖 gap 小于/等于/略高于
  `min_gap`、1 mm tolerance 两侧、safe-speed 通过但 preserved-gap geometry 不通过的反例；
  失败后仍 Parked/Occupied/无 pose；
- cancel、跨 route rebind 的 current occurrence 映射、车尾跨 predecessor 时不同 physical
  footprint 拒绝/相同 footprint 放行/车身完全进入当前边放行、三种 `VehicleStatus` 加可选
  Reserved/Occupied binding 的 despawn、route removal、explicit/virtual parked spawn；
- `NoChange` 逐字段反例：reserve/rebind 任一 route/occurrence/selector 改变必须失败而非
  no-op；cancel/leave/despawn/spawn 的重复调用不得被含糊吞掉；
- counts 从成员重算，stable member/observation order；
- Parked 不进入 occupancy/leader/motion/traversal。

### 10.3 Snapshot/replay/cutover

- explicit/virtual Reserved 与 Occupied save/load；
- same bytes + same input 的 digest/replay 等价；
- missing/wrong-kind target、duplicate binding、capacity overcommit 失败关闭；
- 状态矩阵、Reserved route ownership 与前向可达损坏失败关闭；
- capacity increase、safe decrease、unsafe decrease；
- reserved virtual selected entry 移除失败；reserved explicit entry 前移允许、移到车辆
  cursor 后方失败；occupied entry 变化按合同允许；
- zero-publish：任一 parking migration 失败不改变旧 world/root/cursors/Adapter。

### 10.4 Adapter/资源

- explicit Parked 仍有 space pose；virtual Parked 无 pose 但 vehicle 查询仍 live；
- 成功 park 后隐藏、失败 park 不隐藏；失败 leave 不显示、成功 leave 后显示；
- virtual Parked 无 pose 不触发 removal；成功 despawn 对可见/隐藏/已池化表现都恰好一次
  清除 Adapter 映射；失败 despawn 零副作用；
- 10k/100k 声明容量稀疏占用下无容量等长 allocation；
- 10k/100k 实际 parked 的 retained bytes 按车辆/binding 线性且无额外伪 space 放大。

## 11. 实施切片边界

- **#540 / G1**：本文、ADR、跨层影响、术语和 #304 停车 workload 合同已接受；不改
  production schema 或代码，也不把 Accepted 设计写成已经实现。
- **#541 / G2+**：当前 clean-break 实现消费静态制品权威，并同步
  Runtime/snapshot/cutover/Spatial/Adapter/API/fixtures/docs；定向证据按上节闭合。
- **#543 / research**：只做 exact capacity report；格式容量裁决留在对应独立权威。
- **#304**：消费已接受的停车切片，并继续分别登记信号、干支路、小区出口、公交/出租/
  路侧摩擦等其他域的支持状态；不能因为停车切片冻结就宣称整个 workload G1 完成。

当前实现报告仍必须区分已满足、依赖绑定和明确延后的验收项。#541 的停车切片不完成
#543 的 exact topology/容量核算，也不使 #304 的其他城市 workload 域自动达到 G1 或
Product Pass。
