# laneflow-spatial

引擎无关的标准空间类型，以及共享修订上的位姿采样。交通裁决不在这里。

## 职责与边界

`SpatialSession` 绑定 `Arc<SharedNetworkRevision>`，消费调用方给出的已提交位姿来源。它不导入运行时句柄，也不依赖 `laneflow-runtime`。无窗口交通可以完全不使用本 crate。

依赖方向是：

```text
laneflow-spatial -> laneflow-static-network
laneflow-spatial -> laneflow-static-contract
laneflow-spatial -X-> laneflow-runtime
```

公共 API 不泄漏第三方数学类型或宿主引擎类型。

## 最小使用入口

无窗口示例用 `SpatialBuildOption::Omit` 构建修订，因此没有 Spatial component，也不创建 session。见[从源码接入无窗口交通运行时][source-integration]。

修订保留了 Spatial 时：

1. `SpatialSession::bind(revision)`。
2. 从 `TrafficWorld` 读取已提交位姿来源。
3. `extract_pose_batch` 写入调用方自己的输出缓冲。全部记录成功才替换输出，失败保持旧批次。

## 主要约束与限制

- 修订没有 Spatial component 时，`bind` 返回 `Ok(None)`。有 Spatial 但缺少 lane pose 时，返回 `MissingLanePose`。
- 几何使用有界的 canonical `f32`。交通进度和距离仍以整数毫米为准。
- `PoseSource::Parking` 只接受显式车位序号。虚拟停泊没有位姿输入；没有位姿也不表示车辆已移除。
- 提供给 Bevy 时，session 必须和世界指向同一根 `Arc`。

## 示例与验证

Bevy 示例在需要画面时绑定 Spatial。无窗口示例故意省略它，用来确认交通推进本身不依赖空间采样。

```powershell
cargo +1.99.1 test --locked -p laneflow-spatial
cargo +1.99.1 run --locked -p laneflow-runtime --example headless
```

## 深入阅读

- [从源码接入无窗口交通运行时][source-integration]
- [空间几何][spatial-geometry]
- [当前能力与限制 · Spatial][capabilities-spatial]
- [Bevy 参考适配][laneflow-bevy-readme]

[source-integration]: ../../docs/guides/source-integration.md
[spatial-geometry]: ../../docs/design/spatial-geometry.md
[capabilities-spatial]: ../../docs/reference/capabilities.md#spatial
[laneflow-bevy-readme]: ../../crates/laneflow-bevy/README.md
