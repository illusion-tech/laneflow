# laneflow-compiler-test-support

`laneflow-compiler-test-support` 是 #292 迁移验证使用的集成专用桥
（Integration-only Bridge）。它只把 `ValidatedCanonicalLir` 投影为当前态
`InitialTrafficData`、可选 `SpatialRegistry` 和稳定映射报告。

本包不是生产后端或第三方扩展接口，不读取当前 `JSON`，也不重新定义编译器语义。
投影后的当前态构造器仍执行自身防御校验；桥还会独立重建并核对所有稳定标识前像、信号
周期、参与者分类闭包、路线正向/反向出现项表，以及空间段长、累计弧长和局部基，避免
current 构造器重算派生数据后掩盖 LIR 错误。包保持 `publish = false`，并将在 #294
完成生产切换后删除。

包内集成测试以独立测试 frontend 读取 G1 冻结制品，覆盖完整当前静态快照、空间采样、
固定步进行为与事件、重复编译确定性，以及多机动门等待区出现项。测试可以读取当前
`JSON` 作为迁移预言机；生产库中的 `project()` 不读取该输入，也不会借其补齐 LIR。

同一集成测试文件还提供默认忽略的 `p100_production_compiler_baseline`。它把完整
信号化走廊复制为独立命名空间，只计时 `Compiler::compile`，并在停表后输出五级紧凑
生产基线；普通 workspace test 不运行墙钟测量。精确命令、环境和结果见
`docs/reference/v0.10-compiler-production-baseline.md`。
