# 道路编辑来源与几何编制前端

**文档状态**: Accepted<br>
**最后更新**: 2026-08-31<br>
**适用范围**: 道路编辑状态、有类型道路编辑模型、几何编制前端、程序化生成器接入、
来源持久化编码与 topology/geometry MIR 降阶<br>
**关联文档**: `network-compiler.md`、`compiler-foundation.md`、
`../adr/0020-compiler-owned-static-network-and-static-image.md`、
`../adr/0021-city-simulation-game-traffic-foundation.md`、
`../adr/0023-road-editing-state-and-phased-network-replacement.md`、
`../adr/0025-checked-canonical-network-and-shared-static-network.md`、
`shared-static-network.md`

## 1. 合同

道路编辑来源按模块保存为 size-prefixed FlatBuffers，并通过同一有类型道路编辑模型进入
编译器。该来源格式未发布，不建立旧 Geometry JSON、旧 FlatBuffers 版本、名称别名、
迁移器或隐藏 fallback。

## 2. 产品使用路径

生产优先级固定为：

1. 可视化编辑器创建、修改并保存道路；
2. 游戏初始化或官方工具通过程序化生成器创建道路；
3. importer 把外部道路转换为同一道路编辑模型；
4. 官方 SDK 允许受检工具读写该模型。

不支持玩家或第三方把手工来源文本作为主要编制界面。道路审阅使用画布、实体属性和
语义差异，不以原始文本 diff 作为产品完成条件。

## 3. A → C 的功能切片

### 3.1 A 阶段：建造、拆除、整体替换

- 已建道路始终允许后续修改；
- 用户编辑的是候选道路编辑状态，当前路网在候选通过前保持不变；
- 鼠标拖动、曲线调整和预览不触发完整编译；用户“确认建造”后执行一次完整编译、
  LFCA 后发射检查和共享静态路网候选构建；失败不产生部分道路或半更新关系；
- 成功结果是新的不可变路网修订；首版可在宿主明确维护暂停时切换；
- 可编辑城市存档只保存已经与 Runtime 修订共同提交的道路编辑状态；working/candidate
  不保存。只从发布 LFCA 启动且没有对应道路编辑状态的世界是 runtime-only，可通过已认证
  LFCA asset reference 重载但不能进入道路编辑；发布资产要允许编辑，必须同时提供可重新
  编译并核对到同一修订的 committed 道路编辑状态，并先由 #302 用重编译 exact LFCA 原子
  rebase root/source/diff-base binding。可编辑 session 在内存保留当前 exact base LFCA 供后续
  LFSD 发射，存档仍不保存该临时 base；
- “修改现有道路”和“删除后新建道路”是两个显式操作，前者可保持逻辑道路稳定键，
  后者必须获得新键；
- UI 不要求在已建道路上暴露原始控制柄，允许以整体替换交互完成修改。

### 3.2 C 阶段：候选调整与影响预览

- 从当前道路走向定义创建可拖拽候选；
- 在提交前分别展示道路编辑来源差异和规范路网/LIR 影响差异；
- 未改变语义的实体保持稳定身份，真实拆除/新建显式改变身份；
- 取消或验证失败不改变当前道路；
- 复用 A 的候选编译、验证和原子修订切换，不建立第二条修改路径。

#296 的 A 阶段只冻结稳定实体/属性位置，并为 #298 的规范 LIR 差异提供确定输入；它不
实现 `RoadEditingSourceDiff`。C 阶段需要的 authoring-only 差异（例如控制点改变但规范
LIR 未改变）由独立的后继来源差异能力 [#345] 拥有；不能把
#298 的 LIR 差异冒充完整编辑来源差异。

## 4. 道路编辑状态的最小内容

本节描述可持久化内容的形状，不授予任意工作副本进入城市存档。现行保存入口只读取活动
Runtime 的 `CommittedNetworkSource`；本节只约束其中可编辑的
committed `RoadEditingState` 变体。runtime-only 的 `PublishedLfcaReference` 同样是合法来源，
但不携带 authoring payload，因此不适用下列 `RoadEditingState` 必须保存内容。

必须保存：

- 模块、命名空间、稳定道路/区段/声明键和显式导入；
- 当前道路走向定义、横断面、车道、连接、路口和已支持静态规则；
- schema 字段固定的单位、显式配置档和重建当前道路所需的参数；
- 来源沿袭及程序化生成后的实际落地结果；
- 画布对象和编译来源实体之间的稳定关联。

不得保存为来源权威：

- HIR/MIR/LIR ordinal、共享静态路网内部布局或运行时 handle；
- 车辆、信号时钟、停车占用等每世界状态；
- Adapter mesh、材质、碰撞体和物理表现；
- 全部交互历史或无限 undo 日志。

## 5. 有类型模型与编译边界

编辑器、生成器和 importer 必须产生同一字段私有、受检的道路编辑模型。模型只表达
前端语义和来源选择，不自行拥有全局 topology/geometry normalization。

```text
道路编辑状态的版本化编码
  -> 有界解码与结构校验
  -> 有类型道路编辑模型 / Road Editing source module
  -> shared checked module admission
  -> typed AST -> HIR -> topology/geometry MIR
  -> validated canonical LIR
```

程序化生成器在同进程内可以直接构造有类型模型，但进入可发布编译或城市存档前必须
物化为同一版本化道路编辑状态，并保存实际结果与来源沿袭。匿名 AST 继续只允许非发布
测试。

现行 API 提供第一方 Rust 的字段私有、有类型
构造与写入能力，使游戏初始化生成器不必直接操作 generated wire table。C++/C# 等宿主
可以从同一公开 `.fbs` 生成绑定，但完整跨语言 SDK、引擎事务封装和编辑器 UI 不并入
#296；它们在后继交付中复用相同字段语义和 production buffer。第一方 Rust 构造面、
FlatBuffers writer 与编译器 reader 必须共享验证规则，不得形成第二套可接受值域。

## 6. 几何权威

道路编辑状态 v1 只保存直线和三次 Bé塞尔曲线。圆弧、螺旋线及其他 importer/generator
原语必须在写入 v1 前转换为这两种 segment；第一方转换器必须按所选 2/5/10 cm B1 档
执行固定网格观测并在超限时拒绝转换。转换报告由调用者持有，不进入 `.lfre`、内容摘要或
#296 编译器输入；该观测不构成原始 primitive 到 cubic 的连续硬保证。
v1 不保存其原始 primitive 语义。后继若增加 curve union，未发布 B1 也必须提升来源格式
版本、拒绝旧版本并 clean-regenerate；只有已经产品确认并发布的存档语义才由当次
产品设计决定是否交付迁移。MIR 按 ADR 0022 的配置档执行 stationing、offset、确定性细分
与直接检查，LIR
只保留规范 `f32` 折线及派生静态语义。

保存走向定义不意味着 UI 必须提供控制柄，也不意味着 Runtime、Spatial 或 Adapter
获得曲线权威。表现层可以细分 mesh 或平滑显示，但不能改变规范长度、station、拓扑或
静态身份。

## 7. 来源位置与诊断

新来源位置不再默认等于文本行列。每条可诊断声明至少能定位到：

- 来源模块和来源文档；
- 稳定实体或 owner-local 关系；
- 字段/属性路径；
- 可选画布对象、控制段或选择范围；
- 解码损坏时的有界字节范围。

编译器诊断返回稳定代码、严重度和上述结构化位置。编辑器负责把位置映射为画布选择与
属性面板；CLI 可以打印实体键、属性路径和字节范围，但不伪造不存在的行列。

## 8. 协作与局部编辑

来源组织必须同时支持：

- 按城市区域或逻辑模块拆分、加载和提交；
- 在同一模块内按稳定实体识别语义变化和冲突；
- 不因数组顺序、编码器遍历顺序或文件重排改变稳定身份；
- 原子保存模块及其导入/关系更新，失败不产生半写状态。

物理编码不必直接成为多人协作协议，但必须保留稳定实体/属性定位，使后继
[#345] 的 `RoadEditingSourceDiff` 能比较 authoring-only 改动；当前合同包含模块级读取、
编译和保存，不隐含该 diff engine。模块边界保证一次小改动不解析或重写整个城市。

## 9. 来源编码决策与评估

### 9.1 已选择的 production 编码：按模块保存的 FlatBuffers 来源缓冲区

生产来源编码使用 LaneFlow 自有的**道路编辑来源缓冲区**
（Road Editing Source Buffer）`LF-ROAD-EDITING-SOURCE-v4`：

- 一个逻辑来源模块恰好对应一个 size-prefixed FlatBuffer；
- FlatBuffers 根表固定为 `RoadEditingSource`，文件标识符固定为 ASCII `LFRE`；
- 根表包含唯一模块头和按稳定声明种类分组的有类型向量；
- 城市项目/存档以多个模块 blob 组成模块图，不建立一个必须整体读取的全城来源文件；
- 宿主项目/存档负责跨模块保存事务和 blob 定位，LaneFlow 不在该线格式中发明第二个城市存档
  容器或文件系统布局。

选择的主要原因不是二进制体积，而是产品需求的组合：C++、C#、Rust 等预期工具链可从
同一 `.fbs` schema 生成有类型读写代码；整数、枚举和浮点值不再经过文本数字词法；
编译器可先对借用字节执行有界 verifier，再通过只读 accessor 直接预检和降阶，不构造
整模块 wire 对象图。道路编辑器仍在 LaneFlow 有类型模型上修改数据，保存时重建当前
有界模块；FlatBuffers 的只读随机访问不被误写成“直接在生产 blob 上任意编辑”。

### 9.2 精确物理编码

production 编码使用 FlatBuffers 标准 size prefix 和 file identifier，不再增加 LaneFlow 自定义记录
framing。完整来源文档按下列顺序编码；整数均为无符号小端：

```text
u32_le flatbuffer_byte_len = total_document_byte_len - 4
u32_le root_table_uoffset
4 bytes file_identifier = ASCII "LFRE"
byte[...] size-prefixed FlatBuffer 的其余内容
```

根表 `RoadEditingSource` 的 `format_version:uint` 必须精确为 `4`。其它值失败关闭。约束如下：

- 输入至少能覆盖 size prefix、root offset 和 file identifier；
- `flatbuffer_byte_len + 4` 必须用 checked arithmetic 计算并精确等于输入长度，禁止截断
  和 trailing bytes；
- 在 FlatBuffers verifier 前先检查 `SourceBytesPerModule`、调用点剩余
  `SourceBytesTotal`、size prefix 和 `LFRE`；
- framing 检查后，以受边界保护的标量探针读取 root field 0 并拒绝旧版本；完整
  `size_prefixed_root_with_opts` 成功后才访问其余字段。不得调用 `_unchecked` 入口；
- 完整 size-prefixed buffer 是 #315 `sourceDocumentDigest` / `sourceRecordByteLen` 的
  精确来源字节；文档键由必需的 `ModuleHeader` 保存并经前端验证。

FlatBuffers verifier 必须先证明全部被访问的 offset、vector、table、string 和 union
结构安全；其成功只建立 wire 结构可信度，不替代 LaneFlow 对版本、身份、基数、数值、
引用和领域规则的语义验证。

### 9.3 schema 形状

#284 的 Road Editing v4 来源字段及 LFSM 4 属性路径增量，统一登记于
[`traffic-runtime-right-of-way-policy.md` §4.4](traffic-runtime-right-of-way-policy.md#44-lfsm-4-来源编码与闭合)，
包含具名策略局部成员的 CanonicalSetOrdinal 规则。W2 将两个正式前端、共同
AST/HIR/MIR/LIR 和来源投影接入该合同；当前 LFRE wire 为 4。

schema 路径为 `schemas/road-editing/v4/road-editing.fbs`。
字段级领域语义由同目录 README 与本设计共同冻结。`.fbs` 是精确
wire shape 的机器事实源；生成的 wire 类型只存在于私有、`publish = false` 的生成绑定
边界，不进入 LaneFlow 公共 API、HIR/MIR/LIR 或 Adapter API。编译器在 verifier 成功后
借用 generated view，先完成语义预检，再构造字段私有的有类型道路编辑模型 / Typed
AST；不调用 FlatBuffers object API 把整模块 unpack 为第二棵 owned 对象树。

schema 遵守以下闭合规则：

- 根表固定为 `RoadEditingSource`，且 `module_header:ModuleHeader`、
  `road_alignments:[RoadAlignment]`、**24** 个可构造稳定声明向量与
  `conflict_zone_regions:[ConflictZoneRegion]` 为 required；field id 连续，
  `canonical_frames`、`conflict_zones`、`participant_streams` 与
  `conflict_zone_regions` 分别为 id 25、26、27、28，`right_of_way_policy_sets` 为 id 29。单位由
  `*_meters`、`*_radians`、`*_seconds`、`*_milliseconds` 等精确字段名固定，不保存会与
  字段语义竞争的全局 `Units` table；不使用 reflection、
  FlexBuffers、nested FlatBuffer、动态 schema 或 RPC；
- `RoadAlignment` 保存道路走向，具有模块内稳定编辑键但不属于 Identity v1、不分配
  `StableId128`。LFRE wire 4 的 24 个稳定声明 vector 对应 Identity kind `1..=24`。
  `RoadCorridor` 以 alignment key 和 station 区间引用走向，避免在每个走廊复制完整曲线；
- 不使用尚未在预期 C++、C#、Rust 组合中形成共同稳定基线的 vector-of-union；只有
  每个 `CurveSegment` table 内的普通 `CurveSegmentGeometry` union；
- 当前 exact B1 schema 的每个 table 字段显式分配连续 `id`，enum 与 union discriminant
  显式固定数值；这些编号只绑定当前 exact `format_version`，不提前形成跨版本 no-reuse
  兼容承诺；
- 必需 string、table、struct 和 vector 使用 `(required)`。只有确实区分“缺失”和零值的
  `random_seed` 使用可选 inline `OptionalU64`；新增策略数值/枚举使用 optional scalar，
  语义预检要求显式提供，零值不能由缺字段补齐。Movement 方向可省略。旧 scalar 仍按
  值解释，旧 enum 的 `Unspecified` 哨兵仍拒绝；
- 整数按语义使用 `uint32`、`int32`、`uint64` 等原生整数类型；坐标、长度、速度、时间
  和曲线参数使用 `double`，并继续执行有限值、范围、单位和规范 `f32` 量化检查；
- authoring key 使用受长度上限约束的 UTF-8/ASCII string；owner-scoped 引用以
  `owner-key>...>local-key` 保存完整 owner tuple，每段仍独立受 token 上限，完整引用受
  schema README 的派生上限；键集合可以直接使用
  FlatBuffers string vector，因为读取视图借用原 buffer，不再需要只为避免 Protobuf
  解码分配而发明 `PackedAsciiKeyList`；
- owner-local relation、相位状态、曲线段和其他不分配 StableId 的值嵌在 owner table
  下。曲线使用 `CurveProgram` 的 `CurveSegment` table vector；每个 segment 以普通 union
  字段承载 `LineSegment` 或 `CubicBezierSegment`，段索引只用于 owner-local 属性定位；
  后继增加曲线类型必须提升来源格式版本，不能原地改变 union；
- schema 不允许递归 table 图；模块级目标引用保存稳定 local key，owner-scoped 目标保存
  完整 owner-key tuple；不把 FlatBuffers
  table offset 当成领域身份或跨声明引用。

当前 LFRE wire 4 用于实体声明的顶层有类型 vector 覆盖以下 24 类 Identity v1 实体：
`RoadCorridor`、`RoadSection`、`AuthoringLane`、`LaneEdge`、`Junction`、`Movement`、
`ManeuverPath`、`ManeuverGate`、`WaitingZone`、`StopLine`、`SignalGroup`、`SignalController`、
`SignalPhase`、`ParkingFacility`、`ParkingSpace`、`LaneGroup`、`FacilityBand`、
`ParticipantClass`、`AccessRule`、`VehicleProfile`、`CanonicalFrame`、`ConflictZone`、
`ParticipantStream` 和 `RightOfWayPolicySet`。
上述实体对应 Identity kind `1..=24`，其中 21/22/23 分别为
`ConflictZone` / `CanonicalFrame` / `ParticipantStream`。
来源格式可以用较高层 road/cross-section intent 生成其中部分声明，但任何最终稳定实体
都必须具有 Identity v1 要求的显式、持久 ASCII authoring key；数组位置、table offset
和几何都不能替代稳定身份。

根表使用 `LFRE` 与 size-prefixed `RoadEditingSource` envelope，绑定
`format_version = 4`、Road Editing `frontendVersion = 4`、Identity registry revision 4、
LFCA 5、LFSM 4 与 LFSD 4。`RightOfWayPolicySet`（kind 24）由根表 field 29 承载；
五张新增表的字段和来源路径见路权策略实施合同 §4.4。
field id `0..=17` 的含义及其余字段如下：

| field id | required vector            | 目标语义                                                 |
| -------: | -------------------------- | -------------------------------------------------------- |
|       18 | `parking_facilities`       | `ParkingFacility`；`parking_areas` 禁止出现且不是 alias  |
|       19 | `parking_spaces`           | 可选 `parking_facility` 归属、显式 entry/exit 与泊位几何 |
|    20–25 | 同编号向量                 | `lane_groups` 至 `canonical_frames`                      |
|       26 | `conflict_zones`           | 稳定 `ConflictZone` 声明，显式 key 与 Junction 引用      |
|       27 | `participant_streams`      | 稳定 stream、Junction、ManeuverPath 与有序 passages      |
|       28 | `conflict_zone_regions`    | owner-local 可选 2.5D region；空向量表示 headless        |
|       29 | `right_of_way_policy_sets` | 具名策略、共同法规值及证据、间隙、流/门规则四类局部成员  |

LFRE 4 中停车与冲突相关 table 的 field id 与类型语义固定为：

```text
ParkingFacility:
  0 parking_facility_key:string required
  1 canvas_selection:string optional
  2 virtual_capacity:uint
  3 virtual_entries:[ParkingLaneAnchor] required
  4 virtual_exits:[ParkingLaneAnchor] required

ParkingSpace:
  0 parking_space_key:string required
  1 parking_facility:string optional
  2 entry:ParkingLaneAnchor required
  3 exit:ParkingLaneAnchor required
  4 geometry:ParkingSpaceGeometry required
  5 canvas_selection:string optional

ConflictZone:
  0 conflict_zone_key:string required
  1 junction:string required
  2 canvas_selection:string optional

PathAnchor:
  0 kind:PathAnchorKind
  1 gate:string optional
  2 boundary_index:uint
  3 path_edge_index:uint
  4 progress_meters:double

ConflictPassage:
  0 conflict_zone:string required
  1 entry:PathAnchor required
  2 exit:PathAnchor required

ParticipantStream:
  0 participant_stream_key:string required
  1 junction:string required
  2 maneuver_path:string required
  3 passages:[ConflictPassage] required
  4 canvas_selection:string optional

ConflictZoneRegion:
  0 conflict_zone:string required
  1 canonical_frame:string required
  2 min_y:double
  3 max_y:double
  4 ring_xz:[Vec2F64] required
  5 canvas_selection:string optional
```

`PathAnchorKind` 的 source code 为 `0 Unspecified / 1 Gate / 2 EdgeBoundary / 3 Interior`；
compiler 分别映射到 LFCA anchor kind `0/1/2`。Gate 只允许 gate reference；EdgeBoundary
只解释 boundary index；Interior 只解释 path edge index 与严格正的 progress，所有不适用
string 必须缺失、不适用 scalar 必须为规范零。`Vec2F64` 的 member 0/1 分别为 x/z。
`ConflictZoneRegion.ring_xz` 必须包含 `3..=256` 个点；该上限在模型构造和 raw LFRE
预检时先于 ring 分配或几何 lowering 检查。`ParkingFacility.virtual_entries` 与
`virtual_exits` 分别是以 `(LaneEdge StableId128, progressMillimetres)` 为键的集合；来源
`double` 进度量化到毫米、引用解析后重复的完整锚点失败关闭。
这些 table/field/member 的 LFSM container code 与可达 property path 由
[`portable-canonical-artifact.md` §4.1](portable-canonical-artifact.md#41-bindings-与来源池登记)
唯一登记，schema declaration order 或生成语言 enum 不得另行改号。

因此当前 LFRE wire 4 根表包含 24 个可构造稳定声明向量；`conflict_zone_regions` 是
空间 owner-local 记录向量，不分配独立稳定身份。所有向量继续 required，语义上允许空。
`ParkingSpace.parking_facility` 不进入泊位身份。每个 conflict passage 只保存 zone 与
entry/exit `PathAnchor`；admission Gate 从同一 ManeuverPath 的 Gate 序列派生，不进入
来源或 LFCA wire。region 保存 zone、canonical frame、`min_y`/`max_y` 与 `ring_xz`；
几何只能辅助验证与表现，不能生成或改写行为 passage。

声明、owner-local 地址与 property path 必须逐项投影到
[`portable-canonical-artifact.md`](portable-canonical-artifact.md) 的 LFSM 4 role
13–16 / 30–32
登记。生成的 Rust/C++/C# binding 与 known vectors 必须由同一 exact schema 和固定命令
clean-regenerate。

v1 字段所有权进一步固定为同模块 owner tree：`RoadCorridor.elements` 唯一拥有有序
`RoadSection`/`FacilityBand` 横断面成员，`RoadSection.authoring_lanes` 唯一拥有有序
`AuthoringLane`，`SignalController.signal_phases` 唯一拥有有序 `SignalPhase`；child table
同时显式保存同模块 parent reference，并与 owner vector 精确互证。这些
稳定实体保持独立顶层 table，owner-local `CorridorElement`/`SignalPhaseState` 才嵌套。
道路区段车道的 `LaneEdge` 几何从 alignment 与横断面派生，路口 internal edge 则必须在
同一 `LaneEdge` table 提供 `explicit_geometry`。这样 wire 不重复公共声明，也不把
物理 vector 顺序误作 owner 或身份。

owner-qualified 地址必须继续穿过共同 Typed AST/HIR，不能在 reader 末端退化回旧的
`module namespace + stable_key`。compiler-private 共同表示使用以下逻辑形状；
实现可用等价的受计量 interner/ordinal 优化布局，但不能拼接伪 key：

```rust
struct TypedAstEntityAddress {
    owner_local_keys: Box<[Arc<str>]>, // module-scoped kind 为空，最多 3 项
    local_key: Arc<str>,               // 原始 sibling-local identity key
}

struct DeclarationHeader {
    entity_kind: EntityKind,
    source_address: TypedAstEntityAddress,
    identity_local_key: Arc<str>,
    location: SourceLocation,
}

struct OwnedEntityReference<K> {
    module_namespace: Arc<str>,
    target_address: TypedAstEntityAddress,
    location: SourceLocation,
}
```

每种 HIR symbol table 以 `(target module, TypedAstEntityAddress)` 查找，不以 local key 单独
查找。owner kind 按 `RoadCorridor -> RoadSection/FacilityBand`、
`RoadCorridor -> RoadSection -> AuthoringLane/LaneGroup`、
`Junction -> Movement -> ManeuverPath -> ManeuverGate/WaitingZone` 和
`SignalController -> SignalPhase` 的固定父先子后顺序解析。typed address 只负责找到声明和
解析 parent；最终 Identity 必须按 `EntityKind::required_tags()` 在 Identity v1 registry
登记的完整 tag 顺序构造，owner anchor 只是其中一项。例如 `Movement` 还包含 directed
entry/exit approach keys，`ManeuverPath` 还包含 Movement、entry/exit edge StableId。owner
path 的来源拼写不得直接哈希为产品身份。Synthetic/current 的模块级声明映射为空 owner
path，既有身份与诊断排序不变。
地址 component、共享 backing 和 symbol-key capacity 全部进入 string/scratch/live-byte
账本；不得用重复路径字符串规避计量。

`AuthoringLane` 与 `FacilityBand` 的宽度都使用 required inline
`LinearWidthProfile { start_width_meters, end_width_meters }`。两端有限、非负且不能同时
为零；在对应 corridor station 区间内只做线性插值。插值的固定求值顺序、横断面中心
偏移公式、reference station 参数化、B1 固定采样与失败关闭条件由 ADR 0022 第 6 节
唯一冻结；实现不得把 reference curve 的接受树套到 taper。零端点表示 member 在该 corridor
边界开始或结束，同一 lane 身份不能穿过零宽边界；如需拓扑延续，必须使用另一稳定
lane/edge 并显式连接。v1 不接受自由 offset 曲线或第二份 lane/facility 中心线。

v1 `AccessTargetKind` 只包含 LaneEdge、LaneGroup、RoadSection 与 ManeuverPath；数值 5
保留且无效，第一方 writer/UI 不暴露 FacilityBand target。imports、successors、junction
approach/internal、controller signal group、phase state group 与 access participant class
以及停车设施每个角色内的完整虚拟锚点等全部 set-like vector 在 reader/writer 或共享
lowering 边界拒绝重复。路线边序列不再进入编制来源
（ADR 0029）；同一边的多次有序出现只存在于运行时 `register_route` 输入。
namespace、local key 与 alignment key 禁止
`::`，owner-qualified reference 使用 token 中禁止的 `>` 分隔完整 owner key 链；
qualified reference 必须恰好一个 `::`。每段 token 仍受 53-byte 上限，完整引用按 schema
README 的 270-byte 派生上限检查。完整引用只是借用 source bytes 的语法拼写，不作为
`SingleStringBytes` 或第二份字符串驻留；拆出的 namespace/owner/local-key component 分别
计入 `SingleStringBytes`、`StringItemCount`、`TotalStringBytes` 和阶段 live bytes。owner
reference 禁止跨模块，普通关系目标才可引用 imports。

模块沿袭与编译选项摘要的精确前像、direct 固定检查值、键/引用语法、字段所有者、
有序/集合向量及 scalar 缺省语义由 schema 同目录 `README.md` 冻结。writer 与 reader
必须共用这些规则，不能让 generated builder 的“字段可省略”变成另一套 LaneFlow 语义。

### 9.3.1 拓扑/几何 lowering 闭包

每个 `LaneEdge` 恰好承担以下一种角色：

| 角色              | owner 与来源                                                                                  | geometry                                                         | successor                                 |
| ----------------- | --------------------------------------------------------------------------------------------- | ---------------------------------------------------------------- | ----------------------------------------- |
| section-derived   | 被恰好一个 `AuthoringLane` 引用；不属于任何 `Junction.internal_edges`                         | `explicit_geometry` 必须缺失；由 alignment、station 与横断面派生 | 可以声明不穿越 junction 的普通延续        |
| junction-internal | 被恰好一个 `Junction` 拥有；不被任何 `AuthoringLane` 引用；至少被该 junction 的一条 path 使用 | `explicit_geometry` 必须存在                                     | `successors` 必须为空；转换只从 path 派生 |

同时满足、两者都不满足、多 owner、孤立 internal edge 或重复 topology authority 均失败
关闭。角色不进入 `CanonicalIdentity`，合法角色变化不暗改稳定键。

横断面在 alignment 的参考方向上按 `RoadCorridor.elements`、每个 section 的
`authoring_lanes` 从左到右求值。reference lane 中心线精确等于 reference line；其他
lane/facility 在每个 station 的 lateral center 由全部左侧宽度与自身半宽的累计唯一
导出。线性宽度因此同时表达常宽、加宽、收窄和常见 taper，不增加独立横向控制点。
offset 曲线继续使用 ADR 0022 的同一 B1 固定采样细分。`Forward` lane 保持 reference
参数方向；`Backward` lane 完成 offset、station 强制点与规范 `f32` 量化后，把最终点列
反转为行驶方向。Traffic length、successor、StopLine 和停车 progress 都按最终行驶方向
解释。每个 FacilityBand 产生一条不可遍历 offset 中心线和一行规范
`facility_band_geometries`，但不产生 Traffic length、successor 或路线可遍历性。

每条 `ManeuverPath` 的 entry、全部 internal edge 与 exit 必须解析到同一个
`CanonicalFrame`。一个 internal edge 的 frame 从所有使用它的 path 的 entry/exit
approach frame 集合唯一导出；集合不为单值、任一 approach 无 geometry、显式曲线不在
该 frame，或不同 path 对同一 internal edge 得出冲突 frame，均失败。internal edge 不
保存独立 frame 字段，俯视相交也不能推导 path、movement 或 frame。

对 `Movement.junction = J` 拥有的每条 path，entry 与 exit 都必须属于
`J.approach_edges`，path 中每个 internal occurrence 都必须属于 `J.internal_edges`，且
同一路径不得重复同一 internal edge。`J.internal_edges` 还必须精确等于该 junction
全部 path 的 internal edge 并集；少声明、跨 junction 使用和没有 path 使用的孤立成员
都失败关闭。`J.approach_edges` 还必须全部是 section-derived edge，并与完整编译单元的
junction-internal owner map 全局不相交；本 junction 或另一 junction 的 internal edge
都不能充当 approach/entry/exit。实现复用角色与引用预检已经需要的索引，以声明和关系
occurrence 总数的线性时间完成，不另做几何搜索。

### 9.4 模块、局部编辑与协作

v1 的物理局部性边界是**模块**，不是 FlatBuffers table：

- 城市按区域或逻辑所有权拆为多个不超过现有来源上限的模块 blob；加载/编译只读取所选
  模块及其显式导入闭包；
- 编辑器在内存中的有类型模型上工作，鼠标拖拽不反复序列化，也不使用实验性 reflection
  在 blob 内调整 string/vector 大小；保存或候选编译时才重建当前有界模块；
- 编译器直接遍历 verifier 通过的只读 buffer，先预检、再构造 Typed AST，不因编译而
  unpack 一棵 generated object graph；
- 顶层按声明种类分组的有类型向量保留 stable identity 与 typed property path；
  FlatBuffers table offset、原始 bytes diff 与 #298 的 LIR diff 都不能替代 [#345] 的
  `RoadEditingSourceDiff`；
- 同一声明向量的物理顺序不进入语义。来源 v1 要求 `RoadAlignment` 和模块级身份种类按
  kind/module local key 唯一；owner-scoped 种类只要求同一直接 owner 下 sibling local key
  唯一，不同 parent 可以复用同名 key。道路编辑来源地址因此是“模块 namespace + 根
  table kind + 完整 owner key 链 + local key”，与 Identity v1 的 parent scope 一致但不
  取代 `CanonicalIdentity`。官方 writer 对 owner-scoped 根向量按 owner key tuple、再按
  local key 逐段字典序形成全序；有序 owner 向量保持产品顺序，无序引用集合按解析后的
  namespace/owner tuple/local key 排序。reader 接受任意物理顺序，在模块内先闭合 owner
  tree，再进入共同身份和最终规范顺序；
- A 阶段保存整个候选模块并原子替换；C 阶段复用相同模块事务和实体身份，但其来源差异
  引擎属于独立后继能力，不由当前 compiler/LIR 合同隐式提供。

若未来实际城市证明单模块重写成为可观察瓶颈，应先以 workload 证据调整模块粒度；
只有模块边界仍不足时，才重新设计实体日志、增量容器或数据库存储。当前格式不提前承担
这些复杂度。

### 9.5 版本、未知字段与摘要

当前组合是 `format_version = 4`、Road Editing `frontendVersion = 4`、LFCA 5、LFSM 4 与
LFSD 4；来源前端版本和 LFRE wire 版本分别精确校验。

- 来源描述符固定使用 `SourceLanguage::RoadEditingSource = 2`、
  `SourceLanguage::as_str() = "road-editing-source"` 和 `frontendVersion = 4`；
- `format_version = 4` 是本 exact schema 的精确版本，不是“最低兼容版本”。
  其它版本在语义读取前失败关闭，不提供迁移。
  该 exact schema 未作为稳定 public format 发布。任何可能让旧 bytes 被不同解释的 wire 或语义变化都必须
  再提升 `format_version` 及对应 frontend/geometry semantics code。internal family
  至少保持可在语义读取前识别的 `LFRE + root format_version(id:0,uint)` envelope；若连该
  envelope 也改变，则分配新 file identifier。新未发布版本可在重新设计后重排其他
  field/enum/union 编号并 clean-regenerate，不承担 append/no-reuse 债务；
- 只有后续经产品确认并发布的存档版本，才由当次产品设计冻结
  append/deprecated/no-reuse、跨版本 `flatc --conform` 和是否提供一次性迁移；当前内部格式
  只要求 exact schema/codegen 再现、版本拒绝与同版本 known vectors；
- 旧工具可能在结构 verifier 后看到新版本，但必须在任何 LaneFlow 语义 lowering 和
  规模相关分配前拒绝；不能依靠未知字段忽略完成跨版本 round trip；
- 攻击性输入或错误 writer 在 `format_version = 4` 下附带的未知 vtable slot 语义上无效
  并被忽略，但仍计入来源字节、verifier apparent size 与精确来源摘要。根表无
  `static_routes` 字段。其它未冻结空位不在本合同另设拒绝表；
- FlatBuffers bytes 不是 LaneFlow 规范语义编码。`sourceDocumentDigest` 绑定收到的精确
  字节以便重放和完整性比较；字段布局、向量重排或不同语言 builder 的差异可能只造成
  保守 cache miss；#298 规范路网影响差异、路网修订和实体身份继续由规范模型/LIR
  派生，authoring-only 来源差异属于独立后继能力；
- 官方 writer 的稳定向量排序只用于减少无意义改写，不冒充跨语言、跨版本 canonical
  serialization 保证。

### 9.6 不可信输入与内存边界

编译器只接受借用的完整 size-prefixed buffer，按以下顺序失败关闭：

1. 在分配前检查来源模块/累计字节余额、最小长度、size prefix 的 checked exact length
   和 `LFRE` file identifier；
2. 从受 framing 约束的根表执行有界版本探针，非 `4` 立即拒绝，旧文档无需携带
   v4 新增的 required vector；再使用下述固定公式从当前 `CompileLimits` 与调用点剩余预算导出 `VerifierOptions`，再
   执行 `size_prefixed_root_with_opts`；不使用 crate 默认值；
3. verifier 成功后检查唯一 exact version `4`；其它值在 LaneFlow 语义读取和规模相关
   分配前拒绝；
4. 对借用 view 执行第一遍语义预检：必需值、enum/union、字符串、24 类可构造声明与
   owner-local 集合基数、有限数值、引用键字节和 checked 总量；
5. 只有第一遍证明主要规模和保守工作集上界可容纳后，才构造字段私有 Typed AST、身份
   索引和后续 IR。source bytes 与 FlatBuffers view 都是调用方借用，不产生整模块 wire
   decode heap；v1 不以逐个 `Vec` / `Box` / `Arc` 的 allocator 精确会计作为接入前提；
6. 任一失败不修改 `CompilationUnitBuilder`，后续合法模块仍可使用同一 builder/Compiler
   实例。

令 `S` 为已经通过 exact-length 与 `SourceBytesPerModule/SourceBytesTotal` 检查的完整
size-prefixed 输入字节数，`R` 为调用点剩余 `TypedAstRecordCount`：

```text
max_depth         = 5
max_tables        = checked_add(2, R)
max_apparent_size = checked_mul(S, 16)
ignore_missing_null_terminator = false
```

深度 5 来自 `RoadEditingSource -> RoadAlignment -> CurveProgram -> CurveSegment ->
geometry payload` 的 schema 最长路径。只有 root 与 `Provenance` 两张 wire table 不进入
Typed AST record 计数；`ModuleHeader` 延续 compiler foundation 既有规则消费一条记录。
LFRE 4 中每个显式 `Movement.turn_direction`（包括 Straight/0）还产生一个字段来源
记录，因此逻辑记录数为物理 table occurrence 数减二，再加方向字段出现数。该来源
同时消费一条 `RelationOccurrenceCount`；builder、原始预检与累计模块准入使用同一
口径，不能把 `max_tables` 的物理表检查当作全部逻辑记录预算。
verifier 访问的其余每个 table occurrence（包括 curve/union payload、owner-local table、
停车子表和 IIDM 子表）在第一遍预检中各消费一个
`TypedAstRecordCount` 候选，不增加公开 limit 维度。`max_tables` 命中使用现有
`CompileLimitExceeded(TypedAstRecordCount)`；固定 schema depth 或 16 倍 apparent-size
命中使用新 `InvalidRoadEditingSource` 的闭合
`VerifierDepthExceeded/VerifierApparentSizeExceeded` violation。checked 公式溢出映射到
同一对应 violation。一般结构损坏使用 `MalformedWire`，不得把攻击性 DAG 或损坏伪装成
领域值错误。

16 倍 apparent-size 上限是 v1 的固定 DAG 放大政策，不是测量结果：官方 writer 不得在
不同逻辑字段 occurrence 之间主动复用 table/vector/string offset，FlatBuffers 自身的
vtable dedup 例外；第三方 writer 可使用同一 schema，但超出 16 倍的高度共享 DAG 作为
资源不兼容输入拒绝。必须用正常 Rust/C++/C# writer fixture、边界与边界加一证明该
公式；不能绕过 verifier，也不能新增第二套 FlatBuffers offset parser。错误 trace、
Typed AST、身份索引和诊断的逐 allocation 生命周期及 allocator 证据由 [#374] 独立闭合，
不阻断本契约的 canonical LIR 下游消费。

#### 9.6.1 最小资源护栏 v1

#296 的生产验收只要求以下最小资源护栏：

- `SourceBytesPerModule` / `SourceBytesTotal` 在 verifier 和领域分配前失败关闭；
- verifier 固定 depth、table 与 apparent-size 上限；
- 声明、引用、关系、字符串、几何点和主要 IR 记录使用既有 `CompileLimits` 上限；
- 整体工作集采用故意保守、可审计的逻辑上界，不要求等于普通 Rust allocator 的实际
  requested bytes；
- 全部规模乘加使用 checked arithmetic，无法表示或超过上限时在对应大区块分配前拒绝；
- 候选失败不提交，builder 可重试。

普通 Rust allocator 下，逻辑计数器本身不等价于完全精确的实际内存硬上限。`Vec`
capacity / 扩容共存、`Arc` header 与 DST padding、阶段精确 scratch/live、失败诊断、
exact-minus-one 和正式 allocator/P100 协议统一由 [#374] 交付；若产品最终需要实际内存
硬隔离，#374 还须评估受控 arena、专用 allocator 或进程隔离。#374 阻断最终外部开放与
总体生产切换 Gate，不阻断 #298、#345 或 compiler-private HIR/LIR 结构收敛。

官方 FlatBuffers Rust 生成代码中的 `unsafe` 不作为淘汰条件，但必须隔离为可审计的生成
绑定边界：

- `flatc` 生成 Rust 源码放入私有、`publish = false` 的
  `laneflow-road-editing-wire` package；该 package 只承载固定生成物和必要 Cargo
  metadata，不成为产品层、公共 SDK 或可扩展前端接口；
- 道路编辑生产调用图中的 first-party 手写 crate 继续继承 workspace
  `unsafe_code = forbid`；固定生成路径必须带 `@generated` 标记。ADR 0024 另行冻结的
  `laneflow-format` staged backing 只读 mmap 小岛不属于道路编辑 wire 调用图，并由同一
  xtask 以精确 manifest、source 和表达式单独扫描；
- `laneflow-compiler` 只调用 verifier 驱动的安全 root/accessor，不重导出 wire 类型，
  其 `unsafe_code = forbid` 也使 `_unchecked` 入口无法被调用；
- CI 以固定 `flatc` 重生成，只规范化平台原生 CRLF/LF 后要求 canonical bytes
  byte-for-byte clean diff，同时拒绝精确生成路径以外的 `unsafe` 与
  `allow(unsafe_code)`；生成器/runtime 升级必须重新走依赖审计、fuzz 和 exact-head
  外部审阅。

### 9.7 来源位置与公共入口

现有文本 `SourceSpan` 不再承担所有来源形状。诊断位置收敛为字段私有的闭合
和类型：

```rust
pub enum SourceLocation {
    Text(SourceSpan),
    RoadEditing(RoadEditingSourceLocation),
}

pub struct RoadEditingSourceLocation {
    /* document identity + subject + optional property/canvas ordinals + optional wire range */
}

pub enum RoadEditingDocumentIdentity {
    Input { /* expected source document key supplied outside wire */ },
    Verified { /* module namespace + equal verified wire source document key */ },
}

pub enum RoadEditingSubject {
    ModuleHeader,
    RoadAlignment { /* RoadEditingSourceAddress */ },
    Declaration { /* RoadEditingSourceAddress */ },
    OwnerLocal { /* owner address + relation kind + RoadEditingRelationOccurrence */ },
    Wire { /* root vector kind + u32 physical index + table kind */ },
}

pub enum RoadEditingRelationOccurrence {
    OrderedProductOrdinal(u32),
    CanonicalSetOrdinal(u32),
}

pub struct RoadEditingSourceAddress {
    /* interned module namespace + root table/entity kind + owner-key tuple + local key */
}

pub struct RoadEditingPropertyPath { /* 1..=4 RoadEditingPropertyStep */ }
pub enum RoadEditingPropertyStep {
    TableField { /* known table kind + schema field id */ },
    StructMember { /* known struct kind + member id */ },
    UnionVariant { /* known union kind + discriminant */ },
}

pub struct RoadEditingLocationContext {
    /* interned namespace/key components, property paths, canvas selection keys and display bytes */
}
pub struct RoadEditingStringOrdinal(u32);
pub struct RoadEditingPropertyPathOrdinal(u32);
pub struct RoadEditingCanvasSelectionOrdinal(u32);
pub struct RoadEditingByteRange { /* checked u32 start + length */ }
```

以上字段均私有并由受检构造器建立。道路编辑来源地址与 `CanonicalIdentity` 明确分层：
前者保存来源 v1 的模块/kind/完整 owner-key tuple/local key，后者按 Identity v1 registry
的 `EntityKind::required_tags()` 完整 tag 序列形成产品稳定身份；parent StableId 只是在
适用 kind 中的一项。`RoadEditingPropertyPath` 是最多四步的闭合叶属性路径，不接受任意
字符串或向量下标；例如 lane 结束宽度表示为
`AuthoringLane.width_profile -> LinearWidthProfile.end_width_meters`，Bezier 第二控制点的
x 分量表示为 `CurveSegment.geometry -> CubicBezierSegment -> control_2 -> Vec3F64.x`。
关系 occurrence 已由 subject 携带，故属性路径不重复表达 owner 向量下标。有产品顺序的
owner vector 使用原产品 ordinal；无序唯一集合使用解析/规范化 target reference 后按
`namespace + owner tuple + local key` 排序得到的 canonical ordinal。可解析但目标未知的
reference 仍按其规范拼写参与排序；语法本身损坏时只能使用 `Wire` structural fallback，
物理 vector index 不得进入 `OwnerLocal`、稳定诊断排序或后继来源差异。

每次 `add_road_editing_module` 在 verifier 前先接收已经由 `RoadEditingModuleInput::try_new`
验证的 `expected_source_document_key`，再建立唯一可变的 candidate
`RoadEditingLocationContext`；
verifier 失败时以 `RoadEditingDocumentIdentity::Input` 和受检 wire trace 定位，不能冒充
已读出的 module identity。verifier 成功并验证 wire `source_document_key` 与 expected key
逐 byte 相等后，后续位置改用 `Verified`，语义预检再按规范遍历补齐来源地址的 namespace/key components、
闭合属性路径和 `canvas_selection`。完整 owner-qualified wire reference 只从已计量的借用
source bytes 解析，不作为第二份字符串驻留；每个 intern component 继续受 53-byte 单项
上限，component、路径步和索引容量全部计入现有 total-string/live-byte 预算。位置只保存
有类型 `u32` ordinal，不为每条诊断复制路径或字符串。在 context 第一次进入 builder、
`DiagnosticBundle` 或 source-map owner 前冻结为 `Arc<RoadEditingLocationContext>`，冻结后
不再可变。add 成功后，
`CompilationUnitBuilder` 接管该模块共享 context handle，并分配与 admitted module 记录绑定、
单调且不复用的 builder-local context index；
后续 add 失败时，candidate context 直接移入 `DiagnosticBundle`，已提交模块的 context 只
clone `Arc` handle，builder 保留原 handle 并可继续使用；build 失败使用同一共享规则，
成功时 `ValidatedSourceMapInput` 接管完整 handle 集合。Arc allocation、strong handle 数、
handle vector capacity 和失败 bundle 的 retained bytes 全部预收费并进入 live-byte 账本，
禁止深拷贝 context。跨模块位置先保存 context index、
再保存 context-local ordinal，二者都由受检构造器建立；build 的规范模块重排只移动携带
该 index 的 module record，不重编号 context，也不把 index 当成排序键。因此候选 Typed
AST/module 或 builder 释放后仍可解析。两种返回对象只公开只读解析 accessor；ordinal
只是一轮编译内的紧凑地址，不得持久化、参与摘要或直接用于诊断排序。

`DiagnosticBundle` 返回后，其 candidate context、handle vector 和 retained capacity 转为
caller-owned，不再计入下一次 compiler 调用；仍由 builder 同时持有的 admitted context
allocation 继续且只在 builder ledger 中计一次。调用方保留旧 bundle 并重试不会让 builder
重复承担旧 bundle handle/capacity，但测试必须覆盖“旧 bundle 仍存续 + 同 builder 新候选”
生命周期测试，并分别报告 compiler ledger 与 caller-retained bytes。

verifier 失败使用 `Input` document identity、`Wire` subject、field/vector trace 和可选
`RoadEditingByteRange`；该
range 只允许结构损坏诊断，必须先证明位于输入内。成功验证后的领域诊断使用稳定
alignment/declaration/owner-local subject 与闭合 property，不依赖物理 vector index，也
不反推伪精确 byte range。全局诊断排序先沿用 module/document 规范顺序，再按
`Text=0/RoadEditing=1`、subject kind、稳定 key bytes、owner-local occurrence、
属性路径的语义 step 序列、诊断代码/载荷排序；`Wire` fallback 在 subject kind 后按 root
vector kind、physical index、table kind、属性路径 step、byte range 排序。比较器解析
ordinal 后比较实际 step/key bytes，不能比较分配顺序。这样文本来源现有顺序不变，二进制
来源的稳定语义诊断也不被输入数组置换改变。

首版 production Rust 入口冻结为一个原子借用路径：

```rust
pub struct RoadEditingModuleInput<'a> {
    /* required expected source document key + bytes + optional display source */
}

impl<'a> RoadEditingModuleInput<'a> {
    pub fn try_new(
        expected_source_document_key: &'a str,
        source_bytes: &'a [u8],
        display_source: Option<&'a str>,
    ) -> Result<Self, InvalidRoadEditingModuleInput>;
}

impl CompilationUnitBuilder {
    pub fn add_road_editing_module(
        &mut self,
        input: RoadEditingModuleInput<'_>,
    ) -> Result<&mut Self, DiagnosticBundle>;
}
```

`try_new` 只按 source-document token/53-byte 规则验证 expected key，不读取、哈希或验证
source bytes。builder 调用在同一个 `&mut self` 事务中取得剩余预算、检查 size
prefix/identifier、运行 verifier，并要求验证后的 `ModuleHeader.source_document_key` 与 expected key 逐 byte
相等，随后执行语义预检、降阶并进入 #315 共同 admission。expected key 只提供损坏输入
定位和成功后的绑定，不覆盖 wire module/document identity。不得公开预构造 generated
wire view、裸 Typed AST、可伪造 module descriptor 或绕过 builder 余额的
`RoadEditingModule::decode`。程序化生成器、editor 和 importer 必须先用官方
schema/writer 物化相同来源缓冲区，再进入此唯一编译路径；是否持久化由产品事务决定，
但可发布编译必须保存实际接受的 bytes 与来源沿袭。

`laneflow_compiler::road_editing` 还必须公开以下第一方 Rust **编制模型与
writer**；它们是程序化生成器/编辑器可使用的 production API，但不是编译器 Typed AST。
production **编译输入**仍只有上面的借用 bytes，generated wire adapter 和验证后
Typed AST/HIR/MIR 仍只在 compiler-private / 私有 `laneflow-road-editing-wire` 边界：

```rust
pub struct RoadEditingSourceModuleBuilder<'limits> { /* private */ }
pub struct RoadEditingSourceModule { /* private */ }
pub enum RoadEditingDeclaration {
    RoadCorridor(RoadCorridorInput),
    RoadSection(RoadSectionInput),
    AuthoringLane(AuthoringLaneInput),
    LaneEdge(LaneEdgeInput),
    // schema 其余 18 个稳定声明的一一对应、闭合 variant
}
pub struct RoadAlignmentInput { /* private */ }
// 每个 schema 稳定 table 各一个同名 `*Input` 字段私有类型；owner-local 值也有闭合类型。

impl<'limits> RoadEditingSourceModuleBuilder<'limits> {
    pub fn new(
        header: RoadEditingModuleHeader,
        accuracy: GeometryAccuracyProfile,
        direction: GeometryDirectionProfile,
        limits: &'limits CompileLimits,
    ) -> Result<Self, DiagnosticBundle>;
    pub fn add_alignment(
        &mut self,
        value: RoadAlignmentInput,
    ) -> Result<&mut Self, DiagnosticBundle>;
    pub fn add_declaration(
        &mut self,
        value: RoadEditingDeclaration,
    ) -> Result<&mut Self, DiagnosticBundle>;
    pub fn finish(self) -> Result<RoadEditingSourceModule, DiagnosticBundle>;
}

pub struct RoadEditingSourceWriter<'limits> { /* private */ }
pub struct OwnedRoadEditingSourceBuffer { /* Vec<u8> storage + checked start */ }

impl<'limits> RoadEditingSourceWriter<'limits> {
    pub const fn new(limits: &'limits CompileLimits) -> Self;
    pub fn write(
        self,
        module: RoadEditingSourceModule,
    ) -> Result<OwnedRoadEditingSourceBuffer, DiagnosticBundle>;
}

impl OwnedRoadEditingSourceBuffer {
    pub fn as_bytes(&self) -> &[u8];
    pub fn retained_capacity_bytes(&self) -> usize;
}
```

每个 `*Input` 只提供字段完整的 `try_new(...)` 与只读 accessor；可选字段用有类型
`with_*` 消费式方法，不公开字段、动态 property map、generated table、
`FlatBufferBuilder` 或未受检构造器。builder 在每次 add 时检查 local key/value、集合
重复、同模块 owner 和现有 `CompileLimits` 计数；`finish` 检查模块内 owner tree 完整性，
模块级身份按 kind/local key 唯一，owner-scoped 身份只按直接 owner/local key 唯一。
writer 消费自身与 module，按第 9.4 节的完整 owner-key tuple/local key 和规范无序引用
原地排序；该 tuple 顺序是全序，不能依赖稳定排序保留调用者输入次序。writer 禁止主动
复用不同 occurrence 的 string/vector/table offset，
然后一次生成 size-prefixed buffer。返回对象直接拥有 `FlatBufferBuilder::collapse` 的
storage 与 start，不复制尾部有效 bytes；model 与排序 scratch 在返回前释放，保留容量
通过 accessor 可计量。

writer 与 reader 共同调用私有 `road_editing::rules` 中的 token、reference、scalar、
width/profile 和模块内集合规则，保证同一字段值域只有一份实现。writer 的早期检查只
改善第一方工具反馈；`add_road_editing_module` 仍对收到的 bytes 完整重验，并且与
`build` 共同构成普通跨模块引用、全编译单元 topology/geometry、Identity 与 LIR 语义的
唯一权威。身份 owner tree 已由模块自身闭合，不允许延后到跨模块绑定。writer
不尝试验证尚未加载的导入目标，也不能让“writer 生成”成为可信绕过条件。

第一方 writer 以 `SourceBytesPerModule` 和主要计数上限约束输出规模，但不在 #296 内宣称
其 model、排序 scratch、FlatBuffers runtime 私有容量与返回 buffer 已共享一个逐分配硬上限。
writer 在分配 output storage 前以
schema table/vector/string、每一次 schema emission `push/align` 最多 8-byte padding 和
vtable 上界，以及 `finish_size_prefixed` 的 root offset、`LFRE` identifier、size prefix 和
final minimum alignment 逐项计算
checked wire upper bound，超过 `SourceBytesPerModule` 即失败。storage
只按该上界预分配一次；实际 size prefix 必须等于 `as_bytes().len() - 4`。边界测试
必须覆盖 wire upper、来源字节和候选失败原子性；write 峰值、返回 buffer retained
capacity 和该 buffer 随后进入 reader 时的精确组合峰值由 #374 继续校准。

固定 FlatBuffers runtime 在 output storage 之外还持有 field-location 与 vtable-dedup 等
私有容量。#296 v1 只要求 pinned runtime、来源字节上限和保守工作集上界；这些容器的
capacity、扩容瞬间和 allocator instrumentation 证明由 #374 统一拥有。runtime 版本变化
仍须重新审计其 verifier/writer 安全边界，并由 #374 决定是否重测精确资源证据。

### 9.8 未选候选 B：有界记录流 + Protocol Buffers payload

候选 B 使用 16-byte LaneFlow 固定头和逐记录 `u32_le` 长度；第一条 payload 是唯一
`ModuleHeader`，后续每个稳定声明各占一个 `proto3` message，owner-local 值留在 owner
record。编译器先证明全部 record range，再逐记录 decode、验证、降阶和释放，避免单一
Protobuf 根对象常驻。

它的主要收益是 C++/C#/Rust 工具链成熟、generated Rust 不需要 `unsafe`，并且 record
byte range 天然适合损坏定位。主要代价是 LaneFlow 要长期维护 FlatBuffers 不需要的
自定义 framing；`prost` decode 会为 string、bytes、repeated/nested message 建立 owned
对象，因此还需要限制 message 形状、证明每记录分配上界，或引入 packed key/parallel
array 等 wire 专用结构。现有探针没有完成 FB/PB 同 workload 的定量性能比较，因此不能
宣称 B 已被实测证明更慢或峰值更高；产品负责人选择 size-prefixed FlatBuffers 的依据是
整体产品/架构适配，并
明确接受受控 generated `unsafe`，不以未测性能差距作为事实。

### 9.9 候选比较与依赖结论

先按 ADR 0023 已冻结的八项产品需求逐项比较；“通过（有代价）”表示产品能力成立，但
代价必须进入实际 workload 与治理证据，不能被当成零成本：

| 产品需求                                           | size-prefixed FlatBuffers                                         | 有界记录流 + Protobuf                                          | 单一根对象 Protobuf                      | JSON                                                 |
| -------------------------------------------------- | ----------------------------------------------------------------- | -------------------------------------------------------------- | ---------------------------------------- | ---------------------------------------------------- |
| 官方编辑器、初始化生成器、SDK、importer 跨语言读写 | 通过；官方 C++/C#/Rust codegen                                    | 通过；各语言 Protobuf codegen                                  | 通过                                     | 通过，但每端仍需 DTO/Schema 语义层                   |
| 原生整数、枚举、向量与有界集合                     | 通过；固定宽度 scalar/struct/vector                               | 通过，但为压低 decode 对象放大需 packed key 等规避             | 通过                                     | 不通过原生数值目标；重新引入文本词法                 |
| 区域/模块加载与稳定实体级差异                      | 通过；模块 blob + 24 类可构造声明向量；来源 diff 为独立后继能力   | 通过；模块记录流 + 稳定记录                                    | 通过，但整模块 decode；diff 仍需独立模型 | 通过模块拆分，但 bytes diff 无产品价值               |
| 精确版本、演进与一次性迁移                         | 通过；外部 exact version、显式 field id、`--conform`              | 通过；外部 exact version、field number/reserved                | 通过                                     | 通过，但 shape/数字/缺省策略由自建层承担             |
| 损坏存档失败关闭与分配前资源预检                   | **通过且最匹配**；size exact + verifier limits + zero-object view | 条件通过；自建 framing 后仍需证明每记录 decode allocation      | 不通过当前硬预算门槛；先形成整模块对象图 | 条件通过；parser/token/container 分配账本复杂        |
| 实体/属性/画布诊断与损坏定位                       | 通过；稳定实体/属性 + verifier trace/byte position                | 通过；稳定实体/属性 + record byte range                        | 通过，但损坏通常只有 message 范围        | 文本行列强，但不是产品主要诊断界面                   |
| Rust 编译器与 C++/C# 宿主、依赖和维护成本          | 通过（有代价）；单一官方项目，生成 `unsafe` 需窄审计边界          | 通过（有代价）；Rust `prost` + 各宿主 runtime + 自定义 framing | 通过（有代价）                           | 依赖成熟，但 LaneFlow 自建严格 lexer/schema 叠加明显 |
| 代表性城市保存、加载、编译和协作成本               | 预计通过，必须实测；读取无 wire heap，保存重建当前模块            | 预计通过，必须实测；每记录 decode/释放和 framing 有固定成本    | 编译峰值预期最差                         | 已有复杂度证据；人工可读收益不进入产品矩阵           |

矩阵中只有 FlatBuffers 在不增加自定义 record parser 的同时满足不可信输入预验证和编译
读取零对象图分配。产品负责人已接受它的两项真实代价——模块保存时重建 buffer、生成
Rust 绑定含 `unsafe`——并选择 A；仍须以绝对 workload 证明资源预算，不把选择写成
未经测量的 PB 性能劣势。

| 候选                          | 产品收益                                                              | 代价 / 风险                                                           | 结论                     |
| ----------------------------- | --------------------------------------------------------------------- | --------------------------------------------------------------------- | ------------------------ |
| size-prefixed FlatBuffers     | C++/C#/Rust 共用 IDL；verifier 后零对象图读取；原生标量；模块随机访问 | 编辑保存需重建模块；生成 Rust 绑定含 `unsafe`，必须固定工具并隔离审计 | **已选择：production A** |
| 单一根对象 Protocol Buffers   | 跨语言成熟、mutable object API 直接、原生整数                         | Rust 编译器先形成整模块 decode 对象图，峰值和分配难与硬预算对齐       | 拒绝该组成方式           |
| 有界记录流 + Protobuf payload | 可逐记录预检/释放；Rust 生成代码不含 `unsafe`                         | 自建 framing、逐记录 decode 分配、packed key 等 schema 规避增加复杂度 | 未选择                   |
| 严格 JSON                     | 工具普及、人工可读、文本 diff 直接                                    | 产品不依赖人工文本；数字词法、shape、来源行列和分配账本复杂           | 未选择                   |
| Cap'n Proto                   | 随机访问、规范化能力和 traversal limit                                | 参考实现以 C++ 为主，Rust/C# 实现由不同作者维护且未被参考作者审阅     | 拒绝                     |
| rkyv / postcard               | Rust 侧简单或高性能                                                   | 跨语言 SDK 不成立；rkyv schema/format feature 变化会破坏旧数据        | 拒绝                     |
| CBOR / MessagePack            | 二进制、实现广泛                                                      | 无统一强类型 IDL/codegen，仍需自建 shape、enum、演进与跨语言约束      | 拒绝                     |
| 全自定义二进制                | 可精确控制布局和预检                                                  | 长期 parser、codegen、跨语言实现与安全维护成本最高                    | 拒绝                     |

候选事实依据：

- [FlatBuffers 官方文档：Rust 的受检不可信输入入口](https://flatbuffers.dev/languages/rust/)
- [FlatBuffers 官方文档：schema、required/optional、file identifier 与字段类型](https://flatbuffers.dev/schema/)
- [FlatBuffers 官方文档：schema 演进与 `flatc --conform`](https://flatbuffers.dev/evolution/)
- [FlatBuffers 官方文档：C++、C#、Rust 等支持矩阵](https://flatbuffers.dev/support/)
- [Protocol Buffers 官方文档：序列化不是 canonical](https://protobuf.dev/programming-guides/serialization-not-canonical/)
- [Cap'n Proto 官方文档：其他语言实现状态](https://capnproto.org/otherlang.html)
- [rkyv 文档：schema/format option 兼容边界](https://docs.rs/rkyv/latest/rkyv/#compatibility)

Rust runtime 固定为 crates.io `flatbuffers = 25.12.19`：Apache-2.0、MSRV 1.51，满足
本仓库 Rust 1.98；只启用默认 `std`，不启用 `serde`/`serialize` 或 reflection 依赖。
该 crate 的 `.cargo_vcs_info.json` 与官方 annotated tag `v25.12.19` 都解析到 commit
`7e163021e59cca4f8e1e35a7c828b5c6b7915953`，因此 codegen 也固定该 tag 的 `flatc`，不
使用同版本号但来自其他 commit 的后继 release。生成 Rust 源码提交仓库，正常
`cargo build` 不下载或执行 `flatc`。CI 使用以下官方 asset 和 SHA-256：

| 平台    | release asset                       |     bytes | SHA-256                                                            |
| ------- | ----------------------------------- | --------: | ------------------------------------------------------------------ |
| Windows | `Windows.flatc.binary.zip`          | 1,412,094 | `fff9445c9db907227bc64b54cc98743084c4949282aa4e576cff6a955724ddc8` |
| Ubuntu  | `Linux.flatc.binary.clang++-18.zip` | 2,750,934 | `50c1915deeeb714f2a05c8ec795bd1af898d251a62e2774067703b29188efc90` |

下载后先校验 ZIP 摘要，再解压并要求 `flatc --version` 精确输出
`flatc version 25.12.19`。仓库根目录的精确生成 argv 固定为：

```text
flatc --rust -o crates/laneflow-road-editing-wire/src/generated schemas/road-editing/v4/road-editing.fbs
flatc --cpp -o target/road-editing-codegen/cpp schemas/road-editing/v4/road-editing.fbs
flatc --csharp -o target/road-editing-codegen/csharp schemas/road-editing/v4/road-editing.fbs
```

只有 Rust 的 `road-editing_generated.rs` 提交仓库；C++/C# 输出只用于跨语言 fixture 与
schema probe，不进入 source tree。命令不得增加 `--gen-object-api`、`--rust-serialize`
或 reflection 选项。CI 在空输出目录运行同一 Rust argv，只把平台原生 CRLF/LF 统一为
LF 后 byte-for-byte 比较生成物；不得规范化空白、标识符或其他源码字节。CI 还以同一固定
binary 生成 C++/C# probe；未发布 B1 后继版本不运行跨版本 `--conform`。

依赖版本与生成器身份固定；实际新增依赖时仍须运行
`cargo metadata`、固定版本 `cargo-deny`、workspace tests，并按
`dependency-security.md` 复核许可证、来源、RustSec/Dependabot 和分发影响；`flatc` 与
runtime 必须保持同一固定版本，生成物 clean regeneration 属于数据格式 G3 必需证据；
跨版本 `flatc --conform` 只服务后继已经产品批准的兼容承诺。未选的 B/C 不进入
production 依赖或实现。

## 10. 测试、性能与 workload

以下矩阵是已选择 size-prefixed FlatBuffers 编码的实现与验证要求：

- size prefix、`LFRE`、版本、checked exact length、截断、trailing bytes、错位 offset、
  非法 vtable/vector/string/union 的 known vectors；唯一接受 `format_version = 4`，
  其余未知槽仍忽略；
- verifier 的 `max_depth`、`max_tables`、`max_apparent_size` 以及 required presence、未知
  enum/union、非法数字、字符串/集合边界和 owner-local 关系的正负测试；
- 24 种可构造稳定声明的 identity/reorder/insertion known vectors，
  确保 vector 顺序不改变
  StableId、LIR 或 #298 规范路网影响差异，并逐 kind 与现有
  `EntityKind::required_tags()` registry known vectors 对齐；
- programmatic Rust writer → bytes → production reader → LIR；C++/C# binding 生成继续由固定
  `flatc` 检查覆盖，跨语言 writer → Rust reader fixture 在 B1 publication 或外部 SDK
  promotion 前补齐，不阻断内部使用；
- line/cubic/taper 的 scalar-dual 逐 bit known vectors，覆盖大坐标、九种档位阈值
  `-1/0/+1 ULP`、source-offset canonical weld、水平 cusp/近 cusp、regularity depth/node 上限；
- 模块独立加载、导入闭包、实体属性诊断、候选失败不污染、失败后恢复和同实例重复编译；
- 模块级重复 key/同 owner sibling 重复拒绝、不同 owner 同名 child 接受、完整 owner-key
  来源地址、嵌套 struct/union 叶属性路径、owner-local 来源地址，以及
  失败 `DiagnosticBundle` / 成功 `ValidatedSourceMapInput` 在候选释放后仍能解析
  `canvas_selection` 的生命周期测试；
- 无序 relation 物理排列不改变 canonical occurrence/诊断/source-map，以及 caller 保留旧
  `DiagnosticBundle` 后同一 builder 重试的计量/生命周期测试；
- 任意字节 fuzz / differential harness 在 B1 publication 或外部输入开放前补齐；内部能力
  保留 verifier limits、known corrupt vectors 和 production 调用图无 `_unchecked` 的
  确定性检查，旧 JSON bytes 和任意新版本仍失败关闭；
- 固定 `flatc` 的 Rust 生成物 clean-diff、C++/C# probe、生成路径外除 ADR 0024 已登记的
  单一只读 mmap 调用外无 `unsafe` / `allow(unsafe_code)`；跨版本 `--conform` 只在后续 promotion/publication 决策已经建立
  兼容承诺时成为门禁；
- `SourceBytesPerModule`、`SourceBytesTotal`、`DeclarationCount`、字符串、几何点、
  verifier table/depth/apparent size 与 `CompilerControlledLiveBytes` 的边界/边界加一。

道路编辑入口与其余官方前端共同受
`LF-COMP-SINGLE-NETWORK-1M-v2` 计数、来源字节、暂存、编译器控制存续内存和 file-backed
制品上限约束。资源测试必须从现行 `LF-ROAD-EDITING-SOURCE-v4` writer 生成输入，并报告
实际模块 bytes、声明/关系/几何点计数、scratch、retained capacity 与峰值内存；不得把已删除
的 v1 workload、旧 JSON fixture 或单次观测继续当作当前机器事实源。

[`road-editing-source-semantic-seed-v1.json`](../reference/road-editing-source-semantic-seed-v1.json)
和 [`road-editing-source-reference-machine-v1.json`](../reference/road-editing-source-reference-machine-v1.json)
只保留为历史研究证据，不进入现行 schema、fixture、配置档或生产读取路径。任何新的道路编辑
专项 workload 必须基于 v4 输入另分配标识符，并与其生成器、exact bytes 和测量环境一起冻结。

## 11. 兼容和清理边界

- Geometry JSON v1 尚未发布且未被选择，不建立读取兼容、双写、自动迁移或隐藏 fallback；
- 现行实现不得从 #332 草稿带入 `module/geometry/json.rs`、JSON shape parser、旧 JSON
  Schema 或只服务该 production 格式的校准/证据生成逻辑；历史语义种子只存在于
  test/research 证据，不能被 production feature 或 crate 依赖。几何求值、MIR lowering
  和与格式无关的 known vectors 转移到现行
  typed/wire fixture，不保留隐藏 JSON fallback；
- 不新增或保留 `.proto`、Protobuf framing、`PackedAsciiKeyList` 或 `prost`；改为新增
  `.fbs`、固定 `flatc` 再现入口和私有 generated wire package；
- ADR 0022 的曲线/折线误差档位独立保留并在新来源模型上重新验证；
- 当前只承诺 ADR 0022 B1 内部完整验证语义：schema 不进入 publication，不承诺长期存档
  兼容，也不把 2/5/10 cm 写成连续硬上限；后继 certified continuous-bound 语义或 B1
  production promotion 都重新走
  产品设计判断，而不是在同一语义版本内静默换算法；
- 共同模块接入、来源摘要和资源维度如需适配非文本来源，必须明确
  兼容的抽象语义；不得复制第二套 admission；
- #298 只消费 LIR，不得读取道路编辑状态；#302 只消费可信修订和迁移描述符，不得成为
  编辑器模型权威。


[#345]: https://github.com/illusion-tech/laneflow/issues/345
[#374]: https://github.com/illusion-tech/laneflow/issues/374
