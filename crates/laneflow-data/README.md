# laneflow_data

LaneFlow 当前 v0.4 external package 的 JSON loader 与 Core normalization crate。

本 crate 负责：

- 只接受 `formatVersion: "0.4"`，并在 strict current shape 前拒绝旧版和未来版；
- 解析 closed v0.4 DTO、SI units、`toEdgeId` / `edgeIds` 与 static Signals；
- 保留 JSON syntax/shape path、line/column 和结构化 Core source；
- 调用 `laneflow-core` constructors normalization lane graph、routes、Vehicle Profiles、StopLines、MovementGates、Groups、Controllers 与 Phases；
- 返回单一 current `LoadedPackage`，不公开 raw wire DTO 或历史 version variant。

本 crate 不读取文件、不创建 `CoreWorld`、不拥有 fixed tick 或 runtime entity。依赖方向固定为 `laneflow-data -> laneflow-core`。
