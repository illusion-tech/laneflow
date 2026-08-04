# laneflow-compiler-test-support

`laneflow-compiler-test-support` 是 #292 迁移验证使用的集成专用桥
（Integration-only Bridge）。它只把 `ValidatedCanonicalLir` 投影为当前态
`InitialTrafficData`、可选 `SpatialRegistry` 和稳定映射报告。

本包不是生产后端或第三方扩展接口，不读取当前 `JSON`，也不重新定义编译器语义。
投影后的当前态构造器仍执行自身防御校验。包保持 `publish = false`，并将在 #294 完成
生产切换后删除。

包内集成测试以独立测试 frontend 读取 G1 冻结制品，覆盖完整当前静态快照、空间采样、
固定步进行为与事件、重复编译确定性，以及多机动门等待区出现项。测试可以读取当前
`JSON` 作为迁移预言机；生产库中的 `project()` 不读取该输入，也不会借其补齐 LIR。
