# LFSD 6 完整策略成员差异

沿用 `lfca-policy-references` 的工程策略拓扑，target 分别修改 evidence locator、
gap 数值、stream priority 和 gate prohibition；四个保留 K 各产生一次完整 Modify。
`expected.lfsd` 冻结全部四种成员值、StableRefV1 和稳定引用集合。

差异的规范长度和 SHA-256 对象键见同目录的 `bindings.txt`。
base/target 是 writer 工程 fixture，未经过正式策略前端或共享静态路网规则覆盖验证，不代表
法规语义，也未配套 LFSM，不能作为发布候选。

设 `DUMP_PORTABLE=1` 后运行 `policy_diff_fixed_bytes_cover_all_four_complete_member_values`
测试可重建。修改锚点前须从两份修订的稳定身份和字段独立重建规范成员值，不能只重算摘要。
