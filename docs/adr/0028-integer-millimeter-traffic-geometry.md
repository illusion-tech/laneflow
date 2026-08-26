# 0028 交通一维几何整数毫米与固定步进合同

**状态**: Accepted（#496；已提交一维 / 制品 / Runtime；#500 编译器 IR 交通一维）<br>
**日期**: 2026-08-24<br>
**适用范围**: 交通运行时已提交一维几何、固定步进合法区间、已提交速度、LFCA
长度/速度字段、compiler 边长派生、编译器 Typed AST / HIR / MIR / LIR 交通一维
存储、跟车占用/投影离散判定；以及 #302 快照字段的数值输入<br>
**部分取代**: ADR 0014 中「生产继续 current-`f64`」的实施状态，以及「下一
生产权威为补偿残差感知 `f32` 进度」的目标合同。0014 保持 Accepted 作为 #144
历史证据，不再约束现行生产。#144 的取证与 no-go 不作为必须与旧 `f64` tick 零分歧的
门禁。<br>
**不取代**: ADR 0015 有界 canonical `f32` 空间几何；ADR 0022 编制 `f64` 曲线 →
规范 `f32` 折线；ADR 0003 的「不读墙钟、固定步进、可变 Δt 非法、跨 CPU 不承诺
浮点位级确定性」。本 ADR **不**给已提交整数状态补一条跨 CPU / 跨机器位级承诺。<br>
**关联 Issue**: [#496](https://github.com/illusion-tech/laneflow/issues/496)；
编译器 IR 收口 [#500](https://github.com/illusion-tech/laneflow/issues/500)。
#496 的已提交整数合同是 [#302](https://github.com/illusion-tech/laneflow/issues/302)
快照字段冻结的设计前置（快照不得先冻 `f64` 进度）。<br>
**关联文档**:

- `0003-runtime-tick-and-determinism.md`
- `0014-residual-aware-f32-core-authority-and-migration-gates.md`
- `0015-bounded-f32-canonical-spatial-frames.md`
- `0022-authoring-curve-and-canonical-polyline-error-budgets.md`
- `0025-checked-canonical-network-and-shared-static-network.md`
- `../design/traffic-runtime-integer-geometry.md`
- `../design/shared-static-network.md`
- `../design/portable-canonical-artifact.md`
- `../design/numeric-representation.md`
- `../design/vehicle-following.md`
- `../design/route-system.md`
- `../design/traffic-runtime-shared-consumption.md`

## 背景

#496 之前，`TrafficWorld` 一维几何（边长、边内进度、车长、`min_gap`、停车锚点、限速）
使用 current-`f64` 米，并用米制哨兵代替「大于零 / 约等于零」。
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

G1 冻权威、单位、量化顺序、制品字段与跨实现算法。G2 决定 Rust 方法名与错误变体拼写。
#496：未点名的 Runtime 米制公开面随本权威一并迁走或降为只读换算。#500：公开
`Canonical*View` 的交通一维 **删除** 米制访问器，不留只读换算。毫米访问器为
`length_mm`、`speed_limit_mm_s`、`progress_mm`、`lateral_offset_mm`、
`desired_speed_mm_s`、`min_gap_mm`。

### 1. 已提交一维几何以整数毫米为权威

下列已提交量使用整数毫米，不使用生产路径上的 `1e-9` / `1e-12` 米哨兵：

| 量                                   | 表示                     | 规则                                                  |
| ------------------------------------ | ------------------------ | ----------------------------------------------------- |
| 边长、边内进度                       | `u32` mm                 | 单边 `100..=10_000_000`                               |
| 车长、停车长宽                       | `u32` mm                 | `100..=128_000`（沿用 ADR 0014 `0.1..=128 m`）        |
| `min_gap`                            | `u32` mm                 | `0..=128_000`                                         |
| 停车入口/出口进度                    | `u32` mm                 | `1 <= p <= length_mm - 1`（`length_mm` 为提交后边长） |
| 停车横向偏移                         | `i32` mm                 | `abs <= 128_000`；路外 `abs >= 1`                     |
| 占用间隙、跨 hop 跟车空隙            | `i64` mm                 | 有符号；禁止回绕。单边仍 `u32`                        |
| 路线前缀和、视距累计、`hard_room_mm` | `u32` mm                 | checked 加；溢出不是再加宽，而是 `BeyondFinite`       |
| 单边防御上界                         | `10_000_000` mm（10 km） | 超限失败关闭，不自动拆边                              |
| 停车锚点端点留白                     | `1` mm                   | **另一个常量**，与最短边 `100` 分开                   |

有界距离的 Finite 侧是 `u32` 毫米。满量程约 **4295 km**，已覆盖城市一趟行程
（Spatial 单 frame 约 32 km 盒；家→公司/过境是几十公里，不是跨省 2000 km 单路单）。
前缀和用 checked `u32` 加；溢出 → `BeyondFinite`，**禁止**因此让 `register_route`
失败，也 **不**为「理论最长边序列 × 10 km」积上 `u64`。`BeyondFinite` 语义保留。
实现是分段 `u32` 前缀 + 后缀 `BoundedDistance`（`segmented_route_coordinates` /
`RouteDistanceIndexView`），不是把 Finite 侧改成 `u64`，也不是饱和起点前缀相减。
占用间隙的 `i64` 只服务有符号空隙，不是前缀加宽先例。
路网制品不声明路线（ADR 0029）。

朝向、车头时距、加速度/减速度 **不是** 一维长度，不进毫米权威：时距与加减速
保持受检 `f32` SI，供 IIDM 使用。沿用 ADR 0014 上界，并加上起步下限：

- `max_accel`：`0.5..=50` m/s²
- `comfort_decel` / `emergency_decel`：`0.5..=50` m/s²，且
  `emergency_decel >= comfort_decel`
- `time_headway`：`0 < value <= 60` s

`max_accel` 下限保证 4 ms 下静止起步约 1 s 内能靠行程余数凑满 1 mm。减速度下限保证
从产品速度上界 `100 m/s` 刹停约 10 km，落在 `Finite(u32)` 内；`0.001 m/s²` 需要
约 5000 km，会超出。更弱加减速不是产品车辆，**禁止**速度余数。

编制进入编译器整数 / 受检 `f32` 表面时，**在准入边界量化一次，再按量化后的界限检查**。
之后 Typed AST、HIR、MIR、LIR 的交通一维只带本节整数（或本就不是长度的受检 `f32` SI）。
**禁止**准入已经证明能量化进毫米闭包后丢掉 `u32`、把原来的 `f64` 留下，再在发射 round
第二次。

- 毫米 / 毫米每秒：`round-ties-to-even(f64(SI) × 1000)` 得到整数候选，再套本节整数闭包。
  例如车长 `0.0996 m` → `100 mm` 合法；`0.0994 m` → `99 mm` 失败。**禁止**先用量化前的
  裸 `0.1 m` / `128 m` 拒绝、再量化（那会与毫米权威打架）。
- 时距、三项加减速、停车朝向：`f64` → IEEE 754 binary32 round-ties-to-even，再套本节
  `f32` SI 闭包。朝向闭包 `-π <= x < π`；`+π` / `-π` 的 binary32 为 `0x40490fdb` /
  `0xc0490fdb`。编制/准入：量化后若等于 `+π`，写成 `-π` 再检查。制品与 IR 上的值必须
  已经满足闭包；存着的 `+π` 非法。读器与后发射检查只拒不折，发射只写已提交值。
- 跨字段（停车进度 vs 所引边长）在 **双方都量化之后** 比较；有折线时停车锚点相对
  **空间冻结提交后的**边长关闭，见第 2 节。
裸 SI 不是第二套权威。Spatial 折线仍是 ADR 0015 的 `f32` 米，不走本量化。

### 2. LFCA 只存一条 `U32` 毫米边长；有折线从弧长派生，headless 不求弧

1D 交通要的是边长数字，不是折线。headless **不需要弧长**：无中心线、不采样位姿。

LFCA 交通边只存 `U32` 毫米，**不**另存编制 `F64` 米，也不从已省略的 Spatial 表反推。
结果写入共享交通热列。

有折线与无折线的派生、以及 **编译器（有 LIR）与读器（无 LIR）必须分开对账**，
见 `traffic-runtime-integer-geometry.md` §6。#500：空间冻结用 `length_mm / 1000` 作
观察值对账规范 `f32` 弧长，对账通过后有折线把 IR `length_mm` **提交为**弧长量化结果，
无折线保持准入毫米。此后 LIR 边长与 LFCA `lengthMillimetres` 是同一整数；不得另存
米列再在发射 round。读器仍用 `lengthMillimetres / 1000` 对账弧长，失败不改已量化边长。

`length_mm < 100` 或 `> 10_000_000` 失败关闭。无 Spatial 时不得走车辆 pose
采样。编制解析曲线继续 `f64` 求值再量化为折线。#354 不得把本轴收进 `Point` /
`Vector`。

### 3. 已提交速度为 `u32` 毫米每秒

边限速、profile 期望车速、已提交 `VehicleState` 速度使用 `u32` mm/s：
`round-ties-to-even(f64(m/s) × 1000)`。限速与期望车速必须 `> 0`；静止车速度
可为 `0`。沿用 ADR 0014 的产品上界 `100 m/s` = `100_000` mm/s。

IIDM 与安全包络仍在 `f32` SI 中计算。进入 IIDM 前把 mm / mm/s 转为米只作
**瞬时**；出来后再量化。**禁止**用 `2 × travel / Δt − v0` 作为已提交速度权威
（4 ms 下会变成 `0.25 m/s` 台阶）。**禁止**速度余数。接近期望车速时的空路量化死带
见第 4 节，接受。

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
- **路线剩余**：从当前进度沿本车路线 checked 加到最后一边终点（与现行
  `remaining_to_route_end` 同构）。累加能放入 `u32` 时该项为 `Finite` 并参与
  `hard_room` 的 min；溢出则为 `BeyondFinite`，**该项不参与** `hard_room`（本拍不靠
  路终硬停，也不得因此 `Completed`）。`Completed` 当且仅当剩余为 `Finite(0)`。
  **禁止**把溢出饱和成 `u32::MAX` 当作路终硬约束，也 **禁止**把溢出当成注册失败；
- 若下一条路线 hop 存在但该转移的 Gate **拒绝**：当前 `fromEdge` 终点；
- 若下一条 hop 存在且 Gate 许可：边终点 **不是** 硬停，余量进入下一条。

落地顺序固定：

1. 用已提交整数状态派生 IIDM / 限速包络所需的瞬时 SI 值；
2. 在 SI 中计算候选 travel，并按前车、灯、路终、下游限速包络做舒适截断。
   **SI `travel <= 0` 不是硬停判定**。一维不倒车：负 SI travel 当 0，不扣
   `carry_um`，也不是硬停。`BeyondFinite` 的下游限速目标本拍不参与包络；进入
   `Finite` 后再按真实距离套。不上 `u64` 视距。
3. 用已提交整数计算 `hard_room_mm`（只读 T 快照，不读本拍其他车已算行程）。
   若 `hard_room_mm == 0`：硬停。`travel_mm = 0`，`speed_mm_s = 0`，`carry_um = 0`，
   不 `apply_travel`。若此时路线剩余为 **`Finite(0)`**：`VehicleStatus::Completed`，
   离开占用与 `committed_pose_sources`。`BeyondFinite` 不是路终。
4. 否则（`hard_room_mm > 0`）
   `um = u64(carry_um) + round-ties-to-even(f64(非负 travel_m) × 1_000_000)`，
   `travel_mm = min(um / 1000, hard_room_mm)`（`u64` 运算，避免 `f32` 在大行程
   上丢失微米）；
5. `apply_travel`。
   - 若 `travel_mm == hard_room_mm`：到位后 `speed_mm_s = 0`，`carry_um = 0`。
     若该约束是 **路线剩余 `Finite(0)`**（没有下一 hop）：再把 `VehicleStatus`
     置为 `Completed`，离开占用与 `committed_pose_sources`。Gate 拒绝或仍有后续
     hop 的硬停保持 `Active`。
   - 若 `travel_mm < hard_room_mm`：保留 `carry_um = um % 1000`，速度量化为
     `u32` mm/s，**独立于** 本拍是否凑满 1 mm。整数行程落地后，已提交速度不得超过
     **所在边**限速（余数最多比 SI 包络多送 1 mm 跨边）。如何夹紧属 G2。

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

4 ms 量化死区（**接受**，不为它加状态或改量化）：

- **静止跟停**：IIDM 有效加速度可以远小于 profile `max_accel`。静止 follower、
  静止 leader、bumper 空隙 = `min_gap + 0.1 m`、`max_accel = 0.5 m/s²` 时，有效
  加速度约 `0.0465 m/s²`，4 ms 行程约 `0.37 µm`、下一速度约 `0.186 mm/s`。
  round-ties-to-even 后 `um == 0` 且 `speed_mm_s == 0`，`carry_um` 不增长，下一拍
  状态重复；车辆停在期望间距外约 `100 mm`。这不是 `max_accel < 0.5` 的被拒情形，
  而是舒适层在接近 `min_gap` 时把本拍能量化到 0。`hard_room_mm` 仍大于 0，因此这是
  爬行的退化，不是硬停。建议默认 16 ms 下同工况行程约 `6 µm`，能进入 `carry_um`。
- **空路巡航**：IIDM 空路 `a = max_accel × (1 − (v/v0)⁴)` 接近期望车速时，有效
  加速度可小于 `0.5 mm/s / Δt`。`dt = 4 ms`、`max_accel = 0.5 m/s²`、期望
  `20 m/s` 时，已提交速度稳定在约 `18.61 m/s`（约低 7%），下一增量约
  `0.4997 mm/s`，round-ties-to-even 后整数不变，状态重复。车仍按该速度前进；
  `carry_um` 只攒距离，不能抬速度。相对偏差只取决于 `max_accel` 与步长，与期望
  车速无关。建议默认 16 ms 下同画像约低 1.6%。

**禁止**亚微米累加器、速度余数、把 `carry` 改成更高分辨率、或把该死区当实现缺陷
消掉。死区是 4 ms 最细量子的产品接受面。

### 5. 固定步进合法区间为 `4..=1000` ms

修订 ADR 0003 的「正整数、未冻范围」：

- `WorldConfig.fixed_delta_time_ms ∈ [4, 1000]`，运行中不得改变。
- `TickInput.delta_time_ms` 必须相等，否则拒绝、世界不变。
- **4 ms 是最细量子，不是默认。** 面向画面的产品默认建议 **16 ms**。
  `>= 100 ms` 合法，但不保证跟车观感；`1000 ms` 对齐粗/离线量子（SUMO 级 1 s），
  **不是**慢放。
- 交通运行时 **不**提供慢放：不缩小 Δt、不接受可变 Δt。墙钟变慢只允许 Adapter
  少调用 `step`。
- 每个信号相位时长必须是该世界步长的 **正整数倍**（因此也 `>=` 一步）。
  短相位、不能整除、或步长越出 `4..=1000`，都使 `install` 失败，且这三类原因必须
  可区分（`DeltaOutOfRange` / `PhaseShorterThanTick` / `PhaseNotMultipleOfTick`）。
  检入走廊黄灯 3008 ms、全红 1008 ms，相对 16 ms 整除。

时间权威仍是 checked `u64` 的 `tick_index` / `time_ms`。

### 6. 离散判定用整数比较

占用耗尽：`remaining_mm == 0`。重叠：有符号区间相交，不用 epsilon。跨边按第 4
节 Gate 许可，不用「进度等于边长则无条件进入下一条」。Spatial 采样：
`progress_mm == 0` 钉死折线起点，`progress_mm == length_mm` 钉死终点；中间按进度
占边长的实数比例映射到弧长。禁止 `u32` 整数除。乘除顺序属实现，G2 对齐现行
Spatial 采样。

确定性沿用 ADR 0003：同一软件版本、同一运行环境、同一初始状态和同一 tick/input
序列，已提交 `progress_mm` / `speed_mm_s` / `carry_um` 必须一致。IIDM 所用 `f32`
与规范弧长所用 `f32` **不**承诺跨 CPU / 跨机器位级确定性；余数输入依赖这些
`f32`，因此已提交整数状态也 **不**另作跨 CPU 位级承诺。同进程并行（同一二进制、
同一运行环境）仍须与 1-worker 得到相同已提交状态与事件序。联机 / 跨机器
lockstep 不在本合同范围。

不得用本切片宣称全 tick 位级回放。不同合法步长的世界轨迹 **不可比**，不是回归
失败。

### 7. 破坏性制品与 API；当前树只承认一套合同；#302 必须消费本合同

允许破坏。1.0 前不保留制品双栈：旧米制登记表、旧读器、旧夹具以 git 历史为准，
**不**进当前树，也 **不** 做 v1→毫米转换。公开 Rust 入口不带 `V1`/`V2` 后缀。

当前对象前导 `formatVersion` 与 `ContractVersions.canonicalFormatVersion` 为 **`3`**
（ADR 0029）。`constraintContractVersion` 为 **`2`**；`staticExecutionContractVersion`
为 **`3`**。LFSM `sourceMapFormatVersion` 与 LFSD `semanticDiffFormatVersion` 为 **`2`**。
`networkRevisionDerivationVersion` 保持 **`1`**（哈希算法未改，见下）。
`identityEncodingVersion` 保持 `1`，`identityRegistryRevision` 为 **`2`**（ADR 0029）。
读器拒绝 `formatVersion != 3`。LFSM `canonicalArtifactFormatVersion` 必须等于所绑
LFCA 的 `canonicalFormatVersion`（故为 `3`）。

LFSD Genesis 的 target `ContractVersions` / `ExecutionContract` 必须与所绑 LFCA
一致。Artifact diff 两端合同行仍须逐字段相等。检入走廊按 Genesis 重生，不走格式
迁移 diff。

共享静态路网 admission 与后发射检查只走这一套 registry。构建输入是字段私有、
不可伪造的受检 LFCA；digest / 长度 / `NetworkRevisionId` 规则不变。不得把米列读成
毫米，因为当前表里没有米列。

当前 LFCA 交通热列字段以 `portable-canonical-artifact.md` 与
`traffic-runtime-integer-geometry.md` §6 的毫米/`f32` 表为准，二者必须一致。
未改的字段保持原 tag、名字、类型、必填。不得用「等长度」打包。Spatial
`LaneEdgeGeometry` / `segments` 仍为 `f32` 米，不进本表。

检入走廊必须按 v2 重生。`NetworkRevisionId` **随语义载荷字节变化**；哈希算法仍是
`portable-canonical-artifact.md` §4.2 的 v1：
`SHA-256("laneflow.network-revision.v1\0" || canonicalNetworkSemanticPayloadV1)`
（组帧仍为语义节 `0x0001..0x0006`）。**禁止**空升 `networkRevisionDerivationVersion`
却不定义新算法。毫米字段改变载荷字节，同一套 v1 算法已得到不同 ID。

路线距离：**按查询窗口独立 checked 加**，不从路线头溢出毒死后缀。索引若分段，段内
偏移与段合计是 `u32` mm；下一条边长会让当前段溢出时封段、开新段。不得用
`u32::MAX` 当哨兵。从起点跨段总和仍可 `BeyondFinite`；从当前进度到终点、以及局部
视距，都从查询起点加。靠近终点后可再 `Finite`，`Finite(0)` 进入 `Completed`。路线
注册不因前缀溢出失败。公开查询结果是 `Finite(u32)` / `BeyondFinite` / 已越过。

凡公开、已提交的一维长度、速度、进度与占用间隙，权威都是本节整数单位。现行米制
公开面（观察、命令、派生列、错误载荷、人口/夹具）在 G2 一并迁走或降为只读换算。
车长可缓存在车辆状态上（spawn/replace 时从 profile 拷入）。新车 `carry_um = 0`。
只读米制换算可以有，不得当权威、不得回写。G1 **不**逐个冻结 Rust 访问器或错误变体
名字。

边限速与 profile 期望车速：`1..=100_000` mm/s。`install` / 构建 / spawn 任一
处超限失败关闭。

#496 的已提交整数合同是 #302 快照字段冻结的设计前置。Runtime Snapshot 的每世界
可变状态必须使用本 ADR 的整数进度、余数与 `mm/s`；不得先冻 `f64` 米进度。

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
- 前缀累计超过 `u32::MAX` mm 时拒绝路线注册，或把 `BeyondFinite` 路终饱和成
  `u32::MAX` 硬约束。
- 从路线头前缀溢出后把后续后缀查询一律标成 `BeyondFinite`，导致靠近终点的车无法
  `Completed`。
- 空升 `networkRevisionDerivationVersion = 2` 却沿用 v1 组帧 / 域分隔符，或不定义
  新算法。
- 发布或构建把 LFCA v2 送进只承认 v1 的受检入口。
- 公开一维表面继续用米制作权威，或把 G1 写成现行每个 Rust 访问器的签名对照表。
- 公开 `Canonical*View` 为交通一维保留米制只读换算（#500：删除，不留换算）。
- 用「等长度」或分组行代替逐字段 tag / 名字 / 类型 / 必填的 v2 增量。
- 先按量化前的裸 SI 界限拒绝，再 round 到毫米 / `f32`（与毫米权威打架）。
- 丢掉 ADR 0014 的加减速/时距/尺寸/横向 **上界**，只写下限。
- 路线距离查询或替换阻塞间隙继续用米制作权威。
- 在当前树保留米制 LFCA 登记表、v1 读器或 `*_v1`/`*_v2` 孪生公开 API。
- 把旧米列读成毫米，或为未发布的旧字节堆兼容层 / 迁移 diff。
- 为消除 4 ms 静止跟停或空路巡航量化死区而增加亚微米累加器、更高分辨率
  `carry`、速度余数，或把加速方向改成向上入。
- 把 `vehicle-following.md` §11.2 的 `leader_final_travel` 并入 #496 的
  `hard_room_mm`，打破与现行快照截断的同构。
- 只冻车辆观察面，spawn / pose / 停车 / 替换间隙继续用米制作权威。
- 把一维 mm 收进三维 `Point` / `Vector3<T>`（#354）。
- 把规范折线点、段长、弧长或时间/`u64` 字节长度改成毫米权威（#354 / ADR 0015 /
  0022 边界）。#500 把编译器 Typed AST / HIR / MIR / LIR **交通一维**收口为整数毫米，
  不再拒绝这一项。
- 发 LFCA v2 却不单开 LFSM/LFSD 对象版本，或改写 LFSD v1「Genesis 只允许 v1 支持值」。
- 为制动视距把 `Finite` 加宽成 `u64`，或允许 `comfort`/`emergency` 低于 `0.5 m/s²`。
- 量化后的停车朝向保留 `+π`（`0x40490fdb`）写入制品，或读器把线格式上的 `+π` 折成
  `-π` 再接受（两套字节同一朝向、摘要不同）。
- 负 SI travel 扣减 `carry_um`，或把 `u64` 回绕当行程。

## 后果

- 跟车占用与跨边有产品量级的离散代数；空间几何仍是有界 `f32`。
- 短步长爬行靠微米余数，不靠纳米哨兵；硬停靠整数硬约束，不靠 SI 符号。
- 4 ms 静止跟停在 IIDM 有效加速度过小、本拍不足 0.5 µm 时可以停在期望间距外约
  10 cm；空路巡航在有效加速度小于 `0.5 mm/s / Δt` 时可以稳定低于期望车速（最钝
  合法画像约 7%）。二者都是接受面，不是再加一层余数的理由。
- `hard_room_mm` 与现行快照截断同构；跟车设计文档 §11.2 的投影前车行程仍是另一轴。
- G2 同时改 compiler 发射、唯一登记表、admission、共享列和 Runtime 热状态。
  Genesis 发当前合同；禁止为旧格式做 Artifact 迁移 diff。
- #500：准入后编译器交通一维与制品 / Runtime 同一套整数权威。有折线在空间冻结提交
  弧长量化；停车锚点相对该提交边长关闭。弧长量化越出 `100..=10_000_000` mm 时以边长
  越界失败关闭，不留到发射变成泛型 binding 错误。公开 `Canonical*View` 交通一维只暴露
  毫米 / `mm/s`（时距、加减速、朝向仍为受检 `f32`），删除米制访问器。
- 减速度下限 `0.5` 后，`100 m/s` 刹停约 10 km；`BeyondFinite` 降速目标本拍忽略。
- 路线前缀溢出是 `BeyondFinite`，不是注册失败；路终与局部查询从查询起点独立加。
- `NetworkRevisionId` 仍用 v1 派生算法；载荷变了 ID 就会变，不必新域分隔符。
- 先量化再检查，不保留一套打架的裸 SI 下限。
- #302 不得在本切片完成前进入自身 G1 的快照字段冻结。
- 同进程并行不因撤回跨 CPU 位级承诺而改合同；跨机器联机仍需独立 ADR。

## 实施与治理

1. #496 G1：本 ADR、`traffic-runtime-integer-geometry.md`、ADR 0003/0014 修订
   与 glossary/数值/跟车/消费/信号/制品/共享静态路网 admission 同步。设计 PR
   `Refs: #496`，不 `Closes`。
2. #496 G2：唯一 Delivery PR 实现合同、对齐走廊相位并重生夹具，`Closes #496`。
3. #500 G1：修订本节编译器 IR 权威与 `traffic-runtime-integer-geometry.md` §6。
   设计 PR `Refs: #500`，不 `Closes`。不改 Runtime 已提交合同、不改 LFCA 列名。
4. #500 G2：唯一 Delivery PR 把 Typed AST / HIR / MIR / LIR 交通一维改为整数毫米
   （时距 / 加减速 / 朝向为受检 `f32` SI），公开 `Canonical*View` 删除交通一维米制
   访问器、只留毫米权威（`length_mm` / `speed_limit_mm_s` / `progress_mm` /
   `lateral_offset_mm` / `desired_speed_mm_s` / `min_gap_mm`），重生编译器可移植
   夹具，`Closes #500`。
5. 实现中若发现必须改变毫米量子、步长区间、加速度下限或「先整数硬约束再余数」，
   停工并重开 G1。
