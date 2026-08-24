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

| 量                                                           | 表示                     | 规则                                                |
| ------------------------------------------------------------ | ------------------------ | --------------------------------------------------- |
| 边长、边内进度、车长、`min_gap`、停车入口/出口进度、停车长宽 | `u32` mm                 | 无符号长度                                          |
| 停车横向偏移、占用间隙算术                                   | `i32` mm                 | 有符号；禁止 `u32` 下溢代替「不够」                 |
| 路线前缀和、视距累计                                         | `u64` mm                 | 单边仍 `u32`                                        |
| 单边防御上界                                                 | `10_000_000` mm（10 km） | 超限失败关闭，不自动拆边                            |
| 最短边、最短车长                                             | `100` mm                 | 对齐 Spatial 最短段 `0.1 m`                         |
| 停车锚点端点留白                                             | `1` mm                   | **另一个常量**：`1 <= progress_mm <= length_mm - 1` |
| 路外泊位横向                                                 | `abs(offset_mm) >= 1`    | 不允许标 0 却声称离开中心线                         |

`BoundedDistance::Finite` 改为 `u64` mm；`BeyondFinite` 语义保留。

朝向、车头时距、加速度/减速度 **不是** 一维长度，不进毫米权威：时距与加减速
保持受检 `f32` SI，供 IIDM 使用。profile `max_accel` 的产品下限为
**`0.5 m/s²`**（`install` / 编制检查失败关闭）。该下限保证 4 ms 量子下，静止起步
约 1 s 内能靠行程余数凑满 1 mm；更弱的加速度不是产品车辆，**禁止**另做速度余数
去「救」它。`comfort_decel` / `emergency_decel` / `time_headway` 仍为严格大于零。

### 2. 边长由 compiler 内部规范折线弧长派生

`length_mm = round-ties-to-even(f64(arc_m) × 1000)`，其中 `arc_m` 是 ADR 0022 /
0015 已冻结、compiler 几何遍求出的规范 `f32` 弧长。该派生 **不依赖** 修订是否
带 Spatial 组件：headless 世界同样把结果写入 `SharedTrafficNetwork` 热列
`lane_lengths_mm`。禁止从 Spatial 采样反推边长，禁止作者手写第二条长度。无 Spatial
时不得走车辆 pose 采样，但不影响一维边长权威。

LFCA 只存 `U32` 毫米边长，**不**另存编制 `F64` 米边长。

构建时仍用绝对 `0.01 m` 与相对 `1e-6` 对账弧长；对「最近毫米」通常自动满足。
`length_mm < 100` 或 `> 10_000_000` 失败关闭。

编制解析曲线继续 `f64` 求值再量化为折线。#354 不得把本轴收进 `Point` / `Vector`。

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

硬约束剩余空间 `hard_room_mm` 是本拍整数可走的停车类上限（有符号 mm，下限 0）：

- 前车 `min_gap` 之后的空隙（无前车则不限制）；
- `DenyAndStop` 停车线距离（绿灯/无灯则不限制）；
- **路线剩余**：沿本车路线累加到最后一边终点（与现行 `remaining_to_route_end` 同构）；
- 若下一条路线 hop 存在但该转移的 Gate **拒绝**：当前 `fromEdge` 终点；
- 若下一条 hop 存在且 Gate 许可：边终点 **不是** 硬停，余量进入下一条。

落地顺序固定：

1. 用已提交整数状态派生 IIDM / 限速包络所需的瞬时 SI 值；
2. 在 SI 中计算候选 travel，并按前车、灯、路终、下游限速包络做舒适截断。
   **SI `travel <= 0` 不是硬停判定**；
3. 用已提交整数计算 `hard_room_mm`。若 `hard_room_mm == 0`：硬停。
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
- 爬行：`hard_room_mm > 0` 且量化后 `travel_mm == 0`。保留速度与余数。

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

### 7. 破坏性制品与 API；#302 必须消费本合同

允许破坏。LFCA 长度/速度类字段改为 `U32` 毫米或毫米每秒（单位进字段名）；
时距与三项加减速改为受检 `F32` SI。不保留对旧 `F64` 米或 `F64` 时距/加减速的
兼容读取。检入走廊 LFCA 必须重生。`NetworkRevisionId` 随语义载荷变化。

`VehicleState` 公开观察表面以 `progress_mm` / `carry_um` / `speed_mm_s` 为权威。
可以提供显式只读换算（例如文档化的 `/ 1000`），**不得**把米制换算当作权威
`value()`，也不得在生产路径回写。

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
- 编制另写一条边长，或从 Spatial 采样反推边长。
- 把一维 mm 收进三维 `Point` / `Vector3<T>`（#354）。

## 后果

- 跟车占用与跨边有产品量级的离散代数；空间几何仍是有界 `f32`。
- 短步长爬行靠微米余数，不靠纳米哨兵；硬停靠整数硬约束，不靠 SI 符号。
- G2 必须同时改 compiler 发射、LFCA 登记表、共享列和 Runtime 热状态。
- #302 不得在本切片完成前进入自身 G1 的快照字段冻结。
- 同进程并行不因撤回跨 CPU 位级承诺而改合同；跨机器联机仍需独立 ADR。

## 实施与治理

1. #496 G1：本 ADR、`traffic-runtime-integer-geometry.md`、ADR 0003/0014 修订
   与 glossary/数值/跟车/消费/信号/制品同步。设计 PR `Refs: #496`，不 `Closes`。
2. #496 G2：唯一 Delivery PR 实现合同、对齐走廊相位并重生夹具，`Closes #496`。
3. 实现中若发现必须改变毫米量子、步长区间、加速度下限或「先整数硬约束再余数」，
   停工并重开 G1。
