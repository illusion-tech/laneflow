# 0028 交通一维几何整数毫米与固定步进合同

**状态**: Proposed（#496 G1；未 Pass，不授权实现）<br>
**日期**: 2026-08-24<br>
**适用范围**: 交通运行时已提交一维几何、固定步进合法区间、已提交速度、LFCA
长度/速度字段、compiler 边长派生、跟车占用/投影离散判定；以及 #302 快照字段的
数值输入<br>
**部分取代**: ADR 0014 中「生产继续 current-`f64`」的实施状态，以及「下一生产权威
为补偿残差感知 `f32` 进度」的目标合同。#144 的取证与 no-go 作为历史证据保留，
不作为必须与旧 `f64` tick 零分歧的门禁。<br>
**不取代**: ADR 0015 有界 canonical `f32` 空间几何；ADR 0022 编制 `f64` 曲线 →
规范 `f32` 折线；ADR 0003 的「不读墙钟、固定步进、可变 Δt 非法、跨 CPU 不承诺
浮点位级确定性」。本 ADR **不**给已提交整数状态补一条跨 CPU / 跨机器位级承诺。<br>
**关联 Issue**: [#496](https://github.com/illusion-tech/laneflow/issues/496)
（原生 blocking [#302](https://github.com/illusion-tech/laneflow/issues/302)）<br>
**关联文档**:

- `0003-runtime-tick-and-determinism.md`
- `0014-residual-aware-f32-core-authority-and-migration-gates.md`
- `0015-bounded-f32-canonical-spatial-frames.md`
- `0022-authoring-curve-and-canonical-polyline-error-budgets.md`
- `0025-checked-canonical-network-and-shared-static-network.md`
- `../design/traffic-runtime-integer-geometry.md`
- `../design/shared-static-network.md`
- `../design/numeric-representation.md`
- `../design/vehicle-following.md`
- `../design/traffic-runtime-shared-consumption.md`

## 背景

现行 `TrafficWorld` 一维几何（边长、边内进度、车长、`min_gap`、停车锚点、限速）
使用 current-`f64` 米，并用 `1.0e-9 m` / `1e-12 m` 哨兵代替「大于零 / 约等于零」。
这些哨兵没有产品物理意义；Spatial 最短段已是 `0.1 m`、连接 `5 mm`、长度绑定
`1 cm`。用纳米精度证明「必须 `f64`」是循环论证。

ADR 0014 曾接受残差感知 `f32` 进度为下一目标；#144 因稳态只快 `4.257%`、未过
`5%` 门槛回退。那次门禁还要求与当时 Core `f64` 离散零分歧。仓库没有外部用户，
JSON 生产入口已拆除，该门槛不再构成现行交通权威。

产品需要的是：毫米–厘米上自洽的占用、跨边、停车和限速；与整数毫秒步进对齐；
与已冻结的 `f32` 折线对账。一维格子比再做一套残差浮点更贴这个包络。

三维点/向量（编制 `f64`、canonical `f32`）仍由 ADR 0022 / 0015 与 #354 拥有，
不并入本轴。

## 决策

### 1. 已提交一维几何以整数毫米为权威

下列已提交量使用整数毫米，不使用生产路径上的 `1e-9` / `1e-12` 米哨兵：

| 量                                   | 表示                     | 规则                                            |
| ------------------------------------ | ------------------------ | ----------------------------------------------- |
| 边长、边内进度                       | `u32` mm                 | 单边 `100..=10_000_000`                         |
| 车长、停车长宽                       | `u32` mm                 | `100..=128_000`（沿用 ADR 0014 `0.1..=128 m`）  |
| `min_gap`                            | `u32` mm                 | `0..=128_000`                                   |
| 停车入口/出口进度                    | `u32` mm                 | `1 <= p <= length_mm - 1`                       |
| 停车横向偏移                         | `i32` mm                 | `abs <= 128_000`；路外 `abs >= 1`               |
| 占用间隙、跨 hop 跟车空隙            | `i64` mm                 | 有符号；禁止回绕。单边仍 `u32`                  |
| 路线前缀和、视距累计、`hard_room_mm` | `u32` mm                 | checked 加；溢出不是再加宽，而是 `BeyondFinite` |
| 单边防御上界                         | `10_000_000` mm（10 km） | 超限失败关闭，不自动拆边                        |
| 停车锚点端点留白                     | `1` mm                   | **另一个常量**，与最短边 `100` 分开             |

`BoundedDistance::Finite(u32)` 毫米。满量程约 **4295 km**，已覆盖城市一趟行程
（Spatial 单 frame 约 32 km 盒；家→公司/过境是几十公里，不是跨省 2000 km 单路单）。
前缀和用 checked `u32` 加；溢出 → `BeyondFinite`（或注册失败），**不**为 1920×10 km
理论积上 `u64`。`BeyondFinite` 语义保留。

朝向、车头时距、加速度/减速度 **不是** 一维长度，不进毫米权威：时距与加减速
保持受检 `f32` SI，供 IIDM 使用。沿用 ADR 0014 上界，并加上起步下限：

- `max_accel`：`0.5..=50` m/s²
- `comfort_decel` / `emergency_decel`：`0 < value <= 50` m/s²，且
  `emergency_decel >= comfort_decel`
- `time_headway`：`0 < value <= 60` s

`max_accel` 下限保证 4 ms 下静止起步约 1 s 内能靠行程余数凑满 1 mm；更弱加速度
不是产品车辆，**禁止**速度余数。

### 2. LFCA 只存一条 `U32` 毫米边长；有折线从弧长派生，headless 不求弧

1D 交通要的是边长数字，不是折线。headless **不需要弧长**：无中心线、不采样位姿。

LFCA 交通边只存 `U32` 毫米，**不**另存编制 `F64` 米，也不从已省略的 Spatial 表反推。
结果写入 `SharedTrafficNetwork.lane_lengths_mm`。

- **有冻结规范折线**（`spatial_geometry()` 为 `Some`）：
  `length_mm = round-ties-to-even(f64(arc_m) × 1000)`，`arc_m` 是 ADR 0022 /
  0015 的规范 `f32` 弧长。构建时仍用绝对 `0.01 m` 与相对 `1e-6` 对账 LIR 交通
  长度。
- **无折线**（headless，`spatialPresent=0`，`compiler-foundation` 允许不声明中心线）：
  `length_mm = round-ties-to-even(f64(lir_length_m) × 1000)`，`lir_length_m` 是
  现行 LIR 交通边长（`CanonicalLaneEdgeView::length_meters()`）。这不是第二条
  LFCA 长度字段，也不是要求 G2 为无图形编译编造折线。

`length_mm < 100` 或 `> 10_000_000` 失败关闭。无 Spatial 时不得走车辆 pose
采样。编制解析曲线继续 `f64` 求值再量化为折线。#354 不得把本轴收进 `Point` /
`Vector`。

### 3. 已提交速度为 `u32` 毫米每秒

边限速、profile 期望车速、已提交 `VehicleState` 速度使用 `u32` mm/s：
`round-ties-to-even(f64(m/s) × 1000)`。限速与期望车速必须 `> 0`；静止车速度
可为 `0`。沿用 ADR 0014 的产品上界 `100 m/s` = `100_000` mm/s。

IIDM 与安全包络仍在 `f32` SI 中计算。进入 IIDM 前把 mm / mm/s 转为米只作
**瞬时**；出来后再量化。**禁止**用 `2 × travel / Δt − v0` 作为已提交速度权威
（4 ms 下会变成 `0.25 m/s` 台阶）。**禁止**速度余数。

加速度不写入已提交整数权威。

### 4. 先取整数硬约束，再把余数加在剩余空间内

每辆 live 车持有 `carry_um: u16`，不变式 `0..=999`。

硬约束剩余空间 `hard_room_mm: u32` 是本拍整数可走的停车类上限（下限 0）。前车空隙
用 `i64` 间隙算出后，负值当 0，正值夹到 `u32`：

- 前车 `min_gap` 之后的空隙（无前车则不限制）；可跨 hop 累加，不得用 `i32` 回绕。
  空隙取 **本拍开始时 occupancy 快照**，与现行 `advance_active_vehicle` 对
  `leader_bumper_gap` 再截断到 `(gap - min_gap).max(0)` 同构。**不**计入本拍
  `leader_final_travel`（`vehicle-following.md` §11.2 的前向后传播）。快照中间距
  恰为 `min_gap` 时 `hard_room_mm == 0`，即使前车本拍会走；follower 下一拍才看见
  新空隙。该一拍滞后是现行生产路径的行为，#496 不在本轴改为投影前车行程；
- `DenyAndStop` 停车线距离（绿灯/无灯则不限制）；
- **路线剩余**：沿本车路线累加到最后一边终点（与现行 `remaining_to_route_end` 同构）；
- 若下一条路线 hop 存在但该转移的 Gate **拒绝**：当前 `fromEdge` 终点；
- 若下一条 hop 存在且 Gate 许可：边终点 **不是** 硬停，余量进入下一条。

落地顺序固定：

1. 用已提交整数状态派生 IIDM / 限速包络所需的瞬时 SI 值；
2. 在 SI 中计算候选 travel，并按前车、灯、路终、下游限速包络做舒适截断。
   **SI `travel <= 0` 不是硬停判定**；
3. 用已提交整数计算 `hard_room_mm`（只读 T 快照，不读本拍其他车已算行程）。
   若 `hard_room_mm == 0`：硬停。
   `travel_mm = 0`，`speed_mm_s = 0`，`carry_um = 0`，不 `apply_travel`；
4. 否则
   `um = u64(carry_um) + round-ties-to-even(f64(travel_m) × 1_000_000)`，
   `travel_mm = min(um / 1000, hard_room_mm)`（`u64` 运算，避免 `f32` 在大行程
   上丢失微米）；
5. `apply_travel`。若 `travel_mm == hard_room_mm`（本拍走到停车类约束）：
   到位后 `speed_mm_s = 0`，`carry_um = 0`。若该约束是 **路线剩余为零**（没有
   下一 hop）：再把 `VehicleStatus` 置为 `Completed`，离开占用与
   `committed_pose_sources`。Gate 拒绝或仍有后续 hop 的硬停保持 `Active`。
   否则保留 `carry_um = um % 1000`，速度量化为 `u32` mm/s，**独立于** 本拍是否
   凑满 1 mm。

跨边：仅当本拍行程到达 `fromEdge` 终点 **且** 下一条 hop 存在 **且** 该转移 Gate
（若有）许可。`progress_mm = 0`，**保留** `carry_um`（除非跨边后立即命中硬停）。
`progress_mm == length_mm` 是合法的「停在边终点 / 拒绝 Gate 前」状态，**不得**
无条件跨边，也 **不得** 把这种 Active 硬停写成 `Completed`。`Completed`、
`Parked`、车辆替换：`carry_um = 0`。

禁止把未截断的 `v × Δt` 送进累加器。舍入模式全合同统一为
**IEEE 754 round-ties-to-even**，中间乘法在 `f64` 完成。

硬停与爬行：

- 硬停：整数硬约束剩余为 0，或本拍走完该剩余。清速度与余数。
- 爬行：`hard_room_mm > 0` 且量化后 `travel_mm == 0`。保留本拍量化后的速度与余数
  （二者都可以是 0）。

4 ms 静止跟停量化死区（**接受**，不为它加状态）：

IIDM 有效加速度可以远小于 profile `max_accel`。静止 follower、静止 leader、bumper
空隙 = `min_gap + 0.1 m`、`max_accel = 0.5 m/s²` 时，有效加速度约 `0.0465 m/s²`，
4 ms 行程约 `0.37 µm`、下一速度约 `0.186 mm/s`。round-ties-to-even 后 `um == 0`
且 `speed_mm_s == 0`，`carry_um` 不增长，下一拍状态重复；车辆停在期望间距外约
`100 mm`。这不是 `max_accel < 0.5` 的被拒情形，而是舒适层在接近 `min_gap` 时把本拍
能量化到 0。`hard_room_mm` 仍大于 0，因此这是爬行的退化，不是硬停。

**禁止**亚微米累加器、把 `carry` 改成更高分辨率、或把该死区当实现缺陷消掉。
建议默认 16 ms 下同工况行程约 `6 µm`，能进入 `carry_um`。死区是 4 ms 最细量子的
产品接受面。

### 5. 固定步进合法区间为 `4..=1000` ms

修订 ADR 0003 的「正整数、未冻范围」：

- `WorldConfig.fixed_delta_time_ms ∈ [4, 1000]`，运行中不得改变。
- `TickInput.delta_time_ms` 必须相等，否则拒绝、世界不变。
- **4 ms 是最细量子，不是默认。** 面向画面的产品默认建议 **16 ms**。
  `>= 100 ms` 合法，但不保证跟车观感；`1000 ms` 对齐粗/离线量子（SUMO 级 1 s），
  **不是**慢放。
- 交通运行时 **不**提供慢放：不缩小 Δt、不接受可变 Δt。墙钟变慢只允许 Adapter
  少调用 `step`。
- 每个信号相位 `durationMs` 必须是该世界步长的 **正整数倍**（因此也 `>=` 一步）。
  短相位或不能整除则 `install` 失败。G2 前 `main` 仍只要求 `durationMs >= dt`；
  检入走廊 `yellow_ms = 3000`、`all_red_ms = 1000` 相对建议 16 ms 不能整除，G2
  必须改相位并重生 LFCA，G1 不改生成器输入、不重生制品。

时间权威仍是 checked `u64` 的 `tick_index` / `time_ms`。

### 6. 离散判定用整数比较

占用耗尽：`remaining_mm == 0`。重叠：有符号区间相交，不用 epsilon。跨边按第 4
节 Gate 许可，不用「进度等于边长则无条件进入下一条」。Spatial 采样：
`progress_mm == 0` 钉死折线起点，`progress_mm == length_mm` 钉死终点；中间
`geometry_s = progress_mm × arc / length_mm`。

确定性沿用 ADR 0003：同一软件版本、同一运行环境、同一初始状态和同一 tick/input
序列，已提交 `progress_mm` / `speed_mm_s` / `carry_um` 必须一致。IIDM 所用 `f32`
与规范弧长所用 `f32` **不**承诺跨 CPU / 跨机器位级确定性；余数输入依赖这些
`f32`，因此已提交整数状态也 **不**另作跨 CPU 位级承诺。同进程并行（同一二进制、
同一运行环境）仍须与 1-worker 得到相同已提交状态与事件序。联机 / 跨机器
lockstep 不在本合同范围。

不得用本切片宣称全 tick 位级回放。不同合法步长的世界轨迹 **不可比**，不是回归
失败。

### 7. 破坏性制品与 API；分配 LFCA v2；#302 必须消费本合同

允许破坏。**不得改写已冻的 LFCA v1 登记表。** G2 分配新的规范制品版本：

| 字段                                                                  | v1  | G2                        |
| --------------------------------------------------------------------- | --- | ------------------------- |
| 对象前导 `formatVersion` 与 `ContractVersions.canonicalFormatVersion` | `1` | **`2`**                   |
| `networkRevisionDerivationVersion`                                    | `1` | **`2`**                   |
| `constraintContractVersion`                                           | `1` | **`2`**                   |
| `staticExecutionContractVersion`                                      | `1` | **`2`**                   |
| `identityEncodingVersion` / `identityRegistryRevision`                | `1` | `1`（本切片不改身份前像） |

v1 读器拒绝 `formatVersion != 1`；v2 读器拒绝 v1。不兼容读取。LFSM
`canonicalArtifactFormatVersion` 必须等于所绑 LFCA 的 `canonicalFormatVersion`。
不为本切片单开 LFSM/LFSD 对象版本。

G2 必须同时冻结共享静态路网 **admission**，不得只换制品版本号：

- 现行 `CheckedCanonicalNetworkInputV1` / `check_canonical_network_input_v1` /
  `PostEmissionCheckedBundleV1::canonical_network_input` **只**承认 LFCA v1。
  **禁止**放宽 V1 预检或 V1 capability 以接纳 `formatVersion = 2`。
- G2 分配并行能力（名称冻结；Rust 字段布局可在不扩大能力的前提下调整）：
  `CheckedCanonicalNetworkInputV2`、`check_canonical_network_input_v2`、
  `check_post_emission_bundle_v2`、
  `PostEmissionCheckedBundleV2::canonical_network_input() -> CheckedCanonicalNetworkInputV2`。
- `check_canonical_network_input_v2` 要求对象 kind 精确为 LFCA v2（前导
  `formatVersion` 与 `canonicalFormatVersion` 均为 2），走 **v2 registry 预检**；
  digest / exact length / `NetworkRevisionId` 闭合、字段私有、无公共构造器，规则同 V1。
- 后发射：对 LFCA 走 v2 预检；LFSM/LFSD 仍按现行对象版本预检，但必须
  `LFSM.canonicalArtifactFormatVersion == 2` 且与所绑 LFCA 一致。
  `PostEmissionCheckedBundleV1` 不得派生 V2 capability。
- G2 完成后，`SharedNetworkRevision` 生产构建只消费 V2。G2 前 `main` 仍只消费 V1。
  不得让同一隐式入口把 v1 `F64` 米列读成毫米。API 草图见
  `shared-static-network.md` §3.1。

LFCA v2 长度/速度类字段为 `U32` 毫米或毫米每秒（单位进字段名）；时距、三项
加减速与停车朝向为受检 `F32` SI。检入走廊必须按 v2 重生。`NetworkRevisionId`
随语义载荷变化。

公开观察与命令表面同一套整数权威：

- `VehicleState`：`progress_mm` / `carry_um` / `speed_mm_s`。
- `VehicleSpawnInput` / `replace_completed_vehicle`：`progress_mm`、
  `speed_mm_s`；新车 **`carry_um = 0`**。`progress_mm` 落在当前边
  `0..=length_mm`；`speed_mm_s <=` 当前边限速且 `<= 100_000`。禁止 spawn 时
  再做一层未文档化的米→毫米量化。
- `PoseSource::Lane`：`LaneEdgeOrdinal` + `progress_mm: u32`。Spatial 采样在
  边界把 mm 换成弧长比例；不得把米制进度当作已提交 pose 权威。
- `VehicleReplaceBlock.bumper_gap_mm: i64`（与占用间隙同型）。`ReplaceError::Blocked`
  不再用米制 `bumper_gap` 当权威。
- 路线距离查询（`RouteDistanceIndexView` / `RouteDistanceQuery`）：
  `occurrence_offsets` / `segment_totals` 为 `u32` mm；参数 `from_progress_mm`、
  `horizon_mm` 为 `u32`；`Within(u32)`；`Finite(u32)` / `BeyondFinite` /
  `Passed`。不得保留米制查询再换算。
- 只读米制换算可以有，不得当 `value()`，不得回写。

边限速与 profile 期望车速：`1..=100_000` mm/s。`install` / 构建 / spawn 任一
处超限失败关闭。

#496 原生 blocking #302。Runtime Snapshot 的每世界可变状态必须使用本 ADR 的
整数进度、余数与 `mm/s`；不得先冻 `f64` 米进度。

G2 对照门是本契约自洽，**不是**相对 current-`f64` 的 `5%` 墙钟或离散零分歧。

## 被拒绝的替代方案

- 生产继续统一 `f64` 米，或把 `1e-9` 抄进 `f32`。
- 无余数的裸整数毫米（跟停冻死、限速系统偏差）。
- 残差感知 `f32` 进度作为下一生产权威（不解决哨兵，#144 未过门也不再当门槛）。
- 整数 IIDM，或把加速度写入已提交整数权威。
- 用整数 travel 反推已提交速度，或为过小加速度增加速度余数。
- 用 SI `travel <= 0` 代替整数硬约束判定硬停。
- 到边终点无条件跨边。
- 走到路线终点只清速度、不进入 `Completed`。
- 承诺跨 CPU / 跨机器整数位级相同（行程余数输入仍是 `f32` IIDM）。
- 1–3 ms 步长；Runtime 慢放/可变 Δt。
- 编制另写一条 LFCA 长度字段，或从已省略的 Spatial 表反推边长。
- 强迫 headless 从折线弧长派生边长（无中心线就没有弧）。
- 占用间隙用 `i32` 回绕代替有符号 `i64`。
- 路线 `Finite` 用 `u64` 只为装下 1920×10 km 理论积；产品行程用 `u32` mm，溢出走
  `BeyondFinite`。
- 丢掉 ADR 0014 的加减速/时距/尺寸/横向 **上界**，只写下限。
- 路线距离查询或 `VehicleReplaceBlock` 继续用米制作权威。
- 改写已冻 LFCA v1 登记表，而不分配 `canonicalFormatVersion = 2`。
- 放宽 `CheckedCanonicalNetworkInputV1` / `check_canonical_network_input_v1` /
  `PostEmissionCheckedBundleV1` 以接纳 LFCA v2；或不冻结并行 V2 admission，把构建入口留给 G2 临场发挥。
- 为消除 4 ms 静止跟停量化死区而增加亚微米累加器或更高分辨率 `carry`。
- 把 `vehicle-following.md` §11.2 的 `leader_final_travel` 并入 #496 的
  `hard_room_mm`，打破与现行快照截断的同构。
- spawn / `PoseSource` 继续用米制作权威，只冻 `VehicleState` 观察面。
- 把一维 mm 收进三维 `Point` / `Vector3<T>`（#354）。

## 后果

- 跟车占用与跨边有产品量级的离散代数；空间几何仍是有界 `f32`。
- 短步长爬行靠微米余数，不靠纳米哨兵；硬停靠整数硬约束，不靠 SI 符号。
- 4 ms 静止跟停在 IIDM 有效加速度过小、本拍不足 0.5 µm 时可以停在期望间距外约
  10 cm；这是接受面，不是再加一层余数的理由。
- `hard_room_mm` 与现行快照截断同构；跟车设计文档 §11.2 的投影前车行程仍是另一轴。
- G2 必须同时改 compiler 发射、LFCA 登记表、v2 admission、共享列和 Runtime 热状态。
- #302 不得在本切片完成前进入自身 G1 的快照字段冻结。
- 同进程并行不因撤回跨 CPU 位级承诺而改合同；跨机器联机仍需独立 ADR。

## 实施与治理

1. #496 G1：本 ADR、`traffic-runtime-integer-geometry.md`、ADR 0003/0014 修订
   与 glossary/数值/跟车/消费/信号/制品/共享静态路网 admission 同步。设计 PR
   `Refs: #496`，不 `Closes`。
2. #496 G2：唯一 Delivery PR 实现合同、对齐走廊相位并重生夹具，`Closes #496`。
3. 实现中若发现必须改变毫米量子、步长区间、加速度下限或「先整数硬约束再余数」，
   停工并重开 G1。
