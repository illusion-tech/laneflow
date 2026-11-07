# Core Runtime 可扩展性前置审计

**文档状态**: Retired<br>
**最后更新**: 2026-11-07<br>
**关联议题**: #72、#199、#204、#207、#210、#212

#199 是 #72 的 Core API 可扩展性前置审计，已于 2026-07-22 关闭。审计对象
`laneflow-core` / `CoreWorld` 已随 #301 拆除；#204、#207、#210、#212 的私有原型研究
（阶段与事件合并、分区占用、选择性快照与批处理、降频语义）随之成为历史证据。

审计正文、候选方案、原型结果与可扩展性约束 C1–C13 只存在于 git 历史，不再作为实现
输入。查阅当时全文使用提交
[`caf1c90f581a7731b795a32c594f4977f51e71b2`](https://github.com/illusion-tech/laneflow/blob/caf1c90f581a7731b795a32c594f4977f51e71b2/docs/design/core-runtime-scalability-audit.md)。

仍有效的结论：

- 城市级单世界交通执行是 ADR 0021 保留的规模演进方向；多世界吞吐不能替代单个大型
  世界的扩展证据。生产分区与归约须由独立研究、证据与 G1/ADR 支持。
- 历史阈值与单机数字不能替代
  [`core-runtime-performance-baseline.md`](core-runtime-performance-baseline.md)
  要求的集成认证。
- 原约束中仍适用于现行路径的部分已由对应长期文档承接：tick、确定性与失败原子性见
  [`../adr/0003-runtime-tick-and-determinism.md`](../adr/0003-runtime-tick-and-determinism.md)；
  句柄不透明、无公开排序见
  [`../adr/0005-core-identity-and-handle-model.md`](../adr/0005-core-identity-and-handle-model.md)
  与 `core-id-handles.md`；静态执行约束与每世界执行计划分层见 ADR 0020/0021；路网修订、
  快照与切换见 ADR 0024/0025 与 `traffic-runtime-shared-consumption.md`。
