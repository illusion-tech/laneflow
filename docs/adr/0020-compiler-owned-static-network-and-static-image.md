# ADR 0020：编译器拥有的静态路网与目标静态镜像

**状态**: Accepted<br>
**日期**: 2026-07-29<br>
**适用范围**: 正本来源模块图（Authoritative Source Module Graph）、编译器中间表示
（Compiler IR）、静态路网正本、可移植规范制品（Portable Canonical Artifact）、
目标静态镜像（Target Static Image）、验证收据（Validation Receipt）、源映射
（Source Map）、独立验证器（Independent Validator），以及当前态 Core / 目标态
交通运行时（Traffic Runtime）、Data、Spatial 初始化边界<br>
**目标取代范围**: ADR 0005、0007、0008、0011、0013、0015、0017 中与静态数据 normalization、制品配对和运行时 registry 构建位置冲突的条款，并在目标态以 `LaneFlow Traffic Runtime`/`laneflow-runtime` 一次性不兼容替代当前 `LaneFlow Core`/`laneflow-core`。<br>
**迁移时间表修订（#301 G1）**: 仓库未发布 1.0、无外部运行时消费者。#301 完成后 `laneflow-runtime` / `TrafficWorld` 即唯一可运行交通世界，并拆除 current Core/JSON 运行时入口；不再等待 #294 才切换正本。#294 不再拥有生产切换或拆除旧运行时路径。精确消费契约见 `../design/traffic-runtime-shared-consumption.md`。<br>

**后继决策**: ADR 0029（Accepted；#498 G1）取消本文把 `StaticRoute` 列为 Identity
v1 必选声明种类、并由编译器预编译初始路线出现记录的条款。ADR 0024（Accepted；#299 G1 Pass）已部分取代本文关于独立
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
只属于当前仓库加载器和 fixture；它不建立 compiler import frontend、批量迁移工具或长期
离线兼容入口。编译器迁移正确性以原生有类型来源模块和集成专用 LIR→current 投影
验证。该边界只约束 Traffic v0.10、SpatialPackage v0.1 与 ScenarioManifest v0.1
这组从未对外发布的仓库内 JSON：不提供其编译器导入前端、批量迁移工具或长期离线
兼容入口（原 `current-package-import.md` 已随 #1382 退役）。本澄清不改变通用外部来源导入
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
  - `0021-traffic-infrastructure-and-host-boundary.md`
  - `0024-compiler-post-emission-check-and-minimal-publication-closure.md`
- 详细设计:
  - `../design/network-compiler.md`
  - `../design/compiler-foundation.md`
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

本 ADR 的所有领域术语遵循 `../reference/glossary.md`：以中文术语和中文定义为准，英文只作辅助理解。Rust 类型、crate、字段、版本值、算法和协议常量等精确
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
  编译裁决方；
- 单个 compiler bug 可以系统性污染全部生成资产，仅复用 compiler validation
  不能形成独立预言机。

#291 的目标不是在现有路径旁增加一个生成工具，而是把全部静态路网语义前移到
离线编译期，并让 target Traffic Runtime 只从具有外部信任锚的派生静态镜像建立
只读 view。

ADR 0021 定义交通基础设施与宿主边界。静态编译设计既要优化加载，也要保留单个
大型交通世界的并行扩展、交互式道路编辑、存档/回放、路径规划接入与每世界唯一性
演进空间；城市经济和出行需求继续由宿主拥有。

## 决策

### 1. 编译器是全部静态路网语义的唯一编译裁决方

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
安全约束与运行时状态裁决权（Runtime State Authority）；Spatial
继续拥有 canonical geometry sampling 和 pose 语义；二者不再各自重新解释或重建
静态路网。Current production 的 `LaneFlow Core` 命名、车辆特化和 API 在 cutover
前继续有效。

### 2. 正本来源模块图（Authoritative Source Module Graph）组合平级前端（Frontend），不采用 L1/L2

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
source module graph 仍是逻辑编制正本，但其可重放物理编码必须服务编辑器、程序化
生成、局部加载和协作，而不以人工文本编辑或原始 Git diff 为产品前提。

每个 frontend 只负责自己的语法、来源保真和 source span，不得输出
`InitialTrafficData`、Core registry 或 target runtime object graph。不同 frontend
可以在同一个 compilation unit 中导入模块，并在 HIR 之后共享完全相同的语义
管线。Import 必须保留原始 source digest、importer build/options 和 provenance；
publishable programmatic generator 必须保留 build ID、参数、seed、namespace 与
输入 digest。匿名 AST 注入只允许非发布测试。

### 3. 采用有类型抽象语法树（Typed AST）→ 高层中间表示（HIR）→ 中层中间表示（MIR）→ 已验证规范低层中间表示（Canonical LIR）

编译器中间表示按稳定职责分层：

| 阶段                              | 正本与允许内容                                                          | 禁止内容                                                  |
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
  `SharedIdentityIndex`；
- iteration order 与输出不依赖 hash table iteration；
- 增量 cache key 只影响重用，不影响完整编译结果；
- 并行 pass 必须以稳定 merge order 产生与单线程逐字节相同的输出。

标识编码封装（Identity Envelope）与实体登记表（Entity Registry）使用独立版本
轴。`identityEncodingVersion = 1`
冻结 magic、kind、field count、strictly-increasing tag/length/value bytes；
`identityRegistryRevision = 4` 覆盖 topology、Gate/Waiting、Signals/Phase、
Parking、cross-section/access/profile、canonical frame、`ConflictZone` 与
`ParticipantStream` 与 #284 新增的 `RightOfWayPolicySet` declaration，并统一使用 `ParkingFacility` 名称；既有 identity
canonical bytes 不变。
kind `1..=24` 与 field tag `1..=35` 连续登记；ADR 0029 新增的 `ConflictZone` / `ParticipantStream`
使用已退役且从未对外发布的编号槽位，不改变任何既有合法 kind/tag 或 StableId
（ADR 0029）。新增 kind 只 append registry revision；修改既有 kind 的字段集合、tag 含义
或编码必须提升 encoding version。完整
kind/tag/required-sequence
表以 `network-compiler.md` 为规范。所有**定义子实体身份**的父子关系必须以父实体
StableId 作为父锚点，不能只复制父实体在其来源模块内稳定的裸局部 key；这样跨模块
同名父实体与重新归属仍由完整命名空间裁决。可选组织关系不因此自动成为身份原像；
例如 `ParkingSpace.facilityId` 仅在显式存在时形成
`ParkingSpace -> ParkingFacility` 关系，
不参与 `ParkingSpace` 的 StableId128 派生。

### 4. 一个规范低层中间表示（Canonical LIR）产生四类配套输出

一次成功编译原子地产生：

1. **可移植规范制品（Portable Canonical Artifact）**：平台无关、确定性、可发布和长期审计的静态路网
   事实；内含每个稳定实体的规范身份表（Canonical Identity Table）
   `CanonicalIdentityTable`，保存完整 field-tag/value 原像、声明的 `StableId128` 与
   typed ordinal，承接 public artifact、独立身份重算、迁移和跨实现互操作；
2. **目标交付（原「目标静态镜像」）**：镜像文件/ABI 已由 ADR 0025 决定不交付，
   现行目标交付是受检 LFCA 经 `laneflow-static-network` 构建的进程内
   `SharedNetworkRevision`；
3. **源映射 / 诊断制品（Source Map / Diagnostics Artifact）**：从
   `(entityKind, StableId128, typed ordinal)` 到 source span 和 pass provenance 的
   映射；使用绑定 `networkRevision`、`canonicalArtifactDigest`、compiler build 与
   compilation provenance 的版本化 `SourceMapEnvelope`，可以冗余 tuple 用于显示，
   但该冗余不得作为身份验证原像；身份重算只消费规范制品中的
   `CanonicalIdentityTable`；
4. **语义差异（Semantic Diff）**：以稳定标识和字段语义描述新增、删除、重接、
   geometry/behavior 变化，供 PR/Gate 审阅；使用绑定旧/新路网修订、规范制品摘要与
   精确长度的版本化 `SemanticDiffEnvelope`。

规范发布描述符（Canonical Publication Descriptor）
`CanonicalPublicationDescriptor` 对完整 portable artifact bytes 绑定
`canonicalArtifactDigest + canonicalArtifactByteLength`，并以
`sourceMapDigest + sourceMapByteLength` 绑定完整 `SourceMapEnvelope` exact bytes
（收据绑定已随 ADR 0024 取消）。源映射封套绑定单份制品摘要与编译来源链；语义
差异封套绑定旧/新制品摘要、精确长度和路网修订；目标交付引用其来源制品摘要，
不再有独立的镜像外 descriptor 绑定（ADR 0025）。任何输出失败，本次 compilation
unit 都不得发布部分结果。

Portable canonical artifact 是发行与治理契约，不是运行时热布局。Target static
image 是可丢弃、可按目标重新生成的派生制品，不获得独立 authoring authority。
authoritative source module graph 始终是唯一 authoring SSOT；generated artifact
是否 checked in 只属于发布和审阅策略，不能形成第二套可手改事实来源。

### 5. 交通运行时（Traffic Runtime）与空间层（Spatial）消费共享静态路网

本节原「配置档控制的静态镜像」目标格式——`StaticNetworkImage` 分节容器、
`traffic-headless-v1` / `traffic-spatial-v1` 配置档、平面分块完整性清单、
eager/lazy/background 分块验证与镜像外描述符机制——已由 Accepted ADR 0025 /
#300 G1 决定不交付。现行交付是受检 LFCA 经 `laneflow-static-network` 构建
`SharedNetworkRevision`，无镜像文件、ABI 或配置档；原方案全文见基线提交
[3f80b65f][adr0020-history]。

仍有效的边界（以现行术语表达）：

- `SharedIdentityIndex` 是 snapshot save/load、动态路线重建和路网修订切换的生产
  必需冷索引，不进入 steady tick；规范身份原像只属于可移植制品侧，运行时不为
  身份重算元数据承担 retained memory 或 cache 成本；
- Spatial 数据存在时与 Traffic 使用同一 logical edge ordinal / cross-index，
  不引入 sparse geometry mapping；
- Adapter 只消费 Traffic committed snapshot 与 Spatial pose batch，不读取
  compiler IR，也不获得静态规则 authority；
- 多个 `TrafficWorld` / Spatial session 共享同一不可变 `SharedNetworkRevision`；
- 生产启动不执行 JSON parse、external string rebind、topology reconstruction、
  Traffic/Spatial package join 或 initial Route occurrence recompile。

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
不得为了存档或共享镜像而新增隐藏随机流裁决权。若后续 G1 显式引入 Runtime 自有随机流，
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
日志溢出、迁移失败或预算超限时放弃候选并继续旧修订。旧修订的共享静态对象在
全部借用视图退出后回收。宿主只有在显式维护暂停模式中才可让整个准备期
停表，不能把它作为在线玩家改路的隐式语义；维护暂停的完整停顿必须单独预算。

共享静态数据按访问频率分层（现行落点为 `SharedNetworkRevision` 的 hot columns
与冷数据索引，无镜像文件分层）：

- **hot**：tick/pose 所需 SoA/CSR 数据、flat ranges、precompiled occurrence、
  speed/length/gate constraint 与 geometry sampling index；
- **冷数据**：`SharedIdentityIndex` 等低频映射索引，进入对象但不进入 steady
  tick 循环；
- **external diagnostics**：display name、source provenance、canonical tuple、
  诊断文本和 publication metadata 由 portable artifact、source map 或独立诊断
  制品携带，不进入共享静态层。

hot path 只使用 typed dense handle、contiguous range 和预编译索引。StableId128、
BLAKE3、XXH3、字符串、hash lookup、path matching 与 schema validation 都不得进入
steady tick。无空间数据需求的构建用 `SpatialBuildOption::Omit`，但
`SharedIdentityIndex` 不能被裁掉。Artifact digest、source map 和外部诊断制品仍能
恢复完整诊断。

### 7. 可移植制品（Portable Artifact）与目标交付分离版本与绑定字段

字段不得再被一个 `formatVersion` 混合表达，也不能把版本号、构建选择器和内容绑定
都称为「版本轴」。本节原列出的 `staticImage*` 与 `validationReceipt*` 字段族已随
镜像方案（ADR 0025 决定不交付）与验证收据（ADR 0024 取消）退役；现行各对象的
版本轴、绑定字段与摘要口径以 `../design/portable-canonical-artifact.md` 为准。
原字段清单见基线提交 [3f80b65f][adr0020-history]。

仍有效的规则：

- 每个摘要都是各自目标对象完整 exact bytes 的 SHA-256，对象不得把自身摘要嵌回
  自身字节序列；摘要必须由受认证的外部对象同时绑定同一对象的精确 `u64` 字节长度。
- 消费者在任何与输入大小成正比的读取、解压、分配、解析或 hash 之前，先检查
  调用方上限、地址空间与 exact length；未知长度 stream 只允许 checked
  length+1 bounded reader，不能以最终 digest mismatch 作为输入大小防线。
- `NetworkRevisionId` 不复用 exact-bytes digest：以带域分离的 SHA-256 对冻结的
  目标无关规范路网语义载荷计算；载荷包含 identity/constraint/execution-
  constraint versions 和全部静态路网语义，排除摘要自身、artifact envelope、工具
  provenance、source map/diagnostics、publication metadata 与目标/布局/分区提示。
  格式检查必须从可移植制品独立重算；同一规范语义的不同目标构建共享修订，任何
  运行时可观察静态语义变化都产生新修订。相同 `NetworkRevisionId` 对应不同规范
  语义载荷时必须以 `NetworkRevisionDigestCollision` 失败关闭，不得追加 ordinal、
  salt 或 suffix。
- `SourceMapEnvelope` / `SemanticDiffEnvelope` 以各自格式版本绑定
  `networkRevision`、`canonicalArtifactDigest` 与来源链/两侧修订记录，不内嵌自身
  摘要；跨修订迁移的外部描述符必须认证同一语义差异版本与摘要长度对，缺失、未知
  或错配时不得解析记录或进入迁移事务。
- 加载与运行时对 layout、target、descriptor 与 digest 采用 exact-current、
  fail-closed 规则，不在生产启动路径执行历史迁移；旧制品经离线 compiler
  upgrade/migration 进入当前版本。

### 8. 验证（Validation）与信任（Trust）

原四道分离防线（compiler validation、独立验证器、验证收据封套/外部描述符、
static image structural verifier）中的后三道已由 Accepted ADR 0024 重新裁决：
pre-1.0 不创建 `laneflow-validator` 独立验证器，验证收据从 LFCP v2 移除，镜像
结构校验随镜像方案一并退役。现行验证边界是编译器语义验证加后发射检查
（`check_post_emission_bundle`：只依据最终 LFCA/LFSM/LFSD、显式 diff base 与
limits 重算 digest/length/revision 并核对跨对象 binding）；发布闭合与信任锚
职责见 ADR 0024 与
`../design/compiler-post-emission-check-and-minimal-publication-closure.md`。
仍有效的原则：任何 header 自报的 digest/target/provenance 只是待核对声明，不能
作为自证 trust anchor；只有目标交付字节而无受检可移植制品来源时拒绝。原四道
防线与收据种类的完整清单见基线提交
[3f80b65f][adr0020-history]。

### 9. 包（Crate）目标边界

现行依赖箭头统一表示“左侧依赖右侧”（原 #291 目标态含 `laneflow-static-image`
与 `laneflow-validator`，两者未创建，历史图见基线提交 [3f80b65f][adr0020-history]；
进程内共享层由 `laneflow-static-network` 承担）：

```text
laneflow-format ----------> laneflow-static-contract
laneflow-format --(std feature 可选 staged 适配)--> laneflow-format-mmap
laneflow-static-network --> laneflow-static-contract / format
laneflow-compiler --------> laneflow-static-contract / format / road-editing-wire
laneflow-runtime ---------> laneflow-static-contract / format / static-network /
                            motion-kernel / runtime-snapshot-wire
laneflow-spatial ---------> laneflow-static-contract / static-network（不依赖 runtime）
laneflow-scenario --------> laneflow-static-contract / static-network / compiler / runtime
laneflow-adapter(bevy) ---> laneflow-compiler / runtime / spatial / static-network
```

无 LaneFlow 内依赖的成员：`laneflow-format-mmap`（仅 memmap2/tempfile，workspace
唯一登记的 unsafe 例外）、`laneflow-road-editing-wire` 与
`laneflow-runtime-snapshot-wire`（仅钉版 FlatBuffers 生成物）、
`laneflow-motion-kernel`（无依赖）。边集与各 crate `[dependencies]` 逐一对应，
省略的部分只是外部依赖。

`laneflow-static-contract` 只拥有 StableId/kind/tag、`NetworkRevisionId`、typed
ordinal 和 version/digest/profile primitives，不依赖 Serde、filesystem、Runtime
或 Spatial。现行 crate 清单以仓库 `crates/` 为准。

Target `laneflow-runtime` 是 current `laneflow-core` 的 clean-break 名称，只拥有
tick、已实现执行域的交通参与单元、动态通行定义和每 world 可变交通状态。当前首个
projection 仍是 vehicle/dynamic Route 特化，但不得反向冻结终态 Runtime。
Static/shared contract 不得留在 Runtime，否则 compiler 会反向依赖动态运行时。
Spatial 不依赖 compiler 与引擎，Runtime 不依赖 Spatial。`laneflow-data` 旧 current
JSON 加载实现已随 #301 拆除，不再拥有静态 normalization authority。

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
- 新实现任务不得以 current object graph 作为 compiler IR，也不得让过渡兼容反向
  塑造终态布局。
- integration-only LIR→Core 投影随 Core 一并删除；compiler 正确性看 LFCA/制品，
  需要 tick 的证据只走 Runtime。禁止用同一场景的 `CoreWorld` 对拍作为 #301/#305 检查。
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
- 无法局部化时必须归入同一归约裁决方的连接资源组件。

该图是静态交通语义的派生事实，必须对工作线程数和实际分区计划（Partition Plan）
中立。分区规划提示（Partition Planning Hints，如预估成本、边界权重和推荐切分）
由 `laneflow-static-network` 在共享静态构建时从受检关系派生（ADR 0025），LFCA 不
保存该 payload；提示只影响性能，必须可丢弃、可重建，并且不能改变已提交状态
（Committed State）。

每个 `TrafficWorld` 依据静态约束、硬件拓扑、世界容量和动态负载建立可变的
运行时执行计划（Runtime Execution Plan），拥有实际分区/工作线程分配、
边界交换缓冲、迁移状态和调度统计。它不得回写共享 image，也不得成为存档跨硬件
恢复的行为裁决权。

精确执行路径（Exact Execution Path）统一遵循已提交状态 `T` →
提案/资源声明/归约（Reduction）→ 原子提交 `T + Δ`。所有分区只读取 `T` 和同一
tick 的规范输入；跨分区依赖不能通过额外一 tick 边界邻域延迟（Halo Delay）获得
性能。每个连接资源组件只有一个规范归约裁决方，但互不相交的组件可以由不同工作线程
并行归约；“单归约器（Single Reducer）”不等于全世界永久只有一个归约线程。

唯一规范归约裁决方定义唯一结果与规范顺序，不规定一个组件由单一物理线程串行折叠。
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

### 12. 运行时快照、回放和路径规划不进入共享静态正本

运行时快照（Runtime Snapshot）是独立版本化制品，绑定创建快照时的来源
LFCA digest/length 与 `networkRevision`、运行时/快照版本、
identity/constraint/execution-constraint versions、world identity、tick、
输入命令游标和全部每世界可变交通状态；现行绑定字段与版本以
`../design/traffic-runtime-snapshot.md` 为准。只有后续 G1 显式授予的 Runtime
自有随机流才进入该快照。Caller-owned seed/随机流由上层 Save Manifest 绑定，不进入
Traffic Runtime 隐藏状态。保存和恢复只能复制/比较修订认证结果，不能由调用方
自报覆盖。

同修订恢复可以换用另一份重算得到相同 `networkRevision` 的受检 LFCA，即使
compiler provenance 或 artifact envelope 重发布导致其 canonical artifact
digest/length 不同；identity/constraint/execution-constraint versions 必须精确
相等，并通过 `SharedIdentityIndex` 完整重建稳定静态引用。快照中的原制品
digest/length 只承担来源审计及同制品快速恢复，不是同修订兼容条件。跨修订恢复
必须通过稳定标识与受信任语义差异执行显式迁移；旧 dense ordinal 不得直接解释为
新修订实体。动态通行定义、交通参与单元和其他运行时实体以快照局部标识保存引用
关系；当前 vehicle/dynamic Route 只是首个特化。原进程 runtime
handle/slot/generation 不得成为恢复后身份。

任何保留旧世界状态的跨修订切换/恢复都必须由受信任 semantic diff 驱动；切换
描述符的现行绑定字段、失败关闭与两阶段提交合同以
`../design/traffic-runtime-revision-cutover.md` 为准（原 image/receipt 绑定清单
已随 ADR 0024/0025 退役，原件见基线提交 [3f80b65f][adr0020-history]）。仍有效的
原则：先认证小型描述符，再按调用方上限和统一 pre-hash 规则有界读取 semantic
diff；核对 base/target 修订与两侧受检来源精确一致后，以旧/新 `SharedIdentityIndex`
复核全部映射。未绑定或验证的 diff 只能用于诊断，不得成为状态迁移正本；缺失 diff
时也不得用两个索引和迁移策略执行 index-only 回退，因为稳定身份相同不能证明
topology、geometry、access 或 control 语义相容。本地玩家改路必须先比较两份已
验证 portable artifact、重算或验证完整 diff 并完成后发射检查，再由宿主认证
asset chain 或 pinned digest 认证切换；否则旧世界继续运行。显式放弃旧状态并在
新修订上创建空世界不属于状态迁移。运行时执行计划在恢复时按当前硬件重建，快照
不得要求复现原分区/工作线程布局。

回放使用显式输入命令序列（Input Command Sequence）、checkpoint 和确定性状态
摘要。调试构建可以借助冷标识与源映射产生失同步诊断制品，定位首个分歧 tick、
phase、实体和资源组件。

交通运行时从已提交状态导出交通观测快照，不拥有全局成本政策。路径规划/出行编排
层结合静态路网、观测、收费、游戏政策和偏好构造动态成本快照并产生候选路径。
出行需求和路线选择策略由上层出行与交通编排拥有；交通参与单元 fixed-tick 热路径
不执行全图寻路。成本快照和候选通行定义绑定从当前 `SharedNetworkRevision` 或已
提交观测快照取得的路网修订 token、观测 tick 与成本模型版本；当前车辆
执行域使用 Route，未来执行域由其 G1 冻结等价通行定义。Runtime 对修订不匹配失败
关闭，并继续验证候选静态引用/拓扑；修订标识相等不替代内容验证。#303 G1 已接受合同在
[`traffic-observation-and-routing-integration.md`][routing-integration]
增加观测状态序号，并冻结宿主自有 Routing + LaneFlow 纯契约边界、fixed-tick 过期
窗口与 full/delta/partition 语义；跨修订迁移继续只走 #302 事务。
已提交观测快照必须允许按观测导出节奏（Observation Export Cadence）的完整基线与
版本化增量/分区选择；一致性时点不意味着每 tick 全量复制全网。实现 Gate 同时量化
观测导出、动态成本快照接收和候选通行定义注册边界；上层成本模型算法仍不进入
Traffic Runtime。

## 性能与确定性契约

实现进入 G2 前必须冻结并验证（目标交付为受检 LFCA 经 `laneflow-static-network`
构建的 `SharedNetworkRevision`，无镜像文件/ABI/配置档——ADR 0025）：

- 共享静态路网 hot columns 使用 SoA/CSR/flat ranges，索引宽度默认 `u32`；
- 制品/来源映射的 byte offset/length 检查使用 checked `u64`，不把城市级输入人为
  限制在 4 GiB；table row ordinal、count 和 hot relation range 仍使用 `u32`；
- Traffic Runtime/Spatial 对共享静态数据的读取不经 trait object、字符串 resolver
  或 per-record bounds graph walk；必要安全检查在受检读取（format capability）或
  batch boundary 聚合；
- steady tick 与稳定容量 pose batch 零分配；
- 多 world 共享同一 `Arc<SharedNetworkRevision>` 时静态内存不按 world 复制；
- 多世界共享不能替代单个大型城市世界的 partition/barrier/边界交换与动态负载
  基准；
- 受检 LFCA 读取与共享静态路网构建的墙钟、峰值分配和 retained memory 相对历史
  JSON + normalization 基线具有量化对照（现行证据口径见
  `traffic-runtime-state-integrity.md` §8 与城市基线）；
- 信任与验证边界以 ADR 0024/0025 为准：编译器语义验证 + 后发射检查
  （`check_post_emission_bundle`），不要求镜像描述符/分块验证体系；
- 直接路径、显式参考精确路径与单/多工作线程精确路径分别报告阶段/端到端成本；
  一万/十万交通运行时固定步进（Traffic Runtime Tick）与空间位姿（Spatial Pose）
  既有能力基线不得无解释回退；
- 无空间数据需求的构建调用使用 `SpatialBuildOption::Omit`，不产生 target 文件；
  2/8/32 worlds 共享同一 Traffic 数据；
- load limits（格式硬上限与调用方运行上限）在任何 per-world allocation 前验证，
  恶意 cardinality/size 不得触发无界分配；
- 单线程/并行、clean/incremental、受支持平台的 portable artifact 逐字节一致；
- 同一冻结场景在支持的 worker 数与 partition plan 矩阵中产生相同 committed
  state、事件和确定性状态摘要，且不引入 partition-induced extra tick delay；
- 单大型世界在信号协调、冲突区链、排队溢流和长前车链场景下报告组件/SCC
  分布、最大 SCC 提案占比、归约工作量/跨度、临界路径和实际扩展效率；
- 在线路网修订切换报告准备期 tick 干扰、日志增长/溢出、追赶落后量、静默提交
  停顿、双修订峰值内存、失败放弃和切换事件批次恰一次发布；维护暂停模式单独
  报告完整停顿（现行合同见 `traffic-runtime-revision-cutover.md` §9）；
- 运行时快照保存/加载（Save/Load）报告制品大小、墙钟耗时、主线程停顿、后台干扰、
  峰值内存与回放；观测完整/增量（Full/Delta）导出和动态成本快照接收/候选注册报告
  字节、分配、节奏与 tick 干扰。

无 target-specific 交付：同一规范语义的不同构建共享 `NetworkRevisionId`，进程内
构建不承诺跨平台字节一致；逐字节一致义务由上一条 portable artifact 承担。

## 对既有 ADR 的取代矩阵

| ADR  | 继续有效                                                                                                      | 本 ADR 目标取代                                                                           |
| ---- | ------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------- |
| 0005 | 外部稳定标识（External Stable Identity）、有类型密集句柄（Typed Dense Handle）、热冷分离、动态车辆 / 路线生成 | 核心初始化时分配静态句柄 / 登记表；目标态动态执行层改名为交通运行时（Traffic Runtime）    |
| 0007 | 运行时（Runtime）不依赖 Serde / 文件系统 / 引擎，以及内存中字节边界（In-memory Bytes Boundary）               | 私有 DTO → `InitialTrafficData` 作为终态唯一输入、核心构造器作为全部静态规范化裁决权      |
| 0008 | 精确当前版本（Exact-current）、失败关闭（Fail-closed）、离线迁移（Offline Migration）                         | 单一 `formatVersion` 同时承担来源、制品与静态镜像布局版本                                 |
| 0011 | 不可变发布（Immutable Publication）、规范 URL（Canonical URL）、来源链（Provenance）、运行时不联网            | 发布目录只描述 JSON Schema 系列；未来扩展规范制品、静态镜像变体与验证收据                 |
| 0013 | 交通运行时 / 空间层 / 适配器裁决权、规范几何、长度 / 位姿、失败原子性、无图形支持                             | 交通 / 空间两个独立制品在运行时按外部标识（External ID）和清单摘要（Manifest Digest）联结 |
| 0015 | 有界规范 `f32` 坐标框架、误差 / 内存 / 批量性能边界                                                           | 空间层初始化时从核心句柄和 JSON 几何重建登记表；静态镜像的空间节保持可选                  |
| 0017 | 路口、通行流向、机动路径、机动门、路线出现记录语义，共享内部边，热路径无字符串                                | 核心规范化 / 路线注册期首次编译静态出现记录；静态初始出现记录改由编译器镜像预编译         |

本矩阵只取代“工作发生在哪一层、何时发生、如何存储”的条款，不重新定义上述 ADR
已经冻结的交通、空间和运行时行为语义。ADR 0020 Accepted 状态提交在各历史 ADR
页头登记精确的目标态取代范围；#301 拆除 `laneflow-core` 运行时入口时，再原子更新
各历史 ADR 的实际 Superseded/Partially Superseded 状态。

## 后果

正向后果：

- 静态路网只有一个编译裁决方，Traffic/Spatial 不再重复解析、绑定和建索引；
- source language 可独立演进，Synthetic DSL 不会成为 Geometry/Import/Editor 的下级；
- portable governance contract 与 target performance layout 解耦；
- Traffic Runtime/Spatial 可共享只读静态内存（`Arc<SharedNetworkRevision>`），
  headless 构建不携带 geometry，多 world 只分配可变状态；
- JSON、Serde、external IDs 与 hash maps 退出生产启动和热路径；
- source map 与 semantic diff 提高可审阅性；系统性 bug 防线由编译器语义验证加
  后发射检查承担（ADR 0024 已取消独立 validator 与 receipt）。

成本与风险：

- 需要一次跨 compiler/Runtime/Spatial/Adapter 的架构迁移（#301 已完成拆除旧
  current 路径）；
- portable artifact、source map 与 semantic diff 各自需要版本、fuzz、安全与发布
  治理；
- 受检读取处理不可信字节，mmap 例外的 unsafe 方案接受 wire 审计（ADR 0034）；
- compiler 的确定性并行、增量 invalidation 和 semantic diff 增加实现复杂度；
- 路网修订切换前后的候选共存会暂时增加测试矩阵，但不能成为长期兼容层。

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

它保留了当前启动成本和重复正本。Runtime 只验证结构/runtime precondition，
semantic trust 经受检 LFCA 来源（受检规范路网输入）继承；完整语义由编译器语义
验证加后发射检查（ADR 0024）在发布前完成；拒绝。

### 仅凭稳定身份索引执行跨修订状态迁移

`SharedIdentityIndex` 只能证明 StableId128 与 typed ordinal 的双向对应，不能证明
保持同一稳定身份的实体没有发生拓扑、几何、访问规则或控制语义变化。仅凭旧/新索引
和迁移策略可能把旧占用、动态路线、预约或控制器状态附着到错误语义；拒绝。跨修订
迁移必须消费受信任 semantic diff，其信任与绑定边界以现行切换事务合同
（`../design/traffic-runtime-revision-cutover.md`）为准。

### 仅凭镜像头（Image Header）的规范摘要 / 来源链（Canonical Digest / Provenance）接受不可信字节

攻击者可以伪造这些 header 声明、对恶意字节计算新的摘要，并构造结构合法但语义
恶意或资源消耗过大的输入。Production fast path 的信任来自受检来源（宿主认证
asset chain 或 pinned digest 绑定的受检 LFCA），拒绝 header self-attestation。

### 强制交通节（Traffic）与空间节（Spatial）同时存在

这会违反 ADR 0013 冻结的“Core 不依赖 Spatial、无图形宿主可运行”边界。Traffic
数据与 `SharedIdentityIndex` 必选，Spatial component 由构建选项控制
（`SpatialBuildOption::Omit` / `RetainAvailable`）；拒绝强制 Traffic/Spatial
组合载荷。

### 把最终分区和 worker 分配烘焙进静态镜像

这会把可共享静态事实与硬件、世界容量和动态负载绑定，导致同一地图在不同设备或
存档恢复时产生错误耦合；共享静态层只保存静态执行约束与可重建提示。

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
   artifact/交付、trust、execution planning、crate DAG 和 migration 的描述一致
   （现行目标交付与信任边界以 ADR 0024/0025 修订为准）；
2. architecture、Data、current Core/target Traffic Runtime、Spatial、Adapter 文档
   清楚区分 current production 与 target；
3. #292 不再承诺 L1 或 Core-shaped 输出，而是 compiler foundation + Synthetic DSL
   frontend 的首个纵向实现；
4. 受影响 ADR 的继续有效与取代范围逐项登记；
5. 性能 Gate 包含 headless 构建（`SpatialBuildOption::Omit`）、启动/load
   limits、内存共享和一万/十万 runtime；编译器 workload、规模与预算由后继实现
   G1 依据产品证据独立冻结，不从运行时参与单元规模反推；
6. 不可信来源拒绝、标识 registry known vectors、`SharedIdentityIndex` 与受检
   规范路网输入/后发射检查信任路径已进入 implementation acceptance；
7. 所有 `LaneEdge` 均以独立稳定边键进入身份闭包；道路区段 / 路口角色变化不改写
   既有边身份，current 合法未覆盖边无需伪造所有者即可迁移；
8. 未把具体 archive library、并行框架或增量数据库当作未经基准的既定事实。
9. 宿主应用层、路径规划、不可变路网修订、运行时快照和每世界唯一性边界
   已同步到 architecture、roadmap、glossary 与 Agent Skills。
10. 跨修订状态迁移必须有受信任 semantic diff；稳定身份索引只复核映射，缺失
    证据时 index-only 路径失败关闭（现行切换事务合同见 #302 / cutover 文档）。

[routing-integration]: ../design/traffic-observation-and-routing-integration.md
[adr0020-history]: https://github.com/illusion-tech/laneflow/blob/3f80b65f3fee19cd6085fc0d8be8f58b7a14d551/docs/adr/0020-compiler-owned-static-network-and-static-image.md
