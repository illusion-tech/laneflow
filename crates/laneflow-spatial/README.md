# laneflow_spatial

LaneFlow 引擎无关的标准空间类型与共享根位姿采样 crate。

当前入口：

- 拥有稳定字符串身份的 `CanonicalFrameId`；
- 使用 `f32`、字段私有且只允许受检构造的 canonical 点、向量与单位方向；
- `SpatialSession::bind(Arc<SharedNetworkRevision>)` 绑定共享根；无 Spatial
  component 时返回 `Ok(None)`；
- pose 批次使用不透明 `PoseRecordId` 与 `PoseInput`，不导入 Runtime handle；
- `extract_pose_batch` 在全部记录成功后才提交输出，失败保持旧批次。

依赖方向固定为：

```text
laneflow-spatial -> laneflow-static-network
laneflow-spatial -> laneflow-static-contract
laneflow-spatial -X-> laneflow-runtime
```

本 crate 不向公共 API 泄漏第三方数学类型或宿主引擎类型。契约见
`docs/design/traffic-runtime-shared-consumption.md`。
