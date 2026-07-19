# 0012 Core 数值权威与表现层精度

**状态**: Superseded

**实施说明**: 已被 ADR 0014 取代；当前生产实现在迁移闸口完成前仍保持本文实现状态

**日期**: 2026-07-18

**适用范围**: LaneFlow Core/Data/Spatial/Adapter 的数值 authority、累计精度、presentation 转换与量化边界（#122）

**关联文档**:

- 上游决策:
  - `0001-project-scope.md`
  - `0003-runtime-tick-and-determinism.md`
  - `0004-core-implementation-language.md`
  - `0007-traffic-data-crate-and-loader-boundary.md`
  - `0008-pre-1.0-data-format-version-policy.md`
- 详细设计与证据:
  - `0014-residual-aware-f32-core-authority-and-migration-gates.md`
  - `../design/numeric-representation.md`
  - `../design/core-runtime.md`
  - `../design/data-format.md`
  - `../reference/v0.6-numeric-validation.md`
  - `../roadmap.md`

## 背景

LaneFlow 在首个 Engine Adapter 前需要冻结交通状态、道路几何和宿主 Transform 之间的数值边界。当前 Core 的连续领域量使用 `f64`，fixed tick、tick index 和 absolute time 使用 checked `u64`。Bevy、Unity 和 Godot 默认使用 32-bit vector；Unreal 当前 `FVector` 使用 double；Three.js 和 Babylon.js 的 API 使用 JavaScript `number`。宿主并不存在一个可反向决定 Core authority 的共同标量。

把 Core 全量改成 `f32` 可能缩小 hot state，但也会改变当前接近“全部有限 `f64`”的公共输入范围、错误 payload、JSON normalization、edge boundary、route total、控制器与累计语义。把 `f16` 用于 authority 会进一步引入明显量化误差。另一方面，永远把大世界绝对坐标直接 cast 到宿主 `f32` 也会在 Adapter 端丢失可见精度。因此数值选择必须按 authority、累计、局部表示和 presentation 分层，而不是按“整个项目只用一种 float”处理。

#122 在 commit `a001b5b2d567a172fcaa462e44ed70863fb6f774` 建立了不进入 production Core 的 f64/raw-f32/compensated-f32/mixed 候选模型。f64 模型先在 legacy/locality 两种布局、free-flow/dense/stop-and-go 三类场景与 production Core 逐 tick 对齐，再用于 10k/100k 差分、Criterion、长时累计、layout memory 和量化研究。详细限制与原始结果由 validation 文档保存。

## 2026-07-18 后续复核状态

#140 发现本 ADR 使用的旧 compensated f32 候选只在累加时保存 residual（补偿残差），但 leader gap、edge remaining、boundary 和 snapshot 仍只读取 high component。补全 residual-aware 读语义后，研究模型在 5 种 edge 布局、3 种 10k 场景和 100k dense observation 中通过原严格预算且无离散分歧；同轮 Criterion 中相对 f64 快约 2.4%–7.1%，候选 retained hot layout 为 `88 B/vehicle`，相对 f64 的 `128 B/vehicle` 低 31.25%。

因此，本 ADR 的以下证据前提已被 #140 推翻：

- “只有保留 f64 progress 的 mixed 候选通过”；
- “所有 f32 候选都没有 wall-time 优势”；
- “compensated f32 的剩余误差证明控制链必须回到 f64”。

ADR 0014 已接受补偿残差感知 `f32` 的下一目标契约，并取代本文关于“目标权威继续统一为 `f64`”的长期目标决策。#144 的首次独立原子迁移通过正确性和内存护栏，但稳态性能只提升 `4.257%`、未达到 `5%` 门槛，因此当前生产仍保持本文的 `f64` 状态。完整数字与最大误差来源见 `../reference/v0.6-numeric-validation.md` 第 9 节和第 15 节。

## 决策

### 1. v0.6 Core 连续 authority 保持 `f64`

- `EdgeLength`、`EdgeProgress`、`Speed`、`Acceleration`、Vehicle Profile、Parking anchor/geometry，以及 committed vehicle motion state继续使用 `f64`。
- occupancy、longitudinal candidate、constraint、command spatial 和 route-distance 等会影响离散交通行为的中间量/retained index继续使用 `f64`。
- route total、跨 edge distance 与逐 tick progress integration 不降为 raw `f32`，也不增加仅为抵消 `f32` drift 的 hidden residual authority。
- public Core domain types 不改成 engine scalar、可配置泛型或 build-time precision feature。相同 LaneFlow data 和 Core API 不因宿主引擎而产生两套交通语义。
- fixed delta、tick index、absolute time 和其他离散计数继续使用 checked integer，不改为浮点累计。

这不是“`f64` 在所有硬件上天然更快”的普遍声明。#122 最初观测到所有 f32 候选均无 wall-time 优势且只有保留 f64 progress 的 mixed 候选通过；#140 证明该候选定义和性能结论不完整；#144 又完成了真实 production 转换，实测提升 `4.257%`，但不足以跨过破坏性迁移的 `5%` 门槛。当前继续使用 f64 的依据是该正式不迁移（no-go）结论，而不是 f32 被永久证明无收益。

### 2. current Data format 不因 #122 改变

- current v0.5 JSON 继续使用 JSON number，经 Data loader 进入现有 `f64` Core domain constructors。
- #122 不增加 precision tag、quantized integer encoding、scale/origin metadata，不改变 schema range，也不提升 `formatVersion`。
- Data 必须继续拒绝 non-finite/invalid domain values，并保持 current diagnostics 与 atomic normalization。
- 将来若收窄 current accepted range、改变 Rust public scalar、引入量化 wire encoding 或修改 rounding/saturation，必须按 ADR 0008 建立新的 current data version 和迁移 Issue；JSON token 仍叫 `number` 不能作为“无格式影响”的理由。

### 3. Spatial canonical authority 使用 LaneFlow-owned `f64`

- #123 的 canonical geometry、arc length、sampling parameter、canonical position 和会决定 Core edge binding/length consistency 的量使用 LaneFlow-owned `f64` 类型。
- Core 仍只拥有 traffic coordinate/progress authority；Spatial 拥有 engine-neutral geometry authority。两者的长度一致性、容差和 binding 由 #123 冻结，不把 engine spline/mesh 变成事实源。
- world/georeference offset 不进入 Core hot vehicle state。大范围场景通过 canonical frame、local origin/tile 和显式转换组合，而不是让车辆持有宿主 world transform。
- #122 的 `16_384 m` local envelope 是验证范围，不自动成为 schema hard limit；#123 若选择不同范围，必须重新证明 f32 presentation 误差预算。

### 4. `f32` 只在显式 local presentation 边界产生

LaneFlow 提供或定义 owned conversion，使转换顺序固定为：

```text
Core progress f64
  -> Spatial canonical pose f64
  -> subtract canonical local origin in f64
  -> validate finite/range and canonicalize signed zero
  -> checked f32 local pose
  -> Adapter maps to host Transform/vector
```

- 禁止先把 canonical/world `f64` cast 为 `f32` 再减 origin；那会提前丢失低位。
- overflow、non-finite、超出已声明 local envelope 或无效 basis 必须返回 LaneFlow-owned 可诊断错误；禁止 silent saturation、wrap 或隐式 `as` 出现在公共边界。
- Adapter 可以保留 `f64` 路径供 Unreal 或其他 double host 使用，也可以消费 checked local `f32` batch；两条路径都不得改写 Core/Spatial authority。
- Bevy/glam、Unity、Unreal、Godot、Three.js、Babylon.js 的具体 vector/Transform 类型只能出现在各自 Adapter 末端，不能进入 Core/Spatial/Data 公共契约。
- presentation interpolation、camera-relative rebasing 和视觉 LOD 可以使用 host-local precision，但不得改变 Core tick、vehicle status、occupancy、route progress 或事件。

### 5. `f16` 与量化整数不是权威运行时表示

- IEEE binary16 不用于 edge/progress、speed、gap、constraint、geometry、heading 或 pose authority；本轮矩阵中只有 acceleration 范围满足单项绝对误差 ceiling，其余关键域均失败。
- `f16` crate 只保留为 research/test dependency，不进入 production Core/Data/Spatial/Adapter dependency surface。
- 整数量化在选定 scale 下可以满足部分 round-trip ceiling，但它改变 range、saturation、arithmetic 和 wire semantics，不能作为无损内部替换。
- 将来可以在远景 presentation cache、可丢弃 GPU/transport buffer 或明确 versioned encoding 中单独采用 `f16`/quantization；每个用途必须记录 scale、origin、rounding、range、overflow policy、误差预算和不可作为 authority 的声明。

### 6. 数值判定按领域拆分，当前行为不静默改变

历史 `EDGE_BOUNDARY_EPSILON` 与 `GEOMETRY_GAP_EPSILON` 数值相同但职责混杂。#125 已保持 current-f64 值和比较边界不变，把最小 edge/vehicle/Parking 输入、anchor 端点留白、edge boundary/remainder、纵向约束、物理 gap/overlap 和 computed-speed near-zero 拆为九个 crate-private owner，并直接删除两个旧公开常量。

Spatial heading/basis canonicalization 继续由 #123 拥有。target-f32 的九领域值、Spatial 量化余量和边界判定由 #127 使用范围、ULP、真实运算链和行为 oracle 离线标定；#144 候选曾原子启用但 no-go 后已回退。`4 * max ULP` 只是研究下限，不是已接受的 production 常量；相对误差和 ULP 不进入 production predicate。

### 7. 确定性与失败原子性不因 presentation 精度降低

- 相同 Core 版本、运行环境、初始状态和输入序列继续产生一致结果；跨 CPU/语言/编译器 bit-level float determinism 仍不承诺。
- non-finite rejection、signed-zero canonicalization、checked integer overflow、稳定 iteration/event order 和失败原子性继续是 hard gate。
- f32/f16 presentation buffer 是从 committed f64 snapshot 派生的只读结果。转换失败不得部分提交 batch，也不得修改 Core/Spatial authority。
- 如果未来 scalar 候选改变 edge transition、leader、constraint、projection、status 或事件，即使最终 position error 很小，也必须视为行为变更并重新走 ADR/G1，而不是性能优化。

## 证据摘要

- f64 研究模型在 10k、两种布局和三类 Vehicle Following 场景中与 production Core 的离散事件及连续状态通过逐 tick 对齐。
- #122 的旧 raw/compensated f32 在 10k 的密集或启停场景超过 speed/acceleration budget；legacy free-flow 的 progress drift 分别达到约 `4.951 m` / `0.125 m`。这些数字是历史候选结果，不代表完整 residual-aware 语义。
- #122 的 mixed `f32 compute + f64 progress` 在全部 10k 组合及 100k dense observation 中满足当前 ceiling；历史 Criterion median 相对同布局 f64 慢约 `3.79%..38.65%`。
- #140 的 residual-aware f32 在 5 种 edge 布局、3 种 10k 场景和 100k dense observation 中通过相同严格 ceiling，且无离散分歧；100k dense 最大 progress/speed/acceleration 绝对误差分别约为 `0.000000249 m`、`0.000000608 m/s`、`0.000007241 m/s²`。
- #140 同轮 Criterion 中，10 km cap 的 residual-aware f32 相对 f64 在 10k free/dense/stop 分别快约 2.4%/5.8%/5.0%，100k dense 快约 7.1%。这是 research-only scalar model 证据，不是完整 production 收益承诺。
- 36,000 tick constant-addition 中 raw f32 在 `30 m/s` 下误差约 `8.566 m`；mixed 的误差约 `0.000386 m`。
- 研究模型 retained layout 为 f64 `128 B/vehicle`、raw f32 `80 B`、compensated/mixed `88 B`。这是 scalar candidate vectors 的布局差，不是完整 Core 的已实现内存收益；production 扩展 accountant 仍为 `789.21 B/live vehicle`。
- binary16 最大 round-trip error：progress `4 m`、speed `0.03125 m/s`、extent/offset `0.03125 m`、heading `0.0009765625 rad`，均超过相应 ceiling。

## 后果

正向后果：

- Core/Data current contract 暂无 scalar/schema migration，避免在完整 production candidate、跨组件 oracle 和迁移方案完成前引入破坏性变更。
- route total、boundary、跟驰与 constraint 继续共享一个稳定 f64 authority，避免 residual、saturation 和 mixed rounding 成为隐藏交通状态。
- local-origin 后的 checked f32 输出可以服务 Bevy/Unity/Godot，同时不牺牲大范围 canonical geometry；double host 也不被迫降精度。
- f16/quantization 保留在可丢弃/版本化边界，可在真实带宽或 GPU 证据出现时独立优化。

成本与风险：

- 当前 Core hot vectors 不获得候选模型中约 31%–38% 的局部 layout 缩减；整体内存优化仍需从容器、scratch 生命周期、分区和 workload locality 寻找证据。
- f64-to-local-f32 conversion 需要显式 batch API、origin 生命周期、range diagnostics 和全批次失败语义。
- #123 仍需冻结 canonical/local frame、geometry representation、length consistency 和 sampling API；本 ADR 只冻结数值角色，不替代空间设计。
- 保持 `f64` 不解决 #72 的 active-agent partition、多频率或 mesoscopic 扩展问题；presentation LOD 也不能替代 Core fidelity 分层。

## 替代方案

### Core 全量 authoritative `f32`

raw f32 authority 会收窄 current API/Data compatibility，raw integration 有明显累计漂移，且旧 dense/stop 控制误差超预算，因此仍拒绝“无 residual、无迁移设计的全量 raw f32”。#140 已重新打开 residual-aware f32 候选，但它不是本小节所指的 raw 替换。

### `f32` progress 加 residual/compensation

旧候选的残差只参与写入，不参与间距/剩余量/边界/快照读取，因此其密集/停车失败不能再作为拒绝完整补偿的依据。#140 的补偿残差感知候选通过当前研究预算，候选内存降低 31.25%，同轮 Criterion 也快于 `f64`。ADR 0014 已接受它作为下一目标，但在 Core API/Data/Signals/Parking/route/命令全链验证与原子迁移前，仍不作为当前生产权威。

### mixed `f32 compute + f64 progress`

#122 阶段它是唯一通过差分预算的 `f32` 候选；#140 已证明补偿残差感知的 `f32` 进度也能通过，因此“唯一”不再成立。它仍是有效对照，但不是默认胜者；ADR 0014 要求在完整生产布局上继续比较补偿残差感知的 `f32`、“`f32` 计算 + `f64` 进度”与当前 `f64`。

### 所有层统一 `f64`，Adapter 也不转换

无法直接映射默认 f32 的 host Transform/GPU buffer，也会把大世界精度问题推给每个 Adapter 自行解决，因此拒绝。权威 f64 与末端 local f32 必须同时存在但职责分离。

### authoritative `f16` 或统一量化整数

binary16 在多数关键域直接超过误差 ceiling；整数格式需要领域专用 scale/range/saturation，不能统一替代连续控制和几何计算，因此拒绝。

## 实施与复核

- #122：交付 inventory、误差预算、10k/100k runtime/memory、f64 oracle、f16/quantization、ADR 与跨层迁移判断。
- #140：复核 edge 上界、修复 compensated candidate 的 residual-aware 读语义、保存 argmax provenance，并证明旧的唯一候选/无性能收益前提失效。
- #123：在本 ADR 上冻结 Spatial canonical/local frame、f64 geometry authority、length consistency、sampling 与 checked presentation 输入。
- G1 后拆分独立 Issue：领域 epsilon/validation、Data/API migration audit、长期 numeric validation/performance 和 #122 closure review。
- v0.7 Adapter API/Bevy 实施显式 local-origin 后的 f32 batch conversion，并在真实 host 上复核 batch cost、range error 与失败原子性。
- #141 已交付取代本文的 ADR 0014 目标决策；只有完整的生产补偿残差感知候选补齐正确性、性能、总内存和 API/Data 迁移证据后，才改变当前权威。#140 足以重开设计，ADR 0014 足以冻结目标，两者都不足以单独完成生产迁移。
