# 当前 JSON 退役

**文档状态**: Accepted  
**最后更新**: 2026-08-24  

**关联议题**: #297、#301

current JSON（Traffic v0.10 / SpatialPackage v0.1 / ScenarioManifest v0.1）
从未对外发布。#297 取消 compiler 导入前端；#301 删除 schema、`laneflow-data`
与 `laneflow-current-source`。

因此：

- current JSON 不进入 `laneflow-compiler`；
- 不提供迁移工具、导入特性或长期离线兼容路径；
- 编译器正确性使用编译器原生有类型模块，不以旧 JSON loader 为预言机；
- 可运行世界只从 `SharedNetworkRevision` 安装。

走廊生成器不再构造 current JSON；编制走 compiler，制品是 catalog 0.2 与 LFCA。详细现行路径见
`compiler-foundation.md`、`portable-canonical-artifact.md`、
`shared-static-network.md` 与 `traffic-runtime-shared-consumption.md`。
