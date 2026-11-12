# Data Format

**文档状态**: Retired
**最后更新**: 2026-11-12
**适用范围**: 把读者带到现行静态数据入口

本页不是生产数据契约。静态交通数据由编译器发射 LFCA，再由
`laneflow-static-network` 构建 `SharedNetworkRevision`，并由
`TrafficWorld::install` 安装。

道路编辑来源见 `road-editing-source-and-geometry-frontend.md`。领域规则见
`lane-graph.md`、`route-system.md`、`signal-system.md`、`parking-system.md`、
`road-junction-model.md`、`portable-canonical-artifact.md` 与
`shared-static-network.md`。
