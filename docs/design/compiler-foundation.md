# 编译器基础设施与合成领域专用语言前端

**文档状态**: #292 已接受并完成 G4；#315 共同受检模块接入契约已实现；
#297 current JSON 编译器导入设计已取消；#299 后继边界见 Accepted ADR 0024<br>
**最后更新**: 2026-08-26<br>
**适用范围**: `laneflow-static-contract`、`laneflow-compiler`、
`laneflow-compiler-test-support`、有类型抽象语法树（Typed Abstract Syntax Tree，
Typed AST）→高层中间表示（High-level Intermediate Representation，HIR）→中层
中间表示（Mid-level Intermediate Representation，MIR）→已验证规范低层中间表示
（Validated Canonical Low-level Intermediate Representation，Canonical LIR）、
合成领域专用语言前端（Synthetic Domain-specific Language Frontend，Synthetic DSL
Frontend）、标识 v1（Identity v1）首次实现、确定性（Determinism）编译、诊断
（Diagnostic）与当前态等价投影<br>
**实现状态**: #292 已完成 G4；`laneflow-static-contract` 已建立 `no_std` 值类型、
标识 v1（Identity v1）登记常量、有类型稳定标识与有类型逻辑序号；
`laneflow-compiler` 已建立生产资源配置档、来源模块头、结构化诊断、确定性
`LFSOURCE` 来源记录、显式导入图，以及车道图边、横断面完整所有者树、路口拓扑和
机动门 / 等待区静态闭包的受检合成领域声明。编译器侧标识 v1 编码、BLAKE3-128 派生、完整前像重复 / 碰撞登记、修订 1 全
22 种实体的冻结已知向量与独立测试预言机已经落地；`LaneEdge`、`RoadCorridor`、
`RoadSection`、`AuthoringLane`、`LaneGroup`、`FacilityBand`、`Junction`、`Movement` 和
`ManeuverPath`、`StopLine`、`ManeuverGate` 和 `WaitingZone` 已接入有类型符号解析、
父项先于子项的身份闭包、规范 HIR/MIR/LIR 连续表及来源伴随数据；完整
`entry + internal + exit` 路径、派生路口内部边排他角色、路径转换门、每条 `LaneEdge`
至多一条 `StopLine` 与入口候选路径完整覆盖约束、等待区静态区间约束已经闭合。不可变
固定时制信号程序、完整相位状态和机动门信号绑定
已在运行时之前闭合；停车区域（`ParkingArea`）、停车位（`ParkingSpace`）、入口 / 出口
车道锚点、当前态静态几何和区域反向成员索引也已接入相同原子管线，其中区域归属不参与
停车位身份。参与者类别（`ParticipantClass`）单继承闭包与静态准入规则
（`AccessRule`）已经接入；编译器保留效果（effect）、类别集合、优先级（priority）和
法规来源，分别冻结 LaneEdge / LaneGroup / RoadSection 的边平面目标与 ManeuverPath
路径平面目标，
并在运行时之前拒绝继承环、无类别规则、法规来源不一致和相反效果的精确并列。
FacilityBand target 继续由结构化能力门卫（capability guard）失败关闭，时变窗口尚未
进入合成领域声明。`StaticRoute` 已从编制来源、IR 与制品删除（ADR 0029）；路线出现项只在
`register_route` 编进每世界表。当前道路机动车的 `VehicleProfile` 已按
ADR 0028 整数毫米 / 受检 `f32` SI 约束接入（准入后 IR 不再把交通一维
存成编制 `f64`），唯一引用一个 `ParticipantClass`，并冻结身份、
参数、语义摘要与来源关系；它不构成其他交通执行域的通用参数基类。规范坐标框架
（`CanonicalFrame`）已把 SpatialPackage v0.1 的稳定 `frameId` 接入 Typed
AST→HIR→MIR→Canonical LIR 与来源映射；单位、手性、`+Y` 上方向和有界点范围仍是
全局固定空间契约，声明不拥有 CRS、宿主放置或可变原点。车道图边中心线已经接入同一
管线：空间存在时验证全图恰好一次覆盖、规范 `f32` 线段、交通长度绑定和连接端点连续性，
再按 `LaneEdgeOrdinal` 对齐冻结点、累计弧长、切向与上方向采样表；不声明中心线时仍允许
无图形配置（headless）LIR。集成专用 `laneflow-compiler-test-support` 已把
`ValidatedCanonicalLir` 按有类型序号投影为当前 `InitialTrafficData`、可选
`SpatialRegistry` 与稳定映射报告；它保持 `publish = false`，不读取当前 JSON，也不
成为生产后端。公共 `Compiler` 已经原子
返回配对的 `ValidatedCanonicalLir` 与 `ValidatedSourceMapInput`；首批支持矩阵中的
动态路线生命周期及后继编译遍尚未实现。
#292 G1 已
接受 #308 G4 非生产研究证据及首轮资源 / 性能输入；#308 研究工作负载不能按原自然
身份无损映射为生产语义。可运行交通世界现由 `TrafficWorld` 安装
`SharedNetworkRevision`；`laneflow-data` / `laneflow-core` / current JSON 已拆除。
#315 已按 G2 授权落地共同私有
`TypedAstModule` / `TypedAstDeclaration`、逻辑模块与来源文档独立登记、原子共同接入、
文档集摘要以及 `LF-COMP-P100-INITIAL-v2`。#296 因 production 来源产品前提纠偏返回
FlatBuffers；#332 草稿中的旧 Geometry JSON 原型只作历史证据。
新的 FlatBuffers 道路编辑来源契约由
`road-editing-source-and-geometry-frontend.md` 冻结并由默认 production compiler 入口实现，
但仍是 unpublished B1，不构成已发布存档格式或长期兼容承诺。当前任务与 Gate 状态以
GitHub Issue / PR 为准。#297 调整后不建立 current JSON 编译器前端；
精确退役和测试边界见 `current-package-import.md`。

**关联决策与设计**:

- `../adr/0014-residual-aware-f32-core-authority-and-migration-gates.md`
- `../adr/0020-compiler-owned-static-network-and-static-image.md`
- `../adr/0021-city-simulation-game-traffic-foundation.md`
- `../adr/0023-road-editing-state-and-phased-network-replacement.md`
- `../adr/0025-checked-canonical-network-and-shared-static-network.md`
- `network-compiler.md`
- `road-editing-source-and-geometry-frontend.md`
- `numeric-representation.md`
- `spatial-geometry.md`
- `core-runtime-performance-baseline.md`（复用 P100 硬件身份，不复用运行时规模或预算）
- `compiler-budget-calibration.md`（#308 已关闭；证据在 git 历史）
- `current-package-import.md`
- `../reference/v0.10-compiler-production-baseline.md`
- [G4 工作负载清单](https://github.com/illusion-tech/laneflow/blob/de4cd460a96415cafbd811141568b81f74d73534/docs/reference/compiler-calibration-workloads-v1.json)
- [G4 R0 报告](https://github.com/illusion-tech/laneflow/blob/de4cd460a96415cafbd811141568b81f74d73534/docs/reference/v0.10-compiler-budget-calibration-report.md)
- [G4 R0 Evidence](https://github.com/illusion-tech/laneflow/blob/de4cd460a96415cafbd811141568b81f74d73534/docs/reference/v0.10-compiler-budget-calibration-evidence.json)
- [G4 R0 raw](https://github.com/illusion-tech/laneflow/blob/de4cd460a96415cafbd811141568b81f74d73534/docs/reference/v0.10-compiler-budget-calibration-raw.json)
- `../reference/glossary.md`

## 术语规范

本文的中文术语和中文定义是权威事实，英文只作辅助理解。完整双语映射以
`../reference/glossary.md` 为单一事实源（Single Source of Truth，SSOT）。Rust
类型、包（crate）、字段、版本值、诊断代码和协议常量等精确标识符使用反引号保留
原文。

本文严格区分当前态（Current State）与目标态（Target State）。下文首次出现的领域
术语给出英文辅助名，后续只使用中文规范名或 glossary 登记的精确缩写，不在本文另立
竞争定义。

## 1. 设计状态与目标

本文把已接受（Accepted）的架构决策记录（Architecture Decision Record，ADR）0020
与 `network-compiler.md` 的长期封闭契约（Closed Contract）收窄为 #292 可以直接
实现和验证的首个纵向切片。本文不重新讨论下列已接受结论：

- 编译器是全部静态路网语义的唯一编译权威；
- 不存在 L1/L2，也不以 `InitialTrafficData`、当前核心对象图（current Core object
  graph）或当前空间登记表（current Spatial registry）作为编译器中间表示；
- 所有正式前端是权威来源模块图（Authoritative Source Module Graph）中的平级来源
  模块（Source Module）；
- 只有以已验证规范低层中间表示为静态语义核心的成功编译结果可以进入编译发射器
  （Compiler Emitter）或集成专用投影；
- 稳定声明 / 可独立寻址派生实体使用稳定标识（Stable Identifier），所有者局部关系
  （Owner-local Relation）/ 所有者局部出现项（Owner-local Occurrence）和低层表行
  使用各自不同的引用形式；
- 干净单工作线程编译（Clean Single-thread Compile）是并行或增量实现的确定性
  预言机：内存 LIR 必须具有相同规范语义，后继制品发射器存在时则必须产生相同精确
  字节；
- #301 完成前，由 `Traffic v0.10`、`SpatialPackage v0.1`、
  `ScenarioManifest v0.1`、`laneflow-data`、`laneflow-core` 和
  `laneflow-spatial` 组成的当前路径仍是仓库内可运行契约；#301 完成后该路径拆除。

#292 的目标是：

1. 建立可以继续承载官方几何文档前端、导入前端与编辑器编制界面的真实编译器基础；
2. 用合成领域专用语言前端完成首个可执行全管线；
3. 首次实现并冻结标识 v1 的字节、已知向量（Known Vector）和失败关闭（Fail
   Closed）语义；
4. 以集成专用桥（Integration-only Bridge）把已验证规范低层中间表示投影到当前核心
   和空间层，形成迁移等价证据；
5. 建立编译时延、峰值 / 保留内存（Retained Memory）和规模扩展基线，不以运行时
   交通参与单元规模替代编译器工作量。

#292 G1 已接受本文并在 GitHub 议题（Issue）#292 记录 `G1 Pass`；随后完成 G2、G3
与 G4，当前完成事实和证据见第 13 节。第 12 节继续保存当时的 G1 接受结果与 G2
前置，不把历史 Gate 记录改写成一次性完成；仓库长期文档也不镜像项目（Project）当前列。

## 2. 包与依赖切片

### 2.1 本议题新增的包

```text
laneflow-compiler-test-support ---> laneflow-compiler
laneflow-compiler-test-support ---> laneflow-core
laneflow-compiler-test-support ---> laneflow-spatial

laneflow-compiler -------------> laneflow-static-contract

仅测试开发依赖:
laneflow-compiler-test-support - - -> laneflow-data
```

实线箭头表示左侧的正常库依赖右侧，虚线只表示测试开发依赖；正常库依赖形成包依赖图
（Crate Dependency Graph，crate DAG）。

中文术语与英文辅助名统一见 `../reference/glossary.md`；下表不重复定义双语映射。

| 包                               | 本议题拥有职责                                                                       | 禁止职责 / 依赖                                                             |
| -------------------------------- | ------------------------------------------------------------------------------------ | --------------------------------------------------------------------------- |
| `laneflow-static-contract`       | `StableId128`、实体种类 / 字段标签登记、标识版本、有类型逻辑序号和值级公共常量       | `Serde`、文件系统、编译遍、当前核心、目标交通运行时、空间层、编译器标识实现 |
| `laneflow-compiler`              | 权威来源模块图、编译器中间表示、合成领域专用语言前端、编译遍、诊断、编译器侧标识实现 | 当前数据层 / 核心 / 空间层对象图、公共制品格式、镜像应用二进制接口          |
| `laneflow-compiler-test-support` | 已验证规范低层中间表示到当前核心 / 空间层的投影、等价测试辅助                        | 生产后端、反向补语义、从当前对象图派生标识、被编译器依赖                    |

`laneflow-static-contract` 使用 `#![no_std]`，只保存跨编译器、格式、镜像与运行时的
小型值类型和机器可读登记常量。它不提供完整标识编码函数；标识生成仍由 compiler
实现，#299 在 `laneflow-format` 中增加的后发射检查也不重新实现 StableId 编码、
BLAKE3 派生或碰撞登记。

共享有类型逻辑序号的封闭标记类型（sealed marker）、`Ordinal<K>` 与稳定实体种类
标记类型（kind marker）也由 `laneflow-static-contract` 拥有；有类型抽象语法树、
HIR 和 MIR 的区块分配键（arena key）则留在编译器内部。这样后继
`laneflow-format`、`laneflow-static-network` 和 `laneflow-runtime` 可以共享零成本
类型，而编译器临时表不会被误写成长期公共契约。

`laneflow-compiler-test-support` 设置 `publish = false`。它只在测试开发依赖中使用
`laneflow-data` 加载当前态固定样例作为迁移预言机（Migration Oracle），该扩展在
#292 G1 显式登记；它的正常库依赖不使用 `JSON` 加载器，也不让
`laneflow-compiler` 看见当前态类型。

### 2.2 本议题不提前建立的包

- `laneflow-format` 的公共可移植规范制品（Portable Canonical Artifact）格式由 #298
  交付；
- `laneflow-format` 的后发射检查与 LFCP v2 最小发布闭合由 #299 交付；不创建
  `laneflow-validator`；
- `laneflow-static-network` 的受检 LFCA 构建闭合、performance-first layout 与共享
  生命周期由 #300 交付；
- `laneflow-runtime` 与空间层共享静态路网消费路径由 #301 交付；#301 同时使
  `TrafficWorld` 成为唯一可运行世界并拆除 current Core/JSON 运行时入口；
- 不可变路网修订、运行时快照（Runtime Snapshot）与在线路网修订切换由 #302 交付。

#292 可以定义后继编译发射器所需的只读已验证规范低层中间表示视图（View），但不得用私有
临时线格式提前冻结 #298/#300 的公共字节契约。

### 2.3 #315 官方前端共同接入、#296 道路编辑与 #297 调整后边界

#315 已实现的共同受检模块接入继续服务 Synthetic、道路编辑和后续正式编制前端，
不因 #297 调整而回退。current JSON 不再是编译器前端：

```text
laneflow-compiler ----------------> laneflow-static-contract
laneflow-compiler -X-------------> laneflow-current-source / laneflow-data

laneflow-data --------------------> laneflow-current-source
laneflow-data --------------------> laneflow-core / laneflow-spatial
```

`laneflow-current-source` 只集中当前加载器的 wire DTO、版本、摘要和配对；不提供严格
导入策略、编译器位置表或资源余额。编译器不建立 `current-v0_10-import` 特性，项目不
建立 `laneflow-current-import`。编译器原生有类型模块是投影测试的唯一输入。

#296 FlatBuffers G1 已选择字段私有、借用完整 size-prefixed FlatBuffers bytes 的
`RoadEditingModuleInput`，并由唯一原子
`CompilationUnitBuilder::add_road_editing_module` 在同一次 builder 可变借用中取得
剩余预算、执行有界 verifier、语义预检、降阶和共同接入。它不公开预构造
`GeometryModule`、wire DTO、通用模块特征或第二条接入路径。

道路编辑来源的 owner tree 必须在单模块内闭合：模块级 key 按 kind 唯一，
owner-scoped key 只在直接 parent 下唯一，来源地址和 writer 顺序携带完整 owner-key
tuple。#296 同时提供第一方 Rust 字段私有有类型来源构造面和 writer；writer 只生成
标准 owned bytes，编译器仍把 reader/verifier 作为不受信任字节的唯一准入边界。

### 2.4 #297 current JSON 退役边界

#297 原 current 包迁移导入设计已由
[`current-package-import.md`](current-package-import.md) 取代。项目没有已发布或用户持有的
current JSON，因此不建立迁移前端、严格导入能力、资产报告或离线导入器。仓库内部
夹具在新编制来源可表达相同场景后一次性转换或删除。
## 3. 公共接口与构造权威

### 3.1 生产公共面

`laneflow-compiler` 的首版生产公共面限制为四类能力：官方来源构造、编译执行、
诊断与资源限制、只读已验证输出。#292 只交付合成领域专用语言前端的来源构造接口，
不公开通用前端插件接口。以下第一段代码只列出 #292 G4 已交付的受检构造面：

```rust
pub struct SourceModuleHeader { /* 调用方提供的非内容字段，私有字段 */ }
pub struct SourceModuleDescriptor { /* 私有字段 */ }
pub struct SyntheticModuleBuilder { /* 私有字段 */ }
pub struct SyntheticModule { /* 私有字段 */ }
pub struct CompilationUnitBuilder { /* 私有字段 */ }
pub struct CompilationUnit { /* 私有字段 */ }
pub struct ValidatedCanonicalLir { /* 私有字段 */ }
pub struct ValidatedSourceMapInput { /* 私有字段 */ }
pub struct CompilationMetrics { /* 私有字段 */ }
pub struct CompilationOutput { /* 私有字段 */ }
pub struct CompileLimits { /* 私有字段 */ }
pub struct DiagnosticBundle { /* 私有字段 */ }

impl CompileLimits {
    pub fn p100_initial_v1() -> Self;
}

impl SyntheticModuleBuilder {
    pub fn new(
        header: SourceModuleHeader,
        limits: &CompileLimits,
    ) -> Result<Self, DiagnosticBundle>;

    pub fn finish(self) -> Result<SyntheticModule, DiagnosticBundle>;
}

impl CompilationUnitBuilder {
    pub fn new(limits: CompileLimits) -> Self;

    pub fn add_synthetic_module(
        &mut self,
        module: SyntheticModule,
    ) -> Result<&mut Self, DiagnosticBundle>;

    pub fn build(self) -> Result<CompilationUnit, DiagnosticBundle>;
}

pub struct Compiler { /* 可复用暂存区和不可变配置 */ }

impl Compiler {
    pub fn new() -> Self;

    pub fn retained_capacity_bytes(&self) -> u64;

    pub fn compile(
        &mut self,
        unit: CompilationUnit,
    ) -> Result<CompilationOutput, DiagnosticBundle>;
}

impl CompilationOutput {
    pub fn metrics(&self) -> CompilationMetrics;
}
```

以上代码表达 #292 G1 与后继性能证据边界修订已接受的公共接口形状。G2 可以在不改变公共构造、所有权、
可见性、错误和确定性契约的前提下细化包内私有字段与实现名称；任何公共接口或上述
契约变化都必须重新进入 G1，不得作为实现细节直接修改。

以下第二段代码登记 #315 已实现的多文档共同能力；它不包含 current JSON 专用入口：

```rust
pub struct SourceDocumentOrigin { /* 字段私有的逐文档显示/审计来源 */ }
pub struct SourceDocumentDescriptor { /* 私有字段 */ }

impl CompileLimits {
    // v1 保持不可变。
    pub fn p100_initial_v2() -> Self;
}
```

#315 G2 已实现下列共同变化：`SourceModuleDescriptor` 保留为只读公共逻辑模块值，
新增版本化文档集摘要查询；文档专属的键、摘要、长度和来源记录由只读公共
`SourceDocumentDescriptor` / `SourceDocumentOrigin` 暴露。二者不提供公开构造器；
`ValidatedSourceMapInput` 分别提供稳定顺序的模块与文档视图。

`SyntheticModuleBuilder` 只接受首批支持矩阵中的领域构造；
`CompilationUnitBuilder` 只通过具体且封闭的官方前端入口接收完成受检构造的来源模块。
必须保持：

- `TypedAstSink`、`TypedAstModule` 与官方前端调度接口是包内私有实现；
- 官方前端只能经受检接收器产生 `TypedAstModule`，不能构造 HIR、MIR 或 LIR；
- `ValidatedCanonicalLir` 和 `ValidatedSourceMapInput` 字段私有，只暴露稳定只读视图；
- `CompilationOutput` 原子拥有 LIR、已验证源映射输入和非错误级诊断；
- 不公开未验证 LIR 的后端入口；编译失败不返回部分 LIR 或部分源映射；
- 编译器实例可以复用暂存区容量，但上次失败不能污染下次编译结果。

后继正式前端可以增加各自的受检 `add_*_module` 方法；current JSON 不在其中。
### 3.2 官方前端封闭边界

#292 与 v0.10 不承诺稳定的第三方自定义前端扩展接口。编译器可以在包内使用封闭特征
（sealed trait）或等价私有调度承载官方合成领域专用语言前端、几何文档前端与导入
前端；该边界允许一次模块级动态分派或泛型单态化，不进入记录级循环，但它不是公共
接口、稳定应用二进制接口或跨版本兼容面。官方前端必须通过包内 `TypedAstSink` 的
受检构造接口：

- 每个来源模块必须有稳定编制命名空间标识（Authoring Namespace ID）
  `authoringNamespaceId`、来源语言（Source Language）、`sourceContentDigest`、前端
  版本、选项摘要与来源沿袭（Provenance）；
- 每个声明必须有显式稳定键和来源位置（Source Span）；
- 可发布模块不接受匿名命名空间、缺失来源沿袭或测试专用来源位置；
- 前端专用语法（Frontend-specific Syntax）可以作为来源诊断元数据保留，但不得
  绕过共同 HIR/MIR 编译遍。

**#315 G1 Accepted：**把第一项扩展为每个逻辑来源模块拥有一个或多个来源文档；每份
文档各自绑定稳定文档键、精确内容摘要、长度和逐文档来源记录。模块级来源沿袭只描述
工具、选项和整体转换，不能替代各文档自身的显示/审计来源。该规则属于 #315 G1
已接受目标边界；`SourceDocumentDescriptor` / `SourceDocumentOrigin` 已由 #315 G2 实现。

第三方工具在 v0.10 可以调用公开的合成领域专用语言构造接口；后继官方几何文档
前端、导入前端或编辑器编制界面分别通过其冻结的来源契约接入，而不是让调用方构造
编译器抽象语法树。这样避免在只有一个生产前端时提前冻结 AST 兼容面，也不把 Rust
特征（trait）误写成稳定二进制插件接口。

只有出现明确的第三方自定义语法 / 数据源需求，并且至少有两类独立前端实现能够证明
共同边界后，才另立议题和 G1 讨论公共扩展协议。届时必须重新裁决：

- 版本协商、兼容与弃用策略；
- 进程内库接口还是隔离进程、`WebAssembly` 或进程间通信（Inter-process
  Communication，IPC）协议；
- 不受信任前端的资源限制、崩溃隔离和供应链边界；
- 来源沿袭、诊断位置、确定性与可复现性；
- 公共来源模型是否能保持独立于编译器私有的有类型抽象语法树、HIR 和 MIR。

本段只登记后继决策入口，不预选具体协议，也不构成 v0.10 兼容承诺。

### 3.3 官方前端共同受检模块接入（#315 G1 Accepted）

#315 G1 已接受把当前 `CompilationUnitBuilder` / `CompilationUnit` 对
`SyntheticModule` 的内部依赖替换为编译器私有（compiler-private）共同表示，同时保持
公开构造面为来源专用的具体类型。下列名称表达所有权和阶段边界；除已经接受的公开类型名以及
`TypedAstModule` / `TypedAstDeclaration` 外，G2 可以选择等价的私有实现名称：

```rust
pub struct SyntheticModule { /* 字段私有的官方具体模块 */ }
// #296 的 production 编译输入只公开受检 expected document key + 借用 bytes；
// 公开编制 model/writer 是独立 authoring API。
pub struct RoadEditingModuleInput<'a> { /* expected key + 完整 size-prefixed bytes + 可选显示来源 */ }
// generated wire view 与 verifier 后的 Typed AST/HIR/MIR 保持 compiler-private。

struct AdmittedOfficialModule {
    typed_ast: TypedAstModule,
    resource_counts: ModuleResourceCounts,
}

pub struct SourceDocumentOrigin {
    /* 文档角色和稳定显示/审计来源；字段私有 */
}

pub struct SourceDocumentDescriptor {
    /* 文档键、内容摘要、原始字节长度、所属模块与逐文档来源记录 */
}

struct TypedAstModule {
    descriptor: SourceModuleDescriptor,
    source_documents: Box<[SourceDocumentDescriptor]>,
    imports: Box<[ImportRecord]>,
    declarations: Box<[TypedAstDeclaration]>,
}

struct TypedAstEntityAddress {
    /* owner local-key components（module-scoped 为空）+ 原始 sibling-local key */
}

struct DeclarationHeader {
    /* entity kind + TypedAstEntityAddress + 独立 identity local key + SourceLocation */
}

struct OwnedEntityReference<K> {
    /* target module namespace + TypedAstEntityAddress + SourceLocation */
}

pub struct CompilationUnitBuilder {
    modules: Vec<AdmittedOfficialModule>,
    /* 与 admitted module 对齐的来源位置 context、唯一性索引和累计资源状态 */
}

pub struct CompilationUnit {
    modules: Box<[TypedAstModule]>,
    /* 已验证的编译单元级计数 */
}
```

`SourceModuleDescriptor` 表达逻辑模块的命名空间、来源语言、工具/选项、整体转换来源
沿袭，以及版本化的 `sourceDocumentSetDigest`；精确文档键、摘要、长度和逐文档来源
保存在字段私有的 `SourceDocumentDescriptor`。一个逻辑模块必须拥有一个或多个来源
文档，多文档前端不得为每份文档虚构模块、命名空间或导入边。

`SourceDocumentOrigin` 是冷的显示/审计元数据，不是内容身份或信任锚。实现可以把重复
来源字符串驻留到共享表，并让文档描述符只保留紧凑序号；来源记录不进入文档集摘要
前像。
`sourceDocumentSetDigest` v1 是模块级快速重放/缓存比较值，不替代逐文档精确身份，也不参与实体
稳定标识。官方前端先对每份规范来源记录精确字节各计算一次 SHA-256，形成
`sourceDocumentDigest`；随后只对已经派生的文档描述符计算一次 SHA-256 聚合，不重新读取或哈希
来源全文，并复用模块内文档规范化所需的同一次排序结果，不执行第二次规范排序。聚合前像按
`sourceDocumentKey` UTF-8 字节序排列，依次编码：ASCII 域
`LFSOURCE-DOCUMENT-SET`、`u32` 小端版本 `1`、`u32` 小端文档数，以及每份文档的 `u32`
小端键字节数、键字节、`u32` 小端 `sourceRecordByteLen` 和 32 字节
`sourceDocumentDigest`。算法或编码变化必须提升文档集摘要版本并更新已知向量；不能选择“第一个
文档”充当模块摘要，也不能让单文档模块把文档摘要与文档集摘要混为同一语义。

#315 G2 已把此前承载共同有类型抽象语法树声明的私有 `SyntheticDeclaration` 改名为
`TypedAstDeclaration`。HIR 只遍历
`TypedAstModule` 与 `TypedAstDeclaration`，不能按 `SourceLanguage`、前端种类或公开
模块封装在记录级分支。#315 不提前增加 `SourceLanguage` 变体；#296 FlatBuffers G1 登记未发布
`RoadEditingSource = 3`，旧未发布 `GeometryDocument = 2` 不形成兼容或数值别名；不增加
current JSON 来源语言。

`CompilationUnitBuilder` 的公开入口保持具体且封闭：

```rust
pub fn add_synthetic_module(&mut self, module: SyntheticModule) -> Result<&mut Self, DiagnosticBundle>;
// #296 在同一次 builder 事务中消费借用 bytes：
pub fn add_road_editing_module(
    &mut self,
    input: RoadEditingModuleInput<'_>,
) -> Result<&mut Self, DiagnosticBundle>;
```

#296 的 owner-qualified child 不能继续塞入现有 `module + stable_key` 查找形状。道路编辑
reader 产出的每个声明/引用必须使用上面的 private `TypedAstEntityAddress`；HIR symbol
table 以 `(module, typed address)` 查找，并按固定 owner-kind 父先子后顺序解析 owner。
CanonicalIdentity 随后严格按 Identity v1 registry 的完整 `EntityKind::required_tags()`
构造；parent StableId 只在适用 kind 中作为其中一项，不能替代 Movement/ManeuverPath
等实体的其他必需 tag。实际布局可以使用受计量 interner/ordinal 和共享 backing，不能
拼接 owner path 成伪 key，也不能让 HIR 按来源语言分支；地址 component、索引 capacity
和解析 scratch 全部进入现有 string/live-byte 账本。

具体方法消费 compiler 自身字段私有封装并进入同一个私有接入函数。不得公开
`add_module`、`OfficialFrontend` 特征、裸 `TypedAstModule` 或裸描述符/内容配对入口，
也不得让外部包实现接入特征。
共同接入必须按下列事务边界执行：

1. compiler 同包官方前端的 `finish` 从受检声明和规范来源记录一次性派生一个模块描述符、一个或多个
   文档描述符、逐文档来源记录、导入、共同声明与全部模块资源计数。字段私有性保证调用方不能重配
   其中任一部分。
   逐文档摘要和精确长度只能由前端对各文档的实际规范来源字节计算；模块文档集摘要只能按本节
   v1 前像从这些受检描述符聚合，二者都不能由调用方自报。
2. `add_*_module` 按值消费具体封装；道路编辑入口只借用原始输入，并在调用内部按值
   消费验证结果。道路编辑 add 成功后还把该模块的 location context 移入 builder，以
   不复用的 builder-local context index 与 admitted module record 绑定；内容移动到私有
   接入值；不得克隆完整声明、
   字符串或几何点，也不得再次编码来源、计算摘要或扫描声明重算资源。
3. 私有接入函数在修改构建器前一次性计算命名空间、全部 `sourceDocumentKey`、模块数、文档数、
   来源字节、导入、声明、引用、关系、身份字段、符号、字符串、机动门、等待区、路线
   出现项、几何点和编译器控制存续字节数的候选累计值，并执行全部
   `CompileLimits` 检查。任一失败只返回规范诊断；构建器的模块、索引和计数保持不变，
   已消费模块被释放。
4. 全部检查成功后才移动模块及其来源位置 context handle、写入模块/文档索引并提交
   候选累计值。调用方加入顺序仍不是规范
   顺序；`build` 在全部模块到齐后统一验证未知导入和循环，并冻结依赖优先、命名空间
   字节序打破平局的规范模块拓扑顺序。来源文档序号在逻辑模块顺序之上，再按模块内完整
   `sourceDocumentKey` 的 UTF-8 字节序冻结；它是独立登记，不能从模块序号推导。
5. 精确来源载荷只在官方前端计算 SHA-256、长度、来源位置和来源专用诊断期间存续。
   `finish` 必须在返回具体模块前释放自有来源字节；接受调用方借用字节的前端不得为了
   共同接入复制全文。具体模块和 `CompilationUnitBuilder` 只保留已绑定描述符、来源
   位置、逐文档来源记录、声明与模块资源计数；HIR/MIR/LIR 和源映射不保留第二份来源全文。

这里的“全部”只指共同接入拥有的编译单元聚合维度：`ModuleCount`、`SourceDocumentCount`、
`ImportEdgeCount`、`SourceBytesTotal`、`DeclarationCount`、`TypedAstRecordCount`、
`ReferenceCount`、`RelationOccurrenceCount`、`IdentityFieldOccurrenceCount`、
`RouteOccurrenceCount`、`ManeuverGateCount`、`WaitingZoneCount`、`GeometryPointCount`、
`SymbolCount`、`StringItemCount`、`TotalStringBytes` 和接入后实际
`CompilerControlledLiveBytes`。`SourceBytesPerModule`、`SingleStringBytes` 与前端构造
峰值由具体前端在 `finish` 前检查；HIR/MIR/LIR、诊断、暂存区、输出与保留容量维度仍由
各自阶段检查。共同接入信任字段私有模块携带的一次性模块资源计数，不重新遍历记录；
这既不会漏掉维度，也不会复制其他阶段的验证权威。

`SourceDocumentDescriptor` 是来源伴随记录，不是有类型抽象语法树领域记录，因此不能挤入
`TypedAstRecordCount`；既有模块头继续独立计为一条 typed AST 逻辑记录。#315 为多文档基数增加
独立 `SourceDocumentCount`，文档键仍纳入既有字符串条目数/字节数；`SourceBytesPerModule` 是该逻辑
模块所有来源文档原始字节长度之和，因而也界定任一单文档；`SourceBytesTotal` 继续跨模块累计。
每个文档恰好有一条 `SourceDocumentOrigin` 关联，因此不另增可独立漂移的来源记录基数
维度；来源角色、显示/审计来源字符串和共享驻留表必须分别计入既有
`StringItemCount`、`TotalStringBytes` 与 `CompilerControlledLiveBytes`。

`LF-COMP-P100-INITIAL-v1` 的维度和数值保持不可变，只继续描述 #292 已交付的单文档
Synthetic 路径。#315 G2 使用新的 `LF-COMP-P100-INITIAL-v2`：除新增
`max_source_document_count = 1566` 外，其余精确上限继承 v1。该数值是 #315 已实现的
共同接入容量，不构成任何 current JSON 前端承诺；后继前端若需要更高文档总数，必须
携带实测存续内存和真实工作负载证据另行提升配置档版本。

`CompileLimits` 私有保存配置档修订与受支持维度，不能由调用方补字段或隐式升级。v1
只通过既有 `ModuleCount` 隐式覆盖单文档模块；任何产生多文档模块的正式前端都必须在
读取、哈希、解析或按规模分配前确认 builder 选择了 v2 或后继显式包含
`SourceDocumentCount` 的具名配置档。
第 5 项不改变逐文档 `sourceDocumentDigest`、`sourceRecordByteLen` 或 `SourceBytesTotal` 的逻辑
计数；文档集聚合只扫描紧凑描述符，不再次扫描来源字节，也不允许把调用方自报摘要变成可信输入。
来源字节存续期间的实际峰值必须由前端自身的 `CompileLimits` 检查覆盖；共同接入累计
`SourceBytesTotal`，并以释放后的实际
存续量检查编译单元级编译器控制存续字节数。由此不会把互不共存的多份来源
全文虚构为同时存续，也不会因摘要已经存在而漏掉总输入规模上限。

共同接入不需要特征对象（trait object）。若 G2 为代码组织使用私有枚举（enum）或封闭
特征（sealed trait），只允许每个模块一次的常数次分派，并且必须在进入
`TypedAstModule` 前消除；任何声明、引用、关系、
字符串或几何点循环都不得承担前端变体分支或虚调用。

职责归属保持互补：#315 只拥有共同表示、模块/文档独立登记与源映射不变量、原子接入、
生命周期和共享测试；#296 只拥有道路编辑来源契约及专用降阶；#297 调整后只拥有
current JSON 退役与投影测试边界，不再拥有编译器前端。

## 4. 阶段表示与内存所有权

### 4.1 有类型抽象语法树

有类型抽象语法树保留：

- 来源模块描述符；
- 声明种类、显式稳定键和有类型单位；
- 有类型未解析引用；
- 来源位置与来源沿袭；
- 官方前端展开来源，但不保存当前核心句柄或目标布局（Layout）。

**#315 G1 Accepted：**把第一项扩展为同时保留一个逻辑模块的一个或多个
`SourceDocumentDescriptor`；该 descriptor set 已是 G2 实现的当前阶段表示，
Synthetic 只是其中恰有一份文档的具体前端。

同一个有类型抽象语法树模块内的声明按来源顺序保存，来源顺序只服务诊断和显式有序
语义，不能直接成为稳定标识或最终表顺序。

### 4.2 高层中间表示

高层中间表示完成：

- 来源模块、导入和命名空间的确定性解析；
- 符号表、有类型引用和循环依赖诊断；
- 单位规范化（Normalization）与默认值显式化；
- 合并编制时的覆盖声明；
- 每个解析结果到原来源位置的可追溯映射。

HIR 的内部引用使用编译器私有、有类型的 `u32` 区块分配键。该键只在本次编译有效，
不能进入 LIR、制品、语义差异或稳定诊断代码。

### 4.3 中层中间表示

中层中间表示完成 #292 支持子集内的全部全局静态语义：

- 道路走廊、道路区段、编制车道和车道图边的拓扑（Topology）关系；
- 路口、通行流向、机动路径、机动门和停止线；
- 信号、停车、横断面 / 准入和等待区绑定；
- 显式规范 `f32` 折线的几何（Geometry）连续性与交通边长共同校验（用
  `length_mm / 1000` 观察值）；
- 父项先于子项的标识闭包；
- 机动路径、机动门 / 等待区（路线出现项不在编译器预计算，见 ADR 0029）；
- 所有者 / 成员关系、覆盖关系（Coverage）、唯一所有者和全局一致性。

MIR 可以使用临时哈希与缓存（Hash and Cache），但所有规范遍历都从显式稳定序列或
排序后的完整键产生。临时指纹（Fingerprint）命中后必须比较完整权威值。

#292 的编制交通长度、速度、车辆配置和停车标量仍以 `f64` 进入编译器，但准入先量化
再按毫米闭包检查；已提交热列与制品为整数毫米 / `mm/s`。准入后 Typed AST / HIR /
MIR / LIR 只带整数毫米或受检 `f32` SI，发射只写已提交值，不能由编译发射器再 round。规范空间几何沿用
ADR 0015 的有界规范 `f32`。这不是重新启动 #144 已形成“不迁移（no-go）”结论的残差
`f32` 生产迁移：ADR 0014 本身仍是 Accepted 历史证据；Spatial 精度条款已被 Accepted
ADR 0015 部分取代。现行一维合同是 ADR 0028。交通一维的窄化必须发生在准入，不能由编译发射器再 round。
若再改毫米量子或权威表示，必须原子修订编译器中间表示、可移植规范制品、共享静态
路网（Shared Static Network）、目标交通运行时和迁移预言机。

### 4.4 已验证规范低层中间表示

LIR 的每类表使用表类型专用的 `Ordinal<K>`：

```rust
#[repr(transparent)]
pub struct Ordinal<K> {
    raw: u32,
    marker: PhantomData<fn() -> K>,
}
```

精确实现可以用宏生成具名别名和 Rust 特征，但必须满足：

- `size_of::<Ordinal<K>>() == size_of::<u32>()`；
- 不同 `K` 之间不能隐式转换；
- 从 `usize`/记录数转换时检查 `u32` 上界；
- 表、区间或计数溢出时返回结构化诊断，不回退为 `usize` 布局；
- LIR 表冻结后使用 `Box<[T]>` 或等价连续不可变存储；
- 稳定实体表按“实体种类、规范标识元组（Canonical Identity Tuple）字节”排序并分配
  有类型逻辑序号；
- 所有者局部关系 / 出现项按所有者序号、有类型角色和领域有序语义分配；
- 没有稳定身份的密集派生表行必须定义完整语义排序键和稳定并列裁决，不能回退到
  输入容器遍历顺序；
- 有序关系保留领域顺序并用连续平面区间表达，不能按实体表排序覆盖语义顺序。

LIR 必须保留后继可移植规范制品所需的完整规范标识元组前像，但不预先规定 #298 的线
格式。建议的内存形态是共享字段字节池、字段记录区间，以及每个稳定实体对应的实体
种类、`StableId128`、有类型逻辑序号和字段区间，而不是给每行复制字符串对象。

### 4.5 已验证源映射输入与编译结果

已验证源映射输入（Validated Source-map Input）与 LIR 在同一次成功编译中原子
冻结，但不属于静态路网语义。#292 G4 已接受且当前实现的基线至少保存：

- 来源模块描述符、每个模块绑定的一份来源文档登记和来源沿袭；
- 稳定实体的实体种类、`StableId128`、有类型逻辑序号与主要 / 关联来源位置；
- 所有者局部关系 / 出现项的所有者稳定标识、有类型角色、本次编译局部序号与来源
  位置；
- 生成关系的推导链和贡献来源位置（contributing spans）。

**#315 G2 Implemented：**一模块一文档基线已扩展为分离的来源模块描述符表、来源文档描述符表
与逐文档来源记录：每个文档显式关联所属逻辑模块和一条来源记录，模块级工具/转换沿袭不能
替代该关联；编译单元按第 3.3 节的独立文档顺序冻结全局来源文档登记。从有类型抽象语法树
降阶到 HIR 时，每个声明、关系或诊断位置都把自身
`SourceLocation::source_document_key()`
解析为已登记文档序号，不能从所属模块序号推断文档。为避免后续记录热循环携带或比较
`Arc<str>`，共同准入建立并保留文档键到“所属规范模块序号 + 紧凑文档序号”的唯一索引，
`build` 原位冻结序号，源映射阶段复用该索引而不重新建表。每次解析还必须核对文档所属
模块；键缺失或跨模块错绑均返回结构化诊断。每条已冻结来源记录只保存解析后的序号和
区间；HIR/MIR 模块不保留“默认文档”键或序号。该内部序号不构成可持久制品编码承诺。

#296 道路编辑来源为每个模块建立字段私有的 `RoadEditingLocationContext`，统一拥有
intern 后的来源地址 namespace/key components、闭合属性路径、`canvas_selection` key 和
必要显示字节；完整 owner-qualified wire reference 不作为第二份字符串驻留。输入先以
`RoadEditingModuleInput::try_new` 验证 required expected document key；context 在 verifier
前以该 key 建立 `Input` identity，wire 失败只保留受检 trace。verifier 后 wire document
key 必须与 expected key 相等，语义 preflight 才补齐 `Verified` module/document identity
和稳定地址。

context 在进入任何返回对象前冻结为 `Arc` 或等价的共享不可变 owner。add 成功后 builder
接管 handle；后续 add 失败时 candidate handle 移入 `DiagnosticBundle`，bundle 对已经提交
module context 只复制 handle，builder 保留原 handle 并可继续使用；build 失败遵守同一
规则，成功编译时 `ValidatedSourceMapInput` 接管完整 handle 集合。禁止为了诊断深拷贝
context，Arc allocation/strong handle/vector capacity 和失败 retained bytes 全部预收费并
进入 `CompilerControlledLiveBytes`。道路编辑位置以不复用的 builder-local context index
+ context-local typed ordinal 寻址；规范模块重排携带 index 而不重编号，也不以 index
排序。ordinal 不进入 LIR、摘要、持久编码或规范排序；排序必须解析并比较实际来源地址、
属性 step 与 key bytes。

返回 `DiagnosticBundle` 后，candidate context、bundle handle vector 与 retained capacity
转为 caller-owned，不计入后续 compiler 调用；builder 仍持有的 admitted context allocation
继续只在 builder ledger 计一次。G2 必须验证 caller 保留旧 bundle 时同一 builder 重试，
并把 compiler-controlled 与 caller-retained bytes 分开报告。

后继 #298 只能从同一个已验证编译结果（Validated Compilation Output）中的 LIR 与
该伴随数据原子发射源映射；不能从已经释放的 AST/HIR/MIR 重新猜测来源，也不能让
伴随数据补齐 LIR 缺失的身份字段、所有者或关系。LIR 语义摘要不包含来源位置或来源
沿袭；来源变化可以只改变源映射精确字节，而不改变 LIR 语义或稳定标识。

## 5. 编译遍、确定性与失败原子性

### 5.1 首版编译遍顺序

```text
前端解析与类型化
  -> 校验来源模块描述符与导入图
  -> 解析命名空间、模块、符号与单位
  -> 规范化默认值与编制覆盖声明
  -> 展开合成领域专用语言构造
  -> 构造拓扑与显式规范几何
  -> 绑定信号、停车、准入和等待区
  -> 证明稳定标识所需的所有者与引用关系
  -> 按父项先于子项派生规范标识元组与稳定标识
  -> 校验其余全局语义
  -> 冻结确定性实体与关系顺序
  -> 预计算机动路径、机动门和等待区索引（不含静态路线）
  -> 冻结已验证规范低层中间表示
  -> 冻结已验证源映射输入与成功诊断
```

每个编译遍消费前一阶段的只读视图或取得其所有权，并把暂存数据与输出写入独立
区域。编译遍只有在本阶段错误级诊断为零时才能提交输出。整个编译单元只有在全部
编译遍成功时才能返回完整 `CompilationOutput`。

本顺序是 `network-compiler.md` 推荐顺序在 #292 首个纵向切片内的收窄，不是对上游
顺序的修订：来源格式升级由后继几何文档 / 导入前端接入；静态执行约束与分区规划
提示在对应后继 G1 加入同一 MIR/LIR 管线；规范制品、源映射 / 语义差异和共享静态路网的
原子发射分别由 #298/#300 交付。后继阶段只能在相应位置增加编译遍或发射器，不得在
后端补语义。

### 5.2 确定性规则

干净单工作线程编译是规范预言机：

- 来源模块图中没有依赖关系的并列节点按完整 `authoringNamespaceId` 排序；
- 符号解析不依赖声明插入顺序，显式有序领域值除外；
- 哈希表只做查找（Lookup），不直接迭代产生诊断、有类型逻辑序号或输出；
- 诊断按“来源模块规范顺序、`sourceDocumentKey` UTF-8 字节、来源位置起止行列、
  诊断代码、严重程度、有类型载荷、完整稳定键”形成全局规范顺序；
- 并行分片必须由规范键确定，合并必须使用固定树或稳定串接顺序；
- 墙钟时间（Wall Clock）、指针 / 地址、随机种子、线程完成顺序和平台 `usize` 均
  不得进入结果；
- 干净重复编译、不同哈希种子、声明置换和受支持平台必须产生相同 LIR 语义摘要；
- 后继可移植规范制品实现后，干净编译、增量编译和并行编译还必须逐字节一致。

首版实现只要求交付干净单工作线程预言机。并行和增量不得通过公共接口泄漏线程数、
分片号或缓存身份；只有实际的编译器工作负载（Workload）证明收益后才能增加
生产依赖。

### 5.3 编译资源上限

编译资源上限（Compile Limits）的精确类型 `CompileLimits` 是显式输入，不提供隐式
无限生产模式，也不得以 Rust `Default` 或来源自报字段绕过宿主选择。当前生产实现提供
`CompileLimits::p100_initial_v1()` 与 `CompileLimits::p100_initial_v2()`。调用方必须显式选择具名配置档，测试可以在包内构造
更小边界，但不能获得无限配置。字段保持私有，避免把内部阶段布局变成公共兼容面；
配置档增加维度或改变任一上限时必须提升标识符修订并重新执行边界测试。

**#315 G2 Implemented：**多文档共同接入已增加 `CompileLimits::p100_initial_v2()`，并保留
v1 的精确语义和构造器。配置档选择也是官方前端准入能力，不只是数值查表：v1 仅以
`ModuleCount` 隐式约束一模块一文档形状，
多文档模块要求 v2 或后继显式携带 `SourceDocumentCount` 的配置档。实现不得为 v1 合成默认文档上限、
自动升级或在提交后补检。#296 道路编辑来源按“一模块一个 source buffer/document”
应用同一规则。v2 及其
`SourceDocumentCount` 已经是当前编译器配置档；#296 的具体入口须按自身设计选择并
执行该配置档契约。

**#296 最小资源护栏 v1：**道路编辑来源继续执行来源字节、verifier depth/table/
apparent-size、声明/引用/关系/字符串/几何点、阶段 scratch、输出和总工作集的主要逻辑
上限；所有乘加使用 checked arithmetic，候选失败不得提交。`CompilerControlledLiveBytes`
在该入口首先是故意保守的逻辑工作集 ceiling，不宣称仅凭普通 Rust 计数器即可逐字节
约束实际 allocator。`Vec` capacity/扩容共存、`Arc` header/DST padding、HIR/MIR/LIR
阶段精确生命周期、失败诊断与 allocator/P100 正式证据由
[#374](https://github.com/illusion-tech/laneflow/issues/374) 独立交付；它阻断 #305 的
最终 cutover Gate，但不阻断 #298、#345、#351–#354。该拆分不放宽 schema、verifier、
主要规模上限、失败原子性或 canonical LIR 语义。

`LF-COMP-P100-INITIAL-v1` 以 #308 九个压力分层的逐维上包络为来源。来源 / 领域计数
取 G4 原始测量制品 `v0.10-compiler-budget-calibration-raw.json`（现仅存 git 历史
`de4cd460a96415cafbd811141568b81f74d73534`）中
`limitQualification.limitPairs[].pair.exactDimensionValue` 的压力规模最大值；总存续
与保留容量取紧凑 Evidence
`v0.10-compiler-budget-calibration-evidence.json`（同一提交）中
`results.budgetRecommendations[]` 压力规模所有适用分层的最大建议值。原始制品与
紧凑 Evidence 都由最终主线提交
`606ac52dbc75196c6d37073c72c3d48cbb031be0` 发布，并共同绑定研究来源 / 执行器提交
`de4cd460a96415cafbd811141568b81f74d73534`。精确上限如下：

| 私有配置字段                          | 精确上限 | 计数对象 / 单位                                  |
| ------------------------------------- | -------: | ------------------------------------------------ |
| `max_module_count`                    |      522 | 来源模块                                         |
| `max_import_edge_count`               |     1032 | 模块导入边                                       |
| `max_source_bytes_per_module`         |   542741 | 单个来源模块字节；与总量同界，另作入口前检查     |
| `max_source_bytes_total`              |   542741 | 编译单元来源字节                                 |
| `max_declaration_count`               |    11265 | 来源声明                                         |
| `max_typed_ast_record_count`          |    58387 | 有类型抽象语法树逻辑记录                         |
| `max_hir_record_count`                |    58387 | HIR 逻辑记录                                     |
| `max_mir_record_count`                |    38112 | MIR 逻辑记录                                     |
| `max_lir_record_count`                |    38112 | LIR 逻辑记录                                     |
| `max_reference_count`                 |    37920 | 有类型引用                                       |
| `max_relation_occurrence_count`       |    10032 | 关系出现项                                       |
| `max_identity_field_occurrence_count` |    29184 | 标识字段出现项                                   |
| `max_route_occurrence_count`          |        — | **删除**（ADR 0029：不再预编译静态路线出现项）   |
| `max_maneuver_gate_count`             |     2304 | 机动门                                           |
| `max_waiting_zone_count`              |     1536 | 等待区                                           |
| `max_geometry_point_count`            |    22368 | 规范几何点                                       |
| `max_symbol_count`                    |    11265 | 符号                                             |
| `max_string_item_count`               |    36894 | 驻留字符串项                                     |
| `max_single_string_bytes`             |       53 | 单个驻留语义字符串 / key token component 字节    |
| `max_total_string_bytes`              |   991537 | 驻留字符串总字节                                 |
| `max_diagnostic_count`                |       16 | 规范排序后保留的诊断                             |
| `max_stage_scratch_bytes`             |   304896 | 单次编译遍暂存请求字节                           |
| `max_output_bytes`                    |  2782758 | 正在构造的 LIR / 伴随输出逻辑字节                |
| `max_compiler_controlled_live_bytes`  | 43269120 | 编译器控制总存续请求字节                         |
| `max_retained_capacity_bytes`         | 36925688 | 一次编译结束后编译器实例允许保留的无语义容量字节 |

**#315 G1 Accepted 配置档：**`LF-COMP-P100-INITIAL-v2` 继承上表全部精确值并增加：

| 私有配置字段                | 精确上限 | 计数对象 / 单位 |
| --------------------------- | -------: | --------------- |
| `max_source_document_count` |     1566 | 来源文档描述符  |

该上限等于既有 `max_module_count` 522 乘以 #315 实现时采用的每模块三个来源文档容量；
它在共同接入写入文档索引前给出直接基数边界，不构成 current JSON 前端承诺。
`SourceDocumentCount` 不包含模块头或声明；它们分别继续由 `ModuleCount` /
`TypedAstRecordCount` 计数。配置档 v2 在实现中成为生产选择前，必须按第 3.3 与 10.4 节完成五级、
多文档和边界重新资格验证。

`max_single_string_bytes` 约束进入 Typed AST/HIR/诊断/source-map interner 的单个语义
字符串或 key component，不约束已经由 source-specific parser 就地拆分且不作为第二份
字符串驻留的完整 framing/reference spelling。#296 owner-qualified FlatBuffers reference
先受来源 v1 派生的 270-byte wire 上限，再就地解析为最多一个 namespace 与四个
53-byte key components；每个 component 分别消费 `StringItemCount`，实际 bytes 消费
`TotalStringBytes`，完整 wire spelling 只由 `SourceBytes*` 与 reference occurrence 计量。

单模块来源上限与总来源上限同值，是从已资格验证的总来源上限作出的失败关闭收窄；#292 G2
仍须分别验证单模块与跨模块累计边界。阶段记录数是实现预算而不是公共数据模型：生产 IR
可以使用不同 Rust 结构，但必须为每个逻辑记录定义可审计计数，且不能用拆分/合并结构
绕过总存续字节上限。上述逐维最大值不是已经测过的“所有维度同时取最大”组合；任意
输入同时逼近多个上限时，`max_compiler_controlled_live_bytes` 仍是最终全局失败边界。

受检维度包括：

- 模块数、导入边数和单模块 / 总来源字节；
- 各有类型抽象语法树、HIR、MIR 和 LIR 表的记录数；
- 关系、字段字节、来源位置与诊断数；
- 单个驻留字符串 / key component 长度和已驻留字符串总字节数；source-specific 借用
  framing 的完整长度另由其闭合语法上限约束；
- 编译期间允许的暂存区峰值字节数；
- 来源副本、字符串、各阶段表、关系、诊断、暂存区和正在构造的输出共同形成的
  编译器控制总存续内存峰值，以及编译结束后允许保留的内部容量。

同一个不可变 `CompileLimits` 必须贯穿三个受检边界：

1. `SyntheticModuleBuilder` 在字符串、来源位置、声明和规范化调用记录扩容前执行
   单模块上限；
2. `CompilationUnitBuilder` 使用构建器实测且不可由调用方修改的计数，执行模块、
   导入、总来源字节、总驻留字符串、模块/文档索引和模块包装的累计存续上限，并在
   冻结模块图前预检拓扑排序、环检测、规范重排的阶段 scratch 与总共存峰值；
3. `Compiler` 在每个编译遍分配后继表或扩大暂存区前执行 AST/HIR/MIR/LIR、关系、
   诊断、暂存内存和总存续内存上限。

模块已经在更宽上限下构造不表示可以绕过编译单元上限；
`CompilationUnitBuilder::add_synthetic_module` 必须以受检加法重新核对累计计数。
库内受控分配以请求容量字节记账；调用方在进入 API 前自行持有的外部内存不计入编译器
峰值，但复制 / 驻留进编译器后的字节必须计入。各分项均未超限不能替代总存续内存
核对；所有请求容量和累计字节使用受检加法，平台无法表示请求值时必须在分配前失败。
该段描述 #292/#315 既有共同编译器会计目标；#296 道路编辑来源在 #374 完成前按上文
“最小资源护栏 v1”的保守逻辑 ceiling 验收，不把每个 allocator allocation 的精确相等
关系作为 #296 Delivery 前置。

`max_diagnostic_count` 限制保留的诊断记录数，不限制仍可安全检查的诊断候选数。每个
已执行编译遍必须检查其输入中全部可安全判定的候选，并用按第 5.2 节全局规范顺序
比较的有界最小项收集器只保留最小十六项；收集器已满时，规范顺序更小的新候选必须
替换当前最大项，不能因“已收满”提前终止遍历。发现第十七个候选后只设置稳定的
`diagnosticsTruncated` 状态；该状态不额外占用诊断记录。编译结束时把保留项升序
输出，因此结果恰好是全部已检查候选的规范前缀，同时附带截断状态。首版可以用容量
固定为十六的标准库最大堆或语义等价实现，额外空间为常数，不得先物化全部候选。

前一阶段的结构错误仍可按正常失败原子性阻止依赖无效输出的后继编译遍；诊断上限
本身不能成为停止当前可安全遍历的理由，也不要求为收集更多诊断而构造无效阶段。
任一计数无法表示为 `u32` 时在分配后继大区块前失败。测试至少覆盖单项边界、边界
加一、跨模块累计溢出、暂存区增长拒绝、跨多个已执行编译遍产生超过十六个候选且
后发现的较小规范键能够替换已保留项，以及连续三十二次失败后复用同一编译器实例
仍与新实例结果一致。#308 的 16 GiB/24 GiB/60 s 研究停止护栏不进入
`CompileLimits`。

## 6. 合成领域专用语言前端

### 6.1 来源与复现

合成领域专用语言是 Rust 程序化来源，不是交换格式（Interchange Format）。一个
可发布的合成来源模块必须显式提供：

- `authoringNamespaceId`；
- 生成器构建标识；
- 参数与输入摘要；
- 前端版本和选项摘要；
- 调用方随机种子（如果生成过程使用随机性）；
- 来源沿袭。

领域专用语言构建器按规范化调用记录和参数形成可重放的合成来源记录，并由
`SyntheticModuleBuilder::finish` 计算来源内容摘要，调用方不能自报摘要或单独配对
来源模块描述符。首版合成来源记录使用 `frontendVersion` 版本化的确定性长度前缀
编码，精确字节以 `LFSOURCE` 魔数 / 域前缀开头，并对完整记录精确字节计算 SHA-256
形成 #292 已接受的 `sourceContentDigest`；编码变化必须提升前端版本并更新已知向量。
现行合成前端为 `frontendVersion = 3`：准入后交通一维以整数毫米 / 受检 `f32` SI
写入来源记录，不再写编制 `f64`。
调用机器的绝对路径、墙钟时间和指针地址不进入该记录。
该摘要只服务来源沿袭和重放，不参与实体稳定标识。测试可以使用显式 `test_only`
来源模块头；该能力不得进入发布接口。

领域专用语言构建器的声明方法使用 `#[track_caller]` 或等价宏捕获 Rust 文件、行和
列作为来源位置。#292 已接受的每个来源模块另有调用方提供、与机器路径无关的稳定
`sourceDocumentKey`；该键在整个编译单元内必须唯一，并受 `CompileLimits` 的
`ASCII` 字符集与长度上限约束，重复键在进入诊断排序前失败。
`file!()` 结果只作本地显示和来源沿袭。循环展开的多个声明可以共享调用位置，但诊断
必须同时携带声明的显式稳定键；来源位置、文件路径和展开序号都不得参与标识。

**#315 G1 Accepted：**在不原地改义上述字段的前提下迁移为逐文档形状：每个 `SyntheticModule`
恰有一个 `SourceDocumentDescriptor`，其 `sourceDocumentDigest` 必须逐字节等于既有
`sourceContentDigest`；再按第 3.3 节从单文档描述符派生模块级
`sourceDocumentSetDigest`。构建器把 `sourceDocumentKey`、摘要和 `LFSOURCE` 精确记录
长度不可分绑定到该描述符；后继多文档官方前端也为每份文档派生独立描述符并沿用
编译单元级唯一性规则。该迁移已由 #315 G2 落地，并以逐文档与文档集两类已知向量锁定。

### 6.2 首批支持矩阵

| 领域           | #292 首批支持                                                                                                                                     | 明确拒绝 / 后继                                                  |
| -------------- | ------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------- |
| 模块           | 稳定命名空间、显式导入、来源沿袭、跨模块有类型引用                                                                                                | 网络 / 文件系统隐式发现、匿名可发布模块                          |
| 车道拓扑       | 显式 `laneEdgeKey`、长度、限速、后继关系、自环、合法孤立边                                                                                        | 从坐标或数组位置生成边键                                         |
| 横断面         | 道路走廊（`RoadCorridor`）、道路区段（`RoadSection`）、编制车道（`AuthoringLane`）、车道组（`LaneGroup`）、设施带（`FacilityBand`）及唯一所有者树 | 动态车道用途、多执行域运行时行为                                 |
| 路口           | 路口（`Junction`）、通行流向（`Movement`）、机动路径（`ManeuverPath`）、入口 / 内部 / 出口边及共享内部边角色                                      | `ConflictZone`、`ParticipantStream`、`JunctionGroup`             |
| 机动门与等待区 | 多机动门（`ManeuverGate`）、停止线（`StopLine`）、等待区（`WaitingZone`）及路线出现项                                                             | 等待运行时行为、`ConflictArbiter`、通行权策略                    |
| 信号           | 当前态定时信号组（`SignalGroup`）、信号控制器（`SignalController`）、信号相位（`SignalPhase`）及机动门控制绑定                                    | 感应控制、宿主回调或未冻结的控制器类型                           |
| 停车           | 停车区域（`ParkingArea`）、停车位（`ParkingSpace`）、入口 / 出口锚点和当前态静态几何                                                              | 运行时预约 / 生命周期策略                                        |
| 横断面准入     | 参与者类别（`ParticipantClass`）、准入规则（`AccessRule`）、静态继承 / 准入及当前态车辆投影                                                       | 把 `ParticipantClass` 当执行域；未实现非机动车 / 行人 / 轨道行为 |
| 车辆配置       | 当前态既有车辆跟驰模型的 `VehicleProfile` 静态参数                                                                                                | 把车辆配置提升为所有交通执行域的公共基类                         |
| 静态路线       | **拒绝**：编制来源不得声明路线                                                                                                                    | 每世界 `register_route`；路径规划策略（#303）                    |
| 空间           | 显式规范坐标框架（`CanonicalFrame`）和已量化规范 `f32` 折线、长度 / 连续性校验                                                                    | 曲线、高程求值、曲线细分（tessellation）和几何文档前端           |

首批领域专用语言必须能表达标识 v1 修订 1 的全部实体种类。支持“声明该实体”不表示对应
目标交通运行时执行域或动态能力已经实现。

### 6.3 迁移场景

G1 曾冻结两个 current JSON 固定样例的等价迁移。这些 JSON 文件已随 #301 删除。
编译器正确性继续使用编译器原生有类型模块；不得把已删除 JSON 当作现行夹具或
预言机。走廊 catalog 仍为
`examples/data/v0.2-signalized-corridor.catalog.toml`。

当时覆盖的领域仍必须能由合成 DSL / 道路编辑前端表达：完整车道拓扑、横断面 /
准入、信号、停车、规范几何，以及单个机动路径的多机动门与等待区。
自环边、合法孤立边和重复边出现项都不依赖道路区段或路口所有者。路线出现项不由
编制来源声明（ADR 0029）。

## 7. 标识 v1 首次实现

### 7.1 公共值与独立实现

`laneflow-static-contract` 冻结：

- `StableId128([u8; 16])` 与有类型包装；
- `identityEncodingVersion = 1`；
- `identityRegistryRevision = 1`；
- 实体种类代码 / 英文短名、字段标签代码 / 编码和必需标签序列；
- `LFID` 魔数、文本形态规则和 `BLAKE3` 域分隔字节。

`laneflow-compiler` 独立实现：

- 对必需标签的严格校验；
- 规范字节流式编码；
- `BLAKE3-128` 派生；
- `StableId128` 到完整 `CanonicalIdentity` 及其所有者来源位置的登记；
- 父项先于子项的标识闭包。

#292 已提供不复用编译器编码器的 `xtask` / 测试预言机生成和校验已知向量；该
测试 oracle 继续保护标识规范，但不构成 production validator。#299 后发射检查只消费
最终 LFCA/LFSM/LFSD 字节并执行 ADR 0024 冻结的摘要、长度、路网修订和跨对象绑定检查。

### 7.2 碰撞与重复

- 两个稳定实体声明产生相同完整规范标识元组：
  `DuplicateCanonicalIdentity`；
- 相同 `StableId128` 对应不同完整规范标识元组：
  `IdentityDigestCollision`；
- 未知 / 重复 / 多所有者 / 零所有者在子实体标识派生前失败；
- 不允许追加序号、盐值（salt）、后缀（suffix）或遍历位置修复；
- `XXH3-64/128` 只能作为编译器进程内缓存 / 指纹，命中后比较完整键；
- `XXH64` / `FNV64` 不承担持久标识。

### 7.3 验收

- 修订 1 的每个实体种类至少有一个纳入版本控制的已知向量；
- 缺失、重复、未知、乱序标签和错误字段长度的负向向量；
- 同级重排、无关插入、仅几何编辑的变形测试（Metamorphic Test）；
- `LaneEdge` 覆盖 / 内部角色变化时身份不变，显式替换 / 拆分时使用新键；
- `RoadSection` / `FacilityBand` 的多所有者、零所有者和跨所有者移动测试；
- 编译器编码器与独立预言机的规范字节 / 标识一致；
- 已知向量文件在全部受支持平台逐字节一致。

## 8. 诊断与来源位置

`DiagnosticBundle` 的规范判断依据是稳定诊断代码、严重程度、有类型载荷和来源位置，
不是渲染后的自然语言字符串。首版至少包含：

- 模块、导入、命名空间、符号、引用和单位；
- 重复、未知、多重归属和无所有者；
- 标识编码和摘要碰撞；
- 拓扑、几何、长度和覆盖关系；
- 信号、机动门、等待区、停车和准入；
- 有类型逻辑序号、资源上限和诊断截断。

一个诊断可以有一个主要来源位置和多个关联来源位置。跨模块重复、所有者冲突和引用
错误必须同时指向声明与冲突来源。中文渲染是权威说明；英文辅助渲染不能改变诊断代码
或有类型载荷语义。

公共来源位置是闭合和类型 `SourceLocation`：`Text(SourceSpan)` 保留
“`sourceDocumentKey`、起始行/列、结束行/列”的现有受检 `u32` 表示；
`RoadEditing(RoadEditingSourceLocation)` 使用 module/document、稳定声明或 owner-local
subject、由 1..=4 个已知 table field / struct member / union variant step 形成的闭合叶
property path，以及可选 interned canvas selection。只有 FlatBuffers 结构损坏可以携带
已证明在输入内的 byte range 和物理 vector fallback；领域诊断不得伪造行列或依赖数组
位置。精确类型、规范顺序和资源计量由
`road-editing-source-and-geometry-frontend.md` §9.7 冻结。

#292 已接受的 `sourceDocumentKey` 仍是编译单元内显式、稳定、与机器路径无关的 ASCII
键；合成领域专用语言使用该键、调用位置和声明的显式稳定键，文本前端使用真实文本
范围，道路编辑来源使用有类型实体/属性位置。宿主路径只服务显示和来源沿袭，不参与
规范标识、规范排序、可移植规范制品或语义差异。

**#315 G2 Implemented：**每个位置的文档键必须存在于所属逻辑模块的文档描述符集，并解析
为独立来源文档序号；来源模块序号不能替代或推导文档序号。该多文档解析规则已经由
跨文档来源映射测试证明。

## 9. 集成专用的已验证规范低层中间表示到当前态投影

`laneflow-compiler-test-support` 只接受 `&ValidatedCanonicalLir`，并产生：

- 当前态 `InitialTrafficData`；
- 当前态 `SpatialRegistry` 所需输入与绑定；
- 用于等价断言的稳定映射报告。

投影固定遵守：

- LIR 已完成的稳定标识、所有者、拓扑、长度、出现项和顺序语义不在投影中重算；
- 当前态构造器仍可执行自己的防御校验，但其结果不是编译器语义验证的实现；
- 当前态句柄 / 登记表序号只存在于投影结果，不回写 LIR；
- 投影对象的 current external ID 使用对应 `StableId` 的规范文本，避免不同来源模块的
  owner-local key 在当前全局 ID 空间碰撞；稳定映射报告负责关联 LIR ordinal、
  `StableId` 与 current external ID，当前没有独立 ID 的 `SectionLane` 明确记为无映射；
- 当前 `SpatialRegistry` 只能完整绑定一个 `CanonicalFrame`；无几何 LIR 投影为无
  `SpatialRegistry`，多 frame LIR 在本迁移桥显式失败，不把该限制回写为编译器契约；
- 不读取当前态 `Traffic v0.10` `JSON` 形成目标态语义；
- 投影不成为 `laneflow-compiler` 功能特性（feature），不被生产编译发射器复用；
- #301 拆除 current Core/JSON 运行时入口时删除。

调整后的投影验证矩阵覆盖：

| 证据层     | 比较内容                                                                 |
| ---------- | ------------------------------------------------------------------------ |
| 静态语义   | 实体、稳定关系、有序关系、长度、信号、停车、准入和等待区出现项           |
| 当前态构造 | 当前 Core/Spatial 构造器全部成功，投影关系可从公开查询面逐项核对         |
| 确定性     | 重复编译语义指纹和 LIR ordinal / StableId / current external ID 映射一致 |
| 空间层     | 规范坐标框架、车道图边绑定以及采样位置与切向                             |

旧 `LF-COMP-CURRENT-EQUIV-v1` 曾覆盖的车辆逐步行为、事件和 current JSON
对照只保留为 #292 历史证据，不再是调整后 #297 的活动验收要求。运行时行为由对应 Core /
目标 Traffic Runtime 切片验证；编译器资源基线必须使用编译器原生 workload 另行建立。

### 9.1 当前投影证据

`projection_equivalence` 集成测试直接构造一个覆盖两条可区分所有权链、共七条边、两套
横断面与路口关系、四道机动门、两个等待区、两套固定时制信号、两组停车所有权、
参与者分类、准入、两套车辆配置、两条路线和规范几何的编译器有类型模块，然后验证：

- Canonical LIR 的全部代表性实体数量、完整映射报告和路线出现项；
- 车道拓扑、横断面所有者、路口/机动路径、停止线/门/等待区、信号相位、停车、参与者
  继承、准入裁决、车辆配置与路线序列逐项投影语义；
- 七条边的代表性几何位置、切向和局部基采样；
- 重复编译语义指纹和投影映射确定性，并对两次投影执行同一套有类型语义断言。

测试不依赖 `laneflow-data` 或 `serde_json`，也不从 current JSON 生成编译器输入。旧
loader 的正确性由其自身测试负责。
## 10. 容器、依赖与性能选择

### 10.1 已冻结选择

- 有类型抽象语法树、HIR 和 MIR 使用自有、仅追加的 `Vec<T>` 有类型区块分配器；不
  需要删除或代次，因此不使用 `slotmap` 一类代次槽位；
- 区块分配器插入时完成受检的 `u32` 转换，LIR 冻结为连续不可变表；
- 规范顺序保存为显式 `Vec` / 区间，查找表不拥有迭代顺序；
- 标识摘要使用直接依赖 `blake3`；
- 本议题不引入归档框架、零复制框架、增量数据库或并行框架。

这些选择不排除后继在有证据时加入并行 / 增量实现；它们禁止为了“将来可能需要”先
把代次、锁、特征对象或目标应用二进制接口塞进每个记录。

这里的“已冻结”不绕过 ADR 0020 的独立实现基准要求。自有 `Vec<T>` 区块分配器是
不增加第三方区块分配库的标准库基线，仍须在 #292 工作负载中报告分配、峰值内存和
缓存局部性；若不能满足 G1 冻结的预算，应回到 G1 重开选择。直接依赖 `blake3` 是
实现标识 v1 已接受的 BLAKE3-128 算法，不是用基准重新选择持久标识算法；该依赖仍须
完成 MSRV、许可证、维护、安全和依赖树审计。

### 10.2 #308 候选结论与 G2 实现选择

#308 的三十条候选比较没有得到可重复改善：二十五条处于噪声范围内，一条证据不足，
四条为可重复回退。因此 #292 G1 不为一次性研究继续扩大候选矩阵，也不因本轮结果
新增 `xxhash-rust`、`hashbrown` 或 `indexmap` 生产依赖。首版实现冻结为：

- 调用方可控的命名空间、符号和文档键使用标准库随机种子 `HashMap`，完整键相等是
  正确性条件；
- 规范输出顺序使用显式 `Vec` 排序；冻结后只读、且实际查找占优的内部表可以使用
  排序 `Vec` + 二分查找，但不得改变规范顺序；
- 持久 `StableId128` 继续使用身份 v1 已接受的 BLAKE3-128；它不参与内部容器竞选；
- `XXH3`、XXH64、FNV-1a64 与确定性桶/基数排序均不作为首版默认实现。

G2 只需测量真实生产实现与标准库基线，不再强制重跑 #308 的完整候选矩阵。只有实际
实现无法满足第 11.3 节预算，或新候选会改变公共接口、确定性语义、拒绝服务边界或
依赖方向时，才携带精确分层证据回到 G1。纯私有、依赖中立且不改变预算的等价优化可在
G2/G3 审阅中处理。新增依赖仍须记录最低支持 Rust 版本（Minimum Supported Rust
Version，MSRV）1.96、许可证、维护状态、安全记录和依赖树。

### 10.3 并行与增量

#292 不以小型合成场景编译的虚假加速比冻结并行框架。只有满足以下条件才新增生产
并行：

- 在冻结的大型编译器工作负载上有稳定、可复现的墙钟时延收益；
- 单线程和并行 LIR 语义摘要完全一致；
- 后继可移植规范制品可用后必须逐字节一致；
- 峰值内存、任务调度和合并成本同时报告；
- 并行编译遍不把工作线程 / 分片身份写入诊断或输出。

增量能力同样必须以干净编译为预言机；缓存未命中 / 命中只影响复用，不改变诊断、
LIR 或后继制品。

### 10.4 #315 共同接入性能与验证边界（Accepted）

#315 不进入交通运行时（Traffic Runtime）固定步进热路径，但仍不能以“离线”作为重复
工作和无界内存的豁免。G2/G3 必须在同一 P100 机器、相同发布配置（release）和相同
`LF-COMP-P100-PRODUCTION-R0-v1` 五级工作负载上形成基线/候选（base/candidate）配对证据：

- 既有 `Compiler::compile` 计时边界继续覆盖完整 HIR→MIR→LIR→源映射管线，确认共同
  表示没有引入记录级分派或额外转换；
- 新增接入边界样本只计时“已完成前端构造的具体模块 → `add_*_module` → `build`”，
  模块来源编码与声明构造在停表外，避免把不同前端解析成本归给共同接入；
- 前端 `finish` 另以计数证明每份原始文档只执行一次 SHA-256；文档集摘要只扫描按键排序后的紧凑
  描述符，`SourceDocumentCount` 只增加常数次候选累计/比较，不允许再次扫描原始载荷；
- 逐文档来源记录只进入冷的源映射伴随数据；重复来源字符串可以共享驻留，文档只保留
  紧凑引用。基线/候选必须分别报告构建器存续、成功结果存续、冻结 scratch、准入控制峰值
  和来源记录字符串字节，不能用成功结果存续冒充准入峰值；且不得把宿主
  路径、来源声明或来源表序号写入 LIR 语义摘要、稳定标识或文档集摘要；
- 每级至少 1 次预热和 7 次正式样本，报告中位数、中位数绝对偏差（Median Absolute
  Deviation，MAD）、编译器控制峰值字节和保留容量；不因本次私有重构另立城市容量或
  产品服务等级协议（Service-level Agreement，SLA）；
- 基线/候选的 LIR 语义指纹、规范模块顺序、来源位置、源映射和诊断必须语义等价；既有 Synthetic
  `sourceContentDigest` 必须逐项等于候选单文档的 `sourceDocumentDigest`，新增文档集摘要则必须匹配
  独立 v1 聚合预言机，不能要求它与单文档摘要相等。任何可重复时延或内存回退都必须定位到具体阶段并
  修复，或携带事实回到 G1；
  预期的来源记录在 `finish` 后释放若形成内存改善，也必须由阶段生命周期计数证明，
  不能只看进程噪声。

自动化验证至少覆盖：现有合成来源公共 API；加入顺序变形；命名空间/来源文档
冲突与共同接入适用的全部 `CompileLimits` 维度；一个逻辑模块保留三个来源文档、其中一个文档无声明、
声明/关系位置分别指向不同文档且源映射精确保留；三个文档拥有可区分的来源记录且原始字节释放后仍可
查询；文档序号不从模块序号推导；接入失败后构建器不污染；
未知导入与循环；测试专用第二种官方封装复用同一私有接入而不复制检查；重复性 / 变形测试；
公开面不存在通用模块/特征构造入口；compile-fail 测试证明调用方不能构造或重配模块
摘要、描述符、来源记录和位置。测试专用封装不成为生产 `SourceLanguage` 或
第三方兼容承诺。

#315 的自动化验证继续覆盖共同多文档接入、来源记录、失败原子性、资源计数和规范顺序。
#297 原 current 严格导入、位置表、跨 crate 导入器与三文档资源配置验证已经取消，不再
作为共同接入的后继验收项。
## 11. 测试、工作负载与基准

### 11.1 正确性测试

- 每个编译遍的正向、首错误和多错误稳定排序；
- 成功路径保留警告 / 提示，错误路径不返回部分 LIR 或部分源映射输入；
- 模块、导入和引用循环；
- 来源模块描述符由构建器派生、`sourceContentDigest` 已知向量、重复
  `sourceDocumentKey`；
- 表、区间、有类型逻辑序号、模块 / 编译单元累计资源上限和暂存区边界；
- 标识 v1 已知向量和变形测试；
- 声明置换与无关插入；
- 属性测试（Property Testing）：有效所有者树、路线 / 路径序列和平面区间往返
  一致性；
- 模糊测试（Fuzz Testing）边界：合成领域专用语言构造接口、包内有类型抽象语法树
  接收器、标识字段、来源模块图和编译资源上限；
- 编译器不依赖当前核心 / 空间层的依赖图检查；
- 以编译失败测试（Compile-fail Test）证明未验证阶段无法调用集成专用投影或编译
  发射器，以及调用方不能构造描述符 / 模块错配。

#315 的 `SourceDocumentDescriptor`、逐文档/文档集摘要已知向量和一模块多文档独立
源映射测试只属于第 10.4 节的 **#315 G1 Accepted** 验证增量；它们不倒改上述 #292
已接受的历史通用正确性测试矩阵；#315 G2 已增加并通过这些验证。

### 11.2 编译器工作负载

编译器工作负载只按编制 / 中间表示对象计数，不使用车辆、交通参与单元或
`Agent` 数量。当前固定样例、编译器校准规模（Compiler Calibration Scale）与
编译器压力规模（Compiler Stress Scale）承担不同证据职责：

| 工作负载标识符             | 证据角色        | 目的                                         | 主要计数对象                       |
| -------------------------- | --------------- | -------------------------------------------- | ---------------------------------- |
| `LF-COMP-ID-v1`            | 合成校准 / 压力 | 标识 v1 全部实体种类 / 标签与父项闭包        | 模块、稳定实体、标识字段 / 字节    |
| `LF-COMP-CORRIDOR-v1`      | 合成校准 / 压力 | 线性走廊、信号 / 停车 / 横断面扩展           | 边、关系、路线、信号状态、几何点   |
| `LF-COMP-JUNCTION-GRID-v1` | 合成校准 / 压力 | 密集路口、路径 / 机动门 / 等待区出现项与索引 | 路口、路径、机动门、等待区、出现项 |

投影正确性由第 9.1 节的编译器原生夹具承担，不定义 current JSON 性能工作负载。

| 工作负载 | 模块图配置档       | 基础规模 `B` | 正式五级规模      | 校准规模 | 压力规模 |
| -------- | ------------------ | -----------: | ----------------- | -------: | -------: |
| 标识     | 宽星形 / 深链形    |           32 | 32/64/128/256/512 |      128 |      256 |
| 标识     | 共享汇入有向无环图 |           32 | 32/64/128/256/512 |      256 |      512 |
| 走廊     | 三种配置档         |            1 | 1/2/4/8/16        |        8 |       16 |
| 路口网格 | 三种配置档         |            4 | 4/8/16/32/64      |       32 |       64 |

回归默认只运行基础、校准和压力三级；完整五级阶梯只在预算重校准或性能原因分析时运行。
合成规模不是中国特色城市工作负载、产品容量、运行时交通参与单元规模或最低硬件认证。

每次报告至少包含：

- 输入模块 / 来源字节和各阶段记录数；
- 每个编译遍的墙钟时延与总编译时延；
- 分配次数、峰值字节数和每阶段保留字节数；
- 标识编码 / 哈希、符号解析、排序和出现项预计算的分项成本；
- 冷实例编译（Cold-instance Compile）和稳定容量复用编译（Stable-capacity Reuse
  Compile）；
- 输出 LIR 表 / 关系 / 标识字段字节数；
- 失败输入在限制边界的最大工作量。

### 11.3 #308 G4 证据与 #292 首轮性能门槛

本节保存 #292 原 G1 的研究输入和判定方法。第 11.4 节确认其生产适用性缺口后，第
11.5 节的 append-only G1 修订已经取代其“产品通过门槛”角色；本节数值现只作容量
估算和实现选型输入。

#308 已完成 G4。其原始测量、机器可读 Evidence 与报告冻结了研究替身的 P100 同机
R0 研究基线；产品负责人另行把同一台 `LF-P100-REF-01` 物理机器选定为目标产品推荐参考
机型。硬件选择不把历史 R0 自动升级为产品通过，但允许 #292 在同一物理基准上建立
首轮生产门槛。该门槛标识符为 `LF-COMP-P100-R0-v1`。

`LF-COMP-P100-R0-v1` 只消费校准规模与压力规模的 126 条精确预算建议。自然身份是
`(workloadId, graphProfile, n, sampleKind, binaryMode, metric)`；每个生产基准样本必须
逐项查找 G4 提交中 `v0.10-compiler-budget-calibration-evidence.json` 相同自然身份的
`results.budgetRecommendations[]`，不得用跨工作负载平均值掩盖单项回退。下表仅给出
便于人工审阅的逐等级最大值，不能替代机器可读逐项门槛：

| 等级 | 冷实例墙钟 ns | 复用墙钟 ns | 冷实例存续峰值 B | 复用存续峰值 B | 保留容量 B | 完整进程私有内存 B |
| ---- | ------------: | ----------: | ---------------: | -------------: | ---------: | -----------------: |
| 校准 |      30986300 |    21751700 |         18752072 |       21650640 |   18477896 |           56187294 |
| 压力 |      49878400 |    45679900 |         37474040 |       43269120 |   36925688 |          100798860 |

G2/G3 在 P100 上执行生产干净单工作线程编译时必须遵循以下判定：

1. 使用第 11.2 节相同生成规则、模块图、规模、冷实例 / 稳定容量复用边界与计时边界；
2. 语义正确性先于性能，生产 LIR 摘要不能与研究摘要直接比较，必须由编译器原生
   workload 和独立预言机证明；
3. 校准 / 压力规模的每个适用性能分层不得超过对应 R0 建议；完整进程私有内存只能按
   完整进程样本比较，不能拆成冷实例 / 复用值；
4. 首次生产实现增加必要语义后若超出任一门槛，结果不是实现失败的永久结论，也不得
   静默放宽；必须提交精确分层、必要语义差额与优化判断，回到 G1 冻结新修订预算；
5. 后续优化以首次通过的生产提交另建生产基线，不能继续把研究替身冒充产品预言机。

墙钟门槛已经包含 `26302/22185` 的全局重复性包络；存续峰值和保留容量包络为 `1/1`，
完整进程私有内存包络为 `1241/559`，不得再次乘包络。#308 没有保留可用于决策的增长
斜率，因此 #292 不伪造斜率预算；三级日常回归与五级原因分析已经给出足够的非线性
监测边界。玩家在线编辑停顿、最低/替代硬件、中国特色城市工作负载、增量/并行编译、
可移植制品发射和共享静态路网构建仍不在本门槛内。

研究停止护栏只保护研究机器，不进入生产门槛。不得把运行时交通参与单元规模、多世界
吞吐或研究替身的进程数量换算成编译器产品容量。预算来源、计数对象、单位、P100
硬件身份与证据提交必须保留可追溯性。

### 11.4 G2 实现核对发现的门槛适用性缺口

G2 把 `compiler-calibration-workloads-v1.json` 逐项映射到真实生产语义时确认：第 11.3
节冻结的“相同生成规则 + 相同自然身份”目前不可成立，因而尚不能产生诚实的 126 条
生产通过记录。精确差额是：

- `LF-COMP-ID-v1` 每工作负载单元只登记 1 条 `LaneEdge`，同时登记
  `ManeuverPath`、`ManeuverGate` 和 `WaitingZone`。生产语义中的完整机动路径必须同时
  表达入口边、内部边和出口边；同一条边不能兼任边界与内部角色，因此该抽象记录组合
  不能降为合法 `Synthetic DSL`；
- 模块图配置档的 `crossModuleReferences` 是研究阶段的独立抽象记录。生产前端
  （frontend）
  只有由具体领域声明拥有的有类型引用；为凑齐该记录而新增 `StaticRoute` 等声明会改变
  声明数、关系数和实际编译工作，不能继续使用原自然身份；
- #308 的阶段记录、受控分配和保留容量来自研究替身模型，不是生产
  `typed AST → HIR → MIR → Canonical LIR` 的观测值。把两者按同名指标直接判定会违反
  第 11.3 节“研究替身不得冒充产品预言机”的边界。

因此，`LF-COMP-P100-R0-v1` 仍是资源上限与实现选型的研究输入，不是 #292 当前提交的
产品通过结论。进入整体 G3 前必须回到 G1，二选一冻结后才能建立规模扩展生产基线：

1. 修订三项生产工作负载，使每个规模都能构造完整合法领域语义，并以新工作负载修订
   重新测量生产门槛；
2. 将 #308 明确降为非门禁的容量估算输入，另以真实产品场景和生产编译管线建立首轮
   基线。

不得通过忽略不适用分层、复用原自然身份或只测两个固定样例来伪造生产性能通过。两个
固定样例的迁移等价证据仍由第 9.1 节独立成立。

### 11.5 G1 修订后的首轮生产基线

#292 的 append-only [G1 性能证据边界修订][g1-performance-revision] 已选择第 11.4 节
方案 2：`LF-COMP-P100-R0-v1` 保留为非门禁容量估算输入，真实生产
`Compiler::compile` 使用 `LF-COMP-P100-PRODUCTION-R0-v1` 形成首轮描述性基线。

工作负载 `LF-COMP-PRODUCTION-CORRIDOR-v1` 把完整信号化走廊 Traffic + Spatial 样例
复制到独立命名空间。首次试运行证明第 6 份走廊会让 `SourceBytesTotal=560374` 超过
`LF-COMP-P100-INITIAL-v1` 的 `542741` 字节上限；[G1 规模阶梯更正][g1-scale-correction]
因而冻结 1、2、3、4、5 份完整走廊，不放宽资源配置档。

P100 正式测量对每级执行 1 次预热和 7 次正式样本；输入构造在停表外，唯一计时区只
覆盖 `Compiler::compile`。全部 35 个样本成功且同级语义指纹一致；1→5 份走廊的墙钟
中位数为 2.2446→11.3253 ms，含源映射冻结的全管线编译器控制峰值为
642529→3211621 字节，保留容量恒为 0。完整紧凑证据与环境边界见
`v0.10-compiler-production-baseline.md` 和配对 JSON。
该结果成为后继同机生产回退对照，不是产品 SLA、城市容量或 #298 制品发射预算。

## 12. #292 G1 接受结果与 G2 前置

#292 G1 已确认：

- [x] 公共接口、第三方前端非承诺边界、阶段类型私有性和包依赖有向无环图通过本地
      架构审阅；
- [x] 合成领域专用语言前端的支持 / 拒绝矩阵与两个迁移场景闭合；
- [x] 标识 v1 值类型、独立编码边界和已知向量计划闭合；
- [x] #308 已按自身 Gate 完成非生产校准研究；`CompileLimits` 与容量估算输入已经由
      精确提交和机器可读 Evidence 冻结；G2 发现后的生产性能边界由第 11.5 节追加
      G1 修订；
- [x] 哈希 / 缓存候选结论、拒绝服务边界和依赖审计要求已冻结；首版保留标准库基线，
      真实生产实现结果进入 G2/G3；
- [x] 集成专用投影的静态语义、行为、事件、确定性和空间层等价矩阵闭合；
- [x] ADR 0020、`network-compiler.md`、设计索引和术语引用一致；
- [x] 全面本地文档 / 架构审阅没有未处置发现；
- [x] 对 G1 当前精确提交（exact head）取得有效外部审阅者的干净完成态审阅；
- [x] #292 新增仅追加的 `## G1 设计判断` `Pass` 评论。

`G1 Pass` 只冻结实现输入。实现开始前仍须复核 GitHub 元数据 / 原生依赖、记录 G2，
并让实时 Project 状态与 G2 一致；具体状态只从 GitHub 读取。不得把本地分支、G1
启动评论或本文的 Accepted 状态单独当作 G2 授权。

## 13. #292 G4 完成状态

本节是 #292 当时的完成证据快照。2026-08-10 删除 current JSON 测试转换器并改为
编译器原生投影夹具，不倒改当时已经成立的 G4 事实。

#292 已依次记录 [G2 Pass](https://github.com/illusion-tech/laneflow/issues/292#issuecomment-5160723273)、
[G3 交付确认](https://github.com/illusion-tech/laneflow/issues/292#issuecomment-5177504302) 与
[G4 完成判断](https://github.com/illusion-tech/laneflow/issues/292#issuecomment-5177635981)。
截至 G4 合入 `main@c20308cd4c2d311862f7b4c9aaa1d6768e62406e`：

- [x] 三个包、编译阶段、G1 首批领域语义、Identity v1、诊断和资源上限已经实现；
- [x] 集成专用投影与两个 `LF-COMP-CURRENT-EQUIV-v1` 样例已经形成静态、空间、行为、
      事件和重复编译证据；
- [x] 工作区编译检查/测试/文档测试（workspace check/test/doctest）、三个新包严格
      Clippy、发布配置测试（release tests）、rustdoc、cargo-deny 与 Markdown 表格检查
      已经完成；
- [x] 第 11.4 节的不适用差额已经取得仅追加（append-only）G1 修订，并形成五级
      `LF-COMP-P100-PRODUCTION-R0-v1` 生产基线；
- [x] Related PR #307 与 Delivery PR #314 已通过各自 G3、正常检查和精确头
      （exact-head）外部干净审阅后合入，#292 已完成 G4 并关闭。

本节只同步 #292 的已完成事实。#315 已接受增量不得倒改 #292 的 G1/G2/G3/G4 证据，也不能
把 #292 的生产基线自动解释为新前端的解析成本预算。

## 14. #315 G1 接受结果与 G2 前置

本节保留 #315 当时的 G1 输入；其中为 #297 预留的 current 严格导入、位置表、资源
配置和包依赖目标已于 2026-08-10 被 `current-package-import.md` 取代，不再是活动设计。

- [x] `CompilationUnit` 只保存共同 `TypedAstModule`，HIR 不再依赖
      `SyntheticModule` 或前端变体；逻辑模块与一个或多个来源文档分离建模，源映射不从
      模块序号推导文档序号；
- [x] 具体官方模块、私有共同接入、来源记录释放时点与失败原子性已经冻结；
- [x] 命名空间/文档、导入图、全部资源维度和规范模块顺序只有一个实现权威；
- [x] `LF-COMP-P100-INITIAL-v1` 保持不可变；v2 的 `SourceDocumentCount`、三文档容量推导、
      五级/边界重新资格验证和配置档版本迁移已经冻结；多文档准入只接受显式具备该维度的配置档，
      v1 不合成默认值或自动升级，固定三文档 current 在解析前拒绝 v1；
- [x] #296/#297 的具体受检入口与 `laneflow-current-source` /
      `laneflow-current-import` 目标包依赖图闭合；compiler 只在默认关闭、可退役的迁移
      特性下依赖 current-source 的字段私有受检能力，importer 以 compiler 为唯一 LaneFlow 直接依赖、
      第三方直接依赖限于已冻结支撑白名单且公共输入签名不泄漏 current-source 类型；两者均不依赖
      当前态 Core/Spatial 对象图；
- [x] `laneflow-current-source` 是场景清单到原始 Traffic/Spatial 制品精确绑定和线格式
      解析的唯一权威；当前态生产兼容策略不新增制品数、引用长度或引用总字节拒绝条件，严格编译导入
      策略则通过 source-owned 可克隆精确长度迭代器预检，在任何集合索引分配前以 builder 剩余
      `CurrentSourceLimits` 检查这些维度及实际/声明/组合字节与解码计数；compiler 不镜像 source
      硬上限或构造中间 view `Vec`，且不存在默认、无限、过期上限快照或先无界解码后统计的严格入口；
- [x] ScenarioManifest 组合入口只消费原子成功结果，不重复或绕过长度、SHA-256、媒体
      类型与引用验证；生产兼容结果不构造或保留 compiler-only 位置表，严格导入结果保留
      Manifest/Traffic/Spatial 三个来源文档的独立身份、逐文档来源记录和受限字段/记录位置表，且两者
      共享单一解析权威、均不重读原始字节；`CurrentSourceInput` 有外部 crate 可调用的零复制构造器，
      compiler 当前态入口只接受该借用原始输入并在独占 builder 调用内完成严格验证、降阶和原子提交，
      不公开接受预构造导入能力值；同时保留无需空间制品的 Traffic-only current Core 契约；
- [x] 记录级零动态分派、零完整克隆、一次摘要/计数/排序、多文档紧凑序号解析与配对性能
      验证方案闭合；
- [x] 第三方自定义前端非承诺、各议题职责和 G2 阻塞关系没有歧义；
- [x] `network-compiler.md`、路线图、设计索引、Skills 与术语表引用一致；
- [x] 对 G1 精确头完成本地全面审阅和有效外部审阅，并在 #315 追加只读
      [`## G1 设计判断` Pass 评论](https://github.com/illusion-tech/laneflow/issues/315#issuecomment-5189018791)。

本清单已由 #315 G1 Pass 接受为目标实现输入；在当时它不修改 #292 已接受的历史 Gate
记录，也尚未授权生产 Rust 实现。后继 G2 授权与实现结果见第 15 节。当时 #315 的
Delivery PR 收口是 #296/#297 进入各自 G2 的硬前置；该前置后来以 #315
`G4 Exception` 关闭，但不替代后继议题自身 Gate 或 exact-main 复核。

## 15. #315 共同接入实现事实与交付边界

#315 已取得只读
[`## G2 设计判断` Pass 评论](https://github.com/illusion-tech/laneflow/issues/315#issuecomment-5189685733)，
并在 `main@4465df3` 基线上实现以下共同接入事实：

- [x] 编译单元只保留私有共同 `TypedAstModule` / `TypedAstDeclaration`，具体官方模块
      通过同一条字段私有、原子准入路径提交；生产公共面没有通用前端 trait 或
      `add_module`；
- [x] 逻辑模块与来源文档独立登记，文档集摘要、逐文档摘要/长度/所属命名空间和冷来源
      记录已落地；来源记录完成摘要后释放，不进入编译单元保留状态；
- [x] `LF-COMP-P100-INITIAL-v1` 保持不可变，v2 只新增精确的
      `SourceDocumentCount = 1566`，v1 多文档以结构化配置档不兼容诊断失败关闭；
- [x] 共同准入保留文档归属/序号索引，来源映射按每条 span 的文档键解析独立紧凑序号并
      核对所属逻辑模块，不从模块序号推断文档；
- [x] 边界、边界加一、候选内/跨候选重复键、输入重排、已知向量、逐文档单次摘要、全部
      共同资源维度、三文档模块/实体声明与关系跨文档来源、失败原子性、第二官方包装和公共 API
      compile-fail 测试已经覆盖；
- [x] 构建器模块向量、模块索引、文档索引、冻结 scratch、结果模块包装和源映射平坦文档表
      均进入分阶段资源账本；专项 admission-only 基准报告 median/MAD、冷来源字符串字节、
      构建器/结果存续、冻结 scratch 与准入控制峰值；逐轮验证流水账见 git 历史；
- [x] #296 道路编辑来源语义明确排除在本切片之外；当时排除的 #297 current 导入设计现已
      取消，不再是后继实现目标。

本节只记录稳定实现边界，不充当动态治理状态。Delivery PR、required CI、精确头外部审阅、
full-set G3 证据与治理收口必须从 GitHub 和对应验证快照读取。#315 后来以
[`G4 Exception`](https://github.com/illusion-tech/laneflow/issues/315#issuecomment-5203763190)
关闭；#296 因而不再等待其代码交付，但自身 G2 必须显式复核继承的例外证明边界；
#297 调整后重新完成范围对应的 Gate。

[g1-performance-revision]: https://github.com/illusion-tech/laneflow/issues/292#issuecomment-5175472658
[g1-scale-correction]: https://github.com/illusion-tech/laneflow/issues/292#issuecomment-5175536346
