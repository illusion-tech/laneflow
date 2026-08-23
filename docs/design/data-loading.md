# Data Loading

**文档状态**: Retired  
**最后更新**: 2026-08-23  
**适用范围**: 说明 JSON loader 与 `laneflow-data` 已拆除

`laneflow-data` 与 `laneflow-current-source` 已随 #301 删除。仓库不再提供
current JSON production loader，也不再从 JSON 构造 `CoreWorld`。

当前加载路径：

```text
受检 LFCA -> laneflow-format capability
         -> laneflow-static-network -> SharedNetworkRevision
         -> TrafficWorld::install(Arc)
         -> 可选 SpatialSession::bind(同一 Arc)
```

Runtime 不解析 JSON、LFCA 或 compiler-private LIR。发布加载与玩家确认建造的
admission 边界见 ADR 0024、ADR 0025、`compiler-post-emission-check-and-minimal-publication-closure.md`
与 `shared-static-network.md`。

历史 loader 边界只存在于 git 历史。ADR 0007 保留当时 crate 分层的决策原因。
