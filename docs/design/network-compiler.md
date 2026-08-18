# 路网编译器与目标静态镜像

> **后继架构修订（2026-08-18）**：Accepted ADR 0024 / #299 G1 已把独立
> `laneflow-validator`、规范发布 receipt 和三类 receipt 统一交付方案替换为
> `laneflow-format` 共享后发射检查与最小发布闭合。canonical publication 使用
> LFCP v2 且不再包含 receipt；#300/#302 必须分别重新冻结镜像与切换信任边界。该
> 替换已经生效；本文 v1/receipt 条款只保留为 #291/#298 历史设计事实。当前规范见
> `compiler-post-emission-check-and-minimal-publication-closure.md`。

**文档状态**: Accepted（#291 target design）；#315 共同受检模块接入契约已实现；
#297 current JSON 编译器导入设计已取消<br>
**最后更新**: 2026-08-18<br>
**适用范围**: 权威来源模块图（Authoritative Source Module Graph）、编译器中间表示
（Compiler IR）、静态路网编译权威、标识派生、可移植规范制品（Portable Canonical
Artifact）、目标静态镜像（Target Static Image）、源映射（Source Map）、语义差异
（Semantic Diff）、compiler 后发射检查、交通运行时（Traffic Runtime）命名、
静态执行约束（Static Execution Constraints）、不可变路网修订
（Immutable Network Revision）和当前态（Current）→目标态（Target）迁移<br>
**实现状态**: 未实现；当前态生产路径仍使用 Traffic v0.10 / SpatialPackage v0.1 /
ScenarioManifest v0.1、`InitialTrafficData` 和现有空间登记表（Spatial Registry）；
#292 已完成编译器基础设施（Compiler Foundation）+ 合成领域专用语言前端
（Synthetic DSL Frontend）G4；#315 已落地私有共同 Typed AST、逻辑模块/来源文档独立登记、
原子共同接入、文档集摘要与 v2 文档数配置档；#296 已实现内部、未发布的 FlatBuffers B1
道路编辑 production compiler 入口，旧 Geometry JSON 实现只作历史证据；#297 不再建立
current JSON 编译器前端。该入口的实现不表示整个目标路网编译器、静态镜像或 Traffic
Runtime 已经实现

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
- `../adr/0021-city-simulation-game-traffic-foundation.md`
- `../adr/0024-compiler-post-emission-check-and-minimal-publication-closure.md`
- `core-id-handles.md`
- `compiler-foundation.md`
- `portable-canonical-artifact.md`（#298 G1 已重新接受并完成 G4；动态记录以 Issue Gate Ledger 为准）
- `compiler-post-emission-check-and-minimal-publication-closure.md`
- `current-package-import.md`
- `cross-section-access.md`
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

#291 G1 已接受的目标不是“生成另一份 JSON 的工具”，而是新的静态数据体系：

下图为了对应精确类型名和代码搜索保留英文辅助别名；中文规范链路以上一节为准。
图中英文不能独立改变架构语义。

```text
Road-editing modules ────┐
Synthetic DSL modules ───┼─> authoritative source module graph
Imported modules ────────┤                 │
Other checked modules ───┘                 v
                           typed AST -> HIR -> MIR -> validated canonical LIR
                                                          │
                                                          ├─> portable canonical artifact
                                                          ├─> target StaticNetworkImage
                                                          ├─> source map / diagnostics
                                                          └─> semantic diff

StaticNetworkImage ─┬─> Traffic Runtime: StaticTrafficView + per-world mutable state
                    │                    + RuntimeExecutionPlan
                    └─> Spatial: optional StaticSpatialView + batch scratch/output
```

核心结论：

- 取消 L1/L2 作为架构层；不同来源语言（Source Language）/来源模块是平级前端
  输入；
- 编译单元的唯一数据编制权威是可重放的权威来源模块图，几何文档只是生产场景的
  主要来源语言；
- 编译器是全部静态路网的唯一编译权威；
- `InitialTrafficData` 和 Core 登记表不是中间表示；
- AST/HIR/MIR/LIR 逐级降阶，只有以已验证规范 LIR 为静态语义核心的成功编译结果
  可以进入编译发射器；
- 可移植规范制品与目标静态镜像是同一 LIR 的不同后端；源映射另外消费同次成功编译
  冻结、但不能补充静态语义的已验证来源伴随数据；
- 目标态 `laneflow-runtime` / 空间层直接消费同一不可变镜像中的对齐视图；
- 静态只读数据与每世界可变状态物理分离；
- 生产启动不再解析 JSON、按字符串重绑定、重建登记表或重新编译初始路线出现项；
- compiler 发布前由 `laneflow-format` 从最终字节重算 revision/digest/length 并核对
  LFCA/LFSM/LFSD binding，但不复验完整路网语义；
- 稳定声明/可寻址派生实体、所有者局部出现项与密集 LIR 表行使用不同标识；
- 规范标识元组、BLAKE3-128 `StableId128`、XXH3 瞬态加速边界和密集句柄保持
  热路径分层；
- 可信生产快速路径必须有镜像外部的验证/发布信任锚，不能把 header 中可伪造的
  规范摘要/provenance 当成可信证据；
- `StaticTrafficImage`、`StaticIdentityIndex` 与 `PartitionPlanningHints`
  是所有 v1 生产配置档的必选节；只有 `StaticSpatialImage` 由配置档控制，无图形
  交通运行时不携带几何。
- 编译器保存 worker 数无关的静态执行约束，运行时按世界建立实际执行计划；最终
  partition/worker assignment 不进入稳定标识、可移植制品语义或共享镜像。
- 静态镜像是一个不可变路网修订，而非城市永久不变；道路编辑通过新修订与失败关闭
  镜像切换事务进入运行世界。
- Accepted ADR 0021 把服务中国特色城市模拟游戏交通基础定义为 LaneFlow 第一长期
  产品目标，并让城市经济、出行需求、路线选择策略和游戏规则继续由上层拥有。

本文描述目标态。ADR 0020/0021 Accepted 且阶段 8 生产切换 Issue #294 完成 G4 前，
现有 JSON/Data/Core/Spatial 路径仍是当前态生产契约。

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
| 执行规划（Execution Planning）   | 单世界、单一 current tick pipeline                            | 静态约束 + 可重建提示 + 每世界运行时执行计划              |
| 道路修改（Road Modification）    | 重新加载 current package                                      | 新路网修订 + 验证 + 安全边界镜像切换事务                  |
| 验证（Validation）               | schema + loader + Core/Spatial constructors                   | 编译器语义裁决 + 后发射闭合检查 + 有界镜像结构校验器      |

迁移不得把目标态写成现状，也不得为了复用当前态 DTO/constructor 而冻结错误的
编译器中间表示。

## 4. 权威职责（Authority）

### 4.1 数据编制权威（Authoring Authority）

一个 compilation unit 的唯一 authoring authority 是显式、可重放的
**authoritative source module graph**，不是某一种固定文档格式。#291 已接受、#292 已交付的
单文档基线要求每个 module 至少冻结：

```text
moduleNamespaceId
sourceLanguage
sourceContentDigest
frontendVersion
frontendOptionsDigest
origin / provenance
imports
```

`sourceContentDigest` 是官方前端对单份版本化规范来源记录精确字节计算的 SHA-256，只服务
重放与来源沿袭，不参与实体稳定标识。#292 合成领域专用语言前端的首版记录与摘要规则由
`compiler-foundation.md` 冻结。

#315 G2 已以一个逻辑模块拥有一个或多个来源文档的下列形状替代上述单文档摘要字段：

```text
moduleNamespaceId
sourceLanguage
sourceDocumentSetDigestVersion
sourceDocumentSetDigest
frontendVersion
frontendOptionsDigest
origin / provenance
imports
documents[] {
  sourceDocumentKey
  sourceDocumentDigest
  sourceRecordByteLen
  sourceDocumentOrigin
}
```

`sourceDocumentDigest` 是各官方前端对每份版本化规范来源记录精确字节各计算一次的
SHA-256；`sourceDocumentSetDigest` 是对模块内按文档键排序的文档键、精确长度与逐文档摘要进行
版本化、域分隔聚合后的 SHA-256。两者只服务重放、缓存比较与来源沿袭，不参与实体稳定标识。
聚合不能选择某一文档，也不能重新读取和哈希全部来源载荷；精确 v1 前像以
`compiler-foundation.md` 第 3.3 节为权威。任何会改变规范来源记录字节的编码变化必须提升对应
`frontendVersion`；任何改变文档集聚合前像的变化必须提升
`sourceDocumentSetDigestVersion`。现有 Synthetic `sourceContentDigest` 必须逐项等于候选单文档的
`sourceDocumentDigest`；新的文档集摘要具有独立语义。后继前端必须在各自 G1 冻结精确来源字节。

模块级 `origin / provenance` 描述工具、选项与整体转换沿袭，不能替代每份文档自己的
来源记录（Source Document Origin）。后者保存冷的显示/审计来源，并与文档摘要和长度
不可分关联；调用方提供的宿主路径或发布来源声明不自动具有真实性。实现可以把重复
来源字符串驻留到共享表，但每个文档必须保留明确关联。逐文档来源记录不参与
`sourceDocumentSetDigest`、稳定标识或 LIR 语义摘要，其条目、字符串与存续字节必须
纳入资源上限。

- 道路编辑状态是 production 编辑器和程序化生成场景的主要 source language；可视化编辑器、
  第一方 Rust 构造面与 importer 共享有类型道路编辑模型，使用按模块的
  `LF-ROAD-EDITING-SOURCE-v1` FlatBuffers 来源缓冲区持久化；
- Synthetic DSL source 是测试、fixture、benchmark 和示例 module 的权威输入；
- import module 必须记录原始 source bytes/digest、importer build、选项和 provenance；
  选择 materialize 为道路编辑来源模块时，必须显式记录 authority 切换；
- editor 编辑并持久化道路编辑状态，通过稳定实体、属性和画布 selection 关联诊断，
  不私有化 semantic compiler；
- programmatic generator 若参与可发布 compilation，必须记录 build ID、参数、
  seed、namespace 和输入 digest；匿名 AST 注入只能用于非发布测试。

typed AST/HIR/MIR/LIR 都是 source graph 的派生物。Portable artifact 是经过验证的
canonical publication contract，target static image 是可重建性能制品；二者都不得
反向覆盖 source module。

### 4.2 编译器权威（Compiler Authority）

compiler 唯一负责静态路网的：

- symbol/reference/unit resolution；
- topology/geometry 展开与全局语义；
- 稳定声明/可寻址派生实体的 StableId128 与全部 LIR row 的 deterministic ordinal；
- dense logical ordinal、owner/member/reverse indexes；
- Traffic/Spatial 长度共同派生；
- initial/static Route/Maneuver/Gate/Waiting occurrence；
- worker 数无关的静态执行约束、资源依赖组件、规范合并键与可切分边界；
- 可丢弃、可重建且不拥有行为权威的分区规划提示；
- portable artifact 和 static image emission。

### 4.3 运行时权威（Runtime Authority）

- Traffic Runtime：固定步进（Fixed Tick）、已实现执行域的交通参与单元、动态通行
  定义（Dynamic Traversal Definition）生命周期、控制器时钟（Controller Clock）、
  授权/预约（Grant/Reservation）、停驻占用（Stationary Occupancy）、每世界
  运行时执行计划和所有可变交通状态；
- Spatial：canonical geometry sampling 与 pose batch；
- Adapter：宿主 entity、Transform、frame placement、presentation lifecycle；
- static image：只读静态事实，不持有可变 authority。

### 4.4 城市游戏、出行编排与路径规划权威

- 城市模拟游戏层拥有人口、经济、土地利用、建筑、任务与游戏规则；
- 出行与交通编排层拥有出行需求、出发时刻、参与单元生成、目的地、人口生命周期和
  路线选择策略；
- Traffic Runtime 从已提交状态导出交通观测快照，不拥有全局路径成本政策；
- 路径规划/出行编排层结合静态路网、已提交交通观测、收费、游戏政策与偏好构造
  动态成本快照并生成候选路径，不在交通参与单元 fixed-tick 热路径执行全图寻路；
- Traffic Runtime 只裁决交通参与单元如何按所属执行域的静态规则和已提交动态状态
  安全推进。

具体 routing crate/API 不属于 #291 的 closed contract，但这些权威边界不得被
compiler、Adapter 或 scenario policy 绕过。

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

### 5.2 官方前端共同受检模块接入（Shared Checked Module Admission，#315 G2 Implemented）

合成、几何及后续正式编制来源都以字段私有的具体官方前端模块进入
`CompilationUnitBuilder`，但构建器内部只存在一条编译器私有（compiler-private）
原子接入路径。
该路径统一拥有命名空间/来源文档唯一性、共同接入适用的全部资源上限、失败不污染和
规范模块顺序；前端不得复制这些规则。成功 `build` 后，`CompilationUnit` 只保存共同
`TypedAstModule`，HIR/MIR 不按来源语言分支。
来源模块是命名空间和导入图的逻辑节点，可以拥有一个或多个独立来源文档描述符。来源文档按规范模块
顺序和文档键独立冻结序号，每个来源位置都解析自身文档键；不能从模块序号推导文档，也不能为多文档
来源虚构模块、命名空间或导入边。

来源文档是来源伴随记录，不纳入 `TypedAstRecordCount`。#315 增加独立
`SourceDocumentCount`，并通过新的 `LF-COMP-P100-INITIAL-v2` 约束编译单元文档总数；v1
配置档保持不可变。v2 的初始上限、三文档容量推导和重新资格验证以
`compiler-foundation.md` 第 3.3、10.4 节为权威。
v1 只以 `ModuleCount` 隐式约束一模块一文档形状，当前 Synthetic 符合该形状；任何多文档模块都
必须在前端按规模分配和共同接入提交前确认宿主显式选择了 v2 或后继携带
`SourceDocumentCount` 的版本化配置档。多文档正式前端必须在读取、哈希或解析前拒绝
不具备 `SourceDocumentCount` 的配置档；#296 道路编辑来源按“一模块一个 source
buffer/document”应用
同一规则，不得为 v1 猜测默认值或自动升级。

来源专用封装以移动语义进入接入值，不复制完整 AST、字符串或几何。精确来源载荷
（exact source record）只存续到官方前端 `finish` 完成一次摘要、长度和来源位置派生；
共同接入只保留已绑定摘要、精确长度、来源位置、模块级沿袭与逐文档来源记录。借用
调用方输入的前端不得
为接入复制来源全文。记录级循环禁止特征对象（trait object）、前端枚举
（enum）分支、重复
摘要、重复资源计数或第二次规范排序。精确所有权、失败事务和共同接入性能验证以
`compiler-foundation.md` 第 3.3 与 10.4 节为权威。

current JSON 不属于官方编译器前端。`laneflow-current-source` 只服务当前
`laneflow-data` 加载器，不提供严格导入策略、编译器资源余额或位置表；
`laneflow-compiler` 不依赖它。编译器和投影测试直接构造有类型模块，避免用旧 wire
format 决定编译器语义。精确退役边界见
[`current-package-import.md`](current-package-import.md)。

这不是第三方前端插件协议。#315 只冻结公开面继续使用 LaneFlow 拥有的具体入口以及它们进入共同
私有接入的规则，不交付或冻结 Geometry 公共签名。#296 FlatBuffers G1 选择借用完整
`LF-ROAD-EDITING-SOURCE-v1` bytes 的字段私有 `RoadEditingModuleInput` 和唯一原子
`add_road_editing_module`；wire DTO、typed module 和 descriptor 均保持私有。
#297 已取消 current JSON 迁移入口，不建立相应特性、输入类型或导入器。
通用 `add_module`、公共前端
特征（trait）、裸 Typed AST 和裸描述符/内容配对继续禁止。

### 5.3 道路编辑与几何编制前端（Road Editing and Geometry Frontend）

长期 production authoring frontend。#296 FlatBuffers G1 选择
`LF-ROAD-EDITING-SOURCE-v1`（按模块的 size-prefixed FlatBuffers 来源缓冲区）；道路编辑
状态、有类型道路编辑模型、公开 Rust 构造面、generated `unsafe` 审计边界、阶段
生命周期、曲线细分、stationing、资源/诊断顺序、验证矩阵和性能门槛统一由
`road-editing-source-and-geometry-frontend.md` 管理。本节只保留综合架构边界。目标模型包含：

1. 参考线：三维 curve segments、弧长与方向；
2. 横断面：沿参考线分段变化的 lane/facility 结构；
3. 连接：junction/connection intent、默认生成策略与显式 override；
4. 规则：signals、Gate/WaitingZone、access、parking 和其他静态 overlay。

曲线在 MIR 中按冻结 B1 规则离散为 canonical f32 polyline；static image 不保存
authoring curve evaluator。旧严格 UTF-8 JSON Geometry 方案已经归档，不获得来源格式
兼容承诺；新契约已经由当前生产编译器入口实现，但仍属于内部未发布实现契约；B1
不构成已发布存档格式或长期兼容承诺。

### 5.4 导入与编辑器编制（Import and Editor Authoring）

- importer 保存来源 provenance，必须显式生成稳定 key；不允许用导入遍历 ordinal
  冒充标识；
- editor 直接编辑并持久化道路编辑状态；诊断以稳定实体、属性路径、画布 selection 和
  必要的损坏字节范围回传，不要求虚构文本行列；
- importer/Editor 不维护私有 semantic compiler，所有 module 都进入共同 HIR；
- publishable compile 不接收没有 owning module、闭合 `SourceLocation`/provenance 或稳定
  namespace 的匿名 AST。

## 6. 编译器中间表示与编译遍边界（Compiler IR and Pass Boundaries）

### 6.1 有类型抽象语法树（Typed AST）

保留 frontend 语法与来源：

- explicit/derived declarations；
- stable authoring key；
- 闭合 `SourceLocation`、file/module provenance；文本来源的位置分支才是 `SourceSpan`；
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

HIR 仍能追溯全部 source location。跨 module 重名、引用 cycle、unit/type 错误在此失败。

### 6.3 中层中间表示（MIR）

完成全局静态语义：

- corridor/section/lane 展开；
- boundary、edge、junction、movement、path 生成；
- curve tessellation、canonical frame partition 和 geometry continuity；
- signals、Gate、WaitingZone、parking、access 与 topology 绑定；
- 规范标识元组构造与标识闭包（Identity Closure）；
- Traffic length / Spatial arc length 共同派生；
- route/path occurrence 和 reverse indexes；
- 静态资源依赖、连接资源组件、可切分边界和规范合并键；
- global ownership、coverage、coherence 与 policy-independent safety checks。

MIR 可以使用 compiler arena 和临时 cache，但不得产生 target layout 或把 hash
fingerprint 当持久标识。

### 6.4 已验证规范低层中间表示（Validated Canonical LIR）

LIR 是规范制品、目标静态镜像和语义差异发射器的唯一静态语义输入。源映射发射器
另外消费与同次成功编译原子冻结的已验证来源伴随数据；该数据只能关联来源模块、
来源位置、来源沿袭和下列已冻结键，不能补充静态默认、身份前像、所有者或关系：

- 所有 table row 都有 typed `u32` logical ordinal；
- 稳定 declaration 和可独立寻址的 derived entity 另有 StableId128；
- owner-local relation/occurrence 使用 typed local key，不获得全局 StableId128；
- 所有引用已转为 typed ordinal；
- 数值已规范化到 canonical units/representation；
- 所有 relation 以 deterministic flat sequence/range 表达；
- 静态 occurrence、sampling tables 和 layout-independent precompute 已完成；
- 静态执行约束图与 v1 分区规划提示已规范化，且提示与行为语义分离；
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
  -> resolve and validate relations required by identity
  -> derive canonical identity in parent-before-child order
  -> validate remaining global semantics
  -> normalize deterministic LIR order
  -> precompute occurrence/index/sampling/execution-constraint data
  -> freeze validated canonical LIR
  -> emit all artifacts atomically
```

“标识所需关系验证”只提前关闭规范标识元组依赖的引用、所有者 / 成员和唯一性，不
替代后续完整全局语义验证。任何实体若使用父实体稳定标识作为锚点，编译器必须先
完成父实体标识，再对该子实体的父关系证明恰好一个有效所有者；未知引用
（Unknown）、重复引用（Duplicate）、多所有者（Multiple Owner）或零所有者
（Unowned）都必须在派生子实体标识前失败关闭。不得先选择第一个所有者、按排序
结果消歧或生成临时标识后再继续验证。

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

- helper 只能从 parent StableId、稳定 local key 和 semantic role 组合 child key；
- sibling 重排与无关实体插入不改变既有 ID；
- compiler 推断出尚无稳定 key 的 junction/boundary/connection 时，只能产生待确认
  suggestion/diagnostic，不能发布匿名标识；
- authored relation target 的变化通常属于同一 declaration 的 semantic change；
  只有本节明确列入 tuple 的 topology closure 才改变 StableId128。

车道图边（Lane Graph Edge）`LaneEdge` 是基础拓扑中可独立寻址的实体，不以可选的
道路区段覆盖（Road-section Coverage）或派生的路口内部边角色
（Junction-internal Edge Role）作为身份所有者。每条边必须在来源模块中拥有显式稳定
边键（Explicit Stable Edge Key）`laneEdgeKey`；道路车道成员关系和路口内部边归属只
进入规范关系与语义差异，不进入边的规范标识元组。这样同一条边在加入、移除或调整
横断面 / 路口角色时保持身份，只有显式替换、拆分或合并边并分配新 key 才产生实体级
add/remove。

### 7.3 标识 v1 登记表（Identity v1 Registry）

`identityEncodingVersion = 1` 冻结公共字节 envelope；
`identityRegistryRevision = 1` 冻结本表的 kind、slug 和 required tag sequence。
required tags 必须按数值严格递增编码：

本表是 #291 G1 已接受的 v1 设计，但尚无已发布的 known vector、规范制品或生产
reader；因此本次统一 `LaneEdge` 身份、移除 `ParkingSpace` 对可选停车区域归属的
身份依赖并重排代码 / 标签，是在首次实现冻结前修正 v1 定义，不是对已发布 v1 的
原地兼容修改。首次 implementation G1 必须发布 known vectors；一旦发布，新增 kind
只能提升 registry revision，修改既有 kind 的字段、标签含义或编码必须提升
encoding version。

| 代码（Code） | `entityKind`       | 类别（Category）                              | 英文短名（Slug）    | 必需标签（Required Tags） |
| -----------: | ------------------ | --------------------------------------------- | ------------------- | ------------------------- |
|            1 | `RoadCorridor`     | 声明（Declaration）                           | `corridor`          | `1,2`                     |
|            2 | `RoadSection`      | 声明（Declaration）                           | `section`           | `1,3,33`                  |
|            3 | `AuthoringLane`    | 声明（Declaration）                           | `lane`              | `1,4,32`                  |
|            4 | `LaneEdge`         | 可寻址拓扑实体（Addressable Topology Entity） | `lane-edge`         | `1,5`                     |
|            5 | `Junction`         | 声明（Declaration）                           | `junction`          | `1,6`                     |
|            6 | `Movement`         | 声明（Declaration）                           | `movement`          | `1,8,9,10,34`             |
|            7 | `ManeuverPath`     | 声明（Declaration）                           | `path`              | `1,7,11,12,13`            |
|            8 | `ManeuverGate`     | 声明（Declaration）                           | `gate`              | `1,14,15`                 |
|            9 | `WaitingZone`      | 声明（Declaration）                           | `waiting-zone`      | `1,14,16`                 |
|           10 | `StopLine`         | 声明（Declaration）                           | `stop-line`         | `1,17`                    |
|           11 | `SignalGroup`      | 声明（Declaration）                           | `signal-group`      | `1,18`                    |
|           12 | `SignalController` | 声明（Declaration）                           | `signal-controller` | `1,19`                    |
|           13 | `SignalPhase`      | 声明（Declaration）                           | `signal-phase`      | `1,20,21`                 |
|           14 | `ParkingArea`      | 声明（Declaration）                           | `parking-area`      | `1,22`                    |
|           15 | `ParkingSpace`     | 声明（Declaration）                           | `parking-space`     | `1,24`                    |
|           16 | `LaneGroup`        | 声明（Declaration）                           | `lane-group`        | `1,25,32`                 |
|           17 | `FacilityBand`     | 声明（Declaration）                           | `facility-band`     | `1,26,33`                 |
|           18 | `ParticipantClass` | 声明（Declaration）                           | `participant-class` | `1,27`                    |
|           19 | `AccessRule`       | 声明（Declaration）                           | `access-rule`       | `1,28`                    |
|           20 | `VehicleProfile`   | 声明（Declaration）                           | `vehicle-profile`   | `1,29`                    |
|           21 | `StaticRoute`      | 声明（Declaration）                           | `static-route`      | `1,30`                    |
|           22 | `CanonicalFrame`   | 声明（Declaration）                           | `canonical-frame`   | `1,31`                    |

本表冻结的是 identity v1 已进入当前车辆 projection 的实体集合，不是目标 Traffic
Runtime 永久封闭的参与单元种类表。`VehicleProfile` 与 `StaticRoute` 只服务当前
道路机动车执行域；未来非机动车、步行或轨道执行域若需要不同的运行参数配置
（Runtime Parameter Profile）或通行定义，必须由其 G1 登记新的实体种类/标签
（Entity Kind/Tag）和约束，不得把非车辆参数塞进
`VehicleProfile`，也不得复用 `ParticipantClass` 冒充执行域或行为能力。

所有定义子实体身份的父子关系都使用父实体 `StableId128`，不得把父实体仅在其来源
模块内稳定的裸局部键复制进子实体 tuple。这样跨模块引用、同名父实体和重新归属都由
父实体完整命名空间裁决：

- `RoadSection` 使用 tag 33 `roadCorridorStableId`；
- `AuthoringLane` 使用 tag 32 `roadSectionStableId`；
- `LaneEdge` 不使用 parent anchor，只使用来源模块内显式持久化且唯一的 tag 5
  `laneEdgeKey`；
- `Movement` 使用 tag 34 `junctionStableId`；
- `ManeuverPath`、Signal phase、LaneGroup 和 FacilityBand 继续使用各自登记的
  parent StableId。

`ParkingSpace.areaId` 是可选组织关系，不是停车位身份所有者：字段显式存在时，
canonical LIR 保存有类型的 `ParkingSpace -> ParkingArea` 关系；字段缺省时不生成
该关系。两种情况下 `ParkingSpace` 都只以 authoring namespace 与 tag 24
`parkingSpaceKey` 派生 StableId128，区域归属变化不得造成停车位身份漂移，也不得由
编译器臆造缺失的停车区域。

Movement 的 left/straight/right/u-turn 分类是可重算元数据，不参与标识。
StaticRoute 只表示编译期 authoring route；runtime 注册的 dynamic Route 继续使用
generation-aware handle，不获得持久 StableId128。

`RoadSection` 和 `FacilityBand` 的父锚点来自已验证的唯一所有者关系
（Unique Owner Relation）：恰好一个 `RoadCorridor.elements[]` 分别通过
`sectionId` 或 `bandId` 引用该成员。该关系已经由 `cross-section-access.md` 冻结为
完备所有者树（Complete Owner Tree）；当前态（Current）的 `RoadSectionData` /
`FacilityBandData` 有意不重复保存父实体（Parent）字段。编译器先对全部 `LaneEdge`
建立独立身份，再根据道路走廊声明派生 `RoadCorridor` StableId，并解析
`RoadCorridor.elements[]`，拒绝未知引用、重复引用、多所有者和零所有者；随后：

1. 以已证明唯一的 `roadCorridorStableId` 与 `sectionKey` 派生 `RoadSection`
   StableId；
2. 按 parent-before-child 顺序，以 `roadSectionStableId` 与显式持久化的 `laneKey`
   派生 `AuthoringLane` StableId；
3. 将 `AuthoringLane` 的有序边链解析为对既有 `LaneEdge` StableId / typed ordinal
   的覆盖关系，不重新派生边身份；
4. 以 `roadCorridorStableId` 与 `facilityBandKey` 派生 `FacilityBand` StableId；
5. 从已验证机动路径派生 `Junction -> LaneEdge` 的唯一内部边角色；该角色不创建
   第二个边实体，也不改变既有 `LaneEdge` StableId。

前端可以用嵌套或显式引用表达这些关系，但进入 HIR/MIR 后必须归一为同一关系语义；
validated canonical LIR 必须保存有类型（Typed）的
`RoadSection -> RoadCorridor`、`AuthoringLane -> RoadSection`、
`AuthoringLane -> LaneEdge` 有序覆盖、`Junction -> LaneEdge` 派生内部角色与
`FacilityBand -> RoadCorridor` 关系，不能让发射器（Emitter）、投影器
（Projection）或交通运行时（Traffic Runtime）从输入顺序重新猜测。

每个 frontend 都必须在进入身份闭包前为全部边提供 `laneEdgeKey`：

- Geometry/Synthetic/Editor 来源若展开出逻辑边，必须从显式稳定 authoring key
  产生并持久化边键；几何坐标、曲线离散片段、数组下标和遍历顺序均不得成为边键；
- current JSON 不进入编译器；Traffic v0.10 的 `laneGraph.edges[].id`、
  `RoadSectionData.lanes[].edgeIds` 和派生内部边角色只属于当前加载路径，本设计不定义
  它们到 `laneEdgeKey` 或实体种类的导入映射；
- 后继若为其他真实来源增加导入器，必须在自身 G1 冻结稳定命名空间、显式边键和角色
  关系降阶；不得恢复已取消的 current JSON 映射，也不得按角色选择不同实体种类；
- 缺失边键只能产生待确认建议，未持久化确认前不得发布匿名、按几何或按序号派生的
  边身份。

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
|           5 | `laneEdgeKey`              | ASCII 字节（Bytes）        |
|           6 | `junctionKey`              | ASCII 字节（Bytes）        |
|           7 | `pathKey`                  | ASCII 字节（Bytes）        |
|           8 | `movementKey`              | ASCII 字节（Bytes）        |
|           9 | `directedEntryApproachKey` | ASCII 字节（Bytes）        |
|          10 | `directedExitApproachKey`  | ASCII 字节（Bytes）        |
|          11 | `movementStableId`         | 16 个原始字节（Raw Bytes） |
|          12 | `entryEdgeStableId`        | 16 个原始字节（Raw Bytes） |
|          13 | `exitEdgeStableId`         | 16 个原始字节（Raw Bytes） |
|          14 | `maneuverPathStableId`     | 16 个原始字节（Raw Bytes） |
|          15 | `gateKey`                  | ASCII 字节（Bytes）        |
|          16 | `waitingZoneKey`           | ASCII 字节（Bytes）        |
|          17 | `stopLineKey`              | ASCII 字节（Bytes）        |
|          18 | `signalGroupKey`           | ASCII 字节（Bytes）        |
|          19 | `signalControllerKey`      | ASCII 字节（Bytes）        |
|          20 | `signalControllerStableId` | 16 个原始字节（Raw Bytes） |
|          21 | `phaseKey`                 | ASCII 字节（Bytes）        |
|          22 | `parkingAreaKey`           | ASCII 字节（Bytes）        |
|          24 | `parkingSpaceKey`          | ASCII 字节（Bytes）        |
|          25 | `laneGroupKey`             | ASCII 字节（Bytes）        |
|          26 | `facilityBandKey`          | ASCII 字节（Bytes）        |
|          27 | `participantClassKey`      | ASCII 字节（Bytes）        |
|          28 | `accessRuleKey`            | ASCII 字节（Bytes）        |
|          29 | `vehicleProfileKey`        | ASCII 字节（Bytes）        |
|          30 | `routeKey`                 | ASCII 字节（Bytes）        |
|          31 | `canonicalFrameKey`        | ASCII 字节（Bytes）        |
|          32 | `roadSectionStableId`      | 16 个原始字节（Raw Bytes） |
|          33 | `roadCorridorStableId`     | 16 个原始字节（Raw Bytes） |
|          34 | `junctionStableId`         | 16 个原始字节（Raw Bytes） |

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

compiler 维护 `StableId128 -> CanonicalIdentity + owning source location` 登记。两个 stable
entity 产生同一 tuple 返回 `DuplicateCanonicalIdentity`；相同 digest 对应不同 tuple
返回 `IdentityDigestCollision`。不得追加 ordinal、salt 或 suffix 静默修复。

标识测试（Identity Tests）必须包含：

- 每个 revision-1 kind 至少一个跨平台 known vector；
- missing/duplicate/unknown/out-of-order tag 负向向量；
- sibling reorder、无关 insertion 和 geometry-only edit metamorphic tests；
- section split、boundary/key 和显式 topology closure 变化测试；
- 全部 `LaneEdge`（道路区段已覆盖、路口内部、无所属、自环、孤立）都获得唯一
  `StableId128`；`v0.10-empty-signals-and-parking` 中 `loop`/`isolated` 与引用
  `loop` 的静态路线可以完整进入 `StaticIdentityIndex`；
- 同一 `LaneEdge` 加入 / 移除 RoadSection 覆盖或 Junction internal role 时 ID
  不变；显式替换 / 拆分边并使用新 `laneEdgeKey` 时形成 add/remove；同一 namespace
  重复 `laneEdgeKey` 失败；
- RoadSection/FacilityBand 多所有者 / 零所有者失败、`RoadCorridor.elements[]`
  重排 ID 不变、跨 corridor 移动 ID 改变，以及相同 local key 在不同 corridor 下
  ID 不同；
- 跨 module 的同名 corridor/junction 不产生子实体碰撞，子实体重新归属到另一
  parent StableId 时 ID 改变，而 parent 的 geometry-only edit 不改变子实体 ID；
- compiler 与至少一个独立语言/脚本测试 oracle 的 bytes/ID 一致性；该 oracle 不进入
  production publication 路径。

## 8. 制品与源映射（Artifact and Source Map）

### 8.1 可移植规范制品（Portable Canonical Artifact）

本节保存 Accepted 的长期边界；LFCA/LFSM/LFSD/LFCP 的已接受实现级线格式输入与原子发布
协议集中在 `portable-canonical-artifact.md`。#298 G1 Pass 只冻结后续实现输入，不能用来
推导已经存在的公共格式、生产实现或 G2 开工授权。

平台无关、确定性、closed shape，并包含：

- canonical format、identity encoding/registry、network revision derivation 与
  constraint versions；
- 路网修订标识（Network Revision ID）`NetworkRevisionId`；
- 规范身份表（Canonical Identity Table）`CanonicalIdentityTable`：对每个稳定实体
  保存 `entityKind`、typed ordinal、制品声明的 `StableId128` 与完整有序
  `(fieldTag, fieldValue)` 规范元组前像；该表属于制品语义，不是可裁剪诊断；
- logical entities、typed ordinals、normalized numeric values；
- topology/geometry/static rule relations；
- canonical payload envelope 与 compiler provenance；自身 digest 不嵌入 artifact
  bytes。

它用于 publication、长期审计、migration 和 static image regeneration。它不是 mmap
hot layout，不承诺与 Rust struct ABI 相同，也不因某个
target profile 缺少 Spatial/diagnostic section 而丢失 canonical semantics。

`NetworkRevisionId` v1 是版本化、目标无关且只支持相等性比较的不透明值：

```text
networkRevision =
  SHA-256(
    "laneflow.network-revision.v1\0"
    || canonicalNetworkSemanticPayload
  )
```

规范路网语义载荷（Canonical Network Semantic Payload）
`canonicalNetworkSemanticPayload` 是冻结编码的目标无关规范字节，包含 identity
encoding/registry、完整 `CanonicalIdentityTable`、constraint/execution-constraint
versions，以及全部目标无关的拓扑、几何、静态规则、规范 relation 和静态执行约束；
不包含该摘要自身、artifact envelope、compiler provenance、source
map/diagnostics、publication metadata、target、profile 或 image layout。这样相同
规范语义在不同 compiler provenance 与 target/profile image 中保持同一修订，而任何
运行时可观察静态语义或参与派生的契约版本变化都会产生新修订。Compiler 在 LIR
freeze 后计算该字段；#299 后发射检查从 artifact semantic payload 重算并逐字节
比较，不能信任 artifact 内自报的值，但不重新验证 StableId 派生或完整静态语义。
`canonicalArtifactDigest` 仍认证
完整 artifact exact bytes，外部 descriptor 以 `canonicalArtifactByteLength` 认证
同一字节序列的精确长度；二者不得互相替代，也不得在长度上限预检前读取或 hash
不可信 artifact。
#299 不维护跨发布的 `NetworkRevisionId` 碰撞登记，也不因后发射检查通过而授权切换。
若 #300/#302 需要在镜像或切换信任边界中比较历史 payload，应在各自 G1 重新冻结其
登记、失败关闭与恢复策略，不得把该职责反向扩入 #299。

### 8.2 目标静态镜像（Target Static Image）

按 `targetTriple + staticImageLayoutVersion + staticImageProfileId + partitionHintVersion`
生成。`partitionHintVersion` 是第四根镜像变体选择轴，因为
提示节属于必选字节、但不进入目标无关的路网修订语义：

```text
StaticNetworkImage
  Header
  SectionDirectory
  Required: StaticTrafficImage
    hot SoA tables
    CSR adjacency / flat ranges
    precompiled route/path/gate/waiting occurrences
    static execution constraint graph
  Required: StaticIdentityIndex
    stable-entity ordinal -> StableId128
    sorted (entity kind, StableId128) -> typed ordinal
  Required: PartitionPlanningHints
    rebuildable cost / boundary / recommended-cut hints
  Optional: StaticSpatialImage
    frame/edge-aligned geometry tables
    flat points / cumulative arc / sampling ranges
```

v1 profile 是版本化 closed set，不允许调用方任意拼 feature bits：

| `staticImageProfileId` | 必需节（Required Sections）                                                                 | 用途                                              |
| ---------------------- | ------------------------------------------------------------------------------------------- | ------------------------------------------------- |
| `traffic-headless-v1`  | `StaticTrafficImage`, `StaticIdentityIndex`, `PartitionPlanningHints`                       | 服务器（Server）、测试、无图形宿主                |
| `traffic-spatial-v1`   | `StaticTrafficImage`, `StaticIdentityIndex`, `PartitionPlanningHints`, `StaticSpatialImage` | 引擎适配器（Adapter）、规范位姿（Canonical Pose） |

设计约束：

- `StaticTrafficImage` 可独立验证和挂载；headless profile 不携带 geometry；
- `StaticIdentityIndex` 是所有生产配置档必需、但不进入 tick 的共享冷索引：对每个稳定
  declaration/addressable-derived 提供 typed ordinal → StableId128 的正向表，以及按
  `(entityKind, StableId128)` 排序的 StableId128 → typed ordinal 反向表；它服务
  snapshot save/load、dynamic Route 重建和 network-revision cutover；
- portable artifact 的 `CanonicalIdentityTable` 是独立验证所需前像权威；
  static image 只保留上述 `StaticIdentityIndex`，不复制 field-tag/value 前像，生产
  Runtime 因而不为验证元数据承担 retained memory 或 cache 成本；
- 身份索引可以独立分页、按需映射或在 steady tick 期间不驻留 CPU cache，但不得与
  其他冷数据一起从 production profile 裁掉；
- `sectionMask` 必须与 profile 的 closed section set 精确匹配；缺失、额外或未知
  section 均 fail closed，调用方不能通过 feature bits 组合新 profile；
- Spatial section 存在时必须完整覆盖 v1 所需 edge，并与 Traffic 共享 canonical edge
  ordinal；v1 不引入 sparse Spatial mapping；
- shared immutable bytes，多 `TrafficWorld`/Spatial session 复用；
- 静态执行约束对工作线程数中立；v1 配置档保留提示节以让镜像字节闭合确定，
  运行时可以忽略或重新派生提示，但不得保存最终分配；
- hot/warm/cold 分段，profile 可以裁剪 Spatial，但不能裁剪恢复和切换所需的
  `StaticIdentityIndex`；v1 不定义泛型 `WarmQueryTables`、`ColdDiagnostics` 或
  `traffic-debug-v1`，低频查询若是运行时必需能力必须进入对应 required typed section，
  显示名、规范身份前像、来源位置和诊断文本继续由 portable artifact、source map 与
  diagnostics artifact 外置提供；
- 顶层 section byte offset/length 使用 checked `u64`；table row ordinal、count 和
  hot relation range 使用 checked `u32`，不保存原生指针；
- verifier 完成后 view 的高频索引是 O(1) 或连续 range traversal；
- schema/Serde/object graph 不是 static image ABI；
- 具体 archive/zero-copy library 由安全审计和 benchmark 决定。

生产完整性验证不要求每次启动串行读取完整镜像。镜像外的静态镜像完整性清单
（Static Image Integrity Manifest）`StaticImageIntegrityManifest` 采用版本化、
确定性的平面分块表（Flat Chunk Table）：

```text
staticImageIntegritySchemeVersion
staticImageDigest
staticImageByteLength
integrityChunkSize
integrityChunkCount
chunks[] = (chunkOrdinal, byteOffset, byteLength, sha256)
sectionChunkRanges[]
```

v1 以 SHA-256 认证每个分块（Chunk）；分块按镜像精确字节的连续偏移量顺序完整
覆盖，不存在缺口、重叠或零长度。镜像头、节目录与每个必需节的分块覆盖必须闭合。
`integrityChunkSize` 不是发布者可任意选择的性能旋钮：v1 由已认证的
`constraintSetVersion + staticImageProfileId + staticImageIntegritySchemeVersion`
唯一确定，并由独立镜像重建器复核；具体大小由实现 G1 在最低产品硬件上基准后写入
对应约束集，不改变镜像布局版本。清单自身继续受固定调用方上限约束，不能用极小分块
制造无界清单。
清单不嵌入自己的摘要；`StaticImageDescriptor` 绑定
`staticImageIntegrityManifestDigest + staticImageIntegrityManifestByteLength`。
发布者（Publisher）与独立镜像重建器（Independent Image Builder）必须证明清单的
有序分块、全镜像 `staticImageDigest + staticImageByteLength` 与同一最终镜像精确
字节一致。

v1 选择平面分块表而不是默克尔树（Merkle Tree）：外部信任锚会在打开大镜像前认证
整份有固定上限的小型清单，当前产品路径不需要从不可信远端逐页取得证明；平面表已经
支持节级随机验证与分块并行摘要，同时减少树节点、证明路径和校验器攻击面。若未来
产品要求不下载完整清单的远端认证分页，应提升
`staticImageIntegritySchemeVersion` 并定义新方案，不能原地改变 v1 摘要构造。

生产加载先认证描述符与清单，再验证镜像头/节目录及构造目标有类型视图所需的全部
分块，随后执行这些节的有界结构/交叉索引验证。`TrafficWorld` 构造前必须完成
Traffic、identity 与 partition-hint 必需节；Spatial session 构造前再完成
`StaticSpatialImage` 的分块与 Traffic shared-edge 验证。未验证分块不得暴露给有
类型视图；分块验证不进入 fixed-tick 或 pose 热路径。宿主可以并行预先验证
（Eager Verification）全部分块，也可以让未请求的 Spatial/冷页采用后台验证
（Background Verification）
或延迟验证（Lazy Verification），但不能把“描述符已认证”
误写成“任意镜像字节已可信”。完整镜像 SHA-256 继续服务发布身份、独立重建和显式
完整审计（Full Audit），不再是每次生产启动建立 Traffic view 的强制串行步骤。

相同 portable artifact 的不同 target/profile/partition-hint variant 共享
`canonicalArtifactDigest` 与 `NetworkRevisionId`，但各自拥有独立的
`staticImageDigest + staticImageByteLength` 绑定；若两个变体的 exact bytes
偶然相同，其摘要也可以相同，不能把“变体不同”错误提升为“摘要必然不同”。
Image header 必须声明 `networkRevisionDerivationVersion` 与
`networkRevision`，但这些字段和 header 中的其他 provenance 一样只供外部描述符
核对，不能自证可信。Traffic Runtime 的
构造入口只接收从 `TrustedStaticImage` 拆出的 `StaticTrafficView`，不能要求调用方
同时提供 Spatial section，也不能接受仅结构验证所得的 view。

### 8.3 外部镜像描述符（External Image Descriptor）

> Accepted ADR 0024 已取消由 #299 统一交付的 `static-image-v1` receipt。下文镜像
> digest/length、profile、target、完整性清单和对象外 trust-anchor 原则继续有效；
> `validatorBuildId`、`validationReceipt*` 和 receipt envelope 字段只保留为 #291
> 历史候选，#300 必须在自身 G1 决定当前描述符，不得直接实现这些旧字段。

生产 fast path 的 trust anchor 必须位于 image bytes 之外。版本化
`StaticImageDescriptor` 至少绑定：

```text
staticImageDescriptorVersion
networkRevisionDerivationVersion
networkRevision
canonicalArtifactDigest
canonicalArtifactByteLength
staticImageDigest
staticImageByteLength
staticImageIntegritySchemeVersion
staticImageIntegrityManifestDigest
staticImageIntegrityManifestByteLength
staticImageLayoutVersion
staticImageProfileId
sectionMask
targetTriple
constraintSetVersion
executionConstraintVersion
partitionHintVersion
identityEncodingVersion
identityRegistryRevision
compilerBuildId
validatorBuildId
validationReceiptFormatVersion
validationReceiptKind
validationReceiptDigest
validationReceiptByteLength
```

`staticImageByteLength` 是 descriptor 所认证、参与 `staticImageDigest` 的原始未压缩
image bytes 的精确 `u64` 长度，不是 transport/container 的压缩长度。Descriptor
签发者必须从最终 exact bytes 记录 digest + length；validation receipt 和 independent
image rebuild comparison 同时绑定二者。若分发层使用压缩，压缩输入另受宿主上限，
解压器的输出上限必须预设为 `staticImageByteLength`，不得先无界解压再核对。
`canonicalArtifactByteLength` 与 `validationReceiptByteLength` 分别认证该
descriptor 所引用的 portable artifact 和 validation receipt exact bytes；需要读取
这些对象的 validator、publisher 或审计消费者必须在任何线性工作前执行同一套调用方
上限、地址空间、O(1) 已知长度 / checked length+1 unknown-stream 预检。Runtime 若
不读取它们，可以只比较已认证绑定，不得据此省略发布和验证路径的长度契约。
`staticImageIntegrityManifestDigest + staticImageIntegrityManifestByteLength`
认证 §8.2 的完整性清单 exact bytes；加载器必须先认证 descriptor，再以调用方
`maxStaticImageIntegrityManifestBytes` 对清单执行同样的 pre-hash 长度预检。清单
中的 image digest、长度、scheme version 与 section coverage 必须与 descriptor 和
image header 的待核对声明精确相等，任何不一致都不能建立 section trust。

descriptor 可以由签名 publication manifest、宿主已认证 asset/package manifest 或
应用内 pinned digest 提供。Image header 内的
`networkRevision`/`canonicalArtifactDigest`/target/provenance 只是待核对声明；
攻击者可以伪造它们并对任意 image bytes 计算新的 `staticImageDigest`，因此不能
独立建立 semantic trust。

验证收据不是由 `validatorBuildId` 隐式选择布局的裸记录。每份收据必须使用版本化的
验证收据封套（Validation Receipt Envelope）`ValidationReceiptEnvelope`：

```text
validationReceiptFormatVersion
validationReceiptKind
validatorBuildId
subjectBindings
checkResults
```

外部描述符必须在读取收据前认证同一
`validationReceiptFormatVersion + validationReceiptKind +
validationReceiptDigest + validationReceiptByteLength`；封套不得内嵌自己的摘要。
`validationReceiptKind` 是封闭枚举，v1 只允许：

- `canonical-publication-v1`：绑定 artifact、路网修订和 source map，要求独立语义
  验证、修订重算与 source-map 闭合检查；
- `static-image-v1`：绑定 artifact、路网修订、image 与完整性清单，要求下述三项
  独立成功证据；
- `revision-cutover-v1`：绑定 base/target artifact、路网修订、可信静态镜像描述符
  subject、语义差异和迁移策略，要求两侧制品/修订验证及完整语义差异重算或验证。

未知格式版本、未知种类、描述符与封套版本/种类不一致，或使用其他种类收据替代当前
描述符要求的种类时，一律在解析 subject bindings 前失败关闭。

`StaticImageDescriptor` 的 `validationReceiptKind` 必须是 `static-image-v1`；
该可信描述符的签发前置条件是三项独立成功证据：

1. portable artifact 通过独立语义验证，包括全部稳定身份重算；
2. `networkRevisionDerivationVersion + networkRevision` 从规范语义载荷独立重算并
   与制品声明相等；
3. 不复用 compiler emitter 的独立镜像重建器在相同
   target/layout/profile/partition-hint variant 下产生相同 exact bytes digest +
   length。

`static-image-v1` 收据必须逐项记录三者，并绑定
`canonicalArtifactDigest + canonicalArtifactByteLength`、
`networkRevisionDerivationVersion + networkRevision` 与
`staticImageDigest + staticImageByteLength`，同时记录
`staticImageIntegritySchemeVersion +
staticImageIntegrityManifestDigest + staticImageIntegrityManifestByteLength` 已与
同一 independent rebuild exact bytes 闭合。
`TrustedStaticImage` 只从已认证 descriptor 获得修订标识；调用方参数、image header
或未验证 artifact 均不能另行指定运行时当前修订。

### 8.4 源映射与诊断（Source Map and Diagnostics）

> LFSM wire、来源位置和 artifact binding 继续有效；本节旧
> `canonical-publication-v1` receipt/descriptor 字段已由 LFCP v2 取代。当前规范发布
> binding 见
> `compiler-post-emission-check-and-minimal-publication-closure.md` `8。

源映射不是一组可以脱离编译批次复用的裸记录。每份源映射都必须使用版本化的
源映射封套（Source Map Envelope）`SourceMapEnvelope`：

```text
sourceMapFormatVersion
networkRevisionDerivationVersion
networkRevision
canonicalArtifactDigest
compilerBuildId
records
```

`canonicalArtifactDigest` 绑定本次编译产生的 portable artifact exact bytes，
`networkRevisionDerivationVersion + networkRevision` 绑定其规范语义，
envelope 内的 records 绑定权威来源模块图、frontend/import 工具与显式编译选项的
来源沿袭。即使实体 `StableId128` 与 typed ordinal 都未变化，来源位置、来源模块
或编译输入变化也必须生成新的 `SourceMapEnvelope` exact bytes。

源映射记录在上述 envelope 内按标识类别选择 key：

- 拥有声明（owning declaration）和贡献来源位置（contributing locations）；
- declaration/addressable-derived 使用
  `(entityKind, StableId128, typed ordinal)` 引用 portable artifact 中的
  `CanonicalIdentityTable` row，并附加闭合 `SourceLocation` / provenance；可以为诊断展示
  冗余规范元组，但该副本不是 validator 输入或身份权威；
- owner-local relation/occurrence 使用 owning StableId128、typed role 和本次
  compilation 的 `localIndex`；
- 所有记录保留 LIR table/ordinal 作为本次 compilation 的定位；
- frontend/module/import provenance；
- compiler pass/constraint version；
- generated relation 的推导链。

源映射自己的摘要不得嵌回自身 bytes。规范发布描述符（Canonical Publication
Descriptor）`CanonicalPublicationDescriptor` 必须位于 artifact/source-map bytes
之外并至少绑定：

```text
canonicalPublicationDescriptorVersion
networkRevisionDerivationVersion
networkRevision
canonicalArtifactDigest
canonicalArtifactByteLength
sourceMapFormatVersion
sourceMapDigest
sourceMapByteLength
compilerBuildId
validatorBuildId
validationReceiptFormatVersion
validationReceiptKind
validationReceiptDigest
validationReceiptByteLength
```

该描述符的 `validationReceiptKind` 必须是 `canonical-publication-v1`。
`sourceMapDigest` 是完整 `SourceMapEnvelope` exact bytes 的 SHA-256，
`sourceMapByteLength` 是同一字节序列的精确 `u64` 长度。签发者必须从最终 exact
bytes 记录 digest + length；canonical publication 不允许用空摘要、空记录或缺失字段
伪装已绑定 source map。Independent validator 必须检查 envelope 版本、所有 row /
ordinal / owner-local key 对已验证 artifact 闭合，并让 validation receipt 同时绑定
artifact/revision 与 artifact/source-map digest + length。Descriptor 也必须从最终
validation receipt exact bytes 记录其 digest + length，不能先读取无界 receipt 再
核对摘要。
Descriptor 的真实性必须来自签名 publication manifest、受信任 CI provenance 或
应用内 pinned digest，不能由 source-map envelope 自报。
消费者必须先认证有固定小上限的 descriptor，再按与 8.3 相同的 O(1) 已知长度或
checked length+1 bounded-reader 规则核对源映射长度和摘要，解析 exact-current
envelope，并要求 descriptor、envelope 与已验证 portable artifact 的
`networkRevisionDerivationVersion + networkRevision + canonicalArtifactDigest +
compilerBuildId` 全部精确相等；来源沿袭记录作为 envelope exact bytes 的一部分由
`sourceMapDigest` 认证。任何版本、长度、摘要、artifact、revision 或 provenance
不匹配都以 `SourceMapArtifactMismatch` 失败关闭；记录级三元组 key 只能在完成该
配对后定位行，不能单独证明 source map 属于某份 artifact。

诊断对标 rustc：稳定 code、severity、primary/secondary source location、原因和可执行
建议；文本来源可把 location 渲染为 span。authoring error 指向 source/画布；artifact
corruption/version mismatch 面向运维，不
返回 generated JSON 行号。

### 8.5 语义差异（Semantic Diff）

语义差异不是可以脱离旧/新规范制品解释的裸记录。每份语义差异必须使用版本化的
语义差异封套（Semantic Diff Envelope）`SemanticDiffEnvelope`：

```text
semanticDiffFormatVersion
baseNetworkRevisionDerivationVersion
baseNetworkRevision
baseCanonicalArtifactDigest
baseCanonicalArtifactByteLength
targetNetworkRevisionDerivationVersion
targetNetworkRevision
targetCanonicalArtifactDigest
targetCanonicalArtifactByteLength
records
```

PR 审阅不依赖二进制 diff。封套内 Stable entity 按 StableId128 报告；owner-local derived
record 在 owning StableId128 + typed role 内按 canonical relation value 做确定性
sequence diff，报告 before/after `localIndex`，但不把位置当成跨编译标识：

- entity add/remove/rename-display；
- topology reconnect、owner/member 变化；
- geometry/length/tolerance-significant change；
- Gate/Waiting/signal/access behavior change；
- 标识闭包变化及原因；
- target/profile image layout-only change（不得伪装成 semantic change）。

封套必须绑定
`baseNetworkRevisionDerivationVersion + baseNetworkRevision +
baseCanonicalArtifactDigest + baseCanonicalArtifactByteLength` 与
`targetNetworkRevisionDerivationVersion + targetNetworkRevision +
targetCanonicalArtifactDigest + targetCanonicalArtifactByteLength`。Base 必须是已
通过 independent validator 的 portable artifact；无 baseline 时使用显式 genesis
marker，并把全部 stable entity
和 owner-local sequence 报告为新增。重复 relation value 的序列对齐按最低
before/after `localIndex` 确定性破同值，diff 本身不获得 authoring authority。

PR/治理消费可以直接展示经过验证的 semantic diff；运行时镜像切换不得把 compiler
输出的裸 diff 当作迁移权威。所有跨修订状态迁移都要求 independent validator 对
base/target portable artifact 独立重算或验证其完整语义，并由 §9.6 的外部可信切换
描述符绑定 exact diff bytes digest。缺少该绑定时，diff 只能作为诊断提示，任何
跨修订状态迁移都必须失败关闭；两个可信 `StaticIdentityIndex` 只能复核稳定身份与
密集序号的对应关系，不能证明相同 StableId128 的拓扑、几何、访问规则或其他静态
语义仍与旧状态兼容。新修订可以作为无旧状态的新世界启动，但不得把它伪装成迁移。

本地玩家改路也不构成例外：独立验证器必须比较两份已验证 portable artifact，重算或
验证完整 semantic diff 并签发 validation receipt；宿主随后把 exact diff bytes、
base/target artifact/image/revision、迁移策略和 receipt 绑定为
`NetworkRevisionCutoverDescriptor`，并通过宿主认证 asset chain 或 pinned digest
建立 Runtime 外部的信任锚。没有该证据链时，旧世界继续运行。

## 9. 静态/可变状态和运行时消费

### 9.1 交通运行时（Traffic Runtime）

Target 把当前 `LaneFlow Core` / `laneflow-core` 重命名为
`LaneFlow Traffic Runtime` / `laneflow-runtime`。这不是机械改名：static contract、
format、image layout 和 compiler type 必须移出 runtime crate。当前 production 的
`laneflow-core`/`CoreWorld` 名称在 cutover 前继续有效；target public world 名称为
`TrafficWorld`。

`TrafficWorld` 借用或共享 `StaticTrafficView`，只分配：

- 已实现执行域的交通参与单元与动态通行定义代际（Dynamic Traversal
  Generations）；
- controller clocks/indications；
- grant/reservation/waiting/parking occupancy；
- command/event/snapshot buffers；
- dynamic Route occurrence metadata；
- world identity、输入命令游标、从 `TrustedStaticImage` 派生的当前路网修订绑定；
- 每世界运行时执行计划、边界交换缓冲与调度统计。

当前首个 projection 仍使用 vehicles、dynamic Route 与 parking occupancy；这些
current specialization 不得反向冻结终态 Traffic Runtime 的参与单元或执行域模型。

人口、Routing 和游戏规则的 seed/随机流属于 caller/出行编排层，不进入
`TrafficWorld` 隐藏状态。只有后续 G1 显式授予的 Runtime 自有随机流才成为每世界
状态与运行时快照内容。

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

1. `UnverifiedImageBytes`：任意调用方提供的字节；
2. `StructurallyVerifiedImage`：目标节已通过有界结构校验器，只证明对应视图可安全构造
   且全部运行时前置条件成立；它不证明字节属于受信任发布；
3. `TrustedStaticImage`：在目标节的结构验证之外，已认证描述符与完整性清单、逐分块
   核对目标节摘要，并匹配
   `canonicalArtifactDigest + canonicalArtifactByteLength`、目标/配置档/约束与验证
   收据；它只能暴露已经完成分块与结构验证的有类型节视图，不能把未验证节提升为可信。

Production `TrafficWorld`/Spatial 只从 `TrustedStaticImage` 拆出 view。允许三条
显式路径：

- **published trusted**：认证 descriptor 与完整性清单 -> image/profile/target
  比对 -> 目标节分块验证 -> 结构校验器 -> 节范围可信
  view；实际固定顺序是先认证两个有界小对象，再执行下述 byte-length preflight，
  之后才读取 / 解压 / 分配或 hash 对应 image bytes；
- **local validated build**：portable artifact 先通过 independent validator，再由
  compiler builder 与 independent image builder 生成同 target/layout/profile
  image；digest + exact length 相等后生成与本次构建绑定的 receipt，或直接采用 independent
  builder 的已验证输出；
- **untrusted external**：必须提供 portable artifact，独立验证后本地重建；只有
  image bytes 时拒绝，不能直接进入 fast path。

所有 image 路径在任何与输入长度成正比的工作前都执行失败关闭的镜像长度预检
（Image-length Preflight）：

1. 先认证并解析有固定小上限的 `StaticImageDescriptor`，再按 descriptor 绑定的
   exact length 和 `maxStaticImageIntegrityManifestBytes` 有界读取并认证完整性清单；
   检查
   `staticImageByteLength` 非零、可转换为当前地址空间长度，且不超过 caller /
   process 的 `maxStaticImageBytes`；任一失败时不得打开大对象、分配、解压或 hash；
2. 对 buffer、mmap 或已打开 asset blob，使用 O(1) 长度与 descriptor exact length
   比较；对 filesystem/asset handle 必须 hash 同一已打开对象，不能用路径 metadata
   预检后再重新打开另一个对象；
3. 对无法 O(1) 获得长度的 stream，只允许 bounded reader 最多消费
   checked `staticImageByteLength + 1` bytes；必须恰好读到声明长度并确认无追加
   字节；分块校验器只覆盖清单声明的精确区间，不能先收完整流再判断；
4. transport compression 同时受压缩输入上限和解压输出 exact-length 上限；truncated、
   appended、oversized 或 decompression-overrun 均在构造 image view 前拒绝；
5. 预检后才核对镜像头/配置档/目标，验证构造目标视图所需的全部分块，
   并执行有界结构校验器；全镜像 `staticImageDigest` 可以在
   发布、独立重建、显式完整审计或宿主预先验证（Eager Verification）策略中复核，但
   production section view 不要求每次启动先串行 hash 未消费 section。Verifier 内的
   section/entity/point limits 仍保留为后续防线，不能替代 pre-hash byte bound。

`TrustedStaticImage` 是绑定整份 descriptor 的能力对象，不代表其所有 section 已被
读取。实现必须维护不可伪造的 section verification state；请求新的 Spatial view 时
先完成该节的分块与交叉索引验证。尚未认证的页不能通过内存映射、裸切片、
调试接口或后台任务泄漏给 Runtime/Spatial。完整性清单查找与分块摘要只出现在
加载/首次挂载边界，不进入 fixed-tick、位姿采样或逐参与单元访问路径。
分块验证与有类型视图还必须绑定同一个不可变字节背板（Immutable Byte Backing）。
若文件系统、Mod 包或资产 API 不能保证已打开对象在视图生命周期内不可替换且不可
原地改写，加载器必须把已验证分块复制并封存到只读拥有存储，或拒绝建立可信视图；
验证路径元数据后重新打开、验证后映射另一对象或允许可写别名均不构成信任。

### 9.4 有界结构校验器（Bounded Structural Verifier）

verifier 不重新执行 authoring topology、identity derivation、coverage 求解或
geometry tessellation，但必须检查 runtime 直接依赖的全部 precondition：

- magic/header/layout/target/profile/section compatibility；
- `u64` section offset/length、alignment、section bounds、地址空间转换和 checked
  arithmetic；
- table cardinality、CSR monotonicity、range、owner/member 与 cross-index bounds；
- finite/positive numeric values、speed/length domain；
- cumulative arc monotonicity、point/frame range 和 sampling bounds；
- Traffic、identity 与 partition-hint 必需节，profile 含 Spatial 时的 v1 complete
  coverage 和 shared edge ordinal；
- caller policy 限制的 image bytes、section/entity/point count；
- per-world mutable allocation plan、capacity multiplication 和地址空间上限。

任何 limit、digest、profile 或 structural invariant 失败都在分配/挂载前 fail
closed。签名或宿主 asset authenticity 属于 external descriptor 的来源认证，不是
image header 的可选自证字段。

### 9.5 并行就绪执行规划（Parallel-ready Execution Planning）

静态执行约束图表达冲突、等待、下游存储、控制器等共享资源的依赖组件、安全切分
（Safe Cut）、规范提案/资源声明/事件合并键和提交顺序。它是 LIR 派生的行为约束，
对工作线程数量和具体分区计划中立。

分区规划提示只保存预估成本、边界权重或推荐 cut 等性能信息；运行时可以忽略或重建。
每个 `TrafficWorld` 依据静态约束、硬件与动态负载构造自己的运行时执行计划，实际
分区/工作线程/边界缓冲与迁移状态永不回写 image。

精确路径按以下状态流执行：

```text
已提交状态（Committed State）T
  -> parallel read/evaluate
  -> 提案（Proposals）与资源声明（Resource Claims）
  -> canonical component-local reduction
  -> 原子提交（Atomic Commit）T + delta
```

所有分区读取同一 `T`。跨边界不能增加一 tick 延迟；每个连接资源组件只有一个规范
归约权威，但互不相交组件可以并行归约。工作线程数、任务完成顺序和分区计划不得
改变已提交状态、事件或安全结果。本设计不冻结固定边界邻域、分区算法、任务运行库、
固定点或精确累加器；这些选择必须由热点、误差和基准证据裁决。

“唯一规范归约权威（Canonical Reduction Authority）”定义唯一结果与规范顺序，
不等同于“一个组件只能由一个物理线程串行执行”。实现可以依据资源依赖图的强连通
分量（Strongly Connected Component，SCC）、凝聚有向无环图（Condensation Directed Acyclic Graph，Condensation DAG）、
资源分段和无依赖波次，把同一连接组件拆成确定性并行任务；各段按稳定键局部归约，
再以固定合并树或等价的规范算法合成同一结果。不可进一步分解的 SCC 可以采用内部
并行算法，但必须继续满足 worker/partition 置换等价，不得通过额外 tick 延迟或改变
冲突语义换取吞吐。当前集中式组件合并只保留为精确参考预言机（Exact
Reference Oracle），不能未经工作量/跨度证据直接成为 production 架构。

运行时必须区分归约工作量（Reduction Work）与归约跨度（Reduction Span），至少
报告提案/声明数、连接组件数、SCC 数、最大组件与最大 SCC 的提案占比、凝聚 DAG
临界路径深度、稳定合并层数、归约工作量/跨度比和各阶段屏障等待。令当前 tick 的提案
数为 `P_T`、资源声明数为 `C_T`、被触及资源/SCC 数为 `A_T`、被访问依赖弧数为
`E_T`；production fast path 的目标工作量是
`O(P_T + C_T + A_T + E_T)`，临时内存是 `O(P_T + C_T + A_T)`，不得每 tick 扫描
完整世界、完整静态组件或全部未激活资源。实现应以有类型资源序号预分桶、固定宽度
稳定键的确定性基数/桶排序和复用工作线程缓冲，只遍历当前固定步进的活跃依赖前沿；
静态依赖部分可以预编译结构上界，前车链等动态依赖则必须按活跃有类型序号增量组装，
不能错误套用静态 SCC。全局比较排序可以保留在精确参考预言机，不是生产默认。若某一
行为域确实需要完整组件状态，必须单独记录该项工作量和跨度，不能隐入常数。

信号协调、冲突区链、排队溢流与长前车链能否形成城市核心区巨型组件必须由中国特色
城市工作负载实证，而不是预设一定可分或一定不可分。#220 负责生产分区/归约设计与
基准，#72 保留研究证据根；若单大型世界的有效并行度受上述跨度限制，必须回到 G1
修订执行约束，不能只增加工作线程数。

提案、声明、归约与提交是逻辑阶段，不要求在单 worker 路径物化通用队列、任务图或
真实 barrier。允许提供融合单工作线程执行器（Fused Single-worker Executor）：
它在同一遍历中生成并立即消费可规范确定的局部记录，但必须产生与显式参考路径完全
相同的提交状态、事件、错误和规范顺序。确定性契约约束可观察语义，不强制为没有并发
竞争的执行器支付无意义的调度脚手架。

### 9.6 不可变路网修订与镜像切换

> Accepted ADR 0024 已取消 #299 交付 `revision-cutover-v1` receipt 的前提。下文 base/target、
> LFSD、镜像、迁移策略、对象外认证和失败原子性要求仍是 #302 输入；具体 descriptor
> 和验证形态必须由 #302 G1 重新冻结。

目标静态镜像代表一个路网修订。结构性道路编辑重新进入权威来源模块图、增量编译、
独立验证和外部信任绑定，生成新修订；共享镜像不得原地 mutation。

运行世界只在显式 fixed-tick 安全边界执行失败关闭的镜像切换事务。未变化实体通过
旧/新可信镜像的 `StaticIdentityIndex` 重建 StableId128 ↔ typed ordinal 映射；
删除、重接或语义改变的网络元素必须按受信任语义差异迁移或终止其交通参与单元、
动态通行定义、停驻/预约和控制器状态。当前道路机动车执行域仍具体表现为车辆、
动态路线和停车。稳定 ID 保持不变不表示语义未变化；任一完整性或语义兼容条件无法
证明时，旧修订继续生效。临时封闭等不改变静态身份/拓扑的状态由后续 G1 冻结的
runtime overlay/command 承担。

任何保留旧世界状态的跨修订切换都必须消费受信任语义差异（Trusted Semantic
Diff）。v1 的证据链要求 image 外部的版本化
`NetworkRevisionCutoverDescriptor` 至少绑定：

```text
networkRevisionCutoverDescriptorVersion
baseNetworkRevisionDerivationVersion
baseNetworkRevision
baseCanonicalArtifactDigest
baseCanonicalArtifactByteLength
baseStaticImageDigest
baseStaticImageByteLength
targetNetworkRevisionDerivationVersion
targetNetworkRevision
targetCanonicalArtifactDigest
targetCanonicalArtifactByteLength
targetStaticImageDigest
targetStaticImageByteLength
semanticDiffFormatVersion
semanticDiffDigest
semanticDiffByteLength
migrationPolicyVersion
validatorBuildId
validationReceiptFormatVersion
validationReceiptKind
validationReceiptDigest
validationReceiptByteLength
```

该描述符必须来自签名 publication manifest、宿主认证资产清单或 pinned digest；
`validationReceiptKind` 必须是 `revision-cutover-v1`。Validation receipt 必须证明
independent validator 已针对两个 portable artifact
重算并验证各自路网修订标识，且已按 `semanticDiffFormatVersion` 解析、验证或重算
完整 `SemanticDiffEnvelope`。描述符中的 diff format version/digest/length 必须与
封套 exact bytes 精确一致；base/target 修订、制品摘要/长度和镜像摘要/长度必须分别
与封套及两个可信静态镜像的 descriptor 精确相等。
Runtime 仍须用两个 `StaticIdentityIndex` 核验每个稳定实体映射，不能让 diff 中的
ordinal、数组位置或 compiler 私有顺序成为迁移权威；该索引检查是语义差异验证后的
身份完整性防线，不能证明语义兼容。缺失、未认证、base/target
不匹配或未由独立验证器完整比较的 semantic diff 一律中止迁移。调用方可以显式放弃
旧状态并创建目标修订上的新世界，但这属于新建而非 cutover/migration。

准备切换时，Runtime/宿主先认证 cutover descriptor，再以
`semanticDiffByteLength` 和调用方 `maxSemanticDiffBytes` 在解析、分配或 hash 前
执行 O(1) exact-length / checked length+1 bounded-reader 预检；压缩传输同时限制
压缩输入和解压输出。Base/target artifact 与 validation receipt 的审计/验证读取同样
使用描述符绑定的 exact byte length 和相应调用方上限。任何 truncated、appended、
oversized、length mismatch 或 digest mismatch 都在开始迁移事务前失败关闭。

默认在线切换采用准备（Prepare）→增量追赶（Delta Catch-up）→静默提交
（Quiescent Commit）→回收（Retire）：

1. **准备**：旧世界继续固定步进。Runtime 在基准提交边界 `B` 捕获只读已提交状态，
   并在后台完成新镜像信任/结构、容量、稳定身份映射和结构性迁移；候选世界只是从
   `T[B]` 派生的迁移副本，不得独立模拟目标修订上的未来固定步进、发出事件或接收
   新的游戏命令；迁移策略要求的终止/重映射等切换事件只能形成未提交候选批次；
2. **增量追赶**：从 `B` 起，旧世界每次原子提交同时向有界迁移增量日志（Migration
   Delta Journal）追加规范的已提交动态状态变更、生命周期变化以及命令/事件游标；
   控制器、预约、占用、动态通行定义与参与单元引用都必须闭合。候选只按受信任迁移
   策略重解释这条已提交变更流（Committed Mutation Stream），不重新执行输入命令，
   也不发布第二份行为结果或重复旧世界事件。后台按规范顺序追赶，直到落后量进入
   提交预算。
   日志容量、最大追赶轮次、后台 CPU/内存预算和对正常固定步进的干扰必须由调用方
   策略限制；无法追上、日志溢出、迁移规则失败或目标失效时放弃候选，旧世界不受
   影响；
3. **静默提交**：在 `C` 的固定步进安全边界、下一固定步进读取输入之前短暂静默旧
   世界，冻结命令游标并排空到最新 `T[C]` 的日志尾；Runtime 必须证明
   `(candidateState[C], cutoverEvents[C]) = M(targetRevision, T_old[C], migrationPolicy)`
   成立，其中 `M` 是受切换描述符绑定的
   确定性迁移函数，`cutoverEvents[C]` 是按规范键排序且尚未对外可见的切换事件批次。
   随后复核修订、身份、全部动态引用、输入/事件游标和候选状态摘要，再把镜像/状态
   绑定与该事件批次作为同一原子提交只发布一次；新修订从 `C` 后的下一固定步进才开始
   解释新输入。提交窗口不得重做全量迁移，停顿只包含有界尾部追赶、最终验证和原子
   发布；
4. **回收**：旧镜像/状态在全部借用快照、姿态批次、适配器 token/epoch 退出后回收。
   提交前任一失败保持旧绑定且不得发布任何切换事件；原子发布后不得回滚到会重复
   输入或事件的旧时间线。

宿主可以显式选择维护暂停模式（Paused Maintenance Mode），在世界已经由上层暂停且
玩家可感知该状态时一次性完成迁移；它不是生产在线编辑的隐式默认值。临时封闭等
运行时覆盖层不应为避免该协议而伪装成静态修订。后继实现 G1 必须冻结日志
记录模型、迁移策略和提交预算，但不得重新开放“准备期时间是否推进”这一时钟语义。

### 9.7 运行时快照、存档与回放

> 本节对不可变 revision、snapshot 恢复和切换失败关闭的要求继续有效；任何依赖独立
> validator/receipt 的具体恢复字段均须由 #300/#302 后继设计重新确认。

运行时快照与共享 image 分离，并至少绑定：

```text
originCanonicalArtifactDigest
originCanonicalArtifactByteLength
originStaticImageDigest
originStaticImageByteLength
runtimeSnapshotVersion
runtimeVersion
identityEncodingVersion
identityRegistryRevision
constraintSetVersion
executionConstraintVersion
networkRevisionDerivationVersion
networkRevision
worldIdentity
tick / time
input-command cursor
runtime-owned random-stream state (future explicit G1 only)
```

快照包含全部每世界可变交通状态。动态通行定义、交通参与单元和其他运行时实体以
快照局部标识保存引用关系；当前 dynamic Route 或未来执行域的等价通行定义还保存
可重建的稳定静态实体引用/规范定义。原进程的 runtime handle、slot、generation、
partition 或 worker assignment 不能成为恢复后身份或跨硬件行为权威。跨路网修订
恢复必须显式迁移，不能把旧 dense ordinal 直接解释为新镜像实体。

版本化 `networkRevision` 与兼容的 runtime/snapshot 契约、精确相等的
identity/constraint/execution-constraint versions 是同修订恢复的静态语义权威。
`originCanonicalArtifactDigest + originCanonicalArtifactByteLength` 与
`originStaticImageDigest + originStaticImageByteLength` 记录创建快照时的精确发布
制品和 target/profile image，只承担来源审计与同制品/同镜像快速路径。

恢复可以改用另一个已认证 target/profile image，即使它因 compiler provenance 或
artifact envelope 重发布而绑定不同的 canonical artifact digest/length；但候选
`TrustedStaticImage` 必须通过 #300/#302 冻结的对象外认证输入绑定到与快照精确相等的
`networkRevisionDerivationVersion + networkRevision`。#299 的 LFCP v2 不绑定 image，
不能单独建立该信任。上述 identity/constraint/execution-constraint versions 必须精确
相等，且
`StaticIdentityIndex` 必须完整重建快照中的全部稳定静态引用和有类型关系。任一条件
不满足即失败关闭。不同 artifact digest 本身不触发跨修订迁移；revision 不同才必须
进入 §9.6。该规则使快照不依赖发布 provenance 或 target-specific dense ordinal，
同时保留原制品/镜像的可审计来源。
保存快照时，Runtime 必须从当前 `TrustedStaticImage` binding 复制
`networkRevisionDerivationVersion + networkRevision`；恢复时只与候选可信镜像
descriptor 的同名字段比较，不接受 save manifest、调用方参数或 image header
覆盖。跨修订恢复只能进入 §9.6 的显式迁移事务。

运行时执行计划在恢复后依据当前硬件与负载重建；快照可以保留诊断性调度统计，但
不得要求复现原分区/工作线程布局才能得到相同精确结果。

回放使用显式输入命令序列（Input Command Sequence）、周期 checkpoint 和确定性
状态摘要。调试构建可借助冷标识和源映射产生失同步诊断制品，定位首个分歧 tick、
phase、实体与资源组件。快照线格式、摘要算法和诊断裁剪由后续 Runtime G1 冻结。

### 9.8 路径规划和出行需求接入

Traffic Runtime 从已提交状态导出已提交交通观测快照，不泄漏 tick 中间提案，也不
拥有全局成本政策。路径规划/出行编排层结合静态路网、观测、收费、游戏政策与偏好
构造版本化动态成本快照，返回可由 Runtime 注册/验证的候选通行定义；不得在每个
交通参与单元的 fixed-tick 内全图寻路。当前车辆域使用 Route，未来执行域由其 G1
冻结等价通行定义。成本快照和候选通行定义必须绑定路网修订、观测 tick 与成本模型
版本；路网修订标识必须来自 `TrustedStaticImage` 派生的 Runtime 静态视图或已
提交交通观测快照，不能由 Routing 调用方自报。Runtime 将其与当前可信 image binding
做精确相等比较，并继续逐项验证候选稳定引用/拓扑；修订标识相等只证明缓存一致性，
不替代候选内容验证。Runtime 对 revision mismatch 失败关闭，允许多旧的观测由
Routing G1 冻结。
观测导出必须同时支持按观测导出节奏（Observation Export Cadence）构造的完整基线和
版本化增量/分区选择路径；
“已提交快照”描述一致性时点，不等同于“每 tick 复制全网全部边状态”。Runtime 不得
把上层成本模型吸入 fixed-tick，但必须为导出与候选注册边界提供可测量的条目数、字节、
分配和耗时；Routing 也不能绕过 revision/tick 绑定直接读取 tick 中间数组。
出行需求和路线选择策略属于城市游戏或出行编排层。#291 只冻结该职责边界，不提前
冻结 `laneflow-routing` crate、算法或公共 API。

## 10. 后发射检查与后继信任边界

```text
LFCA/LFSM/LFSD final bytes
  -> laneflow-format post-emission check
  -> borrowed checked bundle
  -> LFCA/LFSM/LFSD content-addressed installation
  -> LFCP v2
  -> authenticated manifest commit
```

#299 不再建立独立 validator 或三类 receipt。公共后发射检查只重算最终对象
digest/length、LFCA revision 和 LFCA/LFSM/LFSD 跨对象 binding；它不重新运行完整
identity、topology、ownership、geometry、rule 或 semantic-diff 算法。精确 API、错误、
LFCP v2 和性能门槛见
`compiler-post-emission-check-and-minimal-publication-closure.md`。

`CanonicalIdentityTable` 继续是 LFCA 不可裁剪的规范语义表，用于审计、诊断和后继
工具；#299 只检查其登记形状、字段顺序、编码和直接值域，不逐实体重算 BLAKE3，也不
建立第二份 collision registry。compiler 的 identity known vectors、碰撞处理和
LIR→LFCA 投影测试继续承担生产语义正确性。

后继职责保持分离：

- #300 必须从受检 LFCA view 构造目标静态镜像，并冻结镜像外认证描述符、完整性清单、
  bounded verifier 和不可变字节背板；不得假设 #299 会提供
  `static-image-v1` receipt 或独立镜像 oracle；
- #302 必须绑定 base/target revision、artifact/image、LFSD、迁移策略和原子切换提交点，
  但不得假设存在 `revision-cutover-v1` receipt；
- Runtime 仍不得信任 image header、LFSD 或调用方自报 revision；真实性来自对象外
  trust anchor，结构安全来自 bounded verifier，激活决定来自 #302 的可信切换输入。

验证职责按交付切片拆分：

| Issue           | 必需验证                                                                                                   | 明确不承担                           |
| --------------- | ---------------------------------------------------------------------------------------------------------- | ------------------------------------ |
| #299            | 最终 LFCA/LFSM/LFSD 格式、digest/length、revision、跨对象 binding、LFCP v2、发布原子性和 P100 checker 成本 | 完整路网语义复验、静态镜像、迁移授权 |
| #300            | 从受检 LFCA 构造镜像、profile/layout、exact comparison、descriptor/manifest、bounded view                  | compiler 来源/LIR 复验、运行时切换   |
| #302            | base/target/LFSD/镜像绑定、迁移策略、失败原子性、激活提交点                                                | 把 LFSD 自身解释为授权               |
| Runtime/Spatial | trusted image 输入、目标节结构、snapshot/replay 和执行行为                                                 | 编译或重新验证 portable artifact     |

#299 的最小验证矩阵包括：Genesis/Artifact base 成功路径、LFCA revision claim 篡改、
LFSM artifact binding 错配、LFSD base/target 错配、截断/追加/超限、检查失败零
installer/manifest 调用、LFCP v2 fixed vector、Windows/Ubuntu exact bytes、零分配和
P100 checker 门槛。它不新增独立 fuzz service、证明 oracle 或测试 DSL。


## 11. 版本、发布与供应链

契约版本轴（Contract Version Axes）：

```text
authoringFormatVersion
canonicalFormatVersion
canonicalPublicationDescriptorVersion
identityEncodingVersion
identityRegistryRevision
networkRevisionDerivationVersion
staticImageLayoutVersion
staticImageDescriptorVersion
staticImageIntegritySchemeVersion
sourceMapFormatVersion
semanticDiffFormatVersion
validationReceiptFormatVersion
networkRevisionCutoverDescriptorVersion
migrationPolicyVersion
executionConstraintVersion
partitionHintVersion
runtimeSnapshotVersion
constraintSetVersion
```

构建与目标选择器（Build and Target Selectors）：

```text
staticImageProfileId
compilerBuildId
validatorBuildId
targetTriple
```

封闭种类选择器（Closed Kind Selectors）：

```text
validationReceiptKind
```

内容身份与长度绑定（Content Identity and Length Bindings）：

```text
networkRevision
canonicalArtifactDigest
canonicalArtifactByteLength
staticImageDigest
staticImageByteLength
staticImageIntegrityManifestDigest
staticImageIntegrityManifestByteLength
sourceMapDigest
sourceMapByteLength
semanticDiffDigest
semanticDiffByteLength
```

- authoring/canonical 历史迁移离线完成；
- runtime exact-current/fail-closed，不在 production startup 迁移；
- portable artifact immutable publication 继承 ADR 0011；
- static image 可按同一 canonical digest 产生多个 target/profile variant；
- `canonicalArtifactDigest`、`staticImageDigest`、
  `staticImageIntegrityManifestDigest`、`sourceMapDigest`、`semanticDiffDigest`
  与 `validationReceiptDigest` 均为各自 exact bytes 的 SHA-256，不使用 entity
  identity digest 代替；
- 每个上述 digest 必须由受认证 external descriptor/manifest 同时绑定同一对象的
  exact `u64` byte length；消费者在任何与输入大小成正比的读取、解压、分配、解析
  或 hash 前，先检查调用方上限、地址空间与 exact length，未知长度 stream 只允许
  checked length+1 bounded reader，不能用后验 digest mismatch 代替输入边界；
- `networkRevision` 是带 `networkRevisionDerivationVersion` 与 domain separator
  的规范语义载荷 SHA-256，不是 artifact/image exact-bytes digest，也不进入
  steady tick hash 计算；
- digest 只存放在其目标对象之外：artifact/image/source-map 均不把自己的
  digest 嵌回自身 bytes；publication manifest/external descriptor 完成外部绑定；
- compiler、format checker、image builder 的 provenance 必须可审计；
- LFCP v2、external image/cutover descriptor 必须按各自职责绑定路网修订标识、exact artifact、image、
  source map、相应 exact byte length、target、profile、constraint 和 tool builds；
- runtime 不联网解析 schema、artifact 或 toolchain。

上述调用方上限不是不可信对象中的自报字段。约束集或宿主策略必须分别提供并在
任何线性工作前应用：

| 目标对象           | 长度字段                                 | 调用方上限                             |
| ------------------ | ---------------------------------------- | -------------------------------------- |
| 规范制品           | `canonicalArtifactByteLength`            | `maxCanonicalArtifactBytes`            |
| 静态镜像           | `staticImageByteLength`                  | `maxStaticImageBytes`                  |
| 静态镜像完整性清单 | `staticImageIntegrityManifestByteLength` | `maxStaticImageIntegrityManifestBytes` |
| 源映射             | `sourceMapByteLength`                    | `maxSourceMapBytes`                    |
| 语义差异           | `semanticDiffByteLength`                 | `maxSemanticDiffBytes`                 |

authoritative source module graph 是 authoring SSOT。Generated artifact 可以作为
release/CI artifact 或为特定治理阶段 checked in，但只能由 compiler 生成并经
后发射检查、内容寻址安装和发布 manifest 闭合；永远不允许手改或与 source graph
竞争 authority。

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
- Traffic、`StaticIdentityIndex` 与 `PartitionPlanningHints` mandatory，Spatial
  由 closed profile 控制，diagnostics 外置；
- immutable image 共享、mutable arrays per world；
- static execution constraints worker-count-neutral，最终执行计划 per world；
- 执行计划公开聚合诊断指标：阶段耗时（Phase Cost）、分区负载（Partition Load）、
  边界交换量（Boundary Exchange Volume）、屏障等待（Barrier Wait）、负载偏斜、
  迁移次数/成本、连接组件/SCC 分布、最大 SCC 提案占比、归约工作量
  （Reduction Work）、归约跨度（Reduction Span）与临界路径深度；不公开内部可变
  容器或调度权威；
- tick 不做 string/hash/path matching；
- `NetworkRevisionId` 只在加载、注册动态通行定义、快照恢复和修订切换等冷边界比较，
  不进入逐交通参与单元 fixed-tick 状态或每 tick hash；
- pose 不做 Traffic/Spatial join；
- production load 不做 JSON parse/registry rebuild。

### 12.3 开发闸口（Gate）

具体数字由实现 G1 在固定性能机上用 current baseline 冻结，但 Gate 至少覆盖：

- load latency、peak allocation、retained bytes，以及 `StaticIdentityIndex` 的共享
  retained bytes、按需映射延迟和双向 lookup latency；
- 描述符/完整性清单认证、必需节预先验证（Eager Verification）、
  Spatial 延迟验证/后台验证（Lazy/Background Verification）和显式全镜像审计的
  读取量、墙钟耗时、
  CPU 并行度与峰值分配；最低产品硬件必须分别 sizing，不能只报告高端 SHA-NI 主机；
- 2/8/32 worlds 的 shared-static scaling；
- 单个大型 world 的 worker/partition scaling、barrier、边界交换、负载偏斜、
  连接组件/SCC 分布、最大 SCC 提案占比、归约工作量/跨度和临界路径；生产候选必须
  与集中式参考预言机（Reference Oracle）比较，并说明巨型组件下的有效并行度；
- 当前直接路径、显式参考精确路径（Reference Exact Path）、融合单 worker 与多 worker
  exact path 的阶段成本与端到端成本；融合优化必须通过状态/事件/错误等价证明，
  不能以省略逻辑阶段破坏确定性；
- 在线镜像切换的准备期 tick 干扰、迁移日志字节/记录/落后量、追赶轮次、静默提交
  停顿、双修订峰值内存、放弃率和借用 token 退休延迟；维护暂停模式的完整停顿单列；
- 运行时快照保存/加载（Save/Load）的制品大小、墙钟耗时、主线程停顿、后台 tick 干扰、峰值
  内存、同修订回放与跨修订迁移；城市游戏存档场景不得只做功能等价；
- 已提交交通观测快照的完整/增量（Full/Delta）导出条目数、字节、频率、墙钟耗时、分配和 tick
  干扰，以及 Runtime 接收/验证动态成本快照和注册候选通行定义的成本；上层成本模型
  构造算法不归 LaneFlow 所有，但边界成本必须测量，不能默认每 tick 全量导出全网；
- 一万/十万 Traffic Runtime 与 Spatial 既有能力基线不得无解释回退；精确执行纪律
  的成本与 SoA/CSR、融合执行器等收益分别报告，不以 baseline 数字单独否决终态契约；
- 动态通行定义编译（Dynamic Traversal Compilation）不进入交通参与单元固定步进；
- 中国特色城市工作负载（Chinese-style City Workload）的拓扑/需求/运行时指标由
  后继 G1 建立，不能用 `LF-SYNTH-v1`、LuST 或多世界集合静默代替；
- target-specific SIMD/alignment 候选相对 portable/common layout 的收益。

不能用“BLAKE3/StableId128 可能变大”推导 tick 回退：ID 位于 cold/compiler boundary，
tick 只使用 32-bit dense handle。若身份索引 retained memory 成为问题，使用共享
只读映射、分块/压缩、按需分页或更紧凑的双向索引解决；不得裁掉 snapshot/cutover
所需映射或缩短持久 identity。Source map、canonical tuple 与显示诊断仍可外置。

## 13. 包（Crate）与依赖目标

下图箭头统一表示“左侧 crate 依赖右侧 crate”：

```text
laneflow-format ---------> laneflow-static-contract
laneflow-static-image ---> laneflow-static-contract
laneflow-static-image ---> laneflow-format

laneflow-compiler -------> laneflow-static-contract
laneflow-compiler -------> laneflow-format
laneflow-compiler -------> laneflow-static-image

laneflow-runtime --------> laneflow-static-contract
laneflow-runtime --------> laneflow-static-image
laneflow-spatial --------> laneflow-static-contract
laneflow-spatial --------> laneflow-runtime
laneflow-spatial --------> laneflow-static-image

laneflow-adapter-* ------> laneflow-runtime
laneflow-adapter-* ------> laneflow-spatial
laneflow-adapter-* ------> laneflow-static-image
```

切换期仅保留下列当前态内部依赖：

```text
laneflow-data ----------------> laneflow-current-source
laneflow-data ----------------> laneflow-core
laneflow-data ----------------> laneflow-spatial

laneflow-compiler -X-> laneflow-current-source / laneflow-data
```

`laneflow-current-source` 为 `laneflow-data` 集中当前 wire DTO、版本、摘要和
ScenarioManifest 配对；它不提供编译器严格导入能力。current JSON 未曾对外发布，仓库
夹具由新编制来源一次性重写或随旧加载器删除，因此不建立迁移 crate、编译器特性或
长期离线导入入口。完整边界见 `current-package-import.md`。

职责与禁止依赖：

中文术语与英文辅助名统一见 `../reference/glossary.md`；下表不重复定义双语映射。

| 包                         | 拥有职责                                                                               | 禁止依赖                                     |
| -------------------------- | -------------------------------------------------------------------------------------- | -------------------------------------------- |
| `laneflow-static-contract` | 稳定标识、路网修订标识、种类 / 标签登记表、有类型序号、版本 / 摘要 / 配置档 / 描述符值 | Serde、文件系统、核心 / 运行时、空间层       |
| `laneflow-format`          | 可移植制品线格式/视图、无分配后发射检查、revision/digest/跨对象 binding                | 编译器来源/IR 语义、文件系统、运行时、空间层 |
| `laneflow-static-image`    | 镜像 ABI、节 / 配置档、有界结构校验器、借用视图                                        | 编译器、运行时、空间层                       |
| `laneflow-compiler`        | 前端、中间表示、编译遍、主发射器、源映射 / 语义差异、LFCP v2 与发布编排                | 当前态数据 / 核心对象图                      |
| `laneflow-runtime`         | 固定步进、已实现执行域的交通参与单元、动态通行定义、可变交通状态                       | 编译器、Serde、文件系统、空间层              |
| `laneflow-spatial`         | 规范几何采样、位姿批次                                                                 | 编译器、引擎                                 |

`laneflow-runtime` 是 current `laneflow-core` 的 target 名称；
`laneflow-static-image` 取代含混的 `laneflow-runtime-image` 名称。共享 static
contract 不能继续留在 Runtime，否则 compiler/format 会反向依赖动态运行时。
`laneflow-data` 是 current JSON 临时内部加载实现，主代码路径切换后不再拥有静态
normalization authority，也不保留离线兼容入口。

## 14. 迁移路线

```text
阶段 0  current JSON/Data/Core/Spatial 路径继续生产服役
阶段 1  #291：ADR 0020/0021 + 本设计完成 G1
阶段 2  #292：static-contract + compiler foundation + Synthetic DSL frontend 纵向闭环
阶段 3  #292 验收：integration-only LIR→current projection 支撑 #282–#285 等价验证
阶段 4a #315：官方前端共同受检模块接入实现；治理收口边界以动态记录为准
阶段 4b #296 道路编辑来源前端 + 拓扑/几何 MIR
        #297 收口 current JSON 测试边界并停止迁移前端
阶段 5  #298 可移植规范制品/源映射/语义差异
        + #299 compiler 后发射检查/LFCP v2/最小发布闭合
阶段 6  #300 目标静态镜像 + #301 交通运行时/空间层共享镜像路径
        + #302 不可变路网修订/运行时快照/在线切换
阶段 7  behavior/perf/security cutover Gate
阶段 8  #294：production cutover，完成 core→runtime rename 并移除 projection/重复构建
```

阶段是架构迁移顺序，不是把终态降级为最小方案。#292 已在阶段 2 与阶段 3 均完成后
达到 G4，因此“#292 G4”与“projection 就绪后恢复 #282–#285”是同一前置条件，不是
两个互相竞争的恢复点。阶段 4a 只建立共享模块基础；#296 仍按自身 Gate 推进，#297
调整后只收口 current JSON 与投影测试边界。每个阶段都
必须沿同一个
AST/HIR/MIR/LIR 与 artifact/image contract 前进，不允许先建一个注定废弃的 Core
builder API。阶段 3 的 bridge 固定为 `laneflow-compiler-test-support` 或等价
integration-only crate：它可以依赖 compiler + current Core/Spatial，将 validated
LIR 投影为 current inputs；compiler 不依赖它，它不构成 public backend contract，
并由阶段 8 的切换责任议题 #294 删除。该投影器（Projection）只消费 LIR 已验证的
StableId、有类型所有者关系（Typed Owner Relation）与其他规范语义，不从当前 JSON
或核心对象图（Core Object Graph）反向派生标识（Identity），测试输入直接由编译器
原生有类型模块构造。

阶段 8 的 #294 一次性不兼容改名不仅覆盖 crate/type，也覆盖文档导航、Agent Skill
ID、工具薄包装和治理枚举：`laneflow-core-design` 目标改为
`laneflow-runtime-design`。在 current `laneflow-core/CoreWorld` 仍服役时只同步
Skill 内容并明确 current/target，不提前删除旧发现入口；具体治理枚举迁移必须由
独立 implementation Issue 原子更新 validator、模板和历史兼容规则。

Cutover 前必须证明：

1. current/target 场景的静态语义、tick、event、pose 等价；
2. deterministic artifact/image；
3. compiler 后发射闭合、外部认证 manifest/descriptor 与 bounded structural
   verifier 安全；#300/#302 的信任输入已由各自 G1 冻结；
4. startup、memory、一万/十万和 multi-world Gate；
5. worker/partition 置换等价、无额外 tick 延迟和单大型 world scaling；
6. publication/migration/source map/semantic diff 可用；
7. snapshot/replay 与 network revision cutover 有失败关闭的后继 G1/实现入口；
8. fallback/rollback 只切换 current/target asset path，不存在两套可变 authority。

## 15. 风险登记

| 风险                        | 结果                                                          | 控制                                                                                                         |
| --------------------------- | ------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------ |
| 编译器系统性缺陷            | 批量污染全部资产                                              | 人工可复核固定向量、compiler 语义测试、真实场景回归、历史缺陷断言；#299 明确接受共享后端不能独立发现此类缺陷 |
| 二进制校验器漏洞            | 不可信字节破坏内存安全                                        | 基于偏移量的格式、加载限制、模糊测试 / `unsafe` 审计                                                         |
| 镜像头声明被误当作信任      | 恶意但结构合法的镜像绕过语义闸口                              | 对象外认证描述符/manifest、有界结构校验器；#300 冻结精确镜像信任输入                                         |
| 哈希前输入无界              | 超大替换资产制造无界读取、解压、分配或摘要工作                | 所有 exact-byte 对象绑定 digest + length；有界 reader 预检                                                   |
| 未认证路网修订              | 快照/路由绕过修订检查或兼容恢复误拒绝                         | 语义载荷派生标识、LFCP v2/外部 manifest 与 #302 可信切换输入                                                 |
| 中间表示泄漏运行时类型      | 后端 / 目标被当前核心对象图锁死                               | 静态契约、目标中立 LIR、无环包依赖图                                                                         |
| 标识漂移                    | 引用、语义差异、缓存和存档失效                                | 制品内规范身份表、compiler 固定向量/碰撞测试、变形测试                                                       |
| 源映射错配                  | 审阅或诊断静默指向旧来源文件 / 位置                           | 版本化封套；发布描述符绑定 exact artifact、revision、digest                                                  |
| 边身份耦合可选角色          | 未覆盖边无身份，或调整 overlay 造成伪删除 / 新增              | `LaneEdge` 独立稳定键；RoadSection/Junction 只保存关系                                                       |
| 增量 / 并行非确定性         | CI / 发布字节漂移                                             | 干净单线程预言机、稳定合并                                                                                   |
| 配置档边界错误              | 无图形配置档携带几何，或交叉索引漂移                          | 交通必需 / 空间可选矩阵、配置档测试                                                                          |
| 当前态 / 目标态双路径长期化 | 测试矩阵和语义漂移                                            | 集成专用桥、明确移除责任人 / 切换闸口                                                                        |
| 来源 / 生成物双重事实源     | 手工修改与漂移                                                | 来源模块图权威、生成摘要、后发射检查与发布 manifest                                                          |
| 过早选择归档库              | ABI、安全或 MSRV 锁定                                         | 先冻结契约，再做基准 / 审计                                                                                  |
| 最终分区进入共享镜像        | 地图与硬件/世界耦合，存档不可移植                             | 静态约束 + 可重建提示 + 每世界执行计划                                                                       |
| 分区诱发行为延迟            | 结果随 cut 改变                                               | 同 tick committed-state barrier 与置换等价测试                                                               |
| 依赖连通导致归约跨度膨胀    | 巨型 SCC 或长凝聚 DAG 成为单大型 world 的阿姆达尔瓶颈         | SCC/DAG 分解；归约工作量/跨度指标；#220 production 研究                                                      |
| 精确路径脚手架成本          | 单 worker 相对当前直接路径显著回退                            | 融合精确执行器；四路径分项基准；语义等价 Gate                                                                |
| 原地修改静态镜像            | 摘要、共享、信任和确定性失效                                  | 不可变路网修订 + 失败关闭镜像切换事务                                                                        |
| 在线迁移候选过期            | 切换遗漏准备期间的 tick/命令或重复事件                        | 有界增量日志、追赶、静默提交、切换事件批次恰一次                                                             |
| 迁移准备干扰正常步进        | 玩家改路导致持续抖动或延迟尖峰                                | 后台预算、落后量/干扰 Gate、显式维护暂停模式                                                                 |
| 每次启动全镜像串行摘要      | 城市级镜像加载受固定串行读取限制                              | 认证分块清单；必需节预先验证；冷/Spatial 延迟验证                                                            |
| 生产配置档裁掉稳定身份索引  | 快照、动态路线与跨修订映射无法恢复                            | 全配置档必需冷索引；双向 round-trip；按需映射                                                                |
| 稳定身份索引替代语义证据    | 同 StableId128 的新语义错误继承旧占用、路线、预约或控制器状态 | 受信任 semantic diff；索引只复核映射；index-only 失败关闭                                                    |
| 未受信任语义差异驱动迁移    | 篡改迁移、错误终止或状态错配                                  | 外部切换描述符、双制品独立验证、身份索引复核                                                                 |
| 通行权运行时交付延期        | 静态契约与运行时执行能力长期不对称                            | 明示当前能力边界；#292 G4 后恢复 #282–#285；#285 跨层闭环                                                    |

## 16. 已接受边界与后继实施条件

#291 G1 已确认以下边界。后继 Issue 可以选择具体实现，但不得在没有重新进入架构 G1
的情况下改变这些 closed contract：

- ADR 0020/0021 明确历史 ADR 的继续有效、扩展与取代范围；
- 本文不再存在 L1/L2 或 Core-shaped compiler IR；
- source module graph、标识 registry/encoding、trust/load path、static-image
  profile 与 crate DAG 均为 closed contract；
- Data/current Core/target Traffic Runtime/Spatial/Adapter 文档清楚标注 current 与
  target；
- #292 已完成 compiler foundation + Synthetic DSL frontend、集成专用
  LIR→current projection 及其 G2/G3/G4；#282–#285 关于 #292 的稳定开工前置已经满足；
  #296 按自身 Gate 推进；#297 不建立 current JSON 编译器前端，并按调整后范围重新
  完成 Gate；
  当前 Project 状态和原生依赖关系不在长期设计中镜像；
- 阶段 8 生产切换、core→runtime 原子改名与旧路径移除由 #294 的 G4 独占，不再
  误绑到 #291 的设计交付 G4；
- 标识 v1、artifact/image/profile/version/validation/performance contract 一致，所有
  生产配置档保留 snapshot/cutover 必需的 `StaticIdentityIndex`；
- 全部 `LaneEdge`（包括合法未覆盖边）以独立稳定边键进入 identity closure，且
  RoadSection/Junction 角色变化不造成边身份漂移；
- 静态执行约束、分区规划提示和每世界运行时执行计划职责分离，且 exact path 无
  partition-induced extra tick delay；
- 城市游戏/出行编排/routing、不可变路网修订、快照/回放和每世界唯一性边界一致；
- `traffic-headless-v1`、untrusted-image rejection 和全部 identity kinds 的验收测试已
  写入后续 implementation Gate；
- 本地 docs links/format/contract checks 通过；
- 本设计的状态、实现证据与后继 Gate 必须继续保持可独立核验。
