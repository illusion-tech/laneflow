# 停车系统设计

**文档状态**: Accepted（#540 G1）<br>
**最后更新**: 2026-08-30<br>
**适用范围**: `ParkingFacility` / `ParkingSpace` 静态模型、显式与虚拟停车资源、
Traffic Runtime 生命周期、快照/修订切换、Spatial/Adapter 和复杂度边界<br>
**实现状态**: 本文是 #541 的唯一实现输入。当前 `main` 仍是
`ParkingArea + ParkingSpace + occupy_parking` 的已交付路径；在 #541 完成前，本文的
`ParkingFacility`、virtual capacity、reserve/park/leave 和无 pose 停驻均未生产化。<br>
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
spawn_parked_vehicle(vehicle_input, target, dormant_route)
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
        exit_route_occurrence,
    },
    VirtualPool {
        facility,
        exit_anchor: VirtualExitAnchorSelector,
        exit_route_occurrence,
    },
}

enum RebindParkingTarget {
    ExplicitSpace {
        space,
        new_entry_route_occurrence,
    },
    VirtualPool {
        facility,
        new_entry_anchor: VirtualEntryAnchorSelector,
        new_entry_route_occurrence,
    },
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

### 5.2 Reserve

Reserve 依次验证：

1. vehicle live、未绑定，target ordinal/kind 当前有效；
2. 显式 space Vacant，或虚拟 pool 的 `reserved + occupied < capacity`；
3. entry 属于 target：显式 target 必须匹配 space entry；虚拟 target 的显式 entry
   selector 必须由该设施拥有，并解析出唯一 exact anchor；
4. caller 指定的动态 route occurrence 必须是该 anchor 所在 LaneEdge 的 exact
   occurrence；anchor 内的 `progress_mm` 只来自 selector，不能由 occurrence 或 facility
   猜测；route/class/access 必须合法；
5. route 引用容量和全部 checked arithmetic 可提交。

成功后立即消耗资源。对虚拟 target，binding 保存所选 entry ordinal；同修订内使用 typed
ordinal，snapshot 使用 semantic anchor。Runtime 不在多个 entry 之间自动选一个。

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
- 车辆在该 occurrence 的前后间隙满足现行 no-overlap/safe insertion 规则；
- 所有 handle、索引和算术可提交。

全部通过后才一次提交 `Parked -> Active`、建立 lane authority 并释放 parking resource。
任何失败保持 binding、容量、vehicle status、lane occupancy 和 pose source 全部不变。

### 5.6 Cancel、rebind、despawn 与 parked spawn

- cancel 只接受 exact Reserved pair，释放资源并解除 entry/route 依赖；Occupied 不能 cancel。
- rebind 只改变 Reserved pair 的 route occurrence/selected entry；virtual rebind 必须显式
  携带新的 entry selector，并按一次事务验证，不得暂时释放容量。
- `despawn_vehicle` 是真正移除 live vehicle 的独立原子命令，不是回流的“先删后建”。它
  接受 Active/Completed/Reserved/Occupied/Parked；停车侧对 Reserved/Occupied/Parked
  通过 aggregate 同步释放资源、反向 binding、route 引用和 vehicle identity；stale
  handle 失败不能释放后来占用同槽位的车辆。
- parked spawn/restore 是专用构造入口：一次验证容量、target、route 和 vehicle 后建立
  完整 Occupied+Parked invariant；普通 park 不能借此跳过 reservation。

### 5.7 幂等和错误分类

窄幂等只覆盖“同一 live vehicle + 同一 exact target + 已经是请求的最终状态”。实现需要
返回显式 `NoChange`/等价结果，不能伪装成新提交。

至少区分：

- stale/unknown vehicle、route、facility、space；
- target kind mismatch、vehicle already bound、target bound by another vehicle；
- virtual capacity exhausted；
- entry/exit selector missing/not owned by target、route occurrence/anchor mismatch；
- not reserved/not arrived/not occupied；
- unsafe exit insertion；
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
- command 在 step 边界线性化；step 内只读一个 tick-start committed snapshot，并一次提交。
- parking member/query/observation 按既有 live/stable vehicle order；不能暴露 hash order。
- 批量 despawn/restore/cutover 可以一次 canonical scan 处理 `B` 个 binding；不得对每个
  facility 再扫描全部 vehicle 形成 `O(F*V)`。
- counts 是提交时更新、测试中可重算的缓存。摘要必须覆盖 target tag、stable identity、
  binding state、Reserved entry route occurrence、selected entry 和 canonical member order。
- virtual reserve/rebind/leave 的 typed command result 必须回显 caller 提供且已解析成功的
  entry/exit selector 与 exact route occurrence；不能只回显 facility 后要求 Adapter 或
  replay 侧重新猜 anchor。
- 不为 command 重新引入全局 event sequence。成功 lifecycle command 的 committed record
  按 command cursor/caller order 观察；step 产生的 arrival/release observation 按稳定 live
  order 观察。同步 typed command result 与 step/observation cursor 按各自既有合同输出，
  不能把两条顺序轴混成延迟 backlog。

## 7. Snapshot、回放与修订切换

### 7.1 Runtime Snapshot v2

每个停车 binding 保存：

```text
vehicle snapshot-local id
binding state: Reserved | Occupied
target:
  ExplicitSpace { parking_space_stable_id }
  VirtualPool { parking_facility_stable_id }
Reserved additionally:
  entry_route_edge_index within the bound dynamic route
Reserved VirtualPool additionally:
  selected_entry { lane_edge_stable_id, progress_mm }
```

显式 target 的 entry/exit 和所有 target 的 capacity/membership counts 不重复入档；恢复从
目标共享修订解析并从 binding 重建。若存档同时保存派生 count，reader 必须拒绝该未登记
字段，不能容忍第二 authority。

恢复顺序：解析/版本检查 → 稳定 identity 解析 → target/anchor/route 闭合 → 资源守恒
检查 → 分配新 runtime handles → 构建完整 aggregate → 原子发布。任何一步失败都没有部分
world。

### 7.2 精确回放

精确回放要求相同语义容量和相同 command input sequence。容量不同可能改变后续 reserve
成败，因此只允许恢复，不宣称跨容量精确 replay 等价。digest 至少覆盖 parking binding
tag、target StableId、state、Reserved entry route occurrence、selected entry 与 stable
vehicle order。

### 7.3 跨修订迁移

迁移先按 StableId 解析 target：

| 变化                                             | 处理                                                     |
| ------------------------------------------------ | -------------------------------------------------------- |
| facility/space 同身份且兼容                      | 继续验证并重绑                                           |
| virtual capacity 增大                            | 允许                                                     |
| virtual capacity 减小但仍 `>= reserved+occupied` | 允许                                                     |
| virtual capacity 小于现有 binding                | 整体失败                                                 |
| bound facility/space 缺失或 kind 改变            | 整体失败                                                 |
| Reserved virtual selected entry 缺失/位移        | 整体失败                                                 |
| Occupied virtual facility 更换 entry             | 不影响该 parked binding                                  |
| Occupied virtual facility 无合法 exit            | 静态候选本身拒绝；不能发布                               |
| bound explicit space 被移出设施                  | target 仍是同一 space 时可保留；设施报表归属随新修订更新 |
| bound explicit space 被删除                      | 整体失败                                                 |

设施 StableId 不含 capacity、anchors 或成员集合，因此调整这些业务事实不会无谓创建新
设施 identity；迁移策略负责判断活动 binding 是否仍兼容。

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

| 路径                    | 时间/空间边界                                                      |
| ----------------------- | ------------------------------------------------------------------ |
| shared static retained  | `O(F + S + A)`，与 `C` 无关                                        |
| 每世界 parking retained | `O(F + S + B)`，与空容量无关                                       |
| reserve/park/cancel     | ordinal 已解析后摊销 `O(1)`；anchor membership 可用有序 range 查找 |
| leave safety            | 复用 edge-local/route-aware 候选，不扫描全部车辆                   |
| fixed tick              | `O(V_active + active constraints)`；不扫描 Parked 或 `C`           |
| snapshot/cutover        | `O(V + B + routes)`，无 `O(C)` 展开                                |

“100k 容量、100 辆实际 parked”只增加 100 个稀疏 binding；“100k 辆实际 parked”则需要
100k 个 live vehicle/binding，这是产品真实状态，不能也不应该伪装成常数内存。

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
- park 前未到达、重复 park、错误 target、stale handle；
- 多出口 leave、unsafe insertion、错误出口；失败后仍 Parked/Occupied/无 pose；
- cancel、rebind、Active/Reserved/Occupied/Parked despawn、route removal、parked spawn；
- counts 从成员重算，stable member/observation order；
- Parked 不进入 occupancy/leader/motion/traversal。

### 10.3 Snapshot/replay/cutover

- explicit/virtual Reserved 与 Occupied save/load；
- same bytes + same input 的 digest/replay 等价；
- missing/wrong-kind target、duplicate binding、capacity overcommit 失败关闭；
- capacity increase、safe decrease、unsafe decrease；
- reserved selected entry 移除失败，occupied entry 变化按合同允许；
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
- **#541 / G2+**：一次 clean-break 实现当前静态制品权威，并改
  shared static/Runtime/snapshot/cutover/Spatial/Adapter/API/fixtures/docs，按上节验证。
- **#543 / research**：只做 exact capacity report；格式容量裁决留在对应独立权威。
- **#304**：消费已接受的停车切片，并继续分别登记信号、干支路、小区出口、公交/出租/
  路侧摩擦等其他域的支持状态；不能因为停车切片冻结就宣称整个 workload G1 完成。

#540 G1 Accepted 后，#541 可以进入 Ready/G2；#541 完成前，当前代码仍以 `ParkingArea`
和具体 `ParkingSpace` 占用为现实。任何实现 PR 必须同时报告满足、依赖绑定和明确延后的
验收项，不能用“核心路径已跑通”替代完整跨层 clean-break。
