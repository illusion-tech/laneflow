# 路网编译器与目标静态镜像

**文档状态**: Draft（#291 G1 综合架构修订）<br>
**最后更新**: 2026-07-28<br>
**适用范围**: 权威来源模块图（Authoritative Source Module Graph）、有类型中间表示
（Typed IR）、静态网络编译权威、标识派生、可移植规范制品（Portable Canonical
Artifact）、目标静态镜像（Target Static Image）、源映射（Source Map）、语义差异
（Semantic Diff）、独立验证器（Independent Validator）、交通运行时（Traffic
Runtime）命名和当前态（Current）→目标态（Target）迁移<br>
**实现状态**: 未实现；当前态生产路径仍使用 Traffic v0.10 / SpatialPackage v0.1 /
ScenarioManifest v0.1、`InitialTrafficData` 和现有空间登记表（Spatial Registry）；
#292 已重划为编译器基础设施（Compiler Foundation）+ 合成领域专用语言前端
（Synthetic DSL Frontend），并继续被 #291 G1 阻断

**关联决策与设计**:

- `../adr/0003-runtime-tick-and-determinism.md`
- `../adr/0005-core-identity-and-handle-model.md`
- `../adr/0007-traffic-data-crate-and-loader-boundary.md`
- `../adr/0008-pre-1.0-data-format-version-policy.md`
- `../adr/0011-schema-identifier-and-publication-contract.md`
- `../adr/0013-engine-neutral-spatial-geometry-and-length-authority.md`
- `../adr/0015-bounded-f32-canonical-spatial-frames.md`
- `../adr/0017-static-road-junction-maneuver-and-gate-identity.md`
- `../adr/0020-compiler-owned-static-network-and-static-image.md`
- `core-id-handles.md`
- `data-format.md`
- `data-loading.md`
- `spatial-geometry.md`
- `road-junction-model.md`
- `signal-system.md`
- `waiting-zone-conflict-right-of-way.md`
- `../reference/glossary.md`

## 术语规范

本文的中文术语和中文定义是权威事实，英文只作辅助理解。完整双语映射以
`../reference/glossary.md` 为 SSOT；本文不另建竞争词表。类型、crate、字段、版本
值、算法和协议常量等精确标识符使用反引号保留原文，但其语义由中文正文裁决。

本文的主链路按中文规范术语表述为：

```text
权威来源模块图（Authoritative Source Module Graph）
  -> 有类型抽象语法树（Typed AST）
  -> 高层中间表示（HIR）
  -> 中层中间表示（MIR）
  -> 已验证规范低层中间表示（Validated Canonical LIR）
  -> 可移植规范制品（Portable Canonical Artifact）
     + 目标静态镜像（Target Static Image）
     + 源映射（Source Map）
     + 语义差异（Semantic Diff）
```

## 1. 结论与状态

#291 冻结的目标不是“生成另一份 JSON 的工具”，而是新的静态数据体系：

下图为了对应精确类型名和代码搜索保留英文辅助别名；中文规范链路以上一节为准。
图中英文不能独立改变架构语义。

```text
Geometry modules ───────┐
Synthetic DSL modules ──┼─> authoritative source module graph
Imported modules ───────┤                 │
Editor-authored modules ┘                 v
                           typed AST -> HIR -> MIR -> validated canonical LIR
                                                          │
                                                          ├─> portable canonical artifact
                                                          ├─> target StaticNetworkImage
                                                          ├─> source map / diagnostics
                                                          └─> semantic diff

StaticNetworkImage ─┬─> Traffic Runtime: StaticTrafficView + per-world mutable state
                    └─> Spatial: optional StaticSpatialView + batch scratch/output
```

核心结论：

- 取消 L1/L2 作为架构层；不同来源语言（Source Language）/来源模块是平级前端
  输入；
- 编译单元的唯一数据编制权威是可重放的权威来源模块图，几何文档只是生产场景的
  主要来源语言；
- 编译器是全部静态网络的唯一编译权威；
- `InitialTrafficData` 和 Core 登记表不是中间表示；
- AST/HIR/MIR/LIR 逐级降阶，只有已验证规范 LIR 可以进入编译发射器；
- 可移植规范制品与目标静态镜像是同一 LIR 的不同后端；
- 目标态 `laneflow-runtime` / 空间层直接消费同一不可变镜像中的对齐视图；
- 静态只读数据与每世界可变状态物理分离；
- 生产启动不再解析 JSON、按字符串重绑定、重建登记表或重新编译初始路线出现项；
- 独立验证器不复用编译器的语义验证实现；
- 稳定声明/可寻址派生实体、所有者局部出现项与密集 LIR 表行使用不同标识；
- 规范标识元组、BLAKE3-128 `StableId128`、XXH3 瞬态加速边界和密集句柄保持
  热路径分层；
- 可信生产快速路径必须有镜像外部的验证/发布信任锚，不能把 header 中可伪造的
  规范摘要/provenance 当成可信证据；
- `StaticTrafficImage` 是必选节，`StaticSpatialImage` 是配置档控制的可选节，无图形
  交通运行时不携带几何。

本文描述目标态。ADR 0020 Accepted 且迁移 G4 完成前，现有
JSON/Data/Core/Spatial 路径仍是当前态生产契约。

## 2. 为什么不能继续 L1/L2

原草案把 scenario builder DSL 定义为 L1，把 geometry compiler 定义为 L2，并让
L1 先输出 Core 输入。这会产生三个结构性错误：

1. **frontend 被误当作 lowering level**：Synthetic DSL 并不比 Geometry/OSM/Editor
   “更低”或“更高”；它们只是不同来源；
2. **Core object graph 泄漏到 compiler 中层**：`InitialTrafficData` 已经完成 Core
   handle 解析、registry/rebind 和 route occurrence 编译，无法作为 target-neutral
   输入；
3. **联合静态事实被拆开**：拓扑、几何、标识、长度和 Gate/Waiting
   coverage 只有在同一个 MIR/LIR 中才能一次性裁决，先 Core 后 Spatial 必然重复
   join 和校验。

因此 #292 的 Synthetic DSL 是 compiler frontend 的首个纵向验证，不是 L1。未来
Geometry、OSM 或 Editor frontend 不依赖 #292 的 DSL 语法或 Core-shaped output，
只依赖共同的 typed AST/HIR contract 和 compiler passes。

## 3. 当前生产与目标边界

| 关注点                           | 当前态生产路径（Current Production）                          | 目标态（Target）                                          |
| -------------------------------- | ------------------------------------------------------------- | --------------------------------------------------------- |
| 数据编制（Authoring）            | 手写规范 JSON + corridor generator 内部 TOML/DTO              | 权威来源模块图；几何文档是主要生产来源语言                |
| 交通加载（Traffic Load）         | JSON → private DTO → Core constructors → `InitialTrafficData` | 可信描述符 + 结构校验器 → `StaticTrafficView`             |
| 空间加载（Spatial Load）         | JSON + manifest → external ID bind → `SpatialRegistry`        | 配置档含 Spatial 时挂载 `StaticSpatialView`               |
| 标识（Identity）                 | 外部字符串在加载期解析为句柄                                  | 编译器生成 StableId128；镜像使用密集 `u32` 句柄           |
| 静态出现项（Static Occurrence）  | 初始/动态 Route 注册时由 Core 编译                            | 初始/静态出现项在镜像中预编译；动态 Route 由 Runtime 编译 |
| 治理制品（Governance Artifact）  | 精确当前版本 JSON Schema/fixture                              | 可移植规范制品 + 源映射 + 语义差异                        |
| 性能制品（Performance Artifact） | JSON object graph 规范化后的登记表                            | 目标/布局/配置档专用的不可变静态镜像                      |
| 验证（Validation）               | schema + loader + Core/Spatial constructors                   | 编译器 + 独立验证器/收据 + 有界结构校验器                 |

迁移不得把目标态写成现状，也不得为了复用当前态 DTO/constructor 而冻结错误的
编译器中间表示。

## 4. 权威职责（Authority）

### 4.1 数据编制权威（Authoring Authority）

一个 compilation unit 的唯一 authoring authority 是显式、可重放的
**authoritative source module graph**，不是某一种固定文档格式。每个 module 至少
冻结：

```text
moduleNamespaceId
sourceLanguage
sourceContentDigest
frontendVersion
frontendOptionsDigest
origin / provenance
imports
```

- Geometry document 是生产场景的主要 source language，但不是唯一 SSOT；
- Synthetic DSL source 是测试、fixture、benchmark 和示例 module 的权威输入；
- import module 必须记录原始 source bytes/digest、importer build、选项和 provenance；
  选择 materialize 为 Geometry module 时，必须显式记录 authority 切换；
- editor 默认编辑并持久化 Geometry module；只有拥有独立、可重放 source format 时
  才成为独立 frontend；
- programmatic generator 若参与可发布 compilation，必须记录 build ID、参数、
  seed、namespace 和输入 digest；匿名 AST 注入只能用于非发布测试。

typed AST/HIR/MIR/LIR 都是 source graph 的派生物。Portable artifact 是经过验证的
canonical publication contract，target static image 是可重建性能制品；二者都不得
反向覆盖 source module。

### 4.2 编译器权威（Compiler Authority）

compiler 唯一负责静态网络的：

- symbol/reference/unit resolution；
- topology/geometry 展开与全局语义；
- 稳定声明/可寻址派生实体的 StableId128 与全部 LIR row 的 deterministic ordinal；
- dense logical ordinal、owner/member/reverse indexes；
- Traffic/Spatial 长度共同派生；
- initial/static Route/Maneuver/Gate/Waiting occurrence；
- portable artifact 和 static image emission。

### 4.3 运行时权威（Runtime Authority）

- Traffic Runtime：fixed tick、vehicle、dynamic Route lifecycle、controller clock、grant/
  reservation、parking occupancy 和所有可变交通状态；
- Spatial：canonical geometry sampling 与 pose batch；
- Adapter：宿主 entity、Transform、frame placement、presentation lifecycle；
- static image：只读静态事实，不持有可变 authority。

## 5. 前端架构（Frontend Architecture）

### 5.1 合成领域专用语言前端（Synthetic DSL Frontend，#292）

主要用途：

- 测试、fixture、benchmark 和示例场景；
- 用少量参数展开规则走廊、网格、路口与交通配置；
- 作为 compiler 全纵向管线的首个可执行 frontend。

它必须输出带稳定 authoring key 和 source span 的 typed AST，不得：

- 直接构造 `InitialTrafficData` 或 `SpatialRegistry`；
- 跳过 HIR/MIR/LIR validation；
- 使用 Rust 容器遍历顺序作为标识/顺序；
- 把 builder-only TOML/Rust type 公开为 interchange contract。

### 5.2 几何文档前端（Geometry Document Frontend）

长期生产 authoring frontend。目标模型包含：

1. 参考线：三维 curve segments、弧长与方向；
2. 横断面：沿参考线分段变化的 lane/facility 结构；
3. 连接：junction/connection intent、默认生成策略与显式 override；
4. 规则：signals、Gate/WaitingZone、access、parking 和其他静态 overlay。

曲线在 MIR 中按确定性误差预算离散为 canonical f32 polyline；static image 不保存
authoring curve evaluator。具体 curve segment 集合由独立 numeric/authoring G1
和 benchmark 冻结，不在 #291 先选 library。

### 5.3 导入与编辑器编制（Import and Editor Authoring）

- importer 保存来源 provenance，必须显式生成稳定 key；不允许用导入遍历 ordinal
  冒充标识；
- editor 直接编辑并持久化 source module，诊断以 source span/画布 selection 回传；
- importer/Editor 不维护私有 semantic compiler，所有 module 都进入共同 HIR；
- publishable compile 不接收没有 owning module、source span/provenance 或稳定
  namespace 的匿名 AST。

## 6. 有类型中间表示与编译遍边界（Typed IR and Pass Boundaries）

### 6.1 有类型抽象语法树（Typed AST）

保留 frontend 语法与来源：

- explicit/derived declarations；
- stable authoring key；
- source span、file/module provenance；
- typed number token 与 unit；
- 尚未解析但已分类的 reference。

AST 不含 Core handle、runtime slot、target ABI 或 compiler 推断出的最终 geometry。

### 6.2 高层中间表示（HIR）

完成：

- module/import/namespace resolution；
- symbol table 与 typed reference；
- unit normalization；
- defaults 的显式化；
- authoring semantic category 与 overlay merge。

HIR 仍能追溯全部 source span。跨 module 重名、引用 cycle、unit/type 错误在此失败。

### 6.3 中层中间表示（MIR）

完成全局静态语义：

- corridor/section/lane 展开；
- boundary、edge、junction、movement、path 生成；
- curve tessellation、canonical frame partition 和 geometry continuity；
- signals、Gate、WaitingZone、parking、access 与 topology 绑定；
- 规范标识元组构造与标识闭包（Identity Closure）；
- Traffic length / Spatial arc length 共同派生；
- route/path occurrence 和 reverse indexes；
- global ownership、coverage、coherence 与 policy-independent safety checks。

MIR 可以使用 compiler arena 和临时 cache，但不得产生 target layout 或把 hash
fingerprint 当持久标识。

### 6.4 已验证规范低层中间表示（Validated Canonical LIR）

LIR 是 emitter 唯一输入：

- 所有 table row 都有 typed `u32` logical ordinal；
- 稳定 declaration 和可独立寻址的 derived entity 另有 StableId128；
- owner-local relation/occurrence 使用 typed local key，不获得全局 StableId128；
- 所有引用已转为 typed ordinal；
- 数值已规范化到 canonical units/representation；
- 所有 relation 以 deterministic flat sequence/range 表达；
- 静态 occurrence、sampling tables 和 layout-independent precompute 已完成；
- source map key 与 semantic diff key 已冻结；
- 未解决引用、隐式默认或需要后端判断的语义均为零。

LIR 使用 `u32` logical ordinal；超出容量必须结构化失败，不能升级为平台相关
`usize` 后继续编译。

### 6.5 推荐编译遍顺序（Recommended Pass Order）

```text
parse/type
  -> upgrade authoring format
  -> resolve namespace/module/symbol/unit
  -> expand cross-section and synthetic constructs
  -> construct topology and geometry
  -> bind signals/parking/access/waiting
  -> derive canonical identity
  -> validate global semantics
  -> normalize deterministic LIR order
  -> precompute occurrence/index/sampling data
  -> freeze validated canonical LIR
  -> emit all artifacts atomically
```

pass 可以并行或增量执行，但 clean single-thread compile 是确定性 oracle；任何模式
都必须生成相同 portable artifact 和 semantic diff。

## 7. 标识派生契约（Identity v1）

### 7.1 三类标识

标识权威是可审计的**规范标识元组（Canonical Identity Tuple）**，不是 UUID/哈希值本身。编译管线
必须区分：

| 对象类别                                                                  | 标识形式                                                          | 典型用途                                                              |
| ------------------------------------------------------------------------- | ----------------------------------------------------------------- | --------------------------------------------------------------------- |
| 稳定声明 / 可独立寻址派生实体（Stable Declaration / Addressable Derived） | `StableId128` + 有类型逻辑序号（Typed Logical Ordinal）           | 跨编译引用、源映射（Source Map）、语义差异（Semantic Diff）、发布审计 |
| 所有者局部关系 / 出现项（Owner-local Relation / Occurrence）              | `(ownerOrdinal, role, localIndex)` 有类型键（Typed Key）          | 路线出现项（Route Occurrence）、相位状态、成员关系 / 区间             |
| 所有低层中间表示 / 静态镜像表行（All LIR / Static-image Table Rows）      | 表类型专用的有类型 `u32` 逻辑序号（Table-specific Typed Ordinal） | 编译发射、交叉索引（Cross-index）、运行时热路径（Runtime Hot Path）   |

`StableId128` 不进入每个 relation、sampling point 或 occurrence。任何需要跨编译
稳定引用、独立 source mapping 或 entity-level add/remove semantic diff 的对象都
不能只靠当前数组 ordinal。Owner-local record 只拥有本次 snapshot 的位置地址，并在
stable owner 内执行 sequence diff；`localIndex` 不是稳定 anchor。Runtime hot state
只保存 typed dense handle；StableId128、字符串和 hash lookup 不进入 Traffic tick
或 Spatial pose batch。

### 7.2 规范标识元组（Canonical Identity Tuple）与稳定锚点（Stable Anchor）

```text
CanonicalIdentity =
  authoringNamespaceId
  + entityKind
  + stable parent anchors
  + stable local anchors
```

`authoringNamespaceId` 属于 source module，而不是 import path 或 compilation-unit
ordinal。稳定 key 在首次创建时写入 authoritative source module；复制 module 若
需要新标识域，必须显式创建 namespace。

允许的 anchor 只能是显式、持久化的 ASCII key 或已经派生完成的 parent
StableId128。显示名称、坐标、浮点几何、采样点、数组下标、横向/纵向序号、容器
迭代顺序、自动分类结果、import traversal ordinal 和全局自增值都不得成为 anchor。

- helper 只能从稳定 parent/local key 和 semantic role 组合 child key；
- sibling 重排与无关实体插入不改变既有 ID；
- compiler 推断出尚无稳定 key 的 junction/boundary/connection 时，只能产生待确认
  suggestion/diagnostic，不能发布匿名标识；
- authored relation target 的变化通常属于同一 declaration 的 semantic change；
  只有本节明确列入 tuple 的 topology closure 才改变 StableId128。

### 7.3 标识 v1 登记表（Identity v1 Registry）

`identityEncodingVersion = 1` 冻结公共字节 envelope；
`identityRegistryRevision = 1` 冻结本表的 kind、slug 和 required tag sequence。
required tags 必须按数值严格递增编码：

| 代码（Code） | `entityKind`           | 类别（Category）                      | 英文短名（Slug）    | 必需标签（Required Tags） |
| -----------: | ---------------------- | ------------------------------------- | ------------------- | ------------------------- |
|            1 | `RoadCorridor`         | 声明（Declaration）                   | `corridor`          | `1,2`                     |
|            2 | `RoadSection`          | 声明（Declaration）                   | `section`           | `1,2,3`                   |
|            3 | `AuthoringLane`        | 声明（Declaration）                   | `lane`              | `1,2,3,4`                 |
|            4 | `RoadLaneEdge`         | 可寻址派生实体（Addressable Derived） | `road-edge`         | `1,2,3,4,5,6`             |
|            5 | `JunctionInternalEdge` | 可寻址派生实体（Addressable Derived） | `internal-edge`     | `1,5,6,7,15`              |
|            6 | `Junction`             | 声明（Declaration）                   | `junction`          | `1,7`                     |
|            7 | `Movement`             | 声明（Declaration）                   | `movement`          | `1,7,9,10,11`             |
|            8 | `ManeuverPath`         | 声明（Declaration）                   | `path`              | `1,8,12,13,14`            |
|            9 | `ManeuverGate`         | 声明（Declaration）                   | `gate`              | `1,16,17`                 |
|           10 | `WaitingZone`          | 声明（Declaration）                   | `waiting-zone`      | `1,16,18`                 |
|           11 | `StopLine`             | 声明（Declaration）                   | `stop-line`         | `1,19`                    |
|           12 | `SignalGroup`          | 声明（Declaration）                   | `signal-group`      | `1,20`                    |
|           13 | `SignalController`     | 声明（Declaration）                   | `signal-controller` | `1,21`                    |
|           14 | `SignalPhase`          | 声明（Declaration）                   | `signal-phase`      | `1,22,23`                 |
|           15 | `ParkingArea`          | 声明（Declaration）                   | `parking-area`      | `1,24`                    |
|           16 | `ParkingSpace`         | 声明（Declaration）                   | `parking-space`     | `1,25,26`                 |
|           17 | `LaneGroup`            | 声明（Declaration）                   | `lane-group`        | `1,27,34`                 |
|           18 | `FacilityBand`         | 声明（Declaration）                   | `facility-band`     | `1,28,35`                 |
|           19 | `ParticipantClass`     | 声明（Declaration）                   | `participant-class` | `1,29`                    |
|           20 | `AccessRule`           | 声明（Declaration）                   | `access-rule`       | `1,30`                    |
|           21 | `VehicleProfile`       | 声明（Declaration）                   | `vehicle-profile`   | `1,31`                    |
|           22 | `StaticRoute`          | 声明（Declaration）                   | `static-route`      | `1,32`                    |
|           23 | `CanonicalFrame`       | 声明（Declaration）                   | `canonical-frame`   | `1,33`                    |

RoadSection lane 可展开为多条 `RoadLaneEdge`，稳定 boundary key 区分同一 lane chain
中的 segment。Junction internal edge 使用 junction-scoped `internalEdgeKey`。
Movement 的 left/straight/right/u-turn 分类是可重算元数据，不参与标识。
Signal phase、ParkingSpace、LaneGroup 和 FacilityBand 使用 parent StableId，而不是
当前 parent ordinal。StaticRoute 只表示编译期 authoring route；runtime 注册的
dynamic Route 继续使用 generation-aware handle，不获得持久 StableId128。

`ConflictZone`、`ParticipantStream`、`JunctionGroup` 等未来 domain 只有在各自 G1
冻结后才 append 新 kind code。新增 kind 提升 `identityRegistryRevision`，但不改变
既有 kind 的 bytes/ID；修改既有 kind 的 required field、tag 含义或编码必须提升
`identityEncodingVersion`。

### 7.4 字段标签登记表（Field Tag Registry）

| 标签（Tag） | 字段（Field）              | 编码（Encoding）           |
| ----------: | -------------------------- | -------------------------- |
|           1 | `authoringNamespaceId`     | ASCII 字节（Bytes）        |
|           2 | `corridorKey`              | ASCII 字节（Bytes）        |
|           3 | `sectionKey`               | ASCII 字节（Bytes）        |
|           4 | `laneKey`                  | ASCII 字节（Bytes）        |
|           5 | `startBoundaryKey`         | ASCII 字节（Bytes）        |
|           6 | `endBoundaryKey`           | ASCII 字节（Bytes）        |
|           7 | `junctionKey`              | ASCII 字节（Bytes）        |
|           8 | `pathKey`                  | ASCII 字节（Bytes）        |
|           9 | `movementKey`              | ASCII 字节（Bytes）        |
|          10 | `directedEntryApproachKey` | ASCII 字节（Bytes）        |
|          11 | `directedExitApproachKey`  | ASCII 字节（Bytes）        |
|          12 | `movementStableId`         | 16 个原始字节（Raw Bytes） |
|          13 | `entryEdgeStableId`        | 16 个原始字节（Raw Bytes） |
|          14 | `exitEdgeStableId`         | 16 个原始字节（Raw Bytes） |
|          15 | `internalEdgeKey`          | ASCII 字节（Bytes）        |
|          16 | `maneuverPathStableId`     | 16 个原始字节（Raw Bytes） |
|          17 | `gateKey`                  | ASCII 字节（Bytes）        |
|          18 | `waitingZoneKey`           | ASCII 字节（Bytes）        |
|          19 | `stopLineKey`              | ASCII 字节（Bytes）        |
|          20 | `signalGroupKey`           | ASCII 字节（Bytes）        |
|          21 | `signalControllerKey`      | ASCII 字节（Bytes）        |
|          22 | `signalControllerStableId` | 16 个原始字节（Raw Bytes） |
|          23 | `phaseKey`                 | ASCII 字节（Bytes）        |
|          24 | `parkingAreaKey`           | ASCII 字节（Bytes）        |
|          25 | `parkingAreaStableId`      | 16 个原始字节（Raw Bytes） |
|          26 | `parkingSpaceKey`          | ASCII 字节（Bytes）        |
|          27 | `laneGroupKey`             | ASCII 字节（Bytes）        |
|          28 | `facilityBandKey`          | ASCII 字节（Bytes）        |
|          29 | `participantClassKey`      | ASCII 字节（Bytes）        |
|          30 | `accessRuleKey`            | ASCII 字节（Bytes）        |
|          31 | `vehicleProfileKey`        | ASCII 字节（Bytes）        |
|          32 | `routeKey`                 | ASCII 字节（Bytes）        |
|          33 | `canonicalFrameKey`        | ASCII 字节（Bytes）        |
|          34 | `roadSectionStableId`      | 16 个原始字节（Raw Bytes） |
|          35 | `roadCorridorStableId`     | 16 个原始字节（Raw Bytes） |

Boundary/Approach/curve segment 若成为独立 LIR table、可被引用或需要独立 semantic
diff，必须通过后续 registry revision 获得 kind；否则只能作为所属 declaration 的
typed anchor/value，不得拥有隐式序号标识。

### 7.5 规范字节编码（Canonical Byte Encoding）与 `StableId128`

v1 编码不依赖语言对象布局或 JSON：

```text
"LFID"                           // 4-byte magic
u16_le(identity_encoding_version = 1)
u16_le(entity_kind)
u16_le(field_count)
repeated {
  u16_le(field_tag)
  u32_le(byte_length)
  exact_field_bytes
}
```

对给定 `entityKind`：

- `field_count` 必须等于 §7.3 required tags 的数量；
- tag 必须与 required sequence 完全相同并严格递增；
- missing、duplicate、unknown、out-of-order tag 全部拒绝；
- 16-byte StableId 字段长度必须精确为 16；
- 字符串必须符合 external ID 约束、大小写敏感，不 trim、case-fold 或 Unicode
  normalize，空字符串非法。

持久化算法：

```text
StableId128 =
  first_16_bytes(BLAKE3(ascii("laneflow.stable-id.v1\0") || canonical_bytes))
```

`\0` 是一个零字节 domain separator。文本形态为
`lfid1_<external-slug>_<32 lowercase hex>`。

### 7.6 算法角色、碰撞与测试

- **BLAKE3-128**：唯一持久化实体标识摘要；
- **XXH3-64/128**：仅用于 compiler 进程内 hash table/cache/fingerprint；命中后
  必须比较完整 canonical tuple，不得进入 artifact 引用；
- **XXH64/FNV64**：不作为持久标识；
- **SHA-256**：承担 artifact/image/publication integrity，不与实体标识摘要
  混用。

compiler 维护 `StableId128 -> CanonicalIdentity + owning span` 登记。两个 stable
entity 产生同一 tuple 返回 `DuplicateCanonicalIdentity`；相同 digest 对应不同 tuple
返回 `IdentityDigestCollision`。不得追加 ordinal、salt 或 suffix 静默修复。

标识测试（Identity Tests）必须包含：

- 每个 revision-1 kind 至少一个跨平台 known vector；
- missing/duplicate/unknown/out-of-order tag 负向向量；
- sibling reorder、无关 insertion 和 geometry-only edit metamorphic tests；
- section split、boundary/key 和显式 topology closure 变化测试；
- compiler、independent validator 和至少一个独立语言/脚本 oracle 的 bytes/ID
  一致性。

## 8. 制品与源映射（Artifact and Source Map）

### 8.1 可移植规范制品（Portable Canonical Artifact）

平台无关、确定性、closed shape，并包含：

- canonical format、identity encoding/registry 与 constraint versions；
- logical entities、typed ordinals、normalized numeric values；
- topology/geometry/static rule relations；
- canonical payload envelope 与 compiler provenance；自身 digest 不嵌入 artifact
  bytes。

它用于 publication、长期审计、migration、跨实现 validator 和 static image
regeneration。它不是 mmap hot layout，不承诺与 Rust struct ABI 相同，也不因某个
target profile 缺少 Spatial/cold section 而丢失 canonical semantics。

### 8.2 目标静态镜像（Target Static Image）

按 `targetTriple + staticImageLayoutVersion + staticImageProfileId` 生成：

```text
StaticNetworkImage
  Header
  SectionDirectory
  Required: StaticTrafficImage
    hot SoA tables
    CSR adjacency / flat ranges
    precompiled route/path/gate/waiting occurrences
  Optional: StaticSpatialImage
    frame/edge-aligned geometry tables
    flat points / cumulative arc / sampling ranges
  Optional: WarmQueryTables
  Optional: ColdIdentityAndDiagnostics
```

v1 profile 是版本化 closed set，不允许调用方任意拼 feature bits：

| `staticImageProfileId` | 必需节（Required Sections）                                                                 | 用途                                              |
| ---------------------- | ------------------------------------------------------------------------------------------- | ------------------------------------------------- |
| `traffic-headless-v1`  | `StaticTrafficImage`                                                                        | 服务器（Server）、测试、无图形宿主                |
| `traffic-spatial-v1`   | `StaticTrafficImage`, `StaticSpatialImage`                                                  | 引擎适配器（Adapter）、规范位姿（Canonical Pose） |
| `traffic-debug-v1`     | `StaticTrafficImage`, `StaticSpatialImage`, `WarmQueryTables`, `ColdIdentityAndDiagnostics` | 编辑器（Editor）、诊断、调试绘制（Debug Draw）    |

设计约束：

- `StaticTrafficImage` 可独立验证和挂载；headless profile 不携带 geometry；
- `sectionMask` 必须与 profile 的 closed section set 精确匹配；缺失、额外或未知
  section 均 fail closed，调用方不能通过 feature bits 组合新 profile；
- Spatial section 存在时必须完整覆盖 v1 所需 edge，并与 Traffic 共享 canonical edge
  ordinal；v1 不引入 sparse Spatial mapping；
- shared immutable bytes，多 `TrafficWorld`/Spatial session 复用；
- hot/warm/cold 分段，profile 可裁剪 Spatial/cold/debug data；
- 顶层 section byte offset/length 使用 checked `u64`；table row ordinal、count 和
  hot relation range 使用 checked `u32`，不保存原生指针；
- verifier 完成后 view 的高频索引是 O(1) 或连续 range traversal；
- schema/Serde/object graph 不是 static image ABI；
- 具体 archive/zero-copy library 由安全审计和 benchmark 决定。

相同 portable artifact 的不同 target/profile variant 共享
`canonicalArtifactDigest`，但各自拥有不同 `staticImageDigest`。Traffic Runtime 的
构造入口只接收从 `TrustedStaticImage` 拆出的 `StaticTrafficView`，不能要求调用方
同时提供 Spatial section，也不能接受仅结构验证所得的 view。

### 8.3 外部镜像描述符（External Image Descriptor）

生产 fast path 的 trust anchor 必须位于 image bytes 之外。版本化
`StaticImageDescriptor` 至少绑定：

```text
staticImageDescriptorVersion
canonicalArtifactDigest
staticImageDigest
staticImageLayoutVersion
staticImageProfileId
sectionMask
targetTriple
constraintSetVersion
identityEncodingVersion
identityRegistryRevision
compilerBuildId
validatorBuildId
validationReceiptDigest
```

descriptor 可以由签名 publication manifest、宿主已认证 asset/package manifest 或
应用内 pinned digest 提供。Image header 内的
`canonicalArtifactDigest`/target/provenance 只是待核对声明；攻击者可以伪造它们并
对任意 image bytes 计算新的 `staticImageDigest`，因此不能独立建立 semantic
trust。

Trusted descriptor 的签发前置条件是：portable artifact 已通过 independent
semantic validator，且不复用 compiler emitter 的 independent image rebuild 在相同
target/layout/profile 下产生相同 exact bytes digest。Validation receipt 必须记录
这两项成功证据。

### 8.4 源映射与诊断（Source Map and Diagnostics）

源映射按标识类别选择 key：

- owning declaration 和 contributing spans；
- declaration/addressable-derived 使用 StableId128 与 canonical identity tuple；
- owner-local relation/occurrence 使用 owning StableId128、typed role 和本次
  compilation 的 `localIndex`；
- 所有记录保留 LIR table/ordinal 作为本次 compilation 的定位；
- frontend/module/import provenance；
- compiler pass/constraint version；
- generated relation 的推导链。

诊断对标 rustc：稳定 code、severity、primary/secondary span、原因和可执行建议。
authoring error 指向 source/画布；artifact corruption/version mismatch 面向运维，不
返回 generated JSON 行号。

### 8.5 语义差异（Semantic Diff）

PR 审阅不依赖二进制 diff。Stable entity 按 StableId128 报告；owner-local derived
record 在 owning StableId128 + typed role 内按 canonical relation value 做确定性
sequence diff，报告 before/after `localIndex`，但不把位置当成跨编译标识：

- entity add/remove/rename-display；
- topology reconnect、owner/member 变化；
- geometry/length/tolerance-significant change；
- Gate/Waiting/signal/access behavior change；
- 标识闭包变化及原因；
- target/profile image layout-only change（不得伪装成 semantic change）。

Semantic diff 必须绑定 `baseCanonicalArtifactDigest` 与
`targetCanonicalArtifactDigest`。Base 必须是已通过 independent validator 的
portable artifact；无 baseline 时使用显式 genesis marker，并把全部 stable entity
和 owner-local sequence 报告为新增。重复 relation value 的序列对齐按最低
before/after `localIndex` 确定性破同值，diff 本身不获得 authoring authority。

## 9. 静态/可变状态和运行时消费

### 9.1 交通运行时（Traffic Runtime）

Target 把当前 `LaneFlow Core` / `laneflow-core` 重命名为
`LaneFlow Traffic Runtime` / `laneflow-runtime`。这不是机械改名：static contract、
format、image layout 和 compiler type 必须移出 runtime crate。当前 production 的
`laneflow-core`/`CoreWorld` 名称在 cutover 前继续有效；target public world 名称为
`TrafficWorld`。

`TrafficWorld` 借用或共享 `StaticTrafficView`，只分配：

- vehicles 与 route generations；
- controller clocks/indications；
- grant/reservation/waiting/parking occupancy；
- command/event/snapshot buffers；
- dynamic Route occurrence metadata。

compiler 预编译 authoring/static initial routes；runtime 新注册的 dynamic Route
继续由 Traffic Runtime 按 image candidate indexes 编译 occurrences，保持
ADR 0017 lifecycle 语义。

### 9.2 空间层（Spatial）

Spatial 只在 profile 含 Spatial section 时借用或共享 `StaticSpatialView`。Geometry
tables 与 Traffic edge ordinal 已对齐，不再建立 `HashMap<EdgeHandle, slot>` 或按
external ID join。Pose batch 仍遵守 ADR 0015 的 canonical f32、frame token、稳定
顺序、零分配和失败原子性。`traffic-headless-v1` 不创建 Spatial runtime。

### 9.3 加载信任状态（Load Trust States）

加载 API 区分三种状态，不能把 structural verification 命名为 trusted：

1. `UnverifiedImageBytes`：任意调用方提供的 bytes；
2. `StructurallyVerifiedImage`：通过 bounded structural verifier，只证明 view 可安全
   构造和全部 runtime precondition 成立；
3. `TrustedStaticImage`：结构验证之外，`staticImageDigest`、`canonicalArtifactDigest`、
   target/profile/constraint 与 image 外部的受信任 descriptor/validation receipt
   匹配。

Production `TrafficWorld`/Spatial 只从 `TrustedStaticImage` 拆出 view。允许三条
显式路径：

- **published trusted**：认证 descriptor + image bytes -> digest/profile/target
  比对 -> structural verifier -> trusted view；
- **local validated build**：portable artifact 先通过 independent validator，再由
  compiler builder 与 independent image builder 生成同 target/layout/profile
  image；digest 相等后生成与本次构建绑定的 receipt，或直接采用 independent
  builder 的已验证输出；
- **untrusted external**：必须提供 portable artifact，独立验证后本地重建；只有
  image bytes 时拒绝，不能直接进入 fast path。

### 9.4 有界结构校验器（Bounded Structural Verifier）

verifier 不重新执行 authoring topology、identity derivation、coverage 求解或
geometry tessellation，但必须检查 runtime 直接依赖的全部 precondition：

- magic/header/layout/target/profile/section compatibility；
- `u64` section offset/length、alignment、section bounds、地址空间转换和 checked
  arithmetic；
- table cardinality、CSR monotonicity、range、owner/member 与 cross-index bounds；
- finite/positive numeric values、speed/length domain；
- cumulative arc monotonicity、point/frame range 和 sampling bounds；
- Traffic mandatory、Spatial v1 complete coverage 和 shared edge ordinal；
- caller policy 限制的 image bytes、section/entity/point count；
- per-world mutable allocation plan、capacity multiplication 和地址空间上限。

任何 limit、digest、profile 或 structural invariant 失败都在分配/挂载前 fail
closed。签名或宿主 asset authenticity 属于 external descriptor 的来源认证，不是
image header 的可选自证字段。

## 10. 独立验证（Independent Validation）

```text
source module graph ─> compiler ─> canonical artifact ─> independent validator
                              │                           └> validation receipt
                              └> target static image ───────> structural verifier

canonical artifact + validation receipt
  ─> independent image rebuild ─> byte/digest comparison

trusted publication/asset manifest
  ─> external descriptor ─> digest/profile/target binding ─> trusted runtime view
```

independent validator 不调用 compiler semantic validation。两者可以共享机器可读
枚举、field tags 和约束常量，但 topology/ownership/coverage/geometry/occurrence
算法必须有独立实现或独立 oracle；independent image builder 也不得复用 compiler
emitter 的 layout population 实现。只有 artifact validation 与 image rebuild
comparison 都成功，publication 才能签发 trusted descriptor/receipt。

验证矩阵：

- identity known vectors、reorder/insertion/metamorphic tests；
- clean/incremental/parallel equivalence；
- compiler vs independent validator differential/fuzz；
- canonical artifact corruption 和 static image offset/range/limit fuzz；
- forged header canonical digest/provenance、attacker-recomputed image digest、
  tampered descriptor/receipt 和 wrong-profile rejection；
- `traffic-headless-v1` 无 Spatial bytes 的 TrafficWorld smoke/equivalence；
- `traffic-spatial-v1` Traffic/Spatial edge ordinal 与 full-coverage property tests；
- source map completeness 与 diagnostic stability；
- semantic diff golden tests；
- current JSON path 与 target image path behavior/determinism/pose equivalence；
- startup wall time、peak allocation、retained static memory；
- multi-world shared-static memory；
- 10k/100k Traffic Runtime tick 与 Spatial pose；
- #72 的 1M entity offline compile/image-build baseline。

## 11. 版本、发布与供应链

独立版本轴：

```text
authoringFormatVersion
canonicalFormatVersion
identityEncodingVersion
identityRegistryRevision
staticImageLayoutVersion
staticImageProfileId
staticImageDescriptorVersion
constraintSetVersion
compilerBuildId
validatorBuildId
targetTriple
canonicalArtifactDigest
staticImageDigest
validationReceiptDigest
```

- authoring/canonical 历史迁移离线完成；
- runtime exact-current/fail-closed，不在 production startup 迁移；
- portable artifact immutable publication 继承 ADR 0011；
- static image 可按同一 canonical digest 产生多个 target/profile variant；
- `canonicalArtifactDigest`、`staticImageDigest` 与 `validationReceiptDigest` 均为
  exact bytes 的 SHA-256，不使用 entity identity digest 代替；
- digest 只存放在其目标对象之外：artifact/image/receipt 均不把自己的 digest
  嵌回自身 bytes；publication manifest/external descriptor 完成外部绑定；
- compiler、validator、image builder 的 provenance 必须可审计；
- external descriptor/receipt 必须绑定 exact artifact、image、target、profile、
  constraint 和 tool builds；
- runtime 不联网解析 schema、artifact 或 toolchain。

authoritative source module graph 是 authoring SSOT。Generated artifact 可以作为
release/CI artifact 或为特定治理阶段 checked in，但只能由 compiler 生成并由
hash/digest/validation-receipt Gate 验证；永远不允许手改或与 source graph 竞争
authority。

## 12. 性能架构

### 12.1 编译期

- arena + typed `u32` ordinal，stable sequence；
- XXH3 只作为 tuple/cache candidate fingerprint，命中后 full equality；
- parallel pass 按 deterministic shard/merge；
- incremental invalidation 以 source/module/identity closure 为边界；
- geometry tessellation 与 semantic validation 可并行，但 LIR freeze 单一稳定顺序。

### 12.2 运行时

- SoA/CSR/flat ranges；
- typed dense `u32` handles/ranges，顶层 section byte offset/length 使用 `u64`；
- precompiled candidate/occurrence/reverse indexes；
- Traffic mandatory、Spatial/cold/debug profile-controlled；
- immutable image 共享、mutable arrays per world；
- tick 不做 string/hash/path matching；
- pose 不做 Traffic/Spatial join；
- production load 不做 JSON parse/registry rebuild。

### 12.3 开发闸口（Gate）

具体数字由实现 G1 在固定性能机上用 current baseline 冻结，但 Gate 至少覆盖：

- load latency、peak allocation、retained bytes；
- 2/8/32 worlds 的 shared-static scaling；
- 10k/100k Traffic Runtime 与 Spatial 既有上限不得回退；
- dynamic Route compilation 不进入 vehicle tick；
- 1M entity offline compile、incremental rebuild 和 image emission；
- target-specific SIMD/alignment 候选相对 portable/common layout 的收益。

不能用“BLAKE3/StableId128 可能变大”推导 tick 回退：ID 位于 cold/compiler boundary，
tick 只使用 32-bit dense handle。若 cold mapping retained memory 成为问题，使用
closed profile、压缩或外置 source map 解决，不缩短持久 identity。

## 13. 包（Crate）与依赖目标

下图箭头统一表示“左侧 crate 依赖右侧 crate”：

```text
laneflow-format ---------> laneflow-static-contract
laneflow-static-image ---> laneflow-static-contract

laneflow-compiler -------> laneflow-static-contract
laneflow-compiler -------> laneflow-format
laneflow-compiler -------> laneflow-static-image

laneflow-validator ------> laneflow-static-contract
laneflow-validator ------> laneflow-format
laneflow-validator ------> laneflow-static-image

laneflow-runtime --------> laneflow-static-contract
laneflow-runtime --------> laneflow-static-image
laneflow-spatial --------> laneflow-static-contract
laneflow-spatial --------> laneflow-runtime
laneflow-spatial --------> laneflow-static-image

laneflow-adapter-* ------> laneflow-runtime
laneflow-adapter-* ------> laneflow-spatial
laneflow-adapter-* ------> laneflow-static-image
```

职责与禁止依赖：

| 包（Crate）                | 拥有职责（Owns）                                                                                    | 禁止依赖（Must Not Depend On）                                      |
| -------------------------- | --------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------- |
| `laneflow-static-contract` | 稳定标识、种类 / 标签登记表、有类型序号、版本 / 摘要 / 配置档 / 描述符值                            | Serde、文件系统、核心 / 运行时（Core / Runtime）、空间层（Spatial） |
| `laneflow-format`          | 可移植制品（Portable Artifact）与发布 / 描述符线格式 / 视图（Publication / Descriptor Wire / View） | 编译器语义遍、运行时（Runtime）、空间层（Spatial）                  |
| `laneflow-static-image`    | 镜像 ABI、节 / 配置档、有界结构校验器（Bounded Verifier）、借用视图（Borrowed Views）               | 编译器、验证器、运行时（Runtime）、空间层（Spatial）                |
| `laneflow-compiler`        | 前端、中间表示、编译遍、主发射器、源映射 / 语义差异                                                 | 当前态数据 / 核心对象图（Current Data / Core Object Graph）         |
| `laneflow-validator`       | 独立制品语义（Independent Artifact Semantics）与镜像重建 / 预言机（Image Rebuild / Oracle）         | 编译器验证实现（Compiler Validation Implementation）                |
| `laneflow-runtime`         | 固定步进、车辆、动态路线、可变交通状态                                                              | 编译器、验证器、Serde、文件系统、空间层（Spatial）                  |
| `laneflow-spatial`         | 规范几何采样（Canonical Geometry Sampling）、位姿批次（Pose Batch）                                 | 编译器、验证器、引擎                                                |

`laneflow-runtime` 是 current `laneflow-core` 的 target 名称；
`laneflow-static-image` 取代含混的 `laneflow-runtime-image` 名称。共享 static
contract 不能继续留在 Runtime，否则 compiler/validator 会反向依赖动态运行时。
`laneflow-data` 是 current JSON compatibility façade，cutover 后不再拥有静态
normalization authority。

## 14. 迁移路线

```text
阶段 0  current JSON/Data/Core/Spatial 路径继续生产服役
阶段 1  #291：ADR 0020 + 本设计完成 G1
阶段 2  #292：static-contract + compiler foundation + Synthetic DSL frontend 纵向闭环
阶段 3  integration-only LIR→current projection 支撑 #282–#285 等价验证
阶段 4  Geometry document frontend + topology/geometry MIR（可与阶段 3 并行）
阶段 5  portable artifact + independent validator + source map/semantic diff
阶段 6  target static image + Traffic Runtime/Spatial shared image path
阶段 7  behavior/perf/security cutover Gate
阶段 8  production cutover，完成 core→runtime rename 并移除 projection/重复构建
```

阶段是架构迁移顺序，不是把终态降级为最小方案。每个阶段都必须沿同一个
AST/HIR/MIR/LIR 与 artifact/image contract 前进，不允许先建一个注定废弃的 Core
builder API。阶段 3 的 bridge 固定为 `laneflow-compiler-test-support` 或等价
integration-only crate：它可以依赖 compiler + current Core/Spatial，将 validated
LIR 投影为 current inputs；compiler 不依赖它，它不构成 public backend contract，
并由阶段 8 的 cutover owner 删除。

阶段 8 的一次性不兼容改名不仅覆盖 crate/type，也覆盖文档导航、Agent Skill ID、
工具薄包装和治理枚举：`laneflow-core-design` 目标改为
`laneflow-runtime-design`。在 current `laneflow-core/CoreWorld` 仍服役时只同步
Skill 内容并明确 current/target，不提前删除旧发现入口；具体治理枚举迁移必须由
独立 implementation Issue 原子更新 validator、模板和历史兼容规则。

Cutover 前必须证明：

1. current/target 场景的静态语义、tick、event、pose 等价；
2. deterministic artifact/image；
3. independent validator、validation receipt、external descriptor 与 bounded
   structural verifier 安全；
4. startup、memory、10k/100k 和 multi-world Gate；
5. publication/migration/source map/semantic diff 可用；
6. fallback/rollback 只切换 current/target asset path，不存在两套可变 authority。

## 15. 风险登记

| 风险                                                                 | 结果                                 | 控制                                                         |
| -------------------------------------------------------------------- | ------------------------------------ | ------------------------------------------------------------ |
| 编译器系统性缺陷（Compiler Systemic Bug）                            | 批量污染全部资产                     | 独立验证器、差分 / 模糊测试（Differential / Fuzz）、语义差异 |
| 二进制校验器漏洞（Binary Verifier Vulnerability）                    | 不可信字节破坏内存安全               | 基于偏移量的格式、加载限制、模糊测试 / `unsafe` 审计         |
| 镜像头声明被误当作信任（Header-as-Trust）                            | 恶意但结构合法的镜像绕过语义闸口     | 外部描述符、验证收据、不可信重建                             |
| 中间表示泄漏运行时类型（IR Leaks Runtime Types）                     | 后端 / 目标被当前核心对象图锁死      | 静态契约、目标中立 LIR、无环包依赖图                         |
| 标识漂移（Identity Drift）                                           | 引用、语义差异、缓存和存档失效       | 精确种类 / 标签登记表、已知向量、变形测试                    |
| 增量 / 并行非确定性（Incremental / Parallel Nondeterminism）         | CI / 发布字节漂移                    | 干净单线程预言机、稳定合并                                   |
| 配置档边界错误（Profile Boundary Error）                             | 无图形配置档携带几何，或交叉索引漂移 | 交通必需 / 空间可选矩阵、配置档测试                          |
| 当前态 / 目标态双路径长期化（Current / Target Dual-path Permanence） | 测试矩阵和语义漂移                   | 集成专用桥、明确移除责任人 / 切换闸口                        |
| 来源 / 生成物双重事实源（Source / Generated Dual SSOT）              | 手工修改与漂移                       | 来源模块图权威、生成摘要 / 收据闸口                          |
| 过早选择归档库（Premature Archive-library Choice）                   | ABI、安全或 MSRV 锁定                | 先冻结契约，再做基准 / 审计                                  |

## 16. #291 G1 完成条件

- ADR 0020 明确历史 ADR 的继续有效与取代范围；
- 本文不再存在 L1/L2 或 Core-shaped compiler IR；
- source module graph、标识 registry/encoding、trust/load path、static-image
  profile 与 crate DAG 均为 closed contract；
- Data/current Core/target Traffic Runtime/Spatial/Adapter 文档清楚标注 current 与
  target；
- #292 已重划为 compiler foundation + Synthetic DSL frontend，并继续保持
  `Blocked by #291`；
- 标识 v1、artifact/image/profile/version/validation/performance contract 一致；
- `traffic-headless-v1`、untrusted-image rejection 和全部 identity kinds 的验收测试已
  写入后续 implementation Gate；
- 本地 docs links/format/contract checks 通过；
- 外部 clean re-review 无未解决 Major，G1 才可推进。
