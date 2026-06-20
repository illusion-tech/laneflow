# laneflow_core

引擎无关的 LaneFlow Core runtime crate。

本 crate 由 issue #9 初始化，作为 v0.1 Core 的实现边界。当前已提供 fixed-step tick 与最小 vehicle state 原语：

- `CoreWorld`：保存固定步长、tick index、simulation time 和最小车辆集合；
- `TickInput` / `StepResult`：表达显式 tick 输入和 post-step 可观察输出；
- `CoreError`：表达 fixed delta、tick delta mismatch、时间溢出和数值校验错误；
- `VehicleState` / `VehicleStatus`：表达最小车辆运行状态；
- `Speed` / `EdgeProgress`：用 newtype 包装 speed 和 edge progress，避免 public API 直接散落裸 `f64`。

当前仍不实现 lane graph traversal、route following、route transition events、runtime commands、Adapter API、C ABI 或 WASM 绑定；这些能力由后续 v0.1 / v0.x 子 issue 增量实现。

当前工具链策略：

- Rust edition: 2024
- MSRV: 1.96
- nightly-only 能力：不允许
