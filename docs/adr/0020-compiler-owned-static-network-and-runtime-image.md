# ADR 0020：编译器拥有静态路网与目标运行时镜像

**状态**: Proposed（#291 G1 修订输入）<br>
**日期**: 2026-07-28<br>
**适用范围**: authoring frontend、编译器 IR、静态路网权威、canonical artifact、target runtime image、source map、独立校验器，以及 Core/Data/Spatial 初始化边界<br>
**目标取代范围**: ADR 0005、0007、0008、0011、0013、0015、0017 中与静态数据 normalization、制品配对和运行时 registry 构建位置冲突的条款；在本 ADR Accepted 且迁移 G4 前，当前生产实现继续由原 ADR 约束<br>

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
- 详细设计:
  - `../design/network-compiler.md`
  - `../design/data-format.md`
  - `../design/data-loading.md`
  - `../design/spatial-geometry.md`
  - `../design/core-id-handles.md`
- GitHub:
  - #291
  - #292

## 背景

LaneFlow 当前以 Traffic JSON、Spatial JSON 和 Scenario Manifest 为运行时输入。
`laneflow-data` 解析外部 ID，调用 Core constructors 构造 `InitialTrafficData`，
编译 Route/Maneuver/Gate occurrence；Spatial loader 随后再次按 Traffic edge
identity 绑定折线并建立自己的 registry。走廊生成器内部又维护一套只服务 JSON
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
离线编译期，并让运行时只验证和挂载已经编译完成的只读镜像。

## 决策

### 1. 编译器是全部静态路网语义的唯一编译权威

编译器负责把 authoring source 降低为一个经过全局验证、具备稳定 identity 和
dense layout 的 canonical LIR。以下工作只发生在编译期：

- authoring key 与引用解析；
- RoadCorridor/RoadSection/Lane/Junction/Movement/ManeuverPath/Gate/WaitingZone、
  signal、parking、access 与 geometry 的静态语义检查；
- StableId128 派生、重复 identity 与 digest collision 检查；
- topology closure、owner/member range、reverse index 和跨域引用生成；
- Route、ManeuverPath、Gate、WaitingZone occurrence 编译；
- Traffic length 与 Spatial arc length 的共同派生及绑定验证；
- target runtime image 的 dense handle 分配与 hot/cold layout。

Core 继续拥有运行时交通规则、车辆和动态 Route lifecycle、tick、安全约束与
runtime state authority；Spatial 继续拥有 canonical geometry sampling 和 pose
语义；二者不再各自重新解释或重建静态网络。

### 2. Synthetic DSL、Geometry、Import 与 Editor 是 frontend，不是 L1/L2

编译器不定义 L1/L2 架构层。下列输入能力是可组合 frontend：

- **Synthetic DSL frontend**：测试、benchmark、示例和程序化场景；
- **Geometry document frontend**：长期 authoring SSOT；
- **Import frontend**：OSM、外部工具和离线 migration；
- **Editor frontend**：交互编辑，同样产出 typed AST。

每个 frontend 只负责自己的语法、来源保真和 source span，不得输出
`InitialTrafficData`、Core registry 或 target runtime object graph。不同 frontend
可以在同一个 compilation unit 中导入模块，并在 HIR 之后共享完全相同的语义
管线。

### 3. 采用 typed AST → HIR → MIR → validated canonical LIR

编译器中间表示按稳定职责分层：

| 阶段          | 权威与允许内容                                                         | 禁止内容                          |
| ------------- | ---------------------------------------------------------------------- | --------------------------------- |
| typed AST     | frontend 语法、显式 key、source span、未解析引用                       | Core handle、target layout        |
| HIR           | namespace、模块、符号、单位、引用与 authoring 语义已解析               | geometry tessellation、dense slot |
| MIR           | 展开 corridor/section/lane、生成 junction/path、曲线离散、全局静态语义 | target ABI、平台指针              |
| canonical LIR | StableId128、规范化单位、完整静态关系、确定性顺序、共享逻辑索引        | JSON object graph、宿主类型       |

每一阶段只由前一阶段构造；公共 pass 以显式输入/输出和结构化诊断运行。LIR 是
所有后端的唯一输入。后端不得补充语义默认、重新推断 topology 或修复缺失关系。

具体 Rust collection、arena library 或序列化库不在本 ADR 冻结；必须通过独立
实现基准选择。但 IR 要求：

- arena/dense ordinal 可用 `u32` 表达，超界时编译失败；
- iteration order 与输出不依赖 hash table iteration；
- 增量 cache key 只影响重用，不影响完整编译结果；
- 并行 pass 必须以稳定 merge order 产生与单线程逐字节相同的输出。

### 4. 一个 canonical LIR 产生四类配套输出

一次成功编译原子地产生：

1. **Portable canonical artifact**：平台无关、确定性、可发布和长期审计的静态网络
   事实；承接 public artifact、迁移和跨实现互操作；
2. **Target runtime image**：按目标平台、布局版本、feature set 与性能 profile
   生成，可重建、面向 mmap/顺序读取和直接索引；
3. **Source map / diagnostics artifact**：StableId128/LIR entity 到 source span、
   canonical tuple 和 pass provenance 的映射；
4. **Semantic diff**：以 stable identity 和 field semantics 描述新增、删除、重接、
   geometry/behavior 变化，供 PR/Gate 审阅。

四者共享同一 `canonicalArtifactDigest` 与 compilation provenance。任何输出失败，
本次 compilation unit 都不得发布部分结果。

Portable canonical artifact 是发行与治理契约，不是运行时热布局。Target runtime
image 是可丢弃、可按目标重新生成的派生制品，不获得独立 authoring authority。
源文档始终是唯一 authoring SSOT；generated artifact 是否 checked in 只属于发布
和审阅策略，不能形成第二套可手改事实源。

### 5. Core 与 Spatial 直接消费同一只读静态镜像

Target runtime image 逻辑上包含共享 header 与两个静态视图：

```text
StaticNetworkImage
  Header / provenance / feature table
  StaticTrafficImage
  StaticSpatialImage
  ColdIdentityAndDiagnostics
```

- Core 只借用或共享拥有 `StaticTrafficImage`；
- Spatial 只借用或共享拥有 `StaticSpatialImage`；
- 两个视图使用同一 logical entity ordinal 和经过编译的 cross-table index；
- Adapter 继续只消费 Core committed snapshot 与 Spatial pose batch，不读取 compiler
  IR，也不获得静态规则 authority；
- 多个 `CoreWorld` 可以共享同一 immutable image。

生产启动不得执行 JSON parse、external string rebind、static hash registry rebuild、
topology reconstruction、Traffic/Spatial package join 或 initial Route occurrence
recompile。加载路径只做有界结构验证、版本/target/provenance/digest 检查、区间与
对齐检查，然后建立只读 view。

### 6. 静态镜像与可变运行时状态物理分离

静态镜像不得内嵌 vehicle、controller clock、reservation、occupancy、runtime
Route generation、world/session nonce 或其他每世界可变状态。运行时状态使用
独立 dense arrays，并以 image ordinal/typed `u32` handle 关联静态表。

静态镜像按访问频率拆分：

- **hot**：tick/pose 所需 SoA/CSR 数据、flat ranges、precompiled occurrence、
  speed/length/gate constraint 与 geometry sampling index；
- **warm**：低频 query、owner/member、debug draw 和可选 spatial detail；
- **cold**：StableId128、external display name、source provenance、canonical tuple、
  诊断文本和 publication metadata。

hot path 只使用 typed dense handle、contiguous range 和预编译索引。StableId128、
BLAKE3、XXH3、字符串、hash lookup、path matching 与 schema validation 都不得进入
steady tick。cold section 可以按 feature profile 从 headless/server image 中剥离，
但 artifact digest 和 source map 仍能恢复诊断。

### 7. Portable artifact 与 runtime image 使用独立版本轴

版本不得再被一个 `formatVersion` 混合表达：

- `authoringFormatVersion`
- `canonicalFormatVersion`
- `identityVersion`
- `runtimeImageLayoutVersion`
- `compilerBuildId`
- `constraintSetVersion`
- `targetTriple`
- `featureSet`
- `canonicalArtifactDigest`

loader/runtime 对 runtime image layout、target、feature 与 digest 采用 exact-current、
fail-closed 规则；不在生产启动路径执行历史迁移。旧 authoring/canonical artifact
通过离线 compiler upgrade/migration 进入当前版本。Runtime image 可以在保持
canonical artifact 不变时因布局或 CPU target 变化而重建。

### 8. 验证采用三道相互独立的防线

1. **Compiler validation**：对 AST/HIR/MIR/LIR 执行完整语义和生成前置检查；
2. **Independent artifact validator**：只消费 portable canonical artifact 和公开
   constraint contract，独立实现 topology、identity、ownership、coverage、
   geometry 与 occurrence 检查；不得调用 compiler semantic validation；
3. **Runtime image verifier**：只验证不可信 bytes 的 header、版本、digest、offset、
   alignment、range、table cardinality、cross-index bounds 和 target compatibility，
   不重跑全量 authoring 语义。

compiler 与 independent validator 可以共享机器可读的常量/枚举/约束声明，但不得
共享实现同一语义判定的函数。CI 至少要求 canonical artifact 通过独立 validator，
runtime image 可从该 artifact 确定性重建，且 runtime verifier 接受生成镜像。

### 9. Crate 目标边界

终态逻辑边界为：

```text
laneflow-format          portable source/artifact contracts and version metadata
laneflow-compiler        frontends, IR, passes, emitters, source map, semantic diff
laneflow-validator       independent portable artifact validator
laneflow-runtime-image   image builder contract, verifier and read-only views
laneflow-core            traffic runtime consuming StaticTrafficImage
laneflow-spatial         pose runtime consuming StaticSpatialImage
laneflow-adapter-*       engine lifecycle/presentation boundary
```

这是一组职责边界，不要求在第一个实现 PR 一次创建全部 crate。`laneflow-data` 在迁移
期间作为 current JSON compatibility façade 存在；终态不再拥有静态语义
normalization 权威。Core 继续不依赖 Serde、filesystem、compiler、validator、
Spatial 或引擎。Spatial 继续不依赖 compiler 或引擎。

### 10. 迁移必须保持 current 与 target 语义可区分

本 ADR 在 Accepted 前是 #291 的目标决策输入；在 runtime image 路径完成 G4 前：

- Traffic v0.10 / SpatialPackage v0.1 / ScenarioManifest v0.1 继续是当前 production
  contract；
- `InitialTrafficData`、current loaders、schemas 和已发布 artifacts 继续按原 ADR
  工作；
- target docs 必须标注“未实现”，不得把 runtime image 描述成 current API；
- 新实现切片不得以 current object graph 作为 compiler IR，也不得让过渡兼容反向
  塑造终态布局；
- 切换必须有同一场景的 behavior/determinism/pose equivalence、启动成本、retained
  memory 和 10k/100k runtime 性能证据；
- production cutover 后才可把旧 JSON runtime path 和重复 registry construction
  标记 Deprecated/移除。

## 性能与确定性契约

实现进入 G2 前必须冻结并验证：

- image hot tables 使用 SoA/CSR/flat ranges，索引宽度默认 `u32`；
- Core/Spatial 对 image 的读取不经 trait object、字符串 resolver 或 per-record bounds
  graph walk；必要安全检查在 image verifier 或 batch boundary 聚合；
- steady tick 与稳定容量 pose batch 零分配；
- 多 world 共享 image 时静态内存不按 world 复制；
- runtime image 加载时间、峰值分配和 retained memory 相对 current JSON +
  normalization 基线具有量化 Gate；
- 10k/100k Core tick 与 Spatial pose Gate不得回退；城市级 1M entity 编译和 image
  构建作为 #72 的离线扩展基准；
- 单线程/并行、clean/incremental、受支持平台的 portable artifact 逐字节一致；
- target-specific image 只要求相同 target/layout/profile 下逐字节一致，不要求不同
  target 的 bytes 相同，但它们必须引用同一 canonical digest。

Target image 的具体零拷贝/归档实现（自有 offset tables、经过审计的 archive crate 或
其他方案）必须通过布局稳定性、安全 verifier、MSRV、维护性和 benchmark 比较后
选择；本 ADR 不以某个库名替代性能与安全契约。

## 对既有 ADR 的取代矩阵

| ADR  | 继续有效                                                                                           | 本 ADR 目标取代                                                                                                  |
| ---- | -------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------- |
| 0005 | external stable identity、typed dense handle、hot/cold 分离、dynamic Vehicle/Route generation      | Core 初始化时为静态网络分配 handle、构建 external-ID registry                                                    |
| 0007 | Core 不依赖 Serde/filesystem/engine、in-memory bytes 边界                                          | private DTO → `InitialTrafficData` 作为终态唯一输入、Core constructors 作为全部静态 normalization owner          |
| 0008 | exact-current、fail-closed、离线 migration                                                         | 单一 `formatVersion` 同时承担 source/artifact/runtime layout                                                     |
| 0011 | immutable publication、canonical URL、provenance、runtime 不联网                                   | publication catalog 只描述 JSON Schema family；未来扩展为 canonical artifact 与 runtime image variant            |
| 0013 | Core/Spatial/Adapter authority、canonical geometry、length/pose 语义、失败原子性                   | Traffic/Spatial 两个独立制品在运行时按 external ID + manifest digest join                                        |
| 0015 | bounded canonical f32 frame、误差/内存/批量性能边界                                                | Spatial 初始化时从 Core handle + JSON geometry 重建 registry                                                     |
| 0017 | Junction/Movement/ManeuverPath/Gate/Route occurrence 语义、shared internal edge、hot path 无字符串 | Core normalization/Route 注册期首次编译静态 occurrence；静态 initial route occurrence 改由 compiler image 预编译 |

本矩阵只取代“工作发生在哪一层、何时发生、如何存储”的条款，不重新定义上述 ADR
已经冻结的交通、空间和 runtime 行为语义。ADR 0020 Accepted 时，应在各历史 ADR
页头增加精确的 Superseded/Partially Superseded 链接；Proposed 阶段不提前改写其
状态。

## 后果

正向后果：

- 静态网络只有一个编译权威，Traffic/Spatial 不再重复解析、绑定和建索引；
- frontend 可独立演进，Synthetic DSL 不会成为 Geometry/Import/Editor 的下级；
- portable governance contract 与 target performance layout 解耦；
- Core/Spatial 可共享只读静态内存，多 world 只分配可变状态；
- JSON、Serde、external IDs 与 hash maps 退出生产启动和热路径；
- source map、semantic diff 和独立 validator 同时提高可审阅性与系统性 bug 防线。

成本与风险：

- 需要一次跨 compiler/Data/Core/Spatial/Adapter initialization 的架构迁移；
- portable artifact、runtime image 和 validator 各自需要版本、fuzz、安全与发布治理；
- image verifier 处理不可信二进制，任何 unsafe/zero-copy 方案都必须接受严格审计；
- compiler 的确定性并行、增量 invalidation 和 semantic diff 增加实现复杂度；
- cutover 前 current/target 双路径会暂时增加测试矩阵，但不能成为长期兼容层。

## 被拒绝的替代方案

### L1 生成 Core input，L2 再补 geometry

它把 frontend 误建模为有序架构层，让 Core object graph 成为 IR，并使 geometry、
identity 和 topology 的联合验证发生得太晚；拒绝。

### 以 `InitialTrafficData` 或 Core registries 作为共享 IR

这些类型已经含有 runtime normalization、handle 和 Core-specific occurrence，
无法保持 target-neutral，也不能同时服务 portable artifact、Spatial layout 和
semantic diff；拒绝。

### canonical JSON 直接作为唯一生产制品

它适合公共审计与兼容，但会把解析、字符串引用和 object graph 重建留在生产启动，
且布局无法针对 Core/Spatial 热路径优化；拒绝。

### 一个跨平台二进制同时承担 portable artifact 与 runtime image

为追求单一 bytes 会冻结最低公共布局、阻碍 target-specific alignment/SIMD/feature
裁剪，并把长期兼容与高性能加载绑在一起；拒绝。

### compiler validation 与 independent validator 复用同一语义实现

这只能证明同一实现自洽，不能发现系统性 compiler bug；拒绝。

### runtime 加载时重新执行完整 semantic validation

它保留了当前启动成本和重复权威。Runtime 只验证结构安全与版本兼容；完整语义由
compiler + independent validator 在发布前完成；拒绝。

## G1 接受条件

#291 只有在以下内容同时完成本地审阅与外部 re-review 后才能勾选 G1：

1. `network-compiler.md` 与本 ADR 对 frontend、IR、artifact、image、validation 和
   migration 的描述一致；
2. architecture、Data、Spatial、Adapter 文档清楚区分 current production 与 target；
3. #292 不再承诺 L1 或 Core-shaped 输出，而是 compiler foundation + Synthetic DSL
   frontend 的首个纵向实现；
4. 受影响 ADR 的继续有效与取代范围逐项登记；
5. 性能 Gate 包含启动、内存共享、10k/100k runtime 和城市级离线编译基线；
6. 未把具体 archive library、并行框架或增量数据库当作未经基准的既定事实。
