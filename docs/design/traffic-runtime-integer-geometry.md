# 交通运行时整数毫米几何

**文档状态**: Review（#496 G1；未 Pass，不授权实现）<br>
**最后更新**: 2026-08-24<br>
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
`portable-canonical-artifact.md`、`signal-system.md`

本文是 #496 的实现级 G1 输入。它不授权 #302 快照容器、#303 Routing、残差
`f32` 进度或整数 IIDM。G2 完成前 `main` 仍为 current-`f64`；不得把本文写成已落地。

## 1. 结论

已提交交通一维几何与限速改为整数合同：

```text
编制 f64 曲线
  -> 规范 f32 折线 / 弧长          （ADR 0022 / 0015，#354 不改）
  -> length_mm = round(arc_m×1000) （compiler 内部弧；headless 同样写入热列）
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

不同合法 Δt 的世界不可对拍。

现行检入走廊 `examples/config/v0.10-signalized-corridor.toml` 的
`fixed_delta_ms = 16`、`yellow_ms = 3000`、`all_red_ms = 1000` 在 G2 相位倍数
规则下非法。G1 不改该 toml、不重生 LFCA；G2 必须把相位改为 16 的正整数倍
（例如 `3008` / `1008`）并重生制品。

## 3. 共享列与 profile

`SharedTrafficNetwork`（有无 Spatial 都写这些热列）：

- `lane_lengths_mm: Box<[u32]>`
- `lane_speed_limits_mm_s: Box<[u32]>`，每项 `> 0`，`<= 100_000`

`VehicleProfileView`：

- `length_mm: u32`，`>= 100`
- `desired_speed_mm_s: u32`，`> 0`，`<= 100_000`
- `min_gap_mm: u32`（0 合法，退化为只禁止重叠）
- `time_headway_seconds`、`max_accel`、`comfort_decel`、`emergency_decel`：受检 `f32` SI
- `max_accel >= 0.5`；`comfort_decel` / `emergency_decel` / `time_headway_seconds` 严格大于零；
  `emergency_decel >= comfort_decel`

停车：入口/出口 `u32` mm，且 `1 <= p <= length_mm - 1`；长宽 `>= 100` mm；横向
`i32` mm，路外 `abs >= 1`；朝向仍为受检弧度 `f32`。

路线剩余：沿 hop 把 `u32` 边长累加到 `u64` mm。`BoundedDistance::Finite(u64)`。
占用间隙与跨 hop 跟车空隙用 checked `i64` mm，禁止 `i32` 回绕。边上区间仍 `u32`。

## 4. 车辆已提交状态

```text
progress_mm: u32           // 当前边上，0..=length_mm
carry_um: u16              // 0..=999
speed_mm_s: u32            // 静止可为 0
```

公开观察表面以上述整数为权威。可以提供显式只读换算（例如文档化的 `/ 1000`），
不得把 `f64` 米当作权威 `value()`，不得在生产路径回写。

`progress_mm == length_mm` 合法，表示停在边终点（含拒绝 Gate 前）。替换 /
`Completed` / `Parked`：`carry_um = 0`。跨边保留 `carry_um`，除非跨边后立即硬停。

## 5. 一拍落地

与现行 `advance_active_vehicle` 同构，插入量化点：

1. 把 `progress_mm` / 边长 / 车长 / `min_gap` / 速度转为瞬时 `f32` 米，喂 IIDM
   与限速包络。
2. SI 中舒适截断 travel（前车、灯、路终、包络）。SI `travel <= 0` **不是**硬停。
3. 整数硬约束 `hard_room_mm: u64`（下限 0）。前车空隙用 `i64` 再夹到 `u64`：
   - 前车 `min_gap` 后空隙（可跨 hop）；
   - `DenyAndStop` 停车线；
   - 本车路线剩余（沿路线累加到最后一边终点，与现行 `remaining_to_route_end`
     同构）；
   - 下一 hop 存在但 Gate 拒绝时的 `fromEdge` 终点。
   下一 hop 存在且 Gate 许可时，边终点不是硬停。
4. `hard_room_mm == 0`：`travel_mm = 0`，`speed_mm_s = 0`，`carry_um = 0`，不
   `apply_travel`。若此时路线剩余已为零：`VehicleStatus::Completed`。
5. 否则
   `um = u64(carry_um) + round-ties-to-even(f64(travel_m) × 1e6)`；
   `travel_mm = min(um / 1000, hard_room_mm)`；
   若 `travel_mm < hard_room_mm`：`carry_um = (um % 1000) as u16`；
   若 `travel_mm == hard_room_mm`：到位后 `carry_um = 0`。
6. 整数 `apply_travel`：`progress_mm + travel_mm`。满边且下一跳许可则余量进下一条、
   `progress_mm = 0`；满边但下一跳拒绝则停在 `progress_mm == length_mm`，保持
   `Active`。
7. 硬停（第 4 步，或第 5 步走到 `hard_room_mm`）：`speed_mm_s = 0`。若走完的是
   **路线剩余**（没有下一 hop）：`VehicleStatus::Completed`，离开占用与
   `committed_pose_sources`。Gate 拒绝不是 `Completed`。否则
   `speed_mm_s = round-ties-to-even(f64(next_speed_m) × 1000)`，不由 travel 反推。

占用循环：边上 `remaining_mm == 0` 结束（`u32`）。重叠：有符号整数区间。跨 hop
间隙用 checked `i64`。生产路径删除米制 `1e-9` / `1e-12` 比较。

舍入一律 IEEE 754 **round-ties-to-even**，缩放在 `f64` 中做（`f32 × 1e6` 在
100 m 行程上会丢微米）。

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

限速：`speed_limit_mm_s = round-ties-to-even(f64(m/s) × 1000)`，且 `> 0`。

LFCA 登记表破坏性更新（不兼容读旧 `F64` 米或 `F64` 时距/加减速/朝向）：

| 现行                                         | 目标                                  |
| -------------------------------------------- | ------------------------------------- |
| `LaneEdge.lengthMeters: F64`                 | `lengthMillimetres: U32`              |
| `LaneEdge.speedLimitMetersPerSecond: F64`    | `speedLimitMillimetresPerSecond: U32` |
| `VehicleProfile.lengthMeters` 等长度         | 对应 `U32` mm                         |
| `VehicleProfile.desiredSpeedMetersPerSecond` | `U32` mm/s                            |
| 停车 progress / lateral / extent             | `U32`/`I32` mm                        |
| `ParkingSpace.headingOffsetRadians: F64`     | `headingOffsetRadians: F32`           |
| 时距与三项加减速                             | 受检 `F32` SI（不保留 `F64`）         |

后发射检查失败关闭旧字节。走廊检入 LFCA 必须由生成器重生并对拍。
`NetworkRevisionId` 随载荷变化。

`laneflow-static-contract` 常量：最短尺寸 `100` mm，端点留白 `1` mm，删除作为
生产判定的 `1.0e-9` 米常量（或标为历史、不再被 Runtime/compiler 引用）。

## 7. Spatial

不改 canonical `f32` 点。采样：

- `progress_mm == 0` → 折线起点；
- `progress_mm == length_mm` → 折线终点；
- 中间 `geometry_s = (progress_mm as f64 / length_mm as f64) * f64(arc)` 再
  转为 `f32`。

横向停车偏移：`offset_mm as f32 / 1000.0`。

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
  跨 hop 占用间隙用 `i64`，长路单不得 `i32` 回绕；
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
