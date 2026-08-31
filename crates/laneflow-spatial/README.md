# laneflow_spatial

LaneFlow 引擎无关的标准空间类型与共享根位姿采样 crate。

当前入口：

- 拥有稳定字符串身份的 `CanonicalFrameId`；
- 使用 `f32`、字段私有且只允许受检构造的 canonical 点、向量与单位方向；
- `SpatialSession::bind(Arc<SharedNetworkRevision>)` 绑定共享根；无 Spatial
  component 时返回 `Ok(None)`；有 Spatial 但无 `lane_pose` 时返回
  `Err(SpatialBindError::MissingLanePose)`；
- pose 批次使用不透明 `PoseRecordId`、`PoseSource` 与 `PoseInput`，不导入 Runtime handle；
- `PoseSource::Parking` 只接受显式 `ParkingSpaceOrdinal`；virtual Parked 没有 pose input，
  无 pose 也不表示 vehicle 已移除；
- `extract_pose_batch(token, inputs, &mut output)` 回显 `FramePlacementToken`，
  全部记录成功后才写入 `output`，失败保持旧批次。

依赖方向固定为：

```text
laneflow-spatial -> laneflow-static-network
laneflow-spatial -> laneflow-static-contract
laneflow-spatial -X-> laneflow-runtime
```

本 crate 不向公共 API 泄漏第三方数学类型或宿主引擎类型。契约见
`docs/design/traffic-runtime-shared-consumption.md`。
