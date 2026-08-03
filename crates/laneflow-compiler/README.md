# laneflow_compiler

LaneFlow 编译器拥有的静态路网（Compiler-owned Static Network）的生产编译器。

本 crate 最终负责受检官方来源构造、确定性编译、结构化诊断、资源限制，以及只读的
已验证规范低层中间表示（Validated Canonical Low-level Intermediate
Representation，Canonical LIR）。当前 G2 切片已建立首个生产编译资源上限配置档
`LF-COMP-P100-INITIAL-v1`、受检来源模块头、结构化诊断，以及可派生 SHA-256 内容
摘要的确定性 `LFSOURCE` 来源记录和显式导入图。官方合成领域专用语言（Synthetic
DSL）已加入首个受检领域声明：车道图边的显式稳定键、交通权威长度、基础道路限速
与有类型下游引用；构建器原子拒绝非法数值、非法或未导入引用、重复声明和重复连接，
并把无序连接规范化后写入版本化来源记录。包内标识 v1 编码器严格校验登记字段，
流式形成规范字节并派生 BLAKE3-128 `StableId128`；修订 1 的 22 种实体均由
`tests/identity-v1-known-vectors.txt` 冻结已知向量，并由独立字节组装预言机校验。
车道图边纵向管线使用有类型 `u32` 区块键完成符号表、
跨模块引用解析和规范 HIR/MIR 连续表，并从
`(authoringNamespaceId, laneEdgeKey)` 派生和传递 `LaneEdgeId`；完整身份前像只在 HIR
临时登记表中用于重复 / 摘要碰撞判断。该阶段尚未接入公共 `Compiler`。其余领域
声明、后继编译遍和已验证输出仍未实现。

公共静态值契约来自 `laneflow-static-contract`。本 crate 不依赖当前核心、空间层、
数据加载器或引擎适配器，也不提前冻结可移植制品、静态镜像或第三方前端插件接口。
