# laneflow_spatial

LaneFlow 引擎无关的标准空间类型与几何注册表 crate。

本 crate 当前由 issue #133 建立生产边界，提供：

- 拥有稳定字符串身份的 `CanonicalFrameId`；
- 使用 `f32`、字段私有且只允许受检构造的 canonical 点、向量与单位方向；
- 点每轴限制在 `[-16_384 m, 16_384 m]`，向量只要求有限；
- 非有限值拒绝、点范围校验、带符号零规范化、稳健归一化与受检命名运算；
- 以 Core `EdgeHandle` 为键、保持 `LaneGraph::edges()` 顺序的 opaque immutable `SpatialRegistry`；
- LaneFlow 自有的结构化 `SpatialError` 与 `SpatialAxis`。

依赖方向固定为 `laneflow-spatial -> laneflow-core`。本 crate 不向公共 API 泄漏第三方数学类型或宿主引擎类型，也不要求 Core 提供 Spatial 注册表；Core 可以继续独立用于无图形宿主运行。

当前 crate-private staged registry builder 只建立完整覆盖、重复/未知/缺失 handle 校验和失败原子性。空间包与场景清单由 #134 交付；折线、长度绑定、采样与公开 registry 构造入口由 #135 交付；frame 放置生命周期和批量位姿由 #136 交付。
