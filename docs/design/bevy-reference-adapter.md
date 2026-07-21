# Bevy Reference Adapter

**文档状态**: Accepted

**最后更新**: 2026-07-21（#169 Plugin/Session 与 fixed schedule）

**适用范围**: v0.7 的 Bevy 0.19 Reference Adapter、headless 集成验证、可选调试可视化与最小 native example

**关联文档**:

- `../architecture.md`
- `../roadmap.md`
- `../adr/0001-project-scope.md`
- `../adr/0013-engine-neutral-spatial-geometry-and-length-authority.md`
- `../adr/0015-bounded-f32-canonical-spatial-frames.md`
- `adapter-api.md`
- `spatial-geometry.md`
- `../reference/validation-matrix.md`
- `../reference/v0.6-spatial-validation.md`

## 1. 目标

`laneflow-bevy` 是 LaneFlow 的首个 Reference Adapter。它用一个真实 Rust 游戏引擎验证 fixed tick 调度、车辆与实体生命周期、批量位姿到宿主 Transform 的转换、调试可视化和最小可运行示例，但不改变 Core、Data 或 Spatial 的权威职责。

v0.7 的完成目标是提供一条可构建、可测试、可演示且默认依赖面受控的 Bevy 集成路径。Bevy 不是跨 ABI、跨语言兼容性的唯一证明。

## 2. 版本、工具链与依赖边界

- v0.7 只支持 Bevy `0.19.x`；实际 patch 由仓库 `Cargo.lock` 固定。
- 升级到 Bevy 0.20 必须创建独立迁移 Issue，重新审计 API、feature graph、MSRV、许可证、RustSec 与性能，不在 v0.7 中静默放宽版本范围。
- LaneFlow workspace MSRV 继续为 Rust `1.96.0`。Bevy 0.19.0 的 MSRV 为 Rust 1.95.0，因此不提高 LaneFlow 的工具链下限。
- Bevy 0.19.0 使用 `MIT OR Apache-2.0`，可进入 LaneFlow 当前 cargo-deny 允许范围；最终实现仍必须以实际 lock graph 重新运行完整 dependency policy。
- production `laneflow-bevy` 直接依赖最小 modular crates：`bevy_app`、`bevy_ecs`、`bevy_time` 与 `bevy_transform`。
- production manifest 对四个 modular crates 关闭 default features；`bevy_app`、`bevy_ecs`、`bevy_time` 只启用 `std`，`bevy_transform` 启用 `std + bevy-support`。默认 graph 不激活 Bevy reflect、async executor 或 backtrace。
- 默认 production feature graph 不包含 umbrella `bevy`、renderer、window、audio、asset、scene、UI、Gizmos，以及没有被实现证明必要的 reflect/state/input。
- 完整 `DefaultPlugins`、render/window、mesh/material 和 Gizmos 只能进入显式 opt-in feature 或 example 边界。

## 3. 权威职责与 Session 边界

v0.7 每个 Bevy `App` 只支持一个活动 `LaneFlowSession`。Session 可以组合或持有：

- `CoreWorld`；
- `SpatialRegistry`；
- 已提交的 pose batch；
- 可复用的 extraction、validation 与 Transform staging scratch；
- 当前 canonical frame placement 与 token；
- Adapter-owned Vehicle/Entity 映射。

组合不改变权威职责：

- Core 决定 fixed quantum、tick index、simulation time、车辆、路线、信号与停车状态。
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

## 8. 可选调试可视化

`debug-gizmos` 是非默认 opt-in feature：

- 只消费最近一次已验证的 presentation batch。
- 最小绘制内容为 frame axes 与车辆 position/forward/up marker。
- 调用方可以提供已经加载的空间几何绘制中心线，但 Adapter 不重新计算长度或建立第二套 Spatial authority。
- 必须提供可配置绘制预算与过滤器；达到预算时按稳定 batch 顺序截取。
- debug feature、运行时开关和预算不得改变 Core、Spatial、映射或 Transform 结果。
- Gizmos/render/window 依赖不进入默认 production graph。

调试绘制用于诊断坐标、frame、映射和 pose，不是 editor、authoring tool、通用 inspector 或高车辆数全量可视化承诺。

## 9. 最小 native example

native example 使用显式 `native-example` feature，并把完整 Bevy `DefaultPlugins`、window、renderer、mesh/material 留在示例边界。

示例必须：

- 加载仓库现有 `examples/data/v0.1-campus.scenario.json` 及其 traffic/spatial artifacts；
- 使用内建简单几何生成车辆和必要场景表现，不要求外部二进制素材；
- 展示 fixed tick、Vehicle/Entity binding、frame-root Transform、车辆移动和可切换 debug Gizmos；
- 提供精确运行命令、dedicated compile check、本机 smoke 记录与截图；
- 不把 camera、input、renderer 或示例资源管理提升为 production Adapter API。

v0.7 不冻结 glTF、prefab、scene、asset pipeline、UI、presentation interpolation 或 LOD/pooling 算法。GUI smoke 不能替代 headless deterministic tests。

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

- 稳定容量下，10k/100k Adapter-owned batch 路径零 allocation/reallocation；
- 10k p95 不超过 `4 ms`；
- 100k p95 不超过 `40 ms`；
- 10k 到 100k 的扩展不超过 `12x`。

共享 CI 运行 correctness、determinism、allocation、workspace/MSRV、example/benchmark compile 与 dependency policy。绝对 wall-clock Gate 只在记录了机器、source commit、命令、样本和后台负载的固定环境运行，不作为跨平台 SLA。

## 11. 执行切片与 PR 角色

| Issue | 交付切片                                     | 直接前置         |
| ----: | -------------------------------------------- | ---------------- |
|  #169 | 最小 crate、Plugin/Session 与 fixed schedule | 无活动 blocker   |
|  #170 | Vehicle/Entity 映射与原子批量 Transform      | #169             |
|  #171 | headless E2E、allocation/performance 与 CI   | #170             |
|  #172 | 可选、预算受控的 debug Gizmos                | #170             |
|  #173 | 最小 native reference example                | #170             |
|  #174 | 最终集成文档与独立 closure review            | #171、#172、#173 |

每个子 Issue 使用自己的唯一 Delivery PR。#169-#173 的 PR 对父 #121 只使用 Related PR 语义，不得以 closing keyword 覆盖父 tracker。#174 的最终 integration PR 同时作为 #174 与 #121 的唯一 Delivery PR；所有子 Issue G4 后才允许 #121 进入最终 G3/G4。

## 12. 兼容性与后续演进

本设计不改变 Core API、current data format 或 Spatial public authority。它新增 Bevy-specific Adapter API；在 v1.0 前仍可按独立 Issue 演进，但以下变化必须重新进入 G1：

- 支持 Bevy 0.20 或第二个 Bevy major/minor line；
- 修改 default dependency/feature graph；
- 修改 fixed schedule ownership、time-drop policy 或 authority；
- 放宽为多 Session、多 canonical frame 或跨 frame migration；
- 改变 mapping 一一性、batch 稳定顺序、placement token 或失败原子性；
- 让宿主类型、Transform 或 Gizmos 进入 Core/Data/Spatial 公共接口。

插值、LOD/pooling、prefab/glTF、WASM、第二个 Engine Adapter 和 foreign-host boundary proof 保持后续独立范围，不是 v0.7 完成条件。
