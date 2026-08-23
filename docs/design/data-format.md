# Data Format

**文档状态**: Retired  
**最后更新**: 2026-08-23  
**适用范围**: 说明 current JSON 不再是生产数据契约

Traffic v0.10、SpatialPackage v0.1 与 ScenarioManifest v0.1 JSON 曾是仓库内部
wire 格式。项目从未发布这些 JSON；#301 已删除对应 schema 与加载 crate。

当前静态交通数据由编译器发射可移植规范制品（LFCA），再由
`laneflow-static-network` 构建 `SharedNetworkRevision`。可运行世界由
`TrafficWorld::install` 安装共享根，不从 JSON 创建。

道路编辑生产来源是未发布的 FlatBuffers B1，见
`road-editing-source-and-geometry-frontend.md`。走廊生成器仍可能写出同名 JSON
字节，那不是 authoring 契约，也不是 Runtime 入口。

历史字段级 wire 语义只存在于 git 历史。现行领域规则见 `lane-graph.md`、
`route-system.md`、`signal-system.md`、`parking-system.md`、
`road-junction-model.md`、`portable-canonical-artifact.md` 与
`shared-static-network.md`。ADR 0007 / 0008 / 0011 保留当时的决策原因。
