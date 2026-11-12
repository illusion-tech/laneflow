# Data Loading

**文档状态**: Retired
**最后更新**: 2026-11-12
**适用范围**: 把读者带到现行加载路径

本页不是加载器契约。现行路径：

```text
受检 LFCA -> laneflow-format capability
         -> laneflow-static-network -> SharedNetworkRevision
         -> TrafficWorld::install(Arc)
         -> 可选 SpatialSession::bind(同一 Arc)
```

Runtime 不解析 LFCA 或 compiler-private LIR。发布加载与玩家确认建造的
admission 边界见 ADR 0024、ADR 0025、
`compiler-post-emission-check-and-minimal-publication-closure.md`
与 `shared-static-network.md`。
