# ADR 0020：编译器拥有静态路网与目标静态镜像

**状态**: Proposed（#291 G1 修订输入）<br>
**日期**: 2026-07-28<br>
**适用范围**: 权威来源模块图（Authoritative Source Module Graph）、编译器中间表示
（Compiler IR）、静态路网权威、可移植规范制品（Portable Canonical Artifact）、
目标静态镜像（Target Static Image）、验证收据（Validation Receipt）、源映射
（Source Map）、独立验证器（Independent Validator），以及当前态 Core / 目标态
交通运行时（Traffic Runtime）、Data、Spatial 初始化边界<br>
**目标取代范围**: ADR 0005、0007、0008、0011、0013、0015、0017 中与静态数据 normalization、制品配对和运行时 registry 构建位置冲突的条款，并把 target `LaneFlow Core`/`laneflow-core` 重命名为 `LaneFlow Traffic Runtime`/`laneflow-runtime`；在本 ADR Accepted 且迁移 G4 前，当前生产实现继续由原 ADR 约束<br>

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
  - `0017-static-road-junction-maneuver-and-gate-identity.md`
- 配套决策:
  - `0021-city-simulation-game-traffic-foundation.md`
- 详细设计:
  - `../design/network-compiler.md`
  - `../design/data-format.md`
  - `../design/data-loading.md`
  - `../design/spatial-geometry.md`
  - `../design/core-id-handles.md`
  - `../reference/glossary.md`
- GitHub:
  - #291
  - #292

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

#291 的目标不是在现有路径旁增加一个生成工具，而是把全部静态网络语义前移到
离线编译期，并让 target Traffic Runtime 只从具有外部信任锚的派生静态镜像建立
只读 view。

ADR 0021 进一步明确：LaneFlow 的第一长期产品目标是为未来的中国特色城市模拟
游戏提供交通基础。#291 因此不仅要优化加载，还必须保留单个大型城市世界的并行
扩展、玩家修改道路、存档/回放、路径规划接入与每世界唯一性演进空间，同时不把
城市经济和出行需求塞入交通运行时。

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

Target `LaneFlow Traffic Runtime` 继续拥有运行时交通规则、车辆和动态 Route
lifecycle、tick、安全约束与 runtime state authority；Spatial 继续拥有 canonical
geometry sampling 和 pose 语义；二者不再各自重新解释或重建静态网络。Current
production 的 `LaneFlow Core` 命名与 API 在 cutover 前继续有效。

### 2. 权威来源模块图（Authoritative Source Module Graph）组合平级前端（Frontend），不采用 L1/L2

一个 compilation unit 的唯一 authoring authority 是显式、可重放的 source module
graph，而不是 Geometry 这一种格式。每个 module 必须绑定稳定 namespace、source
language/content digest、frontend version/options、origin/provenance 与 imports。
下列输入能力是可组合 frontend/source：

- **Synthetic DSL frontend**：测试、benchmark、示例和程序化场景；
- **Geometry document frontend**：production 的主要 authoring language；
- **Import frontend**：OSM、外部工具和离线 migration；
- **Editor authoring surface**：默认持久化 Geometry module；只有定义独立可重放
  source format 时才成为独立 frontend。

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

每一阶段只由前一阶段构造；公共 pass 以显式输入/输出和结构化诊断运行。LIR 是
所有后端的唯一输入。后端不得补充语义默认、重新推断 topology 或修复缺失关系。

具体 Rust collection、arena library 或序列化库不在本 ADR 冻结；必须通过独立
实现基准选择。但 IR 要求：

- arena/dense ordinal 可用 `u32` 表达，超界时编译失败；
- stable declaration/addressable-derived 才有 StableId128；owner-local relation /
  occurrence 使用只在 owning sequence snapshot 内有效的 typed
  `(ownerOrdinal, role, localIndex)` key，不获得全局持久标识；
- iteration order 与输出不依赖 hash table iteration；
- 增量 cache key 只影响重用，不影响完整编译结果；
- 并行 pass 必须以稳定 merge order 产生与单线程逐字节相同的输出。

标识编码封装（Identity Envelope）与实体登记表（Entity Registry）使用独立版本
轴。`identityEncodingVersion = 1`
冻结 magic、kind、field count、strictly-increasing tag/length/value bytes；
`identityRegistryRevision = 1` 必须覆盖 current target 的 topology、Gate/Waiting、
Signals/Phase、Parking、cross-section/access/profile、static Route 与 canonical
frame declaration。新增 kind 只 append registry revision；修改既有 kind 的字段
集合、tag 含义或编码必须提升 encoding version。完整 kind/tag/required-sequence
表以 `network-compiler.md` 为规范。

### 4. 一个规范低层中间表示（Canonical LIR）产生四类配套输出

一次成功编译原子地产生：

1. **可移植规范制品（Portable Canonical Artifact）**：平台无关、确定性、可发布和长期审计的静态网络
   事实；承接 public artifact、迁移和跨实现互操作；
2. **目标静态镜像（Target Static Image）**：按目标平台、布局版本、封闭配置档
   （Closed Profile）与性能要求
   生成，可重建、面向 mmap/顺序读取和直接索引；
3. **源映射 / 诊断制品（Source Map / Diagnostics Artifact）**：StableId128/LIR entity 到 source span、
   canonical tuple 和 pass provenance 的映射；
4. **语义差异（Semantic Diff）**：以稳定标识和字段语义描述新增、删除、重接、
   geometry/behavior 变化，供 PR/Gate 审阅。

Publication manifest 对完整 portable artifact bytes 计算
`canonicalArtifactDigest`，其余三类输出引用该 digest 与 compilation provenance。
Static image 另外由外部 descriptor 绑定 target/profile-specific
`staticImageDigest`。任何输出失败，本次 compilation unit 都不得发布部分结果。

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
  Required: PartitionPlanningHints
  Optional: StaticSpatialImage
  Optional: WarmQueryTables
  Optional: ColdIdentityAndDiagnostics
```

- `traffic-headless-v1` 要求 Traffic + PartitionPlanningHints；
  `traffic-spatial-v1` 再要求 Spatial；`traffic-debug-v1` 再加入
  warm/cold/debug section；
- target `laneflow-runtime`/`TrafficWorld` 只借用或共享 `StaticTrafficView`，不要求
  Spatial bytes；
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

### 6. 静态镜像与可变运行时状态物理分离

静态镜像不得内嵌 vehicle、controller clock、reservation、occupancy、runtime
Route generation、world/session nonce 或其他每世界可变状态。运行时状态使用
独立 dense arrays，并以 image ordinal/typed `u32` handle 关联静态表。

静态镜像表示一个不可变路网修订，不表示城市道路永不变化。玩家或工具修改来源
模块后必须生成并验证新修订；运行世界只能在 fixed-tick 安全边界通过失败关闭的
镜像切换事务原子迁移，不能原地修改共享 image。每世界 seed/随机流、执行计划、
快照和宿主 identity 也不能进入共享 image。

人口、Routing 与游戏规则的 seed/随机流由 caller/出行编排层拥有；Traffic Runtime
不得为了存档或共享镜像而新增隐藏随机权威。若后续 G1 显式引入 Runtime 自有随机流，
它只能属于每世界可变状态。

切换的昂贵验证、分配和迁移在 tick 外准备完整候选状态，安全边界只原子切换
image/state binding；旧 image 在全部借用视图/token 退出后回收。任一准备/提交
失败都继续使用旧修订。

静态镜像按访问频率拆分：

- **hot**：tick/pose 所需 SoA/CSR 数据、flat ranges、precompiled occurrence、
  speed/length/gate constraint 与 geometry sampling index；
- **warm**：低频 query、owner/member、debug draw 和可选 spatial detail；
- **cold**：StableId128、external display name、source provenance、canonical tuple、
  诊断文本和 publication metadata。

hot path 只使用 typed dense handle、contiguous range 和预编译索引。StableId128、
BLAKE3、XXH3、字符串、hash lookup、path matching 与 schema validation 都不得进入
steady tick。Spatial/cold section 可以按 closed profile 从 headless/server image 中剥离，
但 artifact digest 和 source map 仍能恢复诊断。

### 7. 可移植制品（Portable Artifact）与静态镜像（Static Image）使用独立版本轴

版本不得再被一个 `formatVersion` 混合表达：

- `authoringFormatVersion`
- `canonicalFormatVersion`
- `identityEncodingVersion`
- `identityRegistryRevision`
- `staticImageLayoutVersion`
- `staticImageProfileId`
- `staticImageDescriptorVersion`
- `executionConstraintVersion`
- `partitionHintVersion`
- `runtimeSnapshotVersion`
- `compilerBuildId`
- `validatorBuildId`
- `constraintSetVersion`
- `targetTriple`
- `canonicalArtifactDigest`
- `staticImageDigest`
- `validationReceiptDigest`

三个 digest 均为各自目标对象完整 exact bytes 的 SHA-256，因此任何对象都不得把
自己的 digest 嵌回自身 byte sequence。Publication manifest / external descriptor
负责保存目标对象的 digest；image header 可以保存另一个对象的
`canonicalArtifactDigest`，但不保存自己的 `staticImageDigest`。

loader/runtime 对 static image layout、target、profile、descriptor 与 digest 采用 exact-current、
fail-closed 规则；不在生产启动路径执行历史迁移。旧 authoring/canonical artifact
通过离线 compiler upgrade/migration 进入当前版本。Static image 可以在保持
canonical artifact 不变时因布局或 CPU target 变化而重建。

### 8. 验证（Validation）与信任（Trust）采用四道分离防线

1. **Compiler validation**：对 AST/HIR/MIR/LIR 执行完整语义和生成前置检查；
2. **Independent artifact validator**：只消费 portable canonical artifact 和公开
   constraint contract，独立实现 topology、标识、ownership、coverage、
   geometry 与 occurrence 检查；不得调用 compiler semantic validation；
3. **Validation receipt / external descriptor**：绑定 artifact/image digest、target、
   profile、constraint、compiler/validator build；其 authenticity 由签名 publication
   manifest、宿主认证 asset chain 或 pinned digest 提供；
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

Validation receipt 必须记录 artifact semantic validation 与 independent image
rebuild comparison 的成功结果。未完成两者时不得签发可进入 production fast path
的 descriptor。

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

`laneflow-static-contract` 只拥有 StableId/kind/tag、typed ordinal 和
version/digest/profile primitives，不依赖 Serde、filesystem、Runtime 或 Spatial。
`laneflow-static-image` 只拥有 image ABI、section/profile、bounded verifier 和
borrowed views，不依赖 compiler/validator/Runtime/Spatial。

Target `laneflow-runtime` 是 current `laneflow-core` 的 clean-break 名称，只拥有
tick、vehicle、dynamic Route 和每 world 可变交通状态。Static/shared contract 不得
留在 Runtime；否则 compiler/validator 会反向依赖动态运行时。Spatial 继续不依赖
compiler/validator/引擎，Runtime 继续不依赖 Spatial。`laneflow-data` 在迁移期间
作为 current JSON compatibility façade 存在，终态不再拥有静态 normalization
authority。

### 10. 迁移必须保持当前态（Current）与目标态（Target）语义可区分

本 ADR 在 Accepted 前是 #291 的目标决策输入；在 static-image/Traffic Runtime
cutover 完成 G4 前：

- Traffic v0.10 / SpatialPackage v0.1 / ScenarioManifest v0.1 继续是当前 production
  contract；
- `InitialTrafficData`、current loaders、schemas 和已发布 artifacts 继续按原 ADR
  工作；
- target docs 必须标注“未实现”，不得把 static image 或 `laneflow-runtime` 描述成
  current API；
- 新实现切片不得以 current object graph 作为 compiler IR，也不得让过渡兼容反向
  塑造终态布局；
- #292 到 image cutover 之间只允许 integration-only LIR→current projection：
  bridge crate 可以依赖 compiler + current Core/Spatial，compiler 不依赖 bridge；
  bridge 不公开为 production backend，并由 cutover owner 删除；
- 切换必须有同一场景的 behavior/determinism/pose equivalence、启动成本、retained
  memory 和 10k/100k runtime 性能证据；
- production cutover 后才可把旧 JSON runtime path 和重复 registry construction
  标记 Deprecated/移除，并以独立 breaking implementation Issue 完成
  `laneflow-core/CoreWorld` → `laneflow-runtime/TrafficWorld` clean break。

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

本 ADR 不冻结分区算法、任务运行库、固定边界邻域宽度或数值归约算法。工作线程数、
分区计划和任务完成顺序不得改变精确执行路径的已提交状态、事件或安全结果。未来
保真度（Fidelity）降级必须进入独立 ADR/G1，不能伪装成调度优化。

### 12. 运行时快照、回放和路径规划不进入共享静态权威

运行时快照（Runtime Snapshot）是独立版本化制品，至少绑定规范制品摘要、静态镜像
摘要、运行时/约束/快照版本、路网修订、world identity、tick、输入命令游标和全部
每世界可变交通状态；只有后续 G1 显式授予的 Runtime 自有随机流才进入该快照。跨
修订恢复必须通过稳定标识与语义差异执行显式迁移；旧 dense ordinal
不得直接解释为新修订实体。动态 Route、车辆和其他运行时实体以快照局部标识保存
引用关系，原进程 runtime handle/slot/generation 不得成为恢复后身份。
运行时执行计划在恢复时按当前硬件重建，快照不得要求复现原分区/工作线程布局。

回放使用显式输入命令序列（Input Command Sequence）、checkpoint 和确定性状态
摘要。调试构建可以借助冷标识与源映射产生失同步诊断制品，定位首个分歧 tick、
phase、实体和资源组件。

交通运行时从已提交状态导出交通观测快照，不拥有全局成本政策。路径规划/出行编排
层结合静态网络、观测、收费、游戏政策和偏好构造动态成本快照并产生候选路径。
出行需求和路线选择策略由上层出行与交通编排拥有；车辆 fixed-tick 热路径不执行
全图寻路。成本快照和候选 Route 绑定路网修订、观测 tick 与成本模型版本；Runtime
对修订不匹配失败关闭。具体过期容忍、快照线格式、摘要算法、routing crate/API 与
跨修订迁移算法由后续独立 G1 冻结。

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
- 10k/100k 交通运行时固定步进（Traffic Runtime Tick）与空间位姿（Spatial Pose）
  闸口不得回退；城市级 1M 实体编译和镜像
  构建作为 #72 的离线扩展基准；
- `traffic-headless-v1` 不携带 Spatial bytes；2/8/32 worlds 共享 Traffic section；
- load limits 在任何 per-world allocation 前验证，恶意 cardinality/size 不得触发
  无界分配；
- 单线程/并行、clean/incremental、受支持平台的 portable artifact 逐字节一致；
- 同一冻结场景在支持的 worker 数与 partition plan 矩阵中产生相同 committed
  state、事件和确定性状态摘要，且不引入 partition-induced extra tick delay；
- target-specific image 只要求相同 target/layout/profile 下逐字节一致，不要求不同
  target 的 bytes 相同，但它们必须引用同一 canonical digest。

Target image 的具体零拷贝/归档实现（自有 offset tables、经过审计的 archive crate 或
其他方案）必须通过布局稳定性、安全 verifier、MSRV、维护性和 benchmark 比较后
选择；本 ADR 不以某个库名替代性能与安全契约。

## 对既有 ADR 的取代矩阵

| ADR  | 继续有效                                                                                                      | 本 ADR 目标取代                                                                                                                                     |
| ---- | ------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------- |
| 0005 | 外部稳定标识（External Stable Identity）、有类型密集句柄（Typed Dense Handle）、热冷分离、动态车辆 / 路线生成 | 核心初始化时分配静态句柄 / 登记表；目标态动态执行层改名为交通运行时（Traffic Runtime）                                                              |
| 0007 | 运行时（Runtime）不依赖 Serde / 文件系统 / 引擎，以及内存中字节边界（In-memory Bytes Boundary）               | 私有 DTO → `InitialTrafficData` 作为终态唯一输入、核心构造器作为全部静态规范化权威                                                                  |
| 0008 | 精确当前版本（Exact-current）、失败关闭（Fail-closed）、离线迁移（Offline Migration）                         | 单一 `formatVersion` 同时承担来源、制品与静态镜像布局版本                                                                                           |
| 0011 | 不可变发布（Immutable Publication）、规范 URL（Canonical URL）、来源沿袭（Provenance）、运行时不联网          | 发布目录只描述 JSON Schema 系列；未来扩展规范制品、静态镜像变体与验证收据                                                                           |
| 0013 | 交通运行时 / 空间层 / 适配器权威、规范几何、长度 / 位姿、失败原子性、无图形支持                               | 交通 / 空间两个独立制品在运行时按外部标识（External ID）和清单摘要（Manifest Digest）联结；不再以“无空间层核心（Core-without-Spatial）”作为目标架构 |
| 0015 | 有界规范 `f32` 坐标框架、误差 / 内存 / 批量性能边界、无空间层核心（Core-without-Spatial）                     | 空间层初始化时从核心句柄和 JSON 几何重建登记表；静态镜像的空间节保持可选                                                                            |
| 0017 | 路口、通行流向、机动路径、机动门、路线出现项语义，共享内部边，热路径无字符串                                  | 核心规范化 / 路线注册期首次编译静态出现项；静态初始出现项改由编译器镜像预编译                                                                       |

本矩阵只取代“工作发生在哪一层、何时发生、如何存储”的条款，不重新定义上述 ADR
已经冻结的交通、空间和 runtime 行为语义。ADR 0020 Accepted 时，应在各历史 ADR
页头增加精确的 Superseded/Partially Superseded 链接；Proposed 阶段不提前改写其
状态。

## 后果

正向后果：

- 静态网络只有一个编译权威，Traffic/Spatial 不再重复解析、绑定和建索引；
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

### 仅凭镜像头（Image Header）的规范摘要 / 来源沿袭（Canonical Digest / Provenance）接受不可信字节

攻击者可以伪造这些 header 声明、对恶意 image bytes 计算新的
`staticImageDigest`，并构造结构合法但语义恶意或资源消耗过大的 image。Production
fast path 必须有认证的 image 外部 trust anchor；拒绝 header self-attestation。

### 强制交通节（Traffic）与空间节（Spatial）同时存在

这会违反 ADR 0013/0015 的 Core-without-Spatial/headless 边界。Traffic section 必选、
Spatial section 由 closed profile 控制；拒绝 mandatory combined payload。

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

## G1 接受条件

#291 只有在以下内容同时完成本地审阅与外部 re-review 后才能勾选 G1：

1. `network-compiler.md`、ADR 0021 与本 ADR 对 source module graph、IR、标识、
   artifact/image、trust/profile、execution planning、crate DAG 和 migration 的
   描述一致；
2. architecture、Data、current Core/target Traffic Runtime、Spatial、Adapter 文档
   清楚区分 current production 与 target；
3. #292 不再承诺 L1 或 Core-shaped 输出，而是 compiler foundation + Synthetic DSL
   frontend 的首个纵向实现；
4. 受影响 ADR 的继续有效与取代范围逐项登记；
5. 性能 Gate 包含 headless profile、启动/load limits、内存共享、10k/100k runtime
   和城市级离线编译基线；
6. untrusted image rejection、标识 registry known vectors 和 external descriptor
   trust path 已进入 implementation acceptance；
7. 未把具体 archive library、并行框架或增量数据库当作未经基准的既定事实。
8. 城市模拟游戏上层、路径规划、不可变路网修订、运行时快照和每世界唯一性边界
   已同步到 architecture、roadmap、glossary 与 Agent Skills。
