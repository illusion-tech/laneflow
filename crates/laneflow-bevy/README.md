# laneflow_bevy

LaneFlow 的 Bevy 0.19 Reference Adapter crate。

当前入口消费 `TrafficWorld` 与可选 `SpatialSession`。`LaneFlowSession::new` 在提供 Spatial 时要求 `Arc::ptr_eq`：

- `LaneFlowPlugin`：安装 LaneFlow 专用 outer-frame 与 fixed schedule；
- `LaneFlowOuterFrame`：位于 Bevy `First` 之后，读取宿主已经更新的 `Time::delta()`；
- `LaneFlowFixed`：按 Session accumulator 运行零次或多次 `TrafficWorld` 固定步进；
- `LaneFlowFixedSet::{Lifecycle, Step, Observe}`：每个 fixed/catch-up step 内稳定重复的公共阶段链；
- `LaneFlowSession`：单活动 Bevy Resource，组合 `TrafficWorld`、可选 `SpatialSession`、catch-up 配置、Vehicle/Entity 映射与最近一帧结果；
- `replace_completed_vehicle`：Lifecycle 边界的 typed 原子替换；已绑定车辆复用同一 Entity，`Blocked` 时映射与 Transform 不变；
- `LaneFlowWorldMut::{reserve_parking,cancel_parking,park_vehicle,leave_parking,rebind_parking_route,spawn_parked_vehicle}`：由 `LaneFlowSession::world_mut()` 返回的薄包装转发 Runtime typed parking lifecycle；virtual Parked 无 pose 但映射仍 live；
- `despawn_vehicle`：typed Runtime removal + Entity unbind；先验证已绑定 Entity 仍 live，只在 Runtime 成功后以不可失败路径移除映射，失败零副作用；
- `LaneFlowFrameReport` / `LaneFlowAdapterError`：暴露 step 数、完整 backlog、上限状态与结构化失败。

宿主必须在第一次 `App::update` 前安装 Bevy `TimePlugin`（或包含它的 plugin group）并插入一个 `LaneFlowSession`。本 crate 不修改 Bevy `Time<Fixed>`，也不重复安装宿主 plugin。

```rust
use std::num::NonZeroU32;

use bevy_app::App;
use bevy_time::TimePlugin;
use laneflow_bevy::{LaneFlowPlugin, LaneFlowSession, LaneFlowSessionConfig};
use laneflow_runtime::TrafficWorld;
use laneflow_spatial::SpatialSession;

# fn install(
#     app: &mut App,
#     world: TrafficWorld,
#     spatial: Option<SpatialSession>,
# ) {
let config = LaneFlowSessionConfig::new(NonZeroU32::new(8).expect("non-zero"));
app.add_plugins((TimePlugin, LaneFlowPlugin));
app.insert_resource(LaneFlowSession::new(world, spatial, config).expect("paired session"));
# }
```

campus / `native_reference` 的 Core 入口已删除。现行走廊 Bevy 最小路径使用检入的
catalog 0.3 与 LFCA，prepare 绑到已安装共享路网修订，不恢复 50–200 回流；回流见
[#475](https://github.com/illusion-tech/laneflow/issues/475)。Bevy debug gizmos 不是现行交付（[#473](https://github.com/illusion-tech/laneflow/issues/473) 已关闭）。

## 依赖与分发

走廊 example / smoke 的第三方 **dev-dependency** 不进入 `laneflow-bevy` 生产 graph：

- `toml 1.1.4+spec-1.1.0`：crates.io，MIT OR Apache-2.0；只解析检入的 catalog TOML。生成器已使用同一 crate。
- `bevy_math 0.19.1`：crates.io，MIT OR Apache-2.0；smoke 用 `Vec3` 组 `looking_to`。生产 crate 已依赖 `bevy_transform 0.19`，该 crate 本就在 lock graph 中。

二者均在 `deny.toml` 允许的 SPDX 分支内，无 copyleft，不新增 crates.io 之外的来源。已知漏洞由 CI `Dependency policy`（cargo-deny advisories）把关。

最小 Bevy 证据是 `runtime_min`；走廊最小路径是 `signalized_corridor`：

```powershell
cargo +1.98.0 test --locked -p laneflow-bevy --test runtime_min_smoke
cargo +1.98.0 test --locked -p laneflow-bevy --test signalized_corridor_smoke
cargo +1.98.0 check --locked -p laneflow-bevy --example runtime_min --example signalized_corridor --features native-example
```
