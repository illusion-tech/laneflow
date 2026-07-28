# 路网编译器与目标静态镜像

**文档状态**: Draft（#291 G1 综合架构修订）<br>
**最后更新**: 2026-07-29<br>
**适用范围**: 权威来源模块图（Authoritative Source Module Graph）、有类型中间表示
（Typed IR）、静态网络编译权威、标识派生、可移植规范制品（Portable Canonical
Artifact）、目标静态镜像（Target Static Image）、源映射（Source Map）、语义差异
（Semantic Diff）、独立验证器（Independent Validator）、交通运行时（Traffic
Runtime）命名、静态执行约束（Static Execution Constraints）、不可变路网修订
（Immutable Network Revision）和当前态（Current）→目标态（Target）迁移<br>
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
- `../adr/0021-city-simulation-game-traffic-foundation.md`
- `core-id-handles.md`
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

#291 G1 候选目标不是“生成另一份 JSON 的工具”，而是新的静态数据体系：

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
                    │                    + RuntimeExecutionPlan
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
- 编译器保存 worker 数无关的静态执行约束，运行时按世界建立实际执行计划；最终
  partition/worker assignment 不进入稳定标识、可移植制品语义或共享镜像。
- 静态镜像是一个不可变路网修订，而非城市永久不变；道路编辑通过新修订与失败关闭
  镜像切换事务进入运行世界。
- Proposed ADR 0021 提议把服务中国特色城市模拟游戏交通基础定义为 LaneFlow 第一
  长期产品目标，并让城市经济、出行需求、路线选择策略和游戏规则继续由上层拥有。

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
- 静态资源依赖、连接资源组件、可切分边界和规范合并键；
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

本表仍是 #291 G1 前的 Proposed v1，尚无已接受 known vector、规范制品或生产 reader；
因此本次统一 `LaneEdge` 身份并重排代码 / 标签是在首次冻结前修正 v1 定义，不是对已
发布 v1 的原地兼容修改。G1 接受并发布首批 known vectors 后，新增 kind 才只能提升
registry revision，修改既有 kind 的字段、标签含义或编码必须提升 encoding version。

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
|           15 | `ParkingSpace`     | 声明（Declaration）                           | `parking-space`     | `1,23,24`                 |
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

所有真实父子关系都使用父实体 `StableId128`，不得把父实体仅在其来源模块内稳定的
裸局部键复制进子实体 tuple。这样跨模块引用、同名父实体和重新归属都由父实体完整
命名空间裁决：

- `RoadSection` 使用 tag 33 `roadCorridorStableId`；
- `AuthoringLane` 使用 tag 32 `roadSectionStableId`；
- `LaneEdge` 不使用 parent anchor，只使用来源模块内显式持久化且唯一的 tag 5
  `laneEdgeKey`；
- `Movement` 使用 tag 34 `junctionStableId`；
- `ManeuverPath`、Signal phase、ParkingSpace、LaneGroup 和 FacilityBand 继续使用
  各自登记的 parent StableId。

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
- 当前 Traffic v0.10 导入前端对**全部** `laneGraph.edges[].id`（不是只对未覆盖边）
  原样使用其符合 External ID 约束的文本作为 `laneEdgeKey`，并置于导入模块的稳定
  namespace；因此道路区段覆盖、路口内部角色、`loop`、`isolated` 和被静态路线引用
  的未覆盖边都进入同一身份规则；
- current `RoadSectionData.lanes[].edgeIds` 和派生 internal-edge ownership 只建立
  关系；importer 不得因角色存在与否把同一 current edge 分派到不同 identity kind；
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
|          23 | `parkingAreaStableId`      | 16 个原始字节（Raw Bytes） |
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

compiler 维护 `StableId128 -> CanonicalIdentity + owning span` 登记。两个 stable
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
- compiler、independent validator 和至少一个独立语言/脚本 oracle 的 bytes/ID
  一致性。

## 8. 制品与源映射（Artifact and Source Map）

### 8.1 可移植规范制品（Portable Canonical Artifact）

平台无关、确定性、closed shape，并包含：

- canonical format、identity encoding/registry、network revision derivation 与
  constraint versions；
- 路网修订标识（Network Revision ID）`NetworkRevisionId`；
- logical entities、typed ordinals、normalized numeric values；
- topology/geometry/static rule relations；
- canonical payload envelope 与 compiler provenance；自身 digest 不嵌入 artifact
  bytes。

它用于 publication、长期审计、migration、跨实现 validator 和 static image
regeneration。它不是 mmap hot layout，不承诺与 Rust struct ABI 相同，也不因某个
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
encoding/registry、constraint/execution-constraint versions，以及全部目标无关的
拓扑、几何、静态规则、规范 relation 和静态执行约束；不包含该摘要自身、artifact
envelope、compiler/validator provenance、source map/diagnostics、publication
metadata、target、profile 或 image layout。这样相同规范语义在不同 compiler
provenance 与 target/profile image 中保持同一修订，而任何运行时可观察静态语义或
参与派生的契约版本变化都会产生新修订。Compiler 在 LIR freeze 后计算该字段；
independent validator 必须从 artifact semantic payload 独立重算并逐字节比较，
不能信任 artifact 内自报的值。`canonicalArtifactDigest` 仍认证完整 artifact exact
bytes，二者不得互相替代。
若 publication 或 cutover 同时观察到相同 `NetworkRevisionId` 对应不同
`canonicalNetworkSemanticPayload` exact bytes，必须以
`NetworkRevisionDigestCollision` 失败关闭；不得追加 ordinal、随机 salt 或 suffix。

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
    static execution constraint graph
  Required: StaticIdentityIndex
    stable-entity ordinal -> StableId128
    sorted (entity kind, StableId128) -> typed ordinal
  Required: PartitionPlanningHints
    rebuildable cost / boundary / recommended-cut hints
  Optional: StaticSpatialImage
    frame/edge-aligned geometry tables
    flat points / cumulative arc / sampling ranges
  Optional: WarmQueryTables
  Optional: ColdDiagnostics
```

v1 profile 是版本化 closed set，不允许调用方任意拼 feature bits：

| `staticImageProfileId` | 必需节（Required Sections）                                                                                                       | 用途                                              |
| ---------------------- | --------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------- |
| `traffic-headless-v1`  | `StaticTrafficImage`, `StaticIdentityIndex`, `PartitionPlanningHints`                                                             | 服务器（Server）、测试、无图形宿主                |
| `traffic-spatial-v1`   | `StaticTrafficImage`, `StaticIdentityIndex`, `PartitionPlanningHints`, `StaticSpatialImage`                                       | 引擎适配器（Adapter）、规范位姿（Canonical Pose） |
| `traffic-debug-v1`     | `StaticTrafficImage`, `StaticIdentityIndex`, `PartitionPlanningHints`, `StaticSpatialImage`, `WarmQueryTables`, `ColdDiagnostics` | 编辑器（Editor）、诊断、调试绘制（Debug Draw）    |

设计约束：

- `StaticTrafficImage` 可独立验证和挂载；headless profile 不携带 geometry；
- `StaticIdentityIndex` 是所有生产配置档必需、但不进入 tick 的共享冷索引：对每个稳定
  declaration/addressable-derived 提供 typed ordinal → StableId128 的正向表，以及按
  `(entityKind, StableId128)` 排序的 StableId128 → typed ordinal 反向表；它服务
  snapshot save/load、dynamic Route 重建和 network-revision cutover；
- 身份索引可以独立分页、按需映射或在 steady tick 期间不驻留 CPU cache，但不得与
  `ColdDiagnostics` 一起从 production profile 裁掉；
- `sectionMask` 必须与 profile 的 closed section set 精确匹配；缺失、额外或未知
  section 均 fail closed，调用方不能通过 feature bits 组合新 profile；
- Spatial section 存在时必须完整覆盖 v1 所需 edge，并与 Traffic 共享 canonical edge
  ordinal；v1 不引入 sparse Spatial mapping；
- shared immutable bytes，多 `TrafficWorld`/Spatial session 复用；
- 静态执行约束对工作线程数中立；v1 配置档保留提示节以让镜像字节闭合确定，
  运行时可以忽略或重新派生提示，但不得保存最终分配；
- hot/warm/cold 分段，profile 可裁剪 Spatial/diagnostic/debug data，但不能裁剪
  恢复和切换所需的 `StaticIdentityIndex`；
- 顶层 section byte offset/length 使用 checked `u64`；table row ordinal、count 和
  hot relation range 使用 checked `u32`，不保存原生指针；
- verifier 完成后 view 的高频索引是 O(1) 或连续 range traversal；
- schema/Serde/object graph 不是 static image ABI；
- 具体 archive/zero-copy library 由安全审计和 benchmark 决定。

相同 portable artifact 的不同 target/profile variant 共享
`canonicalArtifactDigest` 与 `NetworkRevisionId`，但各自拥有不同
`staticImageDigest`。Image header 必须声明 `networkRevisionDerivationVersion` 与
`networkRevision`，但这些字段和 header 中的其他 provenance 一样只供外部描述符
核对，不能自证可信。Traffic Runtime 的
构造入口只接收从 `TrustedStaticImage` 拆出的 `StaticTrafficView`，不能要求调用方
同时提供 Spatial section，也不能接受仅结构验证所得的 view。

### 8.3 外部镜像描述符（External Image Descriptor）

生产 fast path 的 trust anchor 必须位于 image bytes 之外。版本化
`StaticImageDescriptor` 至少绑定：

```text
staticImageDescriptorVersion
networkRevisionDerivationVersion
networkRevision
canonicalArtifactDigest
staticImageDigest
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
validationReceiptDigest
```

descriptor 可以由签名 publication manifest、宿主已认证 asset/package manifest 或
应用内 pinned digest 提供。Image header 内的
`networkRevision`/`canonicalArtifactDigest`/target/provenance 只是待核对声明；
攻击者可以伪造它们并对任意 image bytes 计算新的 `staticImageDigest`，因此不能
独立建立 semantic trust。

Trusted descriptor 的签发前置条件是：portable artifact 已通过 independent
semantic validator，且不复用 compiler emitter 的 independent image rebuild 在相同
target/layout/profile 下产生相同 exact bytes digest。Validation receipt 必须记录
这两项成功证据，并绑定 independent validator 重算所得的
`networkRevisionDerivationVersion + networkRevision + canonicalArtifactDigest`。
`TrustedStaticImage` 只从已认证 descriptor 获得修订标识；调用方参数、image header
或未验证 artifact 均不能另行指定运行时当前修订。

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

Semantic diff 必须绑定
`baseNetworkRevisionDerivationVersion + baseNetworkRevision +
baseCanonicalArtifactDigest` 与
`targetNetworkRevisionDerivationVersion + targetNetworkRevision +
targetCanonicalArtifactDigest`。Base 必须是已通过 independent validator 的
portable artifact；无 baseline 时使用显式 genesis marker，并把全部 stable entity
和 owner-local sequence 报告为新增。重复 relation value 的序列对齐按最低
before/after `localIndex` 确定性破同值，diff 本身不获得 authoring authority。

PR/治理消费可以直接展示经过验证的 semantic diff；运行时镜像切换不得把 compiler
输出的裸 diff 当作迁移权威。若 diff 参与状态迁移，independent validator 必须对
base/target portable artifact 独立重算或验证其完整语义，并由 §9.6 的外部可信切换
描述符绑定 exact diff bytes digest。缺少该绑定时，diff 只能作为诊断提示，运行时
必须从两个可信 `StaticIdentityIndex` 和已冻结迁移策略重新证明映射。

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

### 9.6 不可变路网修订与镜像切换

目标静态镜像代表一个路网修订。结构性道路编辑重新进入权威来源模块图、增量编译、
独立验证和外部信任绑定，生成新修订；共享镜像不得原地 mutation。

运行世界只在显式 fixed-tick 安全边界执行失败关闭的镜像切换事务。未变化实体通过
旧/新可信镜像的 `StaticIdentityIndex` 重建 StableId128 ↔ typed ordinal 映射；
删除、重接或语义改变的网络元素必须按受信任语义差异迁移或终止其交通参与单元、
动态通行定义、停驻/预约和控制器状态。当前道路机动车执行域仍具体表现为车辆、
动态路线和停车。任一完整性条件无法证明时，旧修订
继续生效。临时封闭等不改变静态身份/拓扑的状态由后续 G1 冻结的 runtime
overlay/command 承担。

运行时若消费语义差异，image 外部的版本化
`NetworkRevisionCutoverDescriptor` 至少绑定：

```text
networkRevisionCutoverDescriptorVersion
baseNetworkRevisionDerivationVersion
baseNetworkRevision
baseCanonicalArtifactDigest
baseStaticImageDigest
targetNetworkRevisionDerivationVersion
targetNetworkRevision
targetCanonicalArtifactDigest
targetStaticImageDigest
semanticDiffDigest
migrationPolicyVersion
validatorBuildId
validationReceiptDigest
```

该描述符必须来自签名 publication manifest、宿主认证资产清单或 pinned digest；
validation receipt 必须证明 independent validator 已针对两个 portable artifact
重算并验证各自路网修订标识，且已验证或重算语义差异。描述符中的 base/target
修订、制品摘要和镜像摘要必须分别与两个可信静态镜像的 descriptor 精确相等。
Runtime 仍须用两个 `StaticIdentityIndex` 核验每个稳定实体映射，不能让 diff 中的
ordinal、数组位置或 compiler 私有顺序成为迁移权威。

切换采用准备（Prepare）→提交（Commit）→回收（Retire）：

- 在 tick 外完成新 image trust/structure 检查、容量检查、状态迁移与候选世界构造；
- 在 fixed-tick 安全边界只原子切换 image/state binding；
- 旧 image 在全部借用快照、Pose batch、Adapter token/epoch 退出后回收；
- 任一准备或提交失败都保持旧 image/state，不暴露半迁移结果。

### 9.7 运行时快照、存档与回放

运行时快照与共享 image 分离，并至少绑定：

```text
canonicalArtifactDigest
originStaticImageDigest
runtimeSnapshotVersion
runtimeVersion
constraintSetVersion
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

`canonicalArtifactDigest` 与版本化 `networkRevision` 是同修订恢复的静态语义权威；
`originStaticImageDigest` 记录创建快照时的精确 target/profile image，供审计和
同镜像快速恢复。恢复可以改用另一个已认证 target/profile image，但仅当它绑定相同
canonical artifact、network revision、identity/constraint versions，且
`StaticIdentityIndex` 能完整重建全部稳定静态引用时；否则失败关闭。该规则使快照
不依赖 target-specific dense ordinal，同时仍保留原镜像的可审计来源。
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
拥有全局成本政策。路径规划/出行编排层结合静态网络、观测、收费、游戏政策与偏好
构造版本化动态成本快照，返回可由 Runtime 注册/验证的候选通行定义；不得在每个
交通参与单元的 fixed-tick 内全图寻路。当前车辆域使用 Route，未来执行域由其 G1
冻结等价通行定义。成本快照和候选通行定义必须绑定路网修订、观测 tick 与成本模型
版本；路网修订标识必须来自 `TrustedStaticImage` 派生的 Runtime 静态视图或已
提交交通观测快照，不能由 Routing 调用方自报。Runtime 将其与当前可信 image binding
做精确相等比较，并继续逐项验证候选稳定引用/拓扑；修订标识相等只证明缓存一致性，
不替代候选内容验证。Runtime 对 revision mismatch 失败关闭，允许多旧的观测由
Routing G1 冻结。
出行需求和路线选择策略属于城市游戏或出行编排层。#291 只冻结该职责边界，不提前
冻结 `laneflow-routing` crate、算法或公共 API。

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
  forged `networkRevision`、tampered descriptor/receipt 和 wrong-profile
  rejection；
- independent validator 重算路网修订标识、同一 artifact 的跨 target/profile
  修订标识相等，以及任一静态语义/派生版本变化的修订标识不等；
- `traffic-headless-v1` 无 Spatial bytes、但包含完整 `StaticIdentityIndex` 的
  TrafficWorld smoke/equivalence；
- `traffic-spatial-v1` Traffic/Spatial edge ordinal 与 full-coverage property tests；
- 所有 production profile 的 typed ordinal ↔ StableId128 round-trip、snapshot
  restore 与 dynamic Route rebuild；
- source map completeness 与 diagnostic stability；
- semantic diff golden tests，以及 forged/tampered diff、错误 base/target digest、
  未受信任 cutover descriptor 的拒绝测试；
- current JSON path 与 target image path behavior/determinism/pose equivalence；
- worker 数/partition plan 置换下 committed state、event 与确定性状态摘要等价；
- 分区边界不引入额外一 tick 延迟，连接资源组件保持唯一规范归约权威；
- 新路网修订验证、镜像切换失败原子性和稳定标识迁移；
- 镜像切换准备阶段双修订峰值内存、提交停顿、失败回滚和旧 token 延迟回收；
- snapshot save/load、same-revision replay、cross-revision rejection/migration 与
  desynchronization diagnostics；
- startup wall time、peak allocation、retained static memory；
- multi-world shared-static memory；
- 一万/十万 Traffic Runtime tick 与 Spatial pose；

## 11. 版本、发布与供应链

独立版本轴：

```text
authoringFormatVersion
canonicalFormatVersion
identityEncodingVersion
identityRegistryRevision
networkRevisionDerivationVersion
staticImageLayoutVersion
staticImageProfileId
staticImageDescriptorVersion
semanticDiffFormatVersion
networkRevisionCutoverDescriptorVersion
migrationPolicyVersion
executionConstraintVersion
partitionHintVersion
runtimeSnapshotVersion
constraintSetVersion
compilerBuildId
validatorBuildId
targetTriple
networkRevision
canonicalArtifactDigest
staticImageDigest
semanticDiffDigest
validationReceiptDigest
```

- authoring/canonical 历史迁移离线完成；
- runtime exact-current/fail-closed，不在 production startup 迁移；
- portable artifact immutable publication 继承 ADR 0011；
- static image 可按同一 canonical digest 产生多个 target/profile variant；
- `canonicalArtifactDigest`、`staticImageDigest`、`semanticDiffDigest` 与
  `validationReceiptDigest` 均为 exact bytes 的 SHA-256，不使用 entity identity
  digest 代替；
- `networkRevision` 是带 `networkRevisionDerivationVersion` 与 domain separator
  的规范语义载荷 SHA-256，不是 artifact/image exact-bytes digest，也不进入
  steady tick hash 计算；
- digest 只存放在其目标对象之外：artifact/image/receipt 均不把自己的 digest
  嵌回自身 bytes；publication manifest/external descriptor 完成外部绑定；
- compiler、validator、image builder 的 provenance 必须可审计；
- external descriptor/receipt 必须绑定路网修订标识、exact artifact、image、
  target、profile、constraint 和 tool builds；
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
- Traffic 与 `StaticIdentityIndex` mandatory，Spatial/diagnostic/debug
  profile-controlled；
- immutable image 共享、mutable arrays per world；
- static execution constraints worker-count-neutral，最终执行计划 per world；
- 执行计划公开聚合诊断指标：阶段耗时（Phase Cost）、分区负载（Partition Load）、
  边界交换量（Boundary Exchange Volume）、屏障等待（Barrier Wait）、负载偏斜、
  迁移次数/成本；不公开内部可变容器或调度权威；
- tick 不做 string/hash/path matching；
- `NetworkRevisionId` 只在加载、注册动态通行定义、快照恢复和修订切换等冷边界比较，
  不进入逐交通参与单元 fixed-tick 状态或每 tick hash；
- pose 不做 Traffic/Spatial join；
- production load 不做 JSON parse/registry rebuild。

### 12.3 开发闸口（Gate）

具体数字由实现 G1 在固定性能机上用 current baseline 冻结，但 Gate 至少覆盖：

- load latency、peak allocation、retained bytes，以及 `StaticIdentityIndex` 的共享
  retained bytes、按需映射延迟和双向 lookup latency；
- 2/8/32 worlds 的 shared-static scaling；
- 单个大型 world 的 worker/partition scaling、barrier、边界交换和负载偏斜；
- 镜像切换的准备/提交/回收耗时、双修订峰值内存与借用 token 退休延迟；
- 一万/十万 Traffic Runtime 与 Spatial 既有上限不得回退；
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
| `laneflow-static-contract` | 稳定标识、路网修订标识、种类 / 标签登记表、有类型序号、版本 / 摘要 / 配置档 / 描述符值              | Serde、文件系统、核心 / 运行时（Core / Runtime）、空间层（Spatial） |
| `laneflow-format`          | 可移植制品（Portable Artifact）与发布 / 描述符线格式 / 视图（Publication / Descriptor Wire / View） | 编译器语义遍、运行时（Runtime）、空间层（Spatial）                  |
| `laneflow-static-image`    | 镜像 ABI、节 / 配置档、有界结构校验器（Bounded Verifier）、借用视图（Borrowed Views）               | 编译器、验证器、运行时（Runtime）、空间层（Spatial）                |
| `laneflow-compiler`        | 前端、中间表示、编译遍、主发射器、源映射 / 语义差异                                                 | 当前态数据 / 核心对象图（Current Data / Core Object Graph）         |
| `laneflow-validator`       | 独立制品语义（Independent Artifact Semantics）与镜像重建 / 预言机（Image Rebuild / Oracle）         | 编译器验证实现（Compiler Validation Implementation）                |
| `laneflow-runtime`         | 固定步进、已实现执行域的交通参与单元、动态通行定义、可变交通状态                                    | 编译器、验证器、Serde、文件系统、空间层（Spatial）                  |
| `laneflow-spatial`         | 规范几何采样（Canonical Geometry Sampling）、位姿批次（Pose Batch）                                 | 编译器、验证器、引擎                                                |

`laneflow-runtime` 是 current `laneflow-core` 的 target 名称；
`laneflow-static-image` 取代含混的 `laneflow-runtime-image` 名称。共享 static
contract 不能继续留在 Runtime，否则 compiler/validator 会反向依赖动态运行时。
`laneflow-data` 是 current JSON compatibility façade，cutover 后不再拥有静态
normalization authority。

## 14. 迁移路线

```text
阶段 0  current JSON/Data/Core/Spatial 路径继续生产服役
阶段 1  #291：ADR 0020/0021 + 本设计完成 G1
阶段 2  #292：static-contract + compiler foundation + Synthetic DSL frontend 纵向闭环
阶段 3  integration-only LIR→current projection 支撑 #282–#285 等价验证
阶段 4  Geometry document frontend + topology/geometry MIR（可与阶段 3 并行）
阶段 5  portable artifact + independent validator + source map/semantic diff
阶段 6  target static image + Traffic Runtime/Spatial shared image path
阶段 7  behavior/perf/security cutover Gate
阶段 8  #294：production cutover，完成 core→runtime rename 并移除 projection/重复构建
```

阶段是架构迁移顺序，不是把终态降级为最小方案。每个阶段都必须沿同一个
AST/HIR/MIR/LIR 与 artifact/image contract 前进，不允许先建一个注定废弃的 Core
builder API。阶段 3 的 bridge 固定为 `laneflow-compiler-test-support` 或等价
integration-only crate：它可以依赖 compiler + current Core/Spatial，将 validated
LIR 投影为 current inputs；compiler 不依赖它，它不构成 public backend contract，
并由阶段 8 的 cutover owner #294 删除。该投影器（Projection）只消费 LIR 已验证的
StableId、有类型所有者关系（Typed Owner Relation）与其他规范语义，不从当前 JSON
或核心对象图（Core Object Graph）反向派生标识（Identity）；未来当前 JSON 的迁移 /
导入前端（Migration / Import Frontend）也必须先产生并验证同一唯一所有者关系，
才能进入 LIR。

阶段 8 的 #294 一次性不兼容改名不仅覆盖 crate/type，也覆盖文档导航、Agent Skill
ID、工具薄包装和治理枚举：`laneflow-core-design` 目标改为
`laneflow-runtime-design`。在 current `laneflow-core/CoreWorld` 仍服役时只同步
Skill 内容并明确 current/target，不提前删除旧发现入口；具体治理枚举迁移必须由
独立 implementation Issue 原子更新 validator、模板和历史兼容规则。

Cutover 前必须证明：

1. current/target 场景的静态语义、tick、event、pose 等价；
2. deterministic artifact/image；
3. independent validator、validation receipt、external descriptor 与 bounded
   structural verifier 安全；
4. startup、memory、一万/十万和 multi-world Gate；
5. worker/partition 置换等价、无额外 tick 延迟和单大型 world scaling；
6. publication/migration/source map/semantic diff 可用；
7. snapshot/replay 与 network revision cutover 有失败关闭的后继 G1/实现入口；
8. fallback/rollback 只切换 current/target asset path，不存在两套可变 authority。

## 15. 风险登记

| 风险                                                                         | 结果                                             | 控制                                                         |
| ---------------------------------------------------------------------------- | ------------------------------------------------ | ------------------------------------------------------------ |
| 编译器系统性缺陷（Compiler Systemic Bug）                                    | 批量污染全部资产                                 | 独立验证器、差分 / 模糊测试（Differential / Fuzz）、语义差异 |
| 二进制校验器漏洞（Binary Verifier Vulnerability）                            | 不可信字节破坏内存安全                           | 基于偏移量的格式、加载限制、模糊测试 / `unsafe` 审计         |
| 镜像头声明被误当作信任（Header-as-Trust）                                    | 恶意但结构合法的镜像绕过语义闸口                 | 外部描述符、验证收据、不可信重建                             |
| 未认证路网修订（Unauthenticated Network Revision）                           | 快照/路由绕过修订检查或兼容恢复误拒绝            | 语义载荷派生标识；descriptor/receipt/cutover 三重绑定        |
| 中间表示泄漏运行时类型（IR Leaks Runtime Types）                             | 后端 / 目标被当前核心对象图锁死                  | 静态契约、目标中立 LIR、无环包依赖图                         |
| 标识漂移（Identity Drift）                                                   | 引用、语义差异、缓存和存档失效                   | 精确种类 / 标签登记表、已知向量、变形测试                    |
| 边身份耦合可选角色（Edge Identity Coupled to Optional Role）                 | 未覆盖边无身份，或调整 overlay 造成伪删除 / 新增 | `LaneEdge` 独立稳定键；RoadSection/Junction 只保存关系       |
| 增量 / 并行非确定性（Incremental / Parallel Nondeterminism）                 | CI / 发布字节漂移                                | 干净单线程预言机、稳定合并                                   |
| 配置档边界错误（Profile Boundary Error）                                     | 无图形配置档携带几何，或交叉索引漂移             | 交通必需 / 空间可选矩阵、配置档测试                          |
| 当前态 / 目标态双路径长期化（Current / Target Dual-path Permanence）         | 测试矩阵和语义漂移                               | 集成专用桥、明确移除责任人 / 切换闸口                        |
| 来源 / 生成物双重事实源（Source / Generated Dual SSOT）                      | 手工修改与漂移                                   | 来源模块图权威、生成摘要 / 收据闸口                          |
| 过早选择归档库（Premature Archive-library Choice）                           | ABI、安全或 MSRV 锁定                            | 先冻结契约，再做基准 / 审计                                  |
| 最终分区进入共享镜像（Final Partition in Shared Image）                      | 地图与硬件/世界耦合，存档不可移植                | 静态约束 + 可重建提示 + 每世界执行计划                       |
| 分区诱发行为延迟（Partition-induced Behavioral Delay）                       | 结果随 cut 改变                                  | 同 tick committed-state barrier 与置换等价测试               |
| 原地修改静态镜像（In-place Static-image Mutation）                           | 摘要、共享、信任和确定性失效                     | 不可变路网修订 + 失败关闭镜像切换事务                        |
| 生产配置档裁掉稳定身份索引（Production Profile Drops Stable Identity Index） | 快照、动态路线与跨修订映射无法恢复               | 全配置档必需冷索引；双向 round-trip；按需映射                |
| 未受信任语义差异驱动迁移（Untrusted Semantic Diff Drives Migration）         | 篡改迁移、错误终止或状态错配                     | 外部切换描述符、双制品独立验证、身份索引复核                 |
| 通行权运行时交付延期（Right-of-way Runtime Delivery Deferral）               | 静态契约与运行时执行能力长期不对称               | 明示当前能力边界；#292 G4 后恢复 #282–#285；#285 跨层闭环    |

## 16. #291 G1 完成条件

- ADR 0020/0021 明确历史 ADR 的继续有效、扩展与取代范围；
- 本文不再存在 L1/L2 或 Core-shaped compiler IR；
- source module graph、标识 registry/encoding、trust/load path、static-image
  profile 与 crate DAG 均为 closed contract；
- Data/current Core/target Traffic Runtime/Spatial/Adapter 文档清楚标注 current 与
  target；
- #292 已重划为 compiler foundation + Synthetic DSL frontend，并继续保持
  `Blocked by #291`；
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
- 外部 clean re-review 无未解决 Major，G1 才可推进。
