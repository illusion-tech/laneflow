# ADR 0020：编译器拥有的静态路网与目标静态镜像

**状态**: Review<br>
**日期**: 2026-07-29<br>
**适用范围**: 权威来源模块图（Authoritative Source Module Graph）、编译器中间表示
（Compiler IR）、静态路网权威、可移植规范制品（Portable Canonical Artifact）、
目标静态镜像（Target Static Image）、验证收据（Validation Receipt）、源映射
（Source Map）、独立验证器（Independent Validator），以及当前态 Core / 目标态
交通运行时（Traffic Runtime）、Data、Spatial 初始化边界<br>
**目标取代范围**: ADR 0005、0007、0008、0011、0013、0015、0017 中与静态数据 normalization、制品配对和运行时 registry 构建位置冲突的条款，并在目标态以 `LaneFlow Traffic Runtime`/`laneflow-runtime` 一次性不兼容替代当前 `LaneFlow Core`/`laneflow-core`。<br>
**迁移时间表修订（#301 G1）**: 仓库未发布 1.0、无外部运行时消费者。#301 完成后 `laneflow-runtime` / `TrafficWorld` 即唯一可运行交通世界，并拆除 current Core/JSON 运行时入口；不再等待 #294 才切换权威。#294 不再拥有生产切换或拆除旧运行时路径。精确消费契约见 `../design/traffic-runtime-shared-consumption.md`。<br>

**后继决策**: ADR 0029（Accepted；#498 G1）取消本文把 `StaticRoute` 列为 Identity
v1 必选声明种类、并由编译器预编译初始路线出现项的条款。ADR 0024（Accepted；#299 G1 Pass）已部分取代本文关于独立
`laneflow-validator`、`canonical-publication-v1` receipt、由 #299 统一交付三类
验证收据，以及 compiler/validator 必须维护两套完整语义实现的决定。本文关于编译器
拥有静态路网、Canonical LIR、可移植规范制品、对象外信任锚和 Runtime/Spatial 分层的
其余决定保持有效。ADR 0025（Accepted；#300 G1 Pass）进一步以进程内
`SharedNetworkRevision` 取代本文 target/profile-specific 静态镜像文件、稳定 ABI、
descriptor/完整性清单、mmap/chunk 和镜像摘要/长度决定；该设计已接受。#439 / PR #436
承载已形成的受检 LFCA 到共享路网基础投影，#440 闭合剩余 Runtime 静态关系，#441
独立记录资源与性能证据；#300 保持父级跟踪项。精确后继边界见
`../design/compiler-post-emission-check-and-minimal-publication-closure.md` 与
`../design/shared-static-network.md`。<br>

**2026-08-10 范围澄清**：current Traffic/Spatial/Scenario JSON 从未作为外部资产发布，
只属于当前仓库加载器和夹具；它不建立 compiler import frontend、批量迁移工具或长期
离线兼容入口。编译器迁移正确性以原生有类型来源模块和集成专用 LIR→current 投影
验证，精确边界见 `../design/current-package-import.md`。本澄清不改变通用外部来源导入
前端或规范制品版本迁移能力。

**关联文档**:

- 上游决策:
  - `0001-project-scope.md`
  - `0003-runtime-tick-and-determinism.md`
  - `0005-core-identity-and-handle-model.md`
  - `0007-traffic-data-crate-and-loader-boundary.md`
  - `0008-pre-1.0-data-format-version-policy.md`
  - `0011-schema-identifier-and-publication-contract.md`
  - `0013-engine-neutral-spatial-geometry-and-length-authority.md`
  - `0015-bounded-f32-canonical-spatial-frames.md`
  - `0025-checked-canonical-network-and-shared-static-network.md`
  - `0017-static-road-junction-maneuver-and-gate-identity.md`
- 配套决策:
  - `0021-city-simulation-game-traffic-foundation.md`
  - `0024-compiler-post-emission-check-and-minimal-publication-closure.md`
- 详细设计:
  - `../design/network-compiler.md`
  - `../design/compiler-foundation.md`
  - `../design/data-format.md`
  - `../design/data-loading.md`
  - `../design/spatial-geometry.md`
  - `../design/core-id-handles.md`
  - `../reference/glossary.md`
- GitHub:
  - #72
  - #220
  - #291
  - #292
  - #294
  - #301

## 术语规范

本 ADR 的所有领域术语遵循 `../reference/glossary.md`：中文术语和中文定义是权威
事实，英文只作辅助理解。Rust 类型、crate、字段、版本值、算法和协议常量等精确
标识符保留原文；它们不构成独立英文语义来源。若本 ADR 的英文别名与中文定义产生
歧义或冲突，以中文定义为准。

## 背景

LaneFlow 当前以 Traffic JSON、Spatial JSON 和 Scenario Manifest 为运行时输入。
`laneflow-data` 解析外部 ID，调用 Core constructors 构造 `InitialTrafficData`，
编译 Route/Maneuver/Gate occurrence；Spatial loader 随后再次按 Traffic edge
标识绑定折线并建立自己的 registry。走廊生成器内部又维护一套只服务 JSON
输出的 DTO，再通过 production loader 重新进入上述路径。

这套边界在手写数据时代提供了清晰的防御层，但不适合作为编译器时代的终态：

- `InitialTrafficData` 已经是 Core 专用、完成 normalization 的初始化对象，不是可供
  多后端复用的中间表示；
- Traffic 与 Spatial 分离加载会重复 ID 解析、引用解析、完整性校验、dense index
  分配和跨制品 join；
- 把拓扑生成器称为 L1、几何编译器称为 L2，会迫使后者消费前者的 Core-shaped
  输出，形成层级依赖，而不是多个 frontend 组合到同一语义管线；
- canonical JSON 既承担治理证据又承担生产启动输入，使 schema/Serde/object graph
  形状反向约束热数据布局；
- compiler、loader、Core 和 Spatial 分别构建部分静态事实，静态路网没有唯一的
  编译权威；
- 单个 compiler bug 可以系统性污染全部生成资产，仅复用 compiler validation
  不能形成独立预言机。

#291 的目标不是在现有路径旁增加一个生成工具，而是把全部静态路网语义前移到
离线编译期，并让 target Traffic Runtime 只从具有外部信任锚的派生静态镜像建立
只读 view。

Accepted ADR 0021 进一步确认：LaneFlow 的第一长期产品目标是为未来的中国特色城市
模拟游戏提供交通基础。#291 的已接受设计因此不仅要优化加载，还必须保留单个大型
城市世界的并行扩展、玩家修改道路、存档/回放、路径规划接入与每世界唯一性演进
空间，同时不把城市经济和出行需求塞入交通运行时。

## 决策

### 1. 编译器是全部静态路网语义的唯一编译权威

编译器负责把 authoring source 降低为一个经过全局验证、具备稳定标识和
dense layout 的 canonical LIR。以下工作只发生在编译期：

- authoring key 与引用解析；
- RoadCorridor/RoadSection/Lane/Junction/Movement/ManeuverPath/Gate/WaitingZone、
  signal、parking、access 与 geometry 的静态语义检查；
- 稳定 declaration/addressable-derived 的 StableId128 派生、重复标识与
  digest collision 检查；
- 全部 LIR table row 的 typed logical ordinal 与 owner-local occurrence key；
- topology closure、owner/member range、reverse index 和跨域引用生成；
- Route、ManeuverPath、Gate、WaitingZone occurrence 编译；
- Traffic length 与 Spatial arc length 的共同派生及绑定验证；
- target static image 的 dense handle 分配与 hot/cold layout。

Target `LaneFlow Traffic Runtime` 继续拥有运行时交通规则、已实现执行域的交通参与
单元、动态通行定义（Dynamic Traversal Definition）生命周期、固定步进（Tick）、
安全约束与运行时状态权威（Runtime State Authority）；Spatial
继续拥有 canonical geometry sampling 和 pose 语义；二者不再各自重新解释或重建
静态路网。Current production 的 `LaneFlow Core` 命名、车辆特化和 API 在 cutover
前继续有效。

### 2. 权威来源模块图（Authoritative Source Module Graph）组合平级前端（Frontend），不采用 L1/L2

一个 compilation unit 的唯一 authoring authority 是显式、可重放的 source module
graph，而不是 Geometry 这一种格式。每个 module 必须绑定稳定 namespace、source
language/content digest、frontend version/options、origin/provenance 与 imports。
下列输入能力是可组合 frontend/source：

- **Synthetic DSL frontend**：测试、benchmark、示例和非发布程序化场景；
- **Road editing / Geometry frontend**：production 道路编辑状态的主要受检前端；可视化
  编辑器、可发布程序化生成器、SDK 与 importer 共享有类型道路编辑模型，#296 的产品
  负责人已选择按模块 size-prefixed FlatBuffers 作为物理来源编码；其 exact schema、资源
  和审计契约仍须通过当前 G1；
- **Import frontend**：OSM、外部工具和离线 migration；
- **Editor authoring surface**：编辑并持久化道路编辑状态，通过画布选择和语义差异消费
  诊断；不拥有私有编译语义。

ADR 0023 重新打开了“Geometry JSON 文档是 production 主要编制语言”的具体格式选择。
source module graph 仍是逻辑编制权威，但其可重放物理编码必须服务编辑器、程序化
生成、局部加载和协作，而不以人工文本编辑或原始 Git diff 为产品前提。

每个 frontend 只负责自己的语法、来源保真和 source span，不得输出
`InitialTrafficData`、Core registry 或 target runtime object graph。不同 frontend
可以在同一个 compilation unit 中导入模块，并在 HIR 之后共享完全相同的语义
管线。Import 必须保留原始 source digest、importer build/options 和 provenance；
publishable programmatic generator 必须保留 build ID、参数、seed、namespace 与
输入 digest。匿名 AST 注入只允许非发布测试。

### 3. 采用有类型抽象语法树（Typed AST）→ 高层中间表示（HIR）→ 中层中间表示（MIR）→ 已验证规范低层中间表示（Canonical LIR）

编译器中间表示按稳定职责分层：

| 阶段                              | 权威与允许内容                                                          | 禁止内容                                                  |
| --------------------------------- | ----------------------------------------------------------------------- | --------------------------------------------------------- |
| 有类型抽象语法树（Typed AST）     | 前端语法、显式键、来源位置（Source Span）、未解析引用                   | 核心句柄（Core Handle）、目标布局（Target Layout）        |
| 高层中间表示（HIR）               | 命名空间、模块、符号、单位、引用与编制语义（Authoring Semantics）已解析 | 几何镶嵌（Geometry Tessellation）、密集槽位（Dense Slot） |
| 中层中间表示（MIR）               | 展开道路走廊 / 区段 / 车道，生成路口 / 路径，完成曲线离散和全局静态语义 | 目标 ABI、平台指针                                        |
| 规范低层中间表示（Canonical LIR） | 稳定实体标识、全部表行的有类型逻辑序号、完整静态关系和确定性顺序        | JSON 对象图（Object Graph）、宿主类型                     |

每一阶段只由前一阶段构造；公共 pass 以显式输入/输出和结构化诊断运行。LIR 是所有
静态语义后端的唯一语义输入；源映射后端可以另外消费与同次成功编译原子冻结、但无权
补充静态语义的已验证来源伴随数据。任何后端都不得补充语义默认、重新推断 topology
或修复缺失关系。

具体 Rust collection、arena library 或序列化库不在本 ADR 冻结；必须通过独立
实现基准选择。但 IR 要求：

- arena/dense ordinal 可用 `u32` 表达，超界时编译失败；
- stable declaration/addressable-derived 才有 StableId128；owner-local relation /
  occurrence 使用只在 owning sequence snapshot 内有效的 typed
  `(ownerOrdinal, role, localIndex)` key，不获得全局持久标识；
- 基础车道图边（Lane Graph Edge）`LaneEdge` 以来源模块的显式稳定
  `laneEdgeKey` 获得统一 StableId128；RoadSection 覆盖与派生 Junction internal
  ownership 是关系 / 角色，不是边身份所有者，未被任一关系覆盖的合法边同样进入
  `StaticIdentityIndex`；
- iteration order 与输出不依赖 hash table iteration；
- 增量 cache key 只影响重用，不影响完整编译结果；
- 并行 pass 必须以稳定 merge order 产生与单线程逐字节相同的输出。

标识编码封装（Identity Envelope）与实体登记表（Entity Registry）使用独立版本
轴。`identityEncodingVersion = 1`
冻结 magic、kind、field count、strictly-increasing tag/length/value bytes；
`identityRegistryRevision = 3` 覆盖 topology、Gate/Waiting、Signals/Phase、
Parking、cross-section/access/profile、canonical frame、`ConflictZone` 与
`ParticipantStream` declaration，并统一使用 `ParkingFacility` 名称；既有 identity
canonical bytes 不变。
kind `1..=23` 与 field tag `1..=34` 连续登记；新增的 `ConflictZone` / `ParticipantStream`
使用已退役且从未对外发布的编号槽位，不改变任何既有合法 kind/tag 或 StableId
（ADR 0029）。新增 kind 只 append registry revision；修改既有 kind 的字段集合、tag 含义
或编码必须提升 encoding version。完整
kind/tag/required-sequence
表以 `network-compiler.md` 为规范。所有**定义子实体身份**的父子关系必须以父实体
StableId 作为父锚点，不能只复制父实体在其来源模块内稳定的裸局部 key；这样跨模块
同名父实体与重新归属仍由完整命名空间裁决。可选组织关系不因此自动成为身份前像；
例如 `ParkingSpace.facilityId` 仅在显式存在时形成
`ParkingSpace -> ParkingFacility` 关系，
不参与 `ParkingSpace` 的 StableId128 派生。

### 4. 一个规范低层中间表示（Canonical LIR）产生四类配套输出

一次成功编译原子地产生：

1. **可移植规范制品（Portable Canonical Artifact）**：平台无关、确定性、可发布和长期审计的静态路网
   事实；内含每个稳定实体的规范身份表（Canonical Identity Table）
   `CanonicalIdentityTable`，保存完整 field-tag/value 前像、声明的 `StableId128` 与
   typed ordinal，承接 public artifact、独立身份重算、迁移和跨实现互操作；
2. **目标静态镜像（Target Static Image）**：按目标平台、布局版本、封闭配置档
   （Closed Profile）、分区提示版本与性能要求
   生成，可重建、面向 mmap/顺序读取和直接索引；
3. **源映射 / 诊断制品（Source Map / Diagnostics Artifact）**：从
   `(entityKind, StableId128, typed ordinal)` 到 source span 和 pass provenance 的
   映射；使用绑定 `networkRevision`、`canonicalArtifactDigest`、compiler build 与
   compilation provenance 的版本化 `SourceMapEnvelope`，可以冗余 tuple 用于显示，
   但该冗余不得作为身份验证前像；身份重算只消费规范制品中的
   `CanonicalIdentityTable`；
4. **语义差异（Semantic Diff）**：以稳定标识和字段语义描述新增、删除、重接、
   geometry/behavior 变化，供 PR/Gate 审阅；使用绑定旧/新路网修订、规范制品摘要与
   精确长度的版本化 `SemanticDiffEnvelope`。

规范发布描述符（Canonical Publication Descriptor）
`CanonicalPublicationDescriptor` 对完整 portable artifact bytes 绑定
`canonicalArtifactDigest + canonicalArtifactByteLength`，并以
`sourceMapDigest + sourceMapByteLength` 绑定完整 `SourceMapEnvelope` exact bytes，
同时认证 `canonical-publication-v1` 收据的
`validationReceiptFormatVersion + validationReceiptKind + validationReceiptDigest +
validationReceiptByteLength`。
源映射封套绑定单份制品摘要与编译来源沿袭；语义差异封套绑定旧/新制品摘要、精确
长度和路网修订；静态镜像引用其来源制品摘要，并另外由外部 descriptor 绑定
target/profile-specific `staticImageDigest + staticImageByteLength`。任何输出失败，
本次 compilation unit 都不得发布部分结果。

Portable canonical artifact 是发行与治理契约，不是运行时热布局。Target static
image 是可丢弃、可按目标重新生成的派生制品，不获得独立 authoring authority。
authoritative source module graph 始终是唯一 authoring SSOT；generated artifact
是否 checked in 只属于发布和审阅策略，不能形成第二套可手改事实源。

### 5. 交通运行时（Traffic Runtime）与空间层（Spatial）消费配置档控制的静态镜像

Target `StaticNetworkImage` 是 sectioned container：

```text
StaticNetworkImage
  Header / provenance / profile
  SectionDirectory
  Required: StaticTrafficImage
  Required: StaticIdentityIndex
  Required: PartitionPlanningHints
  Optional: StaticSpatialImage
```

- `traffic-headless-v1` 要求 Traffic + StaticIdentityIndex + PartitionPlanningHints；
  `traffic-spatial-v1` 再要求 Spatial；v1 不定义泛型 `WarmQueryTables`、
  `ColdDiagnostics` 或 `traffic-debug-v1`。运行时必需的低频索引进入对应 required
  typed section，显示名、规范身份前像、来源位置和诊断文本由 portable artifact、
  source map 与独立 diagnostics artifact 外置提供；
- target `laneflow-runtime`/`TrafficWorld` 只借用或共享 `StaticTrafficView`，不要求
  Spatial bytes；
- `StaticIdentityIndex` 对稳定实体保存 typed ordinal → StableId128 正向表和按
  `(entityKind, StableId128)` 排序的反向表；它是 snapshot save/load、dynamic Route
  重建和路网修订切换的生产必需冷索引，但不进入 steady tick；
- `CanonicalIdentityTable` 只属于 portable artifact / independent validation 输入；
  static image 不复制规范元组前像，生产 Runtime 不为身份重算元数据承担 retained
  memory 或 cache 成本；
- Spatial section 存在时完整覆盖 v1 所需 edge，并与 Traffic 使用同一 logical edge
  ordinal/cross-index；v1 不引入 sparse geometry mapping；
- Adapter 继续只消费 Traffic committed snapshot 与 Spatial pose batch，不读取
  compiler IR，也不获得静态规则 authority；
- 多个 `TrafficWorld`/Spatial session 可以共享同一 immutable image。
- image 必须保存工作线程数无关的静态执行约束图；v1
  `PartitionPlanningHints` 节保存可被运行时忽略或重建的分区规划提示，但不得
  保存最终分区、工作线程、边界邻域所有权（Halo Ownership）或动态负载状态。

生产启动不得执行 JSON parse、external string rebind、static hash registry rebuild、
topology reconstruction、Traffic/Spatial package join 或 initial Route occurrence
recompile。Production fast path 必须先把 image bytes 与 image 外部的 trusted
descriptor/validation receipt 绑定，再执行有界结构验证并建立只读 view。

外部描述符同时绑定版本化静态镜像完整性清单（Static Image Integrity Manifest）
`StaticImageIntegrityManifest` 的摘要与精确长度。v1 清单是覆盖完整镜像精确字节的
平面 SHA-256 分块表（Flat Chunk Table），并记录节到分块区间的闭合映射。分块大小由
已认证的约束集、配置档与完整性方案版本唯一确定，发布者不能任意选择；具体值由实现
G1 在最低产品硬件上基准后写入约束集。生产启动先认证描述符/清单，再预先验证
（Eager Verification）Traffic、identity、partition-hint 等目标
视图的必需分块；Spatial 分块可在创建 Spatial session 前采用延迟验证或后台验证，
未验证字节不得暴露给有类型视图。全镜像 SHA-256 继续作为发布身份、独立重建与
显式完整审计（Full Audit），不是每次启动建立 Traffic view 的强制串行步骤；分块
摘要不进入 fixed tick 或 pose 热路径。
v1 采用平面分块表而非默克尔树（Merkle Tree），因为外部信任锚先认证整份有界小型
清单，平面表已满足节级随机/并行验证且校验器攻击面更小；未来远端认证分页必须提升
完整性方案版本，不能原地改变 v1。
分块验证与有类型视图必须绑定同一个不可变字节背板（Immutable Byte Backing）。
宿主不能保证已打开资产在视图生命周期内不可替换/改写时，必须把已验证分块复制并
封存到只读拥有存储，或拒绝建立可信视图；验证路径后重开或保留可写别名均不可信。

### 6. 静态镜像与可变运行时状态物理分离

静态镜像不得内嵌交通参与单元、controller clock、reservation、occupancy、runtime
traversal generation、world/session nonce 或其他每世界可变状态。当前兼容
projection 中的 vehicle 与 runtime Route 同样属于该禁止范围。运行时状态使用独立
dense arrays，并以 image ordinal/typed `u32` handle 关联静态表。

静态镜像表示一个不可变路网修订，不表示城市道路永不变化。玩家或工具修改来源
模块后必须生成并验证新修订；运行世界只能在 fixed-tick 安全边界通过失败关闭的
镜像切换事务原子迁移，不能原地修改共享 image。每世界 seed/随机流、执行计划、
快照和宿主 identity 也不能进入共享 image。

人口、Routing 与游戏规则的 seed/随机流由 caller/出行编排层拥有；Traffic Runtime
不得为了存档或共享镜像而新增隐藏随机权威。若后续 G1 显式引入 Runtime 自有随机流，
它只能属于每世界可变状态。

默认在线切换使用准备（Prepare）→增量追赶（Delta Catch-up）→静默提交
（Quiescent Commit）→回收（Retire）。旧世界在准备期间继续固定步进；Runtime
从基准提交状态构造候选，并把后续已提交动态状态变化、生命周期变化以及命令/事件
游标写入有界迁移增量日志。候选只按受信任迁移策略重解释这条已提交变更流，不重新
执行输入命令，也不发布第二份行为结果或重复旧世界事件；迁移策略要求的切换事件
只能形成未提交候选批次。后台按规范顺序追赶，直到落后量进入提交预算；在安全边界
短暂静默旧世界，排空日志尾并证明候选状态/切换事件批次等价于“把迁移函数应用到
旧世界最新
已提交状态”的结果，再把镜像/状态绑定与规范排序的切换事件批次作为同一原子提交只
发布一次；放弃候选时不得发布该批次。候选不得自行模拟未来固定步进；无法追上、
日志溢出、迁移失败或预算超限时放弃候选并继续旧修订。旧镜像在
全部借用视图/token 退出后回收。宿主只有在显式维护暂停模式中才可让整个准备期
停表，不能把它作为在线玩家改路的隐式语义；维护暂停的完整停顿必须单独预算。

静态镜像按访问频率拆分：

- **hot**：tick/pose 所需 SoA/CSR 数据、flat ranges、precompiled occurrence、
  speed/length/gate constraint 与 geometry sampling index；
- **warm**：进入具体 required typed section 的低频 query、owner/member 和可选
  spatial detail；
- **cold identity**：全部 production profile 共享、可按需映射的
  `StaticIdentityIndex`；
- **external diagnostics**：display name、source provenance、canonical tuple、
  诊断文本和 publication metadata 由 portable artifact、source map 或独立诊断
  制品携带，不进入 v1 production image profile。

hot path 只使用 typed dense handle、contiguous range 和预编译索引。StableId128、
BLAKE3、XXH3、字符串、hash lookup、path matching 与 schema validation 都不得进入
steady tick。Spatial section 可以按 closed profile 从 headless/server image 中
剥离，但 `StaticIdentityIndex` 不能被裁掉；可通过共享只读映射、分块/压缩或按需
分页控制内存和 cache 影响。Artifact digest、source map 和外部诊断制品仍能恢复
完整诊断。

### 7. 可移植制品（Portable Artifact）与静态镜像（Static Image）分离版本与绑定字段

字段不得再被一个 `formatVersion` 混合表达，也不能把版本号、构建选择器和内容绑定
都称为“版本轴”。

契约版本轴（Contract Version Axes）：

- `authoringFormatVersion`
- `canonicalFormatVersion`
- `canonicalPublicationDescriptorVersion`
- `identityEncodingVersion`
- `identityRegistryRevision`
- `networkRevisionDerivationVersion`
- `staticImageLayoutVersion`
- `staticImageDescriptorVersion`
- `staticImageIntegritySchemeVersion`
- `sourceMapFormatVersion`
- `semanticDiffFormatVersion`
- `validationReceiptFormatVersion`
- `networkRevisionCutoverDescriptorVersion`
- `migrationPolicyVersion`
- `executionConstraintVersion`
- `partitionHintVersion`
- `runtimeSnapshotVersion`
- `constraintSetVersion`

构建与目标选择器（Build and Target Selectors）：

- `staticImageProfileId`
- `compilerBuildId`
- `validatorBuildId`
- `targetTriple`

封闭种类选择器（Closed Kind Selectors）：

- `validationReceiptKind`

内容身份与长度绑定（Content Identity and Length Bindings）：

- `networkRevision`
- `canonicalArtifactDigest`
- `canonicalArtifactByteLength`
- `staticImageDigest`
- `staticImageByteLength`
- `staticImageIntegrityManifestDigest`
- `staticImageIntegrityManifestByteLength`
- `sourceMapDigest`
- `sourceMapByteLength`
- `semanticDiffDigest`
- `semanticDiffByteLength`
- `validationReceiptDigest`
- `validationReceiptByteLength`

六个 digest 均为各自目标对象完整 exact bytes 的 SHA-256，因此任何对象都不得把
自己的 digest 嵌回自身 byte sequence。Publication manifest / external descriptor
负责保存目标对象的 digest；image header 可以保存另一个对象的
`canonicalArtifactDigest`，但不保存自己的 `staticImageDigest`。
每个 digest 必须由受认证 descriptor/manifest 同时绑定同一对象的精确 `u64` byte
length。消费者在任何线性读取、解压、分配、解析或 hash 前先检查调用方上限、
地址空间和 exact length；未知长度 stream 只允许 checked length+1 bounded reader。
该规则统一适用于 portable artifact、static image、静态镜像完整性清单、source map、
semantic diff 和 validation receipt，不能以最终 digest mismatch 作为输入大小防线。
约束集或宿主策略必须分别提供 `maxCanonicalArtifactBytes`、
`maxStaticImageBytes`、`maxStaticImageIntegrityManifestBytes`、
`maxSourceMapBytes`、`maxSemanticDiffBytes` 与 `maxValidationReceiptBytes`；这些
上限不能由待验证对象自报。

`StaticImageDescriptor` 必须同时认证 `staticImageByteLength`，表示参与
`staticImageDigest` 的原始未压缩 image exact bytes 的 `u64` 长度；validation
receipt 与 independent rebuild comparison 绑定 digest + length。Loader 先认证有
固定小上限的 descriptor，再在任何与输入大小成正比的读取、解压、分配或 hash 前
检查该长度非零、可表示且不超过 caller/process limit。已知长度的 buffer/mmap/blob
执行 O(1) exact-length 比较；未知长度 stream 只能通过最多读取
checked `staticImageByteLength + 1` bytes 的 bounded reader，并拒绝
truncated/appended 输入。压缩传输同时限制压缩输入与解压输出；结构校验器的内部
count/range limits 作为后续第二道防线，不能替代 pre-hash byte bound。
同一 descriptor 还必须绑定
`canonicalArtifactDigest + canonicalArtifactByteLength`、
`staticImageIntegritySchemeVersion +
staticImageIntegrityManifestDigest + staticImageIntegrityManifestByteLength` 与
`validationReceiptFormatVersion + validationReceiptKind +
validationReceiptDigest + validationReceiptByteLength`；Runtime 不读取 artifact /
receipt 时只比较已认证绑定，validator、publisher 或审计消费者读取时仍必须执行统一
的 pre-hash 长度上限和 bounded-reader 规则。

完整性清单必须按镜像偏移量连续、无缺口/重叠地认证全部分块，并闭合镜像头、节目录
与配置档必需节的覆盖。发布者与独立镜像重建器必须证明有序分块、全镜像
`staticImageDigest + staticImageByteLength` 和同一最终镜像精确字节一致。加载器对
描述符/清单各自先做调用方长度上限与摘要前预检，再验证目标节分块和有界结构；未
验证节不得通过任何视图暴露。

`SourceMapEnvelope` 必须内含 `sourceMapFormatVersion`、
`networkRevisionDerivationVersion + networkRevision`、`canonicalArtifactDigest`、
`compilerBuildId` 与来源沿袭记录。它不内嵌自己的摘要；外部
`CanonicalPublicationDescriptor` 认证上述配对及
`canonicalArtifactDigest + canonicalArtifactByteLength`、
`sourceMapDigest + sourceMapByteLength` 与
`validationReceiptFormatVersion + validationReceiptKind +
validationReceiptDigest + validationReceiptByteLength`。消费者先认证小型
descriptor，再执行统一的 pre-hash 长度预检，并要求 descriptor、source-map
envelope 与已验证 portable artifact 的 artifact/revision/provenance 字段全部精确
相等。记录级 StableId/ordinal key 只能在该配对成功后查找；任何错配以
`SourceMapArtifactMismatch` 失败关闭。

`SemanticDiffEnvelope` 必须内含 `semanticDiffFormatVersion`，并分别绑定旧/新
`networkRevisionDerivationVersion + networkRevision` 与
`canonicalArtifactDigest + canonicalArtifactByteLength`。跨修订迁移时，外部
`NetworkRevisionCutoverDescriptor` 必须认证同一
`semanticDiffFormatVersion + semanticDiffDigest + semanticDiffByteLength`；
独立验证器按该版本解析并从两份已验证规范制品重算或验证完整差异。缺失、未知或错配
版本时不得解析记录或进入迁移事务。

路网修订标识（Network Revision ID）`NetworkRevisionId` 不复用上述 exact-bytes
digest。v1 以带域分离（Domain Separation）的 SHA-256 对冻结的目标无关规范路网
语义载荷（Canonical Network Semantic Payload）计算 `networkRevision`；该载荷包含
identity/constraint/execution-constraint versions
和全部静态路网语义，排除摘要自身、artifact envelope、工具 provenance、source
map/diagnostics、publication metadata、target/profile/layout/partition-hint。
Independent validator
必须从 portable artifact 独立重算；validation receipt、`StaticImageDescriptor`
和 image header 的外部核对必须绑定
`networkRevisionDerivationVersion + networkRevision`。因此同一规范语义的不同
target/profile image 共享修订，而任何运行时可观察静态语义变化都会产生新修订。
相同 `NetworkRevisionId` 对应不同规范路网语义载荷时 publication/cutover 必须以
`NetworkRevisionDigestCollision` 失败关闭，不得追加 ordinal、salt 或 suffix。

loader/runtime 对 static image layout、target、profile、descriptor 与 digest 采用 exact-current、
fail-closed 规则；不在生产启动路径执行历史迁移。旧 authoring/canonical artifact
通过离线 compiler upgrade/migration 进入当前版本。Static image 可以在保持
canonical artifact 不变时因布局或 CPU target 变化而重建。

### 8. 验证（Validation）与信任（Trust）采用四道分离防线

1. **Compiler validation**：对 AST/HIR/MIR/LIR 执行完整语义和生成前置检查；
2. **独立验证器（Independent Validator）**：只消费 portable canonical artifact 和公开
   constraint contract，独立实现 topology、标识、ownership、coverage、
   geometry 与 occurrence 检查；必须从 artifact 内 `CanonicalIdentityTable` 的完整
   前像独立编码并重算每个 BLAKE3-128 `StableId128`，验证 parent anchor、duplicate
   tuple 与 digest collision；不得调用 compiler semantic validation，也不得依赖
   source map 补齐身份字段；
3. **验证收据封套（Validation Receipt Envelope）/外部描述符（External
   Descriptor）**：以独立 `validationReceiptFormatVersion` 和封闭
   `validationReceiptKind` 绑定路网修订标识、
   artifact/image/完整性清单/source-map digest 与各自 exact byte length、完整性
   scheme、target、profile、constraint、compiler/validator build 与 compilation
   provenance；其 authenticity 由签名 publication manifest、宿主认证 asset chain
   或 pinned digest 提供；
4. **Static image structural verifier**：对不可信 bytes 有界检查 header、版本、
   offset/alignment、table/range/cross-index、numeric/runtime precondition 和 load
   limits，不重跑全量 authoring 语义。

compiler 与 independent validator 可以共享机器可读的常量/枚举/约束声明，但不得
共享实现同一语义判定的函数。CI 与 publication 至少要求 canonical artifact 通过
独立 validator，且 validator/oracle 不复用 compiler image emitter，从该 artifact
按相同 target/layout/profile 独立重建出的 image 与 compiler image 具有相同 exact
bytes digest；structural verifier 还必须接受该镜像。

Image header 内的 canonical artifact digest/target/provenance 只是待核对声明，不能
作为自证 trust anchor。
Published trusted image 必须匹配外部 descriptor；local build 必须绑定刚完成的
independent validation receipt；untrusted external input 必须提供 portable artifact
并在本地验证/重建，只有 image bytes 时拒绝。

每份验证收据必须使用 `ValidationReceiptEnvelope`，内含
`validationReceiptFormatVersion + validationReceiptKind + validatorBuildId +
subjectBindings + checkResults`，且不得内嵌自己的摘要。外部描述符在读取收据前必须
认证同一版本、种类、摘要和精确长度；未知或错配版本/种类失败关闭，不能用
`validatorBuildId` 推断 wire format。v1 收据种类封闭为
`canonical-publication-v1`、`static-image-v1` 与 `revision-cutover-v1`，分别要求
source-map 闭合、独立镜像重建或完整语义差异验证的对应受检对象绑定和成功证据，不得
互相替代。

`static-image-v1` 收据必须记录 artifact semantic validation（包括全部稳定身份独立
重算）、路网修订标识独立重算与 independent image rebuild comparison 的成功结果。
缺失 / 篡改身份前像或声明 ID 不匹配时必须失败，未完成三者时不得签发可进入
production fast path 的 descriptor。运行时当前修订只来自已认证
`StaticImageDescriptor`，不接受调用方或 image header 自报。

独立镜像重建器只能消费独立验证器刚建立的已验证规范制品视图（Validated Artifact
View）；跨进程时可以消费由已认证 `canonical-publication-v1` descriptor/receipt
重新建立的等价能力。
包含重建比较结果的 `static-image-v1` 最终收据必须在比较成功后才签发，绝不能作为
自身独立重建或比较的输入。`revision-cutover-v1` 同理只能在两侧可信描述符和完整
语义差异验证完成后签发。

### 9. 包（Crate）目标边界

终态依赖箭头统一表示“左侧依赖右侧”：

```text
laneflow-format ---------> laneflow-static-contract
laneflow-static-image ---> laneflow-static-contract
laneflow-compiler -------> laneflow-static-contract / format / static-image
laneflow-validator ------> laneflow-static-contract / format / static-image
laneflow-runtime --------> laneflow-static-contract / static-image
laneflow-spatial --------> laneflow-static-contract / runtime / static-image
laneflow-adapter-* ------> laneflow-runtime / spatial / static-image
```

`laneflow-static-contract` 只拥有 StableId/kind/tag、`NetworkRevisionId`、typed
ordinal 和 version/digest/profile primitives，不依赖 Serde、filesystem、Runtime
或 Spatial。
`laneflow-static-image` 只拥有 image ABI、section/profile、bounded verifier 和
borrowed views，不依赖 compiler/validator/Runtime/Spatial。

Target `laneflow-runtime` 是 current `laneflow-core` 的 clean-break 名称，只拥有
tick、已实现执行域的交通参与单元、动态通行定义和每 world 可变交通状态。当前首个
projection 仍是 vehicle/dynamic Route 特化，但不得反向冻结终态 Runtime。
Static/shared contract 不得留在 Runtime；否则 compiler/validator 会反向依赖动态
运行时。Spatial 继续不依赖 compiler/validator/引擎，Runtime 继续不依赖 Spatial。
`laneflow-data` 在主代码路径切换期间作为 current JSON 临时内部加载实现存在，终态
不再拥有静态 normalization authority，也不保留 current JSON 离线导入入口。

### 10. 迁移必须保持当前态（Current）与目标态（Target）语义可区分

本 ADR 在 Accepted 前是 #291 的目标决策输入。#301 G1 修订本节目程，不改变
compiler 拥有静态路网、Runtime 终态名、crate 分层或「不得用 Core 对象图当 compiler
IR」：

- #301 完成前：Traffic v0.10 / SpatialPackage v0.1 / ScenarioManifest v0.1、
  `InitialTrafficData` 与 current loaders 仍是仓库内可运行的 current 路径；目标
  `laneflow-runtime` 文档须标明尚未作为可运行世界落地。
- #301 完成后：`laneflow-runtime` / `TrafficWorld` 是唯一可运行交通世界；current
  JSON 与 `laneflow-core` 不再是 production contract，必须随 #301 完成 PR 从运行时
  入口拆除。文档把 Runtime 写成已实现目标路径，不得再写「未实现 / 非 current API」。
- 新实现切片不得以 current object graph 作为 compiler IR，也不得让过渡兼容反向
  塑造终态布局。
- integration-only LIR→Core 投影随 Core 一并删除；compiler 正确性看 LFCA/制品，
  需要 tick 的证据只走 Runtime。禁止用同一场景的 `CoreWorld` 对拍作为 #301/#305
  门禁。
- #301 的行为证据是新的 Runtime 端到端示例与测试，不是 current/target 双路径等价。
  启动成本、retained memory 与规模证据仍由 #441/#305 按自身 G1 记录，不以 Core
  为对照物。
- `laneflow-core/CoreWorld` → `laneflow-runtime/TrafficWorld` 的 crate 拆除在 #301
  完成；#294 不再独占生产切换。若仍需独立 Issue，只处理文档导航 / Skill 标识符等
  残留改名。

### 11. 静态执行约束、分区提示与运行时执行计划分层

编译器从规范 LIR 派生静态执行约束图（Static Execution Constraint Graph），表达：

- 冲突、等待、下游存储、控制器和其他共享资源的依赖组件；
- 可以安全并行求值或切分的边界；
- 提案（Proposal）、资源声明（Resource Claim）、事件（Event）和变更（Mutation）
  的规范合并键与提交顺序；
- 无法局部化时必须归入同一归约权威的连接资源组件。

该图是静态交通语义的派生事实，必须对工作线程数和实际分区计划（Partition Plan）
中立。编译器
还可以发射分区规划提示（Partition Planning Hints），例如预估成本、边界权重和推荐
切分（Cut）；提示只影响性能，必须可丢弃、可重建，并且不能改变已提交状态
（Committed State）。

每个 `TrafficWorld` 依据静态约束、硬件拓扑、世界容量和动态负载建立可变的
运行时执行计划（Runtime Execution Plan），拥有实际分区/工作线程分配、
边界交换缓冲、迁移状态和调度统计。它不得回写共享 image，也不得成为存档跨硬件
恢复的行为权威。

精确执行路径（Exact Execution Path）统一遵循已提交状态 `T` →
提案/资源声明/归约（Reduction）→ 原子提交 `T + Δ`。所有分区只读取 `T` 和同一
tick 的规范输入；跨分区依赖不能通过额外一 tick 边界邻域延迟（Halo Delay）获得
性能。每个连接资源组件只有一个规范归约权威，但互不相交的组件可以由不同工作线程
并行归约；“单归约器（Single Reducer）”不等于全世界永久只有一个归约线程。

唯一规范归约权威定义唯一结果与规范顺序，不规定一个组件由单一物理线程串行折叠。
生产候选必须比较资源分段、强连通分量（Strongly Connected Component，SCC）、凝聚
有向无环图（Condensation Directed Acyclic Graph，Condensation DAG）波次、稳定局部
归约与固定合并树；当前集中式组件合并只作为精确参考预言机（Exact Reference
Oracle）。运行时至少报告组件/SCC 分布、最大 SCC 提案占比、归约工作量、归约跨度与
临界路径，并在信号协调、冲突区链、排队溢流和长前车链（Leader Chain）工作负载上
证明单大型世界的有效并行度。生产 fast path 的工作量必须与当前 tick 的提案、声明、
被触及资源/SCC 和依赖弧线性相关，不得扫描完整世界、完整静态组件或全部未激活资源；
有类型资源序号预分桶、固定宽度稳定键的确定性基数/桶排序和复用工作线程缓冲是默认
研究方向；静态依赖可以预编译结构上界，前车链等动态依赖必须增量组装活跃图，不能
错误套用静态 SCC。全局比较排序只保留为参考路径。#220 拥有生产分区/归约设计，
#72 保留研究证据根；不可分解跨度若成为阿姆达尔瓶颈（Amdahl Bottleneck），必须
回到 G1，不能只增加工作线程。

提案/声明/归约/提交是可观察语义的逻辑阶段，不强制单工作线程物化通用队列、任务图
或真实屏障。融合单工作线程执行器可以消除无并发竞争的脚手架，但其提交状态、事件、
错误和规范顺序必须与显式参考精确路径（Reference Exact Path）等价；实现不能以性能
为由省略逻辑仲裁或产生依赖工作线程数量的行为。

本 ADR 不冻结分区算法、任务运行库、固定边界邻域宽度或数值归约算法。工作线程数、
分区计划和任务完成顺序不得改变精确执行路径的已提交状态、事件或安全结果。未来
保真度（Fidelity）降级必须进入独立 ADR/G1，不能伪装成调度优化。

### 12. 运行时快照、回放和路径规划不进入共享静态权威

运行时快照（Runtime Snapshot）是独立版本化制品，至少绑定创建快照时的
`originCanonicalArtifactDigest + originCanonicalArtifactByteLength`、
`originStaticImageDigest + originStaticImageByteLength`、运行时/快照版本、
identity/constraint/execution-constraint versions、
`networkRevisionDerivationVersion + networkRevision`、world identity、tick、
输入命令游标和全部每世界可变交通状态；只有后续 G1 显式授予的 Runtime 自有随机流
才进入该快照。Caller-owned seed/随机流由上层 Save Manifest 绑定，不进入 Traffic
Runtime 隐藏状态。保存和恢复只能从 `TrustedStaticImage` descriptor 复制/比较
修订 token，不能由调用方或镜像头覆盖。

同修订恢复可以使用另一个已认证 target/profile image，即使 compiler provenance 或
artifact envelope 重发布导致其 canonical artifact digest/length 不同；候选可信镜像
必须由独立验证收据证明从自身 artifact 重算得到与快照相同的版本化路网修订，
identity/constraint/execution-constraint versions 必须精确相等，并通过
`StaticIdentityIndex` 完整重建稳定静态引用。快照中的原规范制品与原镜像
digest/length 只承担来源审计及同制品/同镜像快速恢复，不是同修订兼容条件。跨修订
恢复必须通过稳定标识与受信任语义差异执行显式迁移；旧 dense ordinal 不得直接解释
为新修订实体。动态通行定义、交通参与单元和其他运行时实体以快照局部标识保存引用
关系；当前 vehicle/dynamic Route 只是首个特化。原进程 runtime
handle/slot/generation 不得成为恢复后身份。

任何保留旧世界状态的跨修订切换/恢复都必须由受信任 semantic diff 驱动。外部可信
`NetworkRevisionCutoverDescriptor` 必须绑定 base/target canonical artifact 和
static image digest/length、base/target 路网修订标识、
`baseCanonicalArtifactByteLength` / `targetCanonicalArtifactByteLength`、
`semanticDiffFormatVersion + semanticDiffDigest + semanticDiffByteLength`、
migration policy version 与
`validationReceiptFormatVersion + validationReceiptKind +
validationReceiptDigest + validationReceiptByteLength`，其中种类必须是
`revision-cutover-v1`。Runtime 先认证该小型
descriptor，再按调用方上限和统一 pre-hash 规则有界读取 semantic diff；随后核对其
base/target revision/artifact/image 三元组与两个可信 image descriptor 精确一致，
再以旧/新 `StaticIdentityIndex` 复核全部映射。未绑定或验证的 diff 只能用于诊断，
不得成为状态迁移权威；缺失 diff 时也不得用两个索引和迁移策略执行 index-only
回退，因为稳定身份相同不能证明 topology、geometry、access 或 control 语义相容。
本地玩家改路必须先由独立验证器比较两份已验证 portable artifact、重算或验证完整
diff 并签发 receipt，再由宿主认证 asset chain 或 pinned digest 认证切换描述符；
否则旧世界继续运行。显式放弃旧状态并在新修订上创建空世界不属于状态迁移。
运行时执行计划在恢复时按当前硬件重建，快照不得要求复现原分区/工作线程布局。

回放使用显式输入命令序列（Input Command Sequence）、checkpoint 和确定性状态
摘要。调试构建可以借助冷标识与源映射产生失同步诊断制品，定位首个分歧 tick、
phase、实体和资源组件。

交通运行时从已提交状态导出交通观测快照，不拥有全局成本政策。路径规划/出行编排
层结合静态路网、观测、收费、游戏政策和偏好构造动态成本快照并产生候选路径。
出行需求和路线选择策略由上层出行与交通编排拥有；交通参与单元 fixed-tick 热路径
不执行全图寻路。成本快照和候选通行定义绑定从当前 `SharedNetworkRevision` 根或已
提交观测快照取得的路网修订 token、观测 tick 与成本模型版本；当前车辆
执行域使用 Route，未来执行域由其 G1 冻结等价通行定义。Runtime 对修订不匹配失败
关闭，并继续验证候选静态引用/拓扑；修订标识相等不替代内容验证。#303 G1 已接受合同在
[`traffic-observation-and-routing-integration.md`](../design/traffic-observation-and-routing-integration.md)
增加观测状态序号，并冻结宿主自有 Routing + LaneFlow 纯契约边界、fixed-tick 过期
窗口与 full/delta/partition 语义；跨修订迁移继续只走 #302 事务。
已提交观测快照必须允许按观测导出节奏（Observation Export Cadence）的完整基线与
版本化增量/分区选择；一致性时点不意味着每 tick 全量复制全网。实现 Gate 同时量化
观测导出、动态成本快照接收和候选通行定义注册边界；上层成本模型算法仍不进入
Traffic Runtime。

## 性能与确定性契约

实现进入 G2 前必须冻结并验证：

- image hot tables 使用 SoA/CSR/flat ranges，索引宽度默认 `u32`；
- 顶层 section byte offset/length 使用 checked `u64`，避免把城市级 image 人为限制
  在 4 GiB；table row ordinal、count 和 hot relation range 仍使用 `u32`；
- Traffic Runtime/Spatial 对 image 的读取不经 trait object、字符串 resolver 或 per-record bounds
  graph walk；必要安全检查在 image verifier 或 batch boundary 聚合；
- steady tick 与稳定容量 pose batch 零分配；
- 多 world 共享 image 时静态内存不按 world 复制；
- 多世界共享不能替代单个大型城市世界的 partition/barrier/边界交换与动态负载
  基准；
- static image 加载时间、峰值分配和 retained memory 相对 current JSON +
  normalization 基线具有量化 Gate；
- 描述符/完整性清单认证、必需节预先验证（Eager Verification）、Spatial
  延迟验证/后台验证（Lazy/Background Verification）和显式全镜像审计分别报告读取量、
  墙钟耗时（Wall Time）、CPU 并行度与峰值分配，
  并在最低产品硬件 sizing；
- 当前直接路径、显式参考精确路径、融合单工作线程与多工作线程精确路径
  分别报告阶段/端到端成本；一万/十万交通运行时固定步进（Traffic Runtime Tick）与
  空间位姿（Spatial Pose）既有能力基线不得无解释回退；
- `traffic-headless-v1` 不携带 Spatial bytes；2/8/32 worlds 共享 Traffic section；
- load limits 在任何 per-world allocation 前验证，恶意 cardinality/size 不得触发
  无界分配；
- 单线程/并行、clean/incremental、受支持平台的 portable artifact 逐字节一致；
- 同一冻结场景在支持的 worker 数与 partition plan 矩阵中产生相同 committed
  state、事件和确定性状态摘要，且不引入 partition-induced extra tick delay；
- 单大型世界在信号协调、冲突区链、排队溢流和长前车链场景下报告组件/SCC
  分布、最大 SCC 提案占比、归约工作量/跨度、临界路径和实际扩展效率；
- 在线镜像切换报告准备期 tick 干扰、日志增长/溢出、追赶落后量、静默提交停顿、
  双修订峰值内存、失败放弃和切换事件批次恰一次发布；维护暂停模式单独报告完整停顿；
- 运行时快照保存/加载（Save/Load）报告制品大小、墙钟耗时、主线程停顿、后台干扰、
  峰值内存与回放；观测完整/增量（Full/Delta）导出和动态成本快照接收/候选注册报告
  字节、分配、节奏与 tick 干扰；
- target-specific image 只要求相同 target/layout/profile 下逐字节一致，不要求不同
  target 的 bytes 相同，但它们必须引用同一 canonical digest。

Target image 的具体零拷贝/归档实现（自有 offset tables、经过审计的 archive crate 或
其他方案）必须通过布局稳定性、安全 verifier、MSRV、维护性和 benchmark 比较后
选择；本 ADR 不以某个库名替代性能与安全契约。

## 对既有 ADR 的取代矩阵

| ADR  | 继续有效                                                                                                      | 本 ADR 目标取代                                                                           |
| ---- | ------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------- |
| 0005 | 外部稳定标识（External Stable Identity）、有类型密集句柄（Typed Dense Handle）、热冷分离、动态车辆 / 路线生成 | 核心初始化时分配静态句柄 / 登记表；目标态动态执行层改名为交通运行时（Traffic Runtime）    |
| 0007 | 运行时（Runtime）不依赖 Serde / 文件系统 / 引擎，以及内存中字节边界（In-memory Bytes Boundary）               | 私有 DTO → `InitialTrafficData` 作为终态唯一输入、核心构造器作为全部静态规范化权威        |
| 0008 | 精确当前版本（Exact-current）、失败关闭（Fail-closed）、离线迁移（Offline Migration）                         | 单一 `formatVersion` 同时承担来源、制品与静态镜像布局版本                                 |
| 0011 | 不可变发布（Immutable Publication）、规范 URL（Canonical URL）、来源沿袭（Provenance）、运行时不联网          | 发布目录只描述 JSON Schema 系列；未来扩展规范制品、静态镜像变体与验证收据                 |
| 0013 | 交通运行时 / 空间层 / 适配器权威、规范几何、长度 / 位姿、失败原子性、无图形支持                               | 交通 / 空间两个独立制品在运行时按外部标识（External ID）和清单摘要（Manifest Digest）联结 |
| 0015 | 有界规范 `f32` 坐标框架、误差 / 内存 / 批量性能边界                                                           | 空间层初始化时从核心句柄和 JSON 几何重建登记表；静态镜像的空间节保持可选                  |
| 0017 | 路口、通行流向、机动路径、机动门、路线出现项语义，共享内部边，热路径无字符串                                  | 核心规范化 / 路线注册期首次编译静态出现项；静态初始出现项改由编译器镜像预编译             |

本矩阵只取代“工作发生在哪一层、何时发生、如何存储”的条款，不重新定义上述 ADR
已经冻结的交通、空间和运行时行为语义。ADR 0020 Accepted 状态提交在各历史 ADR
页头登记精确的目标态取代范围；#301 拆除 `laneflow-core` 运行时入口时，再原子更新
各历史 ADR 的实际 Superseded/Partially Superseded 状态。

## 后果

正向后果：

- 静态路网只有一个编译权威，Traffic/Spatial 不再重复解析、绑定和建索引；
- source language 可独立演进，Synthetic DSL 不会成为 Geometry/Import/Editor 的下级；
- portable governance contract 与 target performance layout 解耦；
- Traffic Runtime/Spatial 可共享只读静态内存，headless 不携带 geometry，多 world 只
  分配可变状态；
- JSON、Serde、external IDs 与 hash maps 退出生产启动和热路径；
- source map、semantic diff、独立 validator 和 external trust receipt 同时提高
  可审阅性与系统性 bug 防线。

成本与风险：

- 需要一次跨 compiler/Data/current Core/target Runtime/Spatial/Adapter 的架构迁移；
- portable artifact、static image、descriptor/receipt 和 validator 各自需要版本、
  fuzz、安全与发布治理；
- image verifier 处理不可信二进制，任何 unsafe/zero-copy 方案都必须接受严格审计；
- compiler 的确定性并行、增量 invalidation 和 semantic diff 增加实现复杂度；
- cutover 前 current/target 双路径会暂时增加测试矩阵，但不能成为长期兼容层。

## 被拒绝的替代方案

### L1 生成 Core 输入（Core Input），L2 再补几何（Geometry）

它把 frontend 误建模为有序架构层，让 Core object graph 成为 IR，并使 geometry、
标识和拓扑的联合验证发生得太晚；拒绝。

### 以 `InitialTrafficData` 或 Core 登记表（Registries）作为共享中间表示（IR）

这些类型已经含有 runtime normalization、handle 和 Core-specific occurrence，
无法保持 target-neutral，也不能同时服务 portable artifact、Spatial layout 和
semantic diff；拒绝。

### 规范 JSON（Canonical JSON）直接作为唯一生产制品

它适合公共审计与兼容，但会把解析、字符串引用和 object graph 重建留在生产启动，
且布局无法针对 Core/Spatial 热路径优化；拒绝。

### 一个跨平台二进制同时承担可移植制品（Portable Artifact）与静态镜像（Static Image）

为追求单一 bytes 会冻结最低公共布局、阻碍 target-specific alignment/SIMD/feature
裁剪，并把长期兼容与高性能加载绑在一起；拒绝。

### 编译器验证（Compiler Validation）与独立验证器（Independent Validator）复用同一语义实现

这只能证明同一实现自洽，不能发现系统性 compiler bug；拒绝。

### 运行时（Runtime）加载时重新执行完整语义验证（Semantic Validation）

它保留了当前启动成本和重复权威。Runtime 只验证结构/runtime precondition，并通过
external descriptor/receipt 继承已完成的 semantic trust；完整语义由 compiler +
independent validator 在发布前完成；拒绝。

### 仅凭稳定身份索引执行跨修订状态迁移

`StaticIdentityIndex` 只能证明 StableId128 与 target-specific dense ordinal 的
双向对应，不能证明保持同一稳定身份的实体没有发生拓扑、几何、访问规则或控制语义
变化。仅凭旧/新索引和迁移策略可能把旧占用、动态路线、预约或控制器状态附着到错误
语义；拒绝。跨修订迁移必须消费由独立验证器覆盖完整 base/target 语义比较、并由可信
切换描述符绑定的 semantic diff。

### 仅凭镜像头（Image Header）的规范摘要 / 来源沿袭（Canonical Digest / Provenance）接受不可信字节

攻击者可以伪造这些 header 声明、对恶意 image bytes 计算新的
`staticImageDigest`，并构造结构合法但语义恶意或资源消耗过大的 image。Production
fast path 必须有认证的 image 外部 trust anchor；拒绝 header self-attestation。

### 强制交通节（Traffic）与空间节（Spatial）同时存在

这会违反 ADR 0013 冻结的“Core 不依赖 Spatial、无图形宿主可运行”边界。Traffic、
`StaticIdentityIndex` 与 `PartitionPlanningHints` section 必选，Spatial section
由 closed profile 控制；拒绝 mandatory Traffic/Spatial combined payload。

### 把最终分区和 worker 分配烘焙进静态镜像

这会把可共享静态事实与硬件、世界容量和动态负载绑定，导致同一地图在不同设备或
存档恢复时产生错误耦合；镜像只保存静态执行约束与可重建提示。

### 用额外一 tick 的跨分区延迟换取简单 halo

这会让结果依赖 partition cut 并改变跟车、信号、冲突和停车时序，不属于精确路径；
拒绝。任何近似路径必须拥有独立 fidelity contract。

### 把固定点或精确累加器预先规定为全部并行归约方案

当前热资源主要是 occupancy、leader、proposal、claim 和 store，而非大规模无序
浮点求和。数值表示和归约算法必须由实际 hotspot、误差与平台基准选择；本 ADR 只
冻结确定性结果契约。

## G1 接受结果

#291 G1 已确认以下条件；后继实现必须保持这些边界。若修改其 closed contract 或
职责分层，必须重新进入相应架构 G1：

1. `network-compiler.md`、ADR 0021 与本 ADR 对 source module graph、IR、标识、
   artifact/image、trust/profile、execution planning、crate DAG 和 migration 的
   描述一致；
2. architecture、Data、current Core/target Traffic Runtime、Spatial、Adapter 文档
   清楚区分 current production 与 target；
3. #292 不再承诺 L1 或 Core-shaped 输出，而是 compiler foundation + Synthetic DSL
   frontend 的首个纵向实现；
4. 受影响 ADR 的继续有效与取代范围逐项登记；
5. 性能 Gate 包含 headless profile、启动/load limits、内存共享和一万/十万
   runtime；编译器 workload、规模与预算由后继实现 G1 依据产品证据独立冻结，不从
   运行时参与单元规模反推；
6. untrusted image rejection、标识 registry known vectors、全 production profile
   `StaticIdentityIndex` 与 external descriptor/cutover descriptor trust path 已进入
   implementation acceptance；
7. 所有 `LaneEdge` 均以独立稳定边键进入身份闭包；道路区段 / 路口角色变化不改写
   既有边身份，current 合法未覆盖边无需伪造所有者即可迁移；
8. 未把具体 archive library、并行框架或增量数据库当作未经基准的既定事实。
9. 城市模拟游戏上层、路径规划、不可变路网修订、运行时快照和每世界唯一性边界
   已同步到 architecture、roadmap、glossary 与 Agent Skills。
10. 跨修订状态迁移必须有受信任 semantic diff 与独立验证证据；稳定身份索引只复核
    映射，缺失证据时 index-only 路径失败关闭。
