# laneflow_data

LaneFlow 当前 v0.3 external package 的 JSON loader 与 Core normalization crate。

本 crate 负责：

- 只接受 `formatVersion: "0.3"`，并在严格 shape validation 前拒绝旧版和未来版；
- 解析 JSON syntax、wire shape 与 units；
- 将 external lane graph、route 和 Vehicle Profile 转换为 `laneflow_core::InitialTrafficData`；
- 返回带 JSON path、line/column、expected/actual version 或 Core source 的结构化 `DataError`。

本 crate 不读取文件、不创建 `CoreWorld`，也不拥有 fixed tick、initial vehicles、spawn schedule、历史格式迁移或 Adapter asset API。详细边界见 [`docs/design/data-loading.md`](../../docs/design/data-loading.md)、[ADR 0007](../../docs/adr/0007-traffic-data-crate-and-loader-boundary.md) 与 [ADR 0008](../../docs/adr/0008-pre-1.0-data-format-version-policy.md)。