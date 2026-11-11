# laneflow-bevy

LaneFlow 的 Bevy 0.20 Reference Adapter。它负责窗口、实体和变换，不重写交通规则。

## 职责与边界

当前入口消费 `TrafficWorld` 与可选 `SpatialSession`。提供 Spatial 时，`LaneFlowSession::new` 要求两者是同一根 `Arc`。

宿主在第一次 `App::update` 之前安装 Bevy `TimePlugin`（或包含它的 plugin group），并插入一个 `LaneFlowSession`。本 crate 不修改 Bevy `Time<Fixed>`，也不替宿主重复安装时间插件。固定步进仍由交通运行时执行；Adapter 按自己的 accumulator 决定一次外帧里步进几次，再提取已经提交的位姿。

交通需求、路线选择和引擎生命周期属于宿主。无窗口接入不需要本 crate，见[从源码接入无窗口交通运行时][source-integration]。

## 最小使用入口

```rust
use std::num::NonZeroU32;

use bevy_app::App;
use bevy_time::TimePlugin;
use laneflow_bevy::{LaneFlowPlugin, LaneFlowSession, LaneFlowSessionConfig};
use laneflow_runtime::TrafficWorld;
use laneflow_spatial::SpatialSession;

fn install(app: &mut App, world: TrafficWorld, spatial: Option<SpatialSession>) {
    let config = LaneFlowSessionConfig::new(NonZeroU32::new(8).expect("non-zero"));
    app.add_plugins((TimePlugin, LaneFlowPlugin));
    app.insert_resource(LaneFlowSession::new(world, spatial, config).expect("paired session"));
}
```

可视示例从 `quickstart` 开始。无窗口交通从 `laneflow-runtime` 的 `headless` 示例开始。

## 主要约束与限制

- `LaneFlowFixed` 按 session 的累积时间运行零次或多次 `TrafficWorld::step`。`LaneFlowFixedSet::{Lifecycle, Step, Observe}` 是每个固定步里的公共阶段。
- `extract_committed_pose_batch` 在同一次调用里采集来源、核对根配对并提取。失败时输出保持原样。成功切换会使先前批次的消费上下文过期。
- `extract_selected_committed_pose_batch` 只提取调用方选中的句柄。`Completed` 和虚拟停泊被省略。重复、失效或过期上下文会整批拒绝。
- `replace_completed_vehicle` 在生命周期边界原子替换，已绑定车辆复用同一个 Entity。暂时不能接纳时不写 `last_error`，映射和变换保持不变。
- 停车命令经 `LaneFlowWorldMut` 转发到运行时。虚拟停泊没有位姿，但映射仍然活着。只有 `despawn_vehicle` 成功才移除映射。
- `LaneFlowSession::{cross_revision_cutover, same_revision_restore}` 是同步的维护暂停式切换，不是低停顿在线切换。`LaneFlowTargetSpatial::Headless` 是显式改成无 Spatial 的唯一形态。
- 走廊示例的 `toml` 和 smoke 用的 `bevy_math` 是 dev-dependency，不进入 `laneflow-bevy` 的生产依赖图。

阶段顺序、批量提取和切换的完整合同见[Bevy 设计][bevy-reference-adapter]。历史 debug gizmo 占位 API 已删除（[#473][github-com-illusion-tech-laneflow-issues-473]）。

## 示例与验证

六个示例的输入来源并不相同：

- `quickstart` 消费检入的编译器测试夹具 LFCA（带策略的最小空间制品），自己注册路线，不经 catalog。
- `corridor_traffic` 与 `junction_debug` 消费检入 LFCA 和各自 catalog。
- `parking_lifecycle` 与 `revision_cutover` 在进程内编译路网，不是读取检入 LFCA。
- `culling_extraction` 在启动时生成路口网格。

| 示例                 | 作用                         | 行为边界                                                                         |
| -------------------- | ---------------------------- | -------------------------------------------------------------------------------- |
| `quickstart`         | 最小接入、车辆绑定与位姿同步 | 两辆车，有限路线，完成后不回流                                                   |
| `corridor_traffic`   | 持续车流与宿主生命周期       | 默认 `100` 车，支持 `50..=200`，完成后在同一 Entity 回流                         |
| `junction_debug`     | 复杂路口领域观测与可视诊断   | 四车确定性场景，F2 开关只读 overlay（mesh/文本）                                 |
| `culling_extraction` | 按视野选择提取与隐藏         | 启动时生成路口网格，默认 9 格约 2,800 车，支持 `1..=32` 格，车辆跑完路线后不回流 |
| `parking_lifecycle`  | 预留、停入、离场与隐藏表现   | 环路上 6 辆车按固定脚本运行，含过早停入、离场失败和真正移除                      |
| `revision_cutover`   | 运行中切换路网修订           | 按 `C` 在两个修订间切换，环路车保持身份，含未清场回切被拒、清场和重试            |

`quickstart` 与 `quickstart_smoke` 共用 `examples/support/quickstart_scene.rs`，显式绑定
`runtime-fixture-policy / fixture-policy`，再创建车辆和配对 Spatial session。
相机与灯光显示同根 Spatial 中心线生成的车道和两辆车；浅色车头标记沿车辆前方向。
`quickstart_presentation.rs` 是 GUI 与无窗口测试共用的车辆绑定、整批提取与同步路径：
所有车辆使用 Session 映射，按批次句柄同步位置与朝向，整批预检失败时保留旧变换并记录
结构化错误。无窗口测试验证全部车辆的位姿，以及缺失 Transform、无 Spatial 的失败路径。

`corridor_traffic` 默认 `100` 车、seed `0`，支持 `--vehicles 50..=200` 与十进制 `u64` 的
`--seed`。全部车辆与独立 proxy 绑定，完成后按每拍 Lifecycle → Step → Observe 在同一 Entity
回流。灯具读取运行时已提交信号组，HUD 读取同帧人口与 accumulator 摘要。鼠标滚轮缩放、左键平移、右键绕焦点旋转。

`junction_debug` 使用检入 catalog 0.1 与 LFCA 的四车确定性场景。默认关闭的 F2 overlay 只读
`LaneFlowSession`，并写入独立 ECS 实体，不触碰运行时状态。开关 overlay 时的运行时摘要一致性由无窗口 smoke 对拍。

`culling_extraction` 在提取前用运行时来源和车道分块判断视野，只提取并写入选中车辆，缺席车辆隐藏但保留绑定；每 60 帧与全量提取逐值对照。按 `F` 冻结选择视域后缩小，可以看到被隐藏的车辆。HUD 的耗时只供观察，不是性能证据。

`parking_lifecycle` 的可见性由车辆状态、停车绑定和位姿来源决定。虚拟停车的车辆隐藏但保留映射，只有 `despawn_vehicle` 成功才销毁 Entity。HUD 显示每个角色的阶段和最近一次被拒绝的命令。

`revision_cutover` 在 `LaneFlowOuterFrameSet::Administration` 中切换。成功时复核旧批次、交付事件、交还旧 Spatial 并重建路面。路线还引用支路时回切会被拒绝，宿主先移除车辆再删除路线，下一帧重试。有车越过目标边长时，等它驶离后重试。

```powershell
cargo +1.99.1 run --locked -p laneflow-bevy --example quickstart --features native-example
cargo +1.99.1 test --locked -p laneflow-bevy --test quickstart_smoke
cargo +1.99.1 run --locked -p laneflow-bevy --example corridor_traffic --features native-example -- --vehicles 100 --seed 0
cargo +1.99.1 test --locked -p laneflow-bevy --test corridor_traffic_smoke --jobs 2 -- --test-threads=2
cargo +1.99.1 test --locked -p laneflow-bevy --example corridor_traffic --features native-example --jobs 2 -- --test-threads=2
cargo +1.99.1 check --locked -p laneflow-bevy --example quickstart --example corridor_traffic --features native-example
cargo +1.99.1 run --locked -p laneflow-bevy --example junction_debug --features native-example
cargo +1.99.1 test --locked -p laneflow-bevy --test junction_debug_smoke
cargo +1.99.1 run --locked -p laneflow-bevy --example culling_extraction --features native-example
cargo +1.99.1 test --locked -p laneflow-bevy --test culling_extraction_smoke
cargo +1.99.1 run --locked -p laneflow-bevy --example parking_lifecycle --features native-example
cargo +1.99.1 test --locked -p laneflow-bevy --test parking_lifecycle_smoke
cargo +1.99.1 test --locked -p laneflow-bevy --features native-example --example parking_lifecycle
cargo +1.99.1 run --locked -p laneflow-bevy --example revision_cutover --features native-example
cargo +1.99.1 test --locked -p laneflow-bevy --test revision_cutover_smoke
cargo +1.99.1 test --locked -p laneflow-bevy --features native-example --example revision_cutover
```

完整 C01–C13 验收、确定性出图与独立无窗口摘要对照见
[Bevy 设计 §15][bevy-reference-adapter-15-corridor_traffic-验收矩阵]。

## 深入阅读

- [从源码接入无窗口交通运行时][source-integration]
- [Bevy 设计][bevy-reference-adapter]
- [当前能力与限制 · Bevy 接入][capabilities-bevy-接入]
- [`laneflow-runtime`][laneflow-runtime-readme]
- [`laneflow-spatial`][laneflow-spatial-readme]

[source-integration]: ../../docs/guides/source-integration.md
[bevy-reference-adapter]: ../../docs/design/bevy-reference-adapter.md
[github-com-illusion-tech-laneflow-issues-473]: https://github.com/illusion-tech/laneflow/issues/473
[bevy-reference-adapter-15-corridor_traffic-验收矩阵]: ../../docs/design/bevy-reference-adapter.md#15-corridor_traffic-验收矩阵
[capabilities-bevy-接入]: ../../docs/reference/capabilities.md#bevy-接入
[laneflow-runtime-readme]: ../laneflow-runtime/README.md
[laneflow-spatial-readme]: ../laneflow-spatial/README.md
