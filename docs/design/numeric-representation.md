# 数值表示与精度

**文档状态**: 已接受（Accepted）

**最后更新**: 2026-07-18（#141 ADR 0014 同步）

**适用范围**: v0.6 数值与空间基础（Numeric & Spatial Foundation）的 Core 数值表示、精度分层和跨层转换边界（#122、#140、#141）

**关联文档**:

- `../architecture.md`
- `../roadmap.md`
- `../adr/0003-runtime-tick-and-determinism.md`
- `../adr/0004-core-implementation-language.md`
- `../adr/0012-core-numeric-authority-and-presentation-precision.md`
- `../adr/0014-residual-aware-f32-core-authority-and-migration-gates.md`
- `core-runtime.md`
- `data-format.md`
- `vehicle-following.md`
- `parking-system.md`
- `../reference/v0.6-numeric-validation.md`

## 1. 状态与目标

本文同时记录当前实现与 ADR 0014 接受的下一目标契约，禁止把尚未完成的迁移写成当前事实：

- 当前生产 Core/Data v0.5 继续使用现有 `f64` 权威和兼容范围，直到独立原子迁移通过全部闸口；
- 下一契约中，`EdgeLength` 和单值热状态使用经过检查的 `f32`，`EdgeProgress` 使用封装的高位分量/残差，固定 tick/时间继续使用经过检查的整数；
- route 距离冻结派生权威、误差、复杂度和溢出语义，由完整证据选择 `f64` 前缀基线或分块局部 `f32` 布局；
- Spatial 标准几何/位姿继续使用 LaneFlow 自有的 `f64`，局部表现 `f32` 只在标准位姿减去局部原点后通过受检转换产生；
- `f16`/量化整数不承担 Core/Spatial 权威，只能按用途进入可丢弃或带版本的存储/传输；
- 下一契约会改变 Core API、接受范围、Data 版本、舍入和 Spatial 长度绑定，因此不属于纯内部优化；
- 不承诺跨 CPU、跨语言或跨编译器的位级浮点确定性。

本设计固定数值职责和迁移闸口，不固定 Spatial 几何容器、空间索引、原点生命周期或 v0.7 的具体 Adapter 批量类型。

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

预算必须同时覆盖正常范围、边界样例和病理样例。接受标准由领域行为决定：例如边界跨越和无重叠必须先保持离散结果一致，再比较连续数值误差。

### 3.1 下一契约的硬性产品范围

#122 的 `16_384 m` edge 范围是历史研究输入，不是模式/API 的硬上限。#140 的 edge 上界证据与 #141 G1 已把以下范围冻结为下一数值契约；当前 v0.5 在原子迁移前仍保持现有兼容行为：

| 数值域                            |                                                下一契约硬范围 |     `f32` 在最大绝对值处的 ULP | 说明                                     |
| --------------------------------- | ------------------------------------------------------------: | -----------------------------: | ---------------------------------------- |
| 固定 tick                         |                                                `1..=1_000 ms` |                            N/A | 整数权威；更长的追赶拆为多个 tick        |
| edge 长度 / edge 进度             |        `min_edge_length < length <= 10_000 m`；进度在 edge 内 |               `0.0009765625 m` | 最小 edge 长度由 #125 与误差阈值分开冻结 |
| 速度                              |                                                 `0..=100 m/s` |      `0.00000762939453125 m/s` | 约 `360 km/h`                            |
| Profile 加速度/减速度             |                                        `0 < value <= 50 m/s²` |    `0.000003814697265625 m/s²` | 只约束配置输入，不截断实际应用加速度     |
| 车辆/Parking 尺寸、偏移、最小间距 | 偏移 `-128..=128 m`；尺寸 `0.1..=128 m`；最小间距 `0..=128 m` |         `0.0000152587890625 m` | 只有偏移可有符号；长度/宽度必须为正      |
| 期望车头时距                      |                                           `0 < value <= 60 s` |       `0.000003814697265625 s` | 零值仍是拒绝输入样本                     |
| 朝向角                            |                                                   `[-PI, PI)` | `0.0000002384185791015625 rad` | 使用 `PI` 附近 ULP                       |

route 总长、制动距离、候选行程、硬投影加速度和查询视距是派生值，不机械套用输入上界，也不静默饱和；它们通过有限性/溢出、连续误差和离散行为闸口。Spatial 局部坐标的 `-16_384..=16_384 m` 继续是 ADR 0013 的表现层验证范围，不是 Core edge 或 Data 模式的硬范围。

10 km 是防御性单 edge 上界，不是日常创作目标。更长道路通过多个 edge 组成 route；运行时不自动拆 edge，因为拆分会改变 ID、route 出现位置、Signal/Parking 锚点、事件归属和空间制品绑定。

### 3.2 连续误差上限

候选实现必须先保持状态、route 出现位置、edge 跨越、前车、约束、投影和事件种类/顺序**精确一致**。在此基础上，首轮连续值上限为：

- edge 长度、进度、保险杠间距、route 局部距离、Parking 几何与局部位置：每 tick/每次采样最大绝对差 `0.01 m`；
- speed：最大绝对差 `0.01 m/s`；
- 加速度：最大绝对差 `0.02 m/s^2`；
- 朝向角：最大绝对差 `0.0001 rad`；
- 非零常规值还必须满足验证文档中的相对误差阈值；接近零和边界只用绝对误差与离散行为判断。

`1 cm` 位置上限小于当前基准 `4.5 m` 车辆长度的 `0.23%`、小于 `2 m` 最小间距的 `0.5%`；`0.01 m/s` 等于 `0.036 km/h`。这些是研究接受上限，不是允许车辆发生 `1 cm` 重叠的安全豁免；无重叠投影的提交状态仍必须非负并保持离散结果一致。

### 3.3 累计误差单独判定

`f32` ULP 足够小，不代表逐 tick `progress += travel` 足够稳定。首轮 constant-input probe 中，`10 m/s`、`16 ms`、`6_250` ticks 的数学位移为 `1_000 m`，raw `f32` 重复累加得到约 `999.9282837 m`，误差约 `-7.17 cm`；带独立补偿量的 `f32` 累加约为 `1000.0000610 m`。

因此 G1 必须独立选择：

- authority/storage scalar；
- integration intermediate；
- residual/compensation 或从局部权威量重算策略；
- edge transition 时 residual 的消费、转移与清零规则。

若 hot state 改为 `f32` 却需要为每辆车增加一个 `f32` residual，必须把该字段计入内存比较；不能只按 `8 byte -> 4 byte` 宣称 progress 节省一半。

## 4. 接受的目标精度分层

| 数值职责                           | 已接受的目标表示                       | 权威归属           | 规则                                                    |
| ---------------------------------- | -------------------------------------- | ------------------ | ------------------------------------------------------- |
| 固定时间增量、tick、绝对时间、计数 | 经过检查的整数                         | Core               | 禁止浮点挂钟时间累计                                    |
| `EdgeLength`                       | 单个经过检查的 `f32`                   | Core               | 静态 edge 局部值，不使用残差                            |
| `EdgeProgress`                     | 私有 `high: f32` + `residual: f32`     | Core               | 有效值为逐分量升宽后的 `f64(high) - f64(residual)`      |
| `Speed`、Profile、尺寸/偏移        | 经过检查的 `f32`                       | Core               | 合法输入受第 3.1 节硬范围约束                           |
| `Acceleration`                     | 经过检查的有符号 `f32`                 | Core               | 实际应用加速度；只要求有限且可表示，不套用 Profile 上界 |
| 纵向/占用/约束/命令临时状态        | 经过检查的 `f32` 生产候选              | Core 派生状态      | 必须通过完整离散判定基准；不普遍升宽敏感控制            |
| route 距离派生索引                 | `f64` 前缀基线 / 分块局部 `f32` 候选   | Core 派生状态      | 冻结语义与闸口，由 #127 选择生产布局                    |
| Data JSON 数值规范化               | 高保真解析值 -> 经过检查的 Core 数值域 | Data/Core          | 解析 `f64` 只用于转换和原始诊断，不是第二权威           |
| 标准几何、弧长、标准位姿           | LaneFlow 自有的 `f64`                  | Spatial            | 保持标准空间精度并加入 Core 长度量化余量                |
| 局部表现位姿                       | 经过检查的 `f32` 或显式 `f64` 宿主路径 | 派生快照，不是权威 | 先在 `f64` 减去局部原点，再转换                         |
| 宿主向量/`Transform`               | 宿主专用                               | Adapter 表现       | 只能出现在 Adapter 末端                                 |
| `f16` / 量化整数                   | 仅用于显式编码/缓存                    | 非权威             | 独立缩放/原点/范围/溢出/错误契约                        |

研究结果解释：

- 直接 `f32` 的 36,000 tick 进度漂移和密集/走走停停控制误差超过预算；
- #122 的旧补偿 `f32` 只在写入时维护残差，读取前车间距、edge 剩余距离、边界和快照时仍只使用高位分量；它没有实现完整的补偿进度语义；
- #140 的补偿残差感知 `f32` 补全读取语义后，通过 5 种 edge 布局、3 种 1 万车辆场景和 10 万车辆密集观测的原严格上限，且无离散分歧；
- “`f32` 计算 + `f64` 进度”不再是唯一通过候选。#140 同轮研究模型中，补偿残差感知 `f32` 相对 `f64` 在 10 km 上界的 1 万车辆场景快约 2.4%–5.8%，10 万车辆密集场景快约 7.1%；该结果不足以直接外推到生产；
- 候选布局可从每辆车 `128 B` 缩到 `80–88 B`，但这是仅供研究的车辆/运动向量，不是完整 Core 已实现的总内存收益；
- ADR 0014 已接受上表作为目标契约，但当前生产实现在原子迁移和完整闸口完成前仍使用 `f64`。目标已冻结不等于生产已经切换。

## 5. 确定性与错误语义

精度变更不得降低以下现有保证：

- 相同 Core 版本、运行环境、初始状态和输入序列产生一致结果；
- `NaN` / 无穷值在进入权威状态前被拒绝；
- 有符号零由领域构造器规范化；
- tick/时间使用经过检查的整数运算；
- 验证/单步/命令失败不部分修改 world；
- 车辆/事件/更新顺序不依赖哈希迭代或浮点排序不稳定性；
- 边界、约束和投影的离散决策有明确且领域化的容差。

跨平台位级确定性仍不属于 v0.6 承诺。若候选精度改变事件、edge 跨越、前车或约束决策，即使最终位置误差很小，也视为行为差异，必须单独解释或阻断。

当前两个 `1.0e-9` 常量不得在未来的局部 `f32` 路径中直接转换后继续复用。后续实施至少要拆分：

- edge/几何的**最小有效尺寸**，属于输入语义，不是比较误差阈值；
- edge 边界吸附容差；
- 间距/重叠零值容差；
- 朝向角规范化容差。

未来容差必须由硬范围的最大 ULP、涉及的运算次数与上述连续误差上限共同约束。首轮以 `4 * max ULP` 作为加法、减法和吸附的保守表示下限：在 `10_000 m` 处约为 `0.00390625 m`，低于 `0.01 m` 上限。它仍不是已接受的生产常量；#125 必须通过判定基准和边界测试冻结各领域常量，不能把研究系数直接复制进生产。

## 6. 跨层转换边界

### 6.1 宿主事实不能反向决定 Core

各宿主当前公开 API 的选择并不统一：

| 宿主       | Transform/向量标量                                       | #122 解释                         |
| ---------- | -------------------------------------------------------- | --------------------------------- |
| Bevy       | `Transform.translation: Vec3`，`from_xyz(f32, f32, f32)` | 需要 local f32 presentation path  |
| Unity      | `Vector3(float x, float y, float z)`                     | 需要 local f32 presentation path  |
| Unreal     | `FVector` 为 `TVector<double>`                           | 可保留 f64 Adapter path           |
| Godot      | 默认 32-bit，支持 double-precision build                 | Adapter 按 build/ABI 选择末端转换 |
| Three.js   | JavaScript `number`                                      | host 数值语义不是 Rust f32 ABI    |
| Babylon.js | JavaScript `number`                                      | host 数值语义不是 Rust f32 ABI    |

资料： [Bevy Transform](https://docs.rs/bevy_transform/latest/bevy_transform/components/struct.Transform.html)、[Unity Vector3 constructor](https://docs.unity3d.com/ja/current/ScriptReference/Vector3-ctor.html)、[Unreal Core math types](https://dev.epicgames.com/documentation/en-us/unreal-engine/API/Runtime/Core)、[Godot Vector3](https://docs.godotengine.org/en/stable/classes/class_vector3.html)、[Three.js Vector3](https://threejs.org/docs/pages/Vector3.html)、[Babylon.js Vector3](https://doc.babylonjs.com/typedoc/classes/BABYLON.Vector3)。

### 6.2 转换顺序与失败语义

唯一允许生成局部 `f32` 位姿的顺序是：

```text
Core 的有效 f64 进度
  -> Spatial 标准 f64 采样
  -> canonical_f64 - local_origin_f64
  -> 有限性/范围/基向量校验 + 有符号零规范化
  -> 局部 f32 位姿批次
  -> 宿主 Transform
```

- 禁止 `(canonical as f32) - (origin as f32)`；必须先用 `f64` 消去大偏移。
- 转换 API 必须由 LaneFlow 拥有，返回结构化数值域/范围错误，并对整个批次执行“先计算、后提交”；失败不能留下部分输出或修改权威状态。
- `f32` 批次必须携带或绑定明确的原点/坐标框架身份，不能被另一个原点的 Adapter 状态误用。
- 双精度宿主可以消费标准/局部 `f64`，不需要人为降为 `f32`；这不形成第二套 Core 状态。
- 表现插值、原点重设和细节层级（LOD）只影响显示，不能反馈写入进度、速度、占用、状态或事件。

### 6.3 API/Data/Spatial/Adapter 迁移判断

| 接口面         | ADR 0014 目标决策                                                   | 迁移                                               |
| -------------- | ------------------------------------------------------------------- | -------------------------------------------------- |
| Core API       | 单一 `f32` 数值域的主 API 使用 `f32`；`EdgeProgress` 暴露有效 `f64` | 破坏性语义/布局变更；由原子迁移议题交付            |
| 当前 Data v0.5 | 迁移前 JSON 结构、加载器范围/诊断保持不变                           | 当前不变；下一有效格式按 ADR 0008 原子替换         |
| 下一 Data 格式 | 硬范围、规范化、诊断与 Core 同步                                    | 模式/加载器/固定样例同一切片；不保留运行时兼容垫片 |
| Spatial API    | 标准 `f64` + Core `f32` 长度量化余量                                | 修订长度绑定；标准/局部转换顺序不变                |
| Adapter API    | 有效进度/位姿到宿主 `Transform`；不暴露残差分量                     | v0.7 首次设计时落实                                |
| 量化存储/传输  | 当前不存在                                                          | 将来必须带版本、通过独立议题迁移                   |

即使 JSON 词法类型仍是 `number`，Core 标量、接受范围、舍入、诊断或线格式规范化的变化也必须按 ADR 0008 视为 Data/API 迁移。Data 可以先以 `f64` 或等价高保真值解析以保留原始错误输入，但只能通过显式受检转换进入 Core。

## 7. G1 证据与后续实施

#122 G1 已具备：

1. 可复现的 Core/Data 数值面 inventory；
2. 每个领域的范围、绝对/相对误差预算和 epsilon 推导；
3. 同一代表性 10k/100k workload 的 f64/raw-f32/旧 compensated/residual-aware/mixed 时间与 candidate memory；
4. production-aligned f64 oracle、逐 tick differential、长时累计、same-runtime replay、non-finite/signed-zero/boundary 证据；
5. f16 与整数定点的 range/error/禁止边界；
6. Core/Data/Spatial/Adapter migration 判断；
7. ADR 0012 的决策、备选方案、后果与实施边界。

#140 还补充了 edge 上界的生产 Core 稳态/构造/跨 edge 压力矩阵、每个最大误差的 tick/车辆/控制来源，以及 `10_000 m` 防御性单 edge 上界证据。完整数字与限制记录在 [`v0.6-numeric-validation.md`](../reference/v0.6-numeric-validation.md) 第 9 节。

#141 G1 进一步冻结：10 km Core/Data 硬上限、其他单一 `f32` 数值域的产品范围、`EdgeLength`/`EdgeProgress` 表示、公共 API 分层、route 距离的语义与候选、Spatial 长度量化余量，以及完整内存至少降低 10% 和必测工作负载不得有无法解释的超过 5% 中位数回退。ADR 0014 保存长期决策。

后续仍需独立交付：#125 领域误差阈值/最小 edge/Spatial 余量，#126 Core API/Data 版本，原子生产转换议题，#127 长期判定基准/性能/扩展常驻内存账本，以及 #128 收口审阅；Spatial/v0.7 分别交付真实几何与 Adapter 转换。任一生产闸口失败时保留当前 `f64`，不放宽预算或保留双精度开关。
