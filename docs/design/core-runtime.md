# Core Runtime

**文档状态**: Retired  
**最后更新**: 2026-08-23  
**适用范围**: 说明 v0.1 `CoreWorld` 运行时基线已被替换

`laneflow-core` / `CoreWorld` 已随 #301 拆除。可运行交通世界是
`laneflow-runtime` / `TrafficWorld`，安装 `Arc<SharedNetworkRevision>`。

tick、确定性与失败原子性的长期决策仍以
[`../adr/0003-runtime-tick-and-determinism.md`](../adr/0003-runtime-tick-and-determinism.md)
为准。现行消费路径见 `traffic-runtime-shared-consumption.md`。身份与句柄见
`core-id-handles.md`。车辆、路线、信号、停车等行为规则仍由对应领域设计约束。

v0.1 最小 runtime 正文只存在于 git 历史，不再作为实现输入。
