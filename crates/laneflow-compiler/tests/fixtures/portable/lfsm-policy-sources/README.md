# 路权来源固定字节样例

LFCA 8 与 LFSM 9 的工程格式样例，来源模块从受检 full-spatial 编译输入取得。
测试在包内来源冻结接缝提供两个策略（第二个没有局部成员）、四类成员、两个同类
evidence、嵌套法规字段和一个 Movement 方向；同名 key 跨成员种类合法。
RoadEditing canvas 使用显式空字符串，Text、canvas 缺失/非空及 65,537 成员的
跨 chunk 情形由相邻测试覆盖。

两个制品的长度与 SHA-256 见同目录的 `bindings.txt`。固定断言同时核对两个对象的
完整 bytes。独立字节解析按 LFCA 成员字段、owner 和局部 key 重建 primary、贡献
路径及全局 location 引用集合，并验证每个 chunk 摘要和两对象绑定。

该格式接缝样例不作为法规有效性证据或可发布候选。两个正式前端的完整策略编译、
来源绑定和发布描述符由 `compiler::policy_tests` 验证；本样例不授予 Runtime 安装权限，
不包含日期或历法选择。

设置进程环境变量 `DUMP_PORTABLE=1` 后运行 compiler 测试
`policy_source_fixture_matches_frozen_wire` 可重建；环境变量只用于测试制品生成。
