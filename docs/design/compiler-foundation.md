# 编译器基础设施与合成领域专用语言前端

**文档状态**: 已接受（Accepted；#292 G1；G2 实现进行中）<br>
**最后更新**: 2026-08-04<br>
**适用范围**: `laneflow-static-contract`、`laneflow-compiler`、
`laneflow-compiler-test-support`、有类型抽象语法树（Typed Abstract Syntax Tree，
Typed AST）→高层中间表示（High-level Intermediate Representation，HIR）→中层
中间表示（Mid-level Intermediate Representation，MIR）→已验证规范低层中间表示
（Validated Canonical Low-level Intermediate Representation，Canonical LIR）、
合成领域专用语言前端（Synthetic Domain-specific Language Frontend，Synthetic DSL
Frontend）、标识 v1（Identity v1）首次实现、确定性（Determinism）编译、诊断
（Diagnostic）与当前态等价投影<br>
**实现状态**: G2 实现进行中；`laneflow-static-contract` 已建立 `no_std` 值类型、
标识 v1（Identity v1）登记常量、有类型稳定标识与有类型逻辑序号；
`laneflow-compiler` 已建立生产资源配置档、来源模块头、结构化诊断、确定性
`LFSOURCE` 来源记录、显式导入图，以及车道图边、横断面完整所有者树、路口拓扑和
机动门 / 等待区静态闭包的受检合成领域声明。编译器侧标识 v1 编码、BLAKE3-128 派生、完整前像重复 / 碰撞登记、修订 1 全
22 种实体的冻结已知向量与独立测试预言机已经落地；`LaneEdge`、`RoadCorridor`、
`RoadSection`、`AuthoringLane`、`LaneGroup`、`FacilityBand`、`Junction`、`Movement` 和
`ManeuverPath`、`StopLine`、`ManeuverGate` 和 `WaitingZone` 已接入有类型符号解析、
父项先于子项的身份闭包、规范 HIR/MIR/LIR 连续表及来源伴随数据；完整
`entry + internal + exit` 路径、派生路口内部边排他角色、路径转换门、停止线使用闭包和
等待区静态区间约束已经闭合。不可变固定时制信号程序、完整相位状态和机动门信号绑定
已在运行时之前闭合；停车区域（`ParkingArea`）、停车位（`ParkingSpace`）、入口 / 出口
车道锚点、当前态静态几何和区域反向成员索引也已接入相同原子管线，其中区域归属不参与
停车位身份。参与者类别（`ParticipantClass`）单继承闭包与静态准入规则
（`AccessRule`）已经接入；编译器保留效果（effect）、类别集合、优先级（priority）和
法规来源，分别冻结 LaneEdge / LaneGroup / RoadSection 的边平面目标与 ManeuverPath
路径平面目标，
并在运行时之前拒绝继承环、无类别规则、法规来源不一致和相反效果的精确并列。
FacilityBand target 继续由结构化能力门卫（capability guard）失败关闭，时变窗口尚未
进入合成领域声明。`StaticRoute` 已保留显式有序边出现项，并预编译相邻边门、
机动路径、机动门、等待区出现项和反向索引。当前道路机动车的 `VehicleProfile` 已按
current Core IIDM `f64` 数值约束接入，唯一引用一个 `ParticipantClass`，并冻结身份、
参数、语义摘要与来源关系；它不构成其他交通执行域的通用参数基类。公共 `Compiler`
已经原子返回配对的 `ValidatedCanonicalLir` 与 `ValidatedSourceMapInput`；首批支持矩阵
中的动态路线生命周期、空间等其余领域及后继编译遍尚未实现。
#292 G1 已
接受 #308 G4 非生产研究证据及首轮资源 / 性能输入；当前生产路径仍是
`Traffic v0.10` / `SpatialPackage v0.1` / `ScenarioManifest v0.1` /
`laneflow-data` / `laneflow-core` / `laneflow-spatial`

**关联决策与设计**:

- `../adr/0014-residual-aware-f32-core-authority-and-migration-gates.md`
- `../adr/0020-compiler-owned-static-network-and-static-image.md`
- `../adr/0021-city-simulation-game-traffic-foundation.md`
- `network-compiler.md`
- `data-format.md`
- `data-loading.md`
- `numeric-representation.md`
- `spatial-geometry.md`
- `core-runtime-performance-baseline.md`（复用 P100 硬件身份，不复用运行时规模或预算）
- `compiler-budget-calibration.md`
- `../reference/compiler-calibration-workloads-v1.json`
- `../reference/v0.10-compiler-budget-calibration-report.md`
- `../reference/v0.10-compiler-budget-calibration-raw.json`
- `../reference/v0.10-compiler-budget-calibration-evidence.json`
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
- #294 完成 G4 前，由 `Traffic v0.10`、`SpatialPackage v0.1`、
  `ScenarioManifest v0.1`、`laneflow-data`、`laneflow-core` 和
  `laneflow-spatial` 组成的当前路径仍是生产契约。

#292 的目标是：

1. 建立可以继续承载官方几何文档前端、导入前端与编辑器编制界面的真实编译器基础；
2. 用合成领域专用语言前端完成首个可执行全管线；
3. 首次实现并冻结标识 v1 的字节、已知向量（Known Vector）和失败关闭（Fail
   Closed）语义；
4. 以集成专用桥（Integration-only Bridge）把已验证规范低层中间表示投影到当前核心
   和空间层，形成迁移等价证据；
5. 建立编译时延、峰值 / 保留内存（Retained Memory）和规模扩展基线，不以运行时
   交通参与单元规模替代编译器工作量。

#292 G1 已接受本文并在 GitHub 议题（Issue）#292 记录 `G1 Pass`。开始生产实现前
仍须从 GitHub 复核实时元数据与原生依赖，并记录 G2 开工判断；仓库长期文档不镜像
项目（Project）当前列。

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

| 包                               | 本议题拥有职责                                                                                                                     | 禁止职责 / 依赖                                                                                                                                  |
| -------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------ |
| `laneflow-static-contract`       | `StableId128`、实体种类（Entity Kind）/ 字段标签（Field Tag）登记、标识版本、有类型逻辑序号（Typed Logical Ordinal）和值级公共常量 | `Serde`、文件系统、编译遍、当前核心、目标交通运行时（Target Traffic Runtime）、空间层、编译器标识实现                                            |
| `laneflow-compiler`              | 权威来源模块图、编译器中间表示、合成领域专用语言前端、编译遍（Compiler Pass）、诊断、编译器侧标识实现                              | 当前数据层 / 核心 / 空间层对象图、公共制品格式、镜像应用二进制接口（Application Binary Interface，ABI）、独立验证器（Independent Validator）语义 |
| `laneflow-compiler-test-support` | 已验证规范低层中间表示到当前核心 / 空间层的投影、等价测试辅助                                                                      | 生产后端（Backend）、反向补语义、从当前对象图派生标识、被编译器依赖                                                                              |

`laneflow-static-contract` 使用 `#![no_std]`，只保存跨编译器、格式、镜像、验证器与
运行时的小型值类型和机器可读登记常量。它不提供编译器与未来独立验证器共用的完整标识
编码函数：二者必须分别实现规范字节编码和重算路径，避免把同一实现缺陷伪装成独立验证。

共享有类型逻辑序号的封闭标记类型（sealed marker）、`Ordinal<K>` 与稳定实体种类
标记类型（kind marker）也由 `laneflow-static-contract` 拥有；有类型抽象语法树、
HIR 和 MIR 的区块分配键（arena key）则留在编译器内部。这样后继
`laneflow-format`、`laneflow-static-image` 和 `laneflow-runtime` 可以共享零成本
类型，而编译器临时表不会被误写成长期公共契约。

`laneflow-compiler-test-support` 设置 `publish = false`。它只在测试开发依赖中使用
`laneflow-data` 加载当前态固定样例作为迁移预言机（Migration Oracle），该扩展在
#292 G1 显式登记；它的正常库依赖不使用 `JSON` 加载器，也不让
`laneflow-compiler` 看见当前态类型。

### 2.2 本议题不提前建立的包

- `laneflow-format` 的公共可移植规范制品（Portable Canonical Artifact）格式由 #298
  交付；
- `laneflow-validator` 的独立语义实现由 #299 交付；
- `laneflow-static-image` 的镜像应用二进制接口、配置档与有界结构校验器由 #300
  交付；
- `laneflow-runtime` 与空间层共享镜像消费路径由 #301 交付；不可变路网修订、
  运行时快照（Runtime Snapshot）与在线镜像切换由 #302 交付；
- 当前态 `laneflow-core` 到目标态 `laneflow-runtime` 的一次性不兼容切换由 #294
  独占。

#292 可以定义后继编译发射器所需的只读已验证规范低层中间表示视图（View），但不得用私有
临时线格式提前冻结 #298/#300 的公共字节契约。

## 3. 公共接口与构造权威

### 3.1 生产公共面

`laneflow-compiler` 的首版生产公共面限制为四类能力：官方来源构造、编译执行、
诊断与资源限制、只读已验证输出。#292 只交付合成领域专用语言前端的来源构造接口，
不公开通用前端插件接口。编译单元（Compilation Unit）只能经下列受检构造面建立：

```rust
pub struct SourceModuleHeader { /* 调用方提供的非内容字段，私有字段 */ }
pub struct SourceModuleDescriptor { /* 私有字段 */ }
pub struct SyntheticModuleBuilder { /* 私有字段 */ }
pub struct SyntheticModule { /* 私有字段 */ }
pub struct CompilationUnitBuilder { /* 私有字段 */ }
pub struct CompilationUnit { /* 私有字段 */ }
pub struct ValidatedCanonicalLir { /* 私有字段 */ }
pub struct ValidatedSourceMapInput { /* 私有字段 */ }
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

    pub fn compile(
        &mut self,
        unit: CompilationUnit,
    ) -> Result<CompilationOutput, DiagnosticBundle>;
}
```

以上代码表达 #292 G1 已接受的公共接口形状。G2 可以在不改变公共构造、所有权、
可见性、错误和确定性契约的前提下细化包内私有字段与实现名称；任何公共接口或上述
契约变化都必须重新进入 G1，不得作为实现细节直接修改。
`SyntheticModuleBuilder` 只接受首批支持矩阵中的领域构造；`CompilationUnitBuilder`
只接受 #292 明确支持的官方来源模块。二者都不能自行伪造有类型抽象语法树或已验证
阶段。必须保持：

- `TypedAstSink`、`TypedAstModule` 与官方前端调度接口是
  `laneflow-compiler` 的包内私有实现；
- 官方前端只能经包内受检接收器 `TypedAstSink` 产生 `TypedAstModule`，不能构造
  HIR、MIR 或 LIR；
- HIR 和 MIR 是 `laneflow-compiler` 私有阶段类型，不成为跨包兼容面；
- `ValidatedCanonicalLir` 字段私有，仅暴露有类型、只读、稳定顺序的表 / 区间
  视图；
- `ValidatedSourceMapInput` 字段私有，只暴露已验证的来源模块、文档、来源位置、
  来源沿袭和 LIR 键关联，不得补充静态语义；
- `CompilationOutput` 原子拥有 LIR、已验证源映射输入和零个或多个非错误级诊断；
- 不公开 `validate_unchecked_lir`、`assume_valid` 或从裸表直接构造
  `ValidatedCanonicalLir` 的入口；
- `Compiler::compile` 发生任一错误级诊断时只返回排序后的 `DiagnosticBundle`，不
  返回部分 LIR 或部分源映射输入；成功时的警告 / 提示保留在
  `CompilationOutput`；
- 编译器实例可以复用暂存区容量，但上次失败不能污染下次编译结果。

`ValidatedCanonicalLir` 的名字表达已经完成本文全部静态语义验证，不是调用方自报
状态。后继规范制品、静态镜像和语义差异发射器，以及集成专用投影，只接受该类型；
源映射发射器另外接受同一 `CompilationOutput` 中的 `ValidatedSourceMapInput`，但
该伴随数据不能新增默认值、关系或其他静态语义。类型系统由此阻止未验证阶段误入
后端，同时允许相同 LIR 语义对应不同来源位置和来源沿袭。

调用方不能独立构造 `SourceModuleDescriptor` 或自报内容摘要。
`SyntheticModuleBuilder::finish` 必须从调用方提供的 `SourceModuleHeader`、受检领域
声明和规范化调用记录原子生成 `SyntheticModule` 及其内嵌描述符；
`CompilationUnitBuilder` 只接收该封装结果，并在加入模块时重新校验累计计数与导入
闭包。后继官方前端可以增加各自的受检 `add_*_module` 方法，但不得开放裸描述符与
模块内容的配对入口。

### 3.2 官方前端封闭边界

#292 与 v0.10 不承诺稳定的第三方自定义前端扩展接口。编译器可以在包内使用封闭特征
（sealed trait）或等价私有调度承载官方合成领域专用语言前端、几何文档前端与导入
前端；该边界允许一次模块级动态分派或泛型单态化，不进入记录级循环，但它不是公共
接口、稳定应用二进制接口或跨版本兼容面。官方前端必须通过包内 `TypedAstSink` 的
受检构造接口：

- 每个来源模块必须有稳定编制命名空间标识（Authoring Namespace ID）
  `authoringNamespaceId`、来源语言（Source Language）、内容摘要、前端版本、选项
  摘要与来源沿袭（Provenance）；
- 每个声明必须有显式稳定键和来源位置（Source Span）；
- 可发布模块不接受匿名命名空间、缺失来源沿袭或测试专用来源位置；
- 前端专用语法（Frontend-specific Syntax）可以作为来源诊断元数据保留，但不得
  绕过共同 HIR/MIR 编译遍。

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

## 4. 阶段表示与内存所有权

### 4.1 有类型抽象语法树

有类型抽象语法树保留：

- 来源模块描述符；
- 声明种类、显式稳定键和有类型单位；
- 有类型未解析引用；
- 来源位置与来源沿袭；
- 官方前端展开来源，但不保存当前核心句柄或目标布局（Layout）。

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
- 显式规范 `f32` 折线的几何（Geometry）连续性与当前交通 `f64` 长度共同校验；
- 父项先于子项的标识闭包；
- 静态路线、机动路径、机动门 / 等待区出现项与反向索引；
- 所有者 / 成员关系、覆盖关系（Coverage）、唯一所有者和全局一致性。

MIR 可以使用临时哈希与缓存（Hash and Cache），但所有规范遍历都从显式稳定序列或
排序后的完整键产生。临时指纹（Fingerprint）命中后必须比较完整权威值。

#292 的交通长度、速度、车辆配置和停车标量沿用当前态已接受的 `f64` 语义；规范空间
几何沿用 ADR 0015 的有界规范 `f32`。这不是重新启动 #144 已形成“不迁移
（no-go）”结论的当前核心 `f32` 生产迁移：ADR 0014 本身仍是已接受的目标数值
契约，但首次生产迁移未通过百分之五性能门槛并完整回退，当前生产交通数值权威继续
使用 `f64`。未来若通过新的数值 G1 改变交通权威表示，必须原子修订编译器中间
表示、可移植规范制品、目标静态镜像（Target Static Image）、目标交通运行时和
迁移预言机，不能由编译发射器在后端私自窄化。

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
冻结，但不属于静态路网语义。它至少保存：

- 来源模块描述符、来源文档登记和来源沿袭；
- 稳定实体的实体种类、`StableId128`、有类型逻辑序号与主要 / 关联来源位置；
- 所有者局部关系 / 出现项的所有者稳定标识、有类型角色、本次编译局部序号与来源
  位置；
- 生成关系的推导链和贡献来源位置（contributing spans）。

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
  -> 预计算静态路线、路径、机动门和等待区出现项及其索引
  -> 冻结已验证规范低层中间表示
  -> 冻结已验证源映射输入与成功诊断
```

每个编译遍消费前一阶段的只读视图或取得其所有权，并把暂存数据与输出写入独立
区域。编译遍只有在本阶段错误级诊断为零时才能提交输出。整个编译单元只有在全部
编译遍成功时才能返回完整 `CompilationOutput`。

本顺序是 `network-compiler.md` 推荐顺序在 #292 首个纵向切片内的收窄，不是对上游
顺序的修订：来源格式升级由后继几何文档 / 导入前端接入；静态执行约束与分区规划
提示在对应后继 G1 加入同一 MIR/LIR 管线；规范制品、源映射 / 语义差异和静态镜像的
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
无限生产模式，也不得以 Rust `Default` 或来源自报字段绕过宿主选择。v0.10 只提供
`CompileLimits::p100_initial_v1()` 这一生产具名构造器；调用方必须显式选择它，测试
可以在包内构造更小边界，但不能获得无限配置。该构造器的稳定配置档标识符为
`LF-COMP-P100-INITIAL-v1`。字段保持私有，避免把内部阶段布局变成公共兼容面；配置档
修订改变任一上限时必须提升标识符修订并重新执行边界测试。

`LF-COMP-P100-INITIAL-v1` 以 #308 九个压力分层的逐维上包络为来源。来源 / 领域计数
取原始测量制品 `v0.10-compiler-budget-calibration-raw.json` 中
`limitQualification.limitPairs[].pair.exactDimensionValue` 的压力规模最大值；总存续
与保留容量取紧凑 Evidence
`v0.10-compiler-budget-calibration-evidence.json` 中
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
| `max_route_occurrence_count`          |     1920 | 路线出现项                                       |
| `max_maneuver_gate_count`             |     2304 | 机动门                                           |
| `max_waiting_zone_count`              |     1536 | 等待区                                           |
| `max_geometry_point_count`            |    22368 | 规范几何点                                       |
| `max_symbol_count`                    |    11265 | 符号                                             |
| `max_string_item_count`               |    36894 | 驻留字符串项                                     |
| `max_single_string_bytes`             |       53 | 单个字符串字节                                   |
| `max_total_string_bytes`              |   991537 | 驻留字符串总字节                                 |
| `max_diagnostic_count`                |       16 | 规范排序后保留的诊断                             |
| `max_stage_scratch_bytes`             |   304896 | 单次编译遍暂存请求字节                           |
| `max_output_bytes`                    |  2782758 | 正在构造的 LIR / 伴随输出逻辑字节                |
| `max_compiler_controlled_live_bytes`  | 43269120 | 编译器控制总存续请求字节                         |
| `max_retained_capacity_bytes`         | 36925688 | 一次编译结束后编译器实例允许保留的无语义容量字节 |

单模块来源上限与总来源上限同值，是从已资格验证的总来源上限作出的失败关闭收窄；#292 G2
仍须分别验证单模块与跨模块累计边界。阶段记录数是实现预算而不是公共数据模型：生产 IR
可以使用不同 Rust 结构，但必须为每个逻辑记录定义可审计计数，且不能用拆分/合并结构
绕过总存续字节上限。上述逐维最大值不是已经测过的“所有维度同时取最大”组合；任意
输入同时逼近多个上限时，`max_compiler_controlled_live_bytes` 仍是最终全局失败边界。

受检维度包括：

- 模块数、导入边数和单模块 / 总来源字节；
- 各有类型抽象语法树、HIR、MIR 和 LIR 表的记录数；
- 关系、字段字节、来源位置与诊断数；
- 单条字符串 / 键长度和已驻留字符串总字节数；
- 编译期间允许的暂存区峰值字节数；
- 来源副本、字符串、各阶段表、关系、诊断、暂存区和正在构造的输出共同形成的
  编译器控制总存续内存峰值，以及编译结束后允许保留的内部容量。

同一个不可变 `CompileLimits` 必须贯穿三个受检边界：

1. `SyntheticModuleBuilder` 在字符串、来源位置、声明和规范化调用记录扩容前执行
   单模块上限；
2. `CompilationUnitBuilder` 使用构建器实测且不可由调用方修改的计数，执行模块、
   导入、总来源字节和总驻留字符串的累计上限；
3. `Compiler` 在每个编译遍分配后继表或扩大暂存区前执行 AST/HIR/MIR/LIR、关系、
   诊断、暂存内存和总存续内存上限。

模块已经在更宽上限下构造不表示可以绕过编译单元上限；
`CompilationUnitBuilder::add_synthetic_module` 必须以受检加法重新核对累计计数。
库内受控分配以请求容量字节记账；调用方在进入 API 前自行持有的外部内存不计入编译器
峰值，但复制 / 驻留进编译器后的字节必须计入。各分项均未超限不能替代总存续内存
核对；所有请求容量和累计字节使用受检加法，平台无法表示请求值时必须在分配前失败。

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
形成 `sourceContentDigest`；编码变化必须提升前端版本并更新已知向量。调用机器的
绝对路径、墙钟时间和指针地址不进入该记录。
该摘要只服务来源沿袭和重放，不参与实体稳定标识。测试可以使用显式 `test_only`
来源模块头；该能力不得进入发布接口。

领域专用语言构建器的声明方法使用 `#[track_caller]` 或等价宏捕获 Rust 文件、行和
列作为来源位置。每个来源模块另有调用方提供、与机器路径无关的稳定
`sourceDocumentKey`；该键在同一来源模块内必须唯一，并受 `CompileLimits` 的
`ASCII` 字符集与长度上限约束，重复键在进入诊断排序前失败。
`file!()` 结果只作本地显示和来源沿袭。循环展开的多个声明可以共享调用位置，但诊断
必须同时携带声明的显式稳定键；来源位置、文件路径和展开序号都不得参与标识。

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
| 静态路线       | 显式有序边序列和静态路线出现项                                                                                                                    | 动态路线生命周期和路径规划策略                                   |
| 空间           | 显式规范坐标框架（`CanonicalFrame`）和已量化规范 `f32` 折线、长度 / 连续性校验                                                                    | 曲线、高程求值、曲线细分（tessellation）和几何文档前端           |

首批领域专用语言必须能表达标识 v1 修订 1 的全部实体种类。支持“声明该实体”不表示对应
目标交通运行时执行域或动态能力已经实现。

### 6.3 迁移场景

G1 冻结以下两个当前态固定样例的等价迁移：

1. `examples/config/v0.10-signalized-corridor.toml` 指向的完整配对集合：
   `examples/data/v0.10-signalized-corridor.laneflow.json`、
   `examples/data/v0.1-signalized-corridor.spatial.json`、
   `examples/data/v0.1-signalized-corridor.scenario.json` 与
   `examples/data/v0.2-signalized-corridor.catalog.toml`；覆盖完整车道拓扑、横断面 /
   准入、信号、停车、静态路线与规范几何；
2. `examples/data/v0.10-multi-gate-waiting-zone.laneflow.json`：覆盖单个机动路径的
   多机动门、等待区与对应路线出现项编译。

若第一项完整走廊在实现基准中证明会把 #292 变成几何文档前端交付，只能通过新的 G1
审阅发现处置记录收窄为其**显式规范折线**投影；不得改用只含两条边的玩具场景并声称
覆盖横断面 / 准入 / 空间层等价。

`examples/data/v0.10-empty-signals-and-parking.laneflow.json` 不作为第三个端到端迁移
场景，但其 `loop`、`isolated`、`loop-once` 进入标识 v1 / 静态路线回归固定样例，
证明自环边、合法孤立边和重复边出现项都不依赖道路区段或路口所有者。

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

未来 `laneflow-validator` 只能复用登记常量和值类型，不能调用编译器编码器或语义
编译遍。#292 另提供不复用编译器编码器的 `xtask` / 测试预言机生成和校验已知向量；
`BLAKE3` 库作为密码摘要原语可以相同，规范字节组装代码必须独立。

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

来源位置使用“`sourceDocumentKey`、起始行 / 列、结束行 / 列”四部分，字段宽度使用
受检的 `u32`。`sourceDocumentKey` 是来源模块内显式、稳定、与机器路径无关的 `ASCII`
键；合成领域专用语言使用该键、调用位置和声明的显式稳定键，未来文本前端使用该键与
真实文本范围。宿主路径只服务显示和来源沿袭，不参与规范标识、规范排序、可移植规范
制品或语义差异。

## 9. 集成专用的已验证规范低层中间表示到当前态投影

`laneflow-compiler-test-support` 只接受 `&ValidatedCanonicalLir`，并产生：

- 当前态 `InitialTrafficData`；
- 当前态 `SpatialRegistry` 所需输入与绑定；
- 用于等价断言的稳定映射报告。

投影固定遵守：

- LIR 已完成的稳定标识、所有者、拓扑、长度、出现项和顺序语义不在投影中重算；
- 当前态构造器仍可执行自己的防御校验，但其结果不是编译器语义验证的实现；
- 当前态句柄 / 登记表序号只存在于投影结果，不回写 LIR；
- 不读取当前态 `Traffic v0.10` `JSON` 形成目标态语义；
- 投影不成为 `laneflow-compiler` 功能特性（feature），不被生产编译发射器复用；
- #294 完成生产切换时删除。

等价矩阵至少覆盖：

| 证据层     | 比较内容                                                           |
| ---------- | ------------------------------------------------------------------ |
| 静态语义   | 实体、稳定关系、有序关系、长度、信号、停车、准入和等待区出现项     |
| 当前态构造 | 当前态构造器全部成功，失败路径诊断能够回链来源                     |
| 核心行为   | 冻结命令序列下车辆状态、信号解释、停车和等待行为                   |
| 事件       | 事件种类、顺序、载荷和固定步进                                     |
| 确定性     | 重复运行、声明置换、不同哈希种子的状态摘要                         |
| 空间层     | 规范坐标框架、车道图边绑定、采样位置 / 切向、停车位姿与错误        |
| 资源       | 投影时延、峰值分配、保留内存；明确其为迁移成本而非目标生产启动性能 |

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

## 11. 测试、工作负载与基准

### 11.1 正确性测试

- 每个编译遍的正向、首错误和多错误稳定排序；
- 成功路径保留警告 / 提示，错误路径不返回部分 LIR 或部分源映射输入；
- 模块、导入和引用循环；
- 来源模块描述符由构建器派生、内容摘要已知向量、重复 `sourceDocumentKey`；
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

### 11.2 编译器工作负载

编译器工作负载只按编制 / 中间表示对象计数，不使用车辆、交通参与单元或
`Agent` 数量。当前固定样例、编译器校准规模（Compiler Calibration Scale）与
编译器压力规模（Compiler Stress Scale）承担不同证据职责：

| 工作负载标识符             | 证据角色              | 目的                                         | 主要计数对象                       |
| -------------------------- | --------------------- | -------------------------------------------- | ---------------------------------- |
| `LF-COMP-ID-v1`            | 合成校准 / 压力       | 标识 v1 全部实体种类 / 标签与父项闭包        | 模块、稳定实体、标识字段 / 字节    |
| `LF-COMP-CORRIDOR-v1`      | 合成校准 / 压力       | 线性走廊、信号 / 停车 / 横断面扩展           | 边、关系、路线、信号状态、几何点   |
| `LF-COMP-JUNCTION-GRID-v1` | 合成校准 / 压力       | 密集路口、路径 / 机动门 / 等待区出现项与索引 | 路口、路径、机动门、等待区、出现项 |
| `LF-COMP-CURRENT-EQUIV-v1` | 当前固定正确性 / 回归 | 两个当前态固定样例的端到端迁移               | 阶段记录、投影记录、诊断           |

`LF-COMP-CURRENT-EQUIV-v1` 只证明现有固定样例的正确性、回归与小输入固定成本，不
通过复制样例推断城市容量，也不与 #308 的
`LF-COMP-RESEARCH-CURRENT-FIXTURES-v1` 合并。前三项直接复用
`compiler-calibration-workloads-v1.json` 的生成规则与下列冻结规模；不得在 #292
另抄一套记录公式：

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

#308 已完成 G4。其原始测量、机器可读 Evidence 与报告冻结了研究替身的 P100 同机
R0 研究基线；产品负责人另行把同一台 `LF-P100-REF-01` 物理机器选定为目标产品推荐参考
机型。硬件选择不把历史 R0 自动升级为产品通过，但允许 #292 在同一物理基准上建立
首轮生产门槛。该门槛标识符为 `LF-COMP-P100-R0-v1`。

`LF-COMP-P100-R0-v1` 只消费校准规模与压力规模的 126 条精确预算建议。自然身份是
`(workloadId, graphProfile, n, sampleKind, binaryMode, metric)`；每个生产基准样本必须
逐项查找 `v0.10-compiler-budget-calibration-evidence.json` 中相同自然身份的
`results.budgetRecommendations[]`，不得用跨工作负载平均值掩盖单项回退。下表仅给出
便于人工审阅的逐等级最大值，不能替代机器可读逐项门槛：

| 等级 | 冷实例墙钟 ns | 复用墙钟 ns | 冷实例存续峰值 B | 复用存续峰值 B | 保留容量 B | 完整进程私有内存 B |
| ---- | ------------: | ----------: | ---------------: | -------------: | ---------: | -----------------: |
| 校准 |      30986300 |    21751700 |         18752072 |       21650640 |   18477896 |           56187294 |
| 压力 |      49878400 |    45679900 |         37474040 |       43269120 |   36925688 |          100798860 |

G2/G3 在 P100 上执行生产干净单工作线程编译时必须遵循以下判定：

1. 使用第 11.2 节相同生成规则、模块图、规模、冷实例 / 稳定容量复用边界与计时边界；
2. 语义正确性先于性能，生产 LIR 摘要不能与研究摘要直接比较，必须由
   `LF-COMP-CURRENT-EQUIV-v1` 和生产独立预言机证明；
3. 校准 / 压力规模的每个适用性能分层不得超过对应 R0 建议；完整进程私有内存只能按
   完整进程样本比较，不能拆成冷实例 / 复用值；
4. 首次生产实现增加必要语义后若超出任一门槛，结果不是实现失败的永久结论，也不得
   静默放宽；必须提交精确分层、必要语义差额与优化判断，回到 G1 冻结新修订预算；
5. 后续优化以首次通过的生产提交另建生产基线，不能继续把研究替身冒充产品预言机。

墙钟门槛已经包含 `26302/22185` 的全局重复性包络；存续峰值和保留容量包络为 `1/1`，
完整进程私有内存包络为 `1241/559`，不得再次乘包络。#308 没有保留可用于决策的增长
斜率，因此 #292 不伪造斜率预算；三级日常回归与五级原因分析已经给出足够的非线性
监测边界。玩家在线编辑停顿、最低/替代硬件、中国特色城市工作负载、增量/并行编译、
可移植制品发射和静态镜像构建仍不在本门槛内。

研究停止护栏只保护研究机器，不进入生产门槛。不得把运行时交通参与单元规模、多世界
吞吐或研究替身的进程数量换算成编译器产品容量。预算来源、计数对象、单位、P100
硬件身份与证据提交必须保留可追溯性。

## 12. #292 G1 接受结果与 G2 前置

#292 G1 已确认：

- [x] 公共接口、第三方前端非承诺边界、阶段类型私有性和包依赖有向无环图通过本地
      架构审阅；
- [x] 合成领域专用语言前端的支持 / 拒绝矩阵与两个迁移场景闭合；
- [x] 标识 v1 值类型、独立编码边界和已知向量计划闭合；
- [x] #308 已按自身 Gate 完成非生产校准研究；`CompileLimits`、工作负载规模、时延 /
      内存门槛及其证据边界已经由精确提交和机器可读 Evidence 冻结；
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
