# Numeric Representation and Precision

**文档状态**: Draft

**最后更新**: 2026-07-18

**适用范围**: v0.6 Numeric & Spatial Foundation 的 Core 数值表示、精度分层和跨层转换边界（#122）

**关联文档**:

- `../architecture.md`
- `../roadmap.md`
- `../adr/0003-runtime-tick-and-determinism.md`
- `../adr/0004-core-implementation-language.md`
- `core-runtime.md`
- `data-format.md`
- `vehicle-following.md`
- `parking-system.md`
- `../reference/v0.6-numeric-validation.md`

## 1. 状态与目标

本文是 #122 的 G1 研究输入，不是已接受的生产契约。G1 通过前：

- 当前 `f64` 实现继续是唯一 production 行为；
- 不授权修改公共 Core newtype、current data schema 或 Adapter/Spatial API；
- `f32`、mixed precision、`f16` 与量化整数都只是待验证候选；
- 不承诺跨 CPU、跨语言或跨编译器的 bit-level floating-point determinism。

G1 必须冻结以下问题：

1. authoritative runtime hot state 和公共领域类型使用什么标量；
2. 哪些累计量必须保持整数、局部参数、补偿累计或按权威状态重算；
3. 每个领域的范围、绝对/相对误差预算与 epsilon 语义；
4. Core、Data、Spatial、Adapter 和离线 authoring/reference 的转换位置；
5. `NaN` / Infinity、signed zero、边界 snap、失败原子性和确定性规则；
6. API、data format 与 pre-1.0 migration 的影响。

## 2. 当前实现基线

### 2.1 权威类型与存储

当前实现不是单一字段上的 `f64` 选择，而是贯穿多个边界：

| 数值域 | 当前表示 | 主要位置 | 当前语义 |
| --- | --- | --- | --- |
| edge length | `EdgeLength(f64)` | `graph.rs` | finite，且严格大于 edge-boundary epsilon |
| edge-local progress | `EdgeProgress(f64)` | `vehicle.rs` | finite、非负、edge-local；每 tick 推进 |
| speed / acceleration | `Speed(f64)` / `Acceleration(f64)` | `vehicle.rs` | finite；speed 非负，acceleration 可正负 |
| Vehicle Profile | raw `f64` fields | `profile.rs` | length、gap、headway、acceleration/deceleration 参数 |
| occupancy / leader | raw `f64` fields | `occupancy.rs` | front progress、vehicle length、bumper gap |
| command spatial scratch | `Vec<f64>` 与 raw `f64` | `command_spatial.rs` | edge length、front progress、reverse distance 与速度上界 |
| route distance index | `Vec<f64>` 与 checked finite state | `route_distance.rs` | segment totals、offset 与 horizon 查询 |
| static Parking geometry | raw `f64` 经 Core domain validation | `parking.rs`、`wire.rs` | anchor progress、lateral/heading/length/width |
| external JSON DTO | raw `f64` | `laneflow-data/src/wire.rs` | JSON number 到 Core domain constructor 的输入 |
| tick / simulation time | checked `u64` | `world.rs` | fixed delta、tick index、absolute milliseconds |

整数 tick/time 已避免长期 wall-clock 浮点累计，但物理推进仍把 fixed milliseconds 转为浮点秒，并在 edge-local progress、route-distance index 和控制器计算中使用 `f64`。

### 2.2 Epsilon 基线

生产代码当前公开两个数值相同、领域语义不同的常量：

- `EDGE_BOUNDARY_EPSILON = 1.0e-9`：edge 最小长度、progress boundary、remainder 与 traversal snap；
- `GEOMETRY_GAP_EPSILON = 1.0e-9`：vehicle geometry、bumper gap、no-overlap 和 Parking geometry。

测试还存在本地 `EPSILON = 1.0e-9`。G1 不应因为当前数值相同就把这些语义重新合并为一个全局 epsilon；必须分别从领域范围、单位和错误预算推导。

### 2.3 风险集中区

精度候选必须重点验证：

- edge boundary 前后的 snap、remainder 和单 tick 多 edge traversal；
- bumper gap 的符号、leader ordering、IIDM/safe-speed 与 no-overlap projection；
- SignalStop、ParkingStop、RouteEnd 约束组合的先后顺序；
- Parking entry/exit anchor 与 geometry validation；
- route-distance segment total、horizon 与超大值降级路径；
- command validation、reverse distance 和失败原子性；
- public error payload、events、snapshots 与 JSON normalization 的可观察值。

### 2.4 当前兼容范围不是产品范围

当前 constructor 与 current schema 对 distance、speed、acceleration、progress 和 geometry 基本只要求 finite/正负号约束，没有产品级上界；Core 还保留 `f64::MAX`、巨大/微小值和累计 overflow 的防御测试。因此当前兼容面实际接近“全部有限 `f64`”，不能把标量替换为 `f32` 解释为无兼容影响的内部优化。

G1 必须分开记录三类范围：

1. **current compatibility range**：当前 API、schema 和错误语义实际接受或诊断的范围；
2. **product validation envelope**：园区、厂区、校园、景区、停车场和局部道路片区中要求正式保证的范围；
3. **scale/pathological fixtures**：为测车辆数量、overflow 和失败路径而构造的合成极值，不自动等同于产品道路长度。

当前 10k/100k workload 中存在把全部车辆铺在单条超长 edge 上的 scale fixture。100k free-flow 场景的 edge 可达到约 `25_001_000 m`，该数量级的 `f32` ULP 为 `2 m`，已经大于常见 fixed tick 的单步位移。它应继续作为 current-range compatibility probe，但不能单独代表 local-coordinate 产品 workload；性能比较还需要等车辆数、等密度、但保持 edge/local coordinate 在验证 envelope 内的 companion fixture。

## 3. 数值域与误差预算

G1 的误差预算必须以 SI 单位和可观察行为表达，不能只写机器 epsilon。每个数值域至少记录：

| 数值域 | 必须确定的范围 | 必须确定的误差指标 | 不可破坏的不变量 |
| --- | --- | --- | --- |
| edge length | 最小有效 edge、代表性最大单 edge | absolute length error、boundary ULP | positive、finite、length authority 一致 |
| edge progress | `[0, edge_length]` 与 boundary 邻域 | per-step drift、boundary snap error | 不倒退、不越界、不漏/重复 transition |
| bumper gap | overlap 邻域到 leader horizon | zero-sign/absolute gap error | no-overlap、leader order、projection safety |
| speed | 静止、城市道路、异常高值 | absolute/relative speed error | finite、非负、停止/恢复语义一致 |
| acceleration | comfort/emergency 范围 | control output 与 braking-distance error | finite、deceleration ordering、safe-speed |
| Parking geometry | anchor、lateral offset、length/width | bind/pose 输入误差 | anchor inside edge、geometry non-degenerate |
| local coordinate | v0.6 spatial tile/local frame 范围 | position/tangent sampling error | 不把大世界绝对坐标误差泄漏进 Core hot state |
| heading | `[-PI, PI)` | angular error | canonical range、方向 continuity |

预算必须同时覆盖正常范围和 boundary/pathological fixtures。接受标准由领域行为决定：例如 boundary transition 和 no-overlap 必须先保持离散结果一致，再比较连续数值误差。

### 3.1 Draft product validation envelope

#122 首轮证据使用以下保守 envelope；它是候选实现的验证输入，不是已接受的 schema/API hard limit：

| 数值域 | Draft envelope | `f32` 在最大绝对值处的 ULP | 说明 |
| --- | ---: | ---: | --- |
| edge length / edge progress | `0..=16_384 m`，有效 edge fixture 从 `0.1 m` 起 | `0.001953125 m` | 单 edge/local frame 上界；更长道路应通过多个 edge 表达 |
| route-local gap / query horizon | `-128..=8_192 m` | `0.0009765625 m` | 负值仅用于 overlap/error seeds；累计 route total 不受此项授权降精度 |
| speed | `0..=100 m/s` | `0.00000762939453125 m/s` | 约 `360 km/h`，高于当前局部 NPC 交通目标 |
| acceleration/deceleration | `-50..=50 m/s^2` | `0.000003814697265625 m/s^2` | 覆盖 comfort、emergency 与异常高值 fixture |
| vehicle/Parking extent、offset、min gap | offset `-128..=128 m`；extent `0.1..=128 m`；min gap `0..=128 m` | `0.0000152587890625 m` | 只有 offset 可有符号；length/width 必须为正 |
| time headway | `0..=60 s` | `0.000003814697265625 s` | 零值只用于 rejection seed |
| local coordinate | `-16_384..=16_384 m` | `0.001953125 m` | world/georeference 不进入 local hot state |
| heading | `[-PI, PI)` | `0.0000002384185791015625 rad` | 使用 `PI` 附近 ULP |

选择 `16_384 m` 是为了显式验证一个远大于典型单条局部道路的 power-of-two 上界，同时让最坏 `f32` 表示步长仍低于 `2 mm`。若 #123 需要更大的单一 local frame，必须重新计算预算，不能只扩大常量。

### 3.2 Draft continuous error ceiling

候选实现必须先保持 status、route occurrence、edge transition、leader、constraint、projection 和 event kind/order **精确一致**。在此基础上，首轮连续值 ceiling 为：

- edge length、progress、bumper gap、route-local distance、Parking geometry 与 local position：每 tick/每次采样最大绝对差 `0.01 m`；
- speed：最大绝对差 `0.01 m/s`；
- acceleration：最大绝对差 `0.02 m/s^2`；
- heading：最大绝对差 `0.0001 rad`；
- 非零常规值还必须满足 validation 文档中的相对误差阈值；接近零和 boundary 只用绝对误差与离散行为判断。

`1 cm` 位置 ceiling 小于当前 benchmark `4.5 m` 车辆长度的 `0.23%`、小于 `2 m` min gap 的 `0.5%`；`0.01 m/s` 等于 `0.036 km/h`。这些是研究接受上限，不是允许车辆发生 `1 cm` overlap 的安全豁免；no-overlap projection 的提交状态仍必须非负并保持离散结果一致。

### 3.3 累计误差单独判定

`f32` ULP 足够小，不代表逐 tick `progress += travel` 足够稳定。首轮 constant-input probe 中，`10 m/s`、`16 ms`、`6_250` ticks 的数学位移为 `1_000 m`，raw `f32` 重复累加得到约 `999.9282837 m`，误差约 `-7.17 cm`；带独立补偿量的 `f32` 累加约为 `1000.0000610 m`。

因此 G1 必须独立选择：

- authority/storage scalar；
- integration intermediate；
- residual/compensation 或从局部权威量重算策略；
- edge transition 时 residual 的消费、转移与清零规则。

若 hot state 改为 `f32` 却需要为每辆车增加一个 `f32` residual，必须把该字段计入内存比较；不能只按 `8 byte -> 4 byte` 宣称 progress 节省一半。

## 4. 候选精度分层

以下是需要通过证据比较的候选，不是当前决策：

### A. 全部 authoritative `f64`

- 保持现有 API 和算法；
- 作为性能、内存和正确性基线；
- 仍需拆分领域 epsilon，并限制无意义的极端范围。

### B. runtime/local authoritative `f32`

- public/hot state、local progress 和 local spatial coordinate 使用 `f32`；
- tick/time/index 保持整数；
- 离线 authoring、导入校验和 reference oracle 可以保留 `f64`；
- 不允许 raw `f32` repeated addition 在未通过长时段 drift 与 boundary oracle 前成为 progress authority；
- 只有在代表性 runtime、差分测试和 migration 边界通过后才可接受。

### C. mixed precision

- hot/local state 使用 `f32`，route total、离线 authoring 或需要扩大动态范围的中间计算使用 `f64`；
- `EdgeProgress`、route-distance total 和 integration residual 可以分别选择精度，不能因同属“distance”被强制统一；
- 每个转换必须由 LaneFlow-owned API 显式执行并返回可诊断错误；
- 不允许无文档的隐式 cast 或因宿主引擎类型反向决定 Core authority。

### D. `f16` / 量化整数

- 仅评估 storage、transport、远景 presentation 或可丢弃缓存；
- 在 G1 证据证明前，禁止用于 edge progress、speed、acceleration、gap、constraint 或 geometry authority；
- 必须分别报告 encode/decode、range saturation 和量化误差。

## 5. 确定性与错误语义

精度变更不得降低以下现有保证：

- 相同 Core 版本、运行环境、初始状态和输入序列产生一致结果；
- `NaN` / Infinity 在进入权威状态前被拒绝；
- signed zero 按领域 constructor canonicalize；
- tick/time 使用 checked integer arithmetic；
- validation/step/command 失败不部分修改 world；
- vehicle/event/update order 不依赖 hash iteration 或浮点排序不稳定性；
- boundary、constraint 和 projection 的离散决策有明确且领域化的 tolerance。

跨平台 bit-level determinism 仍不属于 v0.6 承诺。若候选精度改变事件、edge transition、leader 或 constraint 决策，即使最终位置误差很小，也视为行为差异，必须单独解释或阻断。

当前两个 `1.0e-9` 常量不得直接 cast 为 `f32` 后继续复用。G1 至少要拆分：

- edge/geometry 的**最小有效尺寸**，属于输入语义，不是比较 epsilon；
- edge boundary snap tolerance；
- gap/overlap zero tolerance；
- heading canonicalization tolerance。

候选 tolerance 必须由 validation envelope 的最大 ULP、涉及的运算次数与上述连续误差 ceiling 共同约束。首轮以 `4 * max ULP` 作为 add/subtract/snap 的保守 representation floor：在 `16_384 m` 处约为 `0.0078125 m`，仍低于 `0.01 m` ceiling。最终常量必须由 oracle/boundary tests 冻结，不能把这个 research factor 当作已接受生产值。

## 6. 跨层转换边界

G1 至少需要冻结以下所有权：

- **Core**：authoritative traffic state、领域 constructor、runtime validation 与 step 行为；
- **Data**：JSON number shape、单位、范围诊断和到 Core domain type 的显式 normalization；
- **Spatial**：local/canonical coordinate、geometry sampling 和 Core progress 到 pose 的转换；
- **Adapter**：LaneFlow-owned pose 到宿主 `Transform` 的末端转换；
- **Authoring/reference**：允许高精度输入和 oracle，但不得绕过 Core/Data 的 current contract。

Bevy/glam、Unity、Unreal、Godot、Three.js 或 Babylon.js 的向量/Transform 类型不得成为 Core/Spatial 公共类型。若 Data 的 JSON shape 不变但 Rust public scalar 或 validation range 改变，仍须按 ADR 0008 明确 current-version 与迁移判断，不能把它当作无兼容影响的内部重构。

## 7. G1 所需证据

G1 Pass 前必须具备：

1. 可复现的 Core/Data 数值面 inventory；
2. 每个领域的范围、绝对/相对误差预算和 epsilon 推导；
3. 同一代表性 10k/100k workload 的 `f64` 与候选实现时间/内存对比；
4. 逐 tick `f64` oracle 差分与 property/boundary 测试；
5. `f16`/量化格式的 range、误差和禁止边界；
6. Core API、current data format、Spatial API、Adapter API 和 migration 影响；
7. ADR 0012 的明确决策、备选方案、后果与实施拆分。

详细测量方法和结果记录在 [`v0.6-numeric-validation.md`](../reference/v0.6-numeric-validation.md)。
