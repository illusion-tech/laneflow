# 数值表示与精度

**文档状态**: Accepted（已提交一维几何为整数毫米；Road Editing 3 / 来源编号修订已纳入现行规范）

**最后更新**: 2026-11-09

**适用范围**: 数值分层、误差预算、compiler 前端几何正本，以及已提交整数毫米合同

**关联文档**:

- `../architecture.md`
- `../roadmap.md`
- `../adr/0003-runtime-tick-and-determinism.md`
- `../adr/0004-core-implementation-language.md`
- `../adr/0012-core-numeric-authority-and-presentation-precision.md`
- `../adr/0015-bounded-f32-canonical-spatial-frames.md`
- `../adr/0022-authoring-curve-and-canonical-polyline-error-budgets.md`
- `../adr/0028-integer-millimeter-traffic-geometry.md`
- `traffic-runtime-integer-geometry.md`
- `compiler-foundation.md`
- `road-editing-source-and-geometry-frontend.md`
- `vehicle-following.md`
- `parking-system.md`

## 1. 状态与目标

本文记录现行数值合同：

- `TrafficWorld` / 共享热列 / LFCA 长度与限速为整数毫米 / `mm/s`；生产判定使用整数比较与 `100 mm` / `1 mm` 常量。
- 现行合同是 ADR 0028：已提交一维几何为整数毫米 + 微米余数，速度为 `u32` mm/s，步长 `4..=1000` ms，有折线边长由规范 `f32` 弧长派生后量化并写回 IR，`max_accel` / 舒适与紧急减速度均 `>= 0.5 m/s²`，不另做速度余数。准入后 Typed AST / HIR / MIR / LIR 交通一维与制品同一套整数，公开 `Canonical*View` 删除米制访问器。
- 空间层按 ADR 0015 使用每轴 `±16_384 m` 的 canonical `f32` 几何/位姿；编制曲线按 ADR 0022 在 `f64` 中求值。三维点/向量不并入毫米轴（#354）。

本设计固定数值职责，不固定 Spatial 几何容器、空间索引或具体引擎 ABI。引擎无关边界固定 batch-level frame/placement identity 和 canonical `f32` 位姿类型，但不包含宿主 Transform。

## 2. 现行数值分层

| 数值职责                           | 现行表示                               | 正本归属                       | 规则                                                   |
| ---------------------------------- | -------------------------------------- | ------------------------------ | ------------------------------------------------------ |
| 固定时间增量、tick、绝对时间、计数 | 经过检查的整数                         | Traffic Runtime                | 步长 `4..=1000` ms；禁止浮点挂钟时间累计               |
| 边长、边内进度                     | `u32` mm；进度另带 `carry_um` 微米余数 | 共享静态路网 / Traffic Runtime | 单边上界 `10_000_000` mm；边界与间隙用整数比较         |
| 已提交速度、道路限速               | `u32` mm/s                             | 共享静态路网 / Traffic Runtime | 不另做速度余数                                         |
| Profile 尺寸与最小间距             | 整数毫米                               | 编译器 / 共享静态路网          | 范围见第 4 节                                          |
| Profile 时距与加减速度             | 受检 `f32` SI                          | 编译器 / 共享静态路网          | 范围见第 4 节                                          |
| IIDM 中间量                        | 瞬时 SI                                | 派生值，不是正本               | 不得回写为已提交状态                                   |
| 路线距离                           | 分段 `u32` mm + 后缀 `BoundedDistance` | Traffic Runtime 派生状态       | 溢出为 `BeyondFinite`，不是注册失败                    |
| canonical 几何与位姿               | LaneFlow 自有的有界 `f32`              | Spatial                        | 每轴 `±16_384 m`                                       |
| frame 到宿主位姿                   | 宿主选择的 f32/double placement        | 派生快照，不是正本             | Adapter 显式映射 frame，不反写 Traffic Runtime/Spatial |
| 宿主向量/`Transform`               | 宿主专用                               | Adapter 表现                   | 只能出现在 Adapter 末端                                |

## 3. 范围边界

10 km 是防御性单 edge 上界，不是日常创作目标。更长道路通过多个 edge 组成 route；运行时不自动拆 edge，因为拆分会改变 ID、route 出现位置、Signal/Parking 锚点、事件归属和空间制品绑定。

route 总长、制动距离、候选行程、硬投影加速度和查询视距是派生值，不机械套用输入上界，也不静默饱和。Spatial 局部坐标的 `-16_384..=16_384 m` 是 ADR 0013 的表现层验证范围，不是 Traffic Runtime edge 的硬范围。

## 4. 编译器前端数值正本（#296 / #378）

#296 引入的 compiler 前端遵循已注册管线 LFRE wire → 有类型抽象语法树（typed AST）→ 高层中间表示（HIR）→ topology/geometry 中层中间表示（MIR）→ 已验证规范低层中间表示（validated canonical low-level intermediate representation，canonical LIR），并原子产出已验证源映射输入（validated source-map input）。该管线沿 ADR 0022 五层几何表示计量，不在制品/运行时整数毫米与 Spatial `f32` 之外新增第四种正本；canonical 折线自 geometry compile 起以有界 `f32` 贯穿高层中间表示（HIR）内部 `SpatialHir` 子表示、中层中间表示（MIR）与 LIR。`f64` 只保留在 road-editing analytic/reference 子表示，以及准入前的 LFRE 非几何交通/静态 wire。准入后交通一维在 Typed AST / HIR / MIR / LIR 存整数毫米或受检 `f32` SI。每一域显式归属为：

| 数值域                                      | 标量                                                 | 正本归属                                 | 规则                                                                                                                                                                                                                                                                                                                        |
| ------------------------------------------- | ---------------------------------------------------- | ---------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| LFRE wire 几何                              | `f64`（`Vec3F64` + corridor station/宽度标量）       | 私有 wire / schema                       | FlatBuffers `double`（含 `RoadCorridor` `start/end_station_meters`、`LinearWidthProfile` `start/end_width_meters`），format v4 正本；其它版本失败关闭；`f32` 量化发生在 geometry compile 边界，不在 wire/lowering 层                                                                                                        |
| LFRE 解析几何输入硬范围                     | `f64`                                                | compiler / 静态契约（static contract）   | 编制解析曲线控制点各分量在 `[-16_384, 16_384] m`；corridor start 有限且 `>= 0`，Finite end 有限、`> 0` 且严格大于 start，AlignmentEnd end 必须是 bit-exact `+0.0`；宽度两端有限且 `>= 0`，并且不能同时为零                                                                                                                  |
| LFRE 非几何交通/静态标量                    | wire `f64`；准入后为 mm / 受检 `f32`                 | 私有 wire / schema；准入后为 compiler IR | LFRE wire 的限速、Parking、`VehicleProfile` 等为编制 `f64`（限速有限且严格 `> 0`）。准入先量化再闭包。之后 Typed AST / HIR / MIR / LIR 存整数毫米或受检 `f32` SI，发射只写已提交值。几何路径的「禁用后端窄化」不适用于交通一维准入量化。                                                                                    |
| Parking / VehicleProfile 硬范围             | 编制 `f64` → 量化毫米 / `f32`                        | compiler / 静态契约（static contract）   | 先量化再闭包：停车锚点相对 **提交后** 边长 `L` 为 `1 <= p <= L-1` mm，横向绝对值 `1..=128_000` mm，长/宽与车长 `100..=128_000` mm，朝向量化并折叠后 `-π <= x < π`；期望车速 `1..=100_000` mm/s，时距 `(0, 60]` s，三项加减速 `0.5..=50` m/s²，min gap `0..=128_000` mm，emergency ≥ comfort                                 |
| LFRE enum / union 判别值                    | `u8`（wire `ubyte`）                                 | 私有 wire / schema                       | Road Editing v4 合法代码由 exact schema 冻结；unknown 均无效，零值按枚举定义判断。Straight、Uncontrolled、GateProhibition::None 的显式零值合法并保留字段存在性；Unspecified 与 union NONE 仍无效，新增或改义判别值必须提升来源格式版本                                                                                      |
| 来源链（provenance）`Digest256`             | `[u8; 32]`（wire `[ubyte:32]`）                      | 私有 wire / schema                       | `parameters_and_inputs_digest` / `frontend_options_digest` 固定为原始 32 bytes；Direct 来源链还要求 generator build ID 为 `laneflow-road-editing-direct-v1`，两个 digest 分别精确为 `6b27d0f76693bcd386ac13df724e30f5fb5ad3b9a152a5e1f88de1a624cea8aa` / `b1621e4a2db8d717b6506b0afb6fef5bd4d5156ecfe887c5abf36d08869c7892` |
| compiler 来源前端选择器                     | `u16`（`SourceLanguage`）                            | compiler 来源描述符                      | `SourceLanguage` 连续登记为 `1=SyntheticDsl, 2=RoadEditingSource`，不得与 LFRE wire format version 混同                                                                                                                                                                                                                     |
| compiler 来源描述符版本                     | `u32`                                                | compiler 来源描述符                      | SyntheticDsl `frontendVersion = 5`；RoadEditingSource `frontendVersion = 4`（路权策略实施合同）；`SOURCE_DOCUMENT_SET_DIGEST_VERSION = 1`。前端版本与文档集摘要版本是不同版本轴，均精确匹配，不并存旧读法                                                                                                                   |
| compiler 来源记录字节长度                   | `u32`（受检）                                        | compiler 来源文档描述符                  | `SourceDocumentDescriptor::source_record_byte_len` 为精确 `u32`；超出范围时前端失败关闭                                                                                                                                                                                                                                     |
| compiler 来源文档/文档集摘要                | `[u8; 32]`                                           | compiler 来源描述符                      | 单文档为 source record 的 SHA-256；文档集 SHA-256 原像依次为 `b"LFSOURCE-DOCUMENT-SET"`、little-endian `u32` version、little-endian `u32` document count，再按 key bytes 排序写入 little-endian `u32` key length、key、little-endian `u32` source-record byte length、document digest                                       |
| 几何语义版本（geometry semantics version）  | `u8`                                                 | compiler frontend options                | `ROAD_EDITING_GEOMETRY_SEMANTICS_VERSION = 1`；先于两个 profile code 写入 `frontend_options_digest`，冻结算术图变化时必须提升                                                                                                                                                                                               |
| compiler 几何配置档代码                     | `u8`                                                 | compiler frontend options                | SHA-256 `frontend_options_digest` 输入顺序固定为前导 `ROAD_EDITING_FRONTEND_OPTIONS_DOMAIN = b"laneflow.road-editing.frontend-options.v1\0"`、semantics version、Accuracy `1/2/3`、Direction `1/2/3`、`generator_options_digest: [u8; 32]`                                                                                  |
| `SourceLocation` 闭合数值族                 | `u32` / `u16` / `u8`                                 | compiler 来源位置                        | `SourceSpan` 一基 line/column、relation occurrence、context ordinal、wire physical index 与 checked byte-range start/length 为 `u32`；table field ID 为 `u16`；struct member ID 与 union discriminant 为 `u8`；byte range 加法或输入范围越界时 fail closed                                                                  |
| LFRE format_version                         | `u32`（wire `uint`）                                 | 私有 wire / schema                       | `RoadEditingSource.format_version` 为 FlatBuffers `uint`；精确值为 `4`（ADR 0029 与路权策略实施合同），其它值失败关闭                                                                                                                                                                                                       |
| 信号 offset/duration 计时器                 | `u64`（wire `ulong`）                                | 私有 wire / schema                       | portable 上限为 `9_007_199_254_740_991` ms；offset 为 `0..=max`，duration 为 `1..=max`，cycle 累加不得超过 max 且 offset 必须小于 cycle                                                                                                                                                                                     |
| 来源链（provenance）随机种子                | `u64`（wire `ulong`）                                | 私有 wire / schema                       | `OptionalU64.value` / `Provenance.random_seed` 为精确 `u64`，compiler 模型保留为 `Option<u64>`；仅 Generated 来源链接受可选完整 `u64` 范围，Direct 来源链必须为 `None`，不得无条件收窄或放宽                                                                                                                                |
| transition index / occupancy 计数           | `u32`（wire `uint`）                                 | 私有 wire / schema                       | `max_occupancy` 必须非零；`transition_index` 必须严格小于引用机动路径的派生 transition count（`path.edges.len() - 1`）                                                                                                                                                                                                      |
| access priority                             | `i32`（wire `int`）                                  | 私有 wire / schema                       | `priority` 带符号                                                                                                                                                                                                                                                                                                           |
| compiler/LIR ordinal                        | `u32`（受检）                                        | compiler                                 | `RoadEditingStringOrdinal` 等有类型 ordinal 冻结为 `u32`                                                                                                                                                                                                                                                                    |
| 标识 v1（Identity v1）固定域                | `[u8; 16]` / `[u8; 4]` / `u16` / `u32`               | 静态契约（static contract）/ compiler    | `StableId128([u8; 16])`、`IDENTITY_MAGIC: [u8; 4] = *b"LFID"` 与固定 `STABLE_ID_DOMAIN_PREFIX`；encoding version 为 `u16 = 1`，registry revision 为 `u16 = 4`，`EntityKind` / `FieldTag` 为 `#[repr(u16)]`；原像字段数使用 little-endian `u16`，每个字段字节长度使用 little-endian `u32`                                    |
| 有类型抽象语法树（typed AST）解析曲线控制点 | `f64`（`AuthoringPoint3F64`）                        | compiler                                 | ADR 0022：编制解析曲线（Line/Cubic Bézier）控制点在前端以 `f64` 求值                                                                                                                                                                                                                                                        |
| canonical 折线几何                          | `f32`（`CanonicalPoint3F32Input`）                   | compiler                                 | geometry compile 经 `quantize_point` 把 `f64` evaluator 输出受检量化为有界 `f32`；每个零分量经 `canonical_f32` 规范化为 bit-exact `+0.0f32`，禁止 `-0.0` 进入 LIR 语义指纹                                                                                                                                                  |
| 道路里程站位（Stationing）reference 基表    | `f64` 累计弦长                                       | compiler                                 | 配置档无关、按 source curve segment 组织的累计弦长；固定使用 `Fine2Cm + Smooth1Deg`，即 position `0.01 m` / direction half-angle `0.5°` 的 B1 policy；不按 edge 序列累计                                                                                                                                                    |
| 公开几何配置档目标                          | `f64`                                                | compiler                                 | Accuracy `0.02/0.05/0.10 m`；Direction full-angle `1°/2°/5°` 及其冻结 binary64 `cos²`；用于总目标、诊断与最终 canonical direction 检查                                                                                                                                                                                      |
| B1 细分接受阈值                             | `f64`                                                | compiler analytic/reference              | position `0.01/0.025/0.05 m`；direction half-angle `0.5°/1°/2.5°` 及其冻结 binary64 `cos²`；不得用公开 profile target 替代                                                                                                                                                                                                  |
| canonical 几何硬阈值                        | `f32`                                                | compiler / 静态契约（static contract）   | 点分量范围 `[-16_384, 16_384] m`、segment 长度严格 `> 0.1 m`、source join gap 上限 `0.005 m`、projected-up 长度 `>= 0.008_726_535`                                                                                                                                                                                          |
| Spatial 长度校验阈值                        | `f64`                                                | compiler / 静态契约（static contract）   | absolute tolerance `0.01 m`、relative tolerance `1.0e-6`                                                                                                                                                                                                                                                                    |
| 细分 / 正则性离散上限                       | depth `u8` / visits `u32`                            | compiler                                 | `MAX_SUBDIVISION_DEPTH = 20`；`MAX_REGULARITY_NODE_VISITS = 4095`，超过上限失败关闭                                                                                                                                                                                                                                         |
| 高层中间表示（HIR）内部 `SpatialHir` 子表示 | canonical 折线 `f32`                                 | compiler（owned）                        | 私有 `SpatialHir` 由 `build_spatial_hir` 在高层中间表示（HIR）构造期间建立；`HirCanonicalPoint3F32`/`HirSpatialSegment` 经 `freeze_spatial_polyline` 冻结。长度校验用 `length_mm / 1000` 作观察值对账弧长                                                                                                                   |
| geometry 中层中间表示（MIR）                | canonical 折线 `f32`                                 | compiler                                 | `MirCanonicalPoint3F32`/`MirSpatialSegment` 保留 Spatial 高层中间表示（HIR）的 canonical 折线                                                                                                                                                                                                                               |
| 已验证规范低层中间表示（canonical LIR）折线 | `f32`（`LirCanonicalPoint3F32`/`LirSpatialSegment`） | compiler 输出                            | 运行时规范正本几何。交通一维在制品/运行时与 LIR 均为整数毫米 / 受检 `f32`；发射只写已提交值，不再量化                                                                                                                                                                                                                       |
| LIR 同版本语义指纹（semantic fingerprint）  | `[u8; 32]`                                           | compiler 输出                            | `Lir::semantic_digest` 与 `CompilationMetrics::semantic_fingerprint()` 固定为 32 bytes；#298 的 portable artifact 字节完整性 digest 归 #298                                                                                                                                                                                 |

- 当前已提交交通一维为整数毫米 + `carry_um` + `u32` mm/s（ADR 0028）。compiler 编制曲线保持 `f64` 求值，canonical 折线仍为 `f32` 米。准入后 compiler IR 交通一维不再存编制米列。
- 注册管线只有有类型抽象语法树（typed AST）、高层中间表示（HIR）、中层中间表示（MIR）与已验证规范低层中间表示（canonical LIR）；私有 `SpatialHir` 是 `build_spatial_hir` 在高层中间表示（HIR）构造期间使用的内部子表示，不是第二个已注册阶段。其 canonical 折线（`HirCanonicalPoint3F32`/`HirSpatialSegment`）为 `f32`，由共享 `freeze_spatial_polyline` 冻结；官方来源不直接构造 Spatial 对象。
- road-editing analytic/reference 子表示（编制解析曲线、道路里程站位 reference 基表、配置档阈值、corridor station/宽度）均以 `f64` 存续；canonical 折线自 geometry compile 起量化到有界 `f32` 并贯穿高层中间表示（HIR）内部 `SpatialHir` 子表示、中层中间表示（MIR）与 LIR（ADR 0022 五层表示）。非几何交通/静态标量：LFRE wire 为编制 `f64`；准入后 IR 为整数毫米 / 受检 `f32`。
- 道路里程站位（Stationing）是 authoring 曲线的配置档无关累计弦长基表，按 source curve segment 组织，不属于运行时 edge 序列里程。
- 受检的几何 `f64 → f32` 量化只发生在 geometry compile 的 `quantize_point`（`geometry.rs`）：对 evaluator 输出分量做 canonical 范围检查后再 `as f32`，并由 `canonical_f32` 把每个 signed zero 规范化为 bit-exact `+0.0f32`；不允许在 wire/lowering 以隐式 `as`/截断提前量化折线点，也不允许在高层中间表示（HIR）内部 `SpatialHir` 子表示、中层中间表示（MIR）或 LIR 中对折线点二次量化（ADR 0022）。交通一维的毫米 / 受检 `f32` 量化发生在编制准入，不是这条几何路径。
- offset horizontal-regularity proof 在细分前对 binary64 derivative controls 执行独立 interval walk，证明水平导数在整个 source segment 不为零；该检查不产生 canonical point，也不能由端点 distance/direction test 替代（ADR 0022 220–244）。
- subdivision acceptance 与最终 canonical direction 检查把已量化的 `f32` 弦端点经 `promote_point` 无损提升回 `f64` 后做 distance/direction test（ADR 0022 246–280），保证判定阈值与 authoring numeric freeze 共享同一 binary64 `cos²`；这不授权在 road-editing analytic 子表示之外使用 `f32` 正本或未受检窄化。
- corridor station 端点（`s0/s1`）与宽度端点（`w_i`）从 wire 到有类型抽象语法树（typed AST）全程保持 `f64`（ADR 0022 166–190）；offset 求值 `O(t) = B(t) + d(s(t))·L(t)` 在 `f64` 完成后才经 `quantize_point` 量化，不在 offset 中间量提前窄化。

## 5. 确定性与错误语义

精度变更不得降低以下现有保证：

- 相同 Traffic Runtime 版本、运行环境、初始状态和输入序列产生一致结果；
- `NaN` / 无穷值在进入正本状态前被拒绝；
- 有符号零由领域构造器规范化；
- tick/时间使用经过检查的整数运算；
- 验证/单步/命令失败不部分修改 world；
- 车辆/事件/更新顺序不依赖哈希迭代或浮点排序不稳定性；
- 边界、约束和投影的离散决策有明确且领域化的容差。

跨平台位级确定性不属于当前承诺。若数值表示改变事件、edge 跨越、前车或约束决策，即使最终位置误差很小，也视为行为差异，必须单独解释或阻断。

## 6. 跨层转换边界

### 6.1 宿主事实不能反向决定 Traffic Runtime

各宿主当前公开 API 的选择并不统一：

| 宿主       | Transform/向量标量                                       | #122 解释                        |
| ---------- | -------------------------------------------------------- | -------------------------------- |
| Bevy       | `Transform.translation: Vec3`，`from_xyz(f32, f32, f32)` | 需要 local f32 presentation path |
| Unity      | `Vector3(float x, float y, float z)`                     | 直接映射 canonical f32           |
| Unreal     | `FVector` 为 `TVector<double>`                           | Adapter 在末端无损升宽 f32       |
| Godot      | 默认 32-bit，支持 double-precision build                 | Adapter 按 build/ABI 做末端映射  |
| Three.js   | JavaScript `number`                                      | host 数值语义不是 Rust f32 ABI   |
| Babylon.js | JavaScript `number`                                      | host 数值语义不是 Rust f32 ABI   |

资料： [Bevy Transform][bevy-transform]、[Unity Vector3 constructor][unity-vector3]、[Unreal Core math types][unreal-core]、[Godot Vector3][godot-vector3]、[Three.js Vector3][three-vector3]、[Babylon.js Vector3][babylon-vector3]。

### 6.2 Spatial frame 映射与失败语义

ADR 0015 接受的 runtime 顺序是：

```text
Traffic Runtime 的有效进度
  -> Spatial 有界 canonical f32 采样
  -> frame/有限性/点范围/基向量校验
  -> canonical f32 位姿批次
  -> Adapter 解析 frame 到宿主场景的放置
  -> 宿主 Transform
```

- 转换和批量 API 必须由 LaneFlow 拥有，返回结构化数值域/范围错误，并对整个批次执行“先计算、后提交”；失败不能留下部分输出或修改正本状态。
- `f32` 批次只在 header 携带一次 `CanonicalFrameId + FramePlacementToken`；每条记录只携带 vehicle handle 与 canonical pose，不重复 frame/placement identity。
- `FramePlacementToken(u64)` 是 Adapter/调用方颁发、Spatial 原样回显的不透明等值 token。同一 frame 重新放置、切 tile 或 rebase 时必须颁发新 token；Adapter 提交宿主 Transform 前必须比较当前 token 并拒绝旧批次。
- output frame mismatch 在读取 records 前失败；record 错误携带稳定输入序号和车辆句柄。全部记录先写调用方 scratch，成功后才交换 committed output 并更新 token；任何失败保持旧 frame/token/records 不变。
- 双精度宿主把 canonical `f32` 位置升宽后应用自己的 frame/world placement；这不会形成第二套 Spatial/Runtime 状态。
- 表现插值、原点重设和细节层级（LOD）只影响显示，不能反馈写入进度、速度、占用、状态或事件。

### 6.3 格式与 API 变更判断

Traffic Runtime 标量、接受范围、舍入、诊断或线格式规范化的变化必须按 ADR 0008 视为数据格式 / API 迁移；编制数值只能通过显式受检转换进入 Traffic Runtime。

[bevy-transform]: https://docs.rs/bevy_transform/latest/bevy_transform/components/struct.Transform.html
[unity-vector3]: https://docs.unity3d.com/ja/current/ScriptReference/Vector3-ctor.html
[unreal-core]: https://dev.epicgames.com/documentation/en-us/unreal-engine/API/Runtime/Core
[godot-vector3]: https://docs.godotengine.org/en/stable/classes/class_vector3.html
[three-vector3]: https://threejs.org/docs/pages/Vector3.html
[babylon-vector3]: https://doc.babylonjs.com/typedoc/classes/BABYLON.Vector3
