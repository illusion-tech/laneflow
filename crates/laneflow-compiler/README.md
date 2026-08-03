# laneflow_compiler

LaneFlow 编译器拥有的静态路网（Compiler-owned Static Network）的生产编译器。

本 crate 最终负责受检官方来源构造、确定性编译、结构化诊断、资源限制，以及只读的
已验证规范低层中间表示（Validated Canonical Low-level Intermediate
Representation，Canonical LIR）。当前 G2 切片已建立首个生产编译资源上限配置档
`LF-COMP-P100-INITIAL-v1`，以及受检来源模块头和结构化诊断基础；规范来源记录、
编译遍和已验证输出仍未实现。

公共静态值契约来自 `laneflow-static-contract`。本 crate 不依赖当前核心、空间层、
数据加载器或引擎适配器，也不提前冻结可移植制品、静态镜像或第三方前端插件接口。
