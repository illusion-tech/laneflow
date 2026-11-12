# 当前 JSON 导入边界

**文档状态**: Retired
**最后更新**: 2026-11-12

本页只约束 Traffic v0.10、SpatialPackage v0.1 与 ScenarioManifest v0.1 这一组 JSON。

- 这组 JSON 从未作为外部资产发布。
- 编译器不为这组 JSON 提供导入前端。
- 不提供把这组 JSON 批量迁入现行制品的工具。
- 不提供这组 JSON 的长期离线兼容入口。

道路编辑 FlatBuffers、catalog、LFCA / LFSM / LFSD / LFCP，以及其他现行 JSON
交换，不在这组之内。本边界不取消 [ADR 0020][adr-0020] 保留的通用外部来源导入前端，
也不取消规范制品自身的版本迁移。

可运行世界从 `SharedNetworkRevision` 安装。见
[共享静态路网][shared-static-network] 与
[共享消费][shared-consumption]。

[adr-0020]: ../adr/0020-compiler-owned-static-network-and-static-image.md
[shared-static-network]: shared-static-network.md
[shared-consumption]: traffic-runtime-shared-consumption.md
