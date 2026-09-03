# 道路编辑来源缓冲区 v4 Schema

**状态**: 生产。`format_version = 4`；根表 field id 连续至 id 29。
其它版本的 `LFRE` 在语义读取前失败关闭。<br>
**格式标识**: `LF-ROAD-EDITING-SOURCE-v4`<br>
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
两者必须同时满足。该 schema 是仓库内唯一现行道路编辑来源线格式；它仍是内部格式，
不构成 1.0 前的长期存档兼容承诺。

## 1. 文档边界

- 一个逻辑来源模块恰好对应一个标准 size-prefixed FlatBuffer；根表是
  `RoadEditingSource`，file identifier 是 ASCII `LFRE`，默认扩展名是 `.lfre`。
- `format_version` 必须精确为 `4`。其它值都在任何 LaneFlow 语义
  lowering 或按规模分配前拒绝。
- `module_header`、`road_alignments`、24 个可构造 Identity 声明向量和 owner-local
  `conflict_zone_regions` 都必须在 wire 中存在；向量可以为空。根表无
  `static_routes` / `parking_areas` 字段；现行停车设施字段是
  `parking_facilities`。
  宿主以多个模块 blob 和显式导入图组成城市项目。
- 完整 size-prefixed bytes 是 `sourceDocumentDigest` 与 `sourceRecordByteLen` 的输入；
  buffer bytes、table offset 和 vector index 都不构成领域身份或语义等价依据。

## 2. 键、引用与显示关联

### 2.1 Token

所有 namespace、document key、稳定声明 key、road alignment key、`kind_id`、有向
approach key、引用中的每个 token 和 `canvas_selection` 都先受调用点的
`max_single_string_bytes`（当前 `53` bytes）约束。稳定标识 token 另外满足：

```text
首字节: ASCII 字母或数字
后续字节: ASCII 字母、数字、.、_、:、/、-
```

值大小写敏感，不 trim、不 case-fold、不 Unicode normalize。namespace、声明 local
key 和 `road_alignment_key` 都不能包含保留分隔符 `::`；单个 `:` 仍是合法 token
字符。

### 2.2 有类型引用

模块级身份实体引用只接受：

```text
local-key
namespace::local-key
```

owner-scoped 身份实体引用必须携带从模块根 owner 到目标的完整 key 链：

```text
owner-key>...>local-key
namespace::owner-key>...>local-key
```

`>` 不属于 token 字符；链中每段分别按 token 和 53-byte 上限验证。限定引用必须恰好
包含一个 `::`，两侧非空；namespace 必须等于当前模块或出现在 imports。当前 v4 最深
目标是 `Junction > Movement > ManeuverPath > ManeuverGate/WaitingZone`，所以完整引用
最多 4 个 key token；连同可选 53-byte namespace 和分隔符，wire string 的派生硬上限
固定为 `270 bytes`。该例外只适用于已解析为有类型实体引用的 string，普通 string 和
单个 token 仍不得超过 53 bytes。reader 就地拆分并借用完整 wire spelling；它不把最多
270-byte 拼写作为一条驻留 `SingleStringBytes` 对象，拆出的每个 component 才分别消费
53-byte 单项、string item/total bytes 和 live-byte 预算。

被引用实体的种类由字段或伴随的封闭 enum 决定，因而链深固定：`RoadSection` 和
`FacilityBand` 为 `RoadCorridor > local`；`AuthoringLane`/`LaneGroup` 为
`RoadCorridor > RoadSection > local`；`Movement` 为 `Junction > local`；
`ConflictZone` / `ParticipantStream` 为 `Junction > local`；
`ManeuverPath` 为 `Junction > Movement > local`；`ManeuverGate`/`WaitingZone` 为
`Junction > Movement > ManeuverPath > local`；`SignalPhase` 为
`SignalController > local`。模块级身份种类只允许一个 local token。

定义 owner 的字段必须指向当前模块，禁止 `namespace::`：`RoadSection.road_corridor`、
`AuthoringLane.road_section`、`Movement.junction`、`ManeuverPath.movement`、
`ManeuverGate.maneuver_path`、`WaitingZone.maneuver_path`、
`SignalPhase.signal_controller`、`LaneGroup.road_section` 和
`FacilityBand.road_corridor`、`ConflictZone.junction` 和
`ParticipantStream.junction`。一个 owner tree 不能跨模块；其他普通引用可以指向显式
import。`RoadCorridor.road_alignment_key` 只引用当前模块的 `RoadAlignment`，因为道路
走向不是 Identity v1 实体。

### 2.3 Canvas 关联

可选 `canvas_selection` 保存编辑器稳定画布对象键。它进入来源精确摘要和诊断位置，但
不进入 `CanonicalIdentity`、`StableId128`、LIR 语义指纹或运行时镜像。属性面板定位
继续由 `entity reference + typed property path` 派生，不在 schema 中增加任意 map。

## 3. 根表与身份层

`RoadEditingSource` 包含：

1. `format_version` 与唯一 `ModuleHeader`；
2. 显式位置/方向配置档；
3. `road_alignments:[RoadAlignment]`；
4. 按 Identity 可构造种类排列的 24 个有类型声明向量；
5. 不分配 StableId 的 `conflict_zone_regions` 空间记录。

`RoadAlignment` 是可继续编辑的当前道路走向定义。其 `road_alignment_key` 在模块内稳定
且唯一，但它不是静态路网实体，不分配 `StableId128`，也不进入 LIR。它只为
一个或多个 `RoadCorridor` 提供同一规范坐标框架中的参考曲线。

24 个可构造声明向量与 Identity revision 4 种类一一对应：

| 代码 | FlatBuffers table     | 稳定 key 字段            | 所有者 / 身份锚点来源                           |
| ---: | --------------------- | ------------------------ | ----------------------------------------------- |
|    1 | `RoadCorridor`        | `road_corridor_key`      | 当前 module namespace                           |
|    2 | `RoadSection`         | `road_section_key`       | 同模块 `road_corridor` + 唯一 elements 成员     |
|    3 | `AuthoringLane`       | `authoring_lane_key`     | 同模块 `road_section` + 唯一 lanes 成员         |
|    4 | `LaneEdge`            | `lane_edge_key`          | 当前 module namespace；角色不参与身份           |
|    5 | `Junction`            | `junction_key`           | 当前 module namespace                           |
|    6 | `Movement`            | `movement_key`           | `junction` + 两个有向 approach key              |
|    7 | `ManeuverPath`        | `maneuver_path_key`      | `movement`                                      |
|    8 | `ManeuverGate`        | `maneuver_gate_key`      | `maneuver_path`                                 |
|    9 | `WaitingZone`         | `waiting_zone_key`       | `maneuver_path`                                 |
|   10 | `StopLine`            | `stop_line_key`          | 当前 module namespace                           |
|   11 | `SignalGroup`         | `signal_group_key`       | 当前 module namespace                           |
|   12 | `SignalController`    | `signal_controller_key`  | 当前 module namespace                           |
|   13 | `SignalPhase`         | `signal_phase_key`       | 同模块 controller + 唯一 phases 成员            |
|   14 | `ParkingFacility`     | `parking_facility_key`   | 当前 module namespace                           |
|   15 | `ParkingSpace`        | `parking_space_key`      | 当前 module namespace；可选 facility 不参与身份 |
|   16 | `LaneGroup`           | `lane_group_key`         | `road_section`                                  |
|   17 | `FacilityBand`        | `facility_band_key`      | 同模块 corridor + 唯一 elements 成员            |
|   18 | `ParticipantClass`    | `participant_class_key`  | 当前 module namespace                           |
|   19 | `AccessRule`          | `access_rule_key`        | 当前 module namespace                           |
|   20 | `VehicleProfile`      | `vehicle_profile_key`    | 当前 module namespace                           |
|   21 | `ConflictZone`        | `conflict_zone_key`      | 同模块 `junction`                               |
|   22 | `CanonicalFrame`      | `canonical_frame_key`    | 当前 module namespace                           |
|   23 | `ParticipantStream`   | `participant_stream_key` | 同模块 `junction`                               |
|   24 | `RightOfWayPolicySet` | `policy_set_key`         | 当前 module namespace                           |

所有 table 在其根向量中的物理顺序都不进入语义。一个 buffer 只属于一个 namespace，
且全部身份 owner 必须在同一 buffer。来源 v4 要求 `RoadAlignment` 和模块级身份种类在
各自根向量内 local key 唯一；owner-scoped 种类只要求同一直接 owner 下 sibling local
key 唯一，不同 owner 可以合法复用同名 local key。每个 child 的显式 parent 字段必须与
owner 向量精确互证，缺失、重复或冲突 owner 均失败关闭。

道路编辑来源地址固定为“module namespace + table kind + 完整 owner key 链 + local key”。
官方 writer 对 `RoadAlignment` 按 key 排序；模块级身份种类按 local key 排序；
owner-scoped 根向量按完整 owner key tuple、再按 local key 的 UTF-8 bytes 逐段字典序排序。
该 tuple 是全序，不拼接后比较，也不依赖稳定排序或调用者输入次序。有语义顺序的 owner
向量保持产品顺序；无序引用集合先规范引用拼写（当前模块省略 namespace，导入模块使用
qualified form）再按 namespace、owner key tuple、local key 逐段排序。
production reader 接受任意物理顺序并在 HIR/MIR 中闭合完整身份与最终规范顺序。

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
- v4 不保存一张永远固定、却会与字段后缀竞争权威的 `Units` table。距离、角度、速度和
  时间单位由精确字段名冻结为米、弧度、米每秒、秒或毫秒；writer/reader 不能接受
  每文档单位切换，也不能把宿主单位隐式带入来源。
- `geometry_accuracy_profile` 只接受 `Fine2Cm / Balanced5Cm / Compact10Cm`；这些名称
  表示 ADR 0022 B1 的 `2/5/10 cm` 工程目标，不是连续曲线最大误差保证；
  `geometry_direction_profile` 只接受 `Smooth1Deg / Balanced2Deg / Compact5Deg`。
  `Unspecified` 一律无效，不存在隐式默认、任意浮点容差或资源不足自动降级。
- 最终 `SourceModuleDescriptor.frontendOptionsDigest` 对以下精确前像计算一次 SHA-256：

  ```text
  ASCII "laneflow.road-editing.frontend-options.v1\0"
  || geometry approximation semantics fixed code DeterministicTargetV1 = 1: u8
  || geometry_accuracy_profile: u8
  || geometry_direction_profile: u8
  || provenance.frontend_options_digest: [u8; 32]
  ```

  它绑定 B1 算法语义和实际选择的两个几何档位；不进入来源文档摘要或实体稳定身份。

## 5. 道路走向、横断面和车道

- `RoadAlignment.reference_line` 至少包含一个 `CurveSegment`。v1 union 只接受
  `LineSegment` 和 `CubicBezierSegment`；段起点是前一段终点，第一段起点是
  `CurveProgram.start`。所有点必须有限并位于规范 frame 可证明的范围内。圆弧、螺旋线
  和其他外部曲线必须由 importer/generator 在写入前转换为这两种 segment，并在所选
  2/5/10 cm B1 目标下记录固定网格观测误差；该观测不是连续硬保证。v1 不保存原始
  曲线原语语义。
- 每个 `RoadCorridor` 绑定一个当前模块 alignment 和半开 station 区间。第一段从
  `0 m` 开始；同一 alignment 的 corridor 按 station 无重叠、无间隙完整覆盖。
  `end_station_kind = Finite` 时读取严格更大的 `end_station_meters`；末段必须使用
  `AlignmentEnd`，此时 `end_station_meters` 必须为规范 `+0.0` 且不参与含义。
- `RoadCorridor.elements` 的顺序是横断面从左到右的语义顺序；每个引用按 `kind` 指向
  同模块 `RoadSection` 或 `FacilityBand`。每个 section/band 的显式 `road_corridor`
  必须与该向量互证，并在当前模块恰有一个 corridor owner。
- `reference_section` 必须出现在 `elements`；`reference_lane` 必须出现在该 section 的
  `authoring_lanes`，方向为 `Forward`，其中心线精确绑定 reference line。
- `RoadSection.authoring_lanes` 是从左到右的语义顺序；每个 child 的显式
  `road_section` 必须与该向量互证。每个 `AuthoringLane` v1 恰好
  覆盖一个 `lane_edge`，携带线性宽度档、显式方向和可选 lane group。宽度两端都必须
  有限且非负，不能同时为零；在 corridor station 上按端点线性插值。零端点只表达该
  member 在该 corridor 边界开始或结束，不允许用同一 lane 身份穿过零宽边界；与另一
  edge 的拓扑连接仍须显式声明。需要把同一逻辑车道跨多个 corridor span 延续时，使用
  不同稳定 lane/edge 并通过显式 successor 连接，不从左右下标猜测身份。
- `LaneGroup.road_section` 是同模块唯一 owner；被 lane 引用时必须与该 lane 所属 section 相同。
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
  线性宽度 taper 的精确 `d(s)`、`s(t)`、left 向量、B1 固定采样、舍入顺序和失败关闭
  条件只由 ADR 0022 第 6 节定义，reader/writer 不得另选数值公式。

## 6. 路口、控制、停车与准入

- `Junction.approach_edges` 和 `internal_edges` 按集合解释。internal edge 必须由一个
  junction 唯一拥有、具有显式曲线，并至少被一条属于该 junction 的 maneuver path
  使用。每条 path 的 entry、全部 internal 和 exit edge 必须解析到同一
  `CanonicalFrame`；一个 internal edge 被使用的全部 path 也必须从 entry/exit approach
  导出同一个 frame，冲突即失败。internal edge 不另存 frame 字段。
- `Movement.junction` 是唯一 owner；有向 entry/exit approach key 是 Identity v1 字段，
  不从几何或边名推断。对该 Movement 所属 junction `J`，path 的 entry/exit 必须属于
  `J.approach_edges`，每个 internal occurrence 必须属于 `J.internal_edges` 且同一路径内
  不得重复；`J.internal_edges` 必须精确等于其全部 path internal 成员的并集，跨 junction、
  少声明和孤立成员均失败。`ManeuverPath` 的权威边序列固定为
  `entry_edge + internal_edges + exit_edge`。
- 每个 `Junction.approach_edges` 成员必须是 section-derived edge，且与完整编译单元的
  junction-internal owner map 全局不相交。一个 junction 的 internal edge 不能作为本身或
  另一 junction 的 approach/entry/exit；该检查使用现有 edge role/owner index，不从名称
  或几何推断。
- `ManeuverGate.transition_index` 指向路径边序列的一个有效相邻转换。
  `signal_control = None` 时 `signal_group` 必须缺失；`SignalGroup` 时该字段必须存在。
- `WaitingZone.max_occupancy` 必须大于零；entry/release gate 必须属于同一路径且顺序合法。
- `SignalController.signal_groups` 是无序唯一归属集合；每个 module-scoped `SignalGroup`
  必须被恰好一个 controller 引用，但该归属不改变其 module-scoped Identity anchor；
  `signal_phases` 的顺序定义循环。
  每个 phase 的显式 `signal_controller` 必须与该向量互证并在同模块被恰好一个 controller
  拥有，`duration_milliseconds > 0`，其 owner-local
  `states` 对 controller 全部 signal group 恰好各出现一次。
- `ParkingFacility` 可以同时表达显式泊位、虚拟容量与规范 entry/exit anchor；
  `ParkingSpace.parking_facility` 只是可选组织关系，不参与泊位身份。entry/exit progress、
  矩形尺寸、横向偏移和朝向继续遵守已接受停车几何范围。
- `ConflictZone` 与 `ParticipantStream` 是稳定实体；`ConflictPassage` / `PathAnchor` 是
  stream 的 owner-local 行。每个 stream 绑定同一 Junction 的 ManeuverPath，passage
  顺序与 anchor 规范化遵循公共 LFCA 5 合同。`ConflictZoneRegion` 只提供可选 2.5D
  空间区域，不获得行为 authority。
- `ParticipantClass.extends` 形成可选无环单继承。`VehicleProfile.iidm` 只描述当前道路
  机动车执行域，全部数值继续遵守当前 IIDM 范围。
- `AccessRule.target_kind + target_reference` 构成一个封闭有类型目标；参与者类别集合
  非空。v1 只允许 `LaneEdge / LaneGroup / RoadSection / ManeuverPath`；数值 5 保留且
  无效，第一方 writer/UI 不提供 `FacilityBand` target。后继若支持该能力，必须提升来源
  格式版本。`regulation` 存在时继续遵守同一编译单元法域/版本一致性。
- 所有唯一 owner 向量也拒绝重复 occurrence；“唯一 owner”不是让 reader 选择第一个
  或最后一个。writer 只排序无序集合，绝不修复重复或冲突 owner。路线边序列不在来源
  中声明；场景 catalog 0.4 拥有示例边键，并显式携带必填 `policy_selection`，
  运行时只经 `register_route` 编译。策略形状与绑定顺序见
  [人口与回流合同](../../../docs/design/signalized-corridor-population.md#3-catalog-契约)。

## 7. 数值、顺序与 scalar 缺省

策略成员的 priority、三项间隙和两个门枚举采用 optional scalar，领域准入必须
显式提供；零是合法值，缺失拒绝。Movement 的可选方向缺失不等于 Straight。
这些新增字段的完整合同见[路权策略实施合同 §4.4](../../../docs/design/traffic-runtime-right-of-way-policy.md#44-lfsm-4-来源编码与闭合)。

以下旧字段继续按既有规则解释：

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
  解释。reader 与 writer 都拒绝重复，不用静默去重掩盖来源错误。

## 8. Lowering 与禁止的第二语义层

- `RoadAlignment`、curve segment、corridor element、signal phase state 和其他
  owner-local 值只进入道路编辑 Typed AST / geometry intent，不分配额外 StableId。
- 24 个可构造稳定 table 各产生且只产生一个同名共同声明；所有 wire 字段必须被消费一次，
  或明确只属于来源沿袭/画布诊断。
- schema 不保存 HIR/MIR/LIR ordinal、运行时 handle、目标静态镜像布局、车辆状态、
  mesh、材质、碰撞体或任意扩展 map。
- generated object API 不能成为第二棵 production 领域模型；编译器在 verifier 通过后
  从借用 accessor 做第一遍语义计数，再构造字段私有 Typed AST。

## 9. 演进和验证

- 当前 exact B1 schema 的每张 table 字段 `id` 从 0 连续分配，union 字段同时占用隐式
  type id，enum 与 union 判别值显式固定。B1 v1 只授权内部完整验证，尚未进入
  公共 publication catalog；任何会让旧 bytes 被不同解释的 wire/语义变化都必须提升
  `format_version` 及相应 frontend/geometry semantics code，当前 reader 拒绝旧值且不提供
  迁移。internal family 保持 `LFRE + root format_version(id:0,uint)` 可读 envelope；若该
  envelope 也改变，则分配新 file identifier。
- 未发布的新版本经重新设计可以重排其他 field/enum/union 编号并 clean-regenerate，不承担
  append/deprecated/no-reuse 债务。只有产品确认 promotion/publication 后，才由当次
  产品决策冻结跨版本编号兼容、`flatc --conform` 和一次性迁移范围。
- 未来连续硬保证或 B1 production promotion 都必须重新进行产品设计判断；B1 UI/文档
  只能陈述工程目标和观测证据，不能声明 `2/5/10 cm` 连续最大误差保证。
- 实现必须使用固定 `flatc 25.12.19` 对 Rust、C++、C# 生成物执行 clean regeneration；
  Rust clean diff 只允许把平台原生 CRLF/LF 统一为 LF，其他源码字节必须逐字节一致，并
  证明 production 调用图只从受检 size-prefixed root 进入且不调用 `_unchecked`。

## 10. 路权策略来源

根表 id 29 `right_of_way_policy_sets` 为 required vector，可为空。每份策略是
模块级 kind 24 声明，包含共用 RegulationIdentity 值与 Evidence、GapProfile、
StreamRule、GateRule 四种局部具名成员。成员不能跨模块追加，引用的 stream、Gate
和 class 可以来自显式导入模块。成员按 key 排序产生来源 occurrence；策略及成员的
全部已提供字段进入 LFSM 贡献位置，空集合与零数值仍保留。策略 canvas 的 None 和
空字符串分别保留。业务日期和策略启用时机由宿主拥有。

详见[路权策略实施合同](../../../docs/design/traffic-runtime-right-of-way-policy.md)。
