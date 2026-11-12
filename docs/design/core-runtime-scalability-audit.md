# Core Runtime 可扩展性前置审计

**文档状态**: Retired<br>
**最后更新**: 2026-11-07<br>
**关联议题**: #72、#199、#204、#207、#210、#212

本页不是实现输入。城市规模的现行决策见
[`traffic-runtime-scale-decision-baseline.md`][scale-baseline]，产品测量协议见
[`core-runtime-performance-baseline.md`][performance-baseline]。

当时全文在提交
[`caf1c90f581a7731b795a32c594f4977f51e71b2`][scalability-audit]，
因为那些原型结果不在当前工作区，而单世界扩展结论仍需要能找到出处。

仍有效的结论：

- 城市级单世界交通执行是 ADR 0021 保留的规模演进方向；多世界吞吐不能替代单个大型
  世界的扩展证据。生产分区与归约须由独立研究、证据与 G1/ADR 支持。
- 历史阈值与单机数字不能替代
  [`core-runtime-performance-baseline.md`][performance-baseline]
  要求的集成认证。
- 原约束中仍适用于现行路径的部分已由对应长期文档承接：tick、确定性与失败原子性见
  [`../adr/0003-runtime-tick-and-determinism.md`][adr-0003]；
  句柄不透明、无公开排序见
  [`../adr/0005-core-identity-and-handle-model.md`][adr-0005]
  与 `core-id-handles.md`；静态执行约束与每世界执行计划分层见 ADR 0020/0021；路网修订、
  快照与切换见 ADR 0024/0025 与 `traffic-runtime-shared-consumption.md`。

[scalability-audit]: https://github.com/illusion-tech/laneflow/blob/caf1c90f581a7731b795a32c594f4977f51e71b2/docs/design/core-runtime-scalability-audit.md
[performance-baseline]: core-runtime-performance-baseline.md
[scale-baseline]: traffic-runtime-scale-decision-baseline.md
[adr-0003]: ../adr/0003-runtime-tick-and-determinism.md
[adr-0005]: ../adr/0005-core-identity-and-handle-model.md
