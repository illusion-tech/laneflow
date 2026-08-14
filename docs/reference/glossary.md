# LaneFlow 双语术语表

**文档状态**: Active<br>
**最后更新**: 2026-08-06<br>
**适用范围**: LaneFlow 架构、ADR、设计文档、Agent Skill、Issue/PR 设计说明、
#291 编译器时代静态路网方案与城市模拟游戏交通基础

## 1. 规范性规则

LaneFlow 的长期设计以中文为权威事实，英文只用于辅助理解和对接代码、标准或外部
生态：

1. 规范性术语首次出现时写作“中文术语（English Alias）”；后文优先使用中文。
2. 中英文解释发生冲突时，以本表中文术语和中文定义为准；英文别名不能新增、删除
   或改变语义。
3. Rust 类型、crate、字段、版本值、算法名和协议常量等精确标识符使用反引号保留
   原文；正文必须用中文说明其语义。
4. 缩写首次出现时给出中文全称、英文全称与缩写；后文可以保留缩写。
5. 修改中文规范定义属于设计变更；只改英文辅助别名且不改变中文含义属于编辑性
   修订。
6. 本表未收录的新领域术语不得只用英文写入正式 ADR/design；必须先补中文规范名和
   英文辅助名。
7. 面向读者的数量必须让数量级、单位和计数对象清楚且无歧义。零较多的中文正文建议
   使用规范中文数量，表格、公式和机器输入建议使用完整十进制数字；上下文明确时
   `k`/`M` 等缩写或 `1 万` 等写法可以保留，仅作可读性与一致性建议，不设自动门禁。
   小写 `m` 应特别避免与米混淆。精确代码/测试/文件/协议标识符按原文保留。
8. 交通仿真规模不使用 `Agent` 或“代理”计数；`Agent` 默认指 AI Agent 工作流。
   长期通用规模使用交通参与单元及其分执行域计数，当前车辆实现和历史车辆证据必须
   显式标注为车辆特化，不能反向定义目标交通运行时。
9. 尚未接受的词条统一使用“提案中（Proposed；#议题号）”标记；“目标态”只描述
   已经由对应 G1/ADR 接受、但尚未完成生产切换的设计，不得用“目标态提案中”混合
   两种状态。词条进入本表只统一命名和定义，不自动表示其实现或数值已经被接受。

“中文为权威”不改变代码标识符的精确拼写，也不翻译第三方商标、算法专名或必须按
协议匹配的字面量。

## 2. 状态、治理与迁移

| 中文规范术语         | 英文辅助名（English Alias）              | 精确标识符 / 缩写 | 中文规范含义                                                                               |
| -------------------- | ---------------------------------------- | ----------------- | ------------------------------------------------------------------------------------------ |
| 当前态               | current state                            | `current`         | 当前已实现、已发布并受现有 ADR/API 约束的生产事实。                                        |
| 目标态               | target state                             | `target`          | 已形成设计但尚未完成生产切换的目标事实。                                                   |
| 生产路径             | production path                          | —                 | 被正式产品入口使用并受兼容、验证与治理约束的执行路径。                                     |
| 单一事实源           | single source of truth                   | SSOT              | 同一事实只由一个规范来源拥有，派生物不得反向竞争权威。                                     |
| 权威职责             | authority                                | —                 | 对某类事实作最终裁决并承担一致性责任的组件职责。                                           |
| 一次性不兼容切换     | clean break                              | —                 | 不保留长期双 API 的显式破坏性切换。                                                        |
| 生产切换             | production cutover                       | —                 | 目标路径完成等价性、性能与安全门禁后替代当前生产路径。                                     |
| 集成专用桥           | integration-only bridge                  | —                 | 只服务迁移验证、不属于生产 API，并在切换后删除的桥接实现。                                 |
| 迁移预言机           | migration oracle                         | —                 | 用于对照目标实现语义、行为或字节结果的独立基准实现。                                       |
| 开发闸口             | development gate                         | `G0`–`G4`         | LaneFlow 从立项、设计、开工、合并到完成的治理状态。                                        |
| 城市模拟游戏交通基础 | city simulation game traffic foundation  | —                 | 面向中国特色城市模拟游戏、但不拥有城市经济与出行需求的可嵌入、确定性、可扩展交通基础设施。 |
| 城市模拟游戏层       | city simulation game layer               | —                 | 拥有人口、经济、土地利用、建筑、工作/居住、物流任务与游戏规则的宿主上层。                  |
| 出行与交通编排层     | mobility and traffic orchestration layer | —                 | 拥有出行需求、出发时刻、交通参与单元生成、目的地、人口生命周期与路线选择策略的上层。       |

## 3. 编制来源与编译管线

| 中文规范术语            | 英文辅助名（English Alias）                               | 精确标识符 / 缩写                       | 中文规范含义                                                                                                                                                                                      |
| ----------------------- | --------------------------------------------------------- | --------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 数据编制                | authoring                                                 | —                                       | 创建、导入、编辑或生成静态路网来源的活动。                                                                                                                                                        |
| 权威来源模块图          | authoritative source module graph                         | —                                       | 一个编译单元内唯一可重放的编制事实源；节点是来源模块，边是显式导入关系。                                                                                                                          |
| 来源模块                | source module                                             | —                                       | 目标态已接受（#292 G1、#315 G1）：绑定命名空间、来源语言、工具/选项与沿袭的逻辑编译节点；合成前端恰有一份文档，后继官方前端可拥有多份文档并绑定文档集摘要。                                       |
| 来源模块头              | source module header                                      | `SourceModuleHeader`                    | 目标态已接受（#292 G1、#315 G1）：调用方提供非内容字段，官方前端派生模块描述符、文档集摘要、逐文档摘要与描述符。                                                                                  |
| 来源文档描述符          | source document descriptor                                | `SourceDocumentDescriptor`              | 目标态已接受（#315 G1），把一个来源文档的稳定文档键、精确内容摘要、原始字节长度、所属逻辑模块及逐文档来源记录不可分绑定的字段私有值；模块与文档不要求一一对应。                                   |
| 逐文档来源记录          | per-document source origin record                         | `SourceDocumentOrigin`                  | 目标态已接受（#315 G1），冷显示/审计元数据；区分文档角色，Traffic/Spatial 另绑定清单制品引用。宿主来源未认证；不证明内容或发布真实性，不进入标识、LIR 语义或文档集摘要。                          |
| 来源文档摘要            | source document digest                                    | `sourceDocumentDigest`                  | 目标态已接受（#315 G1），由官方前端对一份版本化规范来源记录的精确原始字节计算一次所得的 SHA-256；它不由调用方自报，也不参与实体稳定标识。                                                         |
| 来源文档集摘要          | source document-set digest                                | `sourceDocumentSetDigest`               | 目标态已接受（#315 G1），对一个逻辑模块内按文档键规范排序的文档键、精确长度和逐文档摘要进行版本化、域分隔聚合所得的 SHA-256；它用于模块级重放/缓存比较，不替代逐文档身份。                        |
| 来源集合摘要            | source collection digest                                  | `sourceCollectionDigest`                | 已接受（#298 G1；未实现），按依赖优先模块顺序对命名空间及每模块来源文档集摘要进行版本化、域分隔聚合所得的 SHA-256；它绑定一次 LFSM 的完整来源模块集合，不替代逐文档摘要，也不进入路网修订标识。   |
| 来源语言                | source language                                           | —                                       | 定义来源模块语法和来源保真的输入语言。                                                                                                                                                            |
| 编制命名空间标识        | authoring namespace ID                                    | `authoringNamespaceId`                  | 隔离稳定标识域、且不依赖文件路径或遍历顺序的持久标识。                                                                                                                                            |
| 编译单元                | compilation unit                                          | —                                       | 由一个权威来源模块图及其显式选项共同构成的原子编译输入。                                                                                                                                          |
| 编译器中间表示          | compiler intermediate representation                      | compiler IR                             | 编译器内部从有类型抽象语法树经 HIR、MIR 到 canonical LIR 的有类型阶段表示总称；各阶段的精确职责由专门词条定义。                                                                                   |
| 编译器基础设施          | compiler foundation                                       | —                                       | 承载表示类型、编译遍驱动器、诊断、区块分配、确定性和编译发射器边界的公共基础。                                                                                                                    |
| 封闭契约                | closed contract                                           | —                                       | 以登记的闭合集合冻结允许的类型、字段、配置和组合；未知或未登记输入必须失败关闭，调用方不得任意扩展。                                                                                              |
| 前端                    | frontend                                                  | —                                       | 只负责特定来源语言的解析、类型化与来源位置，不拥有后续全局静态语义。                                                                                                                              |
| 合成来源模块            | synthetic source module                                   | `SyntheticModule`                       | 已接受（Accepted；#292 G1，已完成 G4），由 `SyntheticModuleBuilder` 受检构造并作为当前唯一官方具体模块进入编译单元；#315 不改变该类型的已接受状态。                                               |
| 官方前端模块            | official frontend module                                  | —                                       | 目标态已接受（#315 G1），对 LaneFlow 官方前端完成受检构造、以具体字段私有类型绑定来源记录身份、描述符、导入、声明和模块资源计数的模块所作的共同分类；它不是第三方扩展协议。                       |
| 共同受检模块接入        | shared checked module admission                           | —                                       | 目标态已接受（#315 G1），把不同官方前端模块原子移入同一编译单元，并统一执行唯一性、资源上限、导入图和规范顺序检查的编译器私有接入路径。                                                           |
| 模块资源计数            | module resource counts                                    | —                                       | 目标态已接受（#315 G1），由官方前端在受检构造期间一次性派生、供共同接入累计检查来源字节、声明、引用、关系、字符串、几何点和存续内存等上限的字段私有计数集合。                                     |
| 合成领域专用语言前端    | Synthetic DSL frontend                                    | —                                       | 面向测试、基准、示例和程序化场景的可重放来源前端。                                                                                                                                                |
| 道路编辑状态            | road editing state                                        | —                                       | 已接受（Accepted；#296 FlatBuffers G1），城市项目/存档中可继续编辑、可重放的 production 道路编制事实；保存当前道路走向、横断面、连接、规则和稳定关系，不包含运行时镜像、车辆状态或完整交互历史。  |
| 道路走向定义            | road alignment definition                                 | —                                       | 已接受（Accepted；#296 FlatBuffers G1），重建当前道路所需的编制描述；来源 v1 只保存 line/cubic Bézier，其他曲线按 B1 工程目标转换并记录观测误差；不属于运行时规范几何。                           |
| 道路编辑前端            | road editing frontend                                     | —                                       | 已接受（Accepted；#296 FlatBuffers G1），把版本化道路编辑状态有界解码为共同有类型道路编辑模型，并产生结构化来源位置的 production 前端。                                                           |
| 道路编辑来源缓冲区      | road editing source buffer                                | `LF-ROAD-EDITING-SOURCE-v1`             | 已接受（Accepted；#296 FlatBuffers G1），一个模块对应一个带 size prefix、`LFRE` file identifier 和 `RoadEditingSource` 根表的 FlatBuffers 文档；B1 仅授权内部验证，尚未发布、不承诺长期存档兼容。 |
| 道路编辑来源根表        | road editing source root                                  | `RoadEditingSource`                     | 已接受（Accepted；#296 FlatBuffers G1），保存精确格式版本、唯一模块头、道路走向定义及 Identity v1 有类型稳定声明向量；owner-local 值留在 owner table，table offset 不构成领域身份。               |
| 生成绑定边界            | generated binding boundary                                | `laneflow-road-editing-wire`            | 已接受（Accepted；#296 FlatBuffers G1），只承载固定 `flatc` Rust 生成物并隔离 generated accessor `unsafe` 的私有、不可发布 package；不成为公共 SDK、第二套领域模型或第三方前端接口。              |
| 有类型道路编辑模型      | typed road editing model                                  | —                                       | 已接受（Accepted；#296 FlatBuffers G1），由可视化编辑器、可发布程序化生成器、SDK 与 importer 共享的字段、单位、稳定身份、引用和不变量契约；物理编码不拥有第二套语义。                             |
| 候选道路修订            | candidate road revision                                   | —                                       | 已接受（Accepted；#296 FlatBuffers G1），尚未提交、必须先完成编译和验证，失败或取消时不得改变当前路网修订的道路编辑候选。                                                                         |
| 几何文档前端            | Geometry document frontend                                | —                                       | #296 旧 G1 的严格 JSON 方案；已由 G1 纠偏重启取代，不再作为 production source format 或实现输入。                                                                                                 |
| 道路编辑来源输入        | road editing source input                                 | `RoadEditingModuleInput`                | 已接受（Accepted；#296 FlatBuffers G1），借用完整 size-prefixed FlatBuffer bytes；由 `CompilationUnitBuilder::add_road_editing_module` 原子完成 verifier、语义预检、降阶和共同接入。              |
| 道路编辑来源地址        | road editing source address                               | `RoadEditingSourceAddress`              | 已接受（Accepted；#296 FlatBuffers G1），由模块 namespace、实体 kind、完整 owner-key tuple 和 sibling-local key 构成；不同 parent 可复用同名 local key；不参与稳定标识。                          |
| 道路编辑属性路径        | road editing property path                                | `RoadEditingPropertyPath`               | 已接受（Accepted；#296 FlatBuffers G1），由已知 table field、struct member 与 union variant 组成的闭合叶属性 step 序列；不接受任意字符串或向量下标。                                              |
| 道路编辑位置上下文      | road editing location context                             | `RoadEditingLocationContext`            | 已接受（Accepted；#296 FlatBuffers G1），在失败诊断或成功源映射中拥有 intern 属性路径、画布选择键与显示字节并解析紧凑 ordinal 的字段私有上下文。                                                  |
| 几何来源模块            | Geometry source module                                    | `GeometryModule`                        | #296 旧 JSON G1/G2 的候选具体模块名；FlatBuffers G1 以借用的 `RoadEditingModuleInput` 原子进入共同接入，不再公开预构造 `GeometryModule`。                                                         |
| 几何精度配置档          | Geometry accuracy profile                                 | `GeometryAccuracyProfile`               | #296 FlatBuffers G1 已接受的 B1 封闭三档工程目标：`2 cm`、`5 cm`、`10 cm`；固定采样与离线观测不构成连续最大误差保证或长期存档兼容承诺。                                                           |
| 几何方向配置档          | Geometry direction profile                                | `GeometryDirectionProfile`              | 已接受（Accepted；#296 FlatBuffers G1）的封闭三档折线方向连续性配置；从旧方案保留，与位置档正交组合、同一编译单元一致。                                                                           |
| 道路里程站位            | Stationing                                                | —                                       | 从道路参考线起点沿配置档无关的编译期 reference station 基表单调累计的距离坐标，用于区段、横断面和连接几何的纵向定位；不属于持久身份，也不由最终输出档位反向改变。                                 |
| 导入前端                | import frontend                                           | —                                       | 把外部来源及其工具、选项和来源沿袭显式转换为来源模块的前端。                                                                                                                                      |
| 当前 JSON               | current JSON                                              | —                                       | Traffic v0.10、SpatialPackage v0.1 与 ScenarioManifest v0.1 的合称；仅供当前内部加载器和仓库夹具使用，不是运行时快照、目标静态镜像、长期编制来源或已发布兼容格式。                                |
| 已验证当前态来源包      | validated current-source bundle                           | `ValidatedCurrentSourceBundle`          | 当前内部实现：原子绑定 DTO、三文档精确身份和场景清单配对的字段私有能力，只供 `laneflow-data` 消费；不进入 compiler，不证明发布真实性，并随旧加载路径删除。                                        |
| 编辑器编制界面          | editor authoring surface                                  | —                                       | 编辑并持久化来源模块、显示来源诊断的交互界面；它不私有化编译语义。                                                                                                                                |
| 交换格式                | interchange format                                        | —                                       | 可由独立工具跨进程或跨语言读写、具有显式版本与兼容规则的序列化数据契约。                                                                                                                          |
| 来源位置                | source location                                           | `SourceLocation`                        | 诊断和源映射使用的闭合和类型；文本来源使用真实 `SourceSpan` 行列，道路编辑来源使用稳定实体/owner-local subject、schema property 与可选画布选区，只有结构损坏可使用 byte range。                   |
| 道路编辑来源差异        | road-editing source diff                                  | `RoadEditingSourceDiff`                 | C 阶段后继 [#345](https://github.com/illusion-tech/laneflow/issues/345)，比较稳定道路编辑实体、属性和 authoring-only 改动；#296 A 只提供所需稳定位置，不实现该差异引擎。                          |
| 来源沿袭                | provenance                                                | —                                       | 输入、工具、参数、构建、转换与发布的可审计来源链。                                                                                                                                                |
| 降阶                    | lowering                                                  | —                                       | 把较高层表示确定性转换为较低层表示、且不补入后端私有语义的过程。                                                                                                                                  |
| 有类型抽象语法树        | typed abstract syntax tree                                | typed AST                               | 保留前端语法、显式键、类型、单位和来源位置的第一层表示。                                                                                                                                          |
| 高层中间表示            | high-level intermediate representation                    | HIR                                     | 已完成模块、命名空间、符号、引用、单位和编制语义解析的表示。                                                                                                                                      |
| 中层中间表示            | mid-level intermediate representation                     | MIR                                     | 已完成拓扑、几何展开和全局静态语义推导、但尚未绑定目标布局的表示。                                                                                                                                |
| 已验证规范低层中间表示  | validated canonical low-level intermediate representation | canonical LIR                           | 后端唯一输入；包含稳定标识、有类型序号、规范数值、确定性关系和布局无关预计算。                                                                                                                    |
| 已验证源映射输入        | validated source-map input                                | `ValidatedSourceMapInput`               | 目标态已接受（#292 G1），与 LIR 同次成功编译冻结、只保存来源模块、来源位置、来源沿袭及 LIR 键关联且不能补充静态语义的伴随数据。                                                                   |
| 已验证编译结果          | validated compilation output                              | `CompilationOutput`                     | 目标态已接受（#292 G1），原子拥有 LIR、已验证源映射输入和非错误级诊断的成功编译结果；错误级诊断存在时不得构造。                                                                                   |
| 编译观测值              | compilation metrics                                       | `CompilationMetrics`                    | #292 G1 修订后接受，只读报告一次成功生产编译的 LIR 记录、逻辑输出、编译器控制峰值和同版本语义指纹；不暴露私有阶段布局，也不等同进程内存或制品摘要。                                               |
| 编译遍                  | compiler pass                                             | pass                                    | 具有显式输入、输出、诊断和确定性要求的一次编译转换或验证步骤。                                                                                                                                    |
| 后端                    | backend                                                   | —                                       | 从已验证编译结果生成特定制品或镜像的编译器末端；静态语义只能来自 LIR，源映射可另读已验证来源伴随数据。                                                                                            |
| 区块分配键              | arena key                                                 | —                                       | 仅在一次编译的有类型区块分配器内有效、不得进入 LIR 或持久制品的私有 `u32` 引用。                                                                                                                  |
| 源映射                  | source map                                                | —                                       | 把规范实体、关系或诊断反向关联到来源模块和来源位置的派生制品。                                                                                                                                    |
| 源映射封套              | source map envelope                                       | `SourceMapEnvelope`                     | 版本化绑定精确规范制品、路网修订与编译来源沿袭，并承载源映射记录的派生制品封套。                                                                                                                  |
| 规范发布描述符          | canonical publication descriptor                          | `CanonicalPublicationDescriptor`        | 位于制品字节外，可信绑定规范制品、源映射、验证收据各自摘要与精确长度，以及路网修订和编译来源沿袭的发布值。                                                                                        |
| 语义差异                | semantic diff                                             | —                                       | 以稳定标识和所有者局部键比较两个规范制品语义变化的结构化结果；跨修订状态迁移时必须经独立验证并由可信切换描述符绑定。                                                                              |
| 语义差异封套            | semantic diff envelope                                    | `SemanticDiffEnvelope`                  | 版本化绑定旧/新路网修订与规范制品精确字节，并承载规范排序差异记录的派生制品封套。                                                                                                                 |
| 验证收据封套            | validation receipt envelope                               | `ValidationReceiptEnvelope`             | 以独立格式版本和封闭收据种类绑定受检对象、验证器构建及必需成功证据的可审计制品封套。                                                                                                              |
| 确定性合并顺序          | deterministic merge order                                 | —                                       | 并行或分片结果按固定规则合并，使输出不依赖线程调度。                                                                                                                                              |
| 编译资源上限            | compile limits                                            | `CompileLimits`                         | 目标态已接受（#292 G1），在后继阶段分配前限制来源、记录、关系、字符串、诊断、暂存区和编译器控制总存续内存的失败关闭边界；v0.10 首轮精确值由具名 P100 配置档冻结。                                 |
| P100 首轮编译资源配置档 | P100 initial compile limits profile                       | `LF-COMP-P100-INITIAL-v1`               | 目标态已接受（#292 G1），以 #308 压力分层逐维上包络冻结、由宿主显式选择且不提供无限模式的首轮生产编译资源上限向量。                                                                               |
| 来源文档计数            | source document count                                     | `SourceDocumentCount`                   | 目标态已接受（#315 G1），一个编译单元内独立来源文档描述符的累计数量；它不属于有类型抽象语法树领域记录，不能挤入 `TypedAstRecordCount`。                                                           |
| 第二版编译资源配置档    | P100 second compile limits profile                        | `LF-COMP-P100-INITIAL-v2`               | 目标态已接受（#315 G1），保持仅以 `ModuleCount` 覆盖一模块一文档的 v1 不变；v2 新增来源文档计数上限，多文档模块必须显式选择该档或后继同维度版本，并重新取得资格。                                 |
| P100 首轮编译性能门槛   | P100 initial compiler performance gate                    | `LF-COMP-P100-R0-v1`                    | #292 原 G1 输入；G2 发现 #308 抽象工作负载不能无损映射生产语义，后继 G1 修订已将其降为非门禁容量估算和实现选型输入，不构成产品通过（Product Pass）。                                              |
| P100 首轮生产编译基线   | P100 initial production compiler baseline                 | `LF-COMP-P100-PRODUCTION-R0-v1`         | #292 G1 修订后接受，以真实合法产品场景在 `LF-P100-REF-01` 上形成的首轮描述性生产 R0；用于后继同机回退对照，不是产品 SLA 或城市容量。                                                              |
| 干净单工作线程编译      | clean single-thread compile                               | —                                       | 目标态已接受（#292 G1），不使用增量缓存、只由一个工作线程从完整来源执行全部生产编译遍的确定性参考路径；内存 LIR 保持规范语义一致，后继制品存在时保持精确字节一致。                                |
| 确定性预言机            | deterministic oracle                                      | —                                       | 其他执行模式必须在冻结的语义或精确字节边界上匹配的参考执行路径。                                                                                                                                  |
| 研究干净单工作线程编译  | clean single-thread research compile                      | —                                       | 提案中（Proposed；#308），由一个工作线程从完整研究来源执行全部代表性研究编译遍的非生产测量路径；不得冒充完整生产编译器。                                                                          |
| 冷实例编译              | cold-instance compile                                     | —                                       | 目标态已接受（#292 G1；研究证据由 #308 G4 提供），以新建编译器或研究管线实例和空暂存容量执行的干净编译；规模相关来源物化是否计时由具名基准协议冻结，进程启动与磁盘读取不计时。                    |
| 稳定容量复用编译        | stable-capacity reuse compile                             | —                                       | 目标态已接受（#292 G1；研究证据由 #308 G4 提供），复用编译器或研究管线的无语义暂存容量、但不复用语义结果的干净编译；它不是增量编译，结果和诊断必须与冷实例编译一致。                              |
| 完整进程样本            | complete-process sample                                   | `complete-process`                      | 提案中（Proposed；#308），由父进程监控覆盖一次完整子进程生命周期所得的单一样本；不能无依据地拆分为该进程内的冷实例或稳定容量复用样本。                                                            |
| 编译器校准规模          | compiler calibration scale                                | —                                       | 目标态已接受（#292 G1；研究证据由 #308 G4 提供），用于观察固定成本、复杂度及缓存/内存拐点的合成编制/中间表示规模；不表示真实城市或产品容量。                                                      |
| 编译器压力规模          | compiler stress scale                                     | —                                       | 目标态已接受（#292 G1；研究证据由 #308 G4 提供），在研究停止护栏内放大编制/中间表示对象、用于暴露资源增长和失败边界的合成规模；不构成产品 SLA。                                                   |
| 研究工作负载清单        | research workload manifest                                | —                                       | 提案中（Proposed；#308），机器可读地冻结工作负载修订、模块图、生成规则、字符串/来源位置类别、记录布局、计数和夹具绑定的非生产契约。                                                               |
| 研究停止护栏            | research stop guardrail                                   | —                                       | 提案中（Proposed；#308），为保护研究机器和限制实验成本而阻止启动或继续放大实验的条件；不得直接改写成生产性能预算。                                                                                |
| 研究停止护栏触发        | research stop guardrail trigger                           | `research-stop-guardrail-triggered`     | 提案中（Proposed；#308），父进程资源监控达到研究停止边界的具名无效原因；精确维度由 `guard.trigger` 保存，正常退出竞态也不得改写成 `monitoring-gap`。                                              |
| 受控分配硬上限          | controlled allocation hard ceiling                        | —                                       | 提案中（Proposed；#308），研究子进程在每次规模相关容量请求前执行、并通过可失败分配保证不会越过的请求字节上限。                                                                                    |
| 研究基准执行器          | research benchmark harness                                | —                                       | 提案中（Proposed；#308），生成固定工作负载、执行测量并输出机器可读证据的非生产程序；不得进入生产编译器公共 API 或依赖图。                                                                         |
| 精确研究预言机          | exact research oracle                                     | —                                       | 提案中（Proposed；#308），不复用受测符号解析、关系展开或容器候选，以直观确定性算法独立核对研究输出的非生产参考实现。                                                                              |
| 研究语义摘要            | research semantic digest                                  | `semanticDigest`                        | 提案中（Proposed；#308），在主计时区外对研究专用规范记录流计算的 SHA-256；只证明研究候选等价，不是生产制品、路网修订摘要或受测编译成本。                                                          |
| 声明局部序号            | declaration-local ordinal                                 | `declarationLocalOrdinalWithinKind`     | 提案中（Proposed；#308），在一个工作负载单元内按实体种类和规范声明构造顺序分配、且在输入置换前冻结的零起始序号；用于展开研究配置键，不是来源遍历序号。                                            |
| 研究语义标量代码        | research semantic scalar code                             | `semanticScalarEncodings`               | 提案中（Proposed；#308），把准入决策、信号灯色等研究语义枚举冻结为明确整数，并冻结 Gate 位置和等待区容量等标量构造公式的记录流契约。                                                              |
| 研究记录所有者          | research record owner                                     | —                                       | 提案中（Proposed；#308），由研究语义记录 envelope 的实体种类与稳定标识共同指认、负责拥有该记录的领域实体；载荷中的被引用实体不得替代它。                                                          |
| 同类所有者序号          | per-kind owner ordinal                                    | `owner_ordinal`                         | 提案中（Proposed；#308），把同一实体种类的研究记录所有者按 StableId128 无符号逐字节排序后得到的零起始稠密序号；必须从身份/声明流重算，不得由执行器自由分配。                                      |
| 有类型诊断载荷          | typed diagnostic payload                                  | `typedPayloadBytes`                     | 提案中（Proposed；#308），按诊断代码分别冻结字段、宽度和顺序的机器载荷；自然语言错误文本不进入研究诊断摘要。                                                                                      |
| 规范诊断顺序            | canonical diagnostic order                                | `diagnosticStream.canonicalOrderKey`    | 提案中（Proposed；#308），按来源位置、诊断代码、严重度和有类型载荷冻结的诊断候选全序；诊断上限只截取该全序前缀。                                                                                  |
| 编译器测量基准规模      | compiler measurement base scale                           | `B`                                     | 提案中（Proposed；#308），首个同时满足时钟量化、样本稳定和护栏要求的二次幂工作单元规模；不表示产品基线。                                                                                          |
| 基准规模试运行摘要      | base-scale pilot summary                                  | `baseScales[].pilotLevels[]`            | 提案中（Proposed；#308），绑定一个候选规模的七个有效新进程运行，并保存墙钟中位数/MAD、语义摘要一致性、护栏与合格判断的机器记录。                                                                  |
| 成本拐点                | cost knee                                                 | —                                       | 提案中（Proposed；#308），单位记录时延或存续内存按预声明阈值持续上升、并在独立执行批次重现的规模级别。                                                                                            |
| 重复性包络              | reproducibility envelope                                  | `E_m`                                   | 提案中（Proposed；#308），以指标为自然身份，绑定产生最坏双向比值的两批阶梯汇总；用于衡量同环境重复性。                                                                                            |
| 平衡候选顺序            | balanced candidate order                                  | —                                       | 提案中（Proposed；#308），让每个候选等次数出现在每个执行位置并同时覆盖正向/反向相邻关系的预先冻结执行序列。                                                                                       |
| 编译器控制总存续内存    | compiler-controlled total live memory                     | —                                       | 目标态已接受（#292 G1；研究证据由 #308 G4 提供），编译请求同时拥有的来源、各阶段记录、字符串、诊断、暂存区和输出构造的存续请求字节总量；不等于进程工作集。                                        |
| 研究阶段记录模型        | research stage record model                               | —                                       | 提案中（Proposed；#308），为非生产编译器预算研究替身冻结阶段记录字段、布局、粒度及记录/载荷/逻辑字节公式；不定义生产 IR 或公共 API。                                                              |
| 研究字符串聚合          | research string aggregate                                 | —                                       | 提案中（Proposed；#308），按模块名、来源文档键、导入目标、命名空间、配置键、来源引用和共享常量的规范顺序，计算字符串项数、最大项字节数和总字节数。                                                |
| 当前固定样例研究投影    | current-fixture research projection                       | —                                       | 提案中（Proposed；#308），把摘要绑定的当前 JSON 固定样例按闭合枚举（closed enumeration）映射到非生产研究阶段记录、领域计数和逻辑字节；不定义 #292 的生产投影。                                    |
| 研究输入变体            | research input variant                                    | `inputVariantId`                        | 提案中（Proposed；#308），在不改变所选工作负载级别的前提下，稳定标识规范有效输入或具名语义/诊断变异的机器可读构造。                                                                               |
| 停车锚点记录            | parking anchor record                                     | record kind `11`                        | 提案中（Proposed；#308），以一条研究语义记录原子保存一个停车位入口与出口各自的车道边和规范进度；不依靠两条相邻记录配对。                                                                          |
| 限制维度                | limit dimension                                           | `dimensionId`                           | 提案中（Proposed；#308），失败关闭研究中可独立设置恰好等于上限与上限加一配对输入的具名资源或记录维度。                                                                                            |
| 失败用例标识符          | failure case ID                                           | `caseId`                                | 提案中（Proposed；#308），稳定区分限制边界、语义错误和诊断截断实验的机器可读标识符；运行标识符不能替代它。                                                                                        |
| 清理实验                | cleanup experiment                                        | —                                       | 提案中（Proposed；#308），以同一实验标识符和固定序号关联合法基线、重复失败、恢复成功与新实例判定基准的失败恢复验证序列。                                                                          |
| 编译器实例身份          | compiler instance identity                                | `compilerInstanceId`                    | 提案中（Proposed；#308），由研究执行器在编译器实例构造时一次性签发、随实例存续且销毁后不得复用的机器身份；不能用进程号、内存地址或运行标识符替代。                                                |
| 候选注册表              | candidate registry                                        | `candidateRegistry`                     | 提案中（Proposed；#308），机器可读地把闭合候选标识符绑定到允许键域、各键域唯一比较基线、种子策略、算法常量和依赖组件身份的版本化清单；自由字符串候选不得参加排名。                                |
| 候选参赛名单            | candidate roster                                          | `candidateRosters[]`                    | 提案中（Proposed；#308），在性能轮次开始前按候选注册表顺序覆盖某一键域全部候选、记录资格处置并据此唯一派生平衡顺序参与者数量 `C` 的机器记录。                                                     |
| 恒定哈希资格验证        | constant-hash qualification                               | `constantHashQualifications[]`          | 提案中（Proposed；#308），以所有键产生同一哈希值的受控构建器和独立预言机，验证内部快速哈希候选仍执行完整键相等、产生确定性语义/诊断并匹配精确结果的机器证据。                                     |
| 候选依赖审计            | candidate dependency audit                                | `dependencyAudit`                       | 提案中（Proposed；#308），为候选的每个容器、哈希器、适配器或排序器组件记录许可证、最低支持 Rust 版本、安全公告和锁文件绑定的机器可读审计。                                                        |
| 安全公告审计            | security advisory audit                                   | `securityAudit`                         | 提案中（Proposed；#308），以工具、数据库快照和观察时间为来源，闭合记录无已知公告、存在公告、不可用或不适用状态；来源不可用不得自报完成。                                                          |
| 键域比较基线            | key-domain comparison baseline                            | `baselineByKeyDomain`                   | 提案中（Proposed；#308），为外部字符串、已验证定长键和规范输出顺序分别冻结的唯一候选比较分母；受测候选或其他优化实现不得替代。                                                                    |
| 测量分层                | measurement stratum                                       | `measurementStratum`                    | 提案中（Proposed；#308），由键域、工作负载/修订、模块图/字符串配置档、生成器、规模、用例、样本种类与二进制模式共同标识、禁止跨字段聚合的一次测量范围。                                            |
| 候选比较分层            | candidate comparison stratum                              | `stratum`                               | 提案中（Proposed；#308），由键域、工作负载配置、规模、样本种类和二进制模式共同标识的一次候选比较范围；不同分层不得聚合。                                                                          |
| 轮次指标汇总            | round metric summary                                      | `roundMetricSummaries[]`                | 提案中（Proposed；#308），按候选、完整测量分层、指标、批次和轮次引用全部有效原始运行，并以精确整数重算中位数与中位绝对偏差的机器记录。                                                            |
| 轮次尝试身份            | round-attempt identity                                    | `roundAttempt`                          | 提案中（Proposed；#308），把同一正式或候选轮次的一次执行与其后重试区分开的稳定身份、重试序号和范围；整轮作废原因必须传播到该尝试已取得的全部样本。                                                |
| 正式阶梯批次汇总        | ladder batch summary                                      | `ladderBatchSummaries[]`                | 提案中（Proposed；#308），引用正式阶梯一个批次的五个轮次指标汇总，并保存跨轮精确整数中位数与中位绝对偏差的机器记录。                                                                              |
| 相邻级别轮次对          | adjacent-level round pair                                 | `adjacentLevelRatios[].roundPairs[]`    | 提案中（Proposed；#308），在同一批次、同一轮和除规模外相同的完整测量分层内，引用上下相邻级别轮次指标汇总并保存精确规范化比值的机器记录。                                                          |
| 拐点评估                | knee assessment                                           | `knees[]`                               | 提案中（Proposed；#308），以候选、指标和上下级分层为自然身份，引用 batch 0/1 比值，并绑定性能分析制品或明确登记未归因状态；规模由上级分层派生。                                                   |
| 候选比较轮次对          | candidate comparison round pair                           | `roundPairs[]`                          | 提案中（Proposed；#308），在同一批次、同一平衡轮次和同一完整分层内，把恰好一个基线轮次指标汇总与一个候选轮次指标汇总一一绑定的机器可读比较单元。                                                  |
| 精确中位比值            | exact median ratio                                        | `medianRatio`                           | 提案中（Proposed；#308），对同轮精确正整数比值排序后，以中间两项的精确算术平均求得并约分的中位数；计算与比较均不得经过浮点舍入。                                                                  |
| R0 研究预算建议         | R0 research budget recommendation                         | `recommendations[]`                     | 提案中（Proposed；#308），以完整分层与指标为自然身份，绑定两批汇总、重复性包络和闭合公式/舍入规则的 #292 输入；不是产品 SLA。                                                                     |
| 正式研究处置            | formal study disposition                                  | `formalStudyDisposition`                | 提案中（Proposed；#308），区分未建立任何可靠基准、正式阶梯不足和正式分析可用三种研究终止状态，并约束相应派生数组能否存在的机器结论。                                                              |
| 编译器校准契约描述符    | compiler calibration contract descriptor                  | `compiler-calibration-contract-v1.json` | 提案中（Proposed；#308），从证据 Schema 外部绑定其自身与工作负载清单精确字节身份的非自指描述符；正式验证必须在解析证据前完成该外部绑定。                                                          |
| 最后私有内存快照        | last private-memory snapshot                              | `lastPrivateBytes`                      | 提案中（Proposed；#308），父进程最近一次实际采集的子进程私有字节观察；子进程未启动或监控缺样时必须使用 `null + reason`，不得伪造为零。                                                            |
| 研究来源提交            | research source commit                                    | `sourceCommit`                          | 提案中（Proposed；#308），绑定受信任契约描述符、工作负载清单、证据 Schema 与被测来源的精确 Git 提交；不自动标识研究包内本地候选组件。                                                             |
| 研究执行器提交          | research harness commit                                   | `harnessCommit`                         | 提案中（Proposed；#308），绑定研究执行器、独立预言机及研究包内本地候选组件实际实现的精确 Git 提交；可以与研究来源提交不同。                                                                       |
| 干净研究工作树          | clean research working tree                               | `dirty = false`                         | 提案中（Proposed；#308），研究来源提交与研究执行器提交对应的受测源码、执行器、清单、Schema 和锁文件均无未提交修改、可由两项提交完整复现的正式测量工作树。                                         |
| 存续字节基准预扫描      | live-byte baseline prescan                                | `limit-baseline`                        | 提案中（Proposed；#308），以冻结基线配置和两个独立全新进程测得一致峰值，为编译器控制存续字节限制配对提供候选无关精确值的归因测量。                                                                |
| 清单单缓冲区下界        | manifest single-buffer lower bound                        | `manifest-single-buffer-lower-bound-v1` | 提案中（Proposed；#308），从绑定清单重算各研究阶段单个受控缓冲区的精确字节并取最大值，作为首级编译器控制存续字节的候选无关停止护栏下界；不叠加候选自报固定开销。                                  |
| 子进程终止观察          | child termination observation                             | `termination`                           | 提案中（Proposed；#308），分别记录未启动、正常退出码、POSIX 信号或其他平台状态；信号终止不得伪造数值退出码，并须保留信号号或原始平台状态。                                                        |
| 当前核心对象图          | current Core object graph                                 | —                                       | 当前 `laneflow-core` 以对象、句柄和登记表承载的运行时构造结果；它用于迁移预言机对照，不是编译器中间表示。                                                                                         |
| 当前空间登记表          | current Spatial registry                                  | `SpatialRegistry`                       | 当前 `laneflow-spatial` 的规范几何登记与采样对象；它用于迁移预言机对照，不是编译器中间表示或目标静态镜像。                                                                                        |

## 4. 标识、引用与数据布局

| 中文规范术语     | 英文辅助名（English Alias） | 精确标识符 / 缩写        | 中文规范含义                                                                                                       |
| ---------------- | --------------------------- | ------------------------ | ------------------------------------------------------------------------------------------------------------------ |
| 规范标识元组     | canonical identity tuple    | `CanonicalIdentity`      | 由编制命名空间、实体种类及稳定父/局部锚点组成的标识权威。                                                          |
| 规范身份表       | canonical identity table    | `CanonicalIdentityTable` | 可移植规范制品中保存每个稳定实体完整字段前像、声明稳定标识与有类型逻辑序号，供独立验证器重算身份的不可裁剪语义表。 |
| 标识编码封装     | identity encoding envelope  | —                        | 冻结魔数、编码版本、实体种类、字段数量和标签/长度/值序列的公共字节外壳。                                           |
| 稳定标识         | stable identifier           | `StableId128`            | 对规范标识元组执行冻结编码和 BLAKE3-128 后得到的 128 位持久标识。                                                  |
| 实体种类         | entity kind                 | `entityKind`             | 标识登记表中具有固定代码、英文短名（slug）和必需字段序列的实体类别。                                               |
| 字段标签         | field tag                   | `field_tag`              | 标识编码中用于标明规范字段语义的冻结数字标签。                                                                     |
| 稳定锚点         | stable anchor               | —                        | 显式持久键或父实体稳定标识；坐标、数组位置和遍历顺序不得作为锚点。                                                 |
| 有类型逻辑序号   | typed logical ordinal       | —                        | 在单次 LIR/镜像内按表类型区分的 `u32` 行序号。                                                                     |
| 密集句柄         | dense handle                | —                        | 运行时使用的紧凑有类型引用；热路径不携带字符串或稳定标识。                                                         |
| 所有者局部关系   | owner-local relation        | —                        | 只在所属实体的当前序列快照内有意义、没有全局稳定标识的关系记录。                                                   |
| 所有者局部出现项 | owner-local occurrence      | —                        | 由 `(ownerOrdinal, role, localIndex)` 定位的当前快照记录；`localIndex` 不是跨编译标识。                            |
| 唯一所有者关系   | unique owner relation       | —                        | 一个成员必须解析到恰好一个有效所有者；零所有者或多所有者均为结构化错误。                                           |
| 完备所有者树     | complete owner tree         | —                        | 所有要求归属的成员都具有唯一所有者、且不存在重复归属或孤儿成员的所有者 / 成员结构。                                |
| 反向索引         | reverse index               | —                        | 从目标或成员回到所有者、关系或候选集合的预计算索引。                                                               |
| 交叉索引         | cross-index                 | —                        | 在两个有类型表或镜像节之间预计算的定向引用索引。                                                                   |
| 数组分列结构     | structure of arrays         | SoA                      | 按字段分列保存同类记录，以提高连续访问和向量化效率的数据布局。                                                     |
| 压缩稀疏行       | compressed sparse row       | CSR                      | 用偏移量/区间表示变长邻接或成员序列的紧凑布局。                                                                    |
| 扁平区间         | flat range                  | —                        | 以起点和长度引用连续表片段、避免逐项指针的数据表示。                                                               |
| 零拷贝           | zero-copy                   | —                        | 在验证后直接借用底层镜像字节建立视图，不复制静态表内容的访问方式。                                                 |
| 热路径           | hot path                    | —                        | 高频固定步进/位姿执行路径；禁止字符串、哈希查找和持久标识计算。                                                    |
| 热数据           | hot data                    | hot                      | 固定步进或位姿批次高频读取、必须按连续布局和缓存局部性优化的数据；热度不单独决定配置档必选性。                     |
| 温数据           | warm data                   | warm                     | 低于稳态热路径频率、但仍可能由运行时查询或边界操作读取的数据；是否必选由有类型节与封闭配置档裁决。                 |
| 冷数据           | cold data                   | cold                     | 主要在加载、恢复、修订切换或诊断边界读取的数据；“冷”不表示可以从生产配置档裁剪。                                   |
| 保留内存         | retained memory             | —                        | 初始化完成后仍由组件持有的内存。                                                                                   |
| 峰值分配         | peak allocation             | —                        | 某操作期间同时存在的最大动态分配量。                                                                               |

## 5. 通用编译、数据与运行术语

| 中文规范术语         | 英文辅助名（English Alias）            | 精确标识符 / 缩写                  | 中文规范含义                                                                                                                     |
| -------------------- | -------------------------------------- | ---------------------------------- | -------------------------------------------------------------------------------------------------------------------------------- |
| 编译器拥有的静态路网 | compiler-owned static network          | —                                  | 全部静态标识、拓扑、几何、规则和索引由编译器一次性裁决的目标架构。                                                               |
| 静态路网             | static network                         | —                                  | 不随世界固定步进改变的道路、关系、规则、几何和预计算索引集合。                                                                   |
| 静态契约             | static contract                        | `laneflow-static-contract`         | 编译器、验证器、镜像、运行时与空间层共享的目标中立值、登记表和版本契约。                                                         |
| 包依赖图             | crate dependency graph                 | crate DAG                          | Rust 包之间必须保持无环、且不得让静态组件反向依赖动态运行时的依赖关系图。                                                        |
| 拓扑                 | topology                               | —                                  | 边、节点、连接、所有权和可遍历关系。                                                                                             |
| 几何                 | geometry                               | —                                  | 坐标框架、曲线、折线、弧长和采样数据。                                                                                           |
| 覆盖关系             | coverage                               | —                                  | 门、等待区、冲突区或规则对路径/出现项的适用范围。                                                                                |
| 出现项               | occurrence                             | —                                  | 某静态声明在路线（Route）、路径或所有者序列中的具体位置记录。                                                                    |
| 所有者 / 成员        | owner / member                         | —                                  | 负责组织事实的实体及其直接从属记录。                                                                                             |
| 元数据               | metadata                               | —                                  | 可重算或辅助解释、但不单独拥有行为权威的数据。                                                                                   |
| 诊断                 | diagnostic                             | —                                  | 带稳定代码、严重程度、来源位置、原因和建议的编译或验证反馈。                                                                     |
| 登记表               | registry                               | —                                  | 按稳定规则保存实体、句柄或解析关系的集合。                                                                                       |
| 解析器               | resolver                               | —                                  | 把外部标识、稳定标识或键解析为有类型引用的组件。                                                                                 |
| 规范化               | normalization                          | —                                  | 把输入转换为满足统一单位、默认、顺序和不变量的规范形态。                                                                         |
| 重绑定               | rebind                                 | —                                  | 在另一个图、世界或制品上下文中重新解析并建立引用。                                                                               |
| 联结                 | join                                   | —                                  | 按共同键把两个独立制品或表的记录关联起来。                                                                                       |
| 编译发射器           | compiler emitter                       | emitter                            | 从已验证 LIR 生成制品或镜像的后端。                                                                                              |
| 静态执行约束图       | static execution constraint graph      | —                                  | 编译器从静态资源依赖派生的工作线程数无关约束、可切分边界与规范合并顺序。                                                         |
| 分区规划提示         | partition planning hints               | `PartitionPlanningHints`           | 可由编译器随镜像发布、但可安全忽略或重建且不拥有行为语义的性能提示；v1 封闭配置档仍要求该有类型节存在。                          |
| 分区                 | partition                              | —                                  | 运行时执行计划中的私有物理所有权/调度单元；不构成公共身份或静态语义。                                                            |
| 工作线程             | worker                                 | —                                  | 执行一个或多个运行时任务的计算资源；数量和完成顺序不得改变精确结果。                                                             |
| 提案                 | proposal                               | —                                  | 并行求值阶段生成、尚未取得提交权威的候选状态变化。                                                                               |
| 资源声明             | resource claim                         | claim                              | 对共享交通资源提出的确定性占用请求；必须经规范归约后才能提交。                                                                   |
| 边界交换             | boundary exchange                      | —                                  | 同一 tick 内分区间传递规范输入、提案或资源声明的私有运行时过程。                                                                 |
| 规范归约权威         | canonical reduction authority          | —                                  | 对一个连接资源组件按稳定键形成唯一提交结果的逻辑权威；不要求由单一物理线程串行执行。                                             |
| 归约工作量           | reduction work                         | —                                  | 完成一次规范归约需要执行的总操作量，用于区别总成本与临界路径。                                                                   |
| 归约跨度             | reduction span                         | —                                  | 在无限并行资源下仍不可消除的规范依赖临界路径成本。                                                                               |
| 强连通分量           | strongly connected component           | SCC                                | 资源依赖图中任意两节点互相可达、需要作为循环依赖分析单元的最大子图。                                                             |
| 凝聚有向无环图       | condensation directed acyclic graph    | condensation DAG                   | 把强连通分量收缩为节点后得到、可用于确定性波次调度的有向无环图。                                                                 |
| 阿姆达尔瓶颈         | Amdahl bottleneck                      | —                                  | 不可并行的临界路径限制整体加速比、使增加工作线程不再获得相称收益的扩展性瓶颈。                                                   |
| 确定性基数/桶排序    | deterministic radix/bucket sorting     | —                                  | 对固定宽度规范键采用与输入规模线性相关、且不依赖线程完成顺序的排序策略族。                                                       |
| 精确参考预言机       | exact reference oracle                 | —                                  | 以直观集中式算法产生规范结果、用于证明优化执行路径语义等价而不要求成为生产热路径的实现。                                         |
| 精确执行路径         | exact execution path                   | —                                  | 不降低交通保真度、且结果不依赖工作线程数或分区计划的执行路径。                                                                   |
| 融合单工作线程执行器 | fused single-worker executor           | —                                  | 保留精确路径可观察语义、但不物化无并发价值的队列、任务图或真实屏障的单工作线程实现。                                             |
| 边界邻域             | halo                                   | —                                  | 分区为同一 tick 依赖读取的相邻状态范围；不能通过过期一 tick 数据改变精确语义。                                                   |
| 屏障                 | barrier                                | —                                  | 一个逻辑阶段等待全部必需分区输入就绪后才允许进入规范归约或提交的同步边界。                                                       |
| 保真度契约           | fidelity contract                      | —                                  | 明确允许的个体、时间、空间与行为语义损失及其验证边界的版本化契约。                                                               |
| 交通保真度           | traffic fidelity                       | —                                  | 交通个体、时间、空间、规则与事件语义相对精确路径的保留程度。                                                                     |
| 过载优雅降级         | graceful overload degradation          | —                                  | 宿主在过载时通过暂停、慢放或统一调整时间推进保持全部逻辑步进、事件和交通语义的显式处置；不等同于保真度降级。                     |
| 保真度降级           | fidelity degradation                   | —                                  | 显式减少个体、时间、空间或行为语义的目标能力变化；必须由独立保真度契约和 G1/ADR 授权，不能伪装成调度或表现优化。                 |
| 工作负载             | workload                               | —                                  | 冻结输入、规模、拓扑、行为组合和测量协议的可复现实验场景。                                                                       |
| 中国特色城市工作负载 | Chinese-style city workload            | —                                  | 用版本化拓扑、需求与运行时指标表达中国特色交通压力的代表性工作负载族。                                                           |
| 多世界集合           | multi-world ensemble                   | ensemble                           | 共享静态数据、但各自持有独立可变状态的一组世界，用于回放、探索或吞吐测量。                                                       |
| 构建器               | builder                                | —                                  | 按已冻结输入构造目标对象的组件；不得隐式补充未文档化语义。                                                                       |
| 加载器               | loader                                 | —                                  | 读取并验证输入、构造当前态对象或目标态视图的边界组件。                                                                           |
| 缓存                 | cache                                  | —                                  | 只影响复用和性能、不改变规范结果的暂存数据。                                                                                     |
| 指纹                 | fingerprint                            | —                                  | 用于快速候选匹配的短摘要；命中后仍须比较完整权威值。                                                                             |
| 快照                 | snapshot                               | —                                  | 在明确时点冻结、供只读消费的运行时状态视图。                                                                                     |
| 视图                 | view                                   | —                                  | 借用或共享已验证底层存储、且不复制权威数据的只读访问接口。                                                                       |
| 句柄                 | handle                                 | —                                  | 进程或世界内使用的有类型实体引用。                                                                                               |
| 键                   | key                                    | —                                  | 在明确作用域内区分声明或关系的值；只有冻结为稳定锚点时才参与持久标识。                                                           |
| 摘要                 | digest                                 | —                                  | 对完整字节或规范值计算的固定长度校验值。                                                                                         |
| 表行                 | table row                              | row                                | 静态表中的一条记录；每条 LIR/镜像表行都有有类型逻辑序号。                                                                        |
| 世界                 | world                                  | —                                  | 共享静态数据并持有一份独立可变运行时状态的实例。                                                                                 |
| 运行时执行计划       | runtime execution plan                 | —                                  | 每世界依据静态约束、硬件与动态负载建立的分区、工作线程、边界交换和迁移计划。                                                     |
| 路网修订             | network revision                       | —                                  | 在其生命周期内不可变、经编译和验证，并由受认证路网修订标识绑定的一个静态路网版本。                                               |
| 规范路网语义载荷     | canonical network semantic payload     | `canonicalNetworkSemanticPayload`  | 排除工具来源、诊断和目标布局，完整冻结运行时可观察静态路网语义及其派生契约版本的目标无关规范字节。                               |
| 路网修订标识         | network revision ID                    | `NetworkRevisionId`                | 对规范路网语义载荷计算、由独立验证和外部描述符认证的版本化不透明摘要；只支持相等性比较。                                         |
| 镜像切换事务         | image cutover transaction              | —                                  | 世界在显式安全边界从一个可信路网修订原子切换到另一个修订的失败关闭事务。                                                         |
| 增量追赶             | delta catch-up                         | —                                  | 候选世界在后台按规范顺序应用基准状态之后的迁移增量，逐步追近运行中旧世界提交点的阶段。                                           |
| 迁移增量日志         | migration delta journal                | —                                  | 在线迁移准备期间有界记录已提交动态状态/生命周期变化及命令/事件游标、供候选世界迁移追赶的版本化日志。                             |
| 已提交变更流         | committed mutation stream              | —                                  | 旧世界在原子提交后产生、供候选迁移状态重解释且不会重复执行输入或发布重复旧世界事件的规范动态状态变化序列。                       |
| 切换事件批次         | cutover event batch                    | —                                  | 迁移函数生成、在准备期保持不可见，并只与新镜像/状态绑定原子提交一次的规范排序事件集合；切换放弃时零发布。                        |
| 静默提交窗口         | quiescent commit window                | —                                  | 固定步进安全边界上短暂冻结旧世界输入、排空日志尾并原子切换绑定的有界时间窗口。                                                   |
| 维护暂停模式         | paused maintenance mode                | —                                  | 由宿主显式声明世界已暂停、允许一次性完成迁移且单独预算完整停顿的切换模式。                                                       |
| 路网修订切换描述符   | network revision cutover descriptor    | `NetworkRevisionCutoverDescriptor` | 在镜像外可信绑定两侧修订、制品、镜像、语义差异、验证收据与迁移策略的切换输入。                                                   |
| 运行时快照           | runtime snapshot                       | —                                  | 绑定版本化路网修订、记录原规范制品及原静态镜像摘要与精确长度，并可借助稳定身份索引在可信镜像上精确恢复的每世界可变状态存档制品。 |
| 快照局部标识         | snapshot-local identity                | —                                  | 只在一个运行时快照内稳定、用于重建动态实体引用且不复用原进程句柄的持久键。                                                       |
| 输入命令序列         | input command sequence                 | —                                  | 按规范顺序驱动世界、可与检查点共同重放的显式外部命令流。                                                                         |
| 检查点               | checkpoint                             | —                                  | 在回放序列中周期保存、用于缩短恢复和失同步定位区间的版本化状态锚点。                                                             |
| 存档清单             | save manifest                          | —                                  | 原子绑定城市游戏、出行编排和 LaneFlow 各版本化存档制品及摘要的上层清单。                                                         |
| 已提交交通观测快照   | committed traffic observation snapshot | —                                  | 从交通运行时已提交状态提取、且不泄漏固定步进中间状态的密度、速度、封闭等观测视图。                                               |
| 观测导出节奏         | observation export cadence             | —                                  | 上层或宿主明确选择的观测完整基线/增量导出频率；不等同于每个固定步进全网复制。                                                    |
| 完整/增量导出        | full/delta export                      | —                                  | 以周期完整基线和其后的版本化变化集共同表达已提交交通观测的边界协议。                                                             |
| 动态成本快照         | dynamic cost snapshot                  | —                                  | 路径规划层从静态路网、已提交交通观测与上层政策派生的版本化路由成本视图。                                                         |
| 路径规划服务         | routing service                        | —                                  | 从静态路网和已提交动态成本快照生成候选路径、但不拥有交通参与单元固定步进的服务。                                                 |
| 出行编排层           | trip orchestration layer               | —                                  | 拥有出行需求、候选路线选择、收费/政策偏好和调用方随机流，并以显式边界驱动 LaneFlow 的上层系统。                                  |
| 出行需求             | travel demand                          | —                                  | 描述谁在何时为何出发的上层输入；不由交通运行时隐藏生成。                                                                         |
| 运行时覆盖层         | runtime overlay                        | —                                  | 不改变静态身份或拓扑、按显式命令暂时修改运行时约束的版本化动态状态。                                                             |
| 会话                 | session                                | —                                  | 宿主或空间层中具有明确生命周期和资源所有权的一次活动上下文。                                                                     |
| 生命周期             | lifecycle                              | —                                  | 交通参与单元、动态通行定义、世界或会话从创建、活动到释放的状态变化规则；当前投影包括车辆与路线。                                 |
| 角色                 | role                                   | —                                  | 在所有者局部键中区分同一所有者下不同关系序列的有类型语义。                                                                       |
| 局部索引             | local index                            | `localIndex`                       | 当前所有者序列快照中的位置；不构成跨编译稳定标识。                                                                               |
| 接近臂               | approach                               | —                                  | 路口中指向入口或出口方向的有向道路接近关系。                                                                                     |
| 约束                 | constraint                             | —                                  | 对有效值、行为候选或运行时结果施加的不可绕过条件。                                                                               |
| 策略                 | policy                                 | —                                  | 在既定事实和安全约束内解释候选或选择行为的可版本化规则。                                                                         |
| 授权                 | grant                                  | —                                  | 运行时对特定车辆在当前条件下通过冲突资源的许可。                                                                                 |
| 预约                 | reservation                            | —                                  | 对停车位、冲突资源或其他有界资源的持有关系。                                                                                     |
| 占用                 | occupancy                              | —                                  | 实体当前物理占据道路、停车位或冲突资源的事实。                                                                                   |
| 候选                 | candidate                              | —                                  | 尚未通过全部约束、验证或裁决的可能结果。                                                                                         |
| 布局                 | layout                                 | —                                  | 表、字段、偏移量、对齐和节的确定性字节组织方式。                                                                                 |
| 版本轴               | version axis                           | —                                  | 只描述一个独立兼容维度、不能与其他维度混用的版本字段。                                                                           |
| 构建与目标选择器     | build and target selector              | —                                  | 选择构建工具、目标平台或封闭配置档的字段；不是协议版本。                                                                         |
| 封闭种类选择器       | closed kind selector                   | —                                  | 选择版本化封套的封闭种类及必需字段集合；不是协议版本。                                                                           |
| 内容身份与长度绑定   | content identity and length binding    | —                                  | 以摘要和精确字节长度共同认证目标对象、或以语义修订标识认证规范内容的外部绑定字段。                                               |
| 镜像头               | image header                           | header                             | 声明魔数、布局、目标、配置档和来源沿袭的镜像起始结构。                                                                           |
| 魔数                 | magic                                  | `"LFID"` 等                        | 用固定字节区分协议或制品类型的标记。                                                                                             |
| 节目录               | section directory                      | —                                  | 记录每个镜像节的种类、偏移量、长度和对齐信息的目录。                                                                             |
| 不可变               | immutable                              | —                                  | 构造后不再改变，可由多个世界/会话安全共享。                                                                                      |
| 可变                 | mutable                                | —                                  | 随运行时命令、固定步进或生命周期改变，必须由明确权威职责拥有。                                                                   |
| 精确当前版本         | exact-current                          | —                                  | 只接受当前冻结版本，不在生产路径隐式兼容历史或未来版本。                                                                         |
| 失败关闭             | fail closed                            | —                                  | 发生歧义、缺失、越界、验证失败或外部状态不可确认时拒绝继续。                                                                     |
| 运行时前置条件       | runtime precondition                   | —                                  | 建立视图或世界前必须由结构校验器证明成立的不变量。                                                                               |
| 特性位               | feature bits                           | —                                  | 可任意组合的功能位；#291 镜像配置明确拒绝以其替代封闭配置档。                                                                    |
| 干净编译             | clean compile                          | —                                  | 不使用增量缓存、从完整来源重新执行的编译。                                                                                       |
| 增量编译             | incremental compile                    | —                                  | 复用未失效中间结果、但必须与干净编译产生相同规范结果的编译。                                                                     |
| 并行编译             | parallel compile                       | —                                  | 并发执行编译遍、并按确定性合并顺序产生结果的编译。                                                                               |
| 确定性               | determinism                            | —                                  | 相同规范输入在冻结条件下产生相同语义、顺序和要求范围内相同字节。                                                                 |
| 行为等价             | behavior equivalence                   | —                                  | 当前态与目标态在冻结场景中产生相同交通状态、事件和安全结果。                                                                     |
| 确定性状态摘要       | deterministic state digest             | —                                  | 对规范化已提交状态计算、用于回放校验和定位首个分歧 tick 的稳定摘要。                                                             |
| 失同步诊断制品       | desynchronization diagnostic artifact  | —                                  | 结合冷标识与源映射记录首个分歧 tick、phase、实体和资源组件的调试制品。                                                           |
| 已知向量             | known vector                           | —                                  | 冻结输入及其精确字节/标识/摘要预期值的跨实现测试向量。                                                                           |
| 属性测试             | property testing                       | —                                  | 生成满足约束的输入并检查不变量、往返或关系性质，而不是只断言少量固定样例的测试。                                                 |
| 编译失败测试         | compile-fail test                      | —                                  | 以预期无法通过类型检查的调用证明非法公共接口组合在编译期不可表达的测试。                                                         |
| 变形测试             | metamorphic test                       | —                                  | 通过不应改变结果的输入变换验证稳定性契约的测试。                                                                                 |
| 模糊测试             | fuzz testing                           | fuzz                               | 以大量生成或变异输入检查崩溃、越界和失败关闭行为。                                                                               |
| 差分测试             | differential testing                   | —                                  | 对相同输入比较两个独立实现的语义、诊断、标识或字节结果。                                                                         |
| 语义验证             | semantic validation                    | —                                  | 检查来源、实体、关系和领域规则是否满足规范语义的验证。                                                                           |
| 结构验证             | structural verification                | —                                  | 在不重跑完整领域语义的前提下，检查镜像布局、边界和运行时前置条件。                                                               |
| 字节完全一致         | byte-for-byte equality                 | exact bytes                        | 两个目标对象的完整字节序列逐字节相同。                                                                                           |
| 字节序               | byte order / endianness                | —                                  | 多字节整数在制品中的排列规则。                                                                                                   |
| 对齐                 | alignment                              | —                                  | 数据地址或偏移量必须满足的倍数约束。                                                                                             |
| 偏移量               | offset                                 | —                                  | 从容器或节起点到目标数据的字节距离。                                                                                             |
| 基数                 | cardinality                            | —                                  | 表、节、实体、点或关系集合的记录数量。                                                                                           |

### 5.1 性能硬件角色

| 中文规范术语         | 英文辅助名（English Alias）                     | 精确标识符 / 缩写        | 中文规范含义                                                                                                                                                                                                         |
| -------------------- | ----------------------------------------------- | ------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 目标产品推荐参考机型 | target product recommended reference machine    | `P100`                   | 由产品负责人选定、用于目标产品性能预算与最终认证的具名物理硬件；它由参考机资产标识和硬件身份指纹共同绑定，不是只凭系列或配置继承的机型类别。它不是最低产品机，选定该角色不等于产品通过，等价或替代机型仍须单独验证。 |
| 参考机资产标识       | reference-machine asset identifier              | `referenceMachineId`     | 仓库为一台选定物理参考机分配的持久逻辑标识；设备更换不得沿用，硬件维修是否保留须由硬件身份指纹和变更记录共同裁决。                                                                                                   |
| 参考机硬件身份指纹   | reference-machine hardware identity fingerprint | `hardwareIdentitySha256` | 按版本化规范从本机 SMBIOS 身份字段计算的 SHA-256；用于确认正式运行是否发生在同一物理参考机，不公开原始序列号，也不单独证明运行环境或产品通过。                                                                       |

## 6. 制品、镜像、信任与验证

| 中文规范术语       | 英文辅助名（English Alias）     | 精确标识符 / 缩写              | 中文规范含义                                                                                                                                                                         |
| ------------------ | ------------------------------- | ------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| 可移植规范制品     | portable canonical artifact     | —                              | 平台无关、确定性、可发布、可长期审计并可供独立验证器读取的静态路网制品。                                                                                                             |
| 修订—制品绑定      | revision–artifact binding       | `RevisionArtifactBindingV1`    | 以路网修订派生版本、路网修订标识、目标制品精确字节摘要和精确字节长度四项共同标识一份制品；只比较 revision 不能证明 exact bytes 相同，只比较 digest/length 不能证明语义修订声明正确。 |
| 格式硬上限         | format hard limit               | —                              | 由格式版本冻结、任何读取器都不得提高或禁用的安全天花板；用于在 hash、分配和建立受检视图前限制对象、计数、字符串、向量和累计暂存规模，不是性能目标。                                  |
| 调用方运行上限     | caller runtime limit            | —                              | 调用方针对一次读取或发射显式提供、可以低于但不得高于格式硬上限的资源预算；改变它不改变 wire version，也不授权扩大格式安全边界。                                                      |
| 目标静态镜像       | target static image             | `StaticNetworkImage`           | 按目标、布局、封闭配置档与分区提示版本生成的可重建运行时性能制品。                                                                                                                   |
| 静态镜像           | static image                    | —                              | 上下文已明确时对目标静态镜像的允许简称；不表示另一种制品或可原地修改的运行时对象。                                                                                                   |
| 分节容器           | sectioned container             | —                              | 通过头部和节目录组织多个有界数据节的镜像容器。                                                                                                                                       |
| 静态交通镜像节     | static traffic image section    | `StaticTrafficImage`           | 所有配置档都必须包含的交通静态表。                                                                                                                                                   |
| 静态空间镜像节     | static spatial image section    | `StaticSpatialImage`           | 仅空间配置档包含的几何和采样静态表。                                                                                                                                                 |
| 稳定身份索引       | static identity index           | `StaticIdentityIndex`          | 所有生产配置档都必须包含、在稳定标识与镜像局部 ordinal 之间双向映射的冷数据索引；它不证明跨修订语义兼容。                                                                            |
| 镜像字节长度       | static image byte length        | `staticImageByteLength`        | 外部可信描述符认证的原始未压缩镜像 exact bytes 长度；必须在读取、解压、分配和摘要前用于有界预检。                                                                                    |
| 完整性分块         | integrity chunk                 | chunk                          | 按认证方案从镜像精确字节切出的连续区间；其摘要只在加载或首次挂载边界验证。                                                                                                           |
| 平面分块表         | flat chunk table                | —                              | 由外部描述符认证、按偏移量顺序列出全部完整性分块及摘要，不依赖递归树节点的版本化结构。                                                                                               |
| 默克尔树           | Merkle tree                     | —                              | 通过分层摘要路径证明局部内容属于受认证根的结构；v1 静态镜像完整性方案不采用。                                                                                                        |
| 静态镜像完整性清单 | static image integrity manifest | `StaticImageIntegrityManifest` | 位于镜像外、由描述符认证，以连续分块摘要和节覆盖证明目标镜像字节完整性的版本化清单。                                                                                                 |
| 完整审计           | full audit                      | —                              | 显式读取并核对完整镜像摘要的发布、重建或运维审计路径；不是生产启动建立局部可信视图的默认前置。                                                                                       |
| 预先验证           | eager verification              | —                              | 在创建任何目标视图前主动完成全部所需分块与结构检查的加载策略。                                                                                                                       |
| 延迟验证           | lazy verification               | —                              | 只在首次请求某节或能力时才完成对应分块与结构检查的加载策略。                                                                                                                         |
| 后台验证           | background verification         | —                              | 在不暴露未验证字节的前提下，由后台任务提前完成尚未请求节检查的加载策略。                                                                                                             |
| 不可变字节背板     | immutable byte backing          | —                              | 从分块验证完成到有类型视图释放期间保持同一精确字节、且不存在可写别名的资产或拥有存储。                                                                                               |
| 静态交通视图       | static traffic view             | `StaticTrafficView`            | 从可信静态镜像拆出的只读交通视图。                                                                                                                                                   |
| 静态空间视图       | static spatial view             | `StaticSpatialView`            | 从可信且含空间节的静态镜像拆出的只读空间视图。                                                                                                                                       |
| 镜像配置档         | image profile                   | `staticImageProfileId`         | 冻结镜像节集合和用途的版本化配置。                                                                                                                                                   |
| 封闭配置档         | closed profile                  | —                              | 只能选择登记值、不能由调用方任意拼特性位的配置档。                                                                                                                                   |
| 无图形配置         | headless profile                | `traffic-headless-v1`          | 不携带空间几何、面向服务器和无图形宿主的交通配置档。                                                                                                                                 |
| 镜像节掩码         | image section mask              | `sectionMask`                  | 必须与所选封闭配置档的节集合精确一致的位集合。                                                                                                                                       |
| 静态镜像描述符     | static image descriptor         | `StaticImageDescriptor`        | 位于镜像字节外、绑定路网修订标识、规范制品/镜像/完整性清单/收据各自摘要与精确长度、版本、目标、配置档和工具的值。                                                                    |
| 可信描述符         | trusted descriptor              | —                              | 经认证且与路网修订标识、每个目标对象的摘要和精确长度、版本、目标平台、配置档及验证收据绑定的外部描述符。                                                                             |
| 验证收据（简称）   | validation receipt              | —                              | “验证收据封套”的行文简称，不引入第二种概念、线格式或精确类型；规范类型仍只使用 `ValidationReceiptEnvelope`。                                                                         |
| 发布清单           | publication manifest            | —                              | 对制品集合、外部描述符、摘要和真实性进行发布级绑定的清单。                                                                                                                           |
| 信任锚             | trust anchor                    | —                              | 位于待验证对象之外、由签名、认证资产链或固定摘要提供的可信依据。                                                                                                                     |
| 未验证镜像字节     | unverified image bytes          | `UnverifiedImageBytes`         | 尚未完成结构与外部信任绑定的任意输入字节。                                                                                                                                           |
| 已结构验证镜像     | structurally verified image     | `StructurallyVerifiedImage`    | 目标节的有界结构检查通过，只证明内存安全和运行时前置条件成立、尚未证明发布来源或内容绑定可信的镜像。                                                                                 |
| 已结构验证规范制品 | checked canonical artifact      | `CheckedPortableArtifactView`  | 已接受（#298 G1；未实现），通过有界结构检查、尚未证明语义或来源可信的可移植规范制品只读视图。                                                                                        |
| 规范发布候选       | canonical publication candidate | `PortablePublicationCandidate` | 已接受（#298 G1；未实现），同次成功编译原子产生的制品、源映射、差异与绑定；尚未独立验证或发布。                                                                                      |
| 可信静态镜像       | trusted static image            | `TrustedStaticImage`           | 与认证外部描述符、完整性清单及验证收据匹配，且只暴露已完成分块/结构验证目标节的能力对象。                                                                                            |
| 已验证规范制品视图 | validated artifact view         | —                              | 独立验证器完成语义和标识重算后建立、供后续独立重建消费的只读能力；不是最终收据。                                                                                                     |
| 编译器语义验证     | compiler semantic validation    | —                              | 编译器对来源和 IR 执行的主语义检查。                                                                                                                                                 |
| 独立验证器         | independent validator           | `laneflow-validator`           | 不复用编译器语义实现，独立检查可移植规范制品、路网修订标识和语义差异的验证器。                                                                                                       |
| 独立镜像重建器     | independent image builder       | —                              | 不复用编译发射器的布局填充实现，从已验证制品重建镜像的独立实现。                                                                                                                     |
| 有界结构校验器     | bounded structural verifier     | —                              | 对不可信镜像字节执行偏移量、区间、数值、基数和资源上限检查的校验器。                                                                                                                 |
| 精确字节摘要       | exact-bytes digest              | —                              | 对目标对象完整字节序列计算的 SHA-256；目标对象不得嵌入自身摘要。                                                                                                                     |
| 内存映射           | memory mapping                  | mmap                           | 让文件字节映射到地址空间、供只读视图按偏移量访问的加载方式。                                                                                                                         |
| 应用二进制接口     | application binary interface    | ABI                            | 镜像布局、对齐、字节序和调用边界共同形成的二进制兼容契约。                                                                                                                           |
| 目标三元组         | target triple                   | `targetTriple`                 | 冻结 CPU 架构、供应商、操作系统和 ABI 环境的目标标识。                                                                                                                               |

## 7. 运行时、空间层与适配器

| 中文规范术语        | 英文辅助名（English Alias）         | 精确标识符 / 缩写    | 中文规范含义                                                                                                                                    |
| ------------------- | ----------------------------------- | -------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------- |
| LaneFlow 交通运行时 | LaneFlow Traffic Runtime            | `laneflow-runtime`   | 目标态动态执行层；拥有固定步进、已实现执行域的交通参与单元、动态通行定义和每世界可变交通状态。                                                  |
| LaneFlow 核心       | LaneFlow Core                       | `laneflow-core`      | 生产切换前当前态动态执行层；目标态由交通运行时一次性不兼容替代。                                                                                |
| 交通世界            | traffic world                       | `TrafficWorld`       | 目标态共享静态交通视图并持有每世界可变状态的运行时实例。                                                                                        |
| 核心世界            | core world                          | `CoreWorld`          | 当前态 `laneflow-core` 的世界类型。                                                                                                             |
| 命令域              | command domain                      | —                    | 已接受（#381）：按领域命令与查询入口划分的 `CoreWorld` 实现组织单位；#381 拆分把 parking、route、vehicle、signal 与 tick 推进各自收敛为子模块。 |
| 固定步进            | fixed tick                          | tick                 | 以确定输入和顺序推进交通状态的一次运行时更新。                                                                                                  |
| 稳态步进            | steady-state tick                   | —                    | 初始化和容量稳定后的高频固定步进路径。                                                                                                          |
| 交通参与者          | traffic participant                 | —                    | 直接使用交通网络或占用交通空间的动态参与者；不等同于城市居民、乘客、出行需求或 AI Agent。                                                       |
| 交通参与单元        | traffic participant unit            | —                    | 交通运行时中可独立保留身份的计数原子；机动车、骑行者组合、行人或轨道运营编组的具体原子由对应执行域 G1 冻结。                                    |
| 交通执行域          | traffic execution domain            | —                    | 共享网络、运动/安全求解和生命周期契约的一类交通参与单元；与准入用 `ParticipantClass` 正交。                                                     |
| 道路机动车执行域    | road motor vehicle execution domain | `road_motor_vehicle` | 当前 Core 与既有车辆工作负载的报告域；该标记不是生产数据枚举，也不冻结其他执行域的类型体系。                                                    |
| 动态通行定义        | dynamic traversal definition        | —                    | 目标交通运行时中描述参与单元运行时通行路径及进度语义的通用概念；当前道路机动车执行域的具体投影是 `Route`。                                      |
| 停驻状态            | stationary state                    | —                    | 参与单元暂不沿通行路径运动、但仍保留身份和领域占用语义的运行时状态；当前道路机动车投影包括停车状态。                                            |
| 车辆运行单元        | vehicle runtime unit                | `VehicleState`       | 当前 Core 的车辆特化交通参与单元；不表示目标交通运行时只允许车辆。                                                                              |
| 车辆状态            | vehicle state                       | —                    | 当前车辆运行单元在某一固定步进边界上的运动、路线与行为状态。                                                                                    |
| 车道图              | lane graph                          | —                    | 供运行时遍历和路线跟随使用的有向车道拓扑。                                                                                                      |
| 路线                | route                               | `Route`              | 当前道路机动车执行域中由有序遍历边和相关出现项组成的车辆行驶计划；运行时注册形态见“动态路线”。                                                  |
| 动态路线            | dynamic route                       | `Route`              | “路线”的运行时注册特化，具有代际感知句柄且不获得持久 `StableId128`；两词共享当前代码标识符。                                                    |
| 跟车                | vehicle following                   | —                    | 根据同一路径上的前车状态施加安全间距和速度约束的运行时行为。                                                                                    |
| 信号                | signal                              | —                    | 由信号控制器和信号组产生、供车辆解释通行约束的运行时指示。                                                                                      |
| 路口规则            | intersection rules                  | —                    | 对冲突流、准入、授权、预约和通行顺序进行运行时裁决的规则集合。                                                                                  |
| 停车                | parking                             | —                    | 对停车区域、停车位、预约、进入、占用和离开进行管理的领域行为。                                                                                  |
| 每世界可变状态      | per-world mutable state             | —                    | 交通参与单元、动态通行定义、控制器时钟、预约、占用和缓冲区等不能进入共享镜像的状态；当前投影包括车辆、动态路线和停车。                          |
| 空间层              | Spatial layer                       | `laneflow-spatial`   | 拥有规范几何采样和位姿语义、但不拥有交通规则的引擎无关组件。                                                                                    |
| 引擎适配器          | engine adapter                      | Adapter              | 把运行时快照和空间位姿映射到宿主引擎生命周期与表现对象的组件。                                                                                  |
| 宿主变换            | host transform                      | Transform            | 宿主引擎中的位置、旋转和缩放表示；不得反写为交通进度权威。                                                                                      |
| 细节层次            | level of detail                     | LOD                  | 按距离、预算或表现需求选择渲染/调试细节的适配器策略。                                                                                           |
| 位姿批次            | pose batch                          | —                    | 空间层按稳定顺序批量产生的位置和朝向结果。                                                                                                      |
| 放置令牌            | placement token                     | —                    | 绑定规范坐标框架与宿主放置状态、防止过期位姿写入的令牌。                                                                                        |

### 7.1 规模计数状态边界

当前已接受的道路机动车车辆特化计数如下。在目标态运行时完成实现、基准迁移与生产
切换前，这些计数继续是 current 性能结果与工作负载的规范用语：

| 中文规范术语       | 英文辅助名（English Alias）       | 精确标识符 / 缩写  | 状态       | 中文规范含义                                                                   |
| ------------------ | --------------------------------- | ------------------ | ---------- | ------------------------------------------------------------------------------ |
| 个体车辆数         | individual vehicle count          | `N_individual`     | 当前已接受 | 当前 Core 中仍存在并保留完整身份、路线/进度、停车与生命周期状态的车辆数。      |
| 道路交通活动车辆数 | road-traffic active vehicle count | `N_traffic_active` | 当前已接受 | 当前处于道路交通系统、每个 Core 基础固定步进参与运动、安全或占用求解的车辆数。 |
| 意图更新车辆数     | intent-update vehicle count       | `N_intent`         | 当前已接受 | 当前固定步进实际重新计算昂贵控制意图的车辆数。                                 |
| 表现车辆数         | presented vehicle count           | `N_presented`      | 当前已接受 | 当前外层帧由适配器或表现层实例化、提取或提交的车辆数。                         |
| 聚合交通量         | aggregate traffic population      | `N_aggregate`      | 当前已接受 | 只以流、包或计数存在、没有完整逐车身份的交通量。                               |

以下六项是 **#291 已接受目标计数**。它们规范目标态运行时与后继通用规模报告，但
Accepted 不表示目标态实现已经存在，也不得据此改写 current 产品通过条件、性能
基准、历史证据或合并门禁：

| 中文规范术语       | 英文辅助名（English Alias）            | 精确标识符 / 缩写           | 状态         | 中文规范含义                                                                                    |
| ------------------ | -------------------------------------- | --------------------------- | ------------ | ----------------------------------------------------------------------------------------------- |
| 个体交通参与单元数 | individual traffic participant count   | `N_individual[d]`           | 目标态已接受 | 执行域 `d` 中仍存在并保留完整身份与生命周期状态的交通参与单元数。                               |
| 活动交通参与单元数 | active traffic participant count       | `N_active[d]`               | 目标态已接受 | 执行域 `d` 中当前参与该域运动、安全或占用求解的个体交通参与单元数。                             |
| 意图更新参与单元数 | intent-update participant count        | `N_intent[d]`               | 目标态已接受 | 当前固定步进在执行域 `d` 中实际重新计算昂贵行为或控制意图的个体交通参与单元数。                 |
| 表现交通参与单元数 | presented traffic participant count    | `N_presented[d]`            | 目标态已接受 | 当前外层帧在执行域 `d` 中由适配器或表现层按个体身份实例化、提取或提交的交通参与单元数。         |
| 聚合交通记录数     | aggregate traffic record count         | `N_aggregate_records[d]`    | 目标态已接受 | 执行域 `d` 中由运行时实际存储、调度或更新的聚合流、包、单元格等记录数，用于衡量计算与内存成本。 |
| 聚合等价参与单元数 | aggregate-equivalent participant count | `N_aggregate_equivalent[d]` | 目标态已接受 | 执行域 `d` 中聚合表示所代表的交通参与单元数，用于描述保真度与覆盖规模；不能替代聚合交通记录数。 |

以下词条与 Core 研究/测试仪器外移（#380）相关：

| 中文规范术语  | 英文辅助名（English Alias）    | 精确标识符 / 缩写 | 中文规范含义                                                                                                                                                 |
| ------------- | ------------------------------ | ----------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| 研究/测试仪器 | research/test instrumentation  | —                 | 已接受（#380），为研究取证或测试门禁目的嵌入或接入运行时执行路径的测量、观察与注入机制的总称；不是生产语义的一部分。                                         |
| 语义研究原型  | semantic research prototype    | —                 | 已接受（#380），以单线程生产执行为精确预言机、验证候选架构精确等价或在显式保真度预算内有界等价的非生产研究执行器；结论可登记为研究证据，不自动成为生产架构。 |
| 性能归因计时  | performance attribution timing | —                 | 已接受（#380），在生产步进路径的具名阶段边界采集、只用于机制归因的耗时测量；不构成产品延迟取证。                                                             |
| 故障注入      | fault injection                | —                 | 已接受（#380），在受检点强制返回受控失败以验证失败原子性等硬不变量的测试机制；注入检查点不得进入默认发布构建。                                               |
| 保留内存账本  | retained memory ledger         | —                 | 已接受（#380），按所有权穷尽枚举组件的保留内存核算结构；新增字段必须触发编译失败以强制分类。                                                                 |
| 仪器探针边界  | instrumentation probe boundary | —                 | 已接受（#380），生产路径持有空操作默认探针、研究态注入测量实现的显式接缝；默认发布构建必须与无仪器构建等价。                                                 |
| 测试支持接缝  | test-support seam              | —                 | 已接受（#380），以编译期门控向测试与研究暴露非生产能力的正式边界；`#[doc(hidden)]` 只控制文档可见性，不构成构建排除。                                        |

## 8. 静态路网领域标识符

下表给出标识 v1（Identity v1）及其候选扩展中精确类型名的中文语义。代码和制品继续使用精确
标识符，中文名称负责解释其规范含义。

| 中文规范术语 | 英文辅助名 / 代码标识符（English Alias / Code Identifier） | 中文规范含义                                                                                                    |
| ------------ | ---------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------- |
| 道路走廊     | `RoadCorridor`                                             | 组织方向性道路区段与非遍历设施带的横断面所有者。                                                                |
| 道路区段     | `RoadSection`                                              | 道路走廊内具有方向和横断面成员关系的区段。                                                                      |
| 编制车道     | `AuthoringLane`                                            | 来源模块中具有稳定键、可展开为道路车道边的车道声明。                                                            |
| 车道图边     | `LaneEdge`                                                 | 具有显式稳定边键、可独立寻址的基础遍历拓扑实体；道路车道覆盖与路口内部边归属只是关系 / 角色，不参与其稳定身份。 |
| 路口         | `Junction`                                                 | 组织通行流向和机动路径的静态路口声明。                                                                          |
| 通行流向     | `Movement`                                                 | 从有向入口接近臂到有向出口接近臂的静态通行意图；转向类别只是元数据。                                            |
| 机动路径     | `ManeuverPath`                                             | 某通行流向内连接入口边、内部边和出口边的可遍历路径。                                                            |
| 机动门       | `ManeuverGate`                                             | 绑定机动路径、用于空间准入和信号约束的静态门。                                                                  |
| 等待区       | `WaitingZone`                                              | 绑定机动路径并表达等待容量、顺序或位置的静态区域。                                                              |
| 停止线       | `StopLine`                                                 | 表达车辆必须在其前满足通行约束的静态线。                                                                        |
| 信号组       | `SignalGroup`                                              | 面向一组门或通行意图输出信号指示的静态组。                                                                      |
| 信号控制器   | `SignalController`                                         | 产生信号相位和指示时间序列的静态控制程序。                                                                      |
| 信号相位     | `SignalPhase`                                              | 信号控制器内具有稳定键的阶段声明。                                                                              |
| 停车区域     | `ParkingArea`                                              | 组织停车位的静态区域。                                                                                          |
| 停车位       | `ParkingSpace`                                             | 可被预约和占用的静态位置；可选归属于停车区域，该组织关系不构成停车位身份父锚点。                                |
| 车道组       | `LaneGroup`                                                | 道路区段内组织车道成员的静态分组。                                                                              |
| 设施带       | `FacilityBand`                                             | 道路走廊内不直接承担机动车遍历拓扑的设施横带。                                                                  |
| 参与者类别   | `ParticipantClass`                                         | 数据声明、可继承的准入分类；不定义交通执行域，也不证明对应运行时行为已经实现。                                  |
| 准入规则     | `AccessRule`                                               | 对参与者与目标施加允许、拒绝或约束效果的静态规则。                                                              |
| 车辆配置     | `VehicleProfile`                                           | 冻结车辆运动与安全参数的静态配置。                                                                              |
| 静态路线     | `StaticRoute`                                              | 编译期来源中声明并预编译的路线。                                                                                |
| 规范坐标框架 | `CanonicalFrame`                                           | 空间几何和位姿共享的稳定局部坐标框架。                                                                          |
| 冲突区       | `ConflictZone`                                             | 多个参与者流可能发生空间冲突、需要运行时裁决的区域。                                                            |
| 参与者流     | `ParticipantStream`                                        | 进入冲突裁决的有向参与者流。                                                                                    |
| 路口组       | `JunctionGroup`                                            | 组织多个路口的更高层静态组合；尚未进入标识 v1（Identity v1）。                                                  |
