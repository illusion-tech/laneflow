# laneflow_bevy

LaneFlow 的 Bevy 0.19 Reference Adapter crate。

当前 #169-#171 切片提供最小 headless 宿主、Transform 同步和 production validation 边界：

- `LaneFlowPlugin`：安装 LaneFlow 专用 outer-frame 与 fixed schedule；
- `LaneFlowOuterFrame`：位于 Bevy `First` 之后，读取宿主已经更新的 `Time::delta()`；
- `LaneFlowFixed`：按 Session accumulator 运行零次或多次 Core fixed step；
- `LaneFlowSession`：单活动 Bevy Resource，组合 `CoreWorld`、`SpatialRegistry`、可复用 pose/Transform 缓冲、catch-up 配置与最近一帧结果；
- `LaneFlowVehicleEntityMap`：由 Session 管理的 `VehicleHandle <-> Entity` 部分双射只读视图；
- `LaneFlowFramePlacement`：显式登记 canonical frame-root 与 `FramePlacementToken`；
- `LaneFlowPresentationReport`：报告 pose、mapped、unbound 与原子写入数量；
- `LaneFlowFrameReport` / `LaneFlowAdapterError`：暴露 step 数、完整 backlog、上限状态与结构化失败。

宿主必须在第一次 `App::update` 前安装 Bevy `TimePlugin`（或包含它的 plugin group）并插入一个 `LaneFlowSession`。使用非空 Vehicle/Entity 映射时，宿主还必须安装 `TransformPlugin`，把每个 proxy 作为当前 frame-root 的直接 child，并保持 root 单位缩放。本 crate 不修改 Bevy `Time<Fixed>`，也不重复安装宿主 plugin。

```rust
use std::num::NonZeroU32;

use bevy_app::App;
use bevy_time::TimePlugin;
use laneflow_bevy::{LaneFlowPlugin, LaneFlowSession, LaneFlowSessionConfig};

# fn install(
#     app: &mut App,
#     core: laneflow_core::CoreWorld,
#     spatial: laneflow_spatial::SpatialRegistry,
# ) {
let config = LaneFlowSessionConfig::new(NonZeroU32::new(8).expect("non-zero"));
app.add_plugins((TimePlugin, LaneFlowPlugin));
app.insert_resource(LaneFlowSession::new(core, spatial, config));
# }
```

Session 通过 `bind_vehicle_entity`、`unbind_vehicle`、`unbind_entity` 与 `rebind_vehicle_entity` 管理映射，并通过 `set_frame_placement` 设置 root/token。每个 `PostUpdate` 在 `TransformSystems::Propagate` 前，从 committed Core 顺序批量提取 Spatial pose；未绑定记录稳定跳过，任一已映射记录失败则所有目标 local Transform 保持旧值。`1 LaneFlow meter = 1 Bevy unit`，LaneFlow tangent 映射到 Bevy `Transform::forward()`。

校园场景 E2E 与 10k/100k 专项验证由 #171 提供；Gizmos 与 native example 分别由 #172 和 #173 交付。固定机验证协议、逐轮数据与适用边界见 `../../docs/reference/v0.7-bevy-validation.md` 和 `../../docs/reference/v0.7-bevy-performance-evidence.json`。
