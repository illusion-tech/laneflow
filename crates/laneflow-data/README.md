# laneflow_data

LaneFlow 当前 v0.6 外部数据包的 JSON loader 与 Core 规范化 crate。

本 crate 负责：

- 只接受 `formatVersion: "0.6"`，并在严格解析当前结构前拒绝旧版和未来版；
- 解析封闭的 v0.6 私有传输对象（DTO）、国际单位制（SI）单位、`toEdgeId` / `edgeIds`、静态信号与静态停车数据；
- 保留 JSON syntax/shape path、line/column 和结构化 Core source；
- 调用 `laneflow-core` constructors normalization lane graph、routes、Vehicle Profiles、Signals、Parking areas/spaces、anchors 与 geometry；
- 返回单一 current `LoadedPackage`，不公开 raw wire DTO 或历史 version variant。

本 crate 不读取文件、不创建 `CoreWorld`、不拥有 fixed tick 或 runtime entity。依赖方向固定为 `laneflow-data -> laneflow-core`。
