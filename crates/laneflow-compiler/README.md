# laneflow_compiler

LaneFlow 编译器拥有的静态路网（Compiler-owned Static Network）的生产编译器。

本 crate 最终负责受检官方来源构造、确定性编译、结构化诊断、资源限制，以及只读的
已验证规范低层中间表示（Validated Canonical Low-level Intermediate
Representation，Canonical LIR）。当前 G2 切片已建立首个生产编译资源上限配置档
`LF-COMP-P100-INITIAL-v1`、受检来源模块头、结构化诊断，以及可派生 SHA-256 内容
摘要的确定性 `LFSOURCE` 来源记录和显式导入图。官方合成领域专用语言（Synthetic
DSL）已接入车道图边、完整横断面所有者树，以及由 `Junction`、`Movement` 和
`ManeuverPath` 构成的路口拓扑闭包。构建器原子拒绝非法数值 / token、非法或未导入
引用、重复声明和重复无序关系，并保留横断面、车道、覆盖链与完整机动路径的领域顺序。
包内标识 v1 编码器严格校验登记字段，
流式形成规范字节并派生 BLAKE3-128 `StableId128`；修订 1 的 22 种实体均由
`tests/identity-v1-known-vectors.txt` 冻结已知向量，并由独立字节组装预言机校验。
纵向管线使用有类型 `u32` 区块键完成跨模块符号解析、车道覆盖与机动路径连通性、
横断面唯一所有者，以及路口内部边排他角色校验，并按父项先于子项派生横断面和路口
实体身份；完整身份前像只在 HIR 临时登记表中用于重复 / 摘要碰撞判断。Canonical LIR
再按各实体的完整 Identity v1 前像字节分配有类型
逻辑序号，把连接和父子关系全部改写为同一 LIR 实例的有类型序号，并以共享字段字节池
保存后继制品所需的完整身份字段；来源位置不进入 LIR 语义摘要。公共 `Compiler` 已将该
当前领域子集接入原子成功闭环：任一阶段失败时只返回结构化诊断，成功时同时返回只读
`ValidatedCanonicalLir`、与其配对的 `ValidatedSourceMapInput` 和非错误级诊断。
`sourceDocumentKey` 在整个编译单元内唯一，来源记录在 AST/HIR/MIR 释放前按 LIR
稳定实体与 owner-local 关系冻结；机动门 / 等待区、信号、停车、准入、路线、空间等
其余首批领域声明，以及后继编译遍和制品发射仍未实现。

公共静态值契约来自 `laneflow-static-contract`。本 crate 不依赖当前核心、空间层、
数据加载器或引擎适配器，也不提前冻结可移植制品、静态镜像或第三方前端插件接口。
