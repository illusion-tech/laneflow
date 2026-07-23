# Vehicle Following 设计

**文档状态**: Accepted  
**最后更新**: 2026-07-24

**适用范围**: Vehicle Following 的 Vehicle Profile、纵向状态、leader/occupancy、IIDM、safe-speed、Traffic v0.7 道路限速、minimum-gap-preserving geometry projection、事件、确定性与性能验收

**关联文档**:

- `../architecture.md`
- `../roadmap.md`
- `../adr/0003-runtime-tick-and-determinism.md`
- `../adr/0005-core-identity-and-handle-model.md`
- `../adr/0006-vehicle-following-control-and-safety.md`
- `../adr/0007-traffic-data-crate-and-loader-boundary.md`
- `../adr/0008-pre-1.0-data-format-version-policy.md`
- `core-id-handles.md`
- `data-loading.md`
- `lane-graph.md`
- `route-system.md`
- `data-format.md`
- `parking-system.md`

## 1. 目标与状态

本文固化 LaneFlow v0.3 Vehicle Following 的最小可执行设计，作为 #71 的 G1 冻结结果、#79 的 Traffic Data 边界输入和 #73-#77 的实施依据；全面审阅发现的性能修复由 #86 收口。

目标：

- 定义车辆纵向几何、profile 和 runtime state。
- 在 fixed tick 中稳定检测 leader，并实现平滑跟驰、停止和恢复。
- 在正常受控模式下保证 tick 后车辆不发生纵向几何重叠，并且不从可行 snapshot 主动侵入 follower `min_gap`。
- 保持 Core 引擎无关、确定性、失败原子性和可扩展性能。
- 明确 Core API、data format、未来 Adapter 和迁移影响。

非目标：

- lane changing、turn-lane selection 和 overtaking。
- signals、intersection priority、merge reservation 和 roundabout conflict。
- parking、事故碰撞和 out-of-control physics。
- 专业交通工程标定或 SUMO 行为兼容。
- public controller trait、插件 ABI 或 Adapter constraint injection。
- 跨 CPU bit-level determinism。
- 百万级城市运行时的 partition/parallel/mesoscopic 实现；该范围由 #72 跟踪。

## 2. 术语

- **Physical edge**：lane graph 中由 `EdgeHandle` 标识的实际 lane edge。
- **Route occurrence**：同一个 physical edge 在有限 route sequence 中的一次出现，由 `route_edge_index` 区分。
- **Front progress**：车辆前保险杠沿当前 physical edge 的 progress。
- **Bumper gap**：follower 前保险杠到 leader 后保险杠的 route-relative 距离。
- **Leader**：沿 follower 已选 route、lookahead 内最近的 Active/Stopped vehicle。
- **Comfort controller**：正常驾驶时产生期望加速度的 IIDM 层。
- **Safe-speed**：把 next speed 限制在 emergency braking 可处理范围内的确定性上界。
- **Base speed limit**：Traffic/LaneGraph immutable per-edge 基础道路限速。
- **Effective speed ceiling**：纵向管线当前实际采用的速度上限；v0.8 没有超车或驾驶风格放宽，因此等于 base speed limit。
- **Safety projection**：emergency braking 仍不能避免本 tick 重叠时的最终 travel 修正。
- **Occupancy snapshot**：单个 tick 内不可变的车辆物理占用视图。

## 3. 分层与 tick phases

LaneFlow 将交通决策分为 route、maneuver/lane、longitudinal、conflict 和 presentation 层。route 与 maneuver 可低频或事件驱动；v0.3 只实现每 fixed tick 执行的 longitudinal 层。

单次 step 的概念 phases 固定为：

```text
1. validate tick/time
2. freeze immutable current-state snapshot
3. build edge occupancy index
4. resolve leader and longitudinal constraints
5. evaluate IIDM comfort acceleration
6. apply current-edge ceiling and downstream speed-limit targets
7. apply emergency safe-speed envelope
8. integrate/reduce ballistic candidate movement
9. solve deterministic no-overlap projection
10. advance route using final travel
11. atomically apply state and ordered events
```

Snapshot 是语义约束，不要求复制完整 world。当前 state 可以保持只读，candidate state 写入可复用 scratch；任一步失败都不得提交 tick/time/state/events。

## 4. Vehicle Profile 与 data format 0.3

### 4.1 Profile 字段

v0.3 Vehicle Profile 包含：

```text
VehicleProfile
  id: external ID
  length: meter
  model: iidm
  desiredSpeed: meter/second
  minGap: meter
  timeHeadway: second
  maxAcceleration: meter/second^2
  comfortableDeceleration: meter/second^2
  emergencyDeceleration: meter/second^2
```

IIDM exponent 固定为 `4`，不是每 profile 可调字段。所有行为字段必填，不使用 loader 隐式默认值。

Validation：

- 所有数值必须 finite。
- `length`、`desiredSpeed`、`timeHeadway`、`maxAcceleration`、`comfortableDeceleration`、`emergencyDeceleration` 严格大于零。
- `length` 严格大于领域专用的最小 vehicle length（current-f64 为 `1.0e-9 m`）。
- `minGap >= 0`。
- `emergencyDeceleration >= comfortableDeceleration`。
- external ID 遵循当前 data-format 的 ASCII token 规则，并在 profile domain 内唯一。

### 4.2 Package 版本

Vehicle Profile shape 由 current `schemas/laneflow-data-v0.7.schema.json` 继续承载。概念 package：

```json
{
  "formatVersion": "0.7",
  "units": {
    "distance": "meter",
    "time": "second"
  },
  "laneGraph": {
    "edges": []
  },
  "routes": [],
  "vehicleProfiles": [
    {
      "id": "passenger-car",
      "length": 4.5,
      "model": "iidm",
      "desiredSpeed": 13.9,
      "minGap": 2.0,
      "timeHeadway": 1.5,
      "maxAcceleration": 1.5,
      "comfortableDeceleration": 2.0,
      "emergencyDeceleration": 6.0
    }
  ],
  "signals": {
    "stopLines": [],
    "movementGates": [],
    "groups": [],
    "controllers": []
  },
  "parking": {
    "areas": [],
    "spaces": []
  }
}
```

规则：

- 当前 v0.7 沿用已接受的 Vehicle Profile 领域语义，并要求每条 edge 显式 `speedLimit` 以及顶层 Signals/Parking objects。
- 顶层 `vehicleProfiles` 必填，允许空数组。
- Core-defined objects 继续采用 closed shape。
- production loader 只接受 `"0.7"`；v0.5、v0.6 和未来版在当前 shape validation 前返回 version error。
- 不隐式合成 profile，不提供历史格式 compatibility shim。
- current format 不持久化 initial vehicles、spawn schedule、demand、runtime handles 或 Adapter metadata。

### 4.3 Runtime identity

Profile external ID 在 world 初始化时归一化为 opaque、world-scoped `VehicleProfileHandle`。Public contract 只要求 `Clone + Copy + Debug + Eq + Hash`，不承诺数值、index 或排序语义。

v0.3 profile registry 在 world 生命周期内不可变，不公开 runtime register/remove/mutate API。Core 提供 external ID 与 handle resolver；tick hot path 只读取 handle 和 compact profile data。

### 4.4 Crate 与 loader 边界

依据 ADR 0007/0008，current v0.7 production loader 位于 `laneflow-data`，依赖方向为 `laneflow-data -> laneflow-core`。Core 不依赖 Serde、JSON、JSON Schema 或文件系统；pre-1.0 的 production loader 只维护当前格式。

public loader 返回单一当前 `LoadedPackage`，不公开历史版本 enum/variant，也不以 optional profile 或空 registry 区分格式。Vehicle Profiles、Signals 与 Parking 都由显式字段构造；空数组是当前格式的合法状态。

Core 使用 `InitialTrafficData` 统一验证 lane graph、初始 routes、immutable profile registry 与 static Signals/Parking registries。data crate 不重复实现 route/profile/signal/parking domain invariant。loader 只接收内存 bytes/string，并返回完成 Core normalization 的当前结果，不创建 `CoreWorld`。

wire DTO 在 #73 阶段保持私有。Vehicle Profile 使用 `IidmProfileSpec` 与 `VehicleProfile::try_new_iidm`，避免多个同类型位置参数；有效 profile 的字段保持私有，v0.3 不公开 model enum 或 controller trait。

## 5. Vehicle runtime state 与迁移

v0.3 最小 state：

```text
VehicleState
  handle: VehicleHandle
  profile: VehicleProfileHandle
  route: RouteHandle
  routeEdgeIndex: usize
  edgeProgress: front-bumper progress
  currentSpeed: nonnegative meter/second
  appliedAcceleration: signed meter/second^2
  status: Active | Stopped | Completed
```

规则：

- v0.2 `speed` 破坏性改名为 `current_speed`。
- 删除含糊的 `effective_speed()`。
- spawn input 使用必填 profile reference 与 `initial_speed`。
- spawn 后 `applied_acceleration = 0`。
- 跟驰导致速度归零时仍保持 `Active`，下一 tick 可以恢复。
- 显式 `Stopped` 和 `Completed` 的 current speed/applied acceleration 必须为零。
- route completion 后 state 归零，并只产生一次 completed event。
- desired speed、length 和 deceleration 参数只存在于 immutable profile，不复制进 mutable state。

`Acceleration` 应使用 signed finite newtype。Vehicle Profile 参数通过 `IidmProfileSpec` 与 fallible constructor 一次性校验，并封装在私有字段中；v0.3 不要求为每个 profile 参数立即公开独立 newtype。

## 6. Leader detection 与 route-relative distance

### 6.1 前保险杠语义

`edge_progress` 表示车辆前保险杠位置。对于 follower `F` 和 candidate leader `L`：

```text
bumper_gap = route_distance(F.front, L.front) - L.length
```

`edge_progress = 0` 允许车身暂时位于 route 入口外。Adapter 可从 front progress 和 length 推导车辆中心，但 Core 不消费世界坐标几何。

### 6.2 Leader 规则

查询顺序：

1. 当前 physical edge 上 front progress 更大的 occupant。
2. follower route sequence 中后续 edge occurrence。
3. 在 lookahead 内选择最小正 route distance 的非 self candidate。

Candidate 自身 route 不影响它对当前 physical edge 的占用。分叉时不搜索 follower 未选 branch；其他 incoming branch 上、尚未进入共享 downstream edge 的车辆不是 longitudinal leader，而应由未来 merge/conflict constraint 处理。车辆进入共享 downstream edge 后，才按普通 leader 处理。

### 6.3 Repeated edge 与 cycle

- Occupancy 按 physical edge 存储，route occurrence 由 follower `route_edge_index` 解释。
- 同一 candidate 映射多个 future occurrence 时，只保留最小正 route distance。
- Follower 始终按 `VehicleHandle` 全局排除 self。
- 环形 route 中，物理坐标位于 follower 后方的其他车辆可以通过下一 occurrence 成为前车，但必须在 lookahead 内。

### 6.4 Overlap

- 同一 physical edge 上两个正 length vehicle 的相同 front progress 是非法重叠，不通过 tie-break 合法化。
- `bumper_gap` 小于负的物理 gap/overlap 阈值时非法。
- 物理 gap/overlap 阈值范围内规范化为零接触。
- 只违反 profile `min_gap`、但未发生物理重叠的 world initialization 或 lifecycle command 输入仍合法；runtime final geometry projection 不得继续缩小该既有异常净距。
- 初始化和 `spawn_vehicle` 必须原子拒绝同 edge、相邻 route boundary 和 repeated occurrence 可见范围内的物理重叠。
- 其他 incoming branch 在进入共享 edge 前不做纵向 overlap 投影；Core 没有足够世界几何判断分支间碰撞。

### 6.5 状态参与

`Active` 和 `Stopped` 进入 occupancy。Snapshot 开始时仍为 Active、但本 tick 将完成 route 的车辆，在本 tick 仍可作为 leader；提交为 `Completed` 后，从下一 tick occupancy 消失。Completed/despawned vehicle 不进入 occupancy。

## 7. Occupancy index

### 7.1 Tick-local flat index

Occupancy 使用按 dense `EdgeHandle` 分段的扁平私有 scratch：

```text
OccupancyIndex
  edgeOffsets: usize[edgeCount + 1]
  occupants: Occupant[]

Occupant
  vehicle: VehicleHandle
  frontProgress: f64
  vehicleLength: f64
  updateSequence: u64
```

每辆 Active/Stopped vehicle 只生成一个 occupant record。跨 edge 车身通过 route distance 减去 vehicle length 处理，不把一辆车复制进多个 edge bucket。

### 7.2 Build 与排序

每 tick：

1. 复用并清零 edge counts。
2. 按稳定 vehicle update order 计数。
3. Prefix sum 生成 edge offsets。
4. 写入连续 occupant buffer。
5. 每个 edge slice 原地排序。

排序键固定为 `(front_progress.total_cmp, update_sequence)`。Progress 入索引前必须 finite，并把负零规范化为正零。完整排序键形成确定全序，可以使用原地 unstable sort；update sequence 只提供稳定 tie-break，不改变 overlap 语义。

### 7.3 Query 与复杂度

- 当前 edge 使用二分查找定位第一个更大 front progress。
- 后续 occurrence 读取 edge slice 最前方的非 self occupant。
- Route 注册时可预计算 cumulative edge lengths。
- 构建目标复杂度为 `O(E + V + sum(sort(n_edge)))`。
- 单车查询为当前 edge `O(log n_edge)` 加 horizon 内 route occurrences。
- 禁止每辆车扫描全体车辆和全局 `O(V^2)`。
- Counts、offsets、occupants、candidate 和 projection buffers 跨 tick 复用。

Occupancy 不进入 public API、不允许 Adapter 缓存，也不跨 lifecycle command 增量修补；spawn/despawn 后在下一 tick 完整重建。

#106 新增的 command-spatial index 不改变上述 tick authority：它只服务 step 之间的 spawn/leave 类局部验证，是 non-authoritative private cache。查询始终从 committed `VehicleState` 读取 progress，并用 route occurrence 做最终过滤；物理 edge membership 只在 spawn/despawn、完成或真实 physical-edge transition 时同步，不因同一 physical edge 上的 progress 变化做每车每 tick 重排。它与 tick-local Occupancy 保持两套职责，禁止用 command index 替代 leader/no-overlap snapshot。

Route-distance 查询使用 overflow-safe segmented index；route-end 常见 horizon 先走当前/下一 edge 的快速拒绝路径，只有目标可能进入有限 horizon 时才访问完整 occurrence index。这样保留大量 finite edge 累计溢出时的正确性，同时避免无关 route scan 或把三个 `Vec` 放进热 `RouteSlot`。对应 full-scan oracle、operation counter 与性能证据见 [`../reference/v0.5-lifecycle-substrate-validation.md`](../reference/v0.5-lifecycle-substrate-validation.md)。

## 8. Longitudinal constraints

v0.3 使用 Core 私有、tick-local `LongitudinalConstraintSet`，分为：

- speed ceilings；
- spatial targets。

Spatial target 概念字段包括 source kind、distance ahead、target speed、desired clearance 和 hard clearance。各 subsystem 只从 snapshot 产生 candidate constraint，不直接修改 vehicle state。Reducer 选择最严格约束；稳定 tie-break 只用于 attribution，不使用任意 numeric priority 改变物理优先级。

Physical constraints 不可绕过；regulatory constraints 由后续 policy subsystem 产生。Road capacity、demand、route cost 和全局统计不进入 longitudinal controller。Signals、intersection 和 parking 后续产生 stop/reservation target，不把规则写入 IIDM。

v0.3 不公开 constraint provider，也不允许 Adapter 任意注入 constraint。

Current Parking 复用本节 private spatial-target/safety ownership：ParkingStop 与 SignalStop/RouteEnd/SpeedLimit 从同一 snapshot 生成并按最严格 admissible motion 归约，spatial hard projection 先于 no-overlap projection。Parked vehicle 排除 lane occupancy；Arrived 但未 commit 的 Active vehicle 仍是 stationary leader。完整 event/order/performance 边界见 [`parking-system.md`](parking-system.md)。

### 8.1 Per-edge 道路限速

- `LaneEdge.speed_limit` 是 immutable 基础道路事实；v0.8 的 `effective_speed_ceiling = base_speed_limit`。
- 当前 edge 先令 IIDM free-flow target 为 `min(profile.desiredSpeed, effective_speed_ceiling)`；candidate speed 若仍会因离散 tick 越限，则先收紧 speed 并重新计算 ballistic motion，成功 step 后不得高于 committed current edge limit。
- route 注册时按 occurrence 预计算全部相邻降限速 transition；hot path 只遍历当前 occurrence 之后、comfortable braking horizon 内的 compact metadata，不查 external ID、不构造临时集合。
- 对距离 `d`、目标限速 `L` 与舒适减速度 `b`，next speed `u` 必须满足：

```text
0.5 * (v + u) * dt + max(0, u^2 - L^2) / (2*b) <= d
```

- 所有 horizon 内的降限速边界都参与归约；更远但更陡的降速可以比最近边界更严格。升限速边界不提前加速，只有车辆以合规速度进入更高限速 edge 后，下一 tick 才恢复 IIDM 加速。
- crossing guard 以同一常加速度轨迹计算边界瞬间速度；正常可行时允许同 tick 连续跨 edge。舒适制动不足时可使用 profile emergency envelope；若 emergency envelope 仍会超限 crossing，则把 travel 投影到首个违规边界、令 final speed 不超过目标限速，并产生一次 `VehicleSpeedLimitProjectionApplied`。
- initial world 与 `spawn_vehicle` 共用同一 normalization；`initial_speed > current edge base speed limit` 原子拒绝。
- 道路限速、SignalStop、ParkingStop、RouteEnd 与 leader/no-overlap 继续选择共同可行的更小 motion；Adapter 不做 clamp。

未来驾驶风格、边界后延迟合规、超车/换道和最高 20% 的短时超车授权不属于当前实现。未来必须由 Core-owned compliance/maneuver policy 单独设计，保持 base limit immutable，并优先用加载期归一化的 fixed-size preset/offset 复用同一 solver；不得引入每 tick PRNG、callback/trait object、external-ID lookup、完整 route 扫描或临时集合。

## 9. IIDM comfort controller

### 9.1 变量

```text
v       follower current speed
v0      profile desired speed
v_l     leader current speed
delta_v v - v_l; positive means follower is faster
s       bumper gap
s0      profile min gap
T       profile time headway
a       profile max acceleration
b       profile comfortable deceleration
delta   4
```

期望动态间距：

```text
s_star = s0 + max(0, v*T + v*delta_v/(2*sqrt(a*b)))
```

### 9.2 Free-road acceleration

```text
if v <= v0:
  a_free = a * (1 - (v/v0)^delta)
else:
  a_free = -b * (1 - (v0/v)^(a*delta/b))
```

### 9.3 Leader interaction

无 leader 或 leader 在 horizon 外时使用 `a_free`。有 leader 且 `s` 严格大于物理 gap/overlap 阈值时令 `z = s_star / s`：

```text
if z >= 1:
  a_iidm = a * (1 - z^2)
else if a_free > 0:
  a_iidm = a_free * (1 - z^(2*a/a_free))
else:
  a_iidm = a_free
```

`s` 小于或等于物理 gap/overlap 阈值时不做除法，comfort 输出直接取 `-b`。最终 comfort acceleration clamp 到 `[-b, a]`。

IIDM evaluator 是 Core 私有纯计算单元：输入 profile 与 observation，输出 desired acceleration，不读取 wall clock、随机数或 world mutation。

## 10. Lookahead 与 safe-speed

### 10.1 Leader query horizon

Leader 尚未找到时使用 stationary-leader worst case 推导 follower 自身搜索上界：

```text
dt = fixed_delta_time_ms / 1000
v_upper = v + a*dt
travel_upper = 0.5*(v + v_upper)*dt
hard_horizon = travel_upper + v_upper^2/(2*b_emergency)
comfort_horizon = s0 + v*T
minimum_gap_horizon = s0 + travel_upper + minimum_gap_tolerance
bumper_gap_horizon = max(hard_horizon, comfort_horizon, minimum_gap_horizon)
front_query_horizon = bumper_gap_horizon + max_vehicle_length
```

`minimum_gap_horizon` 保证低速 follower 也能看到本 tick 内可能被侵入 minimum-gap floor 的 leader；专用 tolerance 覆盖 `s0` 边界附近的 f64 舍入。horizon 外的 leader 即使静止，follower 以 `travel_upper` 前进后仍不会低于 `s0`。查询不按固定 edge 数截断，也不默认遍历完整 route。

### 10.2 Emergency safe-speed

令 `b_f`、`b_l` 为 follower/leader emergency deceleration，`u` 为待求 next speed：

```text
0.5*(v + u)*dt + u^2/(2*b_f)
  <= s + v_l^2/(2*b_l)
```

整理：

```text
rhs = 2*b_f*s + (b_f/b_l)*v_l^2
B = b_f*dt
C = b_f*v*dt - rhs
```

当 `C <= 0` 时存在非负可行根。为避免直接使用 `(-B + sqrt(B^2 - 4*C))/2` 产生浮点消减，固定使用代数等价的稳定形式：

```text
discriminant = B^2 - 4*C
v_safe = (-2*C)/(B + sqrt(discriminant))
```

当 `C > 0` 时，next speed 为零仍不满足停车不等式，`v_safe = 0` 并进入 emergency/projection 路径，不返回 validation error。所有中间结果必须 finite；最终上界可以向安全方向 clamp，但不得因舍入得到比数学正根更大的速度。

Safe-speed 只产生上界：

```text
v_target = min(v_comfort_candidate, v_safe)
v_emergency_floor = max(0, v - b_f*dt)
v_candidate = max(v_target, v_emergency_floor)
```

无 leader 时不应用 leader safe-speed 上界。

## 11. Ballistic integration 与 no-overlap

### 11.1 Ballistic candidate

Tick 内使用常加速度积分：

```text
v_candidate = max(0, v + acceleration*dt)
```

未在 tick 内停车：

```text
travel = 0.5*(v + v_candidate)*dt
```

若负加速度使车辆在 tick 中途停车：

```text
stop_time = v / -acceleration
travel = v*stop_time + 0.5*acceleration*stop_time^2
v_candidate = 0
```

不得产生负速度后再简单 clamp，也不采用 explicit/semi-implicit Euler 作为 v0.3 权威积分。

### 11.2 Final geometry constraint

对每个 follower/leader relation，先定义：

```text
g0 = max(0, normalized snapshot_bumper_gap)
g_floor = min(g0, follower.min_gap)
raw_available_gap = g0 - g_floor
available_gap = 0 if raw_available_gap <= minimum_gap_tolerance
                else raw_available_gap

follower_final_travel <= available_gap + leader_final_travel
final_gap = g0 + leader_final_travel - follower_final_travel
final_gap >= g_floor
```

`normalize_minimum_gap_slack` 把 minimum-gap 专用绝对阈值内的正 slack 规范化为零。因此：

- `g0 >= follower.min_gap` 时，最终至少保留 follower `min_gap`；
- `0 <= g0 < follower.min_gap` 时，最终至少保留 `g0`，不倒车、不瞬移；
- `follower.min_gap == 0` 时退化为原有 no-overlap 约束；
- 该约束在每个有 leader 的 tick 生效，不能等双方完全静止后再应用。

求解目标是在不超过各 vehicle candidate travel 的前提下，得到最大的可行 final travel。Spatial hard projection（Signal/Parking/SpeedLimit/RouteEnd）先确定 leader final travel，再从前向后传播 follower minimum-gap cap。

Leader graph 中每个 vehicle 至多指向一个 leader：

- 无环链从最前方 vehicle 向后传播。
- 多个 follower 读取同一 leader final travel。
- Cycle 选择 `(candidate_travel, update_sequence)` 最小的 vehicle 为 anchor，沿反向 follower 链传播一次，再验证 closing constraint。
- 非负 `available_gap` 下该过程给出确定性最大可行解，目标复杂度 `O(V)`；禁止迭代到收敛或 `O(V^2)`。

### 11.3 Projection event threshold

Emergency braking 在本 tick 可达到的最小 ballistic travel 固定为：

```text
if v <= b_emergency*dt:
  emergency_min_travel = v^2/(2*b_emergency)
else:
  emergency_min_travel = v*dt - 0.5*b_emergency*dt^2
```

对 follower/leader relation：

```text
g0 = max(0, snapshot_bumper_gap)
preserved_gap = min(g0, follower.min_gap)
raw_available_gap = g0 - preserved_gap
available_gap = 0 if raw_available_gap <= minimum_gap_tolerance
                else raw_available_gap
geometry_cap = max(0, available_gap + leader_final_travel)
final_travel = min(candidate_travel, geometry_cap)
```

- Geometry cap 仍不小于 emergency minimum travel 时，属于普通 emergency clamp，不发事件。
- Geometry cap 更小时，final travel clamp 到 cap，final speed 相应降低、必要时归零，允许 effective applied acceleration 超过 profile emergency deceleration，并产生一次 safety projection event。
- 仅仅存在 pre-existing sub-min-gap 不发事件；静止异常状态不会形成每 tick 事件风暴。事件仍只表示本 tick minimum-gap-preserving geometry cap 超出 emergency envelope。

当 `final_travel < candidate_travel` 时，final speed 使用唯一映射：

```text
speed_from_travel = max(0, 2*final_travel/dt - current_speed)
final_speed = min(candidate_speed, speed_from_travel)
```

如果 final travel 小于常加速度减速到零所需距离，`final_speed = 0`；safety projection event 明确表示该结果是几何修正，而不是高精度车辆动力学。没有 geometry clamp 时直接保留 candidate speed。

`applied_acceleration = (final_speed - current_speed) / dt`。它表示本 tick 状态变化对应的有效平均加速度，必须 finite。

## 12. 领域数值策略、finite 与错误语义

- edge boundary/remainder、纵向约束和物理 gap/overlap 分别由 crate-private owner 负责；current-f64 值都为 `1.0e-9 m`，但不得互相别名或由通用近似比较 helper 代理。
- minimum-gap available slack 使用独立的 crate-private `1.0e-9 m` 绝对阈值；阈值内正 slack 向安全方向规范化为零，避免 route segmentation 的浮点微差改变投影分支。
- Vehicle Profile length 的输入下限独立于物理 gap/overlap 阈值。
- computed-speed near-zero 使用独立的 `1.0e-9 m/s` owner，只覆盖已有运行时计算速度判断。
- `Speed::ZERO`、状态速度等 authority 继续使用精确正零与精确相等，不被 near-zero predicate 替代。
- target-f32 的固定领域绝对阈值由 #127 离线标定；#144 曾在候选中启用，但形成性能不迁移（no-go）结论后已回退。相对误差和 ULP 不进入 production predicate。
- 合法 finite 输入若导致中间计算非有限，返回结构化 longitudinal runtime error，step 原子失败。
- Safety projection、正常 emergency braking 和拥堵停车不是 validation error。

## 13. Events 与观察边界

新增离散事件：

```text
VehicleFollowingSafetyProjectionApplied
  tickIndex
  vehicle
  leader
```

道路限速不可行 crossing 使用独立稀疏事件：

```text
VehicleSpeedLimitProjectionApplied
  tickIndex
  vehicle
  route
  fromRouteEdgeIndex
  toRouteEdgeIndex
  fromEdge
  toEdge
```

事件不携带 `f64`，每 vehicle/tick 最多一次。它表示 minimum-gap-preserving 最终几何投影已把 motion 压到 emergency envelope 以下；常规减速、普通 emergency clamp、既有 sub-min-gap 静止状态、停车和恢复不产生事件。Adapter 通过 `VehicleState.current_speed` 和 `applied_acceleration` 观察连续状态。

事件只随成功原子提交返回。车辆间按稳定 update order；同一车辆内顺序为被选中的 spatial projection（SpeedLimit/Signal/Parking 至多一种）、following safety projection、实际 edge transitions、arrival/completion。Route movement 和 route events 必须依据 final travel 计算。

## 14. Public/private API 边界

Public：

- `VehicleProfile` / `VehicleProfileHandle` 和 resolver。
- 迁移后的 `VehicleState` / `VehicleSpawnInput`。
- `VehicleFollowingSafetyProjectionApplied` / `VehicleSpeedLimitProjectionApplied`。
- `SpeedLimit`、LaneGraph 限速查询与结构化 profile/overlap/longitudinal/speed-limit errors。
- 现有 fixed-step `CoreWorld::step` / `StepResult`。

Private：

- OccupancyIndex / Occupant。
- LeaderObservation。
- LongitudinalConstraintSet。
- IIDM evaluator、safe-speed solver 和 projection graph。
- Scratch/candidate buffers。

v0.3 不公开 controller trait、callback、registry 或 arbitrary Adapter injection。第二个内置模型优先使用内部 enum/static dispatch；真正的第三方/跨语言扩展需要新 ADR。

## 15. Determinism 与测试

确定性范围沿用 ADR 0003。测试至少覆盖：

- 相同 world/input 序列逐 tick 状态与事件一致。
- 初始 vehicle 输入排列变化后，按 external ID 对齐结果一致。
- Same-edge、cross-edge、branch、merge-after-shared-edge、repeated edge 和 self exclusion。
- Same progress/overlap rejection、min-gap-only 输入合法状态与 pre-existing sub-min-gap 非恶化。
- Active/Stopped/Completed occupancy。
- IIDM free/interaction 各分支和 desired speed 上下边界。
- Safe-speed discriminant、emergency floor 和 projection threshold。
- 当前 edge ceiling、60→40 advance braking、40→60 不提前加速、连续多个降限、repeated edge、hard projection attribution 与 over-limit spawn 原子失败。
- Ballistic 中途停车。
- Acyclic platoon、same-tick hard-stop 的三车以上 front-to-back 传播、multiple followers、repeated edge 和 explicit cycle anchor。
- `min_gap == 0` 的 no-overlap 退化行为，以及可行 snapshot 的 `final_gap >= min_gap` / 异常 snapshot 的 `final_gap >= g0` property。
- Spawn/despawn、stale handle 和失败原子性。
- 事件数量、顺序和 route transition/completion 一致性。
- 所有状态 finite、speed 非负、normal-mode no-overlap。

推荐 #77 引入成熟 `proptest` dev dependency，生成合法线性 platoon 并持久化失败回归样例。大文本 golden snapshot 不作为主要确定性证据。

## 16. 性能验收

### 16.1 分级

- 10k：每 tick 高精度 Vehicle Following，G3 验收规模。
- 100k：复杂度和扩展性观察，不设置跨机器绝对时间门槛。
- 1M：城市级容量探索，不承诺当前单线程实时。
- 1M+：由 #72 设计 partition、parallel、multi-rate 和 mesoscopic/aggregate 模型。

### 16.2 10k 协议

- 10,000 Active vehicles，连续 60 个 16 ms fixed ticks。
- 场景：free-flow、dense platoon、stop-and-go；projection-heavy 单独报告。
- 指定 reference desktop 常规场景目标 median `<= 1 ms/tick`。
- G3 硬上限 median `<= 4 ms/tick`，即 60 ticks `<= 240 ms`。
- Benchmark 排除 world/schema 构建和样本重置，固定输入并消费状态/事件。
- 记录 CPU、OS、rustc、release profile 和电源模式。
- CI 运行 10k 功能 smoke 与 benchmark compile，不使用共享 CI wall-clock assertion。
- 基线建立后，同机受控三轮 median 回退超过 20% 必须分析，超过 30% 默认阻断，除非记录显式例外。

### 16.3 Scaling constraints

- 禁止 external ID hot-path lookup/clone/sort。
- 禁止 per-vehicle heap object 和 dynamic controller dispatch。
- Scratch buffers 必须复用，event 分配只与实际离散事件量相关。
- speed-limit transition metadata 按 route 共享；无 transition 时 fast reject，steady tick 不做 per-vehicle heap allocation。
- 10k 到 100k 不得呈现 `O(V^2)` 趋势。
- v0.2 临时 1M steady-state 结果只作为乐观研究输入，不构成 v0.3 全市实时声明。

## 17. v0.2 -> v0.3 迁移

| v0.2                 | v0.3                                         | 迁移                                                |
| -------------------- | -------------------------------------------- | --------------------------------------------------- |
| `VehicleState.speed` | `current_speed`                              | 破坏性改名                                          |
| `effective_speed()`  | 删除                                         | 状态直接保存权威当前速度                            |
| 无 acceleration      | `applied_acceleration`                       | 新增 signed finite state                            |
| Spawn `speed`        | `initial_speed`                              | 破坏性改名                                          |
| 无 profile           | 必填 profile reference                       | 显式绑定                                            |
| Point-like progress  | front-bumper progress                        | 语义明确化                                          |
| Data `0.2`           | `0.3` schema/loader（该里程碑当时的 active） | 直接替换 active 格式；旧版明确拒绝，历史由 Git 保存 |
| 无 following event   | safety projection event                      | 稀疏离散事件                                        |

LaneFlow 处于 pre-1.0 阶段，采用直接迁移，不叠加双字段 alias、隐藏 default profile 或 compatibility shim。

## 18. 实施切片

- #71：Vehicle Following 行为、安全、确定性与性能设计。
- #79：Traffic Data crate、production loader 和 Core normalization 边界。
- #73：Vehicle Profile schema、loader、registry/resolver。
- #74：VehicleState、spawn input 和 profile handle 迁移。
- #75：Occupancy index、leader detection 和 overlap validation。
- #76：IIDM、safe-speed、ballistic integration 和 no-overlap projection。
- #77：确定性、不变量、10k 性能和 100k 扩展性验证。
- #86：全面审阅发现的 candidate state scratch 复用与最终性能复核。
- #72：城市级性能架构研究，不阻塞 v0.3。

主实施顺序由 GitHub 原生 blocked-by 链表达：`#71 -> #79 -> #73 -> #74 -> #75 -> #76 -> #77`；#86 是 milestone 全面审阅后追加并完成的收口阻断修复。

## 19. G1 审阅结论

本设计已确认：

- D1-D12 可追踪且无未决产品语义。
- 与 ADR 0003、ADR 0005、ADR 0008 及当前 lane graph、route、data-format 契约一致。
- Comfort、emergency 与 geometry projection 职责不重叠。
- Explicit loop/cycle 有确定性线性求解边界。
- Public API/data format breaking impact 已显式记录。
- 后续实施、验证和城市级研究均有独立 Issue，不扩大 #71。

若实施发现安全矛盾、公式不可实现或未记录 public breaking change，必须回到本设计/ADR 或拆 follow-up；不得通过私有实现静默改变 Accepted 语义。

## 20. v0.8 道路限速约束

#184 将 per-edge road speed limit 纳入同一 `LongitudinalConstraintSet`，不建立第二套车辆控制器。当前 edge 提供 speed ceiling；route 下游更低限速边界提供 advance-braking spatial target，使车辆在 crossing 边界时已不超过新限速。它与 leader/no-overlap、SignalStop 和 route completion 同步求解，不能以 `VehicleProfile.desiredSpeed` 或最后一步 clamp 代替。

spawn/atomic replace 的初始速度不得超过当前 edge 限速，v0.8 默认初始与回流速度为零。限速值、14 条 routes 与测试矩阵见 `example-scenarios.md`；Core API、行为和性能验证由 #185 实施。
