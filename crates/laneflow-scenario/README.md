# laneflow-scenario

`laneflow-scenario` 提供可选、引擎无关的 reference scenario runtime support。

当前只包含 v0.9 protected-turning signalized-corridor 的目标人口、catalog 0.2
PortalLane/weighted RouteChoice normalization、seeded 初始分布、ordered completion
消费和 blocked recycle retry。它依赖 `laneflow-core`，但 Core、Engine Adapter 和宿主
游戏都不反向依赖本 crate。

详细契约见 `docs/design/signalized-corridor-population.md` 与 ADR 0016。
