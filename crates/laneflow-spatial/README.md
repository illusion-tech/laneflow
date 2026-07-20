# laneflow_spatial

LaneFlow 引擎无关的标准空间类型、折线绑定与确定性采样 crate。

本 crate 由 #133 建立生产边界，并由 #135 完成首版折线绑定与采样，提供：

- 拥有稳定字符串身份的 `CanonicalFrameId`；
- 使用 `f32`、字段私有且只允许受检构造的 canonical 点、向量与单位方向；
- 点每轴限制在 `[-16_384 m, 16_384 m]`，向量只要求有限；
- 非有限值拒绝、点范围校验、带符号零规范化、稳健归一化与受检命名运算；
- 借用 `EdgeHandle + &[CanonicalPoint3F32]` 的 `SpatialEdgeInput`，不会在调用边界逐点复制 `frameId` 或坐标字段名；
- 以 Core `EdgeHandle` 为键、保持 `LaneGraph::edges()` 顺序的 opaque immutable `SpatialRegistry`；
- `SpatialRegistry::try_new` 的完整覆盖、线段、basis、累计弧长、Core 长度和连接端点原子校验；
- `SpatialRegistry::sample(EdgeHandle, EdgeProgress)` 返回位置、切向量和上方向组成的 `CanonicalPoseF32`；
- 内部顶点右连续、最终端点使用入段，以及同一版本/目标/运行时/输入下的稳定错误和连续值 bits；
- LaneFlow 自有的结构化 `SpatialError` 与 `SpatialAxis`。

依赖方向固定为 `laneflow-spatial -> laneflow-core`。本 crate 不向公共 API 泄漏第三方数学类型或宿主引擎类型，也不要求 Core 提供 Spatial 注册表；Core 可以继续独立用于无图形宿主运行。

量化后规则固定为：线段长度严格大于 `0.1 m`，projected-up 长度大于等于 `sin(0.5°)`，连接端点距离小于等于 `0.005 m`；长度差必须小于等于 `max(0.01 m, 1e-6 × max(Core 长度, 几何弧长))`，current-f64 Core 量化余量为零。空间包与场景清单由 #134 交付，其 `LoadedSpatialPackage` 可直接映射为借用输入；frame 放置生命周期、批量位姿和 Parking pose 由 #136 交付。
