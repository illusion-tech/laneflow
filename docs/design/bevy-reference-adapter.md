# Bevy Reference Adapter

**文档状态**: Accepted

**最后更新**: 2026-08-24

**适用范围**: v0.7 的 Bevy 0.19 Reference Adapter、headless 集成验证、可选调试可视化与最小 native example

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

- v0.7 只支持 Bevy `0.19.x`；实际 patch 由仓库 `Cargo.lock` 固定。
- 升级到 Bevy 0.20 必须创建独立迁移 Issue，重新审计 API、feature graph、MSRV、许可证、RustSec 与性能，不在 v0.7 中静默放宽版本范围。
- LaneFlow workspace MSRV 继续为 Rust `1.96.0`。Bevy 0.19.0 的 MSRV 为 Rust 1.95.0，因此不提高 LaneFlow 的工具链下限。
- Bevy 0.19.0 使用 `MIT OR Apache-2.0`，可进入 LaneFlow 当前 cargo-deny 允许范围；最终实现仍必须以实际 lock graph 重新运行完整 dependency policy。
- production `laneflow-bevy` 直接依赖最小 modular crates：`bevy_app`、`bevy_ecs`、`bevy_time` 与 `bevy_transform`。LaneFlow 生产依赖只有 `laneflow-runtime` 与 `laneflow-spatial`；`occupy_parking` 的停车序号走 Runtime 再导出，不把 `laneflow-static-contract` 写入 Adapter 生产 graph。
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
- `LaneFlowOuterFrame` 插入宿主 `First` 之后，因此读取的是本帧已经由 `TimePlugin` 更新的 `Time::delta()`；调用方负责安装 `TimePlugin` 或包含它的宿主 plugin group。
- `LaneFlowPlugin` 不重复安装 `TimePlugin` 或 `TransformPlugin`。缺少 Session 时 schedule 无操作；存在 Session 但缺少 `Time` resource 时记录结构化错误。
- `LaneFlowSessionConfig` 要求调用方显式提供非零 `max_catch_up_steps`，不定义隐藏默认值。
- accumulator 保存完整 `Duration`，不按 outer frame 截断亚毫秒余量；只有 Core step 成功后才扣除一个 quantum。达到上限或 step 失败时，当帧停止并保留全部 backlog。
- `LaneFlowFrameReport` 公开 frame delta、成功 step 数、backlog 与 catch-up-limit 状态；Session 保留当帧全部成功 `StepResult` 和最近 `LaneFlowAdapterError`。

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

Presentation 从 committed `TrafficWorld::committed_pose_sources()` 重建 pose inputs：Active vehicle 使用当前 route edge 与 progress，Parked vehicle 使用已提交停车占用，Completed vehicle 不进入 presentation batch。batch 同时包含已映射和未绑定 vehicle，映射查询不会改变 record 顺序。

`LaneFlowPlugin` 安装 exclusive `PostUpdate` 系统并显式排序在 `TransformSystems::Propagate` 前。实现先完成 Spatial batch、frame/token、root、所有 mapped Entity/parent/Transform 与转换结果校验，再统一写入 ECS；exclusive system 内两阶段之间没有其他 system 可以修改 Entity。`LaneFlowPresentationReport` 暴露 `pose_records`、`mapped_records`、`unbound_records` 与 `applied_records`，失败时 `applied_records` 恒为零，具体失败保存在 Session 的最近错误中。

## 8. 可选调试可视化

#301 后仓库不再导出占位 `debug-gizmos` feature 或空 plugin。v0.7 / #172 曾交付过预算受控 gizmos；#301 拆除 Core 表现层时一并删除。campus / `debug_gizmos_smoke` 与 JSON 入口只存在于 git 历史。

该能力不是现行交付。[#473](https://github.com/illusion-tech/laneflow/issues/473) 已关闭：当前最小 Bevy 证据是 `runtime_min` 与无窗口 smoke，不恢复 gizmos 公共 API。真正有可见诊断场景时再开新 Issue；默认 production graph 仍不得包含 Gizmos/render/window。

## 9. 最小 native example

现行最小 native example 是 `runtime_min`：安装 `TrafficWorld` 与可选 `SpatialSession`，用 compiler 夹具 `LFCA-V1-FULL-SPATIAL`，不读取 campus JSON，也不构造 `CoreWorld`。

```powershell
cargo +1.96.0 check --locked -p laneflow-bevy --example runtime_min --features native-example
cargo +1.96.0 test --locked -p laneflow-bevy --test runtime_min_smoke
```

`native-example` 仍是非默认 opt-in，完整 `DefaultPlugins` / window / renderer 留在示例边界。v0.7 `native_reference`、campus JSON 与 `laneflow_data::from_scenario_json_slice` 已删除。现行走廊 native example 使用检入 catalog 0.2 与 LFCA，prepare 绑到已安装共享路网修订；50–200 回流见 [#475](https://github.com/illusion-tech/laneflow/issues/475)。

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

#174 的独立审阅没有发现需要回开生产实现、Adapter API 或 CI 契约的缺口。当时收口流水账见 git 历史。本节的剩余项继续是明确的后续非目标。

## 12. 兼容性与后续演进

本设计不改变 Core API、current data format 或 Spatial public authority。它新增 Bevy-specific Adapter API；在 v1.0 前仍可按独立 Issue 演进，但以下变化必须重新进入 G1：

- 支持 Bevy 0.20 或第二个 Bevy major/minor line；
- 修改 default dependency/feature graph；
- 修改 fixed schedule ownership、time-drop policy 或 authority；
- 放宽为多 Session、多 canonical frame 或跨 frame migration；
- 改变 mapping 一一性、batch 稳定顺序、placement token 或失败原子性；
- 让宿主类型、Transform 或 Gizmos 进入 Core/Data/Spatial 公共接口。

插值、LOD/pooling、prefab/glTF、WASM、第二个 Engine Adapter 和 foreign-host boundary proof 保持后续独立范围，不是 v0.7 完成条件。

## 13. v0.8 直行走廊 schedule 与 proxy 复用

现行 `signalized_corridor` 安装 `TrafficWorld` 与可选 `SpatialSession`，用 catalog 0.2
prepare 绑定车辆。#475 交付 `TrafficWorld::replace_completed_vehicle` 与 Session typed
replace-and-rebind；薄示例仍可不启用 50–200 人口。不得再走已拆除的 JSON 运行时入口。


`LaneFlowFixedSet::{Lifecycle, Step, Observe}` 的固定顺序为：每个 LaneFlow fixed step 前在 `Lifecycle` 应用 pending lifecycle commands，Adapter 在 `Step` 推进一次 `TrafficWorld`，调用方在 `Observe` 消费 committed result 并为下一 boundary 入队。一个 outer frame 内的 catch-up steps 逐步重复完整链，presentation 仍在 outer frame 最多提交一次，因此 frame chunking 不得改变 Population/Runtime 决策。

车辆完成 route 后，既有 proxy/model 不 despawn。等待可用入口时，Completed vehicle 不产生 pose record，Entity 保留最后 Transform。caller 在 `Lifecycle` 使用 `replace_completed_vehicle`：`Blocked` 保持 Runtime/映射/Transform 不变并允许继续处理其他计划；`Replaced` 把同一 Entity 从 old handle 轮换到 new handle，下一次正常 `PostUpdate` presentation 才更新入口位姿。fatal Adapter/Runtime error 停止该 outer frame 当前和后续 catch-up，完整保留 backlog；已成功的前序 command 不做跨 command 回滚。

该边界不提供通用 runtime despawn、Adapter-owned persistent queue/retry 或人口 controller。初始人口在创建 Session 前由调用方建立；seed、portal/lane 抽样与 retry policy 由 `laneflow-scenario` 的 engine-neutral caller policy 拥有。

## 14. v0.8 signalized-corridor native specialization

#189 在 `native-example` feature 下新增独立 `signalized_corridor` target，保留 v0.7
`native_reference` 的最小边界。`laneflow-scenario`、TOML config projection 与测试依赖
只进入 `laneflow-bevy` dev/example graph，不成为 Adapter production dependency 或
public API。

启动顺序固定为：

```text
检入 catalog 0.2 + LFCA
  -> SharedNetworkRevision::build + TrafficWorld::install
  -> catalog bind + population prepare
  -> spawn_vehicle batch
  -> population bind
  -> Session + 可选 proxy
  -> 首个 fixed step
```

任一启动错误都在发布 window/App 前失败。example-local population Resource 在
`Lifecycle` 调用 Adapter typed replacement，在 `Observe` 只消费本次 committed
`frame_step_results().last()`；policy invariant failure 立即停止示例，不扩大公共
Adapter error enum。成功 replacement 保持 Entity identity，blocked/pending 保持最后
Transform。

表现层只消费已加载事实：

- lane surfaces/markings 来自 Spatial centerlines 与 config lane width；
- StopLine 与灯具 binding 来自 Core StopLine/ManeuverGate/SignalGroup registry；
- lamp material 只由 committed `signal_group_state` 驱动；
- camera/ground 从 Spatial bounds 派生；example-local orbit camera 支持滚轮缩放、
  左键水平面平移和右键绕 focus 旋转；
- 运行遥测由 Bevy UI 绘制在画面内，window title 保持稳定；
- Core/Spatial vehicle pose 的 `edgeProgress`/translation 是前保险杠位置；built-in
  vehicle child 沿 local `+Z` 向后延伸，proxy 原点不得作为车身中心；
- built-in vehicle child mesh、颜色、键盘和 screenshot 只属于 example。

HUD 的 example-local 诊断不进入 Adapter public API。宿主 FPS/frame time 按约 1 秒
outer-frame 采样窗口计算；LaneFlow step CPU 从 population `Lifecycle` 完成后开始，到
`Observe` 消费 committed result 前结束，只包围既有 `LaneFlowFixedSet::Step`，并按同一
窗口展示平均 `ms/frame` 与 `us/tick`。HUD 同时读取 `LaneFlowFrameReport` 的最近
step/backlog/catch-up 状态，以及 #203 controller 的 target/running/pending 权威人口。
wall-clock 样本不进入 Core input/state、PRNG 或 replay，不包含 renderer、截图和
`PostUpdate` presentation，也不替代 #171 固定机 Adapter performance Gate。

默认 authoring config 使用 `[0, 29000] ms` controller offsets，在 `58000 ms` cycle
中提供可见半周期差；该 sample content 修正不改变 Traffic/Spatial/Manifest/catalog
格式版本。dedicated example tests 覆盖 50/100/200 production bootstrap/headless
运行、same-Entity recycle 与 outer-frame partition replay；GUI smoke 与截图仍是 G3/G4
前独立的本机和产品验收证据。

## 15. v0.9 protected-turning native specialization

#190 将 `signalized_corridor` 的 authoring/startup config clean-break 到 `0.2`，入口为
`examples/config/v0.10-signalized-corridor.toml`；Traffic/Spatial/Manifest 继续使用
`0.8/0.1/0.1`，scenario-local catalog 切换为 `0.2`。该切片只消费 #229 已有的
Junction/Movement/ManeuverPath/ManeuverGate handles 和 Route occurrences，不改变
production Adapter public API。

example-local observation 在启动期把 protected-left/right Movement external ID
解析为 handles，再按每条 Route 的 compiled Maneuver occurrences 冻结
`RouteHandle -> {ProtectedLeft, Straight, ProtectedRight}`。steady frame/tick 只读取
handles；HUD、车辆材质和 recycle 后的 route 更新不做 external-ID lookup、path
matching 或 geometry inference。共享 StopLine 的多个 Gate 只生成一个 lamp visual，
且启动时必须证明它们引用同一 SignalGroup。

默认 controller program 为 12 phase/84 秒，offset `[0, 42000] ms`。headless tests
覆盖三类 Route 进入 internal edge、SignalStop 后同 vehicle/route 跨同一 Gate 放行、
共享 entry queue、有限 Adapter pose 和 same-Entity recycle identity。#190 G3 前仍
要求 Windows 默认 100 车/seed 0 GUI smoke 与截图；50/100/200、stress seeds、
clearance/performance 的扩大验证由 #191 拥有，独立收口由 #192 拥有。
