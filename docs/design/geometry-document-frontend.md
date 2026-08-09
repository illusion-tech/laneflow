# 几何文档前端与拓扑/几何中层表示

**文档状态**: Frozen（#296 G1；G2 实现权威输入）<br>
**最后更新**: 2026-08-07<br>
**适用范围**: `laneflow-compiler` 的几何文档前端（Geometry Document Frontend）、
`GeometryModuleBuilder` / `GeometryModule`、几何来源格式 v1、拓扑/几何中层表示
（Topology/Geometry MIR）与已验证规范低层中间表示（Validated Canonical LIR）降阶<br>
**关联文档**: `compiler-foundation.md`、`network-compiler.md`、
`spatial-geometry.md`、`../adr/0020-compiler-owned-static-network-and-static-image.md`、
`../adr/0022-authoring-curve-and-canonical-polyline-error-budgets.md`

## 1. 目标与状态边界

本文冻结 #296 的实现输入；对应 Rust 实现已按本文交付并通过 §8 等价与九组合验证。
#296 G1 通过前不得修改生产 Rust；G2 后的实现必须保持以下边界：

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
- 对象字段顺序和普通声明集合的数组顺序不构成语义；v1 只有
  `referenceLine.segments`、按 station 排列的 `crossSectionSpans`、横向
  `elements/lanes`、`internalEdgeSequence`、控制器 `phases` 和静态路线
  `edgeSequence` 是有序语义数组。`imports`、普通声明数组、`roadSections`、
  `facilityBands`、`laneGroups`、`successors`、`approachEdges`、`connections`、相位
  `states` 和 overlay 各声明数组都是无序集合；其中要求不得重复的集合仍按各自规则
  检查，Typed AST 的来源排列只服务诊断；
- 字符串不 trim、不大小写折叠、不做 Unicode normalization。身份键、命名空间、文档
  键和引用继续使用编译器既有 ASCII external-token 契约；面向人的名称可使用 UTF-8，
  但不进入 Identity v1 前像。

文档描述符的 `sourceRecordByteLen` 是输入文档的精确字节数，
`sourceDocumentDigest` 是对同一精确字节序列计算一次 SHA-256 的结果。空白或对象字段
重排可以保持相同 LIR 语义，但会有不同的来源文档摘要；编译器不得先重新序列化 JSON
再计算来源摘要。模块级文档集摘要继续由 #315 的
`LFSOURCE-DOCUMENT-SET` v1 规则从已派生的唯一文档描述符聚合。

### 2.3 顶层封闭形状

机器可检验的完整 wire shape 由
[`geometry-document-v1.schema.json`](../reference/geometry-document-v1.schema.json) 冻结，
使用 JSON Schema Draft 2020-12。本文冻结 schema 不能表达的跨记录约束和唯一降阶；两者
必须同时满足。schema 文件的新增/删除字段、required 集合、枚举值或数值表示变化都属于
几何来源格式变更，不能由 G2 实现自行解释。

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
`generatorBuildId = "laneflow-geometry-direct-v1"`、域分离的空输入摘要、由几何精度/
方向配置档绑定的选项摘要和 `randomSeed = None`。
`GeneratedProvenance` 精确包含
`{ kind: "generated", generatorBuildId, parametersAndInputsDigest,
frontendOptionsDigest, randomSeed, description }`，其中每个摘要是 64 个 ASCII 小写
十六进制字符，编码 32 bytes / 256 bit 摘要；
`randomSeed` 是十进制 `u64` 字符串或 `null`，避免 JSON/JavaScript number 丢失高位。
这些值只记录来源沿袭，不认证文档内容；发布真实性仍由
后继外部描述符承担。两种 `description` 都是受字符串上限约束的可见 UTF-8 文本。
direct 的两个固定摘要前像分别是 ASCII
`laneflow.geometry.direct.inputs.v1\0` 和
`laneflow.geometry.direct.frontend-options.v1\0`；各自直接计算一次 SHA-256，不包含
JSON 空白、显示来源或 compiler build。前者直接成为 direct 的
`parametersAndInputsDigest`，后者成为 direct 的来源前端选项摘要；generated 的来源
前端选项摘要是文档内 `frontendOptionsDigest` 解码后的 32 bytes。最终模块描述符的
`frontendOptionsDigest` 对以下精确前像再计算一次 SHA-256：

```text
ASCII "laneflow.geometry.frontend-options.v1\0"
|| geometryAccuracyProfileCode: u8
|| geometryDirectionProfileCode: u8
|| sourceFrontendOptionsDigest: [u8; 32]
```

因此同一来源文档使用不同几何精度或方向配置档时，来源文档摘要保持不变，但模块选项
摘要、规范点、几何数值和 LIR 语义指纹可以不同。实现必须为 direct 的九种配置档组合
及 generated 零摘要输入冻结已知向量。

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
`u32` 行列范围。行分隔符精确识别原始 UTF-8 bytes 中的 `LF`、`CRLF` 或单独 `CR`；
`CRLF` 只增加一行。列是一基 UTF-8 byte offset：行首为 1，多 byte Unicode scalar
按其编码字节数推进，tab 只推进 1，禁止按显示宽度、Unicode scalar 数或 UTF-16 code
unit 计数。范围覆盖产生该语义的最窄完整原始 JSON value bytes，结束位置是最后一个
被覆盖 byte 的含端点行列；派生实体使用其拥有者和产生它的关系成员作为
primary/related span。输入超过 `u32` 行列表示或来源字节上限时在构造相应表前失败
关闭。

调用方可以通过 `GeometryDocumentInput` 提供一个未认证的稳定显示/审计来源；它进入
唯一 `SourceDocumentOrigin`，不覆盖 `documentKey`、摘要或长度，也不进入稳定身份、
文档集摘要或 LIR 语义。编译器不保存宿主绝对路径的隐式副本。

## 3. 公开 Rust 构造面

#296 只增加 LaneFlow 拥有的具体官方前端类型：

```rust
pub const GEOMETRY_FRONTEND_VERSION: u32 = 1;

pub enum GeometryAccuracyProfile {
    Fine2Cm,
    Balanced5Cm,
    Compact10Cm,
}

pub enum GeometryDirectionProfile {
    Smooth1Deg,
    Balanced2Deg,
    Compact5Deg,
}

pub struct GeometryDocumentInput<'a> { /* 字段私有，只借用 */ }
pub struct GeometryModuleBuilder { /* 字段私有 */ }
pub struct GeometryModule { /* 字段私有 */ }

impl GeometryAccuracyProfile {
    pub const fn stable_name(self) -> &'static str;
    pub const fn max_position_error_meters(self) -> f64;
}

impl GeometryDirectionProfile {
    pub const fn stable_name(self) -> &'static str;
    pub const fn max_runtime_direction_jump_degrees(self) -> f64;
}

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
        accuracy_profile: GeometryAccuracyProfile,
        direction_profile: GeometryDirectionProfile,
        limits: &CompileLimits,
    ) -> Result<Self, DiagnosticBundle>;

    pub fn finish(self) -> Result<GeometryModule, DiagnosticBundle>;
}

impl GeometryModule {
    pub fn descriptor(&self) -> &SourceModuleDescriptor;
    pub fn source_documents(&self) -> impl ExactSizeIterator<Item = &SourceDocumentDescriptor>;
    pub const fn accuracy_profile(&self) -> GeometryAccuracyProfile;
    pub const fn direction_profile(&self) -> GeometryDirectionProfile;
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

三个封闭位置配置档及稳定编码固定为：

| Rust 变体     | 稳定名            | 代码 | 最终位置上限 | 曲线细分子预算 |
| ------------- | ----------------- | ---: | -----------: | -------------: |
| `Fine2Cm`     | `fine-2cm-v1`     |    1 |     `0.02 m` |       `0.01 m` |
| `Balanced5Cm` | `balanced-5cm-v1` |    2 |     `0.05 m` |      `0.025 m` |
| `Compact10Cm` | `compact-10cm-v1` |    3 |     `0.10 m` |       `0.05 m` |

调用方必须显式选择；不提供任意 `f64` 容差、`Default`、按输入规模自动选择或资源不足时
自动降级。`Balanced5Cm` 是产品推荐档，不是隐式默认值。

三个封闭方向配置档及稳定编码固定为：

| Rust 变体      | 稳定名             | 代码 | 最终 f32 相邻弦方向跳变上限 | f64 候选区间端点切向与弦夹角上限 |
| -------------- | ------------------ | ---: | --------------------------: | -------------------------------: |
| `Smooth1Deg`   | `smooth-1deg-v1`   |    1 |                        `1°` |                           `0.5°` |
| `Balanced2Deg` | `balanced-2deg-v1` |    2 |                        `2°` |                             `1°` |
| `Compact5Deg`  | `compact-5deg-v1`  |    3 |                        `5°` |                           `2.5°` |

方向配置档同样必须显式选择，不实现 `Default` 或任意角度入口；`Balanced2Deg` 是产品推荐
档。位置与方向正交组合，v1 恰好支持九种组合，不能按 road/lane/curve 分别选择。

职责和生命周期固定如下：

1. `GeometryDocumentInput::new` 只组装预期文档键、来源字节和显示来源的借用，不解析、
   不分配、不哈希、不验证；
2. `GeometryModuleBuilder::new` 先验证几何精度/方向配置档和预期文档键，并把文档键用于解析
   失败时的真实行列诊断，再执行字节上限预检、一次来源 SHA-256、有界解析、重复键
   检查、版本/closed-shape 检查、位置收集和配置档绑定的模块选项摘要；文档内
   `module.documentKey` 必须与预期键逐字节相等。返回前释放来源全文借用，只保留紧凑的
   前端记录、位置、两个配置档和精确资源计数；
3. `finish` 完成字段类型/单位/局部引用分类、曲线记录校验、到共同 Typed AST 的唯一
   降阶，并执行一次 Geometry 私有 numeric freeze：按第 6 节生成该模块全部最终规范点、
   lane/facility/internal-edge geometry payload 和精确模块资源计数。任一错误只返回规范
   `DiagnosticBundle`，不返回部分模块；
4. `GeometryModule` 按值拥有一个不可拆分的私有 `AdmittedOfficialModule` 候选及其已冻结
   geometry payload；后继 HIR/MIR 只移动、规范重排和执行跨模块/拓扑验证，不重复曲线
   求值、station、offset、细分、量化或点数统计；
5. `add_geometry_module` 按值消费该候选并调用 #315 唯一的私有原子 admission。失败时
   builder 的模块、namespace/document 索引与累计计数完全不变，候选被释放；成功时不
   clone 全量声明、字符串或几何点，不重新哈希来源全文，也不重新统计记录。

同一编译单元内的全部 Geometry 模块必须使用相同的位置与方向配置档。该约束不修改
#315 的共同 admission：Geometry HIR 读取已接入模块的两个有类型配置档，混用时分别
返回 `MixedGeometryAccuracyProfile` 或
`MixedGeometryDirectionProfile`；Synthetic/current-import 模块不参与该比较，没有
Geometry 模块的编译单元也不携带虚构配置档。配置档差异不改变实体稳定标识；位置或
方向档位可以改变规范点、几何数值、派生长度和语义指纹，但不能改变 6.1 节冻结的
reference station 基表或 source station 的语义位置。

由于公共 API 必须先独立 `finish` 每个模块，混用配置档只能在单元 HIR 识别；各模块的
numeric freeze 和共同 admission 已经发生。该失败路径可以浪费受限计算，但 `build`
原子失败、不返回部分输出，也不绕过任何累计 limit。不得为提前发现混用而新增调用方
自报的单元配置、全局可变状态或第二条 admission；性能证据必须包含两个已完成大模块
混用后失败的有界最坏路径。

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

`::` 是限定引用的保留分隔符，不得出现在任何 Geometry v1 local declaration key
（包括 owner-local directed approach key 与 phase key）中；namespace、document key
和引用整体仍使用通用 external-token 字母表。机器 schema 以独立 `localKey` 定义表达
该约束，编译器在 lowering 前再次执行同一语义检查，不能依赖 schema 已先运行。

跨 namespace 引用必须同时存在显式 import。普通声明数组在 Typed AST 中按来源位置
保留以服务诊断，但 HIR/MIR 查找和最终 LIR 顺序不依赖输入排列。

所有者局部关系和出现项不分配 `StableId128`；它们在 MIR 中始终携带目标阶段键和自身
来源位置，并在 LIR 冻结时由同一个有类型排列同时生成目标 ordinal 与 source-map
local index。不得假设 HIR 的阶段局部稳定顺序等于最终完整 Identity v1 顺序。

### 4.2 规范坐标框架与参考线

`FrameRecord` 精确包含稳定 `frameKey`；一个或多个 `RoadRecord` 可以引用同一
`frameKey`。坐标继续使用右手系、`+Y` 上方向、`X/Z` 水平面和每轴
`[-16_384 m, 16_384 m]` 的规范范围。v1 不表达 CRS、宿主放置、动态原点或地形贴合。

每个 `RoadRecord` 精确包含：

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

道路里程站位（Stationing）从 `0 m` 开始，沿 6.1 节配置档无关的 `f64` reference station
基表弦长单调增加。
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
`LaneSpanRecord` 精确提供 `laneKey`、`laneEdgeKey`、`direction = "forward" |
"backward"`、`widthMeters`、`speedLimitMetersPerSecond`、可省略的 `laneGroupKey` 和
`successors`。`successors` 是无序、不得重复的 edge reference 数组；空数组表示普通
道路延续不声明下游边。v1 不存在 `predecessors` wire 字段，反向 predecessor 索引由
successor 关系唯一派生；也不存在未登记的“静态属性”包或扩展 map。`lanes` 数组按
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

reference line 先按 6.1 节的固定、配置档无关算法形成不可变 station 参数表；
cross-section span 边界只在该表上定位。每条 lane/facility offset curve 随后在相同的
原始曲线参数域独立细分，并可增加自己的采样点，但不能反向改变 reference station。
这样外侧曲线仍满足自身误差预算，同时避免“offset 细分改变参考线长度、参考线长度又
改变 span 边界”的循环定义。反向 lane 只在最终中心线冻结时反转点序和拓扑方向，不
改变道路 reference station。

每个 span 的 `corridorKey`、`sectionKey`、`laneKey`、`laneGroupKey`、facility key 和
`laneEdgeKey` 分别进入既有 `RoadCorridor`、`RoadSection`、`AuthoringLane`、
`LaneGroup`、`FacilityBand` 与 `LaneEdge` Identity v1。它们的 owner、横向有序 member
与 edge coverage 在 topology MIR 中闭合；普通声明数组重排不改变身份或关系结果，
但显式 `elements` 的从左到右顺序属于语义。

### 4.4 路口和连接

`JunctionRecord` 精确提供 stable `junctionKey`、无序且不得重复的 `approachEdges`、
无序的 `internalEdges` 声明和无序的 `connections`。每个 internal edge 由当前
Junction 唯一拥有，但可以被该 Junction 的多条 connection path 引用；v1 不执行
“最近端点自动连线”或按车道序号猜测连接。
`ConnectionRecord` 精确包含：

```text
movementKey
directedEntryApproachKey
directedExitApproachKey
maneuverPathKey
entryEdge
internalEdgeSequence: [lane-edge-ref]
exitEdge
```

`InternalEdgeRecord` 精确包含 `laneEdgeKey`、严格正且有限的
`speedLimitMetersPerSecond` 和 `geometry { start, segments }`；限速必须显式提供，不从
entry/exit edge 或相邻 internal edge 继承。`internalEdgeSequence` 是连接内的有序引用，
每项必须解析到当前 Junction 的 `internalEdges`，同一 connection 内不得重复；权威路径
序列唯一等于 `entryEdge + internalEdgeSequence + exitEdge`，不再内嵌声明或接受第二份
完整路径数组。Junction 中没有被任何 connection 引用的 internal edge、或引用另一
Junction 所有 internal edge 均失败关闭。
StopLine、ManeuverGate 与 WaitingZone 只在 4.5 节 overlay 中声明，不在 connection
中保存可选兼容字段。

普通道路延续和路口转换是互斥来源：`LaneSpanRecord.successors` 只可声明不穿越
junction internal path 的普通 section 延续；每个 connection path 的相邻 edge pair
只由该 path 派生。Topology MIR 按规范 edge key 聚合这两组输入，生成最终
`LaneEdge.successors` 及反向 predecessor 索引。同一有向 transition 若同时来自普通
successor 与任一 junction path，或来自不同 Junction，均失败关闭；同一 Junction 内因
共享 internal edge 而由多条 connection path 派生的同一 transition 按
`(junction identity, predecessor edge identity, successor edge identity)` 规范去重，且
所有 occurrence span 在 topology MIR 中保留用于相关诊断。唯一 LIR successor relation
的来源映射取这些 occurrence 按 `(canonical module order, source document order,
span start, span end)` 排序后的最小项，并与 relation 一同经过 LIR permutation；其余
span 不扩展当前单位置 source-map API。该去重只收敛同一权威 owner 的相同事实，不是
兼容两份冲突声明。

一个 connection 的 entry edge、全部 internal edge 和 exit edge 必须解析到同一个
`CanonicalFrame`。该 frame 由 entry/exit approach geometry 唯一导出并互相验证；两端
frame 不同、任一 approach 无 geometry、或 internal geometry 尝试声明另一 frame 时均
失败。Internal edge 不携带独立 `frame` 字段。

topology MIR 复用共同的 Junction/Movement/ManeuverPath、内部边排他角色、路径连通、
Gate/WaitingZone occurrence 与 owner/coverage 约束。几何 MIR 另外验证连接端点、方向、
规范 frame、量化后连续性和路径 station 区间。俯视几何相交不能自动生成 Movement、
冲突、优先权或信号语义。

### 4.5 静态 overlay

`OverlayRecord` 是 closed object，精确包含下列必选数组；每个数组可以为空：

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

各数组记录的完整字段、required 集合、枚举 token 与 closed shape 由 wire schema 冻结。
Geometry 前端只负责严格文本类型、真实 span 和单位化输入，不复制 HIR 的跨模块解析、
owner、coverage、继承、信号完备性、停车锚点、准入冲突或路线 occurrence 验证。
`offsetSeconds` / `durationSeconds` 按原始十进制 token 解析为 `f64` 后以固定顺序乘
`1000.0`，结果必须有限、无小数且可无损收窄为共同 Typed AST 的 `u64` 毫秒；否则失败，
不四舍五入。可选的 `laneGroupKey`、`parkingArea`、`extends`、`regulation.source` 等字段
只允许省略，schema 未显式允许 `null` 的字段不得以 `null` 表示缺失；
`signalControl: null` 是唯一登记的空控制值。

v1 不支持 ConflictZone/ParticipantStream、时变准入、动态路线生命周期、二维区域
布尔运算、mesh、材质或 Adapter 表现。未支持字段必须失败关闭，不能作为不透明扩展
静默保留。

## 5. Typed AST、HIR 与拓扑/几何 MIR

### 5.1 唯一降阶

Geometry records 只能通过私有 `TypedAstSink` 降阶到共同
`TypedAstDeclaration`。不得让 Geometry module 保留一套与 Synthetic 平行的完整
语义 AST 并在 HIR 记录级分派。Typed AST 可以增加表达参考线、横断面和连接意图的
私有声明变体；direct overlay 立即降为既有共同声明变体。

唯一 lowering 固定为：

| wire 来源                                                | Typed AST / MIR 结果                                                                                                                                                              |
| -------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `module.namespace/documentKey/imports/provenance`        | 分别进入唯一模块/文档描述符、共同 module graph 和来源沿袭；不产生领域声明                                                                                                         |
| `units`                                                  | 只验证 v1 固定单位并决定数值字段解释；不产生记录或运行时单位表                                                                                                                    |
| `FrameRecord.frameKey`                                   | 一个 `CanonicalFrame`；edge geometry 成员由 lane/internal-edge geometry intent 反向闭合                                                                                           |
| `RoadRecord.roadKey/frame/referenceLine`                 | `roadKey` 只作前端分组；`frame` 绑定全部派生 edge geometry；reference curve 进入 geometry MIR，不产生 `Road` 实体                                                                 |
| `CrossSectionSpanRecord`                                 | 一个私有 span intent；随后恰好产生一个 `RoadCorridor`                                                                                                                             |
| `RoadSectionRecord.sectionKey/kindId/lanes/laneGroups`   | 一个 `RoadSection`；kind 原样验证，成员关系来自有序数组和 lane 反向绑定                                                                                                           |
| `LaneGroupRecord.laneGroupKey`                           | 一个 `LaneGroup`，owner 为所在 section；成员只由 lane 的可选 `laneGroupKey` 形成                                                                                                  |
| `LaneSpanRecord`                                         | `laneKey` 生成一个 `AuthoringLane`，`laneEdgeKey/speedLimit/successors` 生成一个 `LaneEdge` 候选；width/direction 进入 offset intent，edge length 延后从最终规范点派生            |
| `FacilityBandRecord`                                     | key/kind 生成一个 `FacilityBand`；width 进入不可遍历 offset geometry intent，最终产生一行按 FacilityBand ordinal 排列的 `facility_band_geometries` 与规范点范围                   |
| `JunctionRecord.junctionKey/approachEdges/internalEdges` | 一个 `Junction`；approach 集合只用于 owner、边界与 frame 闭包；每个 internal record 生成由该 Junction 唯一拥有、带显式 speed 的 `LaneEdge` 与 geometry intent，不直接写 successor |
| `ConnectionRecord`                                       | movement/approach keys 生成一个 `Movement`；path key 与 entry/有序 internal 引用/exit 生成一个 `ManeuverPath`；相邻 pair 生成 successor，同一 Junction 内共享 transition 规范去重 |
| `OverlayRecord` 各成员                                   | 逐项直接降为同名既有共同声明，不保留 Geometry 专用副本                                                                                                                            |

任何 wire 字段必须恰好由上表消费一次或仅用于诊断/来源沿袭；不得把无法 lowering 的字段
保留为 opaque extension。由 reference line、宽度前缀和 direction 派生的中心线属于
geometry MIR，不是第二份共同 Typed AST 声明。

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

Geometry 前端的 numeric freeze 独占 authoring 曲线求值、确定性细分、station 切分、
lane/facility offset 和规范 `f32` 点生成，并把 evaluator 与 scratch 在 `finish` 返回前
释放。geometry MIR 拥有这些已冻结 payload 的有类型绑定、跨模块端点/方向验证、规范
排列、累计弧长、切向/上方向、Traffic length 与 Spatial sampling table；它不得再次
求值或生成另一份点。两者是一次编译管线中先后受检的私有阶段，不形成第二个公共 IR，
也不包含 target ABI、mesh、宿主坐标或运行时对象。

每个 `LaneEdge` 的可遍历中心线继续进入 `lane_edge_geometries`、`canonical_points` 和
`spatial_segments`。每个 `FacilityBand` 的不可遍历中心线进入独立的
`facility_band_geometries` 行；每行显式引用一个 `FacilityBandOrdinal`，并按该
FacilityBand 的完整 CanonicalIdentity 排列，精确保存其在同一 `canonical_points` 平面表
中的非空范围，但不产生 Traffic length、
`spatial_segments`、lane successor 或路线可遍历性。它参与 `GeometryPointCount`、
`LirRecordCount`、逻辑输出字节、完整输出 digest 与 LIR 语义指纹；target profile 可以在
emitter 中裁剪非权威表现数据，但 canonical LIR 不得丢弃该表。其来源位置与
FacilityBand ordinal 经过同一 permutation。`canonical_points` 的拼接顺序固定为：先按
LaneEdge ordinal 拼接全部 `lane_edge_geometries` 范围，再按 FacilityBand ordinal 拼接
全部 `facility_band_geometries` 范围；每个范围内部保持该曲线的规范参数顺序。

```rust
struct LirFacilityBandGeometry {
    facility_band: FacilityBandOrdinal,
    canonical_frame: CanonicalFrameOrdinal,
    points: TableRange<LirCanonicalPoint3F32>,
}
```

该表只覆盖 Geometry module 派生且拥有 offset intent 的 FacilityBand；Synthetic、
current-import 或后继官方前端声明但没有 Geometry intent 的 FacilityBand 不产生占位行，
因此它相对全局 `facility_bands` 可以稀疏。每个 Geometry 派生 FacilityBand 必须恰好一
行，同一 ordinal 不得重复。v1 不在该行保存 segment sampling、弧长、宽度副本或
Adapter 样式。宽度仍由 FacilityBand 语义记录拥有，几何行只冻结其已偏移中心线。

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
de Casteljau `t = 0.5` 细分、station 累计和 lane offset；每个标量表达式按本文从左到右
拆成独立 multiply/add/subtract/sqrt，不允许 FMA 收缩、重排或平台专用 fast-math。
输出点每个分量只在进入规范点表时按 IEEE 754 round-to-nearest,
ties-to-even 转为 `f32`，并把 `-0.0` 规范化为 `+0.0`。
JSON 十进制 token 必须先保留精确字节，再以 Rust 标准库 `str::parse::<f64>()` 的最近值
语义转换一次；溢出、非有限结果或不满足字段值域时失败，禁止先经通用 JSON number
或另一精度往返。整数和 `randomSeed` 等整数文本按目标 `u32`/`u64` 直接解析并检查，
不得经 `f64` 往返。

对 cubic Bézier，按原始段顺序深度优先、左子段优先递归。Reference curve 的位置停止
条件是两个内控制点到端点**有限线段**的最大三维平方距离不超过所选位置配置档的曲线
细分子预算平方；投影参数按 `dot(point-start, chord) / dot(chord, chord)` 求得并 clamp
到 `[0, 1]`，再按 `start + clamped * chord` 求最近点。三个子预算依次为
`Fine2Cm = 0.01 m`、`Balanced5Cm = 0.025 m`、`Compact10Cm = 0.05 m`。

方向停止条件独立检查区间两端曲线切向与端点弦。不得调用平台 `acos`：dot 必须为正，
并满足 `dot² >= tangentLength² × chordLength² × cosSquaredThreshold`。三个方向档的
冻结阈值为：

| 方向档         |   半角 |           `f64 cos²` |        IEEE 754 bits |
| -------------- | -----: | -------------------: | -------------------: |
| `Smooth1Deg`   | `0.5°` | `0.9999238475781956` | `0x3fefff604bfad7c5` |
| `Balanced2Deg` |   `1°` | `0.9996954135095479` | `0x3feffd813c5f82b4` |
| `Compact5Deg`  | `2.5°` | `0.9980973490458729` | `0x3feff069da0c0ad2` |

上述半角只决定 `f64` 细分候选，不能单独证明最终方向档。来源 segment 连接点两侧的
解析三维切向必须存在、其 XZ 投影必须非零，且三维夹角不得超过所选方向档。为保证
任意非零 offset 在共享 source 端点位置连续，还必须分别按 6.1 节冻结公式计算两侧
`left`，并要求 `left.x/left.y/left.z` 的规范化 `f64` bits 逐分量相同；仅角度在档位内
但 `left` bits 不同仍失败关闭。该约束不要求三维导数大小或坡度相同，但冻结了 v1 的
水平一阶方向连续性和数值端点唯一性。所有 station 强制点合并且坐标量化后，再按实际
`f32` 点形成的相邻非零三维弦检查内部折点；每个普通 successor 或 junction path
transition 还检查前驱末弦与后继首弦。
这些最终运行时弦的夹角分别必须不超过 `1°`、`2°`、`5°`，仍使用对应全角的冻结
`cos²`、正 dot 和含等号比较，不调用平台 `acos`。因此 ADR 0015 允许的表示角误差不会
叠加到公开方向档之外；量化后超限直接失败关闭，不以 f64 候选已通过为由接受，也不
snap 或平滑。零长度弦、零长度端点切向或水平投影切向为零直接失败。line 不因位置误差
细分，但仍可因 section/station 强制点产生多个受检弦。

| 方向档         | 最终全角 |           `f64 cos²` |        IEEE 754 bits |
| -------------- | -------: | -------------------: | -------------------: |
| `Smooth1Deg`   |     `1°` | `0.9996954135095479` | `0x3feffd813c5f82b4` |
| `Balanced2Deg` |     `2°` | `0.9987820251299122` | `0x3feff605b8b87ffc` |
| `Compact5Deg`  |     `5°` | `0.9924038765061041` | `0x3fefc1c5c6408e0c` |

最终方向检查先把每个规范 `f32` 坐标分量以 `f64::from(value)` 精确提升，再在 `f64` 中
按 `x`、`y`、`z` 顺序计算弦差。`dot(a,b)` 固定为
`((a.x*b.x) + (a.y*b.y)) + (a.z*b.z)`，`lengthSquared(v)` 复用同一顺序；每个乘法和
加法是独立操作，禁止 FMA。阈值只以表中 bits 调用 `f64::from_bits` 构造，不重新调用
三角函数或从十进制解析。零弦、正 dot 和 `dot² >= leftLength² * rightLength² * cos²`
也按本文从左到右执行，因此含等号/加一 ULP 在所有受支持平台得到同一判断。用于 join
比较的 `left` 分量先把 `-0.0` 规范化为 `+0.0`，其他值不做 epsilon、rounding 或 snap。

Station 参数表不使用调用方选择的位置或方向档。它逐原始 segment 建立：line 产生一个
区间；cubic 固定使用 `0.01 m` 内控制点有限弦距与 `0.5°` 端点切向门槛，以同样的
de Casteljau、深度优先、左子段优先算法细分。每个已接受 station 弦精确保存一行：

```text
StationInterval {
  segmentOrdinal,
  t0Bits,
  t1Bits,
  cumulativeStartLengthBits,
  cumulativeEndLengthBits
}
```

首行固定从 `(segmentOrdinal=0, t0Bits=0x0000000000000000,
cumulativeStartLengthBits=0)` 开始；每行 `t0 < t1` 且只属于一个 segment，下一行的累计
起点 bit 等于当前行累计终点 bit，且每行累计终点严格大于累计起点。累计长度按每行
两个 `f64` 求值点的三维欧氏弦长、从左到右依次相加；跨 segment 时下一行使用新
segment 的局部 `t0 = 0`，不从前一 segment 的 `t = 1` 向新 segment 的参数插值。该表
一旦形成即不可修改，也不进入 LIR。

对数值 station `s`，先验证 `0 <= s < finalStation`，再 lower-bound 查找第一行
`cumulativeEndLength >= s`。这使精确内部 segment 边界规范归属前一 segment 的 `t = 1`；
零点归属第一 segment 的 `t = 0`。否则只在找到的同一行内，以该行
`(t0, S0, t1, S1)` 按固定顺序计算
`alpha = (s - S0) / (S1 - S0)`、`t = t0 + alpha * (t1 - t0)`。`"end"` 唯一映射到
最后一行的 `t1`。强制参数在原 authoring evaluator 上求值，只在进入最终点表时量化
一次；它不回写 station 表、不重新累计 reference length，也不参与后续 station 定位。

每条常量偏移 `d` 的 lane/facility 曲线必须独立细分。求值器固定为
`O_d(t) = B(t) + d * left(B'(t))`，其中 `left = +Y cross normalize(project_XZ(B'))`；
切向使用该表达式的解析一阶导数，cubic 的 `B'` / `B''` 按 Bernstein 多项式固定顺序
求值，水平投影长度为零时失败。Offset 位置停止条件不使用有限采样猜测最大误差，而对
每个 de Casteljau 子区间的局部参数 `u in [0,1]` 计算 `O_d''` 的保守范数上界 `K`；线性
插值误差定理给出该 offset curve 到端点弦的最大距离 `<= K/8`，只有
`K <= 8 * positionSubBudget` 时才可接受。区间两端的 `O_d'` 还必须通过所选方向档的
同一半角判定。任一条件失败即在参数中点二分；因此 offset 不引用并不存在的 Bézier
内控制点，也不复用 reference curve 的接受结果。

对控制点 `p0..p3`，导数逐分量按下式括号和书写顺序求值：

```text
u = 1 - t
d10 = p1 - p0; d21 = p2 - p1; d32 = p3 - p2
B' = 3 * (((u * u) * d10) + (((2 * u) * t) * d21) + ((t * t) * d32))
B'' = 6 * (u * ((p2 - (2 * p1)) + p0) + t * ((p3 - (2 * p2)) + p1))
q = (B'.x, B'.z); q2 = q.x*q.x + q.z*q.z; r = sqrt(q2)
r' = (q.x*B''.x + q.z*B''.z) / r
left = (q.z/r, 0, -q.x/r)
left' = ((B''.z*r - q.z*r')/q2, 0, (-B''.x*r + q.x*r')/q2)
O_d' = B' + d*left'
```

向量乘加均按 `x, y, z` 分量分别执行；`q2 == 0`、任一中间值非有限或最终切向为零均
失败。Line 的 `B'' = 0`，其常量 offset 仍是 line，除 station 强制点外不细分。

`K` 由当前 de Casteljau 子曲线控制点 `p0..p3` 以以下固定保守界求得：

```text
v0 = 3*(p1-p0); v1 = 3*(p2-p1); v2 = 3*(p3-p2)
rx = distance(0, [min(v0.x,v1.x,v2.x), max(v0.x,v1.x,v2.x)])
rz = distance(0, [min(v0.z,v1.z,v2.z), max(v0.z,v1.z,v2.z)])
rMin = sqrt(rx*rx + rz*rz)
a0 = 6*((p2 - 2*p1) + p0); a1 = 6*((p3 - 2*p2) + p1)
j = 6*(((p3 - 3*p2) + 3*p1) - p0)
A = max(norm3(a0), norm3(a1))
M1 = max(normXZ(a0), normXZ(a1)); M2 = normXZ(j)
K = A + abs(d) * ((2*M2/rMin) + (6*M1*M1/(rMin*rMin)))
```

`distance(0,[lo,hi])` 在 `lo <= 0 <= hi` 时为零，否则为 `min(abs(lo),abs(hi))`。
Bernstein 凸包保证 `rMin` 不大于区间内任一水平 `|B'|`，并分别给出 `A/M1/M2` 的上界；
由单位向量二阶导数界 `|left''| <= 2*M2/rMin + 6*M1²/rMin²` 得到上述 `K`。
`rMin == 0` 的区间不能被接受，只能继续二分；深度 20 仍为零则以近垂直/退化切向失败。

最终输出参数集合由该 offset 自身接受区间端点、原始 curve segment 端点和 station
强制参数合并。只有通过上述 source join `left` bit 连续性后，曲线参数才规范化：初点
唯一为 `(0, 0)`；每个内部共享边界的
`(segment i, 1)` 与 `(segment i+1, 0)` 是同一个参数，并规范拥有为前一 segment 的
`(i, 1)`；终点唯一为最后 segment 的 `t = 1`。随后按该规范曲线顺序排序，只对同一
规范参数去重，从而每个物理 segment 边界恰好输出一次。再从原 evaluator 求值并量化；
不同规范参数量化到同一点或形成不大于最短段限制的线段时失败，不按坐标去重或移动
语义锚点。
递归深度上限为 20；达到上限仍不满足误差、产生超过 `GeometryPointCount` 的点、或
量化后任一线段长度不严格大于既有
`SPATIAL_MIN_SEGMENT_LENGTH_METERS = 0.1 m` 时失败关闭，不降低精度继续编译。

最终规范中心线相对 authoring/offset evaluator 的位置接受上限由配置档分别固定为
`2 cm`、`5 cm` 和 `10 cm`。所有 station/segment 强制参数合并后，生产 oracle 对每对
最终相邻参数重新证明完整区间，而不是只复用细分候选：line 的解析曲线到 f64 弦误差
为零；reference cubic 使用对应 de Casteljau 子曲线控制点到有限端点弦的最大距离；
offset 使用该参数区间重新计算的 `K/8`。再把两个 f64 端点各自到其最终 f32 点的三维
距离取最大值，与解析曲线到 f64 弦的保守界相加；只有该总界不超过所选总位置上限才
接受。该证明来自三角不等式和端点线性插值的凸组合界，覆盖整个连续参数区间，不是
有限采样或未经证明的预算常量相加。任一区间不通过即失败关闭；station 插点会拆分并
重新证明相邻区间，不能沿用插点前的 oracle 结果。

对不是既有二分边界的最终参数区间 `[t0,t1]`，oracle 以原 segment 控制点执行固定
de Casteljau crop：若 `t1 < 1`，先在 `t1` 分割并保留左子曲线；若 `t0 > 0`，再在保留
曲线的局部参数 `t0 / t1` 分割并保留右子曲线；`t1 = 1` 时第二次局部参数直接为 `t0`。
通用分割的逐分量 lerp 固定为 `a + t * (b - a)`，先减、再乘、再加，禁止 FMA。得到的
四个控制点同时供 reference hull 界和 offset `K` 界使用，不能重新拟合或有限采样。

剩余预算不得被 Adapter 展示细分或车辆物理偏差占用。九种位置/方向组合都不放宽语义锚点：曲线段
端点必须进入点表，station 边界按固定参数表插点，相连端点仍遵守 6.2 节的 `5 mm`
上限且不 snap。实现可以用迭代栈替代递归，但必须在相同配置档下产生逐 bit 相同的点
序列；更换任一配置档集合、算法、阈值或求值顺序需要提升 frontend/constraint 版本并重新
进入 G1。

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
- `TypedAstRecordCount` 只计入成功解析后可逐项枚举的逻辑领域记录：模块头、
  frame、road、reference curve segment、cross-section span、section、lane、lane group、
  facility、junction、connection、internal edge 及 overlay 中各共同声明/owner-local
  record；控制点是 curve segment 的字段，不另算记录。JSON token、标点、字段名、对象
  容器和数组容器均不进入该维度；
- 引用和 owner-local 成员分别计入既有 `ReferenceCount` /
  `RelationOccurrenceCount`；同一 wire
  字段只计入一个领域 count，不能同时作为 tokenizer occurrence 重复计数；
- `GeometryPointCount` 只计算准备进入最终规范几何表的量化 `f32` 点，并按 #315 的共同
  admission 语义跨模块累计；authoring start/control/end 点、station 表节点、offset
  evaluator sample 和细分候选不进入该 count；
- `finish` 的 numeric freeze 在构造 `AdmittedOfficialModule` 前得到上述精确点数和点
  payload；它先以模块自身 `CompileLimits` 检查单模块上限，再由 #315 common admission
  与 builder 已提交计数做 checked 累加和全单元上限检查。后继 HIR/MIR/LIR 只消费该
  payload，禁止 count-only 预跑、估算计数、二次生成或扫描重算；
- parser stack、duplicate-key table、曲线细分栈、station scratch、HIR/MIR 工作集分别
  计入 `StageScratchBytes`；
- 解析后存续的字符串、记录、span、点和模块包装计入真实
  `CompilerControlledLiveBytes`；
- 诊断条数继续受 `DiagnosticCount` 限制，达到保留上限后仍按安全边界完成计数，并
  只保留全局规范顺序最小前缀。

任何按输入规模分配必须先以 checked `u64` 计算 length+1/候选容量并比较上限；来源
长度还必须在哈希或解析前可收窄到描述符所需 `u32`。未知长度流不在 v1 API 范围内。
解析不得先构造无界通用 JSON value tree 再统计。#296 不增加生产解析/几何依赖；
实现使用 crate 私有的有界 JSON tokenizer/parser，直接产生紧凑记录与 span。v1 固定
`MAX_GEOMETRY_JSON_NESTING_DEPTH = 32`：根对象深度为 1，每次进入 object/array 前以
checked `u32` 计算下一深度，33 在压入 parser stack 或分配容器前返回结构化错误。解析
器只能使用受 `StageScratchBytes` 约束的显式栈，不得让不可信嵌套深度增长进程调用栈。
合法 schema 的最深实例、深度 32/33、以及类型错误后嵌套攻击都必须有边界测试。

### 7.2 诊断阶段与顺序

错误优先级固定为：

1. 来源字节/编码/顶层版本与 parser 资源错误；
2. JSON 语法、重复键、closed-shape 与字段类型；
3. 模块头、单位、稳定键、局部重复、曲线局部值域，以及 `finish` numeric freeze 的
   station、offset、细分、量化、最终方向、总误差与单模块 geometry 资源错误；
4. 共同 admission 的 namespace/document/资源错误；
5. HIR 来源归属、import/symbol/unit/owner 候选错误；
6. topology/geometry MIR 的 coverage、跨模块/拓扑端点与方向连续性、已冻结 geometry
   payload 绑定和全局静态语义；
7. Identity closure 与 LIR 规范化错误。

同一阶段按 `(canonical module order, source document order, primary span, diagnostic
code, stable key, typed payload)` 排序。解析恢复只能在当前 closed object/array 的已知
边界内进行；无法证明安全同步点时只报告首个语法错误，不能猜测后续结构。

阶段顺序按实际公共操作生效：`GeometryModuleBuilder::finish` 在模块存在前完成阶段 1–3，
其 numeric freeze 错误规范地遮蔽尚不可执行的 common admission、未知 import、跨模块
owner、配置档混用或全局连接错误；只有成功模块进入编译单元后才执行阶段 4–7。实现
不得缓存一个失败模块并继续后续阶段，也不得为了报告“更多”错误打破该顺序。G2 必须
覆盖同一来源同时含 local freeze 与未知 import、跨模块 owner、配置档混用错误的
known-vector，证明只返回阶段 3 的规范最小诊断。

任一错误级诊断使当前操作失败且不返回部分 Typed AST、HIR、MIR、LIR 或源映射。失败
后的 `Compiler` 可复用容量，但下一次编译的输出、诊断和指标不得受污染。

## 8. 正确性与等价验证矩阵

G2 实现至少必须自动覆盖：

| 类别       | 必须证明的样例                                                                                                                                                                                                                                                                                                                            |
| ---------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 来源       | schema 自检与最小/全字段 golden、BOM/非法 UTF-8、重复键、未知字段、旧/未来版本、精确字节摘要；LF/CRLF/CR、tab、多 byte Unicode 和跨行 value 的一基 UTF-8 byte 行列；摘要 63/64/65 字符、非 ASCII、非小写与非十六进制边界                                                                                                                  |
| 文档基数   | Geometry 单文档在 v1/v2 均成功；不存在 include；与多文档模块组合时 v1 在分配前失败、v2 成功                                                                                                                                                                                                                                               |
| API 原子性 | `new`、`finish`、`add_geometry_module` 每个失败点不泄漏部分模块或改变 unit builder                                                                                                                                                                                                                                                        |
| 标识       | 普通声明重排、无关插入、空白/字段重排和 geometry-only edit 不改变应稳定实体 ID；显式 key/owner/topology 改变按 Identity v1 改变                                                                                                                                                                                                           |
| topology   | cross-section span 完整 coverage、横向 elements、lane predecessor/successor、Junction 级 internal edge 声明、同 Junction 共享 transition 去重、跨 owner 冲突、path owner 和 owner-local occurrence                                                                                                                                        |
| geometry   | 九种位置/方向组合及两类混用拒绝、line/cubic 边界、深度 20、各子预算和三档最终 f32 方向阈值的固定提升/运算序/含等号/加一 ULP、source join `left` bit 连续性、edge join、各档逐区间总位置证明、单段/多段 station lower-bound/边界归属/插值 known vectors、offset `K` 界、lane/facility 独立点范围、语义锚点、最短段、近垂直、连接 5 mm 边界 |
| 来源映射   | 声明、派生 edge、owner-local relation 与 LIR 目标经过同一 permutation；HIR 局部顺序与 LIR Identity 顺序不同的反例                                                                                                                                                                                                                         |
| 等价       | 代表性受保护转向走廊与 #292 Synthetic DSL 产生相同 CanonicalIdentity、全部 LIR 表/关系/数值、语义指纹和 current 投影                                                                                                                                                                                                                      |
| 确定性     | 模块/普通声明、phase states 和其他无序集合重排、无关插入、重复 clean compile、不同优化级别和受支持平台产生相同 LIR 语义与点 bit pattern；全部显式有序数组重排按其语义改变或被 coverage 规则拒绝                                                                                                                                           |
| 资源       | 每个相关 limit 的边界/边界加一、`finish` 精确点数与 payload 一致、多个模块共同累计拒绝、Geometry+Synthetic 稀疏 Facility geometry、配置档混用最坏失败、诊断截断、parse/freeze/admission/build scratch 和失败清理                                                                                                                          |

等价比较不能只比较 current JSON 或对象数量；必须逐表比较有类型 ordinal、稳定身份
完整前像、owner-local relation、规范 `f32` 点 bit、累计弧长、来源键角色和语义指纹。

## 9. 性能与收益/代价

### 9.1 四个独立测量边界

G2 必须在 release、单工作线程、同机 base/candidate 下分别报告：

1. **Geometry parse/build**：从借用原始 bytes 到 `GeometryModuleBuilder`；包含一次
   SHA-256、有界解析、span 和紧凑曲线前端记录，不包含 numeric freeze；
2. **Geometry numeric freeze**：对已构造 builder 执行 `finish`；包含字段/单位/局部引用
   校验、专用降阶、station/offset/细分/量化、最终方向与总误差证明及完整模块计数；
3. **共同 admission**：对已构造 `GeometryModule` 执行 `add_geometry_module + build`；
   不把 parse/curve 成本归因给 #315；
4. **完整 compile**：原始 Geometry bytes 到 `CompilationOutput`；包含 HIR/MIR/LIR 和
   source-map，作为真实产品成本。

每级至少一个预热样本、七个正式样本、三个独立进程，报告每进程中位数/MAD及中位数
的中位数；同时报告原始字节、模块/文档/声明/引用/关系/曲线段/控制点/规范点/LIR 记录、
逻辑输出字节、编译器控制峰值、保留容量与语义指纹。不得把 parser-only 数字、受控
分配记录或操作系统进程内存互相替代。

完整 compile 的编译器控制峰值取同一实际生命周期中 Geometry 模块构建器（builders）
的累计存续/finish 峰值与既有 HIR/MIR/LIR/source-map 后端峰值的最大值。`CompilationMetrics` 的
后端值不能单独替代前端峰值；多个 builder 在 finish 前同时存续时必须累计，已经
finish 的模块与尚未 finish 的 builder 也按实际共存关系入账。

### 9.2 工作负载

Geometry 性能仍采用绝对硬门槛；但门槛只对 exact fixture、exact harness 和具名参考机
成立，不能只由以下自然语言名称取得资格。初始 G1 Pass 冻结本节的 workload identity、
测量协议、机器可读 schema、候选预算和失败关闭规则，不要求尚不存在的 production
实现提前生成运行结果。G2 必须提交机器可读
`docs/reference/geometry-frontend-calibration-contract-v1.json`，它以 byte length 与
SHA-256 绑定 workload manifest、证据 schema 和 reference-machine declaration；这些
exact 工件必须在 G3 Pass 前取得资格。Workload
manifest 必须通过
[`geometry-frontend-calibration-workload-manifest-v1.schema.json`](../reference/geometry-frontend-calibration-workload-manifest-v1.schema.json)
校验，并由独立 cross-record validator 证明恰好覆盖 `3 workloads × 3 position profiles ×
3 direction profiles = 27` 个唯一 row；manifest
中的每个 workload/profile row 至少精确包含：

```text
schemaVersion, workloadId, fixturePath, fixtureByteLength, fixtureSha256
accuracyProfileCode, directionProfileCode
module/document/declaration/reference/relation counts
line/cubic/control-point counts
offset-curve count and absolute-offset distribution
expected canonical-point count, logical output bytes and per-LIR-table counts
semantic fingerprint and complete-output digest
```

Cross-record validator 还必须验证同一 workload 的九行绑定相同 fixture bytes、offset
distribution 的 `curveCount` 总和等于 `offsetCurveCount`、全部 LIR table count 之和等于
`lirRecordCount`，并从实际编译输出重算 canonical point/LIR counts、
`logicalOutputBytes` 与两个 digest；manifest 自报值不能作为 oracle 自证。

`lirTableCounts` 精确使用 schema 中登记的 53 个 record-counted LIR 表名；每行必须包含
全部键，包括零计数表，且拒绝未知键。该 v1 registry 包含 #296 新增的
`facility_band_geometries`，不包含独立资源维度中的 `identity_fields` 或 byte blob；若 G2
实现改变 record-counted 表集合，必须先修改 schema 并返回 G1，不能由 validator 接受
动态表名。

证据还必须绑定 measurement/harness commit、clean tree、`Cargo.lock`、release binary digest、
操作系统/CPU/内存/电源与固件身份、计时量子、预热/正式样本和进程数，并保存原始执行
制品摘要。校验顺序复用 #308 的 trusted contract → schema/manifest exact bytes → evidence
cross-record validation，禁止先信任 evidence 自报的 manifest。

发布二进制摘要（release binary digest）的 exact bytes 核对在 `measure`/`assemble` 的
参考机链路中强制执行；
提交后的离线 `verify` 必须复核 raw/evidence 内二进制身份绑定一致，但不要求 Git 忽略的
历史 `target/` 文件继续存在。审计方持有历史二进制时可显式提供路径并再次核对长度与
SHA-256；二进制本身不作为普通 Git 制品提交。

冻结三个 Geometry workload identity：

- `LF-COMP-GEOMETRY-MIN-v1`：单 frame、单 road、单 cross-section span、单
  RoadSection、两条 lane、line-only；
- `LF-COMP-GEOMETRY-CORRIDOR-v1`：与 #292 代表性受保护转向走廊语义等价，包含 cubic
  connection、signals、parking、access、routes、Gate/WaitingZone；
- `LF-COMP-GEOMETRY-P100-v1`：与 #292 P100 五级最高级相同的合法静态语义规模，并
  保持相同模块图和 LIR 记录数；Geometry 特有计数另行报告。

每个含曲线的工作负载必须运行九种位置/方向组合，报告相同来源下的规范点、逻辑输出、
耗时和内存差异；line-only MIN 也运行九种组合并验证它们只改变选项摘要、不制造多余点。
同语义 Synthetic base 只用于分离 Geometry 固有解析/细分成本，不作为“新文本前端只能
增加百分之十”的错误门槛。

当前 `25 ms` 完整 compile、`6 MiB` 编译器控制峰值和 `0 B` 冷实例保留容量仅为初始 G1
冻结的候选预算，不是已取得资格的最终硬门槛。G2 在 production 实现存在后，必须在
上述 exact manifest 与参考机上完成三进程校准，并把每个门槛及其统计量、预算来源和
适用的九种组合写回 contract/evidence。随后必须新增 append-only 的 G1 calibration
closure：若证据支持候选预算且不改变算法、schema、workload identity 或测量协议，只
接受 exact 数值和证据绑定；若不支持或需要改变任一设计输入，则完整返回 G1 重新审阅。
只有该 calibration closure 与九种组合硬门槛全部通过，#296 才能取得 G3 Pass。相同
组合的后继候选若出现可重复、无法解释的时延或峰值超过已校准基线 `5%`，同样阻断。
不得删除 span、资源检查或偷偷切换到更粗配置档来达标。共同 admission 自身继续满足
#315 的 `5%` 回归门槛。

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
