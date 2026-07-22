# laneflow_data

LaneFlow 当前 Traffic v0.7、SpatialPackage v0.1 与 ScenarioManifest v0.1 的内存 JSON loader 和 normalization crate。

Traffic loader 继续：

- 只接受 `formatVersion: "0.7"`，并在 strict current shape 前拒绝旧版和未来版；
- 解析 closed v0.7 DTO、required per-edge `speedLimit`、SI units、static Signals 与 static Parking；
- 调用 `laneflow-core` constructors 规范化 lane graph、routes、Vehicle Profiles、Signals 与 Parking；
- 返回单一 current `LoadedPackage`，不公开 raw wire DTO 或历史 version variant。

Scenario loader 通过 `from_scenario_json_slice` / `from_scenario_json_str`：

- 只接受 ScenarioManifest v0.1 与 SpatialPackage v0.1；
- 按不透明、大小写敏感的 `artifactRef` 配对调用方已经读取到内存的原始制品；
- 在解析制品前校验 raw byte `size` 与 `sha256:<64 lowercase hex>`；
- 复用现有 Traffic loader，再以该 LaneGraph 校验 Spatial edge 的 unknown、duplicate 与 complete coverage；
- 先以 `f64` 暂存空间坐标，执行有限性和每轴 `[-16_384, 16_384] m` 检查，再转换为 canonical `f32` 点；
- 返回按 `LaneGraph::edges()` 稳定顺序排列的 `LoadedScenario`，任一步失败都不暴露部分结果。

本 crate 不读取文件、不联网、不创建 `CoreWorld`、不拥有 fixed tick 或 runtime entity，也不执行 #135 所属的退化段、弧长、Traffic length binding、端点连续性、基底、采样或 `SpatialRegistry` 提交。

依赖方向固定为：

```text
laneflow-data -> laneflow-core
laneflow-data -> laneflow-spatial
```
