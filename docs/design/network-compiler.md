# 路网编译器与共享静态路网

**文档状态**: Accepted<br>
**最后更新**: 2026-11-06<br>
**适用范围**: 正本来源模块图（Authoritative Source Module Graph）、编译器中间表示
（Compiler IR）、静态路网编译裁决方、标识派生、可移植规范制品（Portable Canonical
Artifact）、共享静态路网（Shared Static Network）、源映射（Source Map）、语义差异
（Semantic Diff）、compiler 后发射检查、交通运行时（Traffic Runtime）命名、
静态执行约束（Static Execution Constraints）、不可变路网修订
（Immutable Network Revision）<br>
**现行路径**: 道路编辑/合成有类型来源进入
`laneflow-compiler`，经 Typed AST → HIR → MIR → Canonical LIR 发射 LFCA/LFSM/LFSD，
再由受检 LFCA 构建 `SharedNetworkRevision` 并供 `TrafficWorld` 与可选
`SharedSpatialNetwork` 消费<br>

**关联决策与设计**:

- `../adr/0003-runtime-tick-and-determinism.md`
- `../adr/0005-core-identity-and-handle-model.md`
- `../adr/0007-traffic-data-crate-and-loader-boundary.md`
- `../adr/0008-pre-1.0-data-format-version-policy.md`
- `../adr/0013-engine-neutral-spatial-geometry-and-length-authority.md`
- `../adr/0015-bounded-f32-canonical-spatial-frames.md`
- `../adr/0017-static-road-junction-maneuver-and-gate-identity.md`
- `../adr/0020-compiler-owned-static-network-and-static-image.md`
- `../adr/0021-traffic-infrastructure-and-host-boundary.md`
- `../adr/0024-compiler-post-emission-check-and-minimal-publication-closure.md`
- `../adr/0025-checked-canonical-network-and-shared-static-network.md`
- `core-id-handles.md`
- `compiler-foundation.md`
- `portable-canonical-artifact.md`
- `compiler-post-emission-check-and-minimal-publication-closure.md`
- `shared-static-network.md`
- `cross-section-access.md`
- `spatial-geometry.md`
- `road-junction-model.md`
- `signal-system.md`
- `waiting-zone-conflict-right-of-way.md`
- `../reference/glossary.md`

## 术语规范

本文的以中文术语和中文定义为准，英文只作辅助理解。完整双语映射以
`../reference/glossary.md` 为 SSOT；本文不另建竞争词表。类型、crate、字段、版本
值、算法和协议常量等精确标识符使用反引号保留原文，但其语义由中文正文裁决。

本文的主链路按中文规范术语表述为：

```text
正本来源模块图（Authoritative Source Module Graph）
  -> 有类型抽象语法树（Typed AST）
  -> 高层中间表示（HIR）
  -> 中层中间表示（MIR）
  -> 已验证规范低层中间表示（Validated Canonical LIR）
  -> 可移植规范制品（Portable Canonical Artifact）
     + 源映射（Source Map）
     + 语义差异（Semantic Diff）
  -> 受检规范路网输入（Checked Canonical Network Input）
  -> 共享路网修订（Shared Network Revision）
```

## 1. 结论与状态

现行路网编译器不是“生成另一份 JSON 的工具”，而是新的静态数据体系：

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
                                                          ├─> LFCA -> checked input
                                                          │           -> SharedNetworkRevision
                                                          ├─> source map / diagnostics
                                                          └─> semantic diff

SharedNetworkRevision ─┬─> Traffic Runtime: SharedTrafficNetwork + per-world mutable state
                       │                    + RuntimeExecutionPlan
                       └─> Spatial: optional SharedSpatialNetwork
                                     + optional lane_pose capability + batch scratch/output
```

核心结论：

- 不同来源语言（Source Language）/来源模块是平级前端输入，不构成架构层级；
- 编译单元的唯一数据编制正本是可重放的正本来源模块图，几何文档只是生产场景的
  主要来源语言；
- 编译器是全部静态路网的唯一编译裁决方；
- AST/HIR/MIR/LIR 逐级降阶，只有以已验证规范 LIR 为静态语义核心的成功编译结果
  可以进入编译发射器；
- 可移植规范制品是持久化规范后端；共享静态路网从受检 LFCA 构建，不另设持久化
  性能后端，源映射另外消费同次成功编译冻结、但不能补充静态语义的来源伴随数据；
- `laneflow-runtime` / 空间层消费同一不可变 `SharedNetworkRevision` 的对齐
  component；
- 静态只读数据与每世界可变状态物理分离；
- 生产启动不再解析 JSON、按字符串重绑定、重建登记表或重新编译初始路线出现记录；
- compiler 发布前由 `laneflow-format` 从最终字节重算 revision/digest/length 并核对
  LFCA/LFSM/LFSD binding，但不复验完整路网语义；
- 稳定声明/可寻址派生实体、所有者局部出现记录与密集 LIR 表行使用不同标识；
- 规范标识元组、BLAKE3-128 `StableId128`、XXH3 瞬态加速边界和密集句柄保持
  热路径分层；
- 发布资产的产品真实性由 LFCP v2 / manifest admission 建立；本地确认建造直接使用
  compiler 后发射受检 capability。两者随后必须进入同一 LFCA builder；
- `SharedTrafficNetwork`、`SharedIdentityIndex` 与 `PartitionPlanningHints`
  是所有共享修订的必选 component；只有 `SharedSpatialNetwork` 可选，无图形
  构建不保留几何。
- 编译器保存 worker 数无关的静态执行约束，运行时按世界建立实际执行计划；最终
  partition/worker assignment 不进入稳定标识、可移植制品语义或共享静态路网。
- 共享静态路网是一个不可变路网修订，而非城市永久不变；拖动/预览不编译，确认建造
  后构建候选，并由 #302 通过失败关闭修订切换事务进入运行世界。
- ADR 0021 定义交通基础设施与宿主边界，城市经济、出行需求、路线选择策略和
  游戏规则继续由宿主拥有。

`laneflow-runtime` 是唯一可运行交通世界；旧 JSON/Data/Core 入口不属于当前合同。
精确消费边界见 `traffic-runtime-shared-consumption.md`。

## 2. 前端不是降阶层级

合成领域专用语言、Geometry、OSM 与 Editor 只是不同来源，不存在“更低”或“更高”的
降阶关系。拓扑、几何、标识、长度和 Gate/Waiting coverage 只有在同一个 MIR/LIR 中才能
一次性裁决，编译器中层不得依赖运行时对象图。

#292 的合成领域专用语言是编译器前端的首个纵向验证。后续 Geometry、OSM 或 Editor 前端
不依赖它的语法，只依赖共同的 Typed AST/HIR 合同与编译遍。

## 3. 当前生产边界

| 关注点                            | 现行合同                                              |
| --------------------------------- | ----------------------------------------------------- |
| 数据编制（Authoring）             | 正本来源模块图；道路编辑来源是主要生产来源语言        |
| 交通加载（Traffic Load）          | 受检 LFCA → `SharedTrafficNetwork`                    |
| 空间加载（Spatial Load）          | 同一修订上的可选 `SharedSpatialNetwork`               |
| 标识（Identity）                  | 编译器生成 `StableId128`；共享路网使用密集 `u32` 句柄 |
| 静态出现记录（Static Occurrence） | 编译器不预编译路线；路线出现记录只进入每世界表        |
| 规范制品                          | LFCA/LFSM/LFSD + 源映射 + 语义差异                    |
| 运行时性能数据                    | 进程内不可变 `SharedNetworkRevision`                  |
| 执行规划                          | 静态约束 + 可重建提示 + 每世界运行时执行计划          |
| 道路修改                          | 确认建造后构建候选修订 + 安全边界修订切换事务         |
| 验证                              | 编译器语义裁决 + 后发射检查 + shared-network 构建闭合 |

## 4. 最终裁决职责（Authority）

### 4.1 数据编制正本（Authoring Authority）

一个 compilation unit 的唯一 authoring authority 是显式、可重放的
**authoritative source module graph**，不是某一种固定文档格式。每个 module 至少冻结：

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
重放与来源链，不参与实体稳定标识。#292 合成领域专用语言前端的首版记录与摘要规则由
`compiler-foundation.md` 冻结。

共同模块接入以一个逻辑模块拥有一个或多个来源文档的下列形状替代单文档摘要字段：

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
版本化、域分隔聚合后的 SHA-256。两者只服务重放、缓存比较与来源链，不参与实体稳定标识。
聚合不能选择某一文档，也不能重新读取和哈希全部来源载荷；精确 v1 原像以
`compiler-foundation.md` 第 3.3 节为准。任何会改变规范来源记录字节的编码变化必须提升对应
`frontendVersion`；任何改变文档集聚合原像的变化必须提升
`sourceDocumentSetDigestVersion`。现有 Synthetic `sourceContentDigest` 必须逐项等于候选单文档的
`sourceDocumentDigest`；新的文档集摘要具有独立语义。任何后继前端都必须冻结自己的精确来源字节。

模块级 `origin / provenance` 描述工具、选项与整体转换来源链，不能替代每份文档自己的
来源记录（Source Document Origin）。后者保存冷的显示/审计来源，并与文档摘要和长度
不可分关联；调用方提供的宿主路径或发布来源声明不自动具有真实性。实现可以把重复
来源字符串驻留到共享表，但每个文档必须保留明确关联。逐文档来源记录不参与
`sourceDocumentSetDigest`、稳定标识或 LIR 语义摘要，其条目、字符串与存续字节必须
纳入资源上限。

- 道路编辑状态是 production 编辑器和程序化生成场景的主要 source language；可视化编辑器、
  第一方 Rust 构造面与 importer 共享有类型道路编辑模型，使用按模块的
  `LF-ROAD-EDITING-SOURCE-v4` FlatBuffers 来源缓冲区持久化；
- Synthetic DSL source 是测试、fixture、benchmark 和示例 module 的正本输入；
- import module 必须记录原始 source bytes/digest、importer build、选项和 provenance；
  选择 materialize 为道路编辑来源模块时，必须显式记录 authority 切换；
- editor 编辑并持久化道路编辑状态，通过稳定实体、属性和画布 selection 关联诊断，
  不私有化 semantic compiler；
- programmatic generator 若参与可发布 compilation，必须记录 build ID、参数、
  seed、namespace 和输入 digest；匿名 AST 注入只能用于非发布测试。

typed AST/HIR/MIR/LIR 都是 source graph 的派生物。Portable artifact 是经过验证的
canonical publication contract；`SharedNetworkRevision` 是从受检 LFCA 重建的进程内
性能数据。二者都不得反向覆盖 source module。

### 4.2 编译器裁决权（Compiler Authority）

compiler 唯一负责静态路网的：

- symbol/reference/unit resolution；
- topology/geometry 展开与全局语义；
- 稳定声明/可寻址派生实体的 StableId128 与全部 LIR row 的 deterministic ordinal；
- dense logical ordinal、owner/member/reverse indexes；
- Traffic/Spatial 长度共同派生；
- ManeuverPath / ManeuverGate / WaitingZone 静态实体与关系（**不是**路线出现记录；
  路线出现记录只在 `TrafficWorld::register_route` 编译，ADR 0029）；
- worker 数无关的静态执行约束、资源依赖组件、规范合并键与可切分边界；
- portable artifact、source map 和 semantic diff emission；共享静态路网由独立
  `laneflow-static-network` crate 从受检 LFCA 构建。

LFCA 不发射分区提示 payload。可丢弃、可重建且不拥有行为裁决权的
`PartitionPlanningHints` 由 `laneflow-static-network` 按显式 derivation version 从上述已
受检规范关系确定性派生；compiler 不经 LIR 私有旁路把提示交给 Runtime。

### 4.3 运行时正本（Runtime Authority）

- Traffic Runtime：固定步进（Fixed Tick）、已实现执行域的交通参与单元、动态通行
  定义（Dynamic Traversal Definition）生命周期、控制器时钟（Controller Clock）、
  授权/预约（Grant/Reservation）、停驻占用（Stationary Occupancy）、每世界
  运行时执行计划和所有可变交通状态；
- Spatial：canonical geometry sampling 与 pose batch；
- Adapter：宿主 entity、Transform、frame placement、presentation lifecycle；
- shared static network：只读静态事实，不持有可变 authority。

### 4.4 宿主应用、出行编排与路径规划裁决权

- 宿主应用层拥有应用业务，例如人口、经济、土地利用、建筑、任务与游戏规则；
- 出行与交通编排层拥有出行需求、出发时刻、参与单元生成、目的地、人口生命周期和
  路线选择策略；
- Traffic Runtime 从已提交状态导出交通观测快照，不拥有全局路径成本政策；
- 路径规划/出行编排层结合静态路网、已提交交通观测、收费、游戏政策与偏好构造
  动态成本快照并生成候选路径，不在交通参与单元 fixed-tick 热路径执行全图寻路；
- Traffic Runtime 只裁决交通参与单元如何按所属执行域的静态规则和已提交动态状态
  安全推进。

具体 routing crate/API 不属于本设计；当前边界是宿主自有 Routing + LaneFlow 纯契约，见
[`traffic-observation-and-routing-integration.md`][routing-integration]。
这些正本边界不得被 compiler、Adapter 或 scenario policy 绕过。

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

### 5.2 官方前端共同受检模块接入（Shared Checked Module Admission）

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
`compiler-foundation.md` 第 3.3、10.4 节为准。
v1 只以 `ModuleCount` 隐式约束一模块一文档形状，当前 Synthetic 符合该形状；任何多文档模块都
必须在前端按规模分配和共同接入提交前确认宿主显式选择了 v2 或后继携带
`SourceDocumentCount` 的版本化配置档。多文档正式前端必须在读取、哈希或解析前拒绝
不具备 `SourceDocumentCount` 的配置档；#296 道路编辑来源按“一模块一个 source
buffer/document”应用
同一规则，不得为 v1 猜测默认值或自动升级。

来源专用封装以移动语义进入接入值，不复制完整 AST、字符串或几何。精确来源载荷
（exact source record）只存续到官方前端 `finish` 完成一次摘要、长度和来源位置派生；
共同接入只保留已绑定摘要、精确长度、来源位置、模块级来源链与逐文档来源记录。借用
调用方输入的前端不得
为接入复制来源全文。记录级循环禁止特征对象（trait object）、前端枚举
（enum）分支、重复
摘要、重复资源计数或第二次规范排序。精确所有权、失败事务和共同接入性能验证以
`compiler-foundation.md` 第 3.3 与 10.4 节为准。

`laneflow-compiler` 不依赖 JSON。编译器测试直接构造有类型模块。

这不是第三方前端插件协议。公开面只使用 LaneFlow 拥有的具体入口以及它们进入共同
私有接入的规则，不冻结 Geometry 公共签名。Road Editing 接口借用完整
`LF-ROAD-EDITING-SOURCE-v4` bytes 的字段私有 `RoadEditingModuleInput` 和唯一原子
`add_road_editing_module`；wire DTO、typed module 和 descriptor 均保持私有。
当前树不存在 JSON 迁移入口、相应特性、输入类型或导入器。
通用 `add_module`、公共前端
特征（trait）、裸 Typed AST 和裸描述符/内容配对继续禁止。

### 5.3 道路编辑与几何编制前端（Road Editing and Geometry Frontend）

长期 production authoring frontend 使用
`LF-ROAD-EDITING-SOURCE-v4`（按模块的 size-prefixed FlatBuffers 来源缓冲区）；道路编辑
状态、有类型道路编辑模型、公开 Rust 构造面、generated `unsafe` 审计边界、阶段
生命周期、曲线细分、stationing、资源/诊断顺序、验证矩阵和性能门槛统一由
`road-editing-source-and-geometry-frontend.md` 管理。本节只保留综合架构边界。目标模型包含：

1. 参考线：三维 curve segments、弧长与方向；
2. 横断面：沿参考线分段变化的 lane/facility 结构；
3. 连接：junction/connection intent、默认生成策略与显式 override；
4. 规则：signals、Gate/WaitingZone、access、parking 和其他静态 overlay。

曲线在 MIR 中按冻结 B1 规则离散为 canonical f32 polyline；共享静态路网不保存
authoring curve evaluator。新契约已经由当前生产编译器入口实现，但仍属于内部未发布实现契约；B1
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

AST 不含 Traffic Runtime handle、runtime slot、target ABI 或 compiler 推断出的最终 geometry。

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

LIR 是规范制品和语义差异发射器的唯一静态语义输入；共享静态路网不直接消费 LIR，
而从 LFCA 重建。源映射发射器
另外消费与同次成功编译原子冻结的已验证来源伴随数据；该数据只能关联来源模块、
来源位置、来源链和下列已冻结键，不能补充静态默认、身份原像、所有者或关系：

- 所有 table row 都有 typed `u32` logical ordinal；
- 稳定 declaration 和可独立寻址的 derived entity 另有 StableId128；
- owner-local relation/occurrence 使用 typed local key，不获得全局 StableId128；
- 所有引用已转为 typed ordinal；
- 数值已规范化到 canonical units/representation；
- 所有 relation 以 deterministic flat sequence/range 表达；
- 静态 occurrence、sampling tables 和 layout-independent precompute 已完成；
- 静态执行约束事实已规范化；v1 LFCA 不包含分区提示 payload；
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

标识正本是可审计的**规范标识元组（Canonical Identity Tuple）**，不是 UUID/哈希值本身。编译管线
必须区分：

| 对象类别                                                                  | 标识形式                                                          | 典型用途                                                              |
| ------------------------------------------------------------------------- | ----------------------------------------------------------------- | --------------------------------------------------------------------- |
| 稳定声明 / 可独立寻址派生实体（Stable Declaration / Addressable Derived） | `StableId128` + 有类型逻辑序号（Typed Logical Ordinal）           | 跨编译引用、源映射（Source Map）、语义差异（Semantic Diff）、发布审计 |
| 所有者局部关系 / 出现记录（Owner-local Relation / Occurrence）            | `(ownerOrdinal, role, localIndex)` 有类型键（Typed Key）          | 相位状态、成员关系 / 区间                                             |
| 所有低层中间表示 / 共享静态表行（All LIR / Shared-static Table Rows）     | 表类型专用的有类型 `u32` 逻辑序号（Table-specific Typed Ordinal） | 编译发射、交叉索引（Cross-index）、运行时热路径（Runtime Hot Path）   |

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
`identityRegistryRevision = 4` 冻结本表的 kind、slug 和 required tag sequence
（kind `1..=24`、tag `1..=35` 连续登记；ADR 0029 及 #284 格式增量）。
required tags 必须按数值严格递增编码：

本表冻结 identity v1 的规范登记。新增 kind
只能提升 registry revision，修改既有 kind 的字段、标签含义或编码必须提升
encoding version。

| 代码（Code） | `entityKind`          | 类别（Category）                              | 英文短名（Slug）          | 必需标签（Required Tags） |
| -----------: | --------------------- | --------------------------------------------- | ------------------------- | ------------------------- |
|            1 | `RoadCorridor`        | 声明（Declaration）                           | `corridor`                | `1,2`                     |
|            2 | `RoadSection`         | 声明（Declaration）                           | `section`                 | `1,3,33`                  |
|            3 | `AuthoringLane`       | 声明（Declaration）                           | `lane`                    | `1,4,32`                  |
|            4 | `LaneEdge`            | 可寻址拓扑实体（Addressable Topology Entity） | `lane-edge`               | `1,5`                     |
|            5 | `Junction`            | 声明（Declaration）                           | `junction`                | `1,6`                     |
|            6 | `Movement`            | 声明（Declaration）                           | `movement`                | `1,8,9,10,34`             |
|            7 | `ManeuverPath`        | 声明（Declaration）                           | `path`                    | `1,7,11,12,13`            |
|            8 | `ManeuverGate`        | 声明（Declaration）                           | `gate`                    | `1,14,15`                 |
|            9 | `WaitingZone`         | 声明（Declaration）                           | `waiting-zone`            | `1,14,16`                 |
|           10 | `StopLine`            | 声明（Declaration）                           | `stop-line`               | `1,17`                    |
|           11 | `SignalGroup`         | 声明（Declaration）                           | `signal-group`            | `1,18`                    |
|           12 | `SignalController`    | 声明（Declaration）                           | `signal-controller`       | `1,19`                    |
|           13 | `SignalPhase`         | 声明（Declaration）                           | `signal-phase`            | `1,20,21`                 |
|           14 | `ParkingFacility`     | 声明（Declaration）                           | `parking-facility`        | `1,22`                    |
|           15 | `ParkingSpace`        | 声明（Declaration）                           | `parking-space`           | `1,24`                    |
|           16 | `LaneGroup`           | 声明（Declaration）                           | `lane-group`              | `1,25,32`                 |
|           17 | `FacilityBand`        | 声明（Declaration）                           | `facility-band`           | `1,26,33`                 |
|           18 | `ParticipantClass`    | 声明（Declaration）                           | `participant-class`       | `1,27`                    |
|           19 | `AccessRule`          | 声明（Declaration）                           | `access-rule`             | `1,28`                    |
|           20 | `VehicleProfile`      | 声明（Declaration）                           | `vehicle-profile`         | `1,29`                    |
|           21 | `ConflictZone`        | 声明（Declaration）                           | `conflict-zone`           | `1,23,34`                 |
|           22 | `CanonicalFrame`      | 声明（Declaration）                           | `canonical-frame`         | `1,31`                    |
|           23 | `ParticipantStream`   | 声明（Declaration）                           | `participant-stream`      | `1,30,34`                 |
|           24 | `RightOfWayPolicySet` | 声明（Declaration）                           | `right-of-way-policy-set` | `1,35`                    |

本表冻结的是 identity v1 可构造静态实体集合，不是 Traffic
Runtime 永久封闭的参与单元种类表。`VehicleProfile` 只服务当前道路机动车执行域；
路线不再作为路网声明种类（ADR 0029）。未来非机动车、步行或轨道执行域若需要不同的
运行参数配置（Runtime Parameter Profile）或通行定义，必须由其 G1 登记新的实体种类/
标签（Entity Kind/Tag）和约束，不得把非车辆参数塞进
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

`ParkingSpace.facilityId` 是可选组织关系，不是停车位身份所有者：字段显式存在时，
canonical LIR 保存有类型的 `ParkingSpace -> ParkingFacility` 关系；字段缺省时不生成
该关系。两种情况下 `ParkingSpace` 都只以 authoring namespace 与 tag 24
`parkingSpaceKey` 派生 StableId128，设施归属变化不得造成停车位身份漂移，也不得由
编译器臆造缺失的停车设施。

`ConflictZone` 与 `ParticipantStream` 都以所属 `Junction` 的 StableId 作为父锚点，
分别配合 tag 23 `conflictZoneKey` 与 tag 30 `participantStreamKey`。stream 对
`ManeuverPath` 的引用不进入身份原像；路径几何或拓扑调整不得无故改写同一编制 stream
的身份，重新归属 Junction 则必须改变身份。

Movement 的 left/straight/right/u-turn 分类是来源显式声明的非身份属性，不参与标识；
缺失不能由几何推断，见路权策略实施合同 §3。
路线只经 runtime `register_route` 使用 generation-aware handle，不获得持久
StableId128，也不进入 Identity 可构造种类（ADR 0029）。

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
- 后继若为其他真实来源增加导入器，必须在自身 G1 冻结稳定命名空间、显式边键和角色
  关系降阶；不得恢复已取消的 current JSON 映射，也不得按角色选择不同实体种类；
- 缺失边键只能产生待确认建议，未持久化确认前不得发布匿名、按几何或按序号派生的
  边身份。

`ParkingFacility`、`ConflictZone`、`ParticipantStream` 与 `RightOfWayPolicySet` 统一登记在
`identityRegistryRevision = 4`。`JunctionGroup`
等其它未来 domain 只有在规范设计冻结后才能 append 新 kind code。新增 kind 提升
registry revision，但不改变既有 kind 的 bytes/ID；修改既有 kind 的 required field、
tag 含义或编码必须提升 `identityEncodingVersion`。LFCA 的物理分块只改变承载方式；
typed ordinal 与 StableId 仍在完整逻辑表范围内定义，不得改成 `(chunk,row)` 身份。

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
|          22 | `parkingFacilityKey`       | ASCII 字节（Bytes）        |
|          23 | `conflictZoneKey`          | ASCII 字节（Bytes）        |
|          24 | `parkingSpaceKey`          | ASCII 字节（Bytes）        |
|          25 | `laneGroupKey`             | ASCII 字节（Bytes）        |
|          26 | `facilityBandKey`          | ASCII 字节（Bytes）        |
|          27 | `participantClassKey`      | ASCII 字节（Bytes）        |
|          28 | `accessRuleKey`            | ASCII 字节（Bytes）        |
|          29 | `vehicleProfileKey`        | ASCII 字节（Bytes）        |
|          30 | `participantStreamKey`     | ASCII 字节（Bytes）        |
|          31 | `canonicalFrameKey`        | ASCII 字节（Bytes）        |
|          32 | `roadSectionStableId`      | 16 个原始字节（Raw Bytes） |
|          33 | `roadCorridorStableId`     | 16 个原始字节（Raw Bytes） |
|          34 | `junctionStableId`         | 16 个原始字节（Raw Bytes） |
|          35 | `rightOfWayPolicySetKey`   | ASCII 字节（Bytes）        |

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

摘要角色与各角色的默认算法以
[ADR 0033][adr-0033] 为准。compiler
中的对应关系：

- **持久标识摘要**：`StableId128` 使用 BLAKE3-128，是唯一持久化实体标识摘要；
- **瞬态指纹**：compiler 进程内 hash table/cache/fingerprint；命中后必须比较完整
  canonical tuple，不得进入 artifact 引用；
- **互操作摘要**：artifact/image/publication integrity 使用 SHA-256，不与实体标识
  摘要混用。

来源文档、文档集、来源集合、编译选项及生成器参数/编制选项摘要均进入可移植规范制品
家族或其派生链，按 ADR 0033 §9 保持 SHA-256；不能仅因用于重放或缓存比较就归为
进程内摘要。逐项字段去向及道路编辑原始选项与派生选项的区别见
[编译器基础 §3.3][compiler-foundation-frontends]。

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
  `loop` 边可以完整进入 `SharedIdentityIndex`（路线不再作为身份种类，ADR 0029）；
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

本节保存长期边界；LFCA/LFSM/LFSD/LFCP 的实现级线格式、后发射闭合与 LFCP binding
集中在 `portable-canonical-artifact.md`。

平台无关、确定性、closed shape，并包含：

- canonical format、identity encoding/registry、network revision derivation 与
  constraint versions；
- 路网修订标识（Network Revision ID）`NetworkRevisionId`；
- 规范身份表（Canonical Identity Table）`CanonicalIdentityTable`：对每个稳定实体
  保存 `entityKind`、typed ordinal、制品声明的 `StableId128` 与完整有序
  `(fieldTag, fieldValue)` 规范元组原像；该表属于制品语义，不是可裁剪诊断；
- logical entities、typed ordinals、normalized numeric values；
- topology/geometry/static rule relations；
- canonical payload envelope 与 compiler provenance；自身 digest 不嵌入 artifact
  bytes。

它用于 publication、长期审计、migration 和共享静态路网重建。它不是 Runtime hot
layout，不承诺与 Rust struct ABI 相同，也不因某次 headless 构建缺少 Spatial
component 而丢失 canonical semantics。

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
map/diagnostics、publication metadata 或共享内存布局。这样相同
规范语义在不同 compiler provenance 与内部布局中保持同一修订，而任何
运行时可观察静态语义或参与派生的契约版本变化都会产生新修订。Compiler 在 LIR
freeze 后计算该字段；#299 后发射检查从 artifact semantic payload 重算并逐字节
比较，不能信任 artifact 内自报的值，但不重新验证 StableId 派生或完整静态语义。
`canonicalArtifactDigest` 仍认证
完整 artifact exact bytes，外部 descriptor 以 `canonicalArtifactByteLength` 认证
同一字节序列的精确长度；二者不得互相替代，也不得在长度上限预检前读取或 hash
不可信 artifact。
#299 不维护跨发布的 `NetworkRevisionId` 碰撞登记，也不因后发射检查通过而授权切换。
若 #300/#302 需要在共享修订或切换边界中比较历史 payload，应在各自 G1 重新冻结其
登记、失败关闭与恢复策略，不得把该职责反向扩入 #299。

### 8.2 共享静态路网（Shared Static Network）

共享静态构建不生成第二个持久化性能制品。`laneflow-static-network`
只接受字段私有的 `CheckedCanonicalNetworkInput`，按 LFCA wire order 先计数和预算，
再一次性 reserve/fill 连续 typed columns、CSR/ranges、身份索引、按显式非语义 derivation
version 从受检关系确定性派生的规划提示，以及可选 Spatial 数据，完成跨表/身份/
Traffic-Spatial/执行约束闭合后返回。构建只认 `formatVersion = 5` 的受检输入；
详见 `shared-static-network.md` §3.1。闭合后返回：

```text
SharedNetworkRevision
  CanonicalNetworkOrigin
    LFCA digest + exact length
    networkRevisionDerivationVersion + NetworkRevisionId
    static contract versions
    partitionPlanningHintsDerivationVersion
  required SharedTrafficNetwork
  required SharedIdentityIndex
  required PartitionPlanningHints
  optional SharedSpatialNetwork
```

根对象及 component 字段私有、不可变且 `Send + Sync`；一个 world 只共享根对象的 `Arc`，
内层热路径从该根借用连续 component/slices/ranges，不逐 component 或逐实体引用计数。
Traffic/Identity/Hints/Spatial 物理拆分但由根直接拥有并逻辑绑定同一修订；公共 API 不返回
独立 component 所有权句柄，也不能独立替换。构建成功后输出不借用 LFCA/LFSM/LFSD
backing；runtime-only 调用方可释放 LFCA，可编辑 session 所需 exact base LFCA 由 editor/
#302 在共享路网修订外单独拥有。失败、取消或预算不足不返回部分 component、不修改现有 world
且不写磁盘。

发布加载先完成 LFCP v2 / manifest admission；玩家确认建造则从
`PostEmissionCheckedBundle` 取得同类受检 LFCA 输入。两条路径在 builder 前汇合。
拖动/预览不触发构建；v1 允许确认建造后的全量候选构建，不交付局部增量、原地修改、
静态镜像文件/ABI、descriptor、integrity manifest、mmap、磁盘 cache 或 target/profile
文件变体。具体 API、内存布局、资源和测试矩阵见 `shared-static-network.md`。

#### 被 Accepted ADR 0025 取代的历史镜像草案

镜像 digest、分块、描述符和 `TrustedStaticImage` 加载步骤只存在于本节的 git 历史，不是现行实现输入。现行构建见上文与 [共享静态路网][shared-static-network]。

### 8.3 外部镜像描述符（External Image Descriptor，历史）

本节已经被 Accepted ADR 0025 整体取代。#300 v1 不定义 `StaticImageDescriptor`、镜像摘要、长度、完整性清单、分块或镜像 trust capability。字段草案只存在于本节的 git 历史，不是现行实现输入，其中的「必须」不再约束实现。

现行信任边界是后发射检查与 [LFCP 2][portable-lfcp]，不是镜像描述符。见 [后发射检查][post-emission]。

### 8.4 源映射与诊断（Source Map and Diagnostics）

现行源映射线格式是 [LFSM 4][portable-lfsm]。带内嵌 `records` 的 `SourceMapEnvelope` 和 `canonical-publication-v1` 描述符不是现行实现输入，字段草案见 git 历史。发布 binding 的检查行为见 [后发射检查 §8][post-emission-lfcp]。

源映射不是一组可以脱离编译批次复用的裸记录。即使实体 `StableId128` 与 typed ordinal 都未变化，来源位置、来源模块或编译输入变化也必须生成新的 LFSM exact bytes。诊断使用稳定 code、severity、primary/secondary source location、原因和可执行建议；文本来源可把 location 渲染为 span。authoring error 指向 source 或画布；artifact corruption 或 version mismatch 面向运维，不返回 generated JSON 行号。

### 8.5 语义差异（Semantic Diff）

现行语义差异线格式是 [LFSD 4][portable-lfsd]。带内嵌 `records` 的 `SemanticDiffEnvelope` 不是现行 wire，字段草案见 git 历史。

语义差异不能脱离旧/新规范制品解释。Stable entity 按 `StableId128` 报告；owner-local derived record 在 owning `StableId128` 与 typed role 内按 canonical relation value 做确定性 sequence diff，报告 before/after `localIndex`，但不把位置当成跨编译标识：

- entity add/remove/rename-display；
- topology reconnect、owner/member 变化；
- geometry/length/tolerance-significant change；
- Gate/Waiting/signal/access behavior change；
- 标识闭包变化及原因；
- shared-network 内部 layout-only change 不进入 LFSD，也不得伪装成 semantic change。

PR 审阅不依赖二进制 diff。Base 与 target 必须通过后发射检查；无 baseline 时使用显式 genesis marker。重复 relation value 的序列对齐按最低 before/after `localIndex` 确定性破同值，diff 本身不获得 authoring authority。

运行时修订切换不得把 compiler 输出的裸 diff 当作迁移授权。两个 `SharedIdentityIndex` 只能复核稳定身份与密集序号的对应关系，不能证明相同 `StableId128` 的拓扑、几何、访问规则或其他静态语义仍与旧状态兼容。跨修订状态迁移的绑定、失败关闭与原子提交见 [修订切换][revision-cutover]。缺少完整切换输入时，旧世界仍继续运行。新修订可以作为无旧状态的新世界启动，但不得把它伪装成迁移。

## 9. 静态/可变状态和运行时消费

### 9.1 交通运行时（Traffic Runtime）

现行运行时是 `LaneFlow Traffic Runtime` / `laneflow-runtime`，公开 world 为
`TrafficWorld`。static contract、format、shared-network builder 和 compiler type
位于各自 crate。

`TrafficWorld` 共享 `Arc<SharedNetworkRevision>` 并从根借用 `SharedTrafficNetwork`，只分配：

- 已实现执行域的交通参与单元与动态通行定义代际（Dynamic Traversal
  Generations）；
- controller clocks/indications；
- grant/reservation/waiting/parking occupancy；
- command/event/snapshot buffers；
- dynamic Route occurrence metadata；
- world identity、输入命令游标、从共享路网修订取得的当前路网修订绑定；
- 每世界运行时执行计划、边界交换缓冲与调度统计。

当前首个 projection 仍使用 vehicles、dynamic Route 与 parking occupancy；这些
current specialization 不得反向冻结终态 Traffic Runtime 的参与单元或执行域模型。

人口、Routing 和游戏规则的 seed/随机流属于 caller/出行编排层，不进入
`TrafficWorld` 隐藏状态。只有后续 G1 显式授予的 Runtime 自有随机流才成为每世界
状态与运行时快照内容。

compiler **不**预编译路线。`TrafficWorld::register_route` 在本世界编译出现记录，
保持 ADR 0017 匹配规则与 ADR 0029 单一入口。

### 9.2 空间层（Spatial）

Spatial 只在 `SharedNetworkRevision` 含 Spatial component 时借用
`SharedSpatialNetwork`。只有 `lane_pose()` capability 存在时，LaneEdge geometry tables
才与 Traffic edge ordinal 完整对齐；facility/profile/frame-only component 不能启动车辆
pose sampling。可用 pose 路径不建立按边索引的 `HashMap` 或按 external ID join，
并继续遵守 ADR 0015 的 canonical f32、frame token、稳定顺序、零分配和失败原子性。
headless 构建不创建 Spatial runtime。Spatial session 只
`bind(Arc<SharedNetworkRevision>)`，不依赖 `laneflow-runtime`；同修订由组合根与
pose 批次上的 `NetworkRevisionId` 保证。详细契约见
`traffic-runtime-shared-consumption.md`。

### 9.3 admission 与共享构建状态

目标 API 区分三类职责，不能把它们合并成一个“可信镜像”状态：

1. 发布 admission：宿主认证 LFCP v2 / manifest 对 exact LFCA 的绑定；
2. `CheckedCanonicalNetworkInput`：`laneflow-format` 已检查 LFCA framing、registry、
   直接值域、实际 digest/length 和重算的 `NetworkRevisionId`，但尚未证明 Runtime
   跨表闭合；
3. `SharedNetworkRevision`：`laneflow-static-network` 已完成计数、分配、填充和
   reference/identity/Traffic-Spatial/execution-constraint closure 的 owned 成功结果。

玩家确认建造不执行发布 admission 或内容存储，而是从同进程
`PostEmissionCheckedBundle` 提供第 2 类输入。无论来源为何，builder 不接受裸
`&[u8]`、调用方自报 digest/revision 或 compiler-private LIR。

#### 被 Accepted ADR 0025 取代的历史镜像加载状态

`UnverifiedImageBytes`、`StructurallyVerifiedImage` 与 `TrustedStaticImage` 的分块和 mmap 方案只存在于本节的 git 历史，不是现行实现输入。

### 9.4 共享构建闭合（Shared Build Closure）

当前 builder 在最终对象成功前检查 canonical row/order、typed count/range、跨表引用、
身份双射、Traffic/Spatial edge/frame/length 对齐、execution contract 以及 caller
output/retained/scratch budget。它不检查路线出现记录：当前 LFCA 合同没有路线实体或路线出现记录
关系表，出现记录只在 `TrafficWorld::register_route` 之后进入本世界（ADR 0029）。它不重新
执行来源、HIR/MIR/LIR 的完整编译语义，也不建立第二套 validator/receipt/证明平台。具体
闭合和错误矩阵以 `shared-static-network.md` §7/§11 为准。

#### 被 Accepted ADR 0025 取代的历史镜像结构校验器

镜像文件读取检查只存在于本节的 git 历史。现行构建闭合见上文，并以 [共享静态路网][shared-static-network] 为准。

### 9.5 并行就绪执行规划（Parallel-ready Execution Planning）

静态执行约束图表达冲突、等待、下游存储、控制器等共享资源的依赖组件、安全切分
（Safe Cut）、规范提案/资源声明/事件合并键和提交顺序。它是 LIR 派生的行为约束，
对工作线程数量和具体分区计划中立。

分区规划提示只保存预估成本、边界权重或推荐 cut 等性能信息；LFCA 不保存该 payload，
`laneflow-static-network` 按显式 derivation version 从受检静态关系确定性派生，运行时可以
忽略或重建。derivation version 不进入 `NetworkRevisionId`，提示差异不得改变精确结果。
每个 `TrafficWorld` 依据静态约束、硬件与动态负载构造自己的运行时执行计划，实际
分区/工作线程/边界缓冲与迁移状态永不回写共享静态路网。

精确路径按以下状态流执行：

```text
已提交状态（Committed State）T
  -> parallel read/evaluate
  -> 提案（Proposals）与资源声明（Resource Claims）
  -> canonical component-local reduction
  -> 原子提交（Atomic Commit）T + delta
```

所有分区读取同一 `T`。跨边界不能增加一 tick 延迟；每个连接资源组件只有一个规范
归约裁决方，但互不相交组件可以并行归约。工作线程数、任务完成顺序和分区计划不得
改变已提交状态、事件或安全结果。本设计不冻结固定边界邻域、分区算法、任务运行库、
固定点或精确累加器；这些选择必须由热点、误差和基准证据裁决。

“唯一规范归约裁决方（Canonical Reduction Authority）”定义唯一结果与规范顺序，
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

### 9.6 不可变路网修订与修订切换

`SharedNetworkRevision` 代表一个路网修订。结构性道路编辑在用户确认建造后重新进入正本来源模块图，顺序发射 file-backed LFCA/LFSM/LFSD，经后发射检查后直接从同一 backing 交给 builder 生成候选；不得保留完整对象级内存 LFCA，共享数组不得原地 mutation。候选构建期间当前修订继续运行。

失败关闭的切换事务、描述符、迁移策略与原子提交的现行合同见 [修订切换][revision-cutover]。临时封闭等不改变静态身份或拓扑的状态不伪装成静态修订。本文不另写运行时步骤，也不把历史 image/receipt 字段当作现行描述符；那些草案见 git 历史。

### 9.7 运行时快照、存档与回放

运行时快照与共享静态数组分离。版本轴、绑定集、恢复与回放的现行合同见 [运行时快照][runtime-snapshot]。本节不再列出另一套字段。

存档只捕获已经成功进入 Runtime 的 committed source 与同一时点快照；working/candidate 不进入存档。发布资产若要支持道路编辑，必须另带可重编译到同一修订的 committed `RoadEditingState`，不能从 LFCA 恢复仅用于编制的曲线。

### 9.8 路径规划和出行需求接入

Traffic Runtime 从已提交状态导出已提交交通观测快照，不泄漏 tick 中间提案，也不
拥有全局成本政策。路径规划/出行编排层结合静态路网、观测、收费、游戏政策与偏好
构造版本化动态成本快照，返回可由 Runtime 注册/验证的候选通行定义；不得在每个
交通参与单元的 fixed-tick 内全图寻路。当前车辆域使用 Route，未来执行域由其 G1
冻结等价通行定义。成本快照和候选通行定义必须绑定路网修订、观测 tick 与
成本模型版本；路网修订标识必须来自当前 `SharedNetworkRevision` 或已
提交交通观测快照，不能由 Routing 调用方自报。Runtime 将其与当前共享路网修订绑定
做精确相等比较，并继续逐项验证候选稳定引用/拓扑；修订标识相等只证明缓存一致性，
不替代候选内容验证。Runtime 对 revision mismatch 失败关闭；观测合同包含状态序号，
并把候选时效冻结为 `[observationTick, validThroughTick]` 的
fixed-tick 闭区间。
观测导出必须同时支持按观测导出节奏（Observation Export Cadence）构造的完整基线和
版本化增量/分区选择路径；
“已提交快照”描述一致性时点，不等同于“每 tick 复制全网全部边状态”。Runtime 不得
把上层成本模型吸入 fixed-tick，但必须为导出与候选注册边界提供可测量的条目数、字节、
分配和耗时；Routing 也不能绕过 revision/tick 绑定直接读取 tick 中间数组。
出行需求和路线选择策略属于宿主应用或出行编排层。当前设计不新增
`laneflow-routing` 算法 crate 或 LaneFlow wire，具体
API/失效/性能合同见 `traffic-observation-and-routing-integration.md`。

## 10. 后发射检查与后继信任边界

```text
LFCA/LFSM/LFSD final bytes
  -> laneflow-format post-emission check
  -> source-owning checked bundle
  -> SharedNetworkRevision
  -> optional LFCP v2 exact bytes/bindings for host publication
```

#299 不再建立独立 validator 或三类 receipt。公共后发射检查只重算最终对象
digest/length、LFCA revision 和 LFCA/LFSM/LFSD 跨对象 binding；它不重新运行完整
identity、topology、ownership、geometry、rule 或 semantic-diff 算法。LaneFlow 不拥有
内容寻址仓库或文件安装事务；宿主按需持久化/认证 LFCP 与 exact bytes。精确 API、错误与性能门槛见 [后发射检查][post-emission]。LFCP 线格式见 [LFCP 2][portable-lfcp]。验证矩阵见 [后发射检查 §11][post-emission-matrix]。

`CanonicalIdentityTable` 继续是 LFCA 不可裁剪的规范语义表，用于审计、诊断和后继工具；后发射检查只检查其登记形状、字段顺序、编码和直接值域，不逐实体重算 BLAKE3，也不建立第二份 collision registry。compiler 的 identity known vectors、碰撞处理和 LIR→LFCA 投影测试继续承担生产语义正确性。

后继职责：

- 共享修订的构造见 [共享静态路网][shared-static-network]；
- 切换绑定见 [修订切换][revision-cutover]；
- Runtime 不信任 LFSD 或调用方自报 revision；revision 来自受检 LFCA 及共享路网修订。



## 11. 版本、发布与供应链

契约版本轴（Contract Version Axes）：

```text
authoringFormatVersion
canonicalFormatVersion
canonicalPublicationDescriptorVersion
identityEncodingVersion
identityRegistryRevision
networkRevisionDerivationVersion
sourceMapFormatVersion
semanticDiffFormatVersion
networkRevisionCutoverDescriptorVersion
migrationPolicyVersion
executionConstraintVersion
partitionHintVersion
runtimeSnapshotVersion
constraintSetVersion
```

构建 provenance：

```text
compilerBuildId
```

内容身份与长度绑定（Content Identity and Length Bindings）：

```text
networkRevision
canonicalArtifactDigest
canonicalArtifactByteLength
sourceMapDigest
sourceMapByteLength
semanticDiffDigest
semanticDiffByteLength
```

- authoring/canonical 历史迁移离线完成；
- runtime exact-current/fail-closed，不在 production startup 迁移；
- 可移植制品的 exact bytes 与 digest 不可变；宿主发布方式不继承已被 ADR 0026 取代的
  schema publication 义务；
- `canonicalArtifactDigest`、`sourceMapDigest` 与 `semanticDiffDigest`
  均为各自 exact bytes 的 SHA-256，不使用 entity
  identity digest 代替；
- 每个上述 digest 必须由受认证 external descriptor/manifest 同时绑定同一对象的
  exact `u64` byte length；消费者在任何与输入大小成正比的读取、解压、分配、解析
  或 hash 前，先检查调用方上限、地址空间与 exact length，未知长度 stream 只允许
  checked length+1 bounded reader，不能用后验 digest mismatch 代替输入边界；
- `networkRevision` 是带 `networkRevisionDerivationVersion` 与 domain separator
  的规范语义载荷 SHA-256，不是 artifact exact-bytes digest，也不进入
  steady tick hash 计算；
- digest 只存放在其目标对象之外：artifact/source-map 均不把自己的
  digest 嵌回自身 bytes；publication manifest 完成外部绑定；
- compiler 与 format checker 的 provenance 必须可审计；
- LFCP v2 与后继 cutover descriptor 按各自职责绑定路网修订标识、exact LFCA/source
  map/diff、相应 exact byte length、constraint 和 tool build；
- `SharedNetworkRevision` 的 `CanonicalNetworkOrigin` 复制 LFCA digest/length、revision
  与必要静态契约版本，但不是可发布 descriptor，也不形成新的格式版本轴；
- runtime 不联网解析 schema、artifact 或 toolchain。

上述调用方上限不是不可信对象中的自报字段。约束集或宿主策略必须分别提供并在
任何线性工作前应用：

| 目标对象 | 长度字段                      | 调用方上限                  |
| -------- | ----------------------------- | --------------------------- |
| 规范制品 | `canonicalArtifactByteLength` | `maxCanonicalArtifactBytes` |
| 源映射   | `sourceMapByteLength`         | `maxSourceMapBytes`         |
| 语义差异 | `semanticDiffByteLength`      | `maxSemanticDiffBytes`      |

共享静态路网不是 exact-byte 对象；builder limits 直接约束 typed count、output logical/
retained bytes 与 scratch，不用可伪造的 LFCA 字段自报预算。

authoritative source module graph 是 authoring SSOT。Generated artifact 可以作为
release/CI artifact 或为特定治理阶段 checked in，但只能由 compiler 生成并经
后发射检查；宿主发布时再由外部认证 manifest 或 pinned digest 绑定。永远不允许手改或与
source graph 竞争 authority。

## 12. 性能架构

### 12.1 编译期

- arena + typed `u32` ordinal，stable sequence；
- XXH3 只作为 tuple/cache candidate fingerprint，命中后 full equality；
- parallel pass 按 deterministic shard/merge；
- incremental invalidation 以 source/module/identity closure 为边界；
- geometry tessellation 与 semantic validation 可并行，但 LIR freeze 单一稳定顺序。

### 12.2 运行时

- SoA/CSR/flat ranges；
- typed dense `u32` handles/ranges；
- precompiled candidate/occurrence/reverse indexes；
- `SharedTrafficNetwork`、`SharedIdentityIndex` 与 `PartitionPlanningHints` mandatory，
  `SharedSpatialNetwork` optional，diagnostics 外置；
- component owned continuous arrays 共享、mutable arrays per world；
- static execution constraints worker-count-neutral，最终执行计划 per world；
- 执行计划公开聚合诊断指标：阶段耗时（Phase Cost）、分区负载（Partition Load）、
  边界交换量（Boundary Exchange Volume）、屏障等待（Barrier Wait）、负载偏斜、
  迁移次数/成本、连接组件/SCC 分布、最大 SCC 提案占比、归约工作量
  （Reduction Work）、归约跨度（Reduction Span）与临界路径深度；不公开内部可变
  容器或调度正本；
- tick 不做 string/hash/path matching；
- `NetworkRevisionId` 只在加载、注册动态通行定义、快照恢复和修订切换等冷边界比较，
  不进入逐交通参与单元 fixed-tick 状态或每 tick hash；
- pose 不做 Traffic/Spatial join；
- production load 不做 JSON parse/registry rebuild。

### 12.3 开发阶段评审（Gate）

具体数字由实现证据在固定性能机上记录；#300 不在缺少最低产品硬件和完整 Runtime
kernel 时虚构绝对 SLA。Gate 至少覆盖：

- LFCA/LFSM/LFSD candidate exact bytes、count/build/closure 墙钟、累计分配、各 component logical/
  retained bytes、scratch peak，以及 builder 成功后不借用三对象；
- headless、facility-only、profile/frame-only、full lane Spatial 和当前最大合法产品场景，
  并分别记录发布构建的 LFCA + candidate arrays + scratch，及可编辑构建的 current 共享静态路网 +
  retained base LFCA + target LFCA/LFSM/LFSD + candidate 共享静态路网 + scratch 共存峰值；
- `SharedIdentityIndex` 双向 lookup 与至少一个 #301 production contiguous traversal
  kernel；
- 2/8/32 worlds 的 shared-static scaling；
- 单个大型 world 的 worker/partition scaling、barrier、边界交换、负载偏斜、
  连接组件/SCC 分布、最大 SCC 提案占比、归约工作量/跨度和临界路径；生产候选必须
  与集中式参考预言机（Reference Oracle）比较，并说明巨型组件下的有效并行度；
- 当前直接路径、显式参考精确路径（Reference Exact Path）、融合单 worker 与多 worker
  exact path 的阶段成本与端到端成本；融合优化必须通过状态/事件/错误等价证明，
  不能以省略逻辑阶段破坏确定性；
- 在线修订切换的准备期 tick 干扰、迁移日志字节/记录/落后量、追赶轮次、静默提交
  停顿、双修订峰值内存、放弃率和借用 token 退休延迟；维护暂停模式的完整停顿单列；
- 运行时快照保存/加载（Save/Load）的制品大小、墙钟耗时、主线程停顿、后台 tick 干扰、峰值
  内存、同修订回放与跨修订迁移；宿主应用存档场景不得只做功能等价；
- 已提交交通观测快照的完整/增量（Full/Delta）导出条目数、字节、频率、墙钟耗时、分配和 tick
  干扰，以及 Runtime 接收/验证动态成本快照和注册候选通行定义的成本；上层成本模型
  构造算法不归 LaneFlow 所有，但边界成本必须测量，不能默认每 tick 全量导出全网；
- 一万/十万 Traffic Runtime 与 Spatial 既有能力基线不得无解释回退；精确执行纪律
  的成本与 SoA/CSR、融合执行器等收益分别报告，不以 baseline 数字单独否决终态契约；
- 动态通行定义编译（Dynamic Traversal Compilation）不进入交通参与单元固定步进；
- 中国特色城市工作负载（Chinese-style City Workload）的拓扑/需求/运行时指标由
  后继 G1 建立，不能用 `LF-SYNTH-v2`、LuST 或多世界集合静默代替；
- target-specific SIMD/alignment 候选相对 portable/common layout 的收益。

不能用“BLAKE3/StableId128 可能变大”推导 tick 回退：ID 位于 cold/compiler boundary，
tick 只使用 32-bit dense handle。若身份索引 retained memory 成为问题，使用更紧凑
的双向索引解决；不得裁掉 snapshot/cutover
所需映射或缩短持久 identity。Source map、canonical tuple 与显示诊断仍可外置。

## 13. 包（Crate）与依赖目标

下图箭头统一表示“左侧 crate 依赖右侧 crate”：

```text
laneflow-format ---------> laneflow-static-contract
laneflow-static-network -> laneflow-static-contract
laneflow-static-network -> laneflow-format

laneflow-compiler -------> laneflow-static-contract
laneflow-compiler -------> laneflow-format

laneflow-runtime --------> laneflow-static-contract
laneflow-runtime --------> laneflow-static-network
laneflow-spatial --------> laneflow-static-contract
laneflow-spatial --------> laneflow-static-network

laneflow-adapter-* ------> laneflow-runtime
laneflow-adapter-* ------> laneflow-spatial
```

compiler 不依赖 JSON loader，也不建立迁移 crate、兼容特性或长期离线导入入口。

职责与禁止依赖：

中文术语与英文辅助名统一见 `../reference/glossary.md`；下表不重复定义双语映射。

| 包                         | 拥有职责                                                                                  | 禁止依赖                                     |
| -------------------------- | ----------------------------------------------------------------------------------------- | -------------------------------------------- |
| `laneflow-static-contract` | 稳定标识、路网修订标识、种类 / 标签登记表、有类型序号、共享静态契约版本                   | Serde、文件系统、核心 / 运行时、空间层       |
| `laneflow-format`          | 可移植制品线格式/视图、无分配后发射检查、revision/digest/跨对象 binding                   | 编译器来源/IR 语义、文件系统、运行时、空间层 |
| `laneflow-static-network`  | 受检 LFCA 构建闭合、typed dense arrays、Traffic/Identity/Hints/可选 Spatial、共享生命周期 | 编译器、Adapter、文件系统                    |
| `laneflow-compiler`        | 前端、中间表示、编译遍、主发射器、源映射 / 语义差异、LFCP v2 与发布编排                   | 已退役的数据 / 核心对象图                    |
| `laneflow-runtime`         | 固定步进、已实现执行域的交通参与单元、动态通行定义、可变交通状态                          | 编译器、Serde、文件系统、空间层              |
| `laneflow-spatial`         | 规范几何采样、位姿批次                                                                    | 编译器、引擎、Runtime                        |

`laneflow-runtime` 是现行动态运行时；`laneflow-static-network` 表达进程内共享静态
路网，不是 image/cache crate。共享 static contract 独立于 Runtime，避免
compiler/format 反向依赖动态运行时。

## 14. 风险登记

| 风险                          | 结果                                                          | 控制                                                                                                        |
| ----------------------------- | ------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------- |
| 编译器系统性缺陷              | 批量污染全部资产                                              | 人工可复核固定向量、compiler 语义测试、真实场景回归、历史缺陷断言；共享后端不承担独立发现全部语义缺陷的职责 |
| 格式/构建闭合漏洞             | 受检直接值域仍形成越界或错配 Runtime 关系                     | `laneflow-format` fuzz + builder 跨表/identity/range closure + 安全 Rust                                    |
| 发布来源与本地构建混同        | 本地候选被误写成已发布资产，或发布资产绕过 admission          | 发布 LFCP v2/manifest admission 与本地 `PostEmissionCheckedBundle` 分入口，同一 builder 后汇合              |
| 哈希前输入无界                | 超大替换资产制造无界读取、解压、分配或摘要工作                | 所有 exact-byte 对象绑定 digest + length；有界 reader 预检                                                  |
| 未认证路网修订                | 快照/路由绕过修订检查或兼容恢复误拒绝                         | 语义载荷派生标识、LFCP v2/外部 manifest 与 #302 可信切换输入                                                |
| 中间表示泄漏运行时类型        | 后端被特定运行时对象图锁死                                    | 静态契约、目标中立 LIR、无环包依赖图                                                                        |
| 标识漂移                      | 引用、语义差异、缓存和存档失效                                | 制品内规范身份表、compiler 固定向量/碰撞测试、变形测试                                                      |
| 源映射错配                    | 审阅或诊断静默指向旧来源文件 / 位置                           | 版本化封套；发布描述符绑定 exact artifact、revision、digest                                                 |
| 边身份耦合可选角色            | 未覆盖边无身份，或调整 overlay 造成伪删除 / 新增              | `LaneEdge` 独立稳定键；RoadSection/Junction 只保存关系                                                      |
| 增量 / 并行非确定性           | CI / 发布字节漂移                                             | 干净单线程预言机、稳定合并                                                                                  |
| Spatial 模式边界错误          | headless 仍分配几何，或 Traffic/Spatial 交叉索引漂移          | Traffic/Identity/Hints 必需、Spatial 可选矩阵与同修订 closure 测试                                          |
| 重新引入旧 JSON/Core 兼容路径 | 测试矩阵和语义漂移                                            | pre-1.0 单一正本；旧格式与实现只留 Git 历史                                                                 |
| 来源 / 生成物双重事实来源     | 手工修改与漂移                                                | 来源模块图正本、生成摘要、后发射检查与发布 manifest                                                         |
| 过早冻结内存布局/ABI          | Runtime kernel 优化被已发布格式锁定                           | 内部 SoA/CSR baseline；字段分组/SIMD 由 production kernel 实测演进，不发布 ABI                              |
| 最终分区进入共享静态路网      | 地图与硬件/世界耦合，存档不可移植                             | 静态约束 + 可重建提示 + 每世界执行计划                                                                      |
| 分区诱发行为延迟              | 结果随 cut 改变                                               | 同 tick committed-state barrier 与置换等价测试                                                              |
| 依赖连通导致归约跨度膨胀      | 巨型 SCC 或长凝聚 DAG 成为单大型 world 的阿姆达尔瓶颈         | SCC/DAG 分解；归约工作量/跨度指标；#220 production 研究                                                     |
| 精确路径脚手架成本            | 单 worker 相对当前直接路径显著回退                            | 融合精确执行器；四路径分项基准；语义等价 Gate                                                               |
| 原地修改共享静态数组          | 并行读取、共享、回滚和确定性失效                              | 不可变候选 + 失败关闭修订切换事务                                                                           |
| 在线迁移候选过期              | 切换遗漏准备期间的 tick/命令或重复事件                        | 有界增量日志、追赶、静默提交、切换事件批次恰一次                                                            |
| 迁移准备干扰正常步进          | 玩家改路导致持续抖动或延迟尖峰                                | 后台预算、落后量/干扰 Gate、显式维护暂停模式                                                                |
| 每次加载 LFCA 重建成本过高    | 最低产品硬件的加载或确认建造停顿不可接受                      | 后台候选、阶段计时/峰值证据；若真实测量阻断则以独立 G1 设计非正本缓存                                       |
| 构建模式裁掉稳定身份索引      | 快照、路线与跨修订映射无法恢复                                | 所有模式必需冷索引与双向 round-trip                                                                         |
| 稳定身份索引替代语义证据      | 同 StableId128 的新语义错误继承旧占用、路线、预约或控制器状态 | 受信任 semantic diff；索引只复核映射；index-only 失败关闭                                                   |
| 未受信任语义差异驱动迁移      | 篡改迁移、错误终止或状态错配                                  | 外部切换描述符、双制品独立验证、身份索引复核                                                                |
| 通行权运行时能力缺口          | 静态契约与运行时执行能力长期不对称                            | 明示当前能力边界；每项能力以独立跨层测试闭合                                                                |

[routing-integration]: traffic-observation-and-routing-integration.md
[adr-0033]: ../adr/0033-digest-role-and-algorithm-selection-authority.md
[compiler-foundation-frontends]: compiler-foundation.md#33-官方前端共同受检模块接入
[post-emission]: compiler-post-emission-check-and-minimal-publication-closure.md

[portable-lfcp]: portable-canonical-artifact.md#6-lfcp-2-与发布闭合
[portable-lfsm]: portable-canonical-artifact.md#4-lfsm-4
[portable-lfsd]: portable-canonical-artifact.md#5-lfsd-4
[post-emission-lfcp]: compiler-post-emission-check-and-minimal-publication-closure.md#8-lfcp-v2
[post-emission-matrix]: compiler-post-emission-check-and-minimal-publication-closure.md#11-验证矩阵
[revision-cutover]: traffic-runtime-revision-cutover.md
[runtime-snapshot]: traffic-runtime-snapshot.md
[shared-static-network]: shared-static-network.md
