# laneflow_compiler

LaneFlow 编译器拥有的静态路网（Compiler-owned Static Network）的生产编译器。

本 crate 最终负责受检官方来源构造、确定性编译、结构化诊断、资源限制，以及只读的
已验证规范低层中间表示（Validated Canonical Low-level Intermediate
Representation，Canonical LIR）。当前 G2 切片已建立首个生产编译资源上限配置档
`LF-COMP-P100-INITIAL-v1`、受检来源模块头、结构化诊断，以及可派生 SHA-256 内容
摘要的确定性 `LFSOURCE` 来源记录和显式导入图。官方合成领域专用语言（Synthetic
DSL）已接入车道图边、完整横断面所有者树，以及由 `Junction`、`Movement` 和
`ManeuverPath` 构成的路口拓扑闭包；`StopLine`、`ManeuverGate` 与 `WaitingZone`
也已形成独立的静态控制边界闭包；不可变固定时制 `SignalController` 程序、
`SignalPhase` 完整状态和 `ManeuverGate` 到 `SignalGroup` 的控制绑定会在运行时之前
闭合并冻结，但不替代运行时的最终通行权裁决；`ParkingFacility`、`ParkingSpace`、入口 /
出口车道锚点和不可变矩形几何同样在编译期闭合，区域归属只作组织关系，不进入
`ParkingSpace` 身份；`ParticipantClass` 单继承闭包与静态 `AccessRule` 也已接入，
支持 LaneEdge、LaneGroup、RoadSection 和 ManeuverPath 两个独立求值平面，并保留
效果（effect）、类别集合、优先级（priority）与法规来源。相反效果的精确并列在编译期
拒绝，FacilityBand target 以结构化能力门卫（capability guard）失败关闭；时变窗口尚未
进入本切片。当前道路机动车 `VehicleProfile` 按 ADR 0028 先量化再关闭：毫米/`mm/s` 整数闭包，时距与加减速为受检 `f32` SI；编制输入仍是 `f64`，
解析唯一 `ParticipantClass` 引用并进入规范身份、语义摘要和来源映射；该配置不是其他
交通执行域的通用参数基类。`CanonicalFrame` 也已把 SpatialPackage v0.1 的稳定
`frameId` 接入同一管线；坐标单位、手性、轴向和范围继续由全局空间契约固定，声明中
不重复保存 CRS、宿主放置或可变原点。可选车道图边中心线会在 HIR 证明全图恰好一次
覆盖、长度绑定和连接端点连续性，并在 Canonical LIR 中按 `LaneEdgeOrdinal` 对齐冻结
规范 `f32` 点、累计弧长、切向与上方向采样表；完全不声明中心线时仍形成合法无图形
配置（headless）LIR。
有序边序列只经运行时 `register_route` 进入每世界表。`ConflictZone`、
`CanonicalFrame`、`ParticipantStream` 分别占用身份种类 21、22、23。构建器原子拒绝非法数值 / token、非法或未
导入引用、重复声明和重复无序关系，并保留横断面、车道、覆盖链与完整机动路径的领域顺序。
包内标识 v1 编码器严格校验登记字段，
流式形成规范字节并派生 BLAKE3-128 `StableId128`；修订 4 的 24 个可构造种类由
`tests/identity-v1-known-vectors.txt` 冻结已知向量，并由独立字节组装预言机校验。
纵向管线使用有类型 `u32` 区块键完成跨模块符号解析、车道覆盖与机动路径连通性、
横断面唯一所有者，以及路口内部边排他角色校验，并按父项先于子项派生横断面和路口
实体身份；机动门转换范围 / 唯一性、停止线边一致性 / 使用闭包，以及等待区门所有权、
严格顺序、非零容量和内部不重叠同样在 HIR 关闭。完整身份前像只在 HIR 临时登记表中
用于重复 / 摘要碰撞判断。Canonical LIR
再按各实体的完整 Identity v1 前像字节分配有类型
逻辑序号，把连接和父子关系全部改写为同一 LIR 实例的有类型序号，并以共享字段字节池
保存后继制品所需的完整身份字段；来源位置不进入 LIR 语义摘要。公共 `Compiler` 已将该
当前领域子集接入原子成功闭环：任一阶段失败时只返回结构化诊断，成功时同时返回只读
`ValidatedCanonicalLir`、与其配对的 `ValidatedSourceMapInput` 和非错误级诊断。
`CompilationUnit` 内部只保留编译器私有的 `TypedAstModule` /
`TypedAstDeclaration`，具体官方前端通过同一条原子私有接入路径提交。逻辑模块与
一份或多份 `SourceDocumentDescriptor` 独立建模；模块使用版本化
`sourceDocumentSetDigest`，每份文档单独保留键、SHA-256、长度与冷显示/审计来源。
道路编辑来源由唯一的 `CompilationUnitBuilder::add_road_editing_module` 接收借用的
size-prefixed `LFRE` bytes；同一事务完成有界 verifier、语义预检、owner-qualified
Typed AST 降阶、authoring geometry 数值冻结和共同准入。失败不会污染 builder，成功后
只保留 owned 描述符、编译结果与共享来源位置 context，不保留 wire bytes 或 generated
view。最终 LaneEdge 与不可遍历 FacilityBand 的规范 `f32` 点表进入同一 HIR/MIR/LIR
管线，但仍分别保存在可遍历边表与设施带稀疏几何表中。alignment station rows、
regularity visit cache、待提交几何包装与单 corridor 临时集合使用一个累计 stage-scratch
账本，并同时受 compiler-controlled live-byte 剩余空间约束，不能按每条 alignment 重复
享用完整上限。
`LF-COMP-P100-INITIAL-v1` 保持不变且只支持每模块一份文档；
`LF-COMP-P100-INITIAL-v2` 只新增 `max_source_document_count = 1566`。
`sourceDocumentKey` 在整个编译单元内唯一，来源位置按每条 span 的文档键解析独立
文档序号，再在 AST/HIR/MIR 释放前按 LIR 稳定实体与 owner-local 关系冻结；
共同准入分别核算构建器存续量、模块图冻结 scratch、构建峰值和成功结果存续量，模块索引、
文档索引及模块包装均不能游离于 `CompileLimits`；
路线生命周期由 `TrafficWorld::register_route` 拥有，不在编译器内。

可移植候选发射由 `emit_portable_candidate` 提供。它只能原子借用同一个
`CompilationOutput` 中已配对的 LIR/source-map input，并接收规范化
`PortableEmissionProvenance`、显式 `PortableDiffBase::{Genesis, Artifact}` 和格式上限。
成功结果 `PortablePublicationCandidate` 同时拥有 LFCA、LFSM、LFSD 的不可变可重读来源、
`Sha256Digest`、`ExactByteLength`、`sha256/<lowercase-hex>` object key 与
`NetworkRevisionId`。这些计算绑定在公开 API 中保持 `laneflow-static-contract` 的强类型，
只在线格式边界转换为原始字节/整数。Artifact base
必须先取得 `ValueCheckedObjectView`；emitter 在变化分类前拒绝不兼容 contract、base 内部
身份/实体错配和跨修订 StableId 前像冲突，并按 A.3/A.5 的字段、set/scalar/domain/occurrence、
geometry、static rule 与全局 spatial 规则产生诊断性 LFSD。

LFCA 5 策略引用在最终目标字节和 Artifact base 上另行核对：四张局部表的 owner 必须
存在，stream/gate/classes/yield 引用必须解析到对应种类，引用集合按 StableId 严格排序；
gap/evidence key 只能解析到同一 policy。每条规则须继承策略级来源或具有已解析依据，
已有来源也不豁免显式依据引用。evidence/gap 的借用键索引先按完整逻辑行数计量，再分配，
受 `StageScratchBytes` 约束并在检查结束后释放；跨 chunk 不改变 owner、key 或引用语义。
该检查不替代规则选择、覆盖性、灯型一致性及完整 LFSD/LFSM 投影闭合；正式策略前端和
共享根消费按路权实施合同分别交付。

发射器对受检 table/RecordVector 的顺序消费使用单游标零拷贝迭代器；writer 适配层把
nested/top fields、rows、tables、sections 降低到少量连续 arena，并通过
`PreparedObject` 只执行一次完整计量/预检。ordinal 随机访问只保留给 singleton 或真正的
定位读取，不得用于全表顺序扫描。

该返回值仍是未受信候选：checked base 只证明格式结构和直接值域，不证明完整 artifact
语义；候选也不授予发布、迁移或运行时加载权限。`read_portable_object_known_length` 和
`read_portable_object_to_end` 只在任何 hash/view 前关闭已知长度 O(1) 与未知流 `max+1`
transport 边界，读取成功仍须进入 format preflight。百万级生产路径由字段私有 staged writer
写入 file-backed backing；`finish` 后能力内部可继续持有字段私有 `File`，但没有 LaneFlow
safe API 可达的写能力。checker 与共享静态
构建直接复用同一 backing，不保留完整对象级 `Box<[u8]>`，也不发生完整对象级复制。
`laneflow-format` 的无分配后发射检查
从 LFCA/LFSM/LFSD 最终字节重算 digest、length 与路网修订，并闭合 LFSM 和 LFSD 的必要
跨对象 binding。成功后可从受检 capability 构造不含 receipt/LFSD binding 的 LFCP v2，或
直接构建 `SharedNetworkRevision`。检查不重跑完整 compiler 语义，也不验证 LFSD change set
完备性。内容仓库、atomic file install、并发 winner、目录耐久和 manifest 提交由宿主、CI 或
打包工具负责；LaneFlow 加载宿主交付对象时仍重新验证 digest、length、revision 与 binding。

跨平台确定性由 `.github/workflows/portable-exact-bytes.yml` 直接导出 production emitter 的
LFCA/LFSM/LFSD bytes：Windows/Ubuntu 各两个 fresh process、每个进程内重复发射，最后在单一
汇总 job 中逐文件比较 exact bytes、length、SHA-256 和冻结 fixture。当前正式 worker 集合为
`{1}`；该工作流不把未实现的并行环境变量解释为 worker 覆盖。

成功输出还通过 `CompilationMetrics` 暴露 LIR 逻辑记录数、逻辑输出字节、编译器控制
峰值字节和同版本语义指纹；`Compiler::retained_capacity_bytes()` 单独报告跨编译保留
容量。它们只服务宿主预算观测和确定性核对，不暴露私有阶段布局，也不能替代版本化
制品摘要或操作系统进程内存。

公共静态值契约来自 `laneflow-static-contract`。本 crate 不依赖当前核心、空间层、
数据加载器或引擎适配器，也不提前冻结可移植制品、共享静态路网布局或第三方前端插件接口。
