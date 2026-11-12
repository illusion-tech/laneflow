# Core Runtime

**文档状态**: Retired
**最后更新**: 2026-11-12
**适用范围**: 把读者带到现行交通世界

可运行交通世界是 `laneflow-runtime` / `TrafficWorld`，安装
`Arc<SharedNetworkRevision>`。

tick、确定性与失败原子性见
[`../adr/0003-runtime-tick-and-determinism.md`][adr-0003]。
消费路径见 `traffic-runtime-shared-consumption.md`。身份与句柄见
`core-id-handles.md`。车辆、路线、信号、停车等行为规则由对应领域设计约束。

[adr-0003]: ../adr/0003-runtime-tick-and-determinism.md
