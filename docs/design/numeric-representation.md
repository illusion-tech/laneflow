# Numeric Representation and Precision

**文档状态**: Accepted

**最后更新**: 2026-07-18

**适用范围**: v0.6 Numeric & Spatial Foundation 的 Core 数值表示、精度分层和跨层转换边界（#122）

**关联文档**:

- `../architecture.md`
- `../roadmap.md`
- `../adr/0003-runtime-tick-and-determinism.md`
- `../adr/0004-core-implementation-language.md`
- `../adr/0012-core-numeric-authority-and-presentation-precision.md`
- `core-runtime.md`
- `data-format.md`
- `vehicle-following.md`
- `parking-system.md`
- `../reference/v0.6-numeric-validation.md`

## 1. 状态与目标

本文与 ADR 0012 冻结 #122 的 G1 数值契约：

- production Core/Data 的连续 authority、累计量和行为相关中间量保持 `f64`；tick/time 保持 checked integer；
- Spatial canonical geometry/pose 使用 LaneFlow-owned `f64`，具体 frame、geometry 和 sampling 由 #123 完成；
- `f32` 只在 canonical pose 减去 local origin 后，通过 checked、LaneFlow-owned presentation conversion 产生；
- `f16`/量化整数不承担 Core/Spatial authority，只能在未来按用途进入可丢弃或 versioned storage/transport；
- current v0.5 schema/API 不因 #122 迁移；领域 epsilon 的语义拆分由后续实施 Issue 交付；
- 不承诺跨 CPU、跨语言或跨编译器的 bit-level floating-point determinism。

本设计固定数值职责，不固定 #123 的 geometry container、空间索引、origin 生命周期或 v0.7 的具体 Adapter batch 类型。

## 2. 当前实现基线

### 2.1 权威类型与存储

当前实现不是单一字段上的 `f64` 选择，而是贯穿多个边界：

| 数值域                  | 当前表示                            | 主要位置                    | 当前语义                                                 |
| ----------------------- | ----------------------------------- | --------------------------- | -------------------------------------------------------- |
| edge length             | `EdgeLength(f64)`                   | `graph.rs`                  | finite，且严格大于 edge-boundary epsilon                 |
| edge-local progress     | `EdgeProgress(f64)`                 | `vehicle.rs`                | finite、非负、edge-local；每 tick 推进                   |
| speed / acceleration    | `Speed(f64)` / `Acceleration(f64)`  | `vehicle.rs`                | finite；speed 非负，acceleration 可正负                  |
| Vehicle Profile         | raw `f64` fields                    | `profile.rs`                | length、gap、headway、acceleration/deceleration 参数     |
| occupancy / leader      | raw `f64` fields                    | `occupancy.rs`              | front progress、vehicle length、bumper gap               |
| command spatial scratch | `Vec<f64>` 与 raw `f64`             | `command_spatial.rs`        | edge length、front progress、reverse distance 与速度上界 |
| route distance index    | `Vec<f64>` 与 checked finite state  | `route_distance.rs`         | segment totals、offset 与 horizon 查询                   |
| static Parking geometry | raw `f64` 经 Core domain validation | `parking.rs`、`wire.rs`     | anchor progress、lateral/heading/length/width            |
| external JSON DTO       | raw `f64`                           | `laneflow-data/src/wire.rs` | JSON number 到 Core domain constructor 的输入            |
| tick / simulation time  | checked `u64`                       | `world.rs`                  | fixed delta、tick index、absolute milliseconds           |

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

| 数值域           | 必须确定的范围                       | 必须确定的误差指标                       | 不可破坏的不变量                            |
| ---------------- | ------------------------------------ | ---------------------------------------- | ------------------------------------------- |
| edge length      | 最小有效 edge、代表性最大单 edge     | absolute length error、boundary ULP      | positive、finite、length authority 一致     |
| edge progress    | `[0, edge_length]` 与 boundary 邻域  | per-step drift、boundary snap error      | 不倒退、不越界、不漏/重复 transition        |
| bumper gap       | overlap 邻域到 leader horizon        | zero-sign/absolute gap error             | no-overlap、leader order、projection safety |
| speed            | 静止、城市道路、异常高值             | absolute/relative speed error            | finite、非负、停止/恢复语义一致             |
| acceleration     | comfort/emergency 范围               | control output 与 braking-distance error | finite、deceleration ordering、safe-speed   |
| Parking geometry | anchor、lateral offset、length/width | bind/pose 输入误差                       | anchor inside edge、geometry non-degenerate |
| local coordinate | v0.6 spatial tile/local frame 范围   | position/tangent sampling error          | 不把大世界绝对坐标误差泄漏进 Core hot state |
| heading          | `[-PI, PI)`                          | angular error                            | canonical range、方向 continuity            |

预算必须同时覆盖正常范围和 boundary/pathological fixtures。接受标准由领域行为决定：例如 boundary transition 和 no-overlap 必须先保持离散结果一致，再比较连续数值误差。

### 3.1 Product validation envelope

#122 首轮证据使用以下保守 envelope；它是候选实现的验证输入，不是已接受的 schema/API hard limit：

| 数值域                                  |                                           G1 validation envelope |     `f32` 在最大绝对值处的 ULP | 说明                                                                |
| --------------------------------------- | ---------------------------------------------------------------: | -----------------------------: | ------------------------------------------------------------------- |
| edge length / edge progress             |                  `0..=16_384 m`，有效 edge fixture 从 `0.1 m` 起 |                `0.001953125 m` | 单 edge/local frame 上界；更长道路应通过多个 edge 表达              |
| route-local gap / query horizon         |                                                 `-128..=8_192 m` |               `0.0009765625 m` | 负值仅用于 overlap/error seeds；累计 route total 不受此项授权降精度 |
| speed                                   |                                                    `0..=100 m/s` |      `0.00000762939453125 m/s` | 约 `360 km/h`，高于当前局部 NPC 交通目标                            |
| acceleration/deceleration               |                                                 `-50..=50 m/s^2` |   `0.000003814697265625 m/s^2` | 覆盖 comfort、emergency 与异常高值 fixture                          |
| vehicle/Parking extent、offset、min gap | offset `-128..=128 m`；extent `0.1..=128 m`；min gap `0..=128 m` |         `0.0000152587890625 m` | 只有 offset 可有符号；length/width 必须为正                         |
| time headway                            |                                                       `0..=60 s` |       `0.000003814697265625 s` | 零值只用于 rejection seed                                           |
| local coordinate                        |                                             `-16_384..=16_384 m` |                `0.001953125 m` | world/georeference 不进入 local hot state                           |
| heading                                 |                                                      `[-PI, PI)` | `0.0000002384185791015625 rad` | 使用 `PI` 附近 ULP                                                  |

选择 `16_384 m` 是为了显式验证一个远大于典型单条局部道路的 power-of-two 上界，同时让最坏 `f32` 表示步长仍低于 `2 mm`。若 #123 需要更大的单一 local frame，必须重新计算预算，不能只扩大常量。

### 3.2 Continuous error ceiling

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

## 4. 接受的精度分层

| Layer / numeric role                                | Accepted representation                      | Authority                        | 规则                                                |
| --------------------------------------------------- | -------------------------------------------- | -------------------------------- | --------------------------------------------------- |
| fixed delta、tick、absolute time、counts            | checked integer (`u64` 等领域整数)           | Core                             | 禁止浮点 wall-clock 累计                            |
| public Core domain、vehicle state、profile、Parking | `f64`                                        | Core                             | current constructors/API 不迁移                     |
| longitudinal/occupancy/constraint/command scratch   | `f64`                                        | Core derived state               | 会影响离散行为，跟随 authority 精度                 |
| edge/route distance totals 与 integration           | `f64`                                        | Core                             | 不使用 raw f32 repeated addition 或 hidden residual |
| Data JSON number normalization                      | raw JSON number -> checked Core `f64` domain | Data/Core                        | current v0.5 shape/range/diagnostics 不变           |
| canonical geometry、arc length、canonical pose      | LaneFlow-owned `f64`                         | Spatial                          | #123 冻结 container、length consistency 和 sampling |
| local presentation pose                             | checked `f32` 或保留 `f64`                   | derived snapshot，不是 authority | 先在 f64 减 local origin，再转换                    |
| host vector/Transform                               | host-specific                                | Adapter presentation             | 只能出现在 Adapter 末端                             |
| `f16` / quantized integer                           | explicit encoding/cache only                 | 非 authority                     | 独立 scale/origin/range/overflow/error contract     |

研究结果解释：

- raw f32 的 36,000 tick progress drift 和 dense/stop-and-go 控制误差超过 budget；
- #122 的旧 compensated f32 只在写入时维护 residual，读取 leader gap、edge remaining、boundary 和 snapshot 时仍只使用 high component；它没有实现完整的 compensated progress 语义；
- #140 的 residual-aware f32 补全读取语义后，通过 5 种 edge 布局、3 种 10k 场景和 100k dense observation 的原严格 ceiling，且无离散分歧；
- `f32 compute + f64 progress` 不再是唯一通过候选。#140 同轮研究模型中，residual-aware f32 相对 f64 在 10 km cap 的 10k 场景快约 2.4%–5.8%，100k dense 快约 7.1%；该结果不足以直接外推 production；
- candidate layout 可从 `128 B/vehicle` 缩到 `80–88 B/vehicle`，但这是 research-only vehicle/motion vectors，不是完整 Core 的已实现总内存收益；
- 因此 current production 仍使用表中的 f64 authority，但理由改为“完整 production residual-aware candidate、跨组件 oracle 与 API/Data 迁移尚未完成”。ADR 0012 的旧唯一候选/无性能收益前提已失效，后续由 #141 和 superseding ADR 复核。

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

当前两个 `1.0e-9` 常量不得在 future local-f32 path 中直接 cast 后继续复用。后续实施至少要拆分：

- edge/geometry 的**最小有效尺寸**，属于输入语义，不是比较 epsilon；
- edge boundary snap tolerance；
- gap/overlap zero tolerance；
- heading canonicalization tolerance。

future tolerance 必须由 validation envelope 的最大 ULP、涉及的运算次数与上述连续误差 ceiling 共同约束。首轮以 `4 * max ULP` 作为 add/subtract/snap 的保守 representation floor：在 `16_384 m` 处约为 `0.0078125 m`，仍低于 `0.01 m` ceiling。它不是已接受的生产常量。current production authority 保持 f64/current behavior；实现 Issue 必须通过 oracle/boundary tests 冻结各领域常量，不能把 research factor 直接复制进 production。

## 6. 跨层转换边界

### 6.1 Host 事实不能反向决定 Core

各宿主当前公开 API 的选择并不统一：

| Host       | Transform/vector scalar                                  | #122 解释                         |
| ---------- | -------------------------------------------------------- | --------------------------------- |
| Bevy       | `Transform.translation: Vec3`，`from_xyz(f32, f32, f32)` | 需要 local f32 presentation path  |
| Unity      | `Vector3(float x, float y, float z)`                     | 需要 local f32 presentation path  |
| Unreal     | `FVector` 为 `TVector<double>`                           | 可保留 f64 Adapter path           |
| Godot      | 默认 32-bit，支持 double-precision build                 | Adapter 按 build/ABI 选择末端转换 |
| Three.js   | JavaScript `number`                                      | host 数值语义不是 Rust f32 ABI    |
| Babylon.js | JavaScript `number`                                      | host 数值语义不是 Rust f32 ABI    |

资料： [Bevy Transform](https://docs.rs/bevy_transform/latest/bevy_transform/components/struct.Transform.html)、[Unity Vector3 constructor](https://docs.unity3d.com/ja/current/ScriptReference/Vector3-ctor.html)、[Unreal Core math types](https://dev.epicgames.com/documentation/en-us/unreal-engine/API/Runtime/Core)、[Godot Vector3](https://docs.godotengine.org/en/stable/classes/class_vector3.html)、[Three.js Vector3](https://threejs.org/docs/pages/Vector3.html)、[Babylon.js Vector3](https://doc.babylonjs.com/typedoc/classes/BABYLON.Vector3)。

### 6.2 Conversion ordering 与失败语义

唯一允许生成 local f32 pose 的顺序是：

```text
Core progress f64
  -> Spatial canonical sample f64
  -> canonical_f64 - local_origin_f64
  -> finite/range/basis validation + signed-zero canonicalization
  -> local f32 pose batch
  -> host Transform
```

- 禁止 `(canonical as f32) - (origin as f32)`；必须先用 f64 消去大 offset。
- conversion API 必须由 LaneFlow 拥有，返回结构化 domain/range error，并对整个 batch compute-then-commit；失败不能留下部分输出或修改 authority。
- f32 batch 必须携带或绑定明确 origin/frame identity，不能被另一个 origin 的 Adapter state 误用。
- double host 可以消费 canonical/local f64，不需要人为降为 f32；这不形成第二套 Core state。
- presentation interpolation/rebase/LOD 只影响显示，不能反馈写入 progress、speed、occupancy、status 或 events。

### 6.3 API/Data/Spatial/Adapter migration 判断

| Surface                     | #122 G1 decision                                                   | Migration                                |
| --------------------------- | ------------------------------------------------------------------ | ---------------------------------------- |
| Core API                    | current `f64` newtypes/state/error 保持                            | none                                     |
| current Data v0.5           | JSON shape、loader range/diagnostics 保持                          | none；不 bump format                     |
| Spatial API                 | future canonical f64 + checked local presentation contract         | 由 #123 首次设计，不是既有 API migration |
| Adapter API                 | LaneFlow pose 到 host Transform 的末端映射；允许 f64/f32 host path | 由 v0.7 首次设计，不是既有 API migration |
| quantized storage/transport | current 不存在                                                     | 将来必须 versioned、独立 Issue/迁移      |

若将来修改 Core scalar、current accepted range、rounding/saturation 或 wire encoding，即使 JSON 仍是 `number`，也必须按 ADR 0008 视为 data/API migration。

## 7. G1 证据与后续实施

#122 G1 已具备：

1. 可复现的 Core/Data 数值面 inventory；
2. 每个领域的范围、绝对/相对误差预算和 epsilon 推导；
3. 同一代表性 10k/100k workload 的 f64/raw-f32/旧 compensated/residual-aware/mixed 时间与 candidate memory；
4. production-aligned f64 oracle、逐 tick differential、长时累计、same-runtime replay、non-finite/signed-zero/boundary 证据；
5. f16 与整数定点的 range/error/禁止边界；
6. Core/Data/Spatial/Adapter migration 判断；
7. ADR 0012 的决策、备选方案、后果与实施边界。

#140 还补充了 edge cap 的 production Core steady/construction/transition 压力矩阵、每个最大误差的 tick/vehicle/control provenance，以及 `10_000 m` 防御性单 edge 上界候选。完整数字与限制记录在 [`v0.6-numeric-validation.md`](../reference/v0.6-numeric-validation.md) 第 9 节。

详细命令、CI-independent local baseline、完整数字与限制记录在 [`v0.6-numeric-validation.md`](../reference/v0.6-numeric-validation.md)。G1 后仍需独立交付：领域 epsilon/validation 拆分、Data/API migration audit、长期 numeric regression/performance 和 closure review；#123/v0.7 分别交付 Spatial 与真实 Adapter conversion。
