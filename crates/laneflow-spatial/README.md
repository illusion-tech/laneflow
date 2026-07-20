# laneflow_spatial

LaneFlow 引擎无关的标准空间类型、折线绑定、确定性采样与批量位姿 crate。

本 crate 由 #133 建立生产边界、#135 完成首版折线绑定与采样，并由 #136 完成 Adapter-facing 批量位姿，提供：

- 拥有稳定字符串身份的 `CanonicalFrameId`；
- 使用 `f32`、字段私有且只允许受检构造的 canonical 点、向量与单位方向；
- 点每轴限制在 `[-16_384 m, 16_384 m]`，向量只要求有限；
- 非有限值拒绝、点范围校验、带符号零规范化、稳健归一化与受检命名运算；
- 借用 `EdgeHandle + &[CanonicalPoint3F32]` 的 `SpatialEdgeInput`，不会在调用边界逐点复制 `frameId` 或坐标字段名；
- 以 Core `EdgeHandle` 为键、保持 `LaneGraph::edges()` 顺序的 opaque immutable `SpatialRegistry`；
- `SpatialRegistry::try_new` 的完整覆盖、线段、basis、累计弧长、Core 长度和连接端点原子校验；
- `SpatialRegistry::sample(EdgeHandle, EdgeProgress)` 返回位置、切向量和上方向组成的 `CanonicalPoseF32`；
- `PoseInputRecord` 使用 `Lane { edge, progress }` 或 `Parking { space }` 显式表达调用方从 committed Core snapshot 选择的位置权威；
- `CanonicalPoseBatchF32` 在 batch header 只保存一次 `CanonicalFrameId + FramePlacementToken`，每条 `CanonicalPoseRecordF32` 只保存 vehicle handle 和 pose；
- `SpatialRegistry::extract_pose_batch` 使用调用方拥有的 committed/scratch 双缓冲，全部记录成功后才交换输出，失败时旧 frame、placement token 和 records 完全不变；
- Parking pose 从 entry anchor 采样，通过 `up × tangent` 左方向、横向偏移和航向偏移生成 canonical `f32` 位姿；
- 内部顶点右连续、最终端点使用入段，以及同一版本/目标/运行时/输入下的稳定错误和连续值 bits；
- LaneFlow 自有的结构化 `SpatialError` 与 `SpatialAxis`；批量 record 错误携带稳定输入序号和 vehicle handle。

依赖方向固定为 `laneflow-spatial -> laneflow-core`。本 crate 不向公共 API 泄漏第三方数学类型或宿主引擎类型，也不要求 Core 提供 Spatial 注册表；Core 可以继续独立用于无图形宿主运行。

量化后规则固定为：线段长度严格大于 `0.1 m`，projected-up 长度大于等于 `sin(0.5°)`，连接端点距离小于等于 `0.005 m`；长度差必须小于等于 `max(0.01 m, 1e-6 × max(Core 长度, 几何弧长))`，current-f64 Core 量化余量为零。空间包与场景清单由 #134 交付，其 `LoadedSpatialPackage` 可直接映射为借用输入；#136 的 placement token 只标识宿主放置版本，不包含或授权修改宿主 Transform。10k/100k 吞吐、稳态分配和 retained-memory Gate 仍由 #137 交付。
