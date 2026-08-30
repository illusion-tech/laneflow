# 交通运行时整数毫米几何

**文档状态**: Review<br>
**最后更新**: 2026-08-30<br>
**适用范围**: `TrafficWorld` 已提交一维几何与速度、`WorldConfig` 步长、
`laneflow-static-network` 热列、LFCA 长度/速度字段、compiler Typed AST / HIR /
MIR / LIR 交通一维存储、公开 `Canonical*View`、compiler 边长派生、
Spatial 采样钉死端点<br>
**关联文档**: `../adr/0028-integer-millimeter-traffic-geometry.md`、
`../adr/0003-runtime-tick-and-determinism.md`、
`../adr/0014-residual-aware-f32-core-authority-and-migration-gates.md`、
`../adr/0015-bounded-f32-canonical-spatial-frames.md`、
`../adr/0022-authoring-curve-and-canonical-polyline-error-budgets.md`、
`numeric-representation.md`、`vehicle-following.md`、
`traffic-runtime-shared-consumption.md`、`shared-static-network.md`、
`portable-canonical-artifact.md`、`signal-system.md`、`route-system.md`

本文是 #496 已落地的 Runtime / 制品合同，以及 #500 已落地的编译器 IR 交通一维合同。
它不覆盖 #302 快照容器、#303 Routing、残差 `f32` 进度或整数 IIDM。当前树制品、
Runtime 与编译器 IR 只承认整数毫米一维几何；编制 `f64` 与 Spatial `f32` 仍在量化之前。

G1 冻权威、单位、量化顺序、制品字段与跨实现算法。公开 `Canonical*View`
交通一维删除米制访问器，不留只读换算。

## 1. 结论

已提交交通一维几何与限速改为整数合同：

```text
编制 f64 曲线 / 编制交通 SI
  -> 准入量化一次（mm / 受检 f32 SI）
  -> Typed AST / HIR / MIR / LIR 交通一维只带整数毫米
  -> 有折线：length_mm/1000 对账规范 f32 弧长；通过后提交 round(弧长)
     无折线：提交准入毫米
  -> LFCA / 共享热列 / Canonical 视图 / TrafficWorld 同一套 u32 mm
```

IIDM 仍在 `f32` SI 中算出「这一拍最多走多远」。**先**用整数硬约束得到
`hard_room_mm`，**再**把微米余数加在剩余空间内。硬停（`hard_room_mm == 0`，
或本拍走完该剩余）清速度与余数；`hard_room_mm > 0` 且不足 1 mm 的爬行保留
速度与余数。不另做速度余数；`max_accel` / `comfort_decel` / `emergency_decel` 均 `>= 0.5 m/s²`。

## 2. 固定步进

| 项           | 合同                                                       |
| ------------ | ---------------------------------------------------------- |
| 合法         | `fixed_delta_time_ms ∈ [4, 1000]`，`install` 外拒绝        |
| 最细量子     | 4 ms，**不是**默认                                         |
| 画面默认建议 | 16 ms（走廊 / Bevy 示例）                                  |
| 粗量子       | `>= 100 ms` 合法，不保证跟车观感；1000 ms 为离线/SUMO 级   |
| 慢放         | Runtime 不提供；Adapter 只可少 `step`，不得改 Δt 或可变 Δt |
| 相位         | `duration_ms % dt == 0 && duration_ms >= dt`               |
| 每次步进输入 | 必须等于配置，否则世界不变                                 |

`install` 必须能区分：步长越出 `4..=1000`、相位短于一步、相位不能整除。校验顺序：
步长区间 → 时长 `>= dt` → 整除。错误类型为 `InstallError::DeltaOutOfRange` /
`PhaseShorterThanTick` / `PhaseNotMultipleOfTick`。

不同合法 Δt 的世界不可对拍。

检入走廊 `examples/config/v0.10-signalized-corridor.toml` 使用
`fixed_delta_ms = 16`、`yellow_ms = 3008`、`all_red_ms = 1008`。制品
`formatVersion` 随 ADR 0029 为 `3`。

## 3. 共享列与 profile

共享交通热列（有无 Spatial 都写）：边长 `u32` mm，边限速 `u32` mm/s 且
`1..=100_000`。

profile：车长 `100..=128_000` mm，期望车速 `1..=100_000` mm/s，`min_gap`
`0..=128_000` mm（0 合法，退化为只禁止重叠）；时距与三项加减速为受检 `f32` SI，
范围见 ADR 0028。停车：入口/出口进度 `u32` mm 且相对 **提交后** `length_mm` 满足
`1 <= p <= length_mm - 1`；长宽
`100..=128_000` mm；横向 `i32` mm，`abs <= 128_000`，路外 `abs >= 1`；朝向受检
`f32` 弧度，闭包 `-π <= x < π`。编制/准入量化后若等于 `+π`（`0x40490fdb`）则写成
`-π`（`0xc0490fdb`）；制品存着的 `+π` 非法，读器只拒不折。限速过渡目标与边限速
同一量子（mm/s）。到下一受控门的距离与路线有界距离同型（`Finite(u32)` /
`BeyondFinite`）。`BeyondFinite` 的降速目标本拍不参与包络。

路线距离按查询窗口独立 checked 加，不上 `u64`，**禁止**因前缀溢出拒绝注册。索引
若分段：段内偏移与段合计是 `u32` mm；下一条边长会让当前段溢出时封段、开新段。
不得用 `u32::MAX` 当哨兵。从起点跨段总和仍可 `BeyondFinite`；从当前进度到终点、
以及局部视距，都从查询起点加。靠近终点后路终剩余可再 `Finite`，`Finite(0)` 才
`Completed`。路终剩余读后缀有界距离或分段窗口差，不靠饱和起点前缀相减。城市一趟
行程（Spatial 单 frame 约 32 km，通勤/过境几十公里）落在约 4295 km 的 `u32`
窗口内；不要为理论最长边序列把 Finite 侧加宽到 `u64`。占用间隙的 checked `i64`
只服务有符号空隙，不是路线前缀先例。实现落在 `BoundedDistance` 与
`segmented_route_coordinates`。

占用间隙、跨 hop 空隙与替换阻塞间隙用 checked `i64` mm。边上区间仍 `u32`。

编制进入毫米 / `f32` 表面时 **先量化，再检查**：毫米类
`round-ties-to-even(f64(SI) × 1000)` 后套整数闭包；时距/加减速/朝向先
round-ties-to-even 到 `f32` 再套 `f32` 闭包。禁止先用裸 `0.1 m` 拒绝再量化。
`0.0996 m` → `100 mm` 合法，`0.0994 m` → `99 mm` 失败。编制/准入：朝向量化后若等于
`+π` 则写成 `-π` 再检查；读器遇到存着的 `+π` 失败关闭。跨字段比较已量化的双方。

## 4. 车辆已提交状态

```text
progress_mm: u32           // 当前边上，0..=length_mm
carry_um: u16              // 0..=999
speed_mm_s: u32            // 静止可为 0
length_mm: u32             // spawn/replace 时从 profile 拷入
```

公开观察与命令表面以上述整数为权威，含 spawn / 替换 / 车道 pose。新车
`carry_um = 0`。车长可缓存在状态上。只读米制换算可以有，不得当权威、不得回写。
G2 决定访问器名字。

`progress_mm == length_mm` 合法，表示停在边终点（含拒绝 Gate 前）。替换 /
`Completed` / `Parked`：`carry_um = 0`。跨边保留 `carry_um`，除非跨边后立即硬停。

## 5. 一拍落地

与现行 `advance_active_vehicle` 同构，插入量化点：

1. 把 `progress_mm` / 边长 / 车长 / `min_gap` / 速度转为瞬时 `f32` 米，喂 IIDM
   与限速包络。
2. SI 中舒适截断 travel（前车、灯、路终、包络）。SI `travel <= 0` **不是**硬停。
   一维不倒车：负 SI travel 当 0，不扣 `carry_um`。`BeyondFinite` 降速目标本拍不参与
   包络。
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
   `um = u64(carry_um) + round-ties-to-even(f64(非负 travel_m) × 1e6)`；
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
   整数行程落地后，已提交速度不得超过**所在边**限速（`carry_um` 可能比 SI 包络
   多送 1 mm 跨边）。如何夹紧属 G2。

占用循环：边上 `remaining_mm == 0` 结束（`u32`）。重叠：有符号整数区间。跨 hop
间隙用 checked `i64`。生产路径删除米制 `1e-9` / `1e-12` 比较。

舍入一律 IEEE 754 **round-ties-to-even**，缩放在 `f64` 中做（`f32 × 1e6` 在
100 m 行程上会丢微米）。

4 ms 量化死区（**接受**，不为它加状态或改量化）：

- **静止跟停**：IIDM 有效加速度可以远小于 `max_accel`。静止 follower / leader、
  空隙 = `min_gap + 0.1 m`、`max_accel = 0.5 m/s²` 时，有效加速度约
  `0.0465 m/s²`，4 ms 行程约 `0.37 µm`、下一速度约 `0.186 mm/s`，量化后 `um` 与
  `speed_mm_s` 均为 0，`carry_um` 不增长，状态重复；车辆停在期望间距外约
  `100 mm`。这是 `hard_room_mm > 0` 的爬行退化，不是硬停，也不是
  `max_accel < 0.5` 非法 profile。建议默认 16 ms 下同工况约 `6 µm`，能进入
  `carry_um`。
- **空路巡航**：IIDM `a = max_accel × (1 − (v/v0)⁴)` 接近期望车速时，有效加速度
  可小于 `0.5 mm/s / Δt`。`dt = 4 ms`、`max_accel = 0.5 m/s²`、期望 `20 m/s` 时，
  已提交速度稳定在约 `18.61 m/s`（约低 7%），下一增量约 `0.4997 mm/s`，
  round-ties-to-even 后整数不变。车仍前进；`carry_um` 只攒距离，不能抬速度。
  相对偏差只取决于 `max_accel` 与步长。建议默认 16 ms 下同画像约低 1.6%。

**禁止**亚微米累加器、速度余数、或把该死区当 G2 必须消掉的缺陷。死区是 4 ms
最细量子的产品接受面。

## 6. compiler 与 LFCA

#500：编制 `f64` 只存在准入边界之前。准入量化一次之后，Typed AST / HIR / MIR / LIR
的交通一维存储权威是整数毫米；时距、三项加减速、朝向是受检 `f32` SI。不得把原来的
`f64` 留下再在发射 round。公开访问器为 `length_mm` / `speed_limit_mm_s` /
`progress_mm` / `lateral_offset_mm` / `desired_speed_mm_s` / `min_gap_mm`。

交通一维在 IR 中的量：

| 记录             | 整数毫米 / `mm/s`         | 仍为受检 `f32` SI |
| ---------------- | ------------------------- | ----------------- |
| `LaneEdge`       | 边长、限速                | —                 |
| `VehicleProfile` | 车长、期望车速、`min_gap` | 时距、三项加减速  |
| 停车锚点 / 矩形  | 进度、横向 `i32` mm、长宽 | 朝向              |

LFCA / Traffic 热列只存 `U32` 毫米边长。headless **不求弧、也不需要弧**。禁止从位姿
采样或空 Spatial 表反推边长。无 Spatial 时不得走车辆 pose 采样。

编译器边长提交（#500；有折线把 IR 写成 LFCA 将写出的值）：

```text
准入:     declared_mm = round-ties-to-even(f64(SI) × 1000)
          require 100 <= declared_mm <= 10_000_000
有折线:   obs_m = f64(declared_mm) / 1000
          abs(obs_m - f64(arc_m))
            <= max(0.01 m, 1.0e-6 * max(obs_m, f64(arc_m)))
          对账失败关闭（观察值，不把米列当第二权威）
          committed_mm = round-ties-to-even(f64(arc_m) × 1000)
          committed_mm 越出 100..=10_000_000
            → 边长越界失败关闭（编译侧，不是发射 binding 错误）
          IR length_mm := committed_mm
无折线:   IR length_mm := declared_mm
LFCA:     写 IR length_mm（此时已与将写出值同一整数）
```

`arc_m` 是规范 `f32` 弧长，仍由 ADR 0015 拥有，不改成毫米。停车锚点相对 **提交后的**
`length_mm` 做整数关闭（`1 <= p <= L - 1`，端点留白 `1` mm）。有折线不得再用准入毫米
或另一份米列量化放行。停车锚点相对已提交 IR `length_mm` 做整数关闭。

**共享路网构建（无 LIR）** 令 `length_m = f64(lengthMillimetres) / 1000`，再
`abs(length_m - f64(arcLengthMeters)) <= max(0.01 m, 1.0e-6 * max(length_m, f64(arc))) + 0.0 m`。
对账失败关闭，不改已量化边长。headless 无此对账。当前树没有 `lengthMeters` 交通列。
该对账与编译器空间冻结同构：两边都用毫米→米观察值对照 `f32` 弧长。

限速在准入量化为 `1..=100_000` mm/s 后原样进入 IR 与 LFCA，不再二次 round。

公开 `Canonical*View`（#500）：交通一维只暴露毫米 / `mm/s`（及受检 `f32` 时距 /
加减速 / 朝向）。**删除** `length_meters()` 等米制访问器，不留只读换算。毫米访问器为
`length_mm`、`speed_limit_mm_s`、`progress_mm`、`lateral_offset_mm`、
`desired_speed_mm_s`、`min_gap_mm`。

制品合同：对象前导 `formatVersion` 与
`ContractVersions.canonicalFormatVersion` 为 `4`；
`constraintContractVersion` 为 `2`；`staticExecutionContractVersion` 为 `4`；
`networkRevisionDerivationVersion` **保持 `1`**（§4.2 组帧与
`"laneflow.network-revision.v1\0"` 未改；毫米载荷与路线表删除会改变 ID，不必新算法）；
`identityEncodingVersion` 保持 `1`，`identityRegistryRevision = 3`。
公开 API 不带世代后缀。读器拒绝 `formatVersion != 4`。
旧米制表与旧读器不进当前树。LFSM `sourceMapFormatVersion = 3`，
`canonicalArtifactFormatVersion` 等于所绑 LFCA。LFSD `semanticDiffFormatVersion = 3`。
Genesis 的 target 合同行必须与所绑 LFCA 一致；Artifact 两端合同行仍须相等。走廊按
Genesis 重生，不做格式迁移 diff。

共享路网受检输入与后发射检查只走这一套 registry，不可伪造。

当前 LFCA 交通热列（**不兼容**旧 `f64` 米列）。**只改下表各行相对历史米制表的名字和/或类型**；
未列出的字段保持原 tag、名字、类型、必填。不得用「等长度」打包。
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

闭包（准入先量化再检查；停车进度相对 **提交后** 边长比较）：

| v2 字段                                                        | 闭包                                                                     |
| -------------------------------------------------------------- | ------------------------------------------------------------------------ |
| `LaneEdge.lengthMillimetres`                                   | `100..=10_000_000`                                                       |
| `LaneEdge.speedLimitMillimetresPerSecond`                      | `1..=100_000`                                                            |
| `ParkingSpace.entryProgressMillimetres`                        | 所引入口边 **提交后** 边长 `L`：`1 <= p <= L - 1`                        |
| `ParkingSpace.exitProgressMillimetres`                         | 所引出口边 **提交后** 边长 `L`：`1 <= p <= L - 1`                        |
| `ParkingSpace.lateralOffsetMillimetres`                        | `abs <= 128_000`；路外 `abs >= 1`                                        |
| `ParkingSpace.headingOffsetRadians`                            | `-π <= x < π`；存着的 `+π`（`0x40490fdb`）非法；编制/准入量化后写成 `-π` |
| `ParkingSpace.lengthMillimetres` / `widthMillimetres`          | 各自 `100..=128_000`                                                     |
| `VehicleProfile.lengthMillimetres`                             | `100..=128_000`                                                          |
| `VehicleProfile.desiredSpeedMillimetresPerSecond`              | `1..=100_000`                                                            |
| `VehicleProfile.minGapMillimetres`                             | `0..=128_000`                                                            |
| `VehicleProfile.timeHeadwaySeconds`                            | `0 < x <= 60`                                                            |
| `VehicleProfile.maxAccelerationMetersPerSecondSquared`         | `0.5..=50`                                                               |
| `VehicleProfile.comfortableDecelerationMetersPerSecondSquared` | `0.5..=50`                                                               |
| `VehicleProfile.emergencyDecelerationMetersPerSecondSquared`   | `0.5..=50`，且 `>= comfortableDeceleration`                              |

后发射检查失败关闭旧 v1 字节。走廊检入 LFCA 必须按 v2 重生并对拍。
`NetworkRevisionId` 随载荷变化（算法仍是 §4.2 v1）。

`laneflow-static-contract` 常量：最短尺寸 `100` mm，端点留白 `1` mm。生产判定使用
这些整数常量，不再引用米制哨兵。

## 7. Spatial

不改 canonical `f32` 点。车道采样输入是 `progress_mm`，不是米：

- `progress_mm == 0` → 折线起点；
- `progress_mm == length_mm` → 折线终点；
- 中间按进度占边长的**实数**比例映射到弧长，再转为 `f32`。`progress_mm` 与
  `length_mm` 不得做整数除。乘除顺序属实现，G2 对齐现行 Spatial 采样。

横向停车偏移按毫米除以 1000 得到米再采样。进度越界与交通/弧长对账失败时，错误载荷
的交通侧用毫米，弧长侧仍是 `f32` 米。无 Spatial 时不建采样 session；一维边长权威
不受影响。

## 8. 确定性与验收

- 已提交 mm / mm/s / `carry_um`：同一软件版本、同一运行环境、同一初始状态和
  同一 tick/input 序列必须一致（ADR 0003）。**不**承诺跨 CPU / 跨机器位级相同。
- IIDM `f32` 与规范弧长 `f32`：不承诺跨 CPU 位级相同。余数输入依赖它们，因此
  整数状态也不另作跨 CPU 承诺。
- 同进程并行（同一二进制、同一运行环境）必须与 1-worker 得到相同已提交状态与
  事件序。联机 / 跨机器 lockstep 不在本切片。
- 不得要求与 current-`f64` 录影零分歧，也不得用 2 车走廊墙钟当 Product Pass。
- 必测（按行为，不按访问器名字）：硬停清余数与速度；爬行时速度保持、余数增长；
  跨边余数保留；拒绝 Gate 时停在边终点且保持 `Active`；走到路线终点进入
  `Completed`；`max_accel < 0.5` 失败；headless 边长即准入毫米；有折线边长即提交后的
  弧长量化毫米，且等于 LFCA `lengthMillimetres`；停车锚点贴着弧长量化后的端点留白；
  10 km 边在编译侧与发射侧同一闭包；弧长量化越出 `100..=10_000_000` mm 以边长越界
  失败，不是发射 binding 错误；跨 hop 间隙 `i64`；路线注册在前缀溢出时仍成功，从起点
  可 `BeyondFinite`，靠近终点可 `Finite(0)` 并 `Completed`；`0.0996 m` → `100 mm`
  合法、`0.0994 m` → `99 mm` 失败；`formatVersion != 4` 失败关闭；
  `networkRevisionDerivationVersion == 1`；4 ms 跟停死区状态重复不是失败；快照
  `hard_room` 与现行截断同构；`dt=3` 与相位不能整除均失败且原因可区分；`dt=4` 与
  `dt=1000` 能 install（夹具相位允许时）；`60 km/h` 长期平均由余数对齐量化后的
  `mm/s`。

## 9. 明确不做

- #302 容器字段布局（但必须预留本整数状态）。
- #303、#354 三维 math kernel。
- 残差 `f32` 进度、整数 IIDM、无余数格子、速度余数。
- Runtime 慢放 API。
- 跨 CPU / 跨机器位级回放或联机 lockstep。
- G1 改走廊 toml 或重生 LFCA。
- 强迫 headless 从折线弧长派生边长。
- 在当前树保留米制 LFCA 登记表、旧读器或 `*_v1`/`*_v2` 孪生公开 API。
- 把旧米列读成毫米，或为未发布旧字节堆兼容层。
- 亚微米累加器，或把 4 ms 静止跟停量化死区当缺陷消掉。
- 把 `vehicle-following.md` §11.2 的 `leader_final_travel` 并入本切片 `hard_room_mm`。
- 用「等长度」打包交通热列字段，或在当前树并行保留历史米制表。
- 前缀超过 `u32::MAX` mm 时注册失败，或把 `BeyondFinite` 饱和成 `u32::MAX` 路终硬停。
- 从路线头溢出后把后续后缀查询一律标成 `BeyondFinite`。
- 空升 `networkRevisionDerivationVersion = 2` 却不改哈希算法。
- 发布或构建把 LFCA v2 送进只承认 v1 的入口。
- 公开一维表面继续用米制作权威。
- 公开 `Canonical*View` 为交通一维保留米制只读换算。
- 把 G1 写成现行每个 Rust 访问器的签名对照表。
- 先按量化前的裸 SI 界限拒绝，再 round 到毫米 / `f32`。
- 把规范折线点、段长、弧长或时间/字节长度改成毫米权威。
- #500 G1 改生产代码或重生夹具。
