# laneflow_bevy

LaneFlow 的 Bevy 0.19 Reference Adapter crate。

v0.7 的 #169-#173 已提供最小 headless 宿主、Transform 同步、production validation 边界、可选调试绘制与原生参考示例：

- `LaneFlowPlugin`：安装 LaneFlow 专用 outer-frame 与 fixed schedule；
- `LaneFlowOuterFrame`：位于 Bevy `First` 之后，读取宿主已经更新的 `Time::delta()`；
- `LaneFlowFixed`：按 Session accumulator 运行零次或多次 Core fixed step；
- `LaneFlowFixedSet::{Lifecycle, Step, Observe}`：每个 fixed/catch-up step 内稳定重复的公共阶段链；
- `LaneFlowSession`：单活动 Bevy Resource，组合 `CoreWorld`、`SpatialRegistry`、可复用 pose/Transform 缓冲、catch-up 配置与最近一帧结果；
- `LaneFlowVehicleEntityMap`：由 Session 管理的 `VehicleHandle <-> Entity` 部分双射只读视图；
- `LaneFlowFramePlacement`：显式登记 canonical frame-root 与 `FramePlacementToken`；
- `LaneFlowPresentationReport`：报告 pose、mapped、unbound 与原子写入数量；
- `LaneFlowFrameReport` / `LaneFlowAdapterError`：暴露 step 数、完整 backlog、上限状态与结构化失败。
- `replace_completed_vehicle` / `LaneFlowVehicleReplaceOutcome`：调用方驱动的 `Completed -> Active` Core replacement 与可选 Entity 原子轮换。
- `LaneFlowDebugGizmosPlugin`：仅在 `debug-gizmos` feature 下提供预算受控的 frame axes、车辆 marker 与调用方中心线绘制。
- `native_reference`：仅在 `native-example` feature 下读取仓库 campus scenario/traffic/spatial artifacts，并使用完整 Bevy window/render 边界演示车辆移动。

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

## v0.8 vehicle lifecycle boundary

调用方把人口/回流 policy system 放入 `LaneFlowFixedSet::Lifecycle`，在其中调用独占 helper `replace_completed_vehicle(&mut World, old, &VehicleReplaceInput)`；需要读取本次 committed step 的 system 放入 `LaneFlowFixedSet::Observe`。`LaneFlowPlugin` 保证每个 catch-up step 都按 `Lifecycle -> Step -> Observe` 执行，而 presentation 仍在每个 outer frame 最多提交一次。

成功结果 `LaneFlowVehicleReplaceOutcome::Replaced` 包含 old/new handle 与 `Option<Entity>`。已绑定 old handle 会复用同一 Entity；未绑定 old handle 只替换 Core vehicle 并保持未绑定。`Blocked` 是可重试结果，不修改 Core、映射、Transform 或 Session error；fatal error 会停止该 outer frame 的 Core/catch-up 推进并保留 backlog。Completed proxy 在等待和 replacement 当下保留最后 Transform，由下一次正常 presentation 更新入口位姿。

本边界不拥有目标车辆数、seed、入口/route 抽样、pending/retry queue、初始人口或通用 runtime spawn/despawn；这些仍由调用方 policy 管理。初始车辆应在创建 `LaneFlowSession` 前写入 Core。

## 可选 debug Gizmos

默认 feature graph 不包含 Gizmos、window、renderer 或 umbrella `bevy`。需要调试绘制时显式启用 `debug-gizmos`，并在 `LaneFlowDebugGizmosPlugin` 之前安装宿主的 `GizmoPlugin`（完整 `DefaultPlugins` 已包含它）。运行时还必须显式插入 `LaneFlowDebugGizmosConfig`；其 `Default` 保持关闭且预算为零。

```rust
# #[cfg(feature = "debug-gizmos")]
# mod debug_example {
use bevy_app::App;
use bevy_asset::AssetPlugin;
use bevy_gizmos::GizmoPlugin;
use laneflow_bevy::{
    LaneFlowDebugGizmosConfig, LaneFlowDebugGizmosPlugin, LaneFlowPlugin,
};

# fn install(app: &mut App) {
app.add_plugins((
    AssetPlugin::default(),
    GizmoPlugin,
    LaneFlowPlugin,
    LaneFlowDebugGizmosPlugin,
));
app.insert_resource(LaneFlowDebugGizmosConfig::enabled(1_000, 4_000));
# }
# }
```

绘制系统只消费当前 outer frame 已通过完整 Adapter 校验的 presentation batch；当前批次失败时不会回退到上一帧。车辆过滤后仍按 batch 稳定顺序应用预算。可选 `LaneFlowDebugCenterlines` 必须带有匹配的 `CanonicalFrameId`，只保留调用方点序并按 segment 预算绘制，不计算弧长、不重采样，也不替代 `SpatialRegistry`。

本机可视 smoke 使用独立 `debug-gizmos-smoke` feature，把 Bevy 3D/window/render 依赖限制在 example 边界。该 feature 显式选择 winit 与 Linux X11 后端，不启用包含 Wayland 等额外平台能力的 `default_platform`：

```powershell
cargo +1.96.0 run -p laneflow-bevy --example debug_gizmos_smoke --features debug-gizmos-smoke --locked
```

## Campus native reference example

`native_reference` 从 `examples/data/` 读取并通过 production loader 校验 `v0.1-campus.scenario.json` 及其 traffic/spatial 引用。示例用内建 cuboid/plane 生成道路和车辆，创建非原点 frame-root，并把两辆 Core vehicle 通过 Adapter 映射绑定到 Bevy proxy Entity。相机、输入、renderer、mesh/material 和截图逻辑只存在于 example，不属于 production Adapter API。

从仓库根目录运行：

```powershell
cargo +1.96.0 run -p laneflow-bevy --example native_reference --features native-example --locked
```

运行时控制：

- `G`：切换预算受控的 debug Gizmos；窗口标题同步显示 `ON/OFF`。
- `F12`：在当前工作目录保存 `laneflow-native-example.png`。
- `Esc` 或关闭窗口：退出示例。

启动时的文件读取、manifest 引用、size/digest、Traffic/Spatial normalization 或 Core world 构造失败会带路径和阶段信息返回；运行中的 Adapter 结构化错误写入 Bevy 日志。CI 使用以下 dedicated compile check，GUI smoke 仍只在本机执行：

```powershell
cargo +1.96.0 check -p laneflow-bevy --example native_reference --features native-example --locked
```

校园 headless E2E 与 10k/100k 专项验证由 #171 交付；固定机验证协议、逐轮数据与适用边界见 `../../docs/reference/v0.7-bevy-validation.md` 和 `../../docs/reference/v0.7-bevy-performance-evidence.json`。#172 的静态 smoke 只验证 debug Gizmos；#173 的 native example 才覆盖真实制品加载、Core 驱动移动、frame-root、映射和完整 window/render 演示。#173 的本机证据见 `../../docs/reference/v0.7-bevy-native-example-validation.md`，v0.7 的最终收口基线见 `../../docs/reference/v0.7-bevy-closure-review.md`。
