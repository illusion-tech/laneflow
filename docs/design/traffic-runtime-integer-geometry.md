+++
schema_version = 1
id = "design-traffic-runtime-integer-geometry"
kind = "design"
status = "accepted"
revised_date = "2026-11-23"
summary = "定义整数毫米运动、速度与停止约束。"
scope = "TrafficWorld 已提交一维几何与速度、WorldConfig 步长、 laneflow-static-network 热列、LFCA 长度/速度字段、compiler Typed AST / HIR / MIR / LIR 交通一维存储、公开 Canonical*View、compiler 边长派生、 Spatial 采样钉死端点"
applicability = "#applicability"
issues = ["https://github.com/illusion-tech/laneflow/issues/196", "https://github.com/illusion-tech/laneflow/issues/1272"]
related = ["adr-0003", "adr-0014", "adr-0015", "adr-0022", "adr-0028", "design-numeric-representation", "design-portable-canonical-artifact", "design-route-system", "design-shared-static-network", "design-signal-system", "design-traffic-runtime-shared-consumption", "design-vehicle-following"]
+++

# 交通运行时整数毫米几何

<a id="applicability"></a>
## 适用边界与依据

停止目标速度归约与耗尽决策表已随 #1272 实现交付（求解器可行域、候选形成、
耗尽决策表、制动前视窗与恢复校验同批落地）。

§5.1 停车可行性保持（含差带余数清除、收紧候选与到位进展）及其对第 5、7 步和舍入规则的修订
由 #196 G1 冻结，实现由后续实现 Issue 交付；交付前现行代码不含 §5.1，仍按
第 3–7 步原序列提交。

关联文档： `../adr/0028-integer-millimeter-traffic-geometry.md`、
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

G1 冻结正本、单位、量化顺序、制品字段与跨实现算法。公开 `Canonical*View`
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

IIDM 仍在 `f32` SI 中算出「这一拍最多走多远」；IIDM 输入不含停止约束，规则
不写入 IIDM。停止约束（#1272）作为空间目标在共同纵向裁决中产生**速度上界**：
接近相（目标距离 `> 0`）按「舒适可行域 → 紧急可行域 → 具名投影」级联收紧
提案速度；紧急减速下界 `F = max(0, v − b_e·Δt)` 在共同候选形成时施加（见
§5）。硬停（`hard_room_mm == 0`，或本拍走完该剩余）的已提交速度由 §5 的
耗尽决策表决定，不再一律清零。#196 起，对持续有效的停止目标，本拍最终提交的
整数状态必须保持所在档位的停车可行性（§5.1）；车辆自身的求解、量化与余数处理
不得使其失去停车能力。不另做速度余数；`max_accel` / `comfort_decel`
/ `emergency_decel` 均 `>= 0.5 m/s²`。

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
`formatVersion` 随当前 LFCA 合同为 `5`。

## 3. 共享列与 profile

共享交通热列（有无 Spatial 都写）：边长 `u32` mm，边限速 `u32` mm/s 且
`1..=100_000`。

profile：车长 `100..=128_000` mm，期望车速 `1..=100_000` mm/s，`min_gap`
`0..=128_000` mm（0 合法，退化为只禁止重叠）；时距与三项加减速为受检 `f32` SI，
范围见 ADR 0028。停车：入口/出口进度 `u32` mm 且相对 **提交后** `length_mm` 满足
`1 <= p <= length_mm - 1`；长宽
`100..=128_000` mm；横向 `i32` mm，`abs <= 128_000`，路外 `abs >= 1`；朝向受检
`f32` 弧度，闭包 `-π <= x < π`。编制/准入量化后若等于 `+π`（`0x40490fdb`）则写成
`-π`（`0xc0490fdb`）；制品存着的 `+π` 非法，读取器只拒不折。限速过渡目标与边限速
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
`+π` 则写成 `-π` 再检查；读取器遇到存着的 `+π` 失败关闭。跨字段比较已量化的双方。

## 4. 车辆已提交状态

```text
progress_mm: u32           // 当前边上，0..=length_mm
carry_um: u16              // 0..=999
speed_mm_s: u32            // 静止可为 0
length_mm: u32             // spawn/replace 时从 profile 拷入
```

公开观察与命令表面以上述整数为正本，含 spawn / 替换 / 车道 pose。新车
`carry_um = 0`。车长可缓存在状态上。只读米制换算可以有，不得当正本、不得回写。
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
3. **共同候选形成与停止目标速度归约（#1272）**。候选
   `U₀ = max(min(u_raw, V_legal, V_safe), F)`，其中 `u_raw` 为 IIDM 提案速度，
   `V_safe` 为 `vehicle-following.md` §10.2 的跟车安全速度（无 leader 时不施加；
   跟车可行时 `V_safe ≥ F`，不可行时由第 4 步硬空间与跟车几何投影处理），`V_legal`
   为当前边合法速度上限（非 `desiredSpeed`；后续边降限速由空间目标与 crossing
   guard 处理，crossing guard 不是当前边通用超速处置），`F = max(0, v − b_e·Δt)`。
   **紧急减速下界在候选形成时施加**：`u_raw` 低于 `F` 而真实安全条件允许 `F`
   时，候选修正到 `F`，属正常结果，不进投影分支。制动前视窗内**已确定必须停**
   的目标（denied 信号/策略门、已判定的 Waiting/Conflict 停止、已知持续阻塞、
   过渡预约、停车入口、有限路终）以 `L = 0` 参与级联；「尚未申请」的资源本身
   不构成目标。级联可行域同时要求：
   - (a) 本拍位移 + `max(0, u² − L²)/(2b)` ≤ 目标距离。位移按实际选中运动计算：
     未拍内停稳用梯形；拍内停稳用实际减速度 `A`（`stop_time = v/A`、
     `travel = v²/(2A)`）且 `A ∈ [v/Δt, b]`——终速在零处饱和，仅检查终速下界
     无法约束实际减速度；
   - (b) 本拍减速度上界：未停稳 `u ≥ max(0, v − b·Δt)`，停稳由 `A ≤ b` 承担。
   舒适域（`b = b_c`）非空取其最大可行 `u`；舒适域空进紧急域（`b = b_e`）；
   两域皆空进**具名几何投影**（travel 投影到目标距离，携带显式归因）。实际
   加速度：未停稳分支 `a_actual = (u − v)/Δt` 唯一确定；停稳分支取档位
   `A = b`（舒适档 `b_c`、紧急档 `b_e`）；`v = 0, u = 0` 时位移为零。「紧急档」
   指使用紧急可行域，不要求未停稳结果固定取 `−b_e`。IIDM 加速度是提案，
   **不得**直接作为归约后实际加速度。归约产出 `(速度, 制动分支, 位移)` 一致
   三元组；同一 `(v, u)` 下不同制动分支给出不同位移，禁止在其后用终态速度
   无条件重建梯形位移，多目标不得互相覆盖较紧目标确定的档位与位移。后续目标
   只收紧不抬高；仅当真实几何、法规或合法速度约束与 `F` 不相容（收紧后
   `< F`）才进入具名投影，投影结果不被 `F` 抬回。归约与第 5、7 步的量化结果
   还须通过 §5.1 的提交判据；未通过时按 §5.1 选取收紧候选。
4. 整数硬约束 `hard_room_mm: u32`（下限 0）。前车空隙用 `i64`，负值当 0，正值夹到
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
5. `hard_room_mm == 0`：`travel_mm = 0`，`carry_um = 0`，不 `apply_travel`，速度按
   第 7 步耗尽决策表。若此时路线剩余为 `Finite(0)`：`VehicleStatus::Completed`。
   `BeyondFinite` 不是路终。否则
   `um = u64(C_eff) + round-ties-to-even(f64(非负 travel_m) × 1e6)`（跟车安全
   量化路径与 §5.1 收紧候选改为向零取整，见下文舍入规则）。`C_eff` 是本拍有效余数：
   §5.1 差带余数清除时为 0，否则为拍初 `carry_um`；
   `travel_mm = min(um / 1000, hard_room_mm)`；
   若 `travel_mm < hard_room_mm`：`carry_um = (um % 1000) as u16`；
   若 `travel_mm == hard_room_mm`：到位后 `carry_um = 0`。
6. 整数 `apply_travel`：`progress_mm + travel_mm`。满边且下一跳许可则余量进下一条、
   `progress_mm = 0`；满边但下一跳拒绝则停在 `progress_mm == length_mm`，保持
   `Active`。
7. **耗尽拍**（`hard_room_mm == 0`，或第 5 步走到 `hard_room_mm`，含首次耗尽）：
   已提交速度 `= min(归约速度, 耗尽上界)`，按**耗尽决策表**决定——

   | 情形 | 规则 |
   | --- | --- |
   | 保持义务下的到线 | 被耗尽的停止目标在本拍负有 §5.1 保持义务时，提交判据推出到线零速；优先于本表其余正常行 |
   | 正常首次耗尽 | 不负 §5.1 保持义务时：`min(u_reduced, 耗尽上界(源))`；正常归约保证 `u_reduced ≥ F`，非过渡源结果即 `F` |
   | 正常持续耗尽（同源 d=0） | 仅出现在不负 §5.1 保持义务的入口之后（具名投影、既有异常初态等）；不再用该源重判 L=0 可行性；位移 0，速度逐拍按源衰减（非过渡 `F`、过渡舒适档） |
   | 过渡预约耗尽 | 耗尽上界 = 舒适衰减；耗尽拍由该上界主导 |
   | 具名几何投影 | 投影优先于耗尽衰减（允许超 `b_e` 并归因）；投影速度不被 `F` 抬回 |
   | 路终 Completed、停车到位 | 当拍零速（终态零速义务，具名例外）；生命周期按各自合同 |
   | 多源落在同一耗尽边界 | 只合并落在该边界上的独立停止义务（同源去重沿用现行过渡/冲突去重）；速度取最严格上界；远处目标只参与接近相归约；Completed/Arrived 生命周期独立判定；现行 `Signal → Parking → RouteEnd` 序只是归因规则，不作速度决策序 |

   非过渡可等待停止源的耗尽上界为紧急衰减 `max(0, v − b_e·Δt)`——该式在
   `v` 小时自然为 0，**包含并取代清零**；过渡维持现行舒适衰减。不承诺统一的
   固定衰减拍数：衰减拍数由到位残余速度、profile 减速度、步长与速度量化
   规则决定。若走完的是**路线剩余 `Finite(0)`**（没有下一 hop）：
   `VehicleStatus::Completed`，离开占用与 `committed_pose_sources`。Gate 拒绝或
   剩余 `BeyondFinite` 都不是 `Completed`。否则
   `speed_mm_s = round-ties-to-even(f64(next_speed_m) × 1000)`（跟车安全量化路径
   改为向零取整），不由 travel 反推。
   整数行程落地后，已提交速度不得超过**所在边**限速（`carry_um` 可能比 SI 包络
   多送 1 mm 跨边）。如何夹紧属 G2。

**正常停车定义与例外（#1272 验收口径）**：前提为停止目标自进入制动前视窗起
持续有效、开始处理时处于紧急可行域内、期间无新增更紧外部约束。满足前提的
停车含最后耗尽拍在内满足

```text
v_n − v_{n+1} ≤ b_e·Δt + ε_v
```

`ε_v` 为显式登记的速度量化容差（覆盖整数 `mm/s`、round-ties-to-even、跟车安全
量化与 §5.1 收紧候选的向零取整，不小于 1 mm/s）。满足前提的轨迹同时负有 §5.1 保持
义务（必要时先清除差带余数），因此在停止线前停稳或到线零速，不经过到线后的
残速衰减。
具名例外只允许：运行期约束骤现的具名投影（cut-in / 前车急停 / 红转边界不可
停 / 本拍新竞争落选 / 阻塞突然出现，逐项归因）、本拍开始时跟车不可行导致的
跟车几何投影（原因须归入新前车进入视野或既有异常初态，见 `vehicle-following.md`
§10.2、§11.3；本拍开始时跟车可行的停车不属于例外）、路终 Completed 拍与停车
到位拍清零、合同允许的既有异常初态入口。
求解器侧下界不能单独保证量化落地后的速度界：`carry` 并入后再判耗尽的整数
落地可以先于 SI 预判耗尽一拍，最终降幅由正常归约下界与耗尽提交规则共同
保证。

占用循环：边上 `remaining_mm == 0` 结束（`u32`）。物理重叠比较路线起点截断后的
非空 `u32` 毫米区间，不比较路线外的负坐标车尾；首 occurrence 进度为零时没有
路线内车身区间。准入额外拒绝同一物理边重合的零进度前杠入口点，防止下一拍
共同进入同一车身区间（`vehicle-following.md` §7.4）。跨 hop
间隙用 checked `i64`。生产路径删除米制 `1e-9` / `1e-12` 比较。

舍入默认 IEEE 754 **round-ties-to-even**，缩放在 `f64` 中做（`f32 × 1e6` 在
100 m 行程上会丢微米）。例外有两类。**跟车安全量化**：本拍开始时有 leader 且跟车
可行（`vehicle-following.md` §10.2）的车辆，第 5 步行程微米增量与第 7 步已提交
速度都向零取整。适用条件只在本拍开始时判定一次，本拍各步不再改判。量化只降低
数值，不改变第 3 步确定的制动分支；量化后速度为零时位移仍取 SI 三元组的行程，
不重新解释为拍内停稳。**停车可行性保持的收紧候选**：常规候选未通过 §5.1 提交判据时，
收紧候选的速度与行程微米增量按 §5.1 精确计算后向零取整；只在判据未通过时使用，
不扩大为所有带停止目标车辆的默认量化。

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

### 5.1 停车可行性保持（#196）

**义务**。拍初处于制动前视窗内、以 `L = 0` 参与第 3 步级联的停止目标，在持续
有效、期间无新增更紧外部约束时，本拍最终提交的整数状态必须保持该目标的停车
可行性：拍初舒适可停时保持舒适可停，仅紧急可停时保持紧急可停。车辆自身的求解、
量化与余数处理造成的越界，不得成为解除停止义务的理由，包括不得据此满足
`signal-system.md` §6 的黄灯困境区能力判据。义务逐拍从已提交状态判定，不保存
「黄灯起点可停」等历史决定，不新增持久状态。保持可停不要求立即固定减速：余量
充足时车辆仍可按提案前进；在档位边界上，保持该档可行即等价于按该档制动（第 3 步
可行域在边界上只含 `u = v − b·Δt`）。

**判据**。记 `D` 为拍初到该目标的路线距离（mm，与第 3 步所用距离相同），`V` 为
拍初已提交速度（mm/s），`C` 为拍初 `carry_um`，`b ∈ {b_c, b_e}` 为档位减速度
（受检 `f32`，按其精确二进制值参与运算），`Δt` 为步长（ms）。定义

```text
Φ_b(V) = ⌈V² / (2·b)⌉                 // µm：(mm/s)² ÷ (m/s²) = µm
连续判据（b 档可停）：  Φ_b(V) ≤ 1000·D
整数判据：             Φ_b(V) ≤ 1000·D − C
```

`Φ_b` 按 `V` 与 `b` 的精确有理值计算后向上取整到整数微米（例如整数或有理运算），
不得以 `f32` / `f64` 舍入后的商代替。`1000·D` 是整数，所以在精确算术下，连续判据
与 `v²/(2b) ≤ d` 等价；`signal-system.md` §6 的困境区能力判据与连续判据共用同一
表达式（紧急档）。二者之差来自尚未落地的余数：连续可停而整数判据不成立的条件是
`1000·D − C < Φ_b(V) ≤ 1000·D`，这一差带宽度可接近 1 mm；向上取整本身的差不到
1 µm。

**保持档位与差带余数清除**。拍初按连续判据取保持档位 `b*`：舒适档成立取 `b_c`；
否则紧急档成立取 `b_e`；都不成立时，本拍对该目标不负保持义务，按第 3 步与具名
投影处理。`b*` 的整数判据不成立（差带入口）时，本拍有效余数 `C_eff = 0`；否则
`C_eff = C`。清除后 `R = 1000·D − C_eff = 1000·D ≥ Φ_{b*}(V)`，整数判据成立。

**提交判据**。设 `q` 为本拍已提交速度（mm/s），`z` 为本拍量化后的行程增量（µm，
不含 `C_eff`）。最终提交必须满足

```text
z + Φ_{b*}(q) ≤ R,        R = 1000·D − C_eff
```

未被 `hard_room_mm` 截断时 `1000·D' − C' = R − z`，下一拍整数判据
`Φ_{b*}(q) ≤ 1000·D' − C'` 成立；被截断时 `C' = 0`、`1000·D' ≥ R − z`，且已提交
速度不高于 `q`，结论不变。到线时（`D' = 0`、`C' = 0`）判据推出 `q = 0`。因此从
整数可行域内部出发不会离开：同一持续目标、同一保持档位、前提不变期间，至多
发生一次该档的差带余数清除。车辆先只能紧急停车、其后恢复舒适可停时，可能为
舒适档再清除一次；次数上界来自证明，不设历史标记。

**候选选择**。第 3–7 步的常规候选（含跟车安全量化）满足提交判据时原样保留。
不满足时，在下列收紧候选中选已提交速度最高者，同速取行程最大者；速度与行程按
精确值计算后向零取整，`q_常规` 为常规候选的已提交速度：

```text
梯形候选：      整数 q，q_下 ≤ q ≤ q_常规，z = ⌊(V + q)·Δt / 2⌋
                q_下 = min(⌊V − b*·Δt⌋, q_常规)（V > b*·Δt）；q_下 = 1（V ≤ b*·Δt）
拍内停稳候选：  V ≤ b*·Δt 时，q = 0，z 取 [Φ_{b*}(V), ⌊V·Δt/2⌋] 内不超过 R 的
                最大值（区间为空时取 Φ_{b*}(V)），实际减速度 A = V²/(2z) ≤ b*
静止末端到位：  V = 0 且 0 < R ≤ ⌊Δt/2⌋ 时，q = 0，z = R
```

`Δt` 以 ms 计时，`(mm/s)·ms = µm`。梯形候选的代价 `z + Φ_{b*}(q)` 随 `q`
单调不减，可行速度构成区间，选最高可行速度即可。全部候选的速度都不高于
`q_常规`，因此不突破跟车、限速与更紧目标已确定的速度上界。

**存在性**。`V > b*·Δt` 时，梯形候选 `q = q_下` 可行：令 `u = V − b*·Δt`（精确值），
则 `q ≤ u`，`z ≤ (V + u)·Δt/2 = T`，且 `T + u²/(2·b*) = V²/(2·b*)`；
`Φ_{b*}(q) < q²/(2·b*) + 1 ≤ u²/(2·b*) + 1`。故
`z + Φ_{b*}(q) < V²/(2·b*) + 1 ≤ Φ_{b*}(V) + 1 ≤ R + 1`；左边是整数，得
`z + Φ_{b*}(q) ≤ R`。`V ≤ b*·Δt` 时，拍内停稳候选 `z = Φ_{b*}(V) ≤ R` 可行，
`Φ_{b*}(0) = 0`。收紧候选的单拍速度降幅比 `b*·Δt` 多出不到 1 mm/s，在 `ε_v`
之内；它可以低于 `F` 不到 1 mm/s，属本节收紧，不是具名投影，也不被 `F` 抬回。
所选候选仍经第 4–7 步硬约束截断与耗尽规则（只降不升），并以最终提交的整数结果
核验提交判据；实数论证不代替这项核验。

**到位与进展**。提交判据要求 `z ≤ R`，到达目标（`C_eff + z = 1000·D`）只能经
`z = R`、`q = 0` 的候选：拍内停稳候选在 `R ≤ ⌊V·Δt/2⌋` 时取 `z = R`，静止末端
到位取 `z = R`。到位时 `D' = 0`、`C' = 0`、速度为零，满足 Parking 到位与
RouteEnd `Completed` 的精确谓词。静止末端到位在拍内先加速后减速：在加速度上限
`a` 与减速度 `b*`（均不低于 `0.5 m/s²`）下，一拍内从静止起停可覆盖
`a·b*/(a + b*)·Δt²/2 ≥ Δt²/8`（m，`Δt` 以 s 计），即至少 `Δt²/8` µm（`Δt` 以 ms
计）；`Δt ≥ 4 ms` 时不小于 `⌊Δt/2⌋` µm，因此在合法 profile 与合法步长下物理可行。

在保持义务的前提下，若车辆静止时常规候选速度为正（无前车且无其余收紧约束时由
IIDM 提案保证），每次提交的行程增量都不小于 1 µm，或直接到位：

- `V > 0`：梯形候选 `z ≥ ⌊V·Δt/2⌋ ≥ ⌊Δt/2⌋ ≥ 2`，拍内停稳候选 `z ≥ Φ_{b*}(V) ≥ 1`；
- `V = 0`：`R > ⌊Δt/2⌋` 时，梯形候选 `q = 1` 的代价为 `⌊Δt/2⌋ + 1 ≤ R`
  （`b ≥ 0.5 m/s²` 时 `Φ_b(1) = 1`），可行且 `z ≥ 2`；否则静止末端到位。

`R` 每拍严格减小，因此车辆在有限拍内到达持续有效的目标，不会在可行域内部永久
停滞。只需停在线前的目标（如信号）同样适用：车辆停在停止线前或恰好到线零速，
不越线。

多个目标同时负有保持义务时，提交判据对每个目标分别成立。有效余数属于车辆：任一
目标处于差带入口即清除，清除只会放宽其余目标的判据。候选须对全部目标同时满足
提交判据：梯形候选的可行速度取各目标可行区间的交集，拍内停稳候选与静止末端到位
按最小的 `R` 取值。候选集合非空：按最严档位构造的存在性候选（梯形 `q_下` 或拍内
停稳 `z = Φ_{b*}(V)`）对全部目标成立，因为紧急档候选的速度与行程都不高于舒适档
对应候选，且舒适档目标满足 `R ≥ Φ_{b_c}(V) ≥ Φ_{b_e}(V)`。

**有效余数的生命周期**。`C_eff` 只作用于本拍候选：P2 预览、查询与未完成的推进
不修改已提交状态；只有成功提交时，余数才与速度、进度一起原子写入。共同运动内核
的标量与列式路径、P2 预览、P5 重算与预览复用消费同一个有效余数、同一判据与同一
候选选择（预览复用条件见 `traffic-runtime-phase-protocol.md` §3.1）。

**成本**。每次修正舍弃不超过 999 µm 的历史行程余数，不增加公开状态字段；允许
现有余数（`carry_um` 可经公开访问器观察）、后续整数进度和轨迹发生变化。该单次
上界不代表每个停止目标或整段轨迹的累计差异上界。

**范围**。本节保证只覆盖拍初连续判据在某档成立的车辆。拍初紧急档也不可停的车辆
不负保持义务：黄灯下按困境区逐拍许可处理，其余来源按具名投影处理；此后若因正常
减速恢复可停，自恢复之拍起重新负保持义务。整数判据未通过不构成困境区许可或其他
解除停止义务的理由。

## 6. compiler 与 LFCA

#500：编制 `f64` 只存在准入边界之前。准入量化一次之后，Typed AST / HIR / MIR / LIR
的交通一维存储正本是整数毫米；时距、三项加减速、朝向是受检 `f32` SI。不得把原来的
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
          对账失败关闭（观察值，不把米列当第二正本）
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
`ContractVersions.canonicalFormatVersion` 为 `5`；
`constraintContractVersion` 为 `3`；`staticExecutionContractVersion` 为 `5`；
`networkRevisionDerivationVersion` **保持 `1`**（§4.2 组帧与
`"laneflow.network-revision.v1\0"` 未改；毫米载荷与路线表删除会改变 ID，不必新算法）；
`identityEncodingVersion` 保持 `1`，`identityRegistryRevision = 4`。
版本已随 #284 格式登记更新，完整版本矩阵见路权策略实施合同 §8。
公开 API 不带世代后缀。读取器拒绝 `formatVersion != 5`。
旧米制表与旧读取器不进当前树。LFSM `sourceMapFormatVersion = 4`，
`canonicalArtifactFormatVersion` 等于所绑 LFCA。LFSD `semanticDiffFormatVersion = 4`。
Genesis 的 target 合同行必须与所绑 LFCA 一致；Artifact 两端合同行仍须相等。走廊按
Genesis 重生，不做格式迁移 diff。

共享路网受检输入与后发射检查只走这一套 registry，不可伪造。

当前 LFCA 交通热列（**不兼容**旧 `f64` 米列）。**只改下表各行相对历史米制表的名字和/或类型**；
未列出的字段保持原 tag、名字、类型、必填。不得用「等长度」打包。
`fieldType`：`3=u32`、`5=f32`、`13=i32`。Spatial `LaneEdgeGeometry.arcLengthMeters` /
`segments.lengthMeters` 仍为 `f32` 米，不进本表。

| 表             | tableKind | tag | 历史米制 `name:type:R`                                | 当前毫米制 `name:type:R`                              |
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

| 当前字段                                                       | 闭包                                                                     |
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

后发射检查拒绝非当前格式的字节。走廊检入 LFCA 必须按当前 `formatVersion = 5`
重生并对拍。
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
的交通侧用毫米，弧长侧仍是 `f32` 米。无 Spatial 时不建采样 session；一维边长正本
不受影响。

## 8. 确定性与验收

- 已提交 mm / mm/s / `carry_um`：同一软件版本、同一运行环境、同一初始状态和
  同一 tick/input 序列必须一致（ADR 0003）。**不**承诺跨 CPU / 跨机器位级相同。
- IIDM `f32` 与规范弧长 `f32`：不承诺跨 CPU 位级相同。余数输入依赖它们，因此
  整数状态也不另作跨 CPU 承诺。
- 同进程并行（同一二进制、同一运行环境）必须与 1-worker 得到相同已提交状态与
  事件序。联机 / 跨机器 lockstep 不在本部分。
- 不得要求与 current-`f64` 录影零分歧，也不得用 2 车走廊墙钟当 Product Pass。
- 必测（按行为，不按访问器名字）：硬停清余数（速度按耗尽决策表）；爬行时速度保持、余数增长；
  跨边余数保留；拒绝 Gate 时停在边终点且保持 `Active`；走到路线终点进入
  `Completed`；停止目标级联三域与实际加速度选择（含 raw 提案低于 `F` 而安全域
  非空、拍内停稳实际减速度越界两类反例回归锚）；耗尽决策表六行；正常停车
  `v_n − v_{n+1} ≤ b_e·Δt + ε_v` 含最后耗尽拍；4 ms 步长与最小合法减速度组合的
  衰减链；`carry` 上界与距线 1 mm 的末端量化；黄灯困境区与红转边界归因；
  `max_accel < 0.5` 失败；headless 边长即准入毫米；有折线边长即提交后的
  弧长量化毫米，且等于 LFCA `lengthMillimetres`；停车锚点贴着弧长量化后的端点留白；
  10 km 边在编译侧与发射侧同一闭包；弧长量化越出 `100..=10_000_000` mm 以边长越界
  失败，不是发射 binding 错误；跨 hop 间隙 `i64`；路线注册在前缀溢出时仍成功，从起点
  可 `BeyondFinite`，靠近终点可 `Finite(0)` 并 `Completed`；`0.0996 m` → `100 mm`
  合法、`0.0994 m` → `99 mm` 失败；`formatVersion != 5` 失败关闭；
  `networkRevisionDerivationVersion == 1`；4 ms 跟停死区状态重复不是失败；快照
  `hard_room` 与现行截断同构；`dt=3` 与相位不能整除均失败且原因可区分；`dt=4` 与
  `dt=1000` 能 install（fixture 相位允许时）；`60 km/h` 长期平均由余数对齐量化后的
  `mm/s`。
- §5.1 必测（#196）：从真实转折拍的完整整数状态（速度、距离、余数与约束）出发，
  舒适档与紧急档的边界骑行在最终提交后保持对应整数判据；差带入口清除余数后进入
  保持，含 #196 评审给出的两组差带入口状态（`V = 6635 mm/s`、`D = 3669 mm`、
  `C = 614 µm` 与 `V = 6405 mm/s`、`D = 3419 mm`、`C = 378 µm`，舒适与紧急减速度
  `2 / 6 m/s²`、步长 16 ms），清除后不再出现困境区许可；先紧急后舒适的二次清除；
  目标持续时停稳或到线零速，不出现到线后的残速衰减；拍初紧急档不可停的车辆仍获
  困境区许可，不为满足新断言强制超减速度停车；黄转红义务延续、转绿释放；收紧
  候选不高于常规候选速度、取最高可行速度且单拍降幅在 `b·Δt + ε_v` 内；多目标的
  候选对全部目标成立；从正常初态出发的到位进展：#1504 评审给出的状态（初速
  `1 m/s`、距持续目标 `1.149 m`、余数 0、加速度上限 `2 m/s²`、舒适与紧急减速度
  `2 / 6 m/s²`、步长 16 ms、期望速度 `16.667 m/s`，无前车）逐拍保持提交判据并
  精确到位，不出现零速固定点；静止末端到位在 4 ms 与 1000 ms 步长下完成；以同类
  初态完成 Parking 到位与 RouteEnd `Completed`；跟车
  无重叠、红灯拒绝、Parking 精确到位、RouteEnd `Completed` 与低速推进不退化；
  标量、列式、P2 预览、P5 重算与预览复用的已提交状态一致；性能增量经测量并由
  维护者接受。

## 9. 明确不做

- #302 容器字段布局（但必须预留本整数状态）。
- #303、#354 三维 math kernel。
- 残差 `f32` 进度、整数 IIDM、无余数格子、速度余数。
- Runtime 慢放 API。
- 跨 CPU / 跨机器位级回放或联机 lockstep。
- G1 改走廊 toml 或重生 LFCA。
- 强迫 headless 从折线弧长派生边长。
- 在当前树保留米制 LFCA 登记表、旧读取器或 `*_v1`/`*_v2` 孪生公开 API。
- 把旧米列读成毫米，或为未发布旧字节堆兼容层。
- 亚微米累加器，或把 4 ms 静止跟停量化死区当缺陷消掉。
- 把 IIDM 提案加速度直接当作空间目标归约后的实际加速度；停止目标归约后的
  停车位移由终态速度无条件梯形重建（§5.1 收紧候选按其明确规定的分支计算位移，不属
  此列）。
- 为 §5.1 新增持久状态（`YellowCommitted`、清除计数、黄灯起点决定等）。
- 把 §5.1 收紧候选的向零取整扩大为所有带停止目标车辆的默认量化；把整数判据未通过当作
  困境区许可或其他解除停止义务的理由；以放宽 `ε_v` 或统一的 1 mm 容差代替差带
  余数清除。
- 把 `vehicle-following.md` §11.2 的 `leader_final_travel` 并入本部分 `hard_room_mm`。
- 用「等长度」打包交通热列字段，或在当前树并行保留历史米制表。
- 前缀超过 `u32::MAX` mm 时注册失败，或把 `BeyondFinite` 饱和成 `u32::MAX` 路终硬停。
- 从路线头溢出后把后续后缀查询一律标成 `BeyondFinite`。
- 空升 `networkRevisionDerivationVersion = 2` 却不改哈希算法。
- 发布或构建把当前 LFCA 送进只承认旧格式的入口。
- 公开一维表面继续用米制为正本。
- 公开 `Canonical*View` 为交通一维保留米制只读换算。
- 把 G1 写成现行每个 Rust 访问器的签名对照表。
- 先按量化前的裸 SI 界限拒绝，再 round 到毫米 / `f32`。
- 把规范折线点、段长、弧长或时间/字节长度改成毫米正本。
- #500 G1 改生产代码或重生 fixture。
