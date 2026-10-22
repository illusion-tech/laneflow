# LFCA 5 策略引用夹具

在既有 full-spatial 拓扑上，通过字段 writer 添加两份工程测试策略、一项依据、一项
间隙参数、通行流规则与门规则。策略身份按 Identity v1 派生，新增行使用 LFCA 5
登记，NetworkRevision 和 chunk/object 摘要从最终字节重算。

本夹具冻结字段与引用闭合。它不来自正式策略前端，不验证运行时规则覆盖、灯型解释
或法律语义，也未配套 LFSM/LFSD；不能作为发布候选。既有拓扑来源只作测试底座。

字段值包含同名 evidence/gap、跨实体稳定引用集合和完整 `u64` 间隙范围。
测试同时构造悬空 owner/目标、跨 owner key、重复及乱序集合、来源继承和跨 chunk 反例。

可在进程环境设置 `DUMP_PORTABLE=1` 后运行
`cargo +1.98.0 test --locked -p laneflow-compiler --lib policy_reference_fixture_matches_frozen_bytes`
重建，再取消该环境变量运行测试。更新固定锚点前须独立复核字段登记、摘要和引用顺序。
