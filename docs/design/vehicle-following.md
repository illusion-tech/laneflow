# Vehicle Following 设计

**文档状态**: Accepted（纵向分层与 IIDM/安全投影仍有效；已提交一维几何为整数毫米，IIDM 仍为瞬时 SI）<br>
**最后更新**: 2026-08-27

**适用范围**: Vehicle Following 的 Vehicle Profile、纵向状态、leader/occupancy、IIDM、safe-speed、per-edge 道路限速、minimum-gap-preserving geometry projection、事件、确定性与性能验收

**关联文档**:

- `../architecture.md`
- `../roadmap.md`
- `../adr/0003-runtime-tick-and-determinism.md`
- `../adr/0005-core-identity-and-handle-model.md`
- `../adr/0006-vehicle-following-control-and-safety.md`
- `../adr/0007-traffic-data-crate-and-loader-boundary.md`
- `../adr/0008-pre-1.0-data-format-version-policy.md`
- `../adr/0028-integer-millimeter-traffic-geometry.md`
- `traffic-runtime-integer-geometry.md`
- `core-id-handles.md`
- `data-loading.md`
- `lane-graph.md`
- `route-system.md`
- `data-format.md`
- `parking-system.md`

## 1. 目标与状态

本文固化 LaneFlow v0.3 Vehicle Following 的最小可执行设计，作为 #71 的 G1 冻结结果、#79 的 Traffic Data 边界输入和 #73-#77 的实施依据；全面审阅发现的性能修复由 #86 收口。

#496 / ADR 0028 把已提交一维几何改为整数毫米，占用/
重叠用整数比较，跨 hop 间隙为 `i64` mm，IIDM 仍为舒适层；`max_accel >= 0.5 m/s²`，
不另做速度余数。落地规则见 `traffic-runtime-integer-geometry.md`。下文若仍出现
历史米制字段名，只描述编制输入或瞬时 SI，不是已提交权威。

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
- 城市级多执行域交通运行时的分区/并行/中观实现；当前道路机动车一百万研究包络
  只是其中一个输入，该范围由 #72 跟踪。

## 2. 术语

- **Physical edge**：lane graph 中由 `EdgeHandle` 标识的实际 lane edge。
- **Route occurrence**：同一个 physical edge 在有限 route sequence 中的一次出现，由 `route_edge_index` 区分。
- **Front progress**：车辆前保险杠沿当前 physical edge 的 progress。
- **Bumper gap**：follower 前保险杠到 leader 后保险杠的 route-relative 距离。
- **Leader**：沿 follower 已选 route、后杠间隙不超过 `bumper_gap_horizon` 的最近 Active 车辆（间隙可负）。查询行走以跟车前视为限。
- **Comfort controller**：正常驾驶时产生期望加速度的 IIDM 层。
- **Safe-speed**：把 next speed 限制在 emergency braking 可处理范围内的确定性上界。
- **Base speed limit**：Traffic/LaneGraph immutable per-edge 基础道路限速。
- **Effective speed ceiling**：纵向管线当前实际采用的速度上限；current v0.10 没有超车或驾驶风格放宽，因此等于 base speed limit。
- **Safety projection**：emergency braking 仍不能避免本 tick 重叠时的最终 travel 修正。
- **Occupancy snapshot**：单个 tick 内不可变的车辆物理占用视图。
- **跟车前视**：每车每拍由当前速度与 profile 按 §10.1 推导的出现项行走窗（`front_query_horizon`）；不是目视距离，也不是后杠间隙接纳窗。

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

- 编制数值必须 finite；准入先量化再检查闭包。
- 车长 `100..=128_000` mm。
- `desiredSpeed` `1..=100_000` mm/s。
- `minGap` `0..=128_000` mm。
- `timeHeadway` 量化到 `f32` 后满足 `0 < x <= 60` s。
- `maxAcceleration` / `comfortableDeceleration` / `emergencyDeceleration` 量化到 `f32`
  后落在 `0.5..=50` m/s²，且 emergency ≥ comfort。
- spawn 用 `progress_mm` / `speed_mm_s` 且 `carry_um = 0`。不另做速度余数。
- `BeyondFinite` 降速目标本拍不参与包络。
- external ID 遵循当前 data-format 的 ASCII token 规则，并在 profile domain 内唯一。

### 4.2 Package 版本

Vehicle Profile 现行持久化是 LFCA `formatVersion = 5` 的 `VehicleProfile` 表（毫米 /
受检 `f32`）。current JSON `schemas/laneflow-data-v0.10.schema.json` 已删除，只作历史形状。历史概念 package 曾列出：

```json
{
  "formatVersion": "0.10",
  "units": {
    "distance": "meter",
    "time": "second"
  },
  "laneGraph": {
    "edges": []
  },
  "junctions": [],
  "movements": [],
  "maneuverPaths": [],
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
      "emergencyDeceleration": 6.0,
      "participantClassId": "car"
    }
  ],
  "participantClasses": [
    { "id": "motorVehicle" },
    { "id": "car", "extendsId": "motorVehicle" }
  ],
  "facilityBands": [],
  "roadSections": [],
  "laneGroups": [],
  "roadCorridors": [],
  "accessRules": [],
  "waitingZones": [],
  "signals": {
    "stopLines": [],
    "maneuverGates": [],
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

- 当前 v0.10 沿用已接受的 Vehicle Profile/IIDM 数值语义，并要求每条 edge 显式
  `speedLimit`、顶层 Junction/Movement/ManeuverPath、ParticipantClass、
  FacilityBand/RoadSection/LaneGroup/RoadCorridor/AccessRule arrays，以及
  Signals/Parking objects。
- 每个 VehicleProfile 必填 `participantClassId` 并引用已声明 ParticipantClass；
  current static AccessRule 在 `(ParticipantClass, Route)` 绑定期校验，不改变 IIDM
  profile 数值或 longitudinal solver。
- 顶层 `waitingZones` 必填，允许空数组；它不改变 Vehicle Profile wire shape。
- 顶层 `vehicleProfiles` 必填，允许空数组。
- Core-defined objects 继续采用 closed shape。
- production loader 只接受 `"0.10"`；v0.9 及更早版本和未来版在 current shape
  validation 前返回 version error。
- 不隐式合成 profile，不提供历史格式 compatibility shim。
- current format 不持久化 initial vehicles、spawn schedule、demand、runtime handles 或 Adapter metadata。

### 4.3 Runtime identity

Profile external ID 在 world 初始化时归一化为 opaque、world-scoped `VehicleProfileHandle`。Public contract 只要求 `Clone + Copy + Debug + Eq + Hash`，不承诺数值、index 或排序语义。

v0.3 profile registry 在 world 生命周期内不可变，不公开 runtime register/remove/mutate API。Core 提供 external ID 与 handle resolver；tick hot path 只读取 handle 和 compact profile data。

### 4.4 Crate 与 loader 边界

现行生产路径是编译器发射 LFCA，再由 `laneflow-format` / `laneflow-static-network`
构建共享根。`laneflow-data` / `laneflow-core` JSON loader 已拆除。Runtime 不依赖
Serde、JSON 或文件系统。

public loader 返回单一当前 `LoadedPackage`，不公开历史版本 enum/variant，也不以
optional profile 或空 registry 区分格式。VehicleProfile、ParticipantClass、
CrossSection、AccessRule、Signals 与 Parking 都由显式字段构造；允许为空的 domain
使用显式空数组表达。

Core 使用 `InitialTrafficData` 统一验证 lane graph、初始 routes、immutable profile、
Junction、ParticipantClass、CrossSection、Access 与 static Signals/Parking
registries。data crate 不重复实现对应 domain invariant。loader 只接收内存
bytes/string，并返回完成 Core normalization 的当前结果，不创建 `CoreWorld`。

wire DTO 在 #73 阶段保持私有。Vehicle Profile 使用 `IidmProfileSpec` 与 `VehicleProfile::try_new_iidm`，避免多个同类型位置参数；有效 profile 的字段保持私有，v0.3 不公开 model enum 或 controller trait。

## 5. Vehicle runtime state 与迁移

v0.3 最小 state：

```text
VehicleState
  handle: VehicleHandle
  profile: VehicleProfileOrdinal
  route: RouteHandle
  route_edge_index: u32
  progress_mm: u32
  carry_um: u16
  speed_mm_s: u32
  status: Active | Parked | Completed
```

规则：

- v0.2 `speed` 破坏性改名为 `current_speed`。
- 删除含糊的 `effective_speed()`。
- spawn input 使用必填 profile reference 与 `initial_speed`。
- spawn 后 `applied_acceleration = 0`。
- 跟驰导致速度归零时仍保持 `Active`，下一 tick 可以恢复。`Completed` 的 current speed 必须为零。当前 `TrafficWorld` 没有独立 `Stopped` 状态。
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

1. 当前物理边上所有 `hi_mm >` follower 前保险杠的占用记录中，取最小 `lo_mm`（最紧后杠）。同边占用重叠时不得只取最小 `hi_mm`。间隙大于 `bumper_gap_horizon` 则本边不接纳。
2. follower 路线跟车前视内后续出现项上该桶全部非 self 记录中的最小 `lo_mm`。入口距离大于跟车前视则早停。
3. 取最小间隙（可负）。接纳以 `bumper_gap_horizon` 为准；超出则本拍无 leader。跟车前视不得短于该行走窗。`bumper_gap_horizon` 仍满足 ADR 0006 的搜索下界。

Candidate 自身 route 不影响它对当前 physical edge 的占用。分叉时不搜索 follower 未选 branch；其他 incoming branch 上、尚未进入共享 downstream edge 的车辆不是 longitudinal leader，而应由未来 merge/conflict constraint 处理。车辆进入共享 downstream edge 后，才按普通 leader 处理。支路汇入后同边占用可以重叠；跟车间隙仍取最紧后杠，不把重叠合法化。

### 6.3 Repeated edge 与 cycle

- 占用按物理边存储，route occurrence 由 follower `route_edge_index` 解释。
- 同一 candidate 映射多个 future occurrence 时，只保留最小间隙（可负）。
- Follower 始终按 `VehicleHandle` 全局排除 self。
- 环形 route 中，物理坐标位于 follower 后方的其他车辆可以通过下一 occurrence 成为前车。该出现项须落在跟车前视内，且后杠间隙不超过 `bumper_gap_horizon`。

### 6.4 Overlap

- 同一 physical edge 上两个正 length vehicle 的相同 front progress 是非法重叠，不通过 tie-break 合法化。
- `bumper_gap` 小于负的物理 gap/overlap 阈值时非法。
- 物理 gap/overlap 阈值范围内规范化为零接触。
- 只违反 profile `min_gap`、但未发生物理重叠的 world initialization 或 lifecycle command 输入仍合法；runtime final geometry projection 不得继续缩小该既有异常净距。
- 初始化和 `spawn_vehicle` 必须原子拒绝同 edge、相邻 route boundary 和 repeated occurrence 可见范围内的物理重叠。
- 其他 incoming branch 在进入共享 edge 前不做纵向 overlap 投影；Core 没有足够世界几何判断分支间碰撞。
- 共享边上若已出现重叠占用，跟车查询取最紧后杠；这不把重叠变成合法几何。

### 6.5 状态参与

当前 `TrafficWorld`：`Active` 进入车道占用。`Parked` 与 `Completed` 不进入。Snapshot 开始时仍为 Active、但本 tick 将完成 route 的车辆，在本 tick 仍可作为 leader；提交为 `Completed` 后，从下一 tick occupancy 消失。

## 7. 占用索引

### 7.1 Tick-local 扁平索引

占用索引（occupancy index / `OccupancyIndex`）是 `TrafficWorld` crate 私有、按物理边分桶的 tick-local scratch，不进入公开 API。占用桶当前与 `LaneEdgeOrdinal` 1:1（私有 newtype `OccupancyBucketOrdinal`）。

```text
OccupancyIndex
  bucketOffsets: usize[bucketCount + 1]
  records: OccupancyRecord[]

OccupancyRecord
  vehicle: VehicleHandle
  bucket: OccupancyBucketOrdinal
  lo_mm: u32
  hi_mm: u32
  updateSequence: u32
```

坐标为整数毫米。`hi_mm` 是该占用记录在本桶上的占用上沿（主记录为前保险杠进度；车身溢出占用为该边上车身片段上沿）。`lo_mm` 是同一片段下沿（后保险杠或该边上车尾起点）。同边间隙为 `i64`：`leader.lo_mm - follower_front_mm`；跨出现项沿 follower 路线累计到该 `lo_mm`。禁止 `u32` 回绕。

每辆 Active 车辆按车身 `for_each_occupancy_interval` 写入：

- 主记录：前保险杠所在物理边；
- 稀疏车身溢出占用：车身仍覆盖的更早边。

一辆车可以在多个桶各有一条占用记录，否则分叉共享茎上车尾不可见。

`Parked` / `Completed` 不进入索引。

### 7.2 Build 与排序

每个成功 `step` 在运动循环前按已提交状态 T 完整重建；不跨生命周期命令增量修补。

1. 复用并清零 bucket counts。
2. 按稳定 `live_order` 对 Active 车辆计数占用记录。
3. Prefix sum 生成 bucket offsets。
4. 写入连续占用记录 buffer。
5. 每个 bucket 原地 unstable sort。

排序键为 `(hi_mm, lo_mm, update_sequence, vehicle.index)`。`update_sequence` 只做稳定 tie-break，不得把同边相同前缘的物理重叠合法化。安装不预留按边展开的峰值。首次重建把 bucket 表扩到边数。占用记录上限为车辆容量 × (`MAX_VEHICLE_LENGTH_MM` / `MIN_LANE_EDGE_LENGTH_MM` + 1)；`+ 1` 计入未对齐车身两端残段。该上限再与后缀下标 `u32` 可编码范围（不含哨兵 `u32::MAX`）取较小值，只作失败关闭天花板，不作为预留目标。重建先按已提交状态计数实际占用记录数 `K`：`K` 超过上限则失败关闭；否则 `try_reserve` 到 `K` 与已有高水位的较大者。分配失败失败关闭。车身跨边数上升允许一次增长；高水位内稳态 tick 不因占用索引新分配。占用区间遍历失败失败关闭，不得静默漏记。

### 7.3 Query 与复杂度

- 当前边：`partition_point` 定位 `hi_mm > follower_front` 的后缀，在该后缀的非 self 记录中取最小 `lo_mm`。间隙大于 `bumper_gap_horizon` 则本边不接纳。
- 后续出现项：沿 **follower** 跟车前视内路线读取该桶非 self 记录中最小 `lo_mm`。按出现项入口距离单调访问；入口等于跟车前视仍访问，大于则早停。接纳仅当后杠间隙不超过 `bumper_gap_horizon`。占用索引仍按全部 `Active` 重建。
- 实现按 `hi_mm` 排序后维护桶内后缀最小 `lo_mm`，以及车辆不同的次小 `lo_mm`；self 排除后查询仍 `O(1)`，避免密队列回到全对扫描。重叠占用不得只返回最小 `hi_mm`。
- 前方距离用 follower 的 route occurrence 解释，不用 candidate 自己的路线。
- 同一 candidate 映射多个 future occurrence 时取最小间隙（可负）。
- 构建：`O(B + K + Σ sort(K_bucket))`，`B` 为物理边桶数，`K` 为占用记录数（约为道路交通活动车辆数 × 车身跨边数）。
- 禁止每辆车扫描全体车辆和全局 `O(N_traffic_active^2)`。

占用索引不进入 public API、不允许 Adapter 缓存。测试可保留全扫描预言机，仅 `cfg(test)` 对拍，并按 `bumper_gap_horizon` 过滤，不进生产热路径。

spawn / replace 的重叠检查读已提交 `VehicleState`，仍对 `live_order` 做命令路径扫描，不用本拍占用索引。占用索引只在 `step` 内从 T 重建，生命周期命令之间不增量修补。

## 8. Longitudinal constraints

v0.3 使用 Core 私有、tick-local `LongitudinalConstraintSet`，分为：

- speed ceilings；
- spatial targets。

Spatial target 概念字段包括 source kind、distance ahead、target speed、desired clearance 和 hard clearance。各 subsystem 只从 snapshot 产生 candidate constraint，不直接修改 vehicle state。Reducer 选择最严格约束；稳定 tie-break 只用于 attribution，不使用任意 numeric priority 改变物理优先级。

Physical constraints 不可绕过；regulatory constraints 由后续 policy subsystem 产生。Road capacity、demand、route cost 和全局统计不进入 longitudinal controller。Signals、intersection 和 parking 后续产生 stop/reservation target，不把规则写入 IIDM。

v0.3 不公开 constraint provider，也不允许 Adapter 任意注入 constraint。

Current Parking 复用本节 private spatial-target/safety ownership：ParkingStop 与 SignalStop/RouteEnd/SpeedLimit 从同一 snapshot 生成并按最严格 admissible motion 归约，spatial hard projection 先于 no-overlap projection。Parked vehicle 排除 lane occupancy；Arrived 但未 commit 的 Active vehicle 仍是 stationary leader。完整 event/order/performance 边界见 [`parking-system.md`](parking-system.md)。

#235 的 Accepted 设计
[`waiting-zone-conflict-right-of-way.md`](waiting-zone-conflict-right-of-way.md)
把 GateStop、WaitingZone capacity/storage 与 missing ConflictGrant 定义为新的
Core-owned spatial constraints。它们只能收紧 candidate motion；right-of-way
business priority 与 reducer attribution tie-break 完全分离，grant 不能覆盖
leader、safe-speed、minimum-gap、RouteEnd 或 final no-overlap。该设计还在 grant
前复用 route-local leader/hard-boundary query，证明车尾可清空全部 conflict
coverage，并针对 physical edge/progress 取得 committed + earlier-staged 可见的
downstream claim；出口存储不足或 claim 冲突是 normal no-grant。未落位 claim 不
伪装成可依赖其继续移动的 leader。同 edge 相邻 claims 必须按 progress 识别实际
follower，并复用本设计的 follower-owned minimum-gap tolerance；candidate 在前方时
不能错误使用 candidate profile 代替后方 existing owner。上述 #235 边界尚未
生产化，current longitudinal runtime 不因此改变。

### 8.1 Per-edge 道路限速

- `LaneEdge.speed_limit` 是 immutable 基础道路事实；v0.8 引入且 current v0.10 继承
  `effective_speed_ceiling = base_speed_limit`。
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

无 leader 时使用 `a_free`。后杠间隙超出 `bumper_gap_horizon` 时本拍即无 leader。有 leader 且 `s` 严格大于物理 gap/overlap 阈值时令 `z = s_star / s`：

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

每车每拍按静止前车最坏情况推导两个毫米窗。跟车前视是出现项行走窗，不得缩短；后杠间隙窗决定本拍是否接纳 leader。

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

`minimum_gap_horizon` 保证低速 follower 也能看到本 tick 内可能被侵入 minimum-gap floor 的车辆；专用 tolerance 覆盖 `s0` 边界附近的舍入。整数毫米合同下 `minimum_gap_tolerance` 取 1 mm。后杠间隙大于 `bumper_gap_horizon` 的车辆即使静止，follower 以 `travel_upper` 前进后仍不会低于 `s0`，本拍不接纳为 leader。

当前 `TrafficWorld` 每车每拍先在 SI 中求有限的 `bumper_gap_horizon`，再**向上取整**到毫米，并令 `front_query_horizon = bumper_gap_horizon + MAX_VEHICLE_LENGTH_MM`（溢出饱和，禁止缩短行走窗）。占用查询在跟车前视内行走；接纳以 `bumper_gap_horizon` 为准。占用索引仍按全部 `Active` 重建。ADR 0006 的搜索下界是 `minimum_gap_horizon`，不是跟车前视本身。

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

ADR 0028 的 `hard_room_mm` **不**实现本节 `leader_final_travel`：
它与现行 `advance_active_vehicle` 同构，只用 T 时刻 occupancy 快照的 `min_gap` 后空隙。
current `TrafficWorld` 也尚未做前向后传播；整数毫米切片不在本轴闭合该差距，也不取代
本节作为跟车投影的设计目标。

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

- 已提交占用、边界和间隙用整数毫米比较，不使用米制哨兵，也不得由通用近似比较 helper 代理。
- Vehicle Profile length 的输入下限独立于占用间隙。
- 已提交速度是 `u32` mm/s；IIDM 瞬时 SI 不算已提交权威，不得回写。
- 相对误差和 ULP 不进入 production predicate。
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
- 现有固定步进 `TrafficWorld::step` / `StepOutcome`。

Private：

- OccupancyIndex / OccupancyRecord。
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
- Active/Parked/Completed occupancy。
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

- 一万：每 tick 高精度 Vehicle Following，G3 验收规模。
- 十万：复杂度和扩展性观察，不设置跨机器绝对时间门槛。
- 一百万：城市级容量探索，不承诺当前单线程实时。
- 一百万+：由 #72 设计 partition、parallel、multi-rate 和 mesoscopic/aggregate 模型。

### 16.2 一万协议

- 一万辆道路交通活动车辆，连续 60 个 16 ms fixed ticks。
- 场景：free-flow、dense platoon、stop-and-go；projection-heavy 单独报告。
- 指定 reference desktop 常规场景目标 median `<= 1 ms/tick`。
- G3 硬上限 median `<= 4 ms/tick`，即 60 ticks `<= 240 ms`。
- Benchmark 排除 world/schema 构建和样本重置，固定输入并消费状态/事件。
- 记录 CPU、OS、rustc、release profile 和电源模式。
- CI 运行一万功能 smoke 与 benchmark compile，不使用共享 CI wall-clock assertion。
- 基线建立后，同机受控三轮 median 回退超过 20% 必须分析，超过 30% 默认阻断，除非记录显式例外。

### 16.3 Scaling constraints

- 禁止 external ID hot-path lookup/clone/sort。
- 禁止 per-vehicle heap object 和 dynamic controller dispatch。
- Scratch buffers 必须复用，event 分配只与实际离散事件量相关。
- speed-limit transition metadata 按 route 共享；无 transition 时 fast reject，steady tick 不做 per-vehicle heap allocation。
- 一万到十万不得呈现 `O(V^2)` 趋势。
- v0.2 临时一百万 steady-state 结果只作为乐观研究输入，不构成 v0.3 全市实时声明。

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
- #77：确定性、不变量、一万性能和十万扩展性验证。
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

## 20. v0.8 引入的道路限速约束

#184 将 per-edge road speed limit 纳入同一 `LongitudinalConstraintSet`，不建立第二套车辆控制器。当前 edge 提供 speed ceiling；route 下游更低限速边界提供 advance-braking spatial target，使车辆在 crossing 边界时已不超过新限速。它与 leader/no-overlap、SignalStop 和 route completion 同步求解，不能以 `VehicleProfile.desiredSpeed` 或最后一步 clamp 代替。

spawn/atomic replace 的初始速度不得超过当前 edge 限速；v0.8 引入且 current v0.10
继承默认初始/回流速度为零的行为。限速值、14 条 routes 与测试矩阵见
`example-scenarios.md`；Core API、行为和性能验证由 #185 实施。
