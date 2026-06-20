# laneflow_core

引擎无关的 LaneFlow Core runtime crate。

本 crate 由 issue #9 初始化，作为 v0.1 Core 的实现边界。当前已提供 fixed-step tick、最小 vehicle state、最小 lane graph / route validation，以及 simple route following 原语：

- `CoreWorld`：保存固定步长、tick index、simulation time 和最小车辆集合；
- `TickInput` / `StepResult`：表达显式 tick 输入和 post-step 可观察输出；
- `CoreError`：表达 fixed delta、tick delta mismatch、时间溢出、lane graph / route / vehicle 静态校验和数值校验错误；
- `LaneGraph` / `LaneEdge` / `EdgeLength`：表达 v0.1 内部 lane graph 输入，并校验 edge id、edge length 和 next edge 引用；
- `Route`：表达 v0.1 内部 route edge sequence，并配合 `CoreWorld::with_traffic_data` 校验 edge 存在性和连通性；
- `VehicleState` / `VehicleStatus`：表达最小车辆运行状态；
- `Speed` / `EdgeProgress`：用 newtype 包装 speed 和 edge progress，避免 public API 直接散落裸 `f64`。
- `CoreEvent`：输出结构化 route transition 事件，包括 `VehicleChangedEdgeEvent` 与 `VehicleCompletedRouteEvent`。

当前仍不实现 vehicle following、signals、parking、runtime commands、Adapter API、C ABI 或 WASM 绑定；这些能力由后续 v0.x 子 issue 增量实现。v0.1 的 lane graph / route 类型仍是内部实现输入，不是稳定 data spec。

当前工具链策略：

- Rust edition: 2024
- MSRV: 1.96
- nightly-only 能力：不允许
