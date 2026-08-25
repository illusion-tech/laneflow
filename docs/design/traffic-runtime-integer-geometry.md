# 交通运行时整数毫米几何

**文档状态**: Review（#496 G1；未 Pass，不授权实现）<br>
**最后更新**: 2026-08-25<br>
**适用范围**: `TrafficWorld` 已提交一维几何与速度、`WorldConfig` 步长、
`laneflow-static-network` 热列、LFCA 长度/速度字段、compiler 边长派生、
Spatial 采样钉死端点<br>
**关联文档**: `../adr/0028-integer-millimeter-traffic-geometry.md`、
`../adr/0003-runtime-tick-and-determinism.md`、
`../adr/0014-residual-aware-f32-core-authority-and-migration-gates.md`、
`../adr/0015-bounded-f32-canonical-spatial-frames.md`、
`../adr/0022-authoring-curve-and-canonical-polyline-error-budgets.md`、
`numeric-representation.md`、`vehicle-following.md`、
`traffic-runtime-shared-consumption.md`、`shared-static-network.md`、
`portable-canonical-artifact.md`、`signal-system.md`、`route-system.md`

本文是 #496 的实现级 G1 输入。它不授权 #302 快照容器、#303 Routing、残差
`f32` 进度或整数 IIDM。G2 完成前 `main` 仍为 current-`f64`；不得把本文写成已落地。

## 1. 结论

已提交交通一维几何与限速改为整数合同：

```text
编制 f64 曲线
  -> 规范 f32 折线 / 弧长          （有折线时；ADR 0022 / 0015，#354 不改）
  -> length_mm = round(弧长或 LIR 交通边长 ×1000)
  -> SharedTrafficNetwork 热列 u32 mm / mm/s
  -> TrafficWorld 进度 u32 mm + carry_um
```

IIDM 仍在 `f32` SI 中算出「这一拍最多走多远」。**先**用整数硬约束得到
`hard_room_mm`，**再**把微米余数加在剩余空间内。硬停（`hard_room_mm == 0`，
或本拍走完该剩余）清速度与余数；`hard_room_mm > 0` 且不足 1 mm 的爬行保留
速度与余数。不另做速度余数；`max_accel >= 0.5 m/s²`。

## 2. 固定步进

| 项           | 合同                                                                                    |
| ------------ | --------------------------------------------------------------------------------------- |
| 合法         | `fixed_delta_time_ms ∈ [4, 1000]`，`install` 外拒绝                                     |
| 最细量子     | 4 ms，**不是**默认                                                                      |
| 画面默认建议 | 16 ms（走廊 / Bevy 示例）                                                               |
| 粗量子       | `>= 100 ms` 合法，不保证跟车观感；1000 ms 为离线/SUMO 级                                |
| 慢放         | Runtime 不提供；Adapter 只可少 `step`，不得改 Δt 或可变 Δt                              |
| 相位         | G2：`duration_ms % dt == 0 && duration_ms >= dt`。G2 前 `main` 仍为 `duration_ms >= dt` |
| `TickInput`  | 必须等于配置，否则世界不变                                                              |

G2 `InstallError`：`dt` 越界 → `DeltaOutOfRange { actual, min: 4, max: 1000 }`；相位短于
一步 → `PhaseShorterThanTick`；相位 `>= dt` 但不能整除 →
`PhaseNotMultipleOfTick { duration_ms, delta_ms }`。顺序：步长区间 → `>= dt` →
`% dt == 0`。不得用 `NonPositiveDelta` 表示 `dt = 3`。

不同合法 Δt 的世界不可对拍。

现行检入走廊 `examples/config/v0.10-signalized-corridor.toml` 的
`fixed_delta_ms = 16`、`yellow_ms = 3000`、`all_red_ms = 1000` 在 G2 相位倍数
规则下非法。G1 不改该 toml、不重生 LFCA；G2 必须把相位改为 16 的正整数倍
（例如 `3008` / `1008`）并重生制品。

## 3. 共享列与 profile

`SharedTrafficNetwork`（有无 Spatial 都写这些热列）：

- `lane_lengths_mm: Box<[u32]>`
- `lane_speed_limits_mm_s: Box<[u32]>`，每项 `1..=100_000`

`VehicleProfileView` 访问器：

- `length_mm() -> u32`，`100..=128_000`
- `desired_speed_mm_s() -> u32`，`1..=100_000`
- `min_gap_mm() -> u32`，`0..=128_000`（0 合法，退化为只禁止重叠）
- `time_headway_seconds()`：受检 `f32`，`0 < value <= 60`
- `max_accel()`：受检 `f32`，`0.5..=50` m/s²
- `comfort_decel()` / `emergency_decel()`：受检 `f32`，`0 < value <= 50` m/s²，且
  `emergency_decel >= comfort_decel`

不得保留权威米制 `length()` / `desired_speed()` / `min_gap()`。

`ParkingSpaceView`：

- `entry()` / `exit()` → `(LaneEdgeOrdinal, u32)` 毫米进度，且 `1 <= p <= length_mm - 1`
- `lateral_offset_mm() -> i32`，`abs <= 128_000`，路外 `abs >= 1`
- `length_mm()` / `width_mm() -> u32`，`100..=128_000`
- `heading_offset_radians() -> f32`

**禁止**齐次 `geometry() -> (f64, f64, f64, f64)` 当权威。

`speed_limit_transitions(route) -> Option<(&[u32], &[LaneEdgeOrdinal], &[u32])>`：
第三列为目标限速 mm/s，与 `lane_speed_limits_mm_s` 同量子。
`next_controlled_distance` 为 `BoundedDistance::Finite(u32)` / `BeyondFinite`。

路线距离按查询窗口独立 checked 加，不上 `u64`，**禁止**因前缀溢出拒绝注册：

- 从路线头起的前缀和溢出 → 仅「从起点算」为 `BeyondFinite`。
- 从当前进度到终点、以及从某 hop 起的局部 `horizon_mm`，都从 **查询起点** 加；
  不吃起点前缀溢出。靠近终点后路终剩余可再 `Finite`，`Finite(0)` 才 `Completed`。
- 不得用 `u32::MAX` 当哨兵。`from_progress_mm` / `horizon_mm` /
  `RouteDistanceQuery::Within` 为 `u32` mm。

占用间隙与跨 hop 跟车空隙用 checked `i64` mm。边上区间仍 `u32`。
`VehicleReplaceBlock.bumper_gap_mm: i64`。

编制进入毫米 / `f32` 表面时 **先量化，再检查**：毫米类
`round-ties-to-even(f64(SI) × 1000)` 后套整数闭包；时距/加减速/朝向先
round-ties-to-even 到 `f32` 再套 `f32` 闭包。禁止先用裸 `0.1 m` 拒绝再量化。
`0.0996 m` → `100 mm` 合法，`0.0994 m` → `99 mm` 失败。跨字段在双方量化后比较。

## 4. 车辆已提交状态

```text
progress_mm: u32           // 当前边上，0..=length_mm
carry_um: u16              // 0..=999
speed_mm_s: u32            // 静止可为 0
length_mm: u32             // spawn/replace 时从 profile 拷入
```

公开观察表面：`progress_mm()` / `carry_um()` / `speed_mm_s()` / `length_mm()`。
**禁止**权威 `length() -> f64`。车长缓存在状态上，不改为每次读 profile。
`VehicleSpawnInput` / `replace_completed_vehicle` 使用 `progress_mm` 与 `speed_mm_s`，
新车 `carry_um = 0`。`PoseSource::Lane` 携带 `progress_mm: u32`，不把米制进度当已提交
权威。可以提供显式只读换算（例如文档化的 `/ 1000`），不得当 `value()`，不得回写。

`progress_mm == length_mm` 合法，表示停在边终点（含拒绝 Gate 前）。替换 /
`Completed` / `Parked`：`carry_um = 0`。跨边保留 `carry_um`，除非跨边后立即硬停。

## 5. 一拍落地

与现行 `advance_active_vehicle` 同构，插入量化点：

1. 把 `progress_mm` / 边长 / 车长 / `min_gap` / 速度转为瞬时 `f32` 米，喂 IIDM
   与限速包络。
2. SI 中舒适截断 travel（前车、灯、路终、包络）。SI `travel <= 0` **不是**硬停。
3. 整数硬约束 `hard_room_mm: u32`（下限 0）。前车空隙用 `i64`，负值当 0，正值夹到
   `u32`。只读 **T 时刻 occupancy 快照**，与现行 `advance_active_vehicle` 同构：
   `step_vehicles` 先按快照算出全部 next，再一次性提交；`hard_room_mm` **不**读本拍
   其他车已算行程，也 **不**加 `vehicle-following.md` §11.2 的
   `leader_final_travel`。
   - 前车 `min_gap` 后空隙（可跨 hop；快照间距恰为 `min_gap` 则本拍 `hard_room_mm == 0`，
     即使前车本拍会走）；
   - `DenyAndStop` 停车线；
   - 本车路线剩余（从当前进度沿路线 checked 加到最后一边终点，与现行
     `remaining_to_route_end` 同构）。`Finite` 参与 min；`BeyondFinite` **不**参与
     `hard_room`，本拍不靠路终硬停，也不得因此 `Completed`。禁止把溢出饱和成
     `u32::MAX`；
   - 下一 hop 存在但 Gate 拒绝时的 `fromEdge` 终点。
   下一 hop 存在且 Gate 许可时，边终点不是硬停。
4. `hard_room_mm == 0`：`travel_mm = 0`，`speed_mm_s = 0`，`carry_um = 0`，不
   `apply_travel`。若此时路线剩余为 `Finite(0)`：`VehicleStatus::Completed`。
   `BeyondFinite` 不是路终。
5. 否则
   `um = u64(carry_um) + round-ties-to-even(f64(travel_m) × 1e6)`；
   `travel_mm = min(um / 1000, hard_room_mm)`；
   若 `travel_mm < hard_room_mm`：`carry_um = (um % 1000) as u16`；
   若 `travel_mm == hard_room_mm`：到位后 `carry_um = 0`。
6. 整数 `apply_travel`：`progress_mm + travel_mm`。满边且下一跳许可则余量进下一条、
   `progress_mm = 0`；满边但下一跳拒绝则停在 `progress_mm == length_mm`，保持
   `Active`。
7. 硬停（第 4 步，或第 5 步走到 `hard_room_mm`）：`speed_mm_s = 0`。若走完的是
   **路线剩余 `Finite(0)`**（没有下一 hop）：`VehicleStatus::Completed`，离开占用与
   `committed_pose_sources`。Gate 拒绝或剩余 `BeyondFinite` 都不是 `Completed`。否则
   `speed_mm_s = round-ties-to-even(f64(next_speed_m) × 1000)`，不由 travel 反推。

占用循环：边上 `remaining_mm == 0` 结束（`u32`）。重叠：有符号整数区间。跨 hop
间隙用 checked `i64`。生产路径删除米制 `1e-9` / `1e-12` 比较。

舍入一律 IEEE 754 **round-ties-to-even**，缩放在 `f64` 中做（`f32 × 1e6` 在
100 m 行程上会丢微米）。

4 ms 静止跟停量化死区（**接受**）：IIDM 有效加速度可以远小于 `max_accel`。静止
follower / leader、空隙 = `min_gap + 0.1 m`、`max_accel = 0.5 m/s²` 时，有效加速度
约 `0.0465 m/s²`，4 ms 行程约 `0.37 µm`、下一速度约 `0.186 mm/s`，量化后 `um` 与
`speed_mm_s` 均为 0，`carry_um` 不增长，状态重复；车辆停在期望间距外约 `100 mm`。
这是 `hard_room_mm > 0` 的爬行退化，不是硬停，也不是 `max_accel < 0.5` 非法 profile。
**禁止**亚微米累加器。建议默认 16 ms 下同工况约 `6 µm`，能进入 `carry_um`。

## 6. compiler 与 LFCA

LFCA / Traffic 热列只存 `U32` 毫米边长。headless **不求弧、也不需要弧**：

```text
有折线:  arc_m: f32
         length_mm = round-ties-to-even(f64(arc_m) × 1000)
无折线:  lir_length_m = CanonicalLaneEdgeView::length_meters()
         length_mm = round-ties-to-even(f64(lir_length_m) × 1000)
require 100 <= length_mm <= 10_000_000
```

两条路径都写入 `lane_lengths_mm`。禁止从 `lane_pose()` 或空 Spatial 表反推边长。
无 Spatial 时不得走车辆 pose 采样。

限速：`speed_limit_mm_s = round-ties-to-even(f64(m/s) × 1000)`，且
`1..=100_000`。

G2 分配 **LFCA v2**：对象前导 `formatVersion` 与
`ContractVersions.canonicalFormatVersion` 为 `2`；
`constraintContractVersion` / `staticExecutionContractVersion` 为 `2`；
`networkRevisionDerivationVersion` **保持 `1`**（§4.2 v1 组帧与
`"laneflow.network-revision.v1\0"` 未改；毫米载荷会改变 ID，不必新算法）；
身份两字段保持 `1`。不得改写 v1 登记表。v1 读器拒绝 v2，v2 读器拒绝 v1。
LFSM `canonicalArtifactFormatVersion` 等于所绑 LFCA 版本。

`SharedNetworkRevision` 的受检输入必须并行升到 v2，不得放宽现行 V1 入口：

- G2 前 `main` 仍只承认 `CheckedCanonicalNetworkInputV1` /
  `check_canonical_network_input_v1` /
  `PostEmissionCheckedBundleV1::canonical_network_input`，且 object kind 精确为
  LFCA v1。
- G2 分配 `CheckedCanonicalNetworkInputV2`、`check_canonical_network_input_v2`、
  `check_post_emission_bundle_v2`、
  `PostEmissionCheckedBundleV2::canonical_network_input() -> CheckedCanonicalNetworkInputV2`。
  object kind 精确为 LFCA v2；走 v2 registry 预检；字段私有、不可伪造。
  `PostEmissionCheckedBundleV1` 不得派生 V2。
- 生产构建 G2 后只消费 V2。草图与能力规则见 `shared-static-network.md` §3.1。
- 发布：`commit_portable_publication_v2`（名字里的 v2 是 LFCP）G2 后对 LFCA v2
  候选走 `check_post_emission_bundle_v2`；`build_lfcp_v2` 消费
  `PostEmissionCheckedBundleV2`。禁止 V1 bundle 发布 v2 制品。

LFCA v2 登记表增量（相对附录 A.1 v1；**不兼容**读旧 `f64`）。**只改下表各行**；未列出的
字段保持 v1 的 tag、名字、类型、必填。不得改写 v1 表，不得用「等长度」打包。
`fieldType`：`3=u32`、`5=f32`、`13=i32`。Spatial `LaneEdgeGeometry.arcLengthMeters` /
`segments.lengthMeters` 仍为 `f32` 米，不进本表。

| 表             | tableKind | tag | v1 `name:type:R`                                      | v2 `name:type:R`                                      |
| -------------- | --------- | --- | ----------------------------------------------------- | ----------------------------------------------------- |
| LaneEdge       | `0x0004`  | 3   | `lengthMeters:f64:R`                                  | `lengthMillimetres:u32:R`                             |
| LaneEdge       | `0x0004`  | 4   | `speedLimitMetersPerSecond:f64:R`                     | `speedLimitMillimetresPerSecond:u32:R`                |
| ParkingSpace   | `0x000f`  | 5   | `entryProgressMeters:f64:R`                           | `entryProgressMillimetres:u32:R`                      |
| ParkingSpace   | `0x000f`  | 7   | `exitProgressMeters:f64:R`                            | `exitProgressMillimetres:u32:R`                       |
| ParkingSpace   | `0x000f`  | 8   | `lateralOffsetMeters:f64:R`                           | `lateralOffsetMillimetres:i32:R`                      |
| ParkingSpace   | `0x000f`  | 9   | `headingOffsetRadians:f64:R`                          | `headingOffsetRadians:f32:R`                          |
| ParkingSpace   | `0x000f`  | 10  | `lengthMeters:f64:R`                                  | `lengthMillimetres:u32:R`                             |
| ParkingSpace   | `0x000f`  | 11  | `widthMeters:f64:R`                                   | `widthMillimetres:u32:R`                              |
| VehicleProfile | `0x0014`  | 4   | `lengthMeters:f64:R`                                  | `lengthMillimetres:u32:R`                             |
| VehicleProfile | `0x0014`  | 5   | `desiredSpeedMetersPerSecond:f64:R`                   | `desiredSpeedMillimetresPerSecond:u32:R`              |
| VehicleProfile | `0x0014`  | 6   | `minGapMeters:f64:R`                                  | `minGapMillimetres:u32:R`                             |
| VehicleProfile | `0x0014`  | 7   | `timeHeadwaySeconds:f64:R`                            | `timeHeadwaySeconds:f32:R`                            |
| VehicleProfile | `0x0014`  | 8   | `maxAccelerationMetersPerSecondSquared:f64:R`         | `maxAccelerationMetersPerSecondSquared:f32:R`         |
| VehicleProfile | `0x0014`  | 9   | `comfortableDecelerationMetersPerSecondSquared:f64:R` | `comfortableDecelerationMetersPerSecondSquared:f32:R` |
| VehicleProfile | `0x0014`  | 10  | `emergencyDecelerationMetersPerSecondSquared:f64:R`   | `emergencyDecelerationMetersPerSecondSquared:f32:R`   |

量化后闭包（先 round，再检查；跨字段在双方量化后比较）：

| v2 字段                                                        | 闭包                                           |
| -------------------------------------------------------------- | ---------------------------------------------- |
| `LaneEdge.lengthMillimetres`                                   | `100..=10_000_000`                             |
| `LaneEdge.speedLimitMillimetresPerSecond`                      | `1..=100_000`                                  |
| `ParkingSpace.entryProgressMillimetres`                        | 所引入口边量化后边长 `L`：`1 <= p <= L - 1`    |
| `ParkingSpace.exitProgressMillimetres`                         | 所引出口边量化后边长 `L`：`1 <= p <= L - 1`    |
| `ParkingSpace.lateralOffsetMillimetres`                        | `abs <= 128_000`；路外 `abs >= 1`              |
| `ParkingSpace.headingOffsetRadians`                            | `-π <= x < π`；`π` 为 binary32 `0x40490fdb`    |
| `ParkingSpace.lengthMillimetres` / `widthMillimetres`          | 各自 `100..=128_000`                           |
| `VehicleProfile.lengthMillimetres`                             | `100..=128_000`                                |
| `VehicleProfile.desiredSpeedMillimetresPerSecond`              | `1..=100_000`                                  |
| `VehicleProfile.minGapMillimetres`                             | `0..=128_000`                                  |
| `VehicleProfile.timeHeadwaySeconds`                            | `0 < x <= 60`                                  |
| `VehicleProfile.maxAccelerationMetersPerSecondSquared`         | `0.5..=50`                                     |
| `VehicleProfile.comfortableDecelerationMetersPerSecondSquared` | `0 < x <= 50`                                  |
| `VehicleProfile.emergencyDecelerationMetersPerSecondSquared`   | `0 < x <= 50`，且 `>= comfortableDeceleration` |

有折线时，量化后的 `lengthMillimetres` 与 `f32` 弧长仍用现行米制容差对账：
`abs(f64(length_mm) / 1000 - f64(arc_m)) <= max(0.01 m, 1.0e-6 * max(length_m, f64(arc)))`。
对账失败关闭，不改写已量化边长。headless 无此对账。

后发射检查失败关闭旧 v1 字节。走廊检入 LFCA 必须按 v2 重生并对拍。
`NetworkRevisionId` 随载荷变化。

`laneflow-static-contract` 常量：最短尺寸 `100` mm，端点留白 `1` mm，删除作为
生产判定的 `1.0e-9` 米常量（或标为历史、不再被 Runtime/compiler 引用）。

## 7. Spatial

不改 canonical `f32` 点。采样：

`PoseSource::Lane` 把 `progress_mm` 交给采样（不是米）：

- `progress_mm == 0` → 折线起点；
- `progress_mm == length_mm` → 折线终点；
- 中间 `geometry_s = (progress_mm as f64 / length_mm as f64) * f64(arc)` 再
  转为 `f32`。

横向停车偏移：`offset_mm as f32 / 1000.0`。

`SpatialError::SharedProgressOutOfRange`：`progress_mm: u32`、`max_mm: u32`。
`BuildError::SpatialLengthMismatch`：`traffic_length_mm: u32`、
`spatial_length_meters: f32`。米制交通长度只作诊断。

无 Spatial 时 `bind` 仍返回 `Ok(None)`，不建 session；一维边长权威不受影响。

## 8. 确定性与验收

- 已提交 mm / mm/s / `carry_um`：同一软件版本、同一运行环境、同一初始状态和
  同一 tick/input 序列必须一致（ADR 0003）。**不**承诺跨 CPU / 跨机器位级相同。
- IIDM `f32` 与规范弧长 `f32`：不承诺跨 CPU 位级相同。余数输入依赖它们，因此
  整数状态也不另作跨 CPU 承诺。
- 同进程并行（同一二进制、同一运行环境）必须与 1-worker 得到相同已提交状态与
  事件序。联机 / 跨机器 lockstep 不在本切片。
- 不得要求与 current-`f64` 录影零分歧，也不得用 2 车走廊墙钟当 Product Pass。
- 必测：硬停清余数与速度；`hard_room_mm > 0` 且量化 travel `< 1 mm` 时速度保持、
  余数增长；跨边余数保留；拒绝 Gate 时停在 `fromEdge` 终点、保持 `Active`、
  不清错边；走到路线终点进入 `Completed` 并离开占用 / pose；`max_accel < 0.5`
  失败；headless 无折线时 `length_mm` 来自 LIR 交通长度 round，不要求弧；
  跨 hop 占用间隙用 `i64`；路线距离查询为 `u32` mm / `BeyondFinite`；
  `VehicleReplaceBlock.bumper_gap_mm` 为 `i64`；spawn `carry_um = 0`；
  `PoseSource::Lane` 为 `progress_mm`；边限速 `> 100_000` mm/s 失败；v1 LFCA
  在 v2 读器上失败关闭；v2 LFCA 在 `check_canonical_network_input_v1` 上失败关闭；
  v1 LFCA 在 `check_canonical_network_input_v2` 上失败关闭；`PostEmissionCheckedBundleV1`
  不能得到 V2 capability；
  4 ms 静止跟停死区：`min_gap + 0.1 m`、双方静止、`max_accel = 0.5` 时本拍
  `travel_mm == 0`、`speed_mm_s == 0`、`carry_um == 0` 且下一拍状态重复，不是失败；
  快照 `hard_room`：follower 与 leader 间距恰为 `min_gap` 时本拍硬停，即使 leader
  本拍会走（与现行 `advance_active_vehicle` 同构）；
  前缀累计超过 `u32::MAX` mm 时 `register_route` / StaticRoute 仍成功；从起点的
  查询 `BeyondFinite`；从靠近终点的 cursor 走到终点为 `Finite`，`Finite(0)` 进入
  `Completed`；局部视距不吃起点前缀溢出；
  `0.0996 m` 车长量化为 `100 mm` 并接受；`0.0994 m` 量化为 `99 mm` 并失败；
  v2 读器按上表逐字段核对 tag/名字/类型，缺字段或把未改字段写成新类型失败关闭；
  v2 LFCA 的 `networkRevisionDerivationVersion == 1`，ID 用 v1 域分隔符重算；
  `commit_portable_publication_v2` 对 v2 候选走 V2 bundle；
  `VehicleState::length_mm()` 为 `u32`；`ParkingSpaceView` 无齐次 `f64` geometry；
  `speed_limit_transitions` 目标为 `u32` mm/s；
  `SharedProgressOutOfRange` / `SpatialLengthMismatch` 交通侧为 mm；
  `dt=3` 为 `DeltaOutOfRange`，相位不能整除为 `PhaseNotMultipleOfTick`；
  `60 km/h` 长期平均速度由余数对齐量化后的 `mm/s`，无系统少走；相位非倍数
  `install` 失败；`dt=3` 失败；`dt=4` 与 `dt=1000` 均能 install（夹具相位允许时）。

## 9. 明确不做

- #302 容器字段布局（但必须预留本整数状态）。
- #303、#354 三维 math kernel。
- 残差 `f32` 进度、整数 IIDM、无余数格子、速度余数。
- Runtime 慢放 API。
- 跨 CPU / 跨机器位级回放或联机 lockstep。
- G1 改走廊 toml 或重生 LFCA。
- 强迫 headless 从折线弧长派生边长。
- 改写已冻 LFCA v1 登记表。
- 放宽 V1 admission 接纳 LFCA v2，或不冻结并行 `CheckedCanonicalNetworkInputV2`。
- 亚微米累加器，或把 4 ms 静止跟停量化死区当缺陷消掉。
- 把 `vehicle-following.md` §11.2 的 `leader_final_travel` 并入本切片 `hard_room_mm`。
- 用「等长度」打包 v2 字段，或改写附录 A.1 的 v1 表。
- 前缀超过 `u32::MAX` mm 时注册失败，或把 `BeyondFinite` 饱和成 `u32::MAX` 路终硬停。
- 从路线头溢出后把后续后缀查询一律标成 `BeyondFinite`。
- 空升 `networkRevisionDerivationVersion = 2` 却不改哈希算法。
- 发布路径把 LFCA v2 送进 V1 bundle 检查。
- `VehicleState::length()`、`ParkingSpaceView::geometry()` 或限速过渡目标继续用米制权威。
- 用 `NonPositiveDelta` / `PhaseShorterThanTick` 表示步长越界或相位不能整除。
- 先按量化前的裸 SI 界限拒绝，再 round 到毫米 / `f32`。
