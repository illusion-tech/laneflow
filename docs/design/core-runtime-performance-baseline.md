# Core Runtime 产品性能基线

**文档状态**: Accepted<br>
**最后更新**: 2026-07-24<br>
**适用范围**: LaneFlow Core、Spatial、Engine Adapter 的 10k/100k 产品目标，1M 研究包络，以及性能、保真度、硬件和证据协议<br>
**关联文档**:

- [`core-runtime-scalability-audit.md`](core-runtime-scalability-audit.md)
- [`core-runtime.md`](core-runtime.md)
- [`adapter-api.md`](adapter-api.md)
- [`bevy-reference-adapter.md`](bevy-reference-adapter.md)
- [`../adr/0001-project-scope.md`](../adr/0001-project-scope.md)
- [`../adr/0003-runtime-tick-and-determinism.md`](../adr/0003-runtime-tick-and-determinism.md)
- [`../reference/validation-matrix.md`](../reference/validation-matrix.md)
- [#215 G1 冻结判断](https://github.com/illusion-tech/laneflow/issues/215#issuecomment-5060652396)

## 1. 目的与状态语义

本文冻结 LaneFlow 的产品性能**目标与测量契约**（target and measurement
contract），用于让单线程优化、多频率候选、单机并行和未来聚合研究使用相同
workload、fidelity、hardware 与 frame-budget 口径。

本文不是硬件认证报告，也不把历史开发机数据升级为产品服务等级协议（product
SLA）。必须区分：

- **目标已定义**：本文已经给出规模、workload、预算、保真度和测量协议。
- **Research evidence / Research Pass**：结果来自 R0 研究机，可以指导优化，但不能
  代表最低或推荐产品硬件。
- **Product Pass**：结果在对应 P10/P100 实机上按本文完整协议通过。
- **Product TBD / Uncertified**：真实产品硬件或必要容差尚未确定，不能宣称通过或
  失败。

因此，本文合入只表示产品基线契约可供下游依赖，不表示 10k/100k 已完成产品
certification，也不表示 1M microscopic realtime 已成为产品目标。

## 2. 范围与非目标

本文覆盖：

- 10k、100k、1M 的规模计数语义；
- canonical synthetic workload 与 presentation selection；
- 研究机、最低产品机、扩展参考机和 1M 观察机的角色；
- Core fixed tick、outer frame、Spatial+Adapter 与宿主剩余预算；
- hard invariant、individual、presentation、aggregate fidelity；
- latency、tail、allocation、retained memory、catch-up、失败重试和 profiler 证据；
- 结果分类，以及 single-thread、multi-rate、parallel、aggregate 的升级触发；
- 产品声明限制、TBD 台账和下游 Issue 输入。

本文不覆盖：

- production scheduler、thread pool、partition、work stealing 或 distributed runtime；
- production multi-rate、interpolation/extrapolation 或 aggregate model 的实现；
- 新 Core/Data/Spatial/Adapter API 或数据格式；
- renderer、asset、animation 或宿主 gameplay 的实现与性能承诺；
- 专业交通工程精度、城市经济模拟或完整 SUMO-like 能力；
- 1M 个体车辆的实时产品承诺。

## 3. 规模计数语义

以后不得裸写“10k/100k/1M agents”。每个结果必须同时记录以下五个正交计数：

- `N_individual`：仍存在且保留完整 logical identity、route/progress、Vehicle
  Profile、Parking/lifecycle 与 committed state 的个体车辆。行驶中与停车中的车辆
  均计入，并按 status/lifecycle 分解。
- `N_traffic_active`：当前处于道路交通系统、每个 Core base tick 参与 travel-lane
  occupancy/leader、constraint、global projection、advance/events 与 atomic motion
  commit 的车辆。因红灯或前车停止但仍在道路上的车辆继续计入；Parked vehicle
  不计入。
- `N_intent`：该 tick 实际重新计算昂贵 controller intent 的车辆数。exact-only
  通常等于 `N_traffic_active`；reduced-rate candidate 可以更小，但不能借此跳过
  `N_traffic_active` 的逐 tick safety authority。
- `N_presented`：该 outer frame 被 Adapter/Presentation materialize、extract 或
  commit 的车辆数。它可以包含需展示的 parked vehicle，但不能反向定义 Core
  fidelity。
- `N_aggregate`：只以 flow、packet 或 count 存在、没有完整逐车 identity 的人口。
  必须单独报告，不能混入 `N_individual` 或用来宣称 active individual agents。

基本关系为：

```text
N_traffic_active <= N_individual
N_intent <= N_traffic_active
```

`N_traffic_active = N_individual` 只表示 100% road-active 的特定 workload，不是
架构恒等式。Parked vehicle 保留 Core 权威的 identity、Parking binding、occupied
state 与确定性 lifecycle transition，但不因此进入每个 tick 的道路运动求解。

推荐使用显式标签记录一个 case，例如：

```text
N_individual=100000; N_traffic_active=75000;
N_intent=<observed mean/distribution>; N_presented=10000; N_aggregate=0
```

### 3.1 标称规模角色

| 标称规模 | 角色                          | 强制解释                                                    | 当前产品状态                           |
| -------: | ----------------------------- | ----------------------------------------------------------- | -------------------------------------- |
|      10k | 产品基线（product baseline）  | 同时报告五个计数与 status/lifecycle mix                     | `Product TBD / Uncertified`，等待 P10  |
|     100k | 扩展目标（scale target）      | 优先保留 100k `N_individual` 的 strong-individual semantics | `Product TBD / Uncertified`，等待 P100 |
|       1M | 研究包络（research envelope） | 分开报告 identity-preserving candidate 与 `N_aggregate`     | Observation only，无 realtime SLA      |

## 4. Canonical workload

不使用单一平均场景承担全部判断。10k/100k 冻结四类互补的 canonical synthetic
workload：

| Workload                 | 个体构成                                                        | 场景要求                                                                                           | 主要验证目标                                         |
| ------------------------ | --------------------------------------------------------------- | -------------------------------------------------------------------------------------------------- | ---------------------------------------------------- |
| W1 Mixed product         | 75% `N_traffic_active` / 25% parked                             | 确定性多 edge/route；混合 following/free-flow、Signals、Parking 与 lifecycle transition            | 主要产品预算、综合吞吐与常规 tail                    |
| W2 Dense following       | 100% `N_traffic_active`；约 15/16 vehicles 在 leader horizon 内 | 延续 dense cohort 压力形态并保留 free-flow 边界                                                    | occupancy/leader 与 longitudinal 持续最坏负载        |
| W3 Parking/lifecycle     | 25% `N_traffic_active` / 75% parked                             | Parking arrival/release、Completed、spawn/despawn/atomic replace                                   | 个体内存、Parking authority 与 lifecycle transaction |
| W4 Synchronized boundary | 使用 W1 个体构成                                                | 将 Signal release、route/edge transition、Parking release 或 lifecycle boundary 对齐为确定性 burst | p95/p99/max、同步尖峰与 failed-step/retry            |

适用规则：

- W1 是主要性能 go/no-go workload。
- W2–W4 是热点、tail、安全和恢复 guardrail；不能只因绝对耗时高于 W1 就判定
  产品失败。
- 10k/100k 运行完整 W1–W4 矩阵。
- 1M 默认只运行 identity、retained-memory 与有限 observation；不运行完整实时
  Gate。
- W1/W3 至少运行 `N_presented = 1% / 10% / 50% / 100%`；其中只有
  10k W1 的 100% 行和 100k W1 的 10% 行是 presentation Gate 主行，其余行是
  强制 observation/sensitivity，不单独产生 Product Pass/Fail。
- `N_intent` 不预设；exact-only 与 reduced-rate candidate 分别报告实际
  mean/distribution。
- 每个适用场景覆盖完整 Signal cycle 和足够的 route、Parking、lifecycle
  transition。
- 75/25 与 25/75 是可复现 synthetic 比例，不是现实部署分布声明。未来 telemetry
  只能新增 telemetry-derived workload，不能静默替换 canonical baseline。
- kernel/microbenchmark 只用于阶段归因，不能替代 W1 integrated product Gate。

每个 case 必须冻结相同的规模口径、seed、configuration provenance、fixed-step
input sequence 与 candidate/oracle 对照边界。

presentation matrix 的判定角色固定如下：

| 规模 / workload | Gate 主行                 | 其余强制行                                    |
| --------------- | ------------------------- | --------------------------------------------- |
| 10k W1          | `N_presented=100%`        | `1% / 10% / 50%` observation/sensitivity      |
| 100k W1         | `N_presented=10%`         | `1% / 50% / 100%` observation/sensitivity     |
| 10k/100k W3     | 无 W1 presentation budget | `1% / 10% / 50% / 100%` guardrail/observation |

### 4.1 `LF-SYNTH-v1` 确定性生成契约

当前 canonical workload 使用确定性生成的匹配规模路网
`LF-SYNTH-v1`，不以某个现实城市或部署分布为代表。真实路网的来源选择、许可、
裁剪、Traffic/Spatial 转换和可复现制品由 #224 单独跟踪；在该工作完成前，
`LF-SYNTH-v1` 只能支持 synthetic baseline claim，不能支持 real-road
representativeness claim。

`LF-SYNTH-v1` 以 100 个 individual 为一个 cell。10k 生成 100 cells，100k
生成 1000 cells；每个 cell 固定包含 4 条互不相交的有向 route、8 条 edge、
1 个 fixed-time signal controller、4 个 signal group/gate/stop line、
1 个 ParkingArea 和 100 个 ParkingSpace。

| 项目                        | 每 cell 冻结值       | 10k / 100k 归一化总数 |
| --------------------------- | -------------------- | --------------------- |
| individual logical slots    | 4 routes × 25 slots  | 10,000 / 100,000      |
| route                       | 4                    | 400 / 4,000           |
| edge                        | 4 entry + 4 exit     | 800 / 8,000           |
| signal controller           | 1                    | 100 / 1,000           |
| signal group/gate/stop line | 4 / 4 / 4            | 400 / 4,000 each      |
| ParkingArea                 | 1                    | 100 / 1,000           |
| ParkingSpace                | 4 routes × 25 spaces | 10,000 / 100,000      |

拓扑与 Spatial geometry 按以下算法生成：

1. `cell_index`、`route_index`、`edge_index`、`slot_index` 都从 0 开始，并按
   `(cell, route, edge, slot)` 升序 normalization。
2. 每条 route 只包含一条 5130 m entry edge 和一条 5130 m exit edge，唯一连接为
   `entry -> exit`，总 route length 为 10260 m；cell 之间没有 lane graph
   connection。两条 edge 的 speed limit 均为 13.9 m/s。
3. `column = cell_index mod 32`，`row = floor(cell_index / 32)`；cell origin 为
   `x = -16300 + 1024 × column`、`z = -16300 + 1024 × row`，单位为米。
4. route `r` 的 local base z 为 `80 × r m`。从 `(0, base_z)` 开始生成 51 条
   200 m 水平 run；偶数 run 沿 `+X`、奇数 run 沿 `-X`，相邻 run 由 1.2 m
   `+Z` connector 连接。完整 centerline 长度精确为
   `51 × 200 + 50 × 1.2 = 10260 m`。
5. 在 centerline progress 5130 m 的 `(100, base_z + 30)` 处分割 entry/exit
   edge；该点是 run 25 的中点，必须同时成为 entry 的最后一点和 exit 的第一点。
   1000 cells 的所有点位于每轴闭区间 `[-16384, 16384] m`，不同 route/cell 的
   geometry band 不重叠。

每条 route 在 entry/exit 边界设置一个 stop line 和一个 gate，并映射到本 cell
对应的 signal group。controller 使用固定 58 s 六阶段周期：

| 阶段     | 时长 | route 0/2 | route 1/3 |
| -------- | ---: | --------- | --------- |
| A green  | 30 s | Green     | Red       |
| A yellow |  3 s | Yellow    | Red       |
| all red  |  1 s | Red       | Red       |
| B green  | 20 s | Red       | Green     |
| B yellow |  3 s | Red       | Yellow    |
| all red  |  1 s | Red       | Red       |

W1–W3 的 controller offset 为
`(cell_index × 997) mod 58000 ms`；W4 全部为 `0 ms`，用于形成同步边界。

每条 route 在 exit edge 上生成 25 个 parking anchors：第 `s` 个 anchor 的 edge
progress 为 `5 + 7.5 × s m`。每个 ParkingSpace 的 length 为 5.0 m、width 为
2.4 m，entry/exit anchor 相同，lateral offset 为 3.0 m，heading offset 为
0。该非零 lateral offset 满足 current static Parking contract。

所有 individual 使用同一基础参数：length 4.5 m、desired speed 13.9 m/s、
minimum gap 2.0 m、time headway 1.5 s、maximum acceleration 1.4 m/s²、
comfortable deceleration 2.0 m/s²、emergency deceleration 6.0 m/s²。

### 4.2 W1–W4 population 与 spacing

每条 route 的 25 个 logical slots 同时是稳定 identity 顺序和 ParkingSpace
顺序。active individual 的 route progress 若小于 5130 m，则映射到 entry edge；
否则映射到 exit edge，并使用 `edge progress = route progress - 5130 m`。

| Workload                 | 每 cell 的确定性初始 population                                                                                                                                                                                                                                                                            |
| ------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| W1 Mixed product         | route 0：25 active，progress `20 + 6.5 × s m`，speed 1.0 m/s；route 1：25 active，同 spacing/speed；route 2：slots 0–11 为 12 active，progress `20 + 28 × s m`，speed 13.9 m/s，其余 13 parked；route 3：slots 0–12 为 13 active，同 free-flow spacing/speed，其余 12 parked。合计 75 active / 25 parked。 |
| W2 Dense following       | 四条 route 的全部 25 slots active，progress `20 + 6.5 × s m`，speed 1.0 m/s。相邻车辆 front-position 间距为 6.5 m，扣除 4.5 m 车长后恰为 2.0 m minimum gap，因此每条 route 除最前方车辆外的 24/25、全局 15/16 individual 有 leader。                                                                       |
| W3 Parking/lifecycle     | route 0–2 的 slots 0–5 和 route 3 的 slots 0–6 active，progress `20 + 28 × s m`，speed 13.9 m/s；其余 parked。合计 25 active / 75 parked。route 3 slot 6 是 lifecycle probe，在 observation 首个 boundary 按下文固定序列重建。                                                                             |
| W4 Synchronized boundary | 继承 W1 的 topology、population 与 spacing，但所有 controller offset 为 0；固定输入序列必须让指定 Signal、route/edge、Parking release 和 atomic lifecycle replace 在同一 observation boundary 发生。W4 只改变边界相位和输入序列，不改变 normalized counts。                                                |

28 m free-flow front-position 间距高于基础参数在 13.9 m/s 时的
`4.5 + 2.0 + 1.5 × 13.9 = 27.35 m` equilibrium spacing。任何 benchmark
harness 对上述 topology、population、spacing、phase 或 stable order 的改变，
都必须产生新的 workload ID，不能仍标记为 `LF-SYNTH-v1`。

W1/W2 的 active population 以及 W3 中非 probe 的 active population 在正式
case 内不得自然耗尽。16 ms 与 33 ms 行从 warm-up 开始到 observation 结束最多分别
经过 696.384 s 与 697.356 s；13.9 m/s speed limit 下最多前进 9693.249 m。
最大普通初始 progress 为 356 m，剩余 9904 m，因此在完整 4+8 Signal cycles
内不会到达 route end。harness 不得用未声明 recycle 补偿更短的私有 route。

W3 的 lifecycle probe 使用以下 fixed-step input sequence：

1. warm-up 结束后的首个 observation boundary，按 cell 升序对 route 3 slot 6
   顺序执行 caller-owned despawn 和 spawn command；spawn 复用同一 logical
   external ID，新 handle 保留该 logical slot，route progress 为 10240 m、
   speed 为 13.9 m/s。该 command batch 计入 W3 lifecycle burst；任一 command
   失败都使该 round 无效。
2. Core step 后按 ordered completion event 建立 frozen replacement plan；下一个
   lifecycle boundary 使用 Core atomic replace，把同一 logical external ID 的新
   handle 放回 route progress 188 m、speed 13.9 m/s。
3. 顺序固定为 `apply pending lifecycle commands -> Core fixed step -> consume
   ordered completion events -> enqueue next-boundary plan`。Blocked plan 不重算、
   不改变顺序，并按同一 frozen input 在后续 boundary 重试；出现未在 manifest
   记录的持续 Blocked 或 active-count drain 时，该 round 无效。

### 4.3 选择、manifest 与防投机约束

- canonical seed 固定为 0。需要选取精确 `N_presented` 比例时，为每个
  `logical_rank` 计算
  `rank_key = SplitMix64::new(logical_rank xor seed).next_u64()`，按
  `(rank_key, logical_rank)` 升序排序后取该比例要求的精确数量，不得直接取 ID
  prefix。
- 每次正式运行必须记录 workload ID、scale、source commit、generator
  implementation commit、归一化对象计数、配置摘要，以及生成后的
  topology/state/fixed-input-sequence SHA-256。
- 同一 workload ID 的同一 scale 必须生成相同 stable order 和相同摘要；任一
  topology/state/input-sequence digest 不同即视为不同 workload。
- production runtime 不得识别 workload ID、seed、cell/route/vehicle ID 或
  选择结果来走专用路径。
- 本文只冻结生成契约，不实现 benchmark harness。后续 harness 尚未输出并校验
  manifest/digest 前，10k/100k 继续保持 `Product TBD / Uncertified`。

## 5. 硬件与平台角色

| 角色                  | 当前基线                                                                                                                                                                | 用途与认证状态                                                       |
| --------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------- |
| R0 Research reference | MECHREVO JIAOLONG；AMD Ryzen 9 9955HX 16C/32T；61.68 GiB；Windows 11 Pro Insider Preview build 29617；平衡电源计划；`x86_64-pc-windows-msvc`；Rust 1.96.0 / LLVM 22.1.2 | 延续 #212 paired optimization 与 profiling；只产生 research evidence |
| P10 Product minimum   | 具体设备/SKU、release OS、内存容量与数值内存上限均为 TBD                                                                                                                | 10k 产品 SLA 的最终认证平台；填入前保持 Uncertified                  |
| P100 Scale reference  | 具体设备/SKU、release OS、内存容量与数值内存上限均为 TBD                                                                                                                | 100k 扩展目标的最终认证平台；填入前保持 Uncertified                  |
| O1 1M observation     | 暂用 R0                                                                                                                                                                 | 只记录 observation，不形成产品 SLA                                   |

硬件和环境规则：

- 当前第一参考平台只覆盖 Windows x86-64。Linux、macOS、Web、移动端必须分别
  建立平台基线，不能从 R0 外推。
- R0 可以报告绝对实验室数值和 paired relative gain；Insider OS、笔记本热/功耗
  状态与平衡电源计划使其不能代表 product minimum。
- P10/P100 必须绑定具体设备或固定 SKU，不能只写核心数和标称 GHz。
- 不通过人为限频或减少核心数模拟 P10/P100；这类结果只能标记为 sensitivity
  experiment。
- 产品认证必须记录 AC/电池、厂商性能模式、Windows 电源计划、OS build、
  BIOS/firmware、CPU、内存配置和工具链。
- 所有规模均报告 retained memory、working set、private bytes 与 commit peak。
- P10/P100 未落实真实设备前，可以继续优化研究，但不得发布 10k/100k 产品 SLA。

## 6. Tick、frame budget 与责任边界

| 层级           |         Core fixed step | Outer frame |    主 presentation | p95 budget                                                                          |
| -------------- | ----------------------: | ----------: | -----------------: | ----------------------------------------------------------------------------------- |
| 10k baseline   |                   16 ms |       60 Hz | 100% `N_presented` | Core ≤ 2 ms/tick；Spatial+Adapter ≤ 4 ms/frame；普通单 tick integrated frame ≤ 6 ms |
| 100k scale     |                   33 ms |       60 Hz |  10% `N_presented` | Core ≤ 16 ms/tick；Spatial+Adapter ≤ 4 ms/frame                                     |
| 100k stretch   |                   16 ms |       60 Hz |  10% `N_presented` | Observation only，不作为当前产品 Gate                                               |
| 1M observation | 不冻结 realtime cadence |      不冻结 |         按实验声明 | 无 realtime budget                                                                  |

16 ms 是 62.5 Hz，33 ms 约为 30.3 Hz；它们不是精确的 60/30 Hz。当前 Core 使用
session-fixed integer-millisecond quantum，因此 60 Hz outer-frame accumulator 的
0/1/偶发多 tick 行为必须进入测试和预算。

责任边界：

- **Core**：traffic authority、occupancy/leader、longitudinal、global projection、
  advance/events 与 atomic commit。
- **Spatial+Adapter**：committed snapshot/materialization、pose extraction、mapping
  validation、Transform staging/apply。
- **Renderer/host**：renderer、asset、animation 与 gameplay 不属于 LaneFlow
  budget；10k 普通帧约保留 10.7 ms 给宿主。

Core、Spatial 与 Adapter 必须在同一次 integrated run 中计时。不同版本、fixture
或进程的历史 percentile 不得相加后冒充 integrated result。

100k scale 的 Core 平均负载按 33 ms quantum/约 30 Hz cadence 折算约为
8 ms/60 Hz outer frame；加 4 ms presentation 后平均约 12 ms。同步 tick frame 仍可
接近 20 ms，因此当前单线程预算只证明 compute target，不构成完整无卡顿 60 FPS
frame SLA。完整 host-frame 目标是否需要并行由第 10 节触发条件决定。

Catch-up 契约：

- canonical `max_catch_up_steps = 2`；
- 达到上限时保留 backlog，不丢 simulation time；
- 不丢中间 `StepResult` 或 events；
- catch-up frame 单独统计，不混入普通帧；
- 恢复到小于一个 quantum 的 frame 数必须报告。

## 7. Fidelity contract

### 7.1 Hard invariants：零容忍

以下项目不使用平均值或 percentile；任一违规立即停止候选：

- overlap count 与 Signal/Parking stop-line violation count 必须为 0；
- speed、acceleration、progress 必须 finite，speed 必须非负；
- identity/generation、route occurrence、Parking binding 与 lifecycle continuity
  必须正确；
- Signal phase/group authority 与 exact oracle 完全一致；
- successful step 的 tick/time，以及 failed-step no-commit、first error、
  retry/fresh replay 完全一致；
- event payload、顺序和因果一致；允许的 event timing shift 只按下一节判断；
- 重复运行和适用的 schedule/traversal/worker permutation 不改变
  state/events/error。

性能目标不能放宽这些 invariant。

### 7.2 Individual behavioral fidelity：物理归一化

候选必须与相同 fixed step、相同 workload 的 exact-only oracle 配对。对每辆车按其
Vehicle Profile 计算：

```text
tau = maximum controller-intent age
acceleration_span = max_acceleration + comfortable_deceleration
speed_budget = acceleration_span * tau
distance_budget = desired_speed * tau + 0.5 * acceleration_span * tau^2
normalized_error = absolute_error / physical_budget
```

- speed 使用 `speed_budget`。
- progress/pose 与 gap 使用 `distance_budget`。
- 对每个 logical identity、每个 tick 计算 error。
- p50 只报告；p95 ≤ 0.25，p99 ≤ 0.50，max ≤ 1.00。
- H、2H、4H 三个观察长度分别满足预算，不能只看最终 endpoint。
- route、Parking、completion event 的 payload、顺序和因果 exact；timing shift
  ≤ `tau`。
- exact-only 与 `N=1` 的 `tau = 0`，不执行除零归一化，而要求 exact
  equivalence。
- `tau`、Vehicle Profile 与 workload 参数必须在测量前冻结，不能为过线事后
  放宽。

### 7.3 Presentation fidelity

- committed Core state → Spatial pose → Adapter Transform 在 committed sample 上
  exact，且整批提交失败原子。
- selection、LOD、visibility 与 outer-frame batching 不改变 Core
  state/events/identity。
- 100k/33 ms Core 可以在 60 Hz presentation 重复最新 committed snapshot。
- interpolation/extrapolation algorithm 与 visual smoothness tolerance 保持 TBD。
  独立 G1 前不得反写 authority，也不得读取未提交 candidate state。

### 7.4 Aggregate fidelity

Aggregate 不是当前 production candidate。若第 10 节触发独立 G1，必须分别报告：

- count conservation；
- route/turn flow；
- queue length；
- travel-time distribution；
- signal compliance；
- exact/aggregate boundary 的 identity loss。

非法生成、丢失和数量不守恒零容忍；其他数值 tolerance 在独立 aggregate G1 中
冻结，不能与 individual fidelity 合并成单一分数。

## 8. Benchmark protocol

### 8.1 运行矩阵与观察长度

- 10k/100k：运行 W1–W4 全矩阵。
- 1M：只运行明确声明的有限 observation。
- `H = 1024 ticks`；H/2H/4H 是正式 observation 内的强制 fidelity checkpoint。
- 每个 case 先按实际 fixed step 和离散 phase 推进语义，计算其中最长完整 Signal
  controller cycle 的 `C_signal_ticks`；没有 Signal controller 时取 `0`。不得只用
  名义毫秒总和忽略逐 phase 的 tick 取整。
- 运行长度使用：

  ```text
  warm_up_ticks = max(512, 4 * C_signal_ticks)
  observation_ticks = max(4096, 8 * C_signal_ticks)
  ```

- 正式 observation 必须同时覆盖 H/2H/4H、至少 8 个完整 Signal cycles 和预声明
  的 lifecycle transitions。为覆盖 Signal cycles 而增加的 ticks 同样进入正式
  latency/tail 统计，不能在报告时丢弃。
- 每个 case 运行 3 个独立 fresh-process rounds；candidate 顺序跨 round 轮换。
- 固定 commit、release binary、Rust 1.96、seed、workload configuration 与
  deterministic outer-frame input sequence。

### 8.2 统计与 tail

Core tick、Spatial+Adapter frame 与 integrated outer frame 分别报告
p50/p95/p99/max。普通帧、catch-up frame 与 lifecycle burst 分开统计。

- p50/p95/p99：先得到每个 round 的对应统计量，再取 3 个 round-level 值的中位数。
- max：使用 3 个有效 round 中的最坏值。
- W1 p95：使用第 6 节预算。
- W1 p99：不得超过 `1.5 × p95 budget`。
- W1 max：不得超过 `2 × p95 budget`，且 Core max 不得超过 fixed quantum。
- 上述 W1 budget 只应用于各规模的 presentation Gate 主行：10k 为
  `N_presented=100%`，100k 为 `N_presented=10%`。W1 的其他 presentation 行
  必须运行并报告，但只用于 observation/sensitivity，不能独立产生 Product
  Pass/Fail。
- W2–W4 不直接套用 W1 绝对预算，但必须满足 hard invariants、无持续 backlog、
  无未解释 tail 爆炸。
- W3 的全部 presentation 行都是强制 guardrail/observation；它们验证
  lifecycle、Parking authority、内存与 presentation scaling，不套用 W1
  presentation budget。

### 8.3 Allocation、内存与失败恢复

- warm-up 后 W1/W2 steady-state Core hot path：
  `0 allocation / 0 reallocation`。
- W3 lifecycle burst 可以分配，但必须单独报告 count、bytes 与 high-water。
- 报告 component capacity、retained high-water、working set、private bytes、
  commit peak，并区分预留容量与实际每 tick 写入量。
- 覆盖 outer frame 中的 0/1/2 tick、catch-up limit 与 two-quantum backlog。
- 记录恢复到 `< 1 quantum` backlog 所需 frame 数。
- W3/W4 注入真实 failed step；失败 latency 与正常路径分开，retry 必须 exact
  等于 fresh replay。

### 8.4 Profiling 与证据

- integrated latency run 是产品 Gate 的唯一性能事实源。
- coarse stage timing、独立 Criterion kernel 与 WPR 只用于机制归因；不得把独立
  数字相加后当作 whole-step 分解。
- WPR 使用独立 run；profile overhead 下的 latency 不进入产品 Gate。
- WPR 记录 binary/PDB、commit、命令、ETL byte size/SHA-256 与 symbol report。
- 小型 raw CSV/JSON、configuration、seed、command、environment manifest 与
  derived report 进入版本化 evidence。
- 多 GB ETL 不提交 Git；记录 size、SHA-256、采集命令和派生报告。存在持久外部
  制品时附链接，否则明确标记未长期保留。
- 外部干扰导致废弃时重跑整个 round 并记录原因；不得选择性删除样本或只发布
  最好 round。
- 每份结果保存第 5 节要求的 OS、CPU、memory、AC、电源计划、厂商模式与后台进程
  provenance。

## 9. 结果分类

| 分类              | 必要条件                                                                 | 允许的声明                             |
| ----------------- | ------------------------------------------------------------------------ | -------------------------------------- |
| `Research Pass`   | R0 按第 6–8 节通过，但 P10/P100 未填写                                   | 可以指导优化；不得发布产品 SLA         |
| `Product Pass`    | 对应 P10/P100 实机通过 performance、fidelity 与完整 protocol             | 可以声明对应硬件和 workload 的产品结果 |
| `Candidate No-go` | hard invariant、fidelity、determinism、atomicity 或 integrated Gate 失败 | 只否决该候选，不终止整个性能方向       |
| `Product TBD`     | 真实硬件或必要 tolerance 尚未定义                                        | 不得伪装为 Pass 或 No-go               |

产品报告必须同时写明：规模五计数、workload、硬件角色、cadence、presentation
比例、classification 与未决项。只给一个“支持 100k”的数字不符合本文契约。

## 10. 优化与架构升级触发

按以下顺序路由，不因单个 microbenchmark 的改善跳级：

1. exact-only 已满足第 6–7 节：不升级架构。
2. W1 超预算，且单线程 component 占 whole-step/CPU `≥ 10%`，预计可以带来
   `≥ 5%` integrated W1 改善：优先 data、layout、algorithm optimization。
3. component `< 5%` whole-step：默认不作为首要目标，除非出现 superlinear
   scaling、tail 或 correctness risk。
4. exact-only 仍超预算，multi-rate candidate 通过 fidelity，且 integrated W1
   whole-step gain `≥ 5%`：新建 production multi-rate G1。
5. #216/#217 完成后，100k Core p95 仍超过 16 ms 且超幅大于 10%，或同步 tick
   frame 无法满足 60 Hz host：进入 #220 parallel phase/partition G1。
6. identity-preserving single-thread、multi-rate、parallel 路径完成后，仍超过
   P100 CPU/memory budget `≥ 25%`，或未来正式提出 1M realtime 产品要求：才允许
   aggregate/exact migration 独立 G1/ADR。
7. hard invariant 或 failed-step atomicity 失败、只有 kernel 收益而 W1 integrated
   无收益，或收益依赖放宽 fidelity/safety、隐藏 catch-up、不可接受 retained
   memory：candidate 不进入 production。

### 10.1 当前工作路由

|     顺序 | Issue / component                        | 当前依据与边界                                                                  |
| -------: | ---------------------------------------- | ------------------------------------------------------------------------------- |
|        1 | #216 occupancy/leader exact path         | 约 35% whole-step；优先数据访问、route-distance index 与局部性                  |
|        1 | #217 longitudinal proposal/data locality | proposal/store 约 51% whole-step；优先 safe-motion、store、lookup 与 batch/SIMD |
|        2 | #218 C4 cache/journal                    | 有稳定收益，但 whole-step 尚未达到 5% trigger                                   |
|        3 | #219 advance/events/commit               | 约 8.5%，在前两项之后                                                           |
| 暂不升级 | global projection                        | 约 4.9%；除非复杂度、tail 或 correctness 风险增长                               |
| 等待输入 | #220 parallel phase graph                | 等待 #215/#216/#217；不从 #212 直接跳入并行实现                                 |

当前 production multi-rate 与 aggregate 均未触发。

## 11. TBD 台账

TBD 是显式停止条件，不是可以用开发机推测值填补的空白。每项必须记录未决原因、
禁止声明、解除触发和 owner。

| TBD                                                          | 未决原因                             | 当前禁止的声明                                       | 解除触发                                         | Owner / 后续承载                      |
| ------------------------------------------------------------ | ------------------------------------ | ---------------------------------------------------- | ------------------------------------------------ | ------------------------------------- |
| P10 具体设备/SKU、release OS、内存与数值内存上限             | 尚未选定最低产品设备                 | 10k Product Pass / SLA                               | 设备确定并完成第 4–8 节 integrated certification | `wangzishi`；后续 certification Issue |
| P100 具体设备/SKU、release OS、内存与数值内存上限            | 尚未选定 scale reference             | 100k Product Pass / SLA                              | 设备确定并完成第 4–8 节 integrated certification | `wangzishi`；后续 certification Issue |
| Presentation interpolation/extrapolation 与 visual tolerance | 当前只冻结 committed sample exact    | 视觉平滑度 SLA、插值误差承诺                         | 独立 G1 冻结算法、authority 与容差               | `wangzishi`；独立 design Issue        |
| Aggregate model 与非守恒数值 tolerance                       | Aggregate 尚未触发，也未选择模型     | aggregate fidelity、1M realtime 或无损 identity 声明 | 第 10 节 trigger 满足并完成独立 G1/ADR           | `wangzishi`；未来 aggregate Issue     |
| Linux/macOS/Web/mobile 平台基线                              | 当前只有 Windows x86-64 R0           | 对这些平台外推 10k/100k SLA                          | 每个平台分别确定硬件/runtime 并运行完整适用协议  | `wangzishi`；平台专用 Issue           |
| 真实路网来源、许可与转换制品                                 | 当前 canonical workload 是 synthetic | real-road representativeness、真实城市 workload SLA  | #224 完成来源选择、转换契约与可复现 artifact     | `wangzishi`；#224                     |

后续 Issue 可以接管某个 TBD，但在长期文档更新前，原 claim restriction 继续有效。

## 12. 下游依赖与事实源

- `core-runtime-scalability-audit.md` 保存 #199、#204、#207、#210、#212 的历史
  研究证据、no-regret constraints 与架构候选；本文保存当前产品 target/measurement
  contract。历史数字不能替代本文要求的 integrated certification。
- #216/#217 是独立 research slice，不新增对 #215 的 native blocker。它们可以
  使用本文 workload、budget 与 classification 作为研究输入，但不能自行宣称产品
  certification。
- #220 继续等待 #215、#216、#217。#215 完成只释放其中一个 blocker，不自动
  允许 #220 开工。
- #220 必须继续保持 strong-individual identity、route/horizon-driven read-only
  halo，以及把跨区 leader chain/cycle 合并为 logical dependency graph 后全局求解
  的约束；不得用固定宽度 halo 或 partition-local projection 改写 safety semantics。
- #218/#219 可以引用本文，但不因本文改变依赖关系。
- 第 8 节完整产品矩阵属于后续 optimization/certification 工作；历史 #212 数据只
  是 research evidence。
- #224 独立承载真实路网来源、许可、裁剪与 Traffic/Spatial 转换；不阻塞
  `LF-SYNTH-v1` 的研究与产品 Gate 准备，也不允许把 synthetic 结果表述为真实路网
  代表性证据。未来 real-road profile 必须使用独立 workload ID，不能静默替换
  canonical synthetic baseline。

## 13. API、兼容性与 ADR

本文不改变：

- Core API；
- Data format；
- Spatial API；
- Adapter API；
- production runtime behavior；
- crate 依赖方向。

本文不新增 ADR，因为没有选择 production partition、scheduler、multi-rate、
aggregate、public World/shard 或 snapshot contract。

以下任一项进入 production 设计时必须重新判断 ADR，并在实现前完成独立 G1：

- public fixed-step 或 overload/catch-up 行为；
- scheduler、worker ownership、parallel phase graph 或 deterministic merge；
- public snapshot/accessor、batch command 或 retained runtime snapshot；
- handle provenance、跨 partition migration、multi-World/shard；
- interpolation/extrapolation authority；
- aggregate/exact migration、identity translation 或分布式 authority。

若实现或实验要求修改上述边界，不得通过放宽本文 target 或 fidelity 隐式吸收，应
暂停当前切片并建立新的设计输入。
