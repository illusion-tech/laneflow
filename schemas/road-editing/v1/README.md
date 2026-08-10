# 道路编辑来源缓冲区 v1 Schema

**状态**: #296 G1 冻结候选；尚未进入 production reader/writer，也未公开发布<br>
**格式标识**: `LF-ROAD-EDITING-SOURCE-v1`<br>
**机器事实源**: [`road-editing.fbs`](road-editing.fbs)<br>
**设计依据**:
[`road-editing-source-and-geometry-frontend.md`](../../../docs/design/road-editing-source-and-geometry-frontend.md)、
[ADR 0023](../../../docs/adr/0023-road-editing-state-and-phased-network-replacement.md)、
[曲线误差档](../../../docs/adr/0022-authoring-curve-and-canonical-polyline-error-budgets.md)、
[横断面与准入](../../../docs/design/cross-section-access.md)、
[信号](../../../docs/design/signal-system.md)、
[停车](../../../docs/design/parking-system.md)、
[车辆跟驰](../../../docs/design/vehicle-following.md)

本目录冻结 FlatBuffers 的精确类型、字段、字段 `id`、枚举值、union 判别值、根表和
file identifier。`.fbs` 冻结 wire shape；本文冻结不能只靠 verifier 表达的领域语义。
两者必须同时满足。G1 通过前不得把本 schema 写成已发布或已实现能力。

## 1. 文档边界

- 一个逻辑来源模块恰好对应一个标准 size-prefixed FlatBuffer；根表是
  `RoadEditingSource`，file identifier 是 ASCII `LFRE`，默认扩展名是 `.lfre`。
- `format_version` 必须精确为 `1`。零、未知值和后继版本都在任何 LaneFlow 语义
  lowering 或按规模分配前拒绝。
- `module_header`、`road_alignments` 和 22 类稳定声明向量都必须在 wire 中
  存在；向量可以为空。宿主以多个模块 blob 和显式导入图组成城市项目。
- 完整 size-prefixed bytes 是 `sourceDocumentDigest` 与 `sourceRecordByteLen` 的输入；
  buffer bytes、table offset 和 vector index 都不构成领域身份或语义等价依据。

## 2. 键、引用与显示关联

### 2.1 Token

所有 namespace、document key、稳定声明 key、road alignment key、`kind_id`、有向
approach key、引用和 `canvas_selection` 都先受调用点的 `max_single_string_bytes`
约束。稳定标识 token 另外满足：

```text
首字节: ASCII 字母或数字
后续字节: ASCII 字母、数字、.、_、:、/、-
```

值大小写敏感，不 trim、不 case-fold、不 Unicode normalize。namespace、声明 local
key 和 `road_alignment_key` 都不能包含保留分隔符 `::`；单个 `:` 仍是合法 token
字符。

### 2.2 有类型引用

所有名为实体引用的 `string` 字段只接受：

```text
local-key
namespace::local-key
```

限定引用必须恰好包含一个 `::`，两侧都非空且分别满足上述 token 规则；不接受多个
分隔符。限定引用的 namespace 必须等于当前模块或出现在 `module_header.imports`；被
引用实体的种类由字段或伴随的封闭 enum 决定。`RoadCorridor.road_alignment_key` 只
引用当前模块的 `RoadAlignment`，不能跨模块，因为道路走向不是 Identity v1 实体。

### 2.3 Canvas 关联

可选 `canvas_selection` 保存编辑器稳定画布对象键。它进入来源精确摘要和诊断位置，但
不进入 `CanonicalIdentity`、`StableId128`、LIR 语义指纹或运行时镜像。属性面板定位
继续由 `entity reference + typed property path` 派生，不在 schema 中增加任意 map。

## 3. 根表与身份层

`RoadEditingSource` 包含：

1. `format_version` 与唯一 `ModuleHeader`；
2. 显式位置/方向配置档；
3. `road_alignments:[RoadAlignment]`；
4. 按 Identity v1 种类代码 1–22 排列的 22 个有类型声明向量。

`RoadAlignment` 是可继续编辑的当前道路走向定义。其 `road_alignment_key` 在模块内稳定
且唯一，但它不是第 23 种静态路网实体，不分配 `StableId128`，也不进入 LIR。它只为
一个或多个 `RoadCorridor` 提供同一规范坐标框架中的参考曲线。

22 个稳定声明向量与 Identity v1 一一对应：

| 代码 | FlatBuffers table  | 稳定 key 字段           | 所有者 / 身份锚点来源                       |
| ---: | ------------------ | ----------------------- | ------------------------------------------- |
|    1 | `RoadCorridor`     | `road_corridor_key`     | 当前 module namespace                       |
|    2 | `RoadSection`      | `road_section_key`      | 唯一 `RoadCorridor.elements`                |
|    3 | `AuthoringLane`    | `authoring_lane_key`    | 唯一 `RoadSection.authoring_lanes`          |
|    4 | `LaneEdge`         | `lane_edge_key`         | 当前 module namespace；角色不参与身份       |
|    5 | `Junction`         | `junction_key`          | 当前 module namespace                       |
|    6 | `Movement`         | `movement_key`          | `junction` + 两个有向 approach key          |
|    7 | `ManeuverPath`     | `maneuver_path_key`     | `movement`                                  |
|    8 | `ManeuverGate`     | `maneuver_gate_key`     | `maneuver_path`                             |
|    9 | `WaitingZone`      | `waiting_zone_key`      | `maneuver_path`                             |
|   10 | `StopLine`         | `stop_line_key`         | 当前 module namespace                       |
|   11 | `SignalGroup`      | `signal_group_key`      | 当前 module namespace                       |
|   12 | `SignalController` | `signal_controller_key` | 当前 module namespace                       |
|   13 | `SignalPhase`      | `signal_phase_key`      | 唯一 `SignalController.signal_phases`       |
|   14 | `ParkingArea`      | `parking_area_key`      | 当前 module namespace                       |
|   15 | `ParkingSpace`     | `parking_space_key`     | 当前 module namespace；可选 area 不参与身份 |
|   16 | `LaneGroup`        | `lane_group_key`        | `road_section`                              |
|   17 | `FacilityBand`     | `facility_band_key`     | 唯一 `RoadCorridor.elements`                |
|   18 | `ParticipantClass` | `participant_class_key` | 当前 module namespace                       |
|   19 | `AccessRule`       | `access_rule_key`       | 当前 module namespace                       |
|   20 | `VehicleProfile`   | `vehicle_profile_key`   | 当前 module namespace                       |
|   21 | `StaticRoute`      | `static_route_key`      | 当前 module namespace                       |
|   22 | `CanonicalFrame`   | `canonical_frame_key`   | 当前 module namespace                       |

所有 table 在其根向量中的物理顺序都不进入语义。一个 buffer 只属于一个 namespace，
且 owner 可能位于另一模块；因此官方 writer 对 `RoadAlignment` 按模块内
`road_alignment_key`、对每个稳定声明向量按模块内 local key 的 UTF-8 bytes 排序，
不尝试在单模块 writer 中构造跨模块 `CanonicalIdentity`。有语义顺序的 owner 向量
保持产品顺序；无序引用集合先规范引用拼写（当前模块使用 local form，导入模块使用
qualified form）再按 bytes 排序。production reader 接受任意物理顺序并在 HIR/MIR 中
闭合完整身份与最终规范顺序。

## 4. 模块头、固定单位和配置档

- `ModuleHeader.authoring_namespace_id` 是该模块的 Identity v1 namespace；
  `source_document_key` 与机器路径无关；`imports` 按集合解释，重复、自导入、未知模块
  和环均失败关闭。
- `Provenance` 的两个 `Digest256` 是原始 SHA-256 bytes。`Direct` 必须使用
  `generator_build_id = "laneflow-road-editing-direct-v1"`、缺失 `random_seed`，并分别以
  ASCII `laneflow.road-editing.direct.inputs.v1\0` 和
  `laneflow.road-editing.direct.frontend-options.v1\0` 为完整前像计算 SHA-256；对应十六进制
  检查值是 `6b27d0f76693bcd386ac13df724e30f5fb5ad3b9a152a5e1f88de1a624cea8aa` 与
  `b1621e4a2db8d717b6506b0afb6fef5bd4d5156ecfe887c5abf36d08869c7892`。
  `Generated` 必须保存真实生成器 build ID、参数/输入摘要、来源前端选项摘要和可选
  seed。两类 `description` 都必须是非空、受长度上限约束的可见 ASCII。沿袭不认证内容，
  也不能替代实际保存的道路编辑状态。
- v1 不保存一张永远固定、却会与字段后缀竞争权威的 `Units` table。距离、角度、速度和
  时间单位由精确字段名冻结为米、弧度、米每秒、秒或毫秒；writer/reader 不能接受
  每文档单位切换，也不能把宿主单位隐式带入来源。
- `geometry_accuracy_profile` 只接受 `Fine2Cm / Balanced5Cm / Compact10Cm`；
  `geometry_direction_profile` 只接受 `Smooth1Deg / Balanced2Deg / Compact5Deg`。
  `Unspecified` 一律无效，不存在隐式默认、任意浮点容差或资源不足自动降级。
- 最终 `SourceModuleDescriptor.frontendOptionsDigest` 对以下精确前像计算一次 SHA-256：

  ```text
  ASCII "laneflow.road-editing.frontend-options.v1\0"
  || geometry_accuracy_profile: u8
  || geometry_direction_profile: u8
  || provenance.frontend_options_digest: [u8; 32]
  ```

  它绑定实际选择的两个几何档位；不进入来源文档摘要或实体稳定身份。

## 5. 道路走向、横断面和车道

- `RoadAlignment.reference_line` 至少包含一个 `CurveSegment`。v1 union 只接受
  `LineSegment` 和 `CubicBezierSegment`；段起点是前一段终点，第一段起点是
  `CurveProgram.start`。所有点必须有限并位于规范 frame 可证明的范围内。圆弧、螺旋线
  和其他外部曲线必须由 importer/generator 在写入前转换为这两种 segment，并在所选
  2/5/10 cm 位置档内证明误差；v1 不保存原始曲线原语语义。
- 每个 `RoadCorridor` 绑定一个当前模块 alignment 和半开 station 区间。第一段从
  `0 m` 开始；同一 alignment 的 corridor 按 station 无重叠、无间隙完整覆盖。
  `end_station_kind = Finite` 时读取严格更大的 `end_station_meters`；末段必须使用
  `AlignmentEnd`，此时 `end_station_meters` 必须为规范 `+0.0` 且不参与含义。
- `RoadCorridor.elements` 的顺序是横断面从左到右的语义顺序；每个引用按 `kind` 指向
  `RoadSection` 或 `FacilityBand`。每个 section/band 必须在整个模块图中恰有一个
  corridor owner。
- `reference_section` 必须出现在 `elements`；`reference_lane` 必须出现在该 section 的
  `authoring_lanes`，方向为 `Forward`，其中心线精确绑定 reference line。
- `RoadSection.authoring_lanes` 是从左到右的语义顺序。每个 `AuthoringLane` v1 恰好
  覆盖一个 `lane_edge`，携带线性宽度档、显式方向和可选 lane group。宽度两端都必须
  有限且非负，不能同时为零；在 corridor station 上按端点线性插值。零端点只表达该
  member 在该 corridor 边界开始或结束，不允许用同一 lane 身份穿过零宽边界；与另一
  edge 的拓扑连接仍须显式声明。需要把同一逻辑车道跨多个 corridor span 延续时，使用
  不同稳定 lane/edge 并通过显式 successor 连接，不从左右下标猜测身份。
- `LaneGroup.road_section` 是唯一 owner；被 lane 引用时必须与该 lane 所属 section 相同。
- `FacilityBand` 携带 non-traversable `kind_id` 和同样的线性非负宽度档，不产生可遍历
  边；零端点同样只能表达 member 在 corridor 边界开始或结束。
- `LaneEdge.speed_limit_meters_per_second` 必须有限且严格大于零。道路区段 edge 不设置
  `explicit_geometry`，其中心线由 alignment、station、横断面宽度和方向派生；路口
  internal edge 必须设置 `explicit_geometry`。`successors` 只声明不穿越 junction
  internal path 的普通延续，路口转换从 `ManeuverPath` 唯一派生。
- 每个 `LaneEdge` 恰好处于两种互斥角色之一：section-derived edge 被恰好一个
  `AuthoringLane` 引用、不是任何 junction internal edge、没有 `explicit_geometry`；
  junction-internal edge 被恰好一个 `Junction` 拥有、不被 `AuthoringLane` 引用、必须
  有 `explicit_geometry`、`successors` 为空，并至少被该 junction 的一条 maneuver path
  使用。两者同时成立或都不成立均失败关闭。
- reference lane 的中心线精确等于 alignment reference line。其余 lane/facility 的
  横向位置在每个 station 由从左到右宽度前缀的半宽累计唯一导出；v1 不接受独立横向
  offset 或第二份中心线。`Backward` lane 先在 alignment 参考方向求 offset，最终规范
  点序列再反转到行驶方向；successor、停车 progress 和 StopLine 均按该行驶方向解释。

## 6. 路口、控制、停车与准入

- `Junction.approach_edges` 和 `internal_edges` 按集合解释。internal edge 必须由一个
  junction 唯一拥有、具有显式曲线，并至少被一条属于该 junction 的 maneuver path
  使用。每条 path 的 entry、全部 internal 和 exit edge 必须解析到同一
  `CanonicalFrame`；一个 internal edge 被使用的全部 path 也必须从 entry/exit approach
  导出同一个 frame，冲突即失败。internal edge 不另存 frame 字段。
- `Movement.junction` 是唯一 owner；有向 entry/exit approach key 是 Identity v1 字段，
  不从几何或边名推断。`ManeuverPath` 的权威边序列固定为
  `entry_edge + internal_edges + exit_edge`。
- `ManeuverGate.transition_index` 指向路径边序列的一个有效相邻转换。
  `signal_control = None` 时 `signal_group` 必须缺失；`SignalGroup` 时该字段必须存在。
- `WaitingZone.max_occupancy` 必须大于零；entry/release gate 必须属于同一路径且顺序合法。
- `SignalController.signal_groups` 是无序唯一 owner 集合；`signal_phases` 的顺序定义循环。
  每个 phase 被恰好一个 controller 拥有，`duration_milliseconds > 0`，其 owner-local
  `states` 对 controller 全部 signal group 恰好各出现一次。
- `ParkingSpace.parking_area` 只是可选组织关系，不参与身份。entry/exit progress、矩形
  尺寸、横向偏移和朝向继续遵守已接受停车几何范围。
- `ParticipantClass.extends` 形成可选无环单继承。`VehicleProfile.iidm` 只描述当前道路
  机动车执行域，全部数值继续遵守当前 IIDM 范围。
- `AccessRule.target_kind + target_reference` 构成一个封闭有类型目标；参与者类别集合
  非空。v1 只允许 `LaneEdge / LaneGroup / RoadSection / ManeuverPath`；数值 5 保留且
  无效，第一方 writer/UI 不提供 `FacilityBand` target。后继若支持该能力，必须提升来源
  格式版本。`regulation` 存在时继续遵守同一编译单元法域/版本一致性。
- `StaticRoute.edge_sequence` 非空、有序且允许同一 edge 多次出现；相邻 edge 必须直接
  连通，每次出现继续由路线内下标形成 owner-local occurrence。
- 所有唯一 owner 向量也拒绝重复 occurrence；“唯一 owner”不是让 reader 选择第一个
  或最后一个。writer 只排序无序集合，绝不修复重复或冲突 owner。

## 7. 数值、顺序与 scalar 缺省

FlatBuffers scalar 的 wire 缺省值与显式写入同一默认值不可区分。本契约不把“是否写入
默认 scalar”当作产品语义：

- `Unspecified` enum、`format_version = 0`、宽度档两端同时为零、零限速、零 duration
  和零 occupancy 都由 LaneFlow 语义预检拒绝；
- `offset_milliseconds = 0`、`transition_index = 0`、`priority = 0` 和合法的零坐标按
  数值本身解释，不要求 writer 使用 `force_defaults`；
- 所有 `double` 必须有限，带符号零在进入 Typed AST 时规范为 `+0.0`；其他范围、曲线
  误差、station、offset、停车和 IIDM 规则沿用对应 design/ADR；
- 关系向量只有在本文明确写作“顺序”的位置才保留顺序；imports、successors、
  approach/internal edge owner 集合、signal group 集合和 participant class 集合按集合
  解释。reader 与 writer 都拒绝重复，不用静默去重掩盖来源错误；`StaticRoute` 的有序
  edge occurrence 是唯一明确允许重复引用的例外。

## 8. Lowering 与禁止的第二语义层

- `RoadAlignment`、curve segment、corridor element、signal phase state 和其他
  owner-local 值只进入道路编辑 Typed AST / geometry intent，不分配额外 StableId。
- 22 类稳定 table 各产生且只产生一个同名共同声明；所有 wire 字段必须被消费一次，
  或明确只属于来源沿袭/画布诊断。
- schema 不保存 HIR/MIR/LIR ordinal、运行时 handle、目标静态镜像布局、车辆状态、
  mesh、材质、碰撞体或任意扩展 map。
- generated object API 不能成为第二棵 production 领域模型；编译器在 verifier 通过后
  从借用 accessor 做第一遍语义计数，再构造字段私有 Typed AST。

## 9. 演进和验证

- 每张 table 的字段 `id` 从 0 连续分配；union 字段同时占用隐式 type id。字段只能在
  末尾追加或标记 `(deprecated)`，不能改类型、改默认语义、移动/复用 id。
- enum 与 union 判别值显式固定且永不复用；新增持久字段、声明种类、曲线 union 或
  语义变化都提升 `format_version`，并提供显式一次性迁移器。
- G2 必须使用固定 `flatc 25.12.19` 对 Rust、C++、C# 生成物执行 clean regeneration，
  对后继 schema 执行 `flatc --conform`，并证明 production 调用图只从受检
  size-prefixed root 进入且不调用 `_unchecked`。
