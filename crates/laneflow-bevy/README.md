# laneflow_bevy

LaneFlow 的 Bevy 0.19 Reference Adapter crate。

当前 #169 切片提供最小 headless 宿主边界：

- `LaneFlowPlugin`：安装 LaneFlow 专用 outer-frame 与 fixed schedule；
- `LaneFlowOuterFrame`：位于 Bevy `First` 之后，读取宿主已经更新的 `Time::delta()`；
- `LaneFlowFixed`：按 Session accumulator 运行零次或多次 Core fixed step；
- `LaneFlowSession`：单活动 Bevy Resource，组合 `CoreWorld`、`SpatialRegistry`、可复用 pose scratch、catch-up 配置与最近一帧结果；
- `LaneFlowFrameReport` / `LaneFlowAdapterError`：暴露 step 数、完整 backlog、上限状态与结构化失败。

宿主必须在第一次 `App::update` 前安装 Bevy `TimePlugin`（或包含它的 plugin group）并插入一个 `LaneFlowSession`。本 crate 不修改 Bevy `Time<Fixed>`，也不重复安装 `TimePlugin` 或 `TransformPlugin`。

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

Vehicle/Entity 映射、原子 Transform apply、Gizmos 与 native example 分别由 #170、#172 和 #173 交付，不属于当前 crate 切片的已完成能力。
