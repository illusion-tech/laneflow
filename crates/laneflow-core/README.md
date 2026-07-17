# laneflow_core

引擎无关的 LaneFlow Core runtime crate。

本 crate 由 issue #9 初始化，作为当前 Core runtime 的实现边界。它提供 fixed-step tick、typed handle registry、lane graph / route validation、Vehicle Profile、IIDM comfort control、emergency safe-speed、ballistic integration 与最终 no-overlap projection：

- `CoreWorld`：保存固定步长、tick index、simulation time、lane graph、immutable Vehicle Profile / Signals / Parking registries、committed signal snapshot、route / vehicle registry 和 stable update order，并复用私有 signal / candidate state / occupancy / leader / longitudinal scratch；
- `TickInput` / `StepResult`：表达显式 tick 输入和 post-step 可观察输出；
- `CoreError`：表达 fixed delta、tick delta mismatch、时间溢出、lane graph / route / Signals / Parking / vehicle 静态校验、物理重叠、leader / longitudinal 非有限计算、stale handle、route lifecycle 和数值校验错误；
- `LaneGraph` / `LaneEdge` / `EdgeLength`：表达 lane graph 输入，并在初始化时把 external edge ID 解析为 `EdgeHandle` runtime 连接；
- `Route`：表达外部 route edge sequence 输入，并由 `CoreWorld` 注册为 `RouteHandle` + `EdgeHandle` sequence；
- `VehicleSpawnInput`：表达 vehicle 初始化 / spawn 输入，使用 external vehicle ID、Vehicle Profile handle、route ID 和初始速度；
- `VehicleState` / `VehicleStatus`：表达 handle-based 最小车辆运行状态，包含 profile、front-bumper progress、当前速度和本 tick 实际加速度；
- `VehicleHandle` / `RouteHandle` / `EdgeHandle` / Parking handles：表达 Core runtime 内部 typed handle，external ID 通过对应 registry/world resolver 回查；
- `VehicleProfile` / `IidmProfileSpec`：表达经过校验的 immutable IIDM Vehicle Profile；
- `VehicleProfileHandle` / `VehicleProfileRegistry`：表达 profile typed handle、稳定输入顺序和双向 resolver；
- `SignalRegistry`、Signal handles 与 current snapshots：表达经过归一化的 StopLine、MovementGate、Group、Controller/Phase，保留稳定输入顺序和预解析 resolver，并以 absolute integer time 提供 time-0/post-step Controller/Group/Gate query；
- `ParkingRegistry`、`ParkingArea` / `ParkingSpace` 与 opaque handles：表达 immutable area membership、entry/exit anchors、edge-relative geometry、稳定输入/member 顺序和 O(1) resolvers；
- `InitialTrafficData`：统一校验 lane graph、初始 routes、immutable profile / Signals / Parking registries，并把 graph-dependent registries 原子重绑定到自身 lane graph；
- `Speed` / `Acceleration` / `EdgeProgress`：用 newtype 包装非负速度、finite 有符号加速度和 front-bumper progress，避免 public API 直接散落裸 `f64`。
- `CoreEvent`：输出 signal/following safety projection、route transition、route completion 与稀疏 signal phase/aspect change 事件，payload 使用 handle 而不是复制 external ID。
- `spawn_vehicle` / `despawn_vehicle` / `register_route` / `remove_route`：提供最小 runtime lifecycle API；route 移除会拒绝仍被 live vehicle 引用的 route。
- 私有 occupancy / leader detection：按 physical edge 构建可复用扁平索引，沿 follower 已选 route 解析最近 leader，并在初始化与 runtime spawn 时拒绝物理车身重叠。
- 私有 Vehicle Following pipeline：基于 tick-start snapshot 计算 IIDM comfort acceleration 与 emergency safe-speed，再通过确定性的 functional graph 投影得到最大可行 no-overlap travel；事件与状态只在整 tick 成功后原子提交。

当前已实现 v0.4 Signals 全链路和 v0.5 immutable static Parking registry/current data；停车 reservation/occupancy/commands、ParkingStop 与 parked lifecycle 尚由 #108/#109 承接。仍不实现 lane changing、intersection conflict、公开 controller extension、Adapter API、C ABI 或 WASM 绑定。完整边界见 [Signal System](../../docs/design/signal-system.md) 与 [Parking System](../../docs/design/parking-system.md)。

## 当前 data-format 边界

当前唯一 active 外部格式是 v0.5，正式契约见 [data-format 设计](../../docs/design/data-format.md) 与 [JSON Schema](../../schemas/laneflow-data-v0.5.schema.json)。Core 的 `LaneGraph`、`Route`、Vehicle Profile、Signals、Parking 和 external ID / handle 边界与当前格式对齐；production JSON loader 位于同一 workspace 的 `laneflow-data`，Core 不依赖 Serde、JSON 或 schema validator。

LaneFlow 在 1.0 前只维护一个 active data format，旧版由 loader 明确拒绝；历史能力来源和迁移证据通过 Git 与 milestone 收口报告审计，不在 production 代码中保留并行兼容实现。该政策不构成 v1.0 长期兼容承诺，详见 ADR 0008。

当前工具链策略：

- Rust edition: 2024
- MSRV: 1.96
- nightly-only 能力：不允许
