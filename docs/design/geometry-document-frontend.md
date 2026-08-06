# 几何文档前端与拓扑/几何中层表示

**文档状态**: Review（#296 G1 候选；尚未授权 G2）<br>
**最后更新**: 2026-08-06<br>
**适用范围**: `laneflow-compiler` 的几何文档前端（Geometry Document Frontend）、
`GeometryModuleBuilder` / `GeometryModule`、几何来源格式 v1、拓扑/几何中层表示
（Topology/Geometry MIR）与已验证规范低层中间表示（Validated Canonical LIR）降阶<br>
**关联文档**: `compiler-foundation.md`、`network-compiler.md`、
`spatial-geometry.md`、`../adr/0020-compiler-owned-static-network-and-static-image.md`

## 1. 目标与状态边界

本文冻结 #296 的实现输入，不表示对应 Rust 已经交付。#296 G1 通过前不得修改生产
Rust；G2 后的实现必须保持以下边界：

- 几何文档是生产场景的主要编制语言，但不是唯一来源权威；它与合成领域专用语言和
  后继导入前端平级进入共同有类型抽象语法树（Typed AST）。
- 编译器联合拥有拓扑、规范几何、静态规则、身份闭包和确定性降阶；当前
  `InitialTrafficData`、`SpatialRegistry` 或 JSON 对象图不得成为中间表示。
- #315 已交付的私有共同 `TypedAstModule`、原子模块接入、来源文档登记、资源账本和
  规范模块排序是唯一共同入口；本切片不复制这些机制。
- #298 及后继后端只消费已验证规范低层中间表示；本文不定义可移植制品、独立验证器、
  静态镜像、编辑器界面或导入协议。
- 当前 `Traffic v0.10` / `SpatialPackage v0.1` / Core / Spatial 生产路径在 #294 G4
  前保持不变。

## 2. 来源文档与版本

### 2.1 一逻辑模块一文档

几何来源格式 v1 的一个逻辑模块**恰好拥有一份来源文档**。模块内不存在 `include`、
文件片段、目录扫描、glob 或隐式 companion document；跨文件组合只能建立多个逻辑
模块，并通过 `imports` 显式形成权威来源模块图。

因此：

- Geometry module 自身只需要 `ModuleCount`，且与不可变
  `LF-COMP-P100-INITIAL-v1` 的隐式一模块一文档约束兼容；
- 同一编译单元若还包含任何多文档官方模块，调用方必须在构造
  `CompilationUnitBuilder` 前显式选择具备 `SourceDocumentCount` 的 v2 或后继配置档；
- `GeometryModuleBuilder` 不自动选择、升级或替换配置档，也不合成文档数上限；
- 后继若要让一个 Geometry module 拥有多份文档，必须提升几何来源格式版本，并重新
  进入 G1；不能在 v1 中增加 include 语义。

### 2.2 编码与媒体类型

v1 使用严格 UTF-8 JSON：

- 媒体类型为 `application/vnd.laneflow.geometry+json;version=1`；
- 顶层 `geometryVersion` 必须精确为字符串 `"1"`；缺失、旧值、未来值或非字符串均
  失败关闭；
- 不接受 UTF-8 BOM、注释、尾随逗号、重复对象键、非有限数、JSON 之外的转义或
  UTF-16/UTF-32；
- 所有对象是 closed shape，未知字段为错误；可选字段省略与显式 `null` 不等价，除
  非字段表明确允许 `null`；
- 对象字段顺序和普通声明集合的数组顺序不构成语义；曲线段、路径边、路线出现项、
  相位状态等明确标为有序的数组保留顺序；
- 字符串不 trim、不大小写折叠、不做 Unicode normalization。身份键、命名空间、文档
  键和引用继续使用编译器既有 ASCII external-token 契约；面向人的名称可使用 UTF-8，
  但不进入 Identity v1 前像。

文档描述符的 `sourceRecordByteLen` 是输入文档的精确字节数，
`sourceDocumentDigest` 是对同一精确字节序列计算一次 SHA-256 的结果。空白或对象字段
重排可以保持相同 LIR 语义，但会有不同的来源文档摘要；编译器不得先重新序列化 JSON
再计算来源摘要。模块级文档集摘要继续由 #315 的
`LFSOURCE-DOCUMENT-SET` v1 规则从已派生的唯一文档描述符聚合。

### 2.3 顶层封闭形状

```text
GeometryDocumentV1 {
  geometryVersion: "1",
  module: ModuleRecord,
  units: UnitsRecord,
  frames: [FrameRecord],
  roads: [RoadRecord],
  junctions: [JunctionRecord],
  overlays: OverlayRecord
}
```

`frames` 和 `roads` 都必须非空；`junctions` 可以为空，`overlays` 必须存在但其各数组
可以为空。每个 road/frame 引用、stable key、声明和有序关系仍受配置档计数及共同
唯一性规则约束。

`ModuleRecord` 精确包含：

```text
namespace: external-token
documentKey: external-token
imports: [external-token]
provenance: DirectProvenance | GeneratedProvenance
```

`namespace` 是 `authoringNamespaceId`；`documentKey` 是与机器路径无关的稳定来源文档
键。`imports` 是目标 namespace 集合，重复、自导入、未知目标与循环分别复用共同模块图
诊断。`DirectProvenance` 精确为 `{ kind: "direct", description }`；编译器为其记录固定
`generatorBuildId = "laneflow-geometry-direct-v1"`、域分离的空输入/固定选项摘要和
`randomSeed = None`。`GeneratedProvenance` 精确包含
`{ kind: "generated", generatorBuildId, parametersAndInputsDigest,
frontendOptionsDigest, randomSeed, description }`，其中摘要是 64 位小写十六进制，
`randomSeed` 是十进制 `u64` 字符串或 `null`，避免 JSON/JavaScript number 丢失高位。
这些值只记录来源沿袭，不认证文档内容；发布真实性仍由
后继外部描述符承担。两种 `description` 都是受字符串上限约束的可见 UTF-8 文本。
direct 的两个固定摘要前像分别是 ASCII
`laneflow.geometry.direct.inputs.v1\0` 和
`laneflow.geometry.frontend-options.v1\0`；各自直接计算一次 SHA-256，不包含 JSON
空白、显示来源或 compiler build。实现必须为两者冻结已知向量。

`UnitsRecord` v1 必须精确为：

```text
distance: "meter"
angle: "radian"
speed: "meter-per-second"
time: "second"
```

v1 不接受逐字段单位后缀或隐式角度单位。Typed AST 保留原始数字 token 与字段类型，
HIR 在使用前验证有限值、范围和单位并规范化带符号零；不得先转换为 `f32` 再执行
authoring 语义。

### 2.4 来源位置与外部显示来源

解析器为模块、每个声明、稳定键、引用、数值和有序关系成员记录真实的一基
`u32` 行列范围。范围覆盖产生该语义的最窄完整 JSON value；派生实体使用其拥有者和
产生它的关系成员作为 primary/related span。输入超过 `u32` 行列表示或来源字节上限时
在构造相应表前失败关闭。

调用方可以通过 `GeometryDocumentInput` 提供一个未认证的稳定显示/审计来源；它进入
唯一 `SourceDocumentOrigin`，不覆盖 `documentKey`、摘要或长度，也不进入稳定身份、
文档集摘要或 LIR 语义。编译器不保存宿主绝对路径的隐式副本。

## 3. 公开 Rust 构造面

#296 只增加 LaneFlow 拥有的具体官方前端类型：

```rust
pub const GEOMETRY_FRONTEND_VERSION: u32 = 1;

pub struct GeometryDocumentInput<'a> { /* 字段私有，只借用 */ }
pub struct GeometryModuleBuilder { /* 字段私有 */ }
pub struct GeometryModule { /* 字段私有 */ }

impl<'a> GeometryDocumentInput<'a> {
    pub fn new(
        source_document_key: &'a str,
        source_bytes: &'a [u8],
        display_source: Option<&'a str>,
    ) -> Self;
}

impl GeometryModuleBuilder {
    pub fn new(
        input: GeometryDocumentInput<'_>,
        limits: &CompileLimits,
    ) -> Result<Self, DiagnosticBundle>;

    pub fn finish(self) -> Result<GeometryModule, DiagnosticBundle>;
}

impl GeometryModule {
    pub fn descriptor(&self) -> &SourceModuleDescriptor;
    pub fn source_documents(&self) -> impl ExactSizeIterator<Item = &SourceDocumentDescriptor>;
}

impl CompilationUnitBuilder {
    pub fn add_geometry_module(
        &mut self,
        module: GeometryModule,
    ) -> Result<&mut Self, DiagnosticBundle>;
}
```

所有类型由 `laneflow-compiler` 拥有并从 crate root 导出。它们不实现公开字段、
`Default`、反序列化、从裸 Typed AST 转换或可绕过校验的构造器。

职责和生命周期固定如下：

1. `GeometryDocumentInput::new` 只组装预期文档键、来源字节和显示来源的借用，不解析、
   不分配、不哈希、不验证；
2. `GeometryModuleBuilder::new` 先验证预期文档键并把它用于解析失败时的真实行列诊断，
   再执行字节上限预检、一次 SHA-256、有界解析、重复键检查、版本/closed-shape 检查
   和位置收集；文档内 `module.documentKey` 必须与预期键逐字节相等。返回前释放来源
   全文借用，只保留紧凑的前端记录、位置、描述符输入和精确资源计数；
3. `finish` 完成字段类型/单位/局部引用分类、曲线记录校验以及到共同 Typed AST 的唯一
   降阶；任一错误只返回规范 `DiagnosticBundle`，不返回部分模块；
4. `GeometryModule` 按值拥有一个不可拆分的私有 `AdmittedOfficialModule` 候选；
5. `add_geometry_module` 按值消费该候选并调用 #315 唯一的私有原子 admission。失败时
   builder 的模块、namespace/document 索引与累计计数完全不变，候选被释放；成功时不
   clone 全量声明、字符串或几何点，不重新哈希来源全文，也不重新统计记录。

不得新增公共 `Frontend` / `OfficialFrontend` trait、通用 `add_module`、独立 geometry
crate 或记录级动态分派。`SourceLanguage` 增加封闭变体
`GeometryDocument = 2`，稳定名称为 `geometry-document`；该枚举仍不是插件登记表。

## 4. v1 编制模型

### 4.1 稳定键、引用和顺序

每个可稳定声明或可独立寻址派生实体都必须由文档显式提供 stable authoring key。
不得从数组下标、坐标、遍历顺序、显示名或浮点摘要构造持久身份。引用采用：

```text
local-key
namespace::local-key
```

跨 namespace 引用必须同时存在显式 import。普通声明数组在 Typed AST 中按来源位置
保留以服务诊断，但 HIR/MIR 查找和最终 LIR 顺序不依赖输入排列。

所有者局部关系和出现项不分配 `StableId128`；它们在 MIR 中始终携带目标阶段键和自身
来源位置，并在 LIR 冻结时由同一个有类型排列同时生成目标 ordinal 与 source-map
local index。不得假设 HIR 的阶段局部稳定顺序等于最终完整 Identity v1 顺序。

### 4.2 规范坐标框架与参考线

`FrameRecord` 精确包含稳定 `frameKey`；一个或多个 `RoadRecord` 可以引用同一
`frameKey`。坐标继续使用右手系、`+Y` 上方向、`X/Z` 水平面和每轴
`[-16_384 m, 16_384 m]` 的规范范围。v1 不表达 CRS、宿主放置、动态原点或地形贴合。

每个 `RoadRecord` 至少包含：

```text
roadKey
frame
referenceLine { start: [x, y, z], segments: [CurveSegment] }
crossSectionSpans: [CrossSectionSpanRecord]
```

`roadKey` 是 Geometry 前端内用于组织连续参考线的显式键，不额外产生 Identity v1 尚未
登记的 `Road` 实体。每个纵向 `CrossSectionSpanRecord` 分别显式声明并降阶为一个
`RoadCorridor`；横断面变化因而使用新 corridor/section/lane-edge keys，不靠数组位置
延续旧身份。`roadKey` / `spanKey` 只作为不可跨模块引用的前端语法分组键，服务重复
检查和诊断稳定性；它们不是共同 Typed AST 的 stable declaration，不分配
`StableId128`，也不进入 LIR 或语义差异键。

`CurveSegment` 是有序闭合枚举：

- `line { end }`；
- `cubicBezier { control1, control2, end }`。

`referenceLine.segments` 与 `crossSectionSpans` 都必须非空。
段起点是前一段终点，第一段起点是 `referenceLine.start`。所有输入点以有限 `f64`
authoring 值检查；段端点必须精确衔接，不提供自动 snap。v1 不接受圆弧、clothoid、
NURBS、外部曲线库对象或自定义 evaluator；需要这些能力时提升来源版本，不能改变 v1
判读。

### 4.3 stationing、横断面与边生成

道路里程站位（Stationing）从 `0 m` 开始，沿参考线离散后的规范折线弧长单调增加。
每个纵向 `CrossSectionSpanRecord` 精确包含：

```text
spanKey
corridorKey
startStationMeters
endStationMeters: number | "end"
referenceSectionKey
referenceLaneKey
elements: [RoadSectionElement | FacilityBandElement]
roadSections: [RoadSectionRecord]
facilityBands: [FacilityBandRecord]
```

同一道路的 cross-section spans 必须按 station 构成无重叠、无间隙的完整覆盖；首项
起点精确为零，每个数值终点严格大于起点且与下一项起点在规范化 `f64` bit 上精确
相等，只有末项可以且必须使用 `"end"` 表示参考线最终 station。v1 不以容差重叠、
snap 或调用方另报总长闭合纵向区间。
`elements` 按 corridor 参考方向从左到右显式排列非空异构成员，且
`referenceSectionKey` 必须引用其中一个 `RoadSectionElement`；`referenceLaneKey` 必须
引用该 section 中一条 `direction = "forward"` 的 lane，并把该 lane 中心线精确绑定到
reference line。每个 RoadSection/FacilityBand 恰好出现一次；其 stable key、`kindId`
与 lane-bearing/non-traversable 类别继续复用共同约束。

`RoadSectionRecord` 精确包含 `sectionKey`、`kindId`、`lanes` 和 `laneGroups`。每个
`LaneSpanRecord` 显式提供 `laneKey`、`laneEdgeKey`、`direction = "forward" |
"backward"`、严格正宽度、限速、可选 `laneGroupKey` 与静态属性；`lanes` 数组按
corridor 参考方向从左到右排列，属于显式语义顺序。每个
`LaneGroupRecord` 显式提供 stable `laneGroupKey` 并由当前 RoadSection 唯一拥有。
同一 RoadSection 的全部 lane 必须有相同 direction；对向车道必须属于另一个
RoadSection。`FacilityBandRecord` 精确提供 stable `facilityBandKey`、non-traversable
`kindId` 和严格正宽度。lane/facility 的身份不由宽度、横向位置或数组下标派生。

MIR 从 reference lane 的零中心偏移向左右两侧，按显式 `elements` 及各 section 的
`lanes` 顺序对 lane/facility 宽度作确定性前缀和，得到全部中心偏移和边界偏移；结果
必须与显式横向顺序一致。在每个规范参考线采样点用投影到 `X/Z` 平面的单位切向构造
左向量，再生成 lane 中心线。
近垂直切向、偏移后越界、相邻边界倒置或量化后退化均为错误；不做隐式修补。
每个 `LaneSpanRecord` 产生一个明确 stable `LaneEdge`，交通长度与空间弧长从同一条
量化后中心线派生。需要跨 section 保持同一逻辑 lane 时由显式 predecessor/successor
引用闭合，不能按左右序号猜测。

reference line 先独立形成固定的 station 参数表；cross-section span 边界只在该表上
定位。每条
lane/facility offset curve 随后在相同的曲线参数域独立细分，并可增加自己的采样点，
但不能反向改变 reference station。这样外侧曲线仍满足自身误差预算，同时避免
“offset 细分改变参考线长度、参考线长度又改变 span 边界”的循环定义。反向 lane
只在最终中心线冻结时反转点序和拓扑方向，不改变道路 reference station。

每个 span 的 `corridorKey`、`sectionKey`、`laneKey`、`laneGroupKey`、facility key 和
`laneEdgeKey` 分别进入既有 `RoadCorridor`、`RoadSection`、`AuthoringLane`、
`LaneGroup`、`FacilityBand` 与 `LaneEdge` Identity v1。它们的 owner、横向有序 member
与 edge coverage 在 topology MIR 中闭合；普通声明数组重排不改变身份或关系结果，
但显式 `elements` 的从左到右顺序属于语义。

### 4.4 路口和连接

`JunctionRecord` 显式提供 stable `junctionKey`、approach edge 集合和
`connections`。v1 不执行“最近端点自动连线”或按车道序号猜测连接。每个 connection
必须显式提供：

- `movementKey` 与有向 entry/exit approach keys；
- `maneuverPathKey`；
- 零个或多个具有显式 `laneEdgeKey` 的 internal edges；
- 路径的完整 `entry + internal + exit` 有序 edge 引用；
- internal edge 的 line/cubic-Bezier 几何；
- 可选显式 `stopLineKey`、`maneuverGateKey` 与 `waitingZoneKey` 引用。

topology MIR 复用共同的 Junction/Movement/ManeuverPath、内部边排他角色、路径连通、
Gate/WaitingZone occurrence 与 owner/coverage 约束。几何 MIR 另外验证连接端点、方向、
规范 frame、量化后连续性和路径 station 区间。俯视几何相交不能自动生成 Movement、
冲突、优先权或信号语义。

### 4.5 静态 overlay

`OverlayRecord` 是 closed object，包含与当前 #292 支持矩阵对应的可选数组：

```text
signalGroups
signalControllers
parkingAreas
parkingSpaces
participantClasses
vehicleProfiles
accessRules
staticRoutes
stopLines
maneuverGates
waitingZones
```

每类记录使用与共同 Typed AST 相同的稳定键、引用、值域和有序关系语义；Geometry
前端只负责严格文本类型、真实 span 和单位化输入，不复制 HIR 的跨模块解析、owner、
coverage、继承、信号完备性、停车锚点、准入冲突或路线 occurrence 验证。

v1 不支持 ConflictZone/ParticipantStream、时变准入、动态路线生命周期、二维区域
布尔运算、mesh、材质或 Adapter 表现。未支持字段必须失败关闭，不能作为不透明扩展
静默保留。

## 5. Typed AST、HIR 与拓扑/几何 MIR

### 5.1 唯一降阶

Geometry records 只能通过私有 `TypedAstSink` 降阶到共同
`TypedAstDeclaration`。不得让 Geometry module 保留一套与 Synthetic 平行的完整
语义 AST 并在 HIR 记录级分派。Typed AST 可以增加表达参考线、横断面和连接意图的
私有声明变体；direct overlay 立即降为既有共同声明变体。

### 5.2 HIR 权威

HIR 完成并且只完成：

- namespace/import/symbol 解析与有类型引用；
- 数字 token 的有限值、单位和值域规范化；
- 显式默认展开；v1 不存在依赖遍历顺序的上下文默认；
- road/section/lane、junction/connection 和 overlay 的 owner 候选闭合；
- 每项来源位置归属预检，且必须早于任一可能遮蔽它的语义诊断。

HIR 可为阶段局部确定性按稳定键排序，但该顺序不构成 LIR canonical order。

### 5.3 topology MIR

topology MIR 拥有 corridor/section/lane 展开、显式 lane edge、junction/movement/path、
predecessor/successor、owner/member/coverage、overlay 绑定、路线 occurrence 与反向索引
的 target-neutral 表。所有 stable entity 先闭合完整 CanonicalIdentity，再进入最终
排序；owner-local relation 保留有类型 MIR row key 和 span，不分配 StableId。

### 5.4 geometry MIR

geometry MIR 拥有 authoring 曲线求值、确定性细分、station 切分、lane offset、规范
`f32` 点、累计弧长、切向/上方向、Traffic length 与 Spatial sampling table。它不包含
target ABI、mesh、宿主坐标或运行时对象。

topology 与 geometry MIR 是同一次编译的两个私有视图，不是两个可独立发布或先后调用
的编译器层。两者以有类型 key 互相引用，并在进入 LIR 前共同通过：

- 每个 Geometry 派生 edge 恰好一条 geometry，Geometry module 不能产生稀疏未知
  覆盖；后继 headless static-image profile 只可在 emitter 裁剪 Spatial section，不能
  从 canonical LIR 删除 Geometry 的规范语义；
- owner、coverage、station、拓扑端点和几何端点一致；
- 所有静态 overlay 已绑定到相同 stable entity/owner-local occurrence；
- LIR 规范排列同时重排语义目标与其来源位置。

## 6. 数值、曲线细分与确定性

### 6.1 计算链

v1 的 clean single-thread 预言机使用 Rust `f64` 执行三维 line/cubic Bézier 求值、
de Casteljau `t = 0.5` 细分、station 累计和 lane offset；禁止 fused/non-fused 不一致的
平台专用 fast-math。输出点每个分量只在进入规范点表时按 IEEE 754 round-to-nearest,
ties-to-even 转为 `f32`，并把 `-0.0` 规范化为 `+0.0`。
JSON 十进制 token 必须先保留精确字节，再以 Rust 标准库 `str::parse::<f64>()` 的最近值
语义转换一次；溢出、非有限结果或不满足字段值域时失败，禁止先经通用 JSON number
或另一精度往返。整数和 `randomSeed` 等整数文本按目标 `u32`/`u64` 直接解析并检查，
不得经 `f64` 往返。

对 cubic Bézier，按原始段顺序深度优先、左子段优先递归；当两个内控制点到端点弦的
最大三维距离不超过 `0.0025 m`，且两端切向与弦的夹角判定都通过时停止。角度判定不
调用平台 `acos`：切向与弦的 dot 必须为正，并满足
`dot² >= tangentLength² × chordLength² × 0x3fefffd812cce4e6`；末项是冻结的 `f64`
`cos²(0.25°) = 0.9999809615320856`。零长度弦或零长度端点切向直接失败。line 不细分，
除非 section/station 边界要求插点。reference line 使用该规则形成 station 参数表；
每条 offset curve 还必须对其自身位置/切向重复相同停止判定并独立细分。
递归深度上限为 20；达到上限仍不满足误差、产生超过 `GeometryPointCount` 的点、或
量化后任一线段长度不严格大于既有
`SPATIAL_MIN_SEGMENT_LENGTH_METERS = 0.1 m` 时失败关闭，不降低精度继续编译。

以上阈值给最终 `1 cm` 位置预算保留曲线近似、`f32` 量化和连接检查余量。实现可以用
迭代栈替代递归，但必须产生逐 bit 相同的点序列；更换算法、阈值或求值顺序需要提升
frontend/constraint 版本并重新进入 G1。

### 6.2 长度与连续性

规范点完成后：

- geometry segment length、累计弧长和切向按实际 `f32` 点计算；
- Traffic `LaneEdge.length` 从同一量化后点序列以规范 `f64` 累计派生，不接受文档
  另报长度；
- 长度绑定继续使用
  `max(0.01 m, 1e-6 × max(trafficLength, geometryArcLength))`，当前 Core 长度量化
  余量为零；
- 相连端点距离不得超过 `0.005 m`；超差失败，不 snap；
- 投影上方向长度必须至少为既有 `SPATIAL_MIN_PROJECTED_UP_LENGTH`；
- 所有比较使用含等号边界的既有常量，禁止 epsilon 叠加或 fallback 到另一份几何。

`f64` authoring curve 只在前端/geometry MIR 存续；LIR 不保存 evaluator、控制点或第二份
高精度运行时权威。离线 `f64` oracle 可以用于测试，但不能进入生产输出。

## 7. 资源、诊断与失败原子性

### 7.1 资源维度

Geometry 前端复用全部 #315 共同累计维度。额外工作必须归入现有维度：

- 原始文档字节计入 `SourceBytesPerModule` / `SourceBytesTotal`；
- JSON token、对象、数组、曲线段、section/lane/connection 与 overlay 记录计入
  `TypedAstRecordCount`，引用和 owner-local 成员分别计入既有引用/关系维度；
- authoring 控制点和最终规范折线点都在其存续阶段计入 `GeometryPointCount`，峰值按
  同时存续集合而不是两者相加后的历史累计解释；
- parser stack、duplicate-key table、曲线细分栈、station scratch、HIR/MIR 工作集分别
  计入 `StageScratchBytes`；
- 解析后存续的字符串、记录、span、点和模块包装计入真实
  `CompilerControlledLiveBytes`；
- 诊断条数继续受 `DiagnosticCount` 限制，达到保留上限后仍按安全边界完成计数，并
  只保留全局规范顺序最小前缀。

任何按输入规模分配必须先以 checked `u64` 计算 length+1/候选容量并比较上限；来源
长度还必须在哈希或解析前可收窄到描述符所需 `u32`。未知长度流不在 v1 API 范围内。
解析不得先构造无界通用 JSON value tree 再统计。#296 不增加生产解析/几何依赖；
实现使用 crate 私有的有界 JSON tokenizer/parser，直接产生紧凑记录与 span。

### 7.2 诊断阶段与顺序

错误优先级固定为：

1. 来源字节/编码/顶层版本与 parser 资源错误；
2. JSON 语法、重复键、closed-shape 与字段类型；
3. 模块头、单位、稳定键、局部重复与曲线局部值域；
4. 共同 admission 的 namespace/document/资源错误；
5. HIR 来源归属、import/symbol/unit/owner 候选错误；
6. topology/geometry MIR 的 coverage、station、连续性、细分和全局静态语义；
7. Identity closure 与 LIR 规范化错误。

同一阶段按 `(canonical module order, source document order, primary span, diagnostic
code, stable key, typed payload)` 排序。解析恢复只能在当前 closed object/array 的已知
边界内进行；无法证明安全同步点时只报告首个语法错误，不能猜测后续结构。

任一错误级诊断使当前操作失败且不返回部分 Typed AST、HIR、MIR、LIR 或源映射。失败
后的 `Compiler` 可复用容量，但下一次编译的输出、诊断和指标不得受污染。

## 8. 正确性与等价验证矩阵

G2 实现至少必须自动覆盖：

| 类别       | 必须证明的样例                                                                                                                        |
| ---------- | ------------------------------------------------------------------------------------------------------------------------------------- |
| 来源       | 最小合法文档、BOM/非法 UTF-8、重复键、未知字段、旧/未来版本、精确字节摘要与真实行列                                                   |
| 文档基数   | Geometry 单文档在 v1/v2 均成功；不存在 include；与多文档模块组合时 v1 在分配前失败、v2 成功                                           |
| API 原子性 | `new`、`finish`、`add_geometry_module` 每个失败点不泄漏部分模块或改变 unit builder                                                    |
| 标识       | 普通声明重排、无关插入、空白/字段重排和 geometry-only edit 不改变应稳定实体 ID；显式 key/owner/topology 改变按 Identity v1 改变       |
| topology   | cross-section span 完整 coverage、横向 elements、lane predecessor/successor、junction/path owner、内部边排他和 owner-local occurrence |
| geometry   | line/cubic 边界、深度 20、阈值含等号/加一 ULP、station 边界、offset、范围、最短段、近垂直、连接 5 mm 边界                             |
| 来源映射   | 声明、派生 edge、owner-local relation 与 LIR 目标经过同一 permutation；HIR 局部顺序与 LIR Identity 顺序不同的反例                     |
| 等价       | 代表性受保护转向走廊与 #292 Synthetic DSL 产生相同 CanonicalIdentity、全部 LIR 表/关系/数值、语义指纹和 current 投影                  |
| 确定性     | 模块/声明重排、无关插入、重复 clean compile、不同优化级别和受支持平台产生相同 LIR 语义与点 bit pattern                                |
| 资源       | 每个相关 limit 的边界/边界加一、诊断截断、parse/细分/共同 admission/build scratch 和失败清理                                          |

等价比较不能只比较 current JSON 或对象数量；必须逐表比较有类型 ordinal、稳定身份
完整前像、owner-local relation、规范 `f32` 点 bit、累计弧长、来源键角色和语义指纹。

## 9. 性能与收益/代价

### 9.1 三个独立测量边界

G2 必须在 release、单工作线程、同机 base/candidate 下分别报告：

1. **Geometry parse/build**：从借用原始 bytes 到 `GeometryModule`；包含一次 SHA-256、
   有界解析、span、曲线前端记录与专用降阶，不包含共同 admission；
2. **共同 admission**：对已构造 `GeometryModule` 执行 `add_geometry_module + build`；
   不把 parse/curve 成本归因给 #315；
3. **完整 compile**：原始 Geometry bytes 到 `CompilationOutput`；包含 HIR/MIR/LIR 和
   source-map，作为真实产品成本。

每级至少一个预热样本、七个正式样本、三个独立进程，报告每进程中位数/MAD及中位数
的中位数；同时报告原始字节、模块/文档/声明/引用/关系/曲线段/控制点/规范点/LIR 记录、
逻辑输出字节、编译器控制峰值、保留容量与语义指纹。不得把 parser-only 数字、受控
分配记录或操作系统进程内存互相替代。

### 9.2 工作负载

冻结三个 Geometry 工作负载：

- `LF-COMP-GEOMETRY-MIN-v1`：单 frame、单 road、单 cross-section span、单
  RoadSection、两条 lane、line-only；
- `LF-COMP-GEOMETRY-CORRIDOR-v1`：与 #292 代表性受保护转向走廊语义等价，包含 cubic
  connection、signals、parking、access、routes、Gate/WaitingZone；
- `LF-COMP-GEOMETRY-P100-v1`：与 #292 P100 五级最高级相同的合法静态语义规模，并
  保持相同模块图和 LIR 记录数；Geometry 特有计数另行报告。

同语义 Synthetic base 只用于分离 Geometry 固有解析/细分成本，不作为“新文本前端只
能增加百分之十”的错误门槛。P100 推荐参考机上，最高级完整 compile 的三进程中位数
的中位数必须不超过 `25 ms`，编译器控制峰值不得超过 `6 MiB`，冷实例完成后的保留
容量必须为 `0 B`；相同 Geometry 候选迭代若出现可重复、无法解释的时延或峰值超过
`5%` 回退同样阻断。超过任一门槛必须以 workload/profile/算法证据重新进入 G1，不得
删除 span、资源检查或降低曲线误差来达标。共同 admission 自身继续满足 #315 的
`5%` 回归门槛。

收益是：生产编制语义、拓扑和几何由同一编译器闭合，消除手写 Traffic/Spatial 双份
长度、外部 ID join 和 Adapter 端样条权威。代价是：严格来源格式、曲线细分和真实 span
增加解析/暂存内存；v1 只支持 line/cubic Bézier 且一模块一文档，复杂 CAD/GIS 输入要
由后继 importer 显式转换。该限制是有意的 closed contract，不通过兼容 shim 隐藏。

## 10. 明确不做与重新进入 G1 的触发条件

本切片不实现或冻结：

- 公共第三方前端插件协议、独立 geometry crate、编辑器 UI、OSM/CAD importer；
- portable artifact、独立 validator、target static image 或 Runtime/Spatial cutover；
- CRS、地形、mesh、材质、宿主 Transform、动态路线或时变规则；
- 圆弧/clothoid/NURBS、自定义 evaluator、自动路口连接、自动冲突/优先权推断；
- 增量/并行编译、缓存协议或 source-compatible 历史格式读取。

下列任一变化必须重新进入 G1：公共 API 所有权/错误/可见性变化；Geometry module
多文档化；来源编码或版本判读变化；曲线类型、细分算法/阈值、station/offset 语义变化；
新增稳定身份类型或改变 Identity v1 前像；新增 crate/依赖；让 HIR/MIR 成为公共兼容
面；或让 current Core/Spatial 对象图进入生产 IR。
