# laneflow_core

引擎无关的 LaneFlow Core runtime crate。

本 crate 由 issue #9 初始化，作为 Core runtime 的实现边界。当前已在 v0.1 原型能力上对齐 v0.2 lane graph / route / ID handle 设计，并迁移到 v0.3 Vehicle Profile 与纵向运行态，提供 fixed-step tick、typed handle registry、最小 vehicle state、lane graph / route validation，以及 simple route following 原语：

- `CoreWorld`：保存固定步长、tick index、simulation time、lane graph、immutable Vehicle Profile registry、route / vehicle registry 和 stable update order，并在每个 tick 重建私有 occupancy / leader scratch；
- `TickInput` / `StepResult`：表达显式 tick 输入和 post-step 可观察输出；
- `CoreError`：表达 fixed delta、tick delta mismatch、时间溢出、lane graph / route / vehicle 静态校验、物理重叠、leader 计算、stale handle、route lifecycle 和数值校验错误；
- `LaneGraph` / `LaneEdge` / `EdgeLength`：表达 lane graph 输入，并在初始化时把 external edge ID 解析为 `EdgeHandle` runtime 连接；
- `Route`：表达外部 route edge sequence 输入，并由 `CoreWorld` 注册为 `RouteHandle` + `EdgeHandle` sequence；
- `VehicleSpawnInput`：表达 vehicle 初始化 / spawn 输入，使用 external vehicle ID、Vehicle Profile handle、route ID 和初始速度；
- `VehicleState` / `VehicleStatus`：表达 handle-based 最小车辆运行状态，包含 profile、front-bumper progress、当前速度和本 tick 实际加速度；
- `VehicleHandle` / `RouteHandle` / `EdgeHandle`：表达 Core runtime 内部 typed handle，external ID 通过 `CoreWorld` resolver 回查；
- `VehicleProfile` / `IidmProfileSpec`：表达经过校验的 immutable IIDM Vehicle Profile；
- `VehicleProfileHandle` / `VehicleProfileRegistry`：表达 profile typed handle、稳定输入顺序和双向 resolver；
- `InitialTrafficData`：统一校验 lane graph、初始 routes 与 immutable profile registry，供 data loader 与后续 world 初始化复用；
- `Speed` / `Acceleration` / `EdgeProgress`：用 newtype 包装非负速度、finite 有符号加速度和 front-bumper progress，避免 public API 直接散落裸 `f64`。
- `CoreEvent`：输出结构化 route transition 事件，包括 `VehicleChangedEdgeEvent` 与 `VehicleCompletedRouteEvent`，事件 payload 使用 handle 而不是复制 external ID。
- `spawn_vehicle` / `despawn_vehicle` / `register_route` / `remove_route`：提供最小 runtime lifecycle API；route 移除会拒绝仍被 live vehicle 引用的 route。
- 私有 occupancy / leader detection：按 physical edge 构建可复用扁平索引，沿 follower 已选 route 解析最近 leader，并在初始化与 runtime spawn 时拒绝物理车身重叠。

当前仍不实现 IIDM comfort control、safe-speed、no-overlap projection、signals、parking、Adapter API、C ABI 或 WASM 绑定；这些能力由后续 v0.x 子 issue 增量实现。

## 当前 data-format 边界

v0.2 里程碑已稳定 lane graph / route 的领域语义；当前 active 外部格式已直接演进为 v0.3，正式契约见 [data-format 设计](../../docs/design/data-format.md) 与 [JSON Schema](../../schemas/laneflow-data-v0.3.schema.json)。Core 的 `LaneGraph`、`Route`、Vehicle Profile 和 external ID / handle 边界与当前格式对齐；production JSON loader 位于同一 workspace 的 `laneflow-data`，Core 不依赖 Serde、JSON 或 schema validator。

LaneFlow 在 1.0 前只维护一个 active data format，旧版由 loader 明确拒绝；历史契约通过 Git 与 v0.2 收口报告审计。该政策不构成 v1.0 长期兼容承诺，详见 ADR 0008。

当前工具链策略：

- Rust edition: 2024
- MSRV: 1.96
- nightly-only 能力：不允许
