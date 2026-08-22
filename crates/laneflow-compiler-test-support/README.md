# laneflow-compiler-test-support

`laneflow-compiler-test-support` 是 #292 迁移验证使用的集成专用桥
（Integration-only Bridge）。它只把 `ValidatedCanonicalLir` 投影为当前态
`InitialTrafficData`、可选 `SpatialRegistry` 和稳定映射报告。

本包不是生产后端或第三方扩展接口，不读取当前 `JSON`，也不重新定义编译器语义。
投影后的当前态构造器仍执行自身防御校验；桥还会独立重建并核对所有稳定标识前像、信号
周期、参与者分类闭包、横断面 / 路口 / 控制 / 等待区 / 停车的双向 owner-member
索引、路线正向 / 反向出现项表，以及空间段长、累计弧长和局部基，避免 current 构造器
重算派生数据后掩盖 LIR 错误。包保持 `publish = false`，并将在 #301 拆除 current 运行时入口后
删除。

包内集成测试直接构造编译器原生的有类型模块，以两条可区分所有权链覆盖完整代表性
静态契约、空间采样、多机动门与等待区出现项，以及重复投影的有类型语义确定性。测试
不读取 current JSON，也不维护 current JSON 到编译器输入的转换器；旧 JSON loader 的
解析测试留在 `laneflow-data`。

历史 `LF-COMP-P100-PRODUCTION-R0-v1` 基线记录保留在
`docs/reference/v0.10-compiler-production-baseline.md`，但它不再作为可重放测试入口，
后续编译器基准应使用编译器原生 workload。
