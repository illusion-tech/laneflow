# Bevy Reference Adapter

**文档状态**: Accepted

**最后更新**: 2026-10-24（`junction_debug` 模块拆分与 overlay 出图开关）

**适用范围**: Bevy 0.20 Reference Adapter、headless 集成验证、可选调试可视化与最小 native example

**关联文档**:

- `../architecture.md`
- `../roadmap.md`
- `../adr/0001-project-scope.md`
- `../adr/0013-engine-neutral-spatial-geometry-and-length-authority.md`
- `../adr/0015-bounded-f32-canonical-spatial-frames.md`
- `../adr/0016-scenario-population-and-recycle-lifecycle-authority.md`
- `adapter-api.md`
- `spatial-geometry.md`
- `example-scenarios.md`
- `traffic-runtime-shared-consumption.md`
- `../reference/validation-matrix.md`

## 1. 目标

`laneflow-bevy` 是 LaneFlow 的首个 Reference Adapter。它用一个真实 Rust 游戏引擎验证 fixed tick 调度、车辆与实体生命周期、批量位姿到宿主 Transform 的转换、调试可视化和最小可运行示例，但不改变 Runtime 或 Spatial 的权威职责。#301 后 Session 安装 `TrafficWorld` 与可选 `SpatialSession`；`CoreWorld` 与 current JSON 入口已拆除。历史 campus / 走廊 Core 示例不再是现行证据。

v0.7 的完成目标是提供一条可构建、可测试、可演示且默认依赖面受控的 Bevy 集成路径。Bevy 不是跨 ABI、跨语言兼容性的唯一证明。

## 2. 版本、工具链与依赖边界

- 当前只支持 Bevy `0.20.x`；实际 patch 由仓库 `Cargo.lock` 固定。Bevy 伞形 crate 与
  子 crate 版本严格绑定，所有 Bevy 系依赖必须整体升级，同一依赖图不得出现两条 Bevy
  版本线。
- #912 迁移审计：Adapter 与 example 源码无需改动即在 0.20 编译、通过 clippy 与全部
  headless 测试；`junction_debug` 的 `persp` / `close` / `signals` 预设出图人工核对
  路面、标线、灯具自发光、大气与阴影无回归。项目未使用屏幕空间透射，0.20 改为
  opt-in 不改变画面。生产 modular feature graph 与 `native-example` 的十个伞形
  feature 名均未变化。
- LaneFlow workspace MSRV 为 Rust `1.99.1`（#495 提升到 1.98，#994 更新到 1.98.1，#1003 更新到 1.99.1；不跟随开发机静默变化）。Bevy 0.20.0 的 MSRV 为 Rust 1.97.1，不构成 LaneFlow 的工具链下限约束。
- Bevy 0.20.0 使用 `MIT OR Apache-2.0`；#912 以实际 lock graph 运行 cargo-deny
  `--all-features`，advisories、bans、licenses、sources 全部通过，并删除迁移后不再
  出现的 render 路径许可证例外。
- #171 的 Adapter 性能 Gate 数字是 Bevy 0.19 campus 路径的历史证据；#912 未在固定
  机重测，不把它当作 0.20 的性能结论。
- production `laneflow-bevy` 直接依赖最小 modular crates：`bevy_app`、`bevy_ecs`、`bevy_time` 与 `bevy_transform`。LaneFlow 生产依赖只有 `laneflow-runtime` 与 `laneflow-spatial`；parking target/record 由 Runtime 再导出，不把 `laneflow-static-contract` 写入 Adapter 生产 graph。
- production manifest 对四个 modular crates 关闭 default features；`bevy_app`、`bevy_ecs`、`bevy_time` 只启用 `std`，`bevy_transform` 启用 `std + bevy-support`。默认 graph 不激活 Bevy reflect、async executor 或 backtrace。
- 默认 production feature graph 不包含 umbrella `bevy`、renderer、window、audio、asset、scene、UI、Gizmos，以及没有被实现证明必要的 reflect/state/input。
- 完整 `DefaultPlugins`、render/window、mesh/material 和 Gizmos 只能进入显式 opt-in feature 或 example 边界。

## 3. 权威职责与 Session 边界

v0.7 每个 Bevy `App` 只支持一个活动 `LaneFlowSession`。Session 可以组合或持有：

- `TrafficWorld`；
- 可选 `SpatialSession`；
- 已提交的 pose batch；
- 可复用的 extraction、validation 与 Transform staging scratch；
- 当前 canonical frame placement 与 token；
- Adapter-owned Vehicle/Entity 映射。

组合不改变权威职责：

- Runtime 决定 fixed quantum、tick index、simulation time、车辆、路线、信号与停车状态。
- Spatial 决定 canonical frame、中心线、弧长、绑定和 canonical pose。
- Bevy Adapter 决定 schedule 集成、Entity、local Transform 与 frame placement。
- Presentation 决定模型、材质、动画、可见性、LOD、pooling、debug draw 与示例 UI。

宿主 Transform、插值结果、可见性或 LOD 不得反写 Core progress、status、occupancy、route、事件或 Spatial geometry。

## 4. Fixed schedule

Bevy 拥有 outer frame 与宿主 schedule。LaneFlow 不修改宿主全局 `Time<Fixed>`，而是提供 LaneFlow 专用 fixed schedule：

1. 每个 outer frame 读取宿主提供的 frame delta。
2. 使用整数毫秒 `Duration` accumulator 累加时间。
3. 当 accumulator 足够一个 LaneFlow fixed quantum 时，运行一次 LaneFlow fixed schedule。
4. 单个 outer frame 的 catch-up step 数有可配置上限。
5. 达到上限后保留 backlog；不得静默丢弃 simulation time。
6. 每次 Core step 成功提交后，才允许读取 snapshot/events 并构造 presentation 输入。

相同初始状态、输入和总 elapsed time 在不同 outer-frame 分块下必须产生相同 Core tick/state。presentation 提交仍受每个 outer frame 最多一次批量 extraction/apply 的限制。

#169 的具体 Bevy 映射为：

- `LaneFlowPlugin` 安装 `LaneFlowOuterFrame` 与 `LaneFlowFixed` 两个单线程 schedule。
- `LaneFlowFixedSet::{Lifecycle, Step, Observe}` 在每个 fixed step 内按该顺序执行；同一 outer frame 的每个 catch-up step 都重复完整链。调用方把 replacement policy system 放入 `Lifecycle`，把 committed result/event 消费放入 `Observe`。
- `LaneFlowOuterFrameSet::{Administration, Drive}` 在每个 outer frame 内按该顺序执行（#534）。`Administration` 每 outer frame 必运行、与 accumulator/fixed step 无关：维护暂停式切换等行政操作放在这里在结构上零步进可达——`LaneFlowFixed` 整个 schedule 受 `can_step()` 门控，零步进帧连 `Lifecycle` 都不运行，行政操作不得依赖 fixed step 的存在。
- **公开 set 无 Session 安全不变量**：`Administration` 以 `resource_exists::<LaneFlowSession>` 条件门控；Session 未插入或暂时移除的帧，宿主行政系统被跳过而非 panic。今后任何加入 `LaneFlowOuterFrame` 的公开 set 都必须维持该不变量（配套回归测试见 `revision_cutover.rs`）。
- `LaneFlowOuterFrame` 插入宿主 `First` 之后，因此读取的是本帧已经由 `TimePlugin` 更新的 `Time::delta()`；调用方负责安装 `TimePlugin` 或包含它的宿主 plugin group。
- `LaneFlowPlugin` 不重复安装 `TimePlugin` 或 `TransformPlugin`。缺少 Session 时 schedule 无操作；存在 Session 但缺少 `Time` resource 时记录结构化错误。
- `LaneFlowSessionConfig` 要求调用方显式提供非零 `max_catch_up_steps`，不定义隐藏默认值。
- accumulator 保存完整 `Duration`，不按 outer frame 截断亚毫秒余量；只有 Core step 成功后才扣除一个 quantum。达到上限或 step 失败时，当帧停止并保留全部 backlog。
- `LaneFlowFrameReport` 公开 frame delta、成功 step 数、backlog 与 catch-up-limit 状态；Session 保留当帧全部成功 `StepResult` 和最近 `LaneFlowAdapterError`。
- #285 阶段一新增 `LaneFlowSession::junction_observation` 只读借用视图（设计见 `junction-observation-and-validation.md` §3）：在 `Observe` 阶段逐拍读取复杂路口领域的边界上下文（世界身份/世代/修订/三游标）、live 车辆行、Waiting 计数/member/决策、Conflict 决策/预约与 transition event，并转调 `TrafficWorld::route_gate` 机动门定位。视图创建为 O(1)、无堆分配，headless Session 同样可读；信号组仍经 `committed_signal_groups()` 取得并单独计账。

## 5. Vehicle 与 Entity 映射

Adapter 维护 `VehicleHandle <-> Entity` 的部分双射：

- 已绑定记录必须严格一一对应；同一 Vehicle 绑定多个 Entity、同一 Entity 绑定多个 Vehicle 或重复 bind 都是结构化错误。
- 映射可以只覆盖 Core 车辆子集。未绑定车辆用于 LOD、streaming、pooling 或尚未实例化状态，是正常情况。
- 已登记但 Entity 已失效的 stale mapping 是错误；不得把它当作未绑定静默跳过。
- Adapter 提供 bind/unbind/rebind 边界，但不冻结宿主 bundle、prefab、model spawn、pool 或 despawn 类型。
- stable pose/input order 由 committed Core snapshot 和 Spatial batch 提供，不依赖 ECS query、HashMap 或 Entity iteration order。

Presentation 可以自行创建或回收模型 Entity。用于接收 LaneFlow pose 的 proxy Entity 与模型 Entity 可以分层，模型轴向、尺寸与 pivot 修正只放在 proxy 下的 presentation child。

#170 冻结并实现的公开边界为：

- `LaneFlowSession::{bind_vehicle_entity, unbind_vehicle, vehicle_entity}` 是映射的写/查入口；bind 只接受当前 `TrafficWorld` 中存在的 vehicle。重复 bind（含完全相同的 Vehicle/Entity）是结构化错误。
- #475 的 `replace_completed_vehicle(&mut World, old, VehicleSpawnInput)` 是 replacement 唯一公共组合事务；成功时已绑定 vehicle 复用同一 Entity 并原子轮换到 new handle，未绑定 vehicle 继续保持未绑定。`Blocked` 时映射与 Transform 不变。不得经 `world_mut()` 直接调用 `TrafficWorld::replace_completed_vehicle`。

## 6. Canonical frame 与 Bevy Transform

每个活动 canonical frame 使用一个 Bevy frame-root Entity，所有 LaneFlow vehicle proxy 是该 root 的 child：

- frame-root local/global Transform 表达 canonical frame 在宿主场景中的刚性 placement。
- vehicle proxy 的 local Transform 直接表达 LaneFlow canonical pose。
- root scale 必须为 `Vec3::ONE`；非单位缩放会改变 LaneFlow meter 语义，因此拒绝。
- `1 LaneFlow meter = 1 Bevy unit`。
- LaneFlow 与 Bevy 均使用右手、Y-up；位置不需要交换手性或上轴。
- LaneFlow tangent 映射为 Bevy forward，即 `Transform::forward()` 的 `-local_z`；canonical up 映射为 Bevy up。
- 模型自身的 forward/pivot/尺寸差异只能由 presentation child 修正。

同一 frame 被重新放置、切 tile、rebase 或替换 root 时，Adapter 必须颁发新的 `FramePlacementToken`。旧 token 的 batch 不得在新 placement 下提交。

具体 API 使用 `LaneFlowFramePlacement::new(root, token)` 与 `LaneFlowSession::set_frame_placement`。完全相同的 placement 可幂等重设；token 不变但 root 变化会被拒绝。`clear_frame_placement` 只清除 placement，不隐式清空 Vehicle/Entity 映射；存在映射而没有 placement 时，presentation 返回结构化错误。

> **实现状态（#534 核对）**：本段的 placement 管理 API 与下节的 presentation
> 提交系统是 v0.7 历史冻结的规范目标，不是当前 baseline 的已交付能力——
> `LaneFlowFramePlacement` / `set_frame_placement` / `clear_frame_placement`
> 与 exclusive `PostUpdate` 提交系统均不在当前导出面。现行已交付的位姿消费路径
> 是 §16 的 `extract_committed_pose_batch` 受控提取（宿主自持
> `FramePlacementToken`）。恢复完整 placement/presentation 系统需要独立 Issue，
> 不由 #534 承载。

frame-root 和 proxy 的 local `Transform` 必须有限，frame-root 必须为单位缩放，proxy 必须是当前 root 的直接 `ChildOf`。root 可以通过自身 local/global transform 放置到宿主世界，proxy local transform 始终保留 canonical meter 语义。

v0.7 不支持一个 `App` 中的多活动 Session、多活动 canonical frame 或车辆跨 frame 迁移。这些能力需要独立设计和生命周期协议。

## 7. 批量提取与原子 Transform 提交

每个 outer frame 在完成零次、一次或多次 LaneFlow fixed step 后，最多执行一次 presentation extraction/apply：

```text
committed Core snapshot/events
  -> stable PoseInputRecord sequence
  -> SpatialRegistry::extract_pose_batch
  -> Adapter validation/staging
  -> Bevy local Transform commit
  -> Bevy transform propagation
```

Bevy Transform 写入系统运行在 `PostUpdate`，并位于 `TransformSystems::Propagate` 之前。

提交采用两阶段处理：

1. 在可复用 scratch 中验证 batch frame、placement token、稳定映射、所有有限值、Entity 存活与转换后的 Transform。
2. 只有全部已映射记录通过后，才把 staged local Transform 写入 ECS。

错误语义：

- 任一已映射记录失败时，所有目标 Transform 保持进入本轮前的值。
- 未绑定记录是允许的，按稳定顺序跳过，不使批次失败。
- duplicate、registered-stale、frame/token mismatch、non-finite 与无效旋转均为结构化错误。
- 默认同步路径不得对每辆车调用 Spatial 单记录查询。
- 稳定容量下必须复用 extraction、validation 与 Transform staging 内存。

Presentation 从 committed `TrafficWorld::committed_pose_sources()` 重建 pose inputs：Active vehicle 使用当前 route edge 与 progress；explicit Parked 使用已提交 `ParkingSpace` pose；virtual Parked 与 Completed 不进入 presentation batch。batch 同时包含已映射和未绑定 vehicle，映射查询不会改变 record 顺序。无 pose 不能被解释为 removal。

`LaneFlowPlugin` 安装 exclusive `PostUpdate` 系统并显式排序在 `TransformSystems::Propagate` 前。实现先完成 Spatial batch、frame/token、root、所有 mapped Entity/parent/Transform 与转换结果校验，再统一写入 ECS；exclusive system 内两阶段之间没有其他 system 可以修改 Entity。`LaneFlowPresentationReport` 暴露 `pose_records`、`mapped_records`、`unbound_records` 与 `applied_records`，失败时 `applied_records` 恒为零，具体失败保存在 Session 的最近错误中。

上两段同属 v0.7 历史冻结的 presentation 规范目标（见 §6 实现状态说明），不是当前
baseline 的已交付能力；`quickstart` / `corridor_traffic` 现行示例由宿主系统
在 `Observe` 后调用受控提取并自行写入 Transform。

## 8. 可选调试可视化

#285 阶段二以 `junction_debug` native example 交付了可见诊断（[#285](https://github.com/illusion-tech/laneflow/issues/285)）：
默认关闭的 F2 调试 overlay 全部用 Bevy mesh 与文本落地，不恢复 Gizmos 公共 API，也不
为恢复历史 Gizmos API 增加兼容层。v0.7 / #172 的预算受控 gizmos 与 campus /
`debug_gizmos_smoke` 只存在于 git 历史。

overlay 内容与纪律（设计 §4，`junction-observation-and-validation.md`）：

- 静态 Gate 标记来自 `route_gate` / `ManeuverGateView` + 停止线定位；多次经过同一
  Gate 的画面去重仅限标记，不合并各自 route/hop 的决策与预约。
- Waiting 区间用 entry→release Gate 之间的规范路径边子段 ribbon 表示，不新造
  Waiting polygon authority；Conflict 区域用 `ConflictZoneRegion` 的 `ring_xz`
  描边 mesh，缺少区域几何时显示带身份的文字标记，不计算几何相交补冲突事实。
- 选定车辆面板（bevy_ui 文本）每拍在 `LaneFlowFixedSet::Observe` 消费
  `junction_observation`：机动遍历阶段、最近 Waiting/Conflict 决策 outcome 与
  NoGrant 归因、当前 `ConflictReservation` 摘要、未来 Gate route/hop 列表；按车
  聚合时对每拍决策批次建一次索引。bevy 内嵌字体仅覆盖 ASCII，面板文本使用精确
  英文标识符。
- 信号状态点用 `committed_signal_groups()`，其拥有式 materialization 的分配与
  耗时单独计账，不因观测视图零分配就声称整个观测链零分配。
- 静态绘制缓存以（路网修订、世界世代）为键，换根或换世代后重建；旧世代的动态
  标记不覆盖当前状态。overlay 全部只读 `LaneFlowSession` 并写独立 ECS 实体；
  默认调试关闭，开/关与呈现比例在相同命令输入下不改变 Runtime 状态与事件摘要
  （无窗口 smoke 以 `deterministic_state_digest` + 每拍 transition/决策摘要
  对拍把关，见 `examples/support/junction_debug_smoke.rs`）。

默认 production graph 仍不得包含 Gizmos/render/window；`native-example` 仍是非默认
opt-in。渲染、文本与模型的耗时不得混作 Runtime 性能。

## 9. 最小 native example

现行最小 native example 是 `quickstart`：从带策略的 compiler 空间夹具安装
`TrafficWorld` 与配对 `SpatialSession`，以相机、灯光和同根 Spatial 中心线生成的
车道 ribbon 展示两辆车。所有车辆通过 `bind_vehicle_entity` 绑定；宿主按
`vehicles()` 与 pose records 的对应顺序同步位置和朝向，Bevy forward 对应切向量。
模型尺寸、抬升与车头标记位于代理的表现子实体，不修改规范位姿。

GUI 与 `quickstart_smoke` 共用 `quickstart_scene.rs` 初始化和
`quickstart_presentation.rs` 绑定/同步路径。同步在 `PostUpdate`、Transform 传播前运行；
提取失败、绑定缺失、Transform 不可用或非有限变换都会记录结构化错误，并保留整批旧变换。
无窗口测试覆盖全部车辆的绑定、移动与完整朝向，以及失败时不产生半批更新。
本例不扩展 Runtime 或 Adapter API，也不承担持续人口回流或性能认证。

```powershell
cargo +1.99.1 run --locked -p laneflow-bevy --example quickstart --features native-example
cargo +1.99.1 check --locked -p laneflow-bevy --example quickstart --features native-example
cargo +1.99.1 test --locked -p laneflow-bevy --test quickstart_smoke
```

`native-example` 仍是非默认 opt-in，完整 `DefaultPlugins` / window / renderer 留在示例边界。v0.7 `native_reference`、campus JSON 与 `laneflow_data::from_scenario_json_slice` 已删除。现行走廊 native example 使用检入 catalog 0.4 与 LFCA，prepare 绑到已安装共享路网修订并 `register_route`；50–200 回流见 [#475](https://github.com/illusion-tech/laneflow/issues/475)。

#285 阶段二的复杂路口调试示例是 `junction_debug`（同样 `native-example` opt-in）：
检入 catalog 0.1 + LFCA 的固定四车 spawn 计划（直行、保护左转待转、许可左转、环路
重复过门），F2 调试 overlay 见 §8；无窗口 smoke 是 `junction_debug_smoke`。

```powershell
cargo +1.99.1 check --locked -p laneflow-bevy --example junction_debug --features native-example
cargo +1.99.1 test --locked -p laneflow-bevy --test junction_debug_smoke
```

道路表面由 `examples/support/junction_road.rs` 消费同修订车道几何生成：路口为连续
铺装，四角路缘与人行道相切；外围成对同向车道共用一幅路面，足宽处画分隔虚线，
直线汇合/分流处连续收窄，环路两侧均铺设人行道。内侧接外部道路的人行道，外侧经
中央分隔带末端的步行连接面衔接；主路端外侧步道收至 2 m，避免相邻两条步道重叠。
道路分隔带设圆头，过街区域保留连续铺装。外部道路边界按
世界横坐标去重，车道导向按同次穿越的进口与出口方向显示；表现不更改规范轨迹。

西进口的一条左转路径采用待转区。两侧白色短虚线从对应进口车道边界沿同修订
转入路径连续延伸至 release，线宽 0.15 m、实线段与间隔均为 0.5 m；箭头位于
entry→release 储车区间。连接、储车与驶出由同一条左转圆弧分段，储车段本身
随左转方向弯曲，不横移后拉直为 S 弯。标线按实际 WaitingZone 路径逐条生成，
不增加车道用途。
进口停止线和待转区前端停车线保留；内部 entry 准入门不再画成额外停车横线。
入区信号允许主路直行/左转绿灯通行，出区只在左转放行时通过。默认沿弧线 12 m 储车段
按 4.5 m 车长和 2 m 间距推导容量为 2 辆，连接段不增加储车容量；Runtime 仍检查
实际车长、车间距与物理占用。固定四车计划中仍只有一辆待转车；另以三车排队
回归验证两辆整车入区、第三辆区外等候及左转绿灯释放后继续准入。
时序与几何编制详见生成器 README，Adapter 不自行放行车辆。

`examples/support/junction_signals.rs` 为每个进口生成一套路口对侧悬臂灯架，共四套、
七组实体灯头，灯头朝向对应来车方向，包含立柱、基础、横臂、背板和遮光罩。
实体灯头由该进口各机动路径的最终放行信号绑定，待转路径取 release；共享相位
可驱动多个进口的实体灯，禁止取信号组首门作为唯一灯具位置。保护左转使用箭头灯，
带冲突通行段的许可左转使用圆灯配固定左转方向牌，避免误画为保护放行。

原生出图固定物理分辨率，并在明确的模拟 tick 暂停后捕获，避免不同 GPU 帧率
对应不同交通状态。预设为 `persp`、`topdown`、`close`、`merge`、`waiting`、`signals`；至少等待 150 个
渲染帧让管线就绪。`--at-tick` 默认为 6_100，出图与每帧耗时不属于性能证据。
图像转换或保存失败时返回非零进程退出码；成功退出才表示截图已保存。
`--overlay`（或环境变量 `JUNCTION_DEBUG_OVERLAY=1`）让 §8 调试 overlay 启动即开启，
出图包含诊断面板与静态标记；此后仍可用 F2 切换，默认关闭语义不变。

示例入口只做装配，职责按 `examples/support/` 模块划分：`junction_cli`（参数与环境
变量回退）、`junction_camera`（相机预设、天空与光照）、`junction_capture`（定 tick
出图）、`junction_road` 与 `junction_surface`（路面铺装与标线）、`junction_signals`
（灯具与灯态）、`junction_vehicles`（车辆表现与位姿同步）、`junction_overlay`
（§8 调试 overlay）。

```powershell
cargo +1.99.1 run --locked -p laneflow-bevy --features native-example --example junction_debug -- --screenshot junction-close.png --camera close --at-tick 3500
cargo +1.99.1 run --locked -p laneflow-bevy --features native-example --example junction_debug -- --screenshot junction-overlay.png --camera close --at-tick 3500 --overlay
cargo +1.99.1 run --locked -p laneflow-bevy --features native-example --example junction_debug -- --screenshot junction-merge.png --camera merge --at-tick 0
cargo +1.99.1 run --locked -p laneflow-bevy --features native-example --example junction_debug -- --screenshot junction-waiting.png --camera waiting --at-tick 1600
cargo +1.99.1 test --locked -p laneflow-bevy --features native-example --example junction_debug
```

## 10. 验证与性能 Gate

默认 headless tests 直接构造 Bevy `App` 并驱动 update，不依赖 window、renderer 或 OS event loop。必须覆盖：

- 0/1/多 fixed step、catch-up 上限和 backlog 保留；
- 相同总 elapsed time 的不同 outer-frame 分块；
- bind/unbind/rebind、partial mapping、duplicate 与 stale entity；
- Y-up、`-Z` forward、frame-root/child placement 与 rebase；
- frame/token/finite/mapping/entity first-error 与整批失败原子性；
- campus artifacts 的 load → Core step → Spatial batch → Bevy apply E2E；
- default feature graph、feature-on compile、MSRV、workspace tests 与 cargo-deny `--all-features`。

固定 Windows 性能机的 production release Gate 计量：

```text
Spatial batch extract
  + Adapter validation/mapping
  + Bevy local Transform write
  + Bevy transform propagation
```

该边界不包含 Core 交通求解和 renderer。冻结门槛为：

- 稳定容量下，一万/十万 Adapter-owned batch 路径零 allocation/reallocation；
- 一万 p95 不超过 `4 ms`；
- 十万 p95 不超过 `40 ms`；
- 一万到十万的扩展不超过 `12x`。

共享 CI 运行 correctness、determinism、allocation、workspace/MSRV、example/benchmark compile 与 dependency policy。绝对 wall-clock Gate 只在记录了机器、source commit、命令、样本和后台负载的固定环境运行，不作为跨平台 SLA。

#171 已在 source commit `d7e8b1e` 上固化该边界：稳定容量的一万/十万完整 `PostUpdate` 均为零 allocation/reallocation；Rust 1.96 固定机五轮 p95 中位数为 `3.067 ms` / `35.852 ms`，扩展 `11.691x`。逐轮协议见 git 历史。该结果是当时 Bevy 0.19 campus 路径证据，不是 #301 后 `TrafficWorld` 的产品 SLA。

## 11. 执行切片与 PR 角色

| Issue | 交付切片                                               | 直接前置            |
| ----: | ------------------------------------------------------ | ------------------- |
|  #169 | 最小 crate、Plugin/Session 与 fixed schedule（已完成） | 无活动 blocker      |
|  #170 | Vehicle/Entity 映射与原子批量 Transform（已完成）      | #169（已完成）      |
|  #171 | headless E2E、allocation/performance 与 CI（已完成）   | #170（已完成）      |
|  #172 | 可选、预算受控的 debug Gizmos（已完成）                | #170（已完成）      |
|  #173 | 最小 native reference example（已完成）                | #170（已完成）      |
|  #174 | 最终集成文档与独立 closure review                      | #171-#173（已完成） |

每个子 Issue 使用自己的唯一 Delivery PR。#169-#173 的 PR 对父 #121 只使用 Related PR 语义，不得以 closing keyword 覆盖父 tracker。#174 的最终 integration PR 同时作为 #174 与 #121 的唯一 Delivery PR；所有子 Issue G4 后才允许 #121 进入最终 G3/G4。

#174 的独立审阅没有发现需要回开生产实现、Adapter API 或 CI 契约的缺口。当时完成记录见 git 历史。本节的剩余项继续是明确的后续非目标。

## 12. 兼容性与后续演进

本设计不改变 Core API、current data format 或 Spatial public authority。它新增 Bevy-specific Adapter API；在 v1.0 前仍可按独立 Issue 演进，但以下变化必须重新进入 G1：

- 迁移到 Bevy 0.21 或同时支持第二个 Bevy major/minor line；迁移 Issue 必须重新审计
  API、feature graph、MSRV、许可证、RustSec 与 example 画面；
- 修改 default dependency/feature graph；
- 修改 fixed schedule ownership、time-drop policy 或 authority；
- 放宽为多 Session、多 canonical frame 或跨 frame migration；
- 改变 mapping 一一性、batch 稳定顺序、placement token 或失败原子性；
- 让宿主类型、Transform 或 Gizmos 进入 Core/Data/Spatial 公共接口。

插值、LOD/pooling、prefab/glTF、WASM、第二个 Engine Adapter 和 foreign-host boundary proof 保持后续独立范围，不是 v0.7 完成条件。

## 13. corridor_traffic 宿主生命周期

### 13.1 现状、目标与权威

本节至 §15 定义 `corridor_traffic` 的宿主契约与验收边界。该示例由
`examples/support/corridor_scene.rs`、`corridor_host.rs` 与独立 GUI 模块组合；
全部人口使用一对一 proxy、逐拍回流与整批位姿同步。`corridor_traffic` 已替换旧走廊
Cargo target、源码、关联 smoke 和入口文档，不保留别名或第二套持续车流示例。
最小可见入口仍为 `quickstart`；本节不把该最小示例或历史验证算作持续车流证据。

`laneflow-scenario` 已拥有 catalog 0.4、`50..=200` 人口、seed、PRNG、冻结回流计划与
FIFO retry；其权威仍在[人口设计](signalized-corridor-population.md)。Runtime 拥有
车辆身份和替换事务；Adapter 拥有 Session、固定调度及 Vehicle/Entity 映射。新增宿主
代码只在 `laneflow-bevy` example/support 边界组合这些能力，不能进入 production
依赖或公共 API。公开 API 若出现缺口，另立任务，不在示例里复制 Runtime 规则。

### 13.2 启动与一对一绑定

GUI 和 headless 使用同一个 example-local bootstrap，顺序为：

1. 解析 §14.1 参数；从检入 LFCA 经现行受检 loader 构建共享路网修订，解析 catalog 0.4。
2. `bind` catalog 到该修订，解析车辆 profile；以 config、bound catalog、共享路网修订
   和 profile 调用 `CorridorPopulationPrepare::prepare`。此时尚未注册世界局部路线。
3. 使用 bound catalog 的显式 `policy_selection` 安装唯一 `TrafficWorld`；容量必须
   容纳目标人口与全部 catalog 路线，fixed quantum 为 `16 ms`。
4. `spawn_initial_vehicles` 在同一世界注册路线并生成初始人口，再把返回的车辆和路线
   句柄交给 prepare 的 `bind`。tick 0 必须满足 `Running = target, Pending = 0`。
5. 将此世界与绑定同一根 `Arc` 的 `SpatialSession` 组成 `LaneFlowSession`，显式设置
   `max_catch_up_steps = 8`；建立 target 个 proxy，并逐一 `bind_vehicle_entity`。
6. 完成静态几何/灯具绑定、初始位姿预检及宿主资源安装后，才允许首个 fixed step 和
   对外呈现。初始人口错误在启动窗口前返回；ECS/渲染启动错误也不得放行首拍。

任一步失败都结束这次启动，丢弃尚未发布的世界/controller；保留原始错误，不把部分
回滚失败当作空世界重试。controller、世界、Session 与 proxy 集合具有同一宿主生命
周期；可复制的修订/策略值不是世界实例令牌，不能拿另一世界的同值上下文代替。
本示例不执行存档恢复、换根或运行时调整人口。

### 13.3 每拍回流顺序与结果转换

`LaneFlowPlugin` 是唯一步进驱动。宿主在每一次 `LaneFlowFixed` 中依次执行：

| 阶段        | 宿主动作                                                                                                                                            | 成功后的边界                                                                       |
| ----------- | --------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------- |
| `Lifecycle` | 从当前 Session 取修订/策略，调用 controller 的 `apply_pending`；回调调用自由函数 `laneflow_bevy::replace_completed_vehicle(&mut World, old, input)` | 每个进入本边界时已 pending 的计划最多尝试一次                                      |
| `Step`      | 由 Adapter 推进一个 `16 ms` Tick                                                                                                                    | 只有提交成功才扣除 accumulator、记录 `StepResult`                                  |
| `Observe`   | 恰好一次 `consume_world(session.world())`                                                                                                           | 消费该拍已提交世界，按稳定 live 顺序冻结新完成车辆的回流计划，留给下一 `Lifecycle` |

`Lifecycle` 使用 exclusive system，在取出人口 Resource 的作用域内访问同一个 Bevy
World；不能同时持有 Session 可变借用再调用组合事务。不得直接经 `world_mut()`
调用 Runtime replace，也不得用 despawn + spawn 拼接回流。

Adapter outcome 按原语义转换为 `CorridorReplaceAttemptOutcome`：

- `Replaced(record)`：核对 `old` 和本示例必有的 `entity`，以 `VehicleReplaceRecord`
  的 old/new 轮换 controller logical slot；原 Entity、模型 child 和最后 Transform
  保留，入口位姿由本帧后续 presentation 提交。
- `Blocked(block)`：入口重叠；原样传递 block，保留 Completed old、映射和 Transform。
- `Retryable(error)`：当前停车或前后车暂不能接纳；对应现有
  `StopConstraintUnsatisfiable`、`UnsafeLeader`、`UnsafeFollower`，转换为 policy 的
  `Retryable`。与 Blocked 一样移到 FIFO 队尾，不重抽、不降速、不改计划，不阻止
  其他 pending 计划。
- Adapter/Runtime 错误及 policy contract 错误：进入 §13.4；不得把
  `DownstreamSpeedUnsatisfiable` 或身份错误改报为可重试。

`consume_world` 接收的是逐拍已提交世界，不是 `frame_step_results().last()`。
它要求 `last_consumed_tick + 1`；同拍重复消费、跳过中间拍均为错误。零步进帧不运行
人口链；多步 catch-up 帧逐拍重复完整链，不能在帧末合并消费。Completed 没有 pose
record 不代表删除：其 Entity 和最后合法 Transform 必须一直保留至成功换绑。

### 13.4 失败停止边界

example-local 健康状态保留首个结构化失败（阶段、Tick、原始错误），不增加公共
`LaneFlowAdapterError` 变体，也不假定存在公开 Session error setter。分别为
`Lifecycle`、`Step`、`Observe` 增加读取该状态的 run condition：每个 set 到达时检查，
不能只在整个 fixed schedule 入口检查一次。

- `Lifecycle` 失败后，本拍 `Step`/`Observe` 和本帧后续 catch-up 不再提交。
- `Step` 的 Adapter 错误由现有 Session 守卫阻止 `Observe` 与后续步进。
- `Observe` 失败时，该拍 Runtime 已提交；不回滚它，也不继续后续 catch-up。
- presentation 失败拒绝本轮 Transform 整批提交，并阻止下一帧步进。

当前 Adapter 的 outer-frame 循环可能在 policy 失败后继续有限次检查空 schedule，
但不得再提交 Tick。仅成功的 Tick 扣时间；剩余 backlog 原样保留，不清 accumulator
或伪造成功报告。帧末统一读取 Session error 和宿主失败，输出诊断并以非零状态退出。
不得依靠异步 `AppExit` 单独阻止同帧 catch-up，也不得清错后继续演示。

事务原子性只覆盖单次 typed replacement。前序成功命令不跨命令回滚；callback 成功
后发现 old/new/entity 不一致时，世界可能已经变化，须停止并报告，不能再次替换同一
old。失败快照标记失败阶段，不作为满足人口不变量的成功 HUD/回放证据。

## 14. corridor_traffic 参数与表现

### 14.1 启动参数

唯一人口参数是 `--vehicles N`（整数 `50..=200`，默认 `100`）和 `--seed S`
（十进制 `u64`，默认 `0`）。`--help` 显示范围和默认值后成功退出；未知、重复、缺值、
溢出或越界参数在创建窗口及首拍之前失败，不静默 clamp。人口在一次运行中固定。

运行命令：

```powershell
cargo +1.99.1 run --locked -p laneflow-bevy --example corridor_traffic --features native-example -- --vehicles 100 --seed 0
```

沿用 checked-in LFCA、catalog 0.4 与 authoring config 0.2；不重生成场景或改变 PRNG、
槽位/路线顺序、权重和初始接纳规则。现有信号程序是 `12 phase / 84,064 ms`、offset
`[0, 42000] ms`，灯具必须消费 Runtime 提交状态，不能自己按墙钟模拟相位。

### 14.2 位姿、灯具与相机

宿主在 `PostUpdate`、`TransformSystems::Propagate` 之前最多提取/提交一次完整 batch：

1. 用 `extract_committed_pose_batch` 填充复用缓冲；验证消费上下文仍属当前世界，
   以及宿主自持的 `FramePlacementToken` 与当前 root 一致。
2. 对所有记录按车辆句柄找到已绑定 proxy；验证 Entity、父子关系、root 单位缩放、
   有限的 position/tangent/up 及转换后的 Transform，先全部暂存。
3. 整批通过后统一写入 local Transform；任一失败保持所有目标旧值。当前生产插件
   没有 §6–§7 的完整 presentation 系统，本例在 example-local 边界承担该提交，
   不恢复不存在的公共 placement API。

全部人口必须绑定，不接受第一条 pose 代表全部车辆或静默跳过缺失绑定。车头方向为
`Transform::forward()` 的 `-Z`，up 来自 canonical pose；proxy 原点是前保险杠，
车身 child 沿 local `+Z` 向后延伸，长度来自车辆 profile。不得从屏幕运动或两帧位置
差反推朝向，也不得把表现 Transform 写回 Runtime。

道路 mesh/标线来自同一 Spatial centerline，宽度取场景 authoring 配置的展示参数；
不从 mesh 推导车道长度或占用。灯具在启动时由 StopLine → ManeuverGate → SignalGroup
关系冻结绑定；共享 StopLine 只有在所有关联 Gate 指向同组时合并为一盏，缺失或歧义
使启动失败。每帧只按 `committed_signal_groups()` 更新对应灯具材质，steady path
不做字符串查找，也不凭路线或车辆状态猜灯色。

相机初始 focus 和可见范围从 Spatial bounds 派生，保证默认窗口能看到两个路口与
车辆；支持滚轮缩放、左键水平平移和右键绕 focus 旋转。未锁定指针的拖拽使用本窗口
逻辑像素增量，忽略其他窗口的输入。角度、距离限位保持有限值，按输入增量更新，
不能因帧率改变控制速度。模型、灯具、相机和 UI 全部只属于
example；不添加动态人口滑块、FPS/CPU 性能面板或新的 Adapter 调试 API。

### 14.3 HUD 与同帧摘要

在 outer-frame Drive 完成、最后一次 Observe 成功后建立一个只读帧摘要，HUD 和
headless 验证消费同一个摘要构造函数。零步进帧仍建立报告，人口计数沿用上次成功
消费结果。画面内使用 ASCII 标签（内嵌字体边界），窗口标题保持稳定：

| 字段                           | 唯一来源与显示口径                                                                                                    |
| ------------------------------ | --------------------------------------------------------------------------------------------------------------------- |
| `vehicles / running / pending` | controller 的 `counts().target/running/pending`；成功边界始终 `running + pending = target`，不数 mesh 或 pose records |
| `steps`                        | `LaneFlowFrameReport::steps_run()`，只计本 outer frame 成功提交的 Tick                                                |
| `backlog_ms`                   | `LaneFlowFrameReport::backlog()`；显示毫秒小数点后 3 位，比较使用原始 Duration                                        |
| `catch_up_limit`               | Session config 的上限 `8` 和 report 的 `catch_up_limit_reached()`；不能仅凭 `steps == 8` 猜测                         |
| `tick / seed`                  | 当前世界已提交 Tick 与本次启动 seed，辅助截图/回放定位                                                                |

摘要还保留 outer-frame 序号、输入 delta、simulation time 与信号组状态供验收定位；
它不驱动交通决策。HUD 不遍历 `pending_vehicles()` 分配临时列表，不维护第二套计数。
不把画面 FPS、构建耗时或短程运行当作性能结论。

## 15. corridor_traffic 验收矩阵

### 15.1 执行边界

下表是实现必须提供的验收，实际运行结果记录在交付证据中。headless harness 必须复用
§13 bootstrap、生命周期系统、presentation 和摘要构造，在真实 Bevy `App` 中安装
`TimePlugin`、`TransformPlugin`、`LaneFlowPlugin`，但不创建窗口或 renderer。几何/灯具
绑定、proxy Transform 和 HUD 文本可以无 GPU 核对，mesh/material/window 安装留在 GUI
分支；不能另写一套人口或计数系统来充当 headless。仅手写
`apply_pending → TrafficWorld::step → consume_world` 循环不能替代宿主调度验收。

测试以 `TimeUpdateStrategy::ManualDuration` 提供 frame delta；明确处理首帧初始化，
并控制虚拟时间上限，证明传给 LaneFlow 的实际 delta 等于测试输入。比较在相同已
提交 Tick 且 backlog 排空的边界进行；原始 Entity 数值不要求跨 App 相同，要比较
各 App 内 logical slot → Entity 的身份连续性。

| ID               | 输入/操作                                                                                                          | 可判定的通过条件                                                                                                                                |
| ---------------- | ------------------------------------------------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------- |
| C01 启动         | `50/100/200`，seed `0`、固定非零值及 `u64::MAX`；同参重建                                                          | tick 0 人口准确、pending 0、无重叠，世界路线/句柄合法，target 个不同 proxy 全部绑定；同参初始计划及 PRNG 状态相同                               |
| C02 拒绝启动     | `49/201`、缺值/重复/未知参数、seed 溢出；错误 catalog/策略/profile、不足槽位及接纳失败                             | 无首拍、非零退出；非法参数不创建窗口；回滚不完整保留原错误并丢弃未发布实例，不以残余世界重试                                                    |
| C03 固定调度     | 零 delta、亚毫秒余量、每帧 `1/4/8` 拍及超出 `8` 拍输入                                                             | 零拍不调用人口链；每个成功 Tick 恰好一次 Lifecycle/Observe；超限保留 backlog，后续零 delta 帧可排空；report 数值与实际提交一致                  |
| C04 完成与回流   | 默认夹具实际完成路线，观察 completion、pending 和下一边界 replacement                                              | completion 不在同一 Observe 内替换；成功后 old stale/new live，同一 logical slot 始终复用原 proxy/model；无 despawn + spawn                     |
| C05 可重试       | 构造入口 overlap 与三类 Retryable；同边界另有可成功计划，再解除约束                                                | old/mapping/Transform、冻结输入和 PRNG 不变；每计划每边界一次、其他计划继续；恢复后相同计划成功且人口不减少                                     |
| C06 失败边界     | Lifecycle host/policy 与 Observe 错误时仍有 catch-up 预算；另测 Drive 前的 `MissingTimeResource`                   | 首个错误可诊断且非零退出；Lifecycle/缺失 Time 错误无本拍提交，Observe 错误只保留已提交拍；剩余拍不提交、已有 backlog 不丢；前序成功替换不回滚   |
| C07 错误结果     | 测试替身返回不匹配 old、已跟踪 new 或错误 Entity；跨世界/策略上下文误接                                            | 前置不匹配不调用 callback；提交后不一致立即停止、不重试旧句柄；跳拍或重复 consume 明确失败                                                      |
| C08 分帧回放     | `100` 车 / seed 0、相同输入；真实 App 使用 `1/4/8` 拍、含零帧/余量/超限的分块，至少 `12,000` Tick                  | 每拍比较 logical 句柄/车辆状态、pending FIFO 与输入、PRNG、已消费 Tick、信号和 replacement 顺序；最终 Tick/time/backlog 一致且确有回流          |
| C09 有界持续运行 | `50` 和 `200` 车各运行，目标至少 `50` 次成功 replacement、上限 `50,000` Tick                                       | 每个成功 Observe 后 running + pending = target；pending 不超 target；controller 预留容器容量、车辆 proxy/model 数不增长；达不到成功次数明确失败 |
| C10 完整位姿     | 全人口、直行/左转/右转、Completed 等待；注入 stale Entity/token、非有限值或坏父子关系                              | 每个有效记录的平移/朝向/up 正确；前保险杠原点正确；Completed 保留旧值；错误时整批零写入，不只验证首车                                           |
| C11 信号         | 默认两个控制器运行至少 `5,254` 个 `16 ms` Tick，覆盖完整 `84,064 ms` 周期、回绕前后状态及各相位边界；缺失/歧义绑定 | 每盏灯与同 Tick committed group 一致，共享停止线不重复/误合并；错误绑定启动失败，外部墙钟与相机不影响信号                                       |
| C12 HUD          | 零拍、正常帧、超限、存在 pending、回流后帧                                                                         | UI 文本由同帧摘要格式化；数值与 controller/report 完全一致，Duration 仅在显示时舍入；错误帧不输出成功摘要                                       |
| C13 GUI          | Windows 默认 `100 / seed 0` 出图；平移/旋转/缩放，观察回流                                                         | 两路口、道路、完整车身、灯色和 HUD 可辨；朝向/前保险杠正确，Entity 连续；同帧截图与摘要核对，人工结论注明环境与限制                             |

C06 的 Adapter 故障使用现有公开 schedule 触发：先正常初始化 headless App，并以
超限帧建立非零 backlog；随后移除默认 `Time` resource，直接运行一次真实
`LaneFlowOuterFrame`，再调用同一宿主失败收集/退出路径。移除后不先调用
`App::update`，避免先执行 `TimePlugin` 的系统。断言 Session 报
`MissingTimeResource`、`steps_run = 0`、Tick/世界状态/人口与已有 backlog 不变，且
宿主记录失败并请求非零退出。这是 outer-frame 驱动错误，不是 Runtime `StepFailed`。

§13.4 的真实 Step 失败停止语义仍适用，但 C06 不要求外部 example 强制制造它：
Adapter 的 `step_world` 是私有实现且总是传入世界配置的 delta；Runtime 的
`StepFailpoint` 仅供 crate 内部测试。Step 事务原子性由 Runtime 内部故障测试验证，
不算作本 example 的端到端覆盖。本切片不新增测试钩子或公共 API；若后续要求真实
`StepFailed` 穿过 Adapter 到宿主的注入验收，须另立测试接口设计，不能用缺失 Time
或伪造错误枚举替代该路径的证据。

C07 的故障注入留在 example 测试边界，不增加生产 API。当前可复制修订/策略不能
识别同修订、同策略的另一世界，故此项须同时验证宿主一对一资源装配，不能宣称已有
不透明世界令牌自动拦截。C08 比较完整序列和末端，缺失、截短、失败或无实际回流
均不得以共同前缀相同判为通过。已有 scenario 的短 soak、PRNG golden 和 10,000 次
replacement ignored 测试继续作为底层证据，不冒充新增 Bevy 调度或 GUI 证据；
C09 是有界性验收，不是吞吐、堆峰值或长期稳定性认证。

### 15.2 复现与证据

共享宿主验收同时接入无 GPU 的 `corridor_traffic_smoke` 与 native example 测试；
后者还验证真实 `Text`、灯具材质和相机输入系统。两者不创建窗口或 renderer：

```powershell
cargo +1.99.1 test --locked -p laneflow-bevy --test corridor_traffic_smoke --jobs 2 -- --test-threads=2
cargo +1.99.1 test --locked -p laneflow-bevy --example corridor_traffic --features native-example --jobs 2 -- --test-threads=2
```

C13 的自动出图使用 example-local `corridor_replay.rs`：先输入 120 个零 delta 帧，
再循环微秒输入 `[0, 500, 15500, 128000, 128000, 160000, 0]`，在目标 Tick 截断总时间，
排空 backlog 后以零步进帧捕获。GUI 的 HUD 更新排在 UI layout 前；截图请求同时
核对实际 `Text` 和灯具状态，并写出同名 TOML 摘要。以下环境变量仅供验收出图，
不增加运行时人口参数；输出目录须已存在，默认目标为 `12000` Tick，上限 `50000`：

```powershell
$env:CORRIDOR_TRAFFIC_CAPTURE = 'C:\evidence\corridor.png'
$env:CORRIDOR_TRAFFIC_CAPTURE_TICK = '12000'
cargo +1.99.1 run --locked -p laneflow-bevy --example corridor_traffic --features native-example -- --vehicles 100 --seed 0
$env:CORRIDOR_TRAFFIC_REFERENCE = 'C:\evidence\headless.toml'
cargo +1.99.1 test --locked -p laneflow-bevy --test corridor_traffic_smoke c13_independent_headless_capture_reference -- --test-threads=2
```

默认 `100 / seed 0` 下，GUI 生成的 `corridor.toml` 应与独立测试的 `headless.toml`
逐字节相同；两者含 LFCA/catalog SHA-256、frame/Tick、原始 Duration、信号与 HUD 文本。
相机手工操作另行验证，截图不替代逐拍身份或事务断言。

验收记录必须列出实际 commit、工具链/平台、LFCA/catalog 标识、参数、frame-delta
输入和已完成用例。C13 的自动出图核对使用同一确定性 frame-delta 脚本分别驱动 GUI
与独立 headless App，保存截图帧的序号/Tick、摘要与显示文本；不能比较两个任意墙钟
时刻的画面。手工相机操作另记，不改变交通输入。截图只证明可见结果，C04–C10 的
身份/原子性/调度结论必须来自可断言的 headless 证据。

构建和测试串行执行；本地默认 Cargo jobs、测试线程和 Rayon 各限制为 `2`，需要时
继承受限 CPU affinity 与低优先级。设计验收不要求独占 CPU 或整套性能 Gate。

## 16. 修订绑定生命周期与维护暂停式切换（#534）

Session 组合 `TrafficWorld` 与可选 `SpatialSession`，配对不变量（同一根
`Arc`）由 Adapter 承担：构造时校验一次，且封闭提取路径每批复核——公开面不存在
绕过配对检查的裸可变 Spatial 访问（`spatial_mut` 已移除）。

- `LaneFlowSession::{cross_revision_cutover, same_revision_restore}`：同步维护暂停式
  切换。目标 Spatial 的可失败验证前置于 Runtime 事务；`commit` 成功后不可失败地
  换绑——成功返回时已完成新配对，无生产可见「新世界 + 旧 Spatial」中间态；事务
  不外泄，失败路径无在途残留、合法重试可成功。`LaneFlowTargetSpatial::Headless`
  是显式转 headless 的唯一形态。
- `LaneFlowSession::extract_committed_pose_batch`：v1 同步封闭路径——同一调用内
  采集 committed pose sources → 配对校验（每批固定 O(1)）→ 按当前配对 Spatial
  提取，产出原地重填调用方持有的 `LaneFlowCommittedPoseBatch`（稳定容量复用；
  失败时输出原样保持）。批次携带消费上下文 `(world_id, WorldGeneration)` 与
  对齐的车辆句柄；任何成功切换使先前上下文过期，宿主持有结果跨区间时用
  `consumption_context_is_current` 复核，过期整批拒绝。稳态完整提取路径零新增分配；Runtime
  来源按值返回的历史成本（#545 登记）已由 #712 的借用迭代器消除。
- `LaneFlowCutoverRecord`（`#[must_use]`）：换出的旧 `SpatialSession`（在途借用
  可完成，结果不得作为当前世界表现提交）、切换后世界绑定与恰一次事件批次——
  语句位丢弃记录即丢弃事件交付。
- 调度边界：切换入口可在任意不处于 fixed step 执行中的宿主系统调用；结构性
  推荐位是 `LaneFlowOuterFrameSet::Administration`（每 outer frame 必运行、先于
  本帧步进与位姿采集、零步进帧可达）。切换是「维护暂停」，不是低停顿在线热
  切换；维护期间墙钟进入 `Time`/accumulator 由宿主停表政策定义，禁止静默清
  backlog。

端到端验收矩阵（存活 Session 跨修订与同修订换根、几何真值、旧输入/旧结果拒绝、
配错根失败关闭、ABA 回旧根对象、失败原子性与重试、旧根对象借用完成与回收、零步进与
catch-up 调度、显式 headless）见 `crates/laneflow-bevy/tests/revision_cutover.rs`；
夹具为同键同长、中心线平移的两条边，几何真值按平移量精确判别。语义权威见
`adapter-api.md` §9 与切换合同 §7。
