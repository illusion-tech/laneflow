# 适配器应用程序接口（API）

**文档状态**: Accepted（#301 后 current 为 `TrafficWorld` + `SpatialSession`）

**最后更新**: 2026-08-31（#541 typed parking lifecycle）

**适用范围**: 交通运行时（Traffic Runtime）、Spatial 与引擎适配器（Engine Adapter）之间的只读位姿与生命周期契约；具体 Bevy 0.19 specialization 见 `bevy-reference-adapter.md`

**关联文档**:

- `../architecture.md`
- `../adr/0001-project-scope.md`
- `../adr/0012-core-numeric-authority-and-presentation-precision.md`
- `../adr/0013-engine-neutral-spatial-geometry-and-length-authority.md`
- `../adr/0015-bounded-f32-canonical-spatial-frames.md`
- `../adr/0016-scenario-population-and-recycle-lifecycle-authority.md`
- `../adr/0020-compiler-owned-static-network-and-static-image.md`
- `../adr/0024-compiler-post-emission-check-and-minimal-publication-closure.md`
- `../adr/0025-checked-canonical-network-and-shared-static-network.md`
- `../reference/glossary.md`
- `spatial-geometry.md`
- `bevy-reference-adapter.md`
- `example-scenarios.md`
- `traffic-runtime-shared-consumption.md`
- `parking-system.md`

## 1. 目标与术语

适配器应用程序接口（API）让宿主引擎驱动 LaneFlow 的固定步长推进（fixed tick）、同步车辆生命周期，并把交通运行时已提交的交通状态转为宿主表现。它不复制交通运行时的交通规则，也不把引擎专用类型泄漏到 Runtime、Data 或 Spatial。

本文中的“宿主”指接入 LaneFlow 的 Bevy、Unity、Unreal、Godot 或 Web 运行环境；“位姿（pose）”指位置和朝向基向量；“批量（batch）”指按稳定顺序一次处理多辆车。组件名 Runtime、Data、Spatial、Adapter 和 Presentation 分别表示交通运行时、数据层、空间层、适配层和表现层。

`laneflow-core` / `CoreWorld` / JSON 运行时入口已由 #301 拆除。current 可运行世界是 `laneflow-runtime::TrafficWorld`。历史 Core snapshot / `SpatialRegistry` 形状不再是生产契约。

> **实现状态**：当前 Runtime/Spatial/Bevy 路径中，显式 parked vehicle 继续使用
> ParkingSpace pose；virtual
> parked vehicle 仍是 live Runtime identity，但不进入 committed pose source 集合。
> Bevy Session 已提供完整 typed parking lifecycle，只有 typed despawn-and-unbind 清除映射。

## 2. 权威职责

“权威职责（authority）”表示某项状态由哪一层定义并最终裁决。

| 关注点                               | 权威层                | 适配器职责                                        |
| ------------------------------------ | --------------------- | ------------------------------------------------- |
| 固定步长、车辆、路线、信号和停车状态 | Traffic Runtime       | 调度 `TrafficWorld::step`，读取已提交 pose / 信号 |
| 中心线、弧长和位姿采样               | Spatial               | 绑定同一共享根，消费 pose 批次                    |
| 交通与空间制品的解析                 | 编译器 / 共享静态路网 | 提供已构建的 `Arc<SharedNetworkRevision>`         |
| 实体、预制体、变换、动画和细节层次   | Adapter/Presentation  | 作为唯一的宿主表现事实源                          |

适配器不得把宿主变换（Transform）反写为 Runtime 进度，也不得用引擎样条曲线长度覆盖共享根 / Spatial 的长度绑定。

宿主 asset pipeline 为发布加载认证 LFCP v2 / manifest，或为本地编辑提供已提交道路
状态；`laneflow-format` 与 `laneflow-static-network` 产生完整
`Arc<SharedNetworkRevision>`。Runtime 安装完整根。`SpatialSession::bind` 只接受该
根 `Arc`，**不**从 Runtime 发布的 snapshot/facade 借用，也不得依赖 `laneflow-runtime`。
同时持有 world 与 session 的 Adapter 必须 `Arc::ptr_eq`（或证明两者来自同一保留的根
`Arc`）。不存在独立的 Traffic 或 Spatial component 安装入口。Adapter 不解析 compiler IR
或 LFCA table，不重建 Traffic/Spatial binding，也不读取共享数组中的静态规则来自行裁决
行为或单独替换 component。pose 使用与 Runtime 无关的 `PoseRecordId`，见
`traffic-runtime-shared-consumption.md`。

## 3. 生命周期顺序

current 路径：

```text
读取引擎资源
  -> 加载认证 LFCP v2 / 已提交道路状态，构建 Arc<SharedNetworkRevision>
  -> TrafficWorld::install(同一根 Arc)
  -> SpatialSession::bind(同一根 Arc)；配对 Arc::ptr_eq
  -> 建立 PoseRecordId 与宿主实体的绑定
  -> 在 LaneFlowFixedSet::Lifecycle 经 world_mut 提交生命周期命令
  -> TrafficWorld::step
  -> 读取 committed_pose_sources，映射 PoseRecordId
  -> SpatialSession::extract_pose_batch
  -> 提交宿主生命周期、变换和表现结果
```

Bevy Reference Adapter 把上述 world 与可选 Spatial 收进唯一活动的 `LaneFlowSession`。
`LaneFlowSession::new` 在提供 Spatial 时强制 `Arc::ptr_eq`，失败为 `RevisionMismatch`。
生命周期命令经 `LaneFlowSession::world_mut()` 调用 `TrafficWorld` 的
`register_route` / `spawn_vehicle` / `remove_route`，并通过 Session typed wrapper 调用
`reserve_parking` / `cancel_parking` / `park_vehicle` / `leave_parking` /
`rebind_parking_route` / `spawn_parked_vehicle`。原子替换只走 typed
`laneflow_bevy::replace_completed_vehicle`；真正移除已绑定车辆只走 typed
`despawn_vehicle`/despawn-and-unbind 入口。两者都不得经 raw `world_mut()` 绕过宿主映射
事务，以免 Runtime 成功后留下 stale 映射。只读观察走 `world()`。

适配器只能从已提交状态生成表现结果。推进、Spatial 提取或宿主转换任一步失败时，都不能留下只完成一部分的车辆映射或变换批次。

## 4. 位姿输入与输出

Adapter 从 `TrafficWorld::committed_pose_sources()` 读取稳定顺序的
`(VehicleHandle, PoseSource)`，再映射为不透明 `PoseRecordId`（`u32`）。Spatial
不接收 `VehicleHandle`，不遍历宿主实体组件系统（ECS），也不重新判断车辆生命周期。
Bevy 用 `pose_input` 完成 Runtime `PoseSource` 到 Spatial `PoseInput` 的映射。

```text
PoseInput {
  record: PoseRecordId
  source: Lane { edge: LaneEdgeOrdinal, progress }
        | Parking { space: ParkingSpaceOrdinal }
}
```

- 行驶中或停止中的车道车辆使用 `Lane`；显式泊位中的已停放车辆使用 `Parking`。位置权威判别由 source enum 表达，不增加可互相矛盾的 status 字段。
- lane 进度与共享根边长同域；parking 用共享根停车位序号。
- 已完成或已移除车辆不出现在 `committed_pose_sources`，由调用方决定是否清理宿主实体。
- 输入和输出顺序必须稳定，不能依赖引擎实体组件系统（ECS）或散列表的遍历顺序。
- 一批必须同一 canonical frame；批次头带共享根 `NetworkRevisionId`、
  `CanonicalFrameOrdinal` 与 `FramePlacementToken`，混 frame 整批失败。

#540 合同增加一种“live 但不可表现”的已提交状态，而不是给 `PoseInput` 增加
`ParkingFacility` 分支：

- 成功 virtual park 后，Runtime binding/status 已提交，车辆从
  `committed_pose_sources` 缺席；Adapter 此时才隐藏或回收宿主实体。
- virtual park 失败时仍保留原 lane pose 和实体；virtual leave 失败时继续隐藏。
- 成功 leave 后，Runtime 同一提交建立 Active/lane authority；Adapter 只在新的 committed
  Lane source 可见后重建/显示实体。
- “本批无 pose”本身不能判定 despawn、Completed 或 virtual Parked。Adapter 必须消费
  Runtime typed status/binding 或 committed lifecycle observation；可见性不能反向裁决
  parking authority。
- 成功 typed `despawn_vehicle` 才表示真正移除。Adapter 必须在同一组合事务中清除
  Runtime handle ↔ 宿主 Entity/池槽映射；对象当前可见、因 virtual Parked 隐藏或已经
  回收到表现池，都只能改变清理动作，不能改变 removal 语义。

Spatial 提供 LaneFlow 自有的有界 `f32` canonical 位姿。生产输出为：

```text
CanonicalPoseBatch {
  network_revision: Option<NetworkRevisionId>
  canonical_frame: Option<CanonicalFrameOrdinal>
  placement_token: FramePlacementToken
  records: Vec<CanonicalPoseRecord>
}

CanonicalPoseRecord { record: PoseRecordId, pose: CanonicalPoseF32 }
```

`canonical_frame` 和 `placement_token` 只在 batch header 保存一次，不逐车辆重复。批次内位置每轴位于 `±16_384 m`；点、切向量和上方向都不暴露宿主或第三方类型。LaneFlow 不维护默认 canonical `f64` 位姿作为第二套运行时权威。

适配器拥有 frame 到宿主场景的放置和生命周期映射，可以在宿主末端使用 double world placement、tile 或相机相对原点，但不得把转换后的宿主位置反写到 Spatial / Runtime。`FramePlacementToken(u64)` 是调用方颁发、只比较相等性的 opaque token；Spatial 原样回显，Adapter 在提交 Transform 前必须复核 token 仍是当前值。同一 frame 重新放置、切 tile 或 rebase 时必须换 token，因此旧批次不能在 placement 切换后提交。token 不包含世界坐标、origin value 或宿主 Transform。

## 5. 宿主转换

适配器必须显式完成：

- 从 LaneFlow 标准的右手、Y 轴向上坐标系映射到宿主的手性、上方向和前方向约定；
- 从 LaneFlow 的切向量和上方向向量构造宿主旋转与变换；
- canonical frame 的宿主放置、分块或相机相对定位；
- 引擎标量类型与数值范围检查；
- 实体生命周期、插值、细节层次（LOD）和调试绘制。

Bevy/glam、Unity `Vector3`、Unreal `FVector`、Godot `Vector3` 以及 JavaScript/Web 向量类型只能出现在对应适配器的末端。LaneFlow 不承诺宿主 `Transform` 的二进制接口（ABI）或序列化布局。

## 6. 批量处理与错误语义

- `SpatialSession::extract_pose_batch(&mut self, placement_token, inputs, output)`
  接收调用方拥有的 input slice 与可复用 `CanonicalPoseBatch`。session 内部保留
  scratch，成功时写入 `output`。
- 混用多个 canonical frame 在提交 `output` 前失败（`BatchFrameMismatch` 带
  `expected_frame` / `actual_frame`）；任一无效 edge、space、progress、朝向基或
  canonical 范围记录都会使整个批次失败，并报告稳定输入序号、`PoseRecordId` 和
  结构化 source。
- 全部 records 先写 scratch；只有全部成功后才写入 `output` 并更新 placement
  token。失败时旧 `output` 的 frame、token 和 records 逐项不变，scratch 清空但保留容量。
- 调用方可跨 tick 复用 `output` 与 session scratch；稳定容量下不要求每批新分配。
- canonical frame 与宿主坐标之间的转换不得修改 `TrafficWorld`、`SpatialSession` 或共享根。
- 单记录查询可以用于调试，但不能作为当前道路机动车执行域一万或十万车辆运行单元的
  默认同步路径。

## 7. v0.7 Bevy specialization

#121 已在 `bevy-reference-adapter.md` 冻结 v0.7 的 Bevy 0.19 支持线、最小 modular dependency graph、专用 fixed schedule、单活动 Session、frame-root/child Transform、placement token 复核、两阶段原子批量提交、可选 Gizmos、最小 native example。#301 之后该 Session 持有 `TrafficWorld` 与可选 `SpatialSession`。可选 Gizmos 不是现行交付；#172 历史实现见 git，[#473](https://github.com/illusion-tech/laneflow/issues/473) 已关闭。

该 specialization 不改变本文的跨引擎权威职责、`f32` canonical 精度边界、稳定批量顺序、失败原子性和宿主类型隔离。v0.7 仍不冻结 presentation interpolation、LOD/pooling、glTF/prefab/scene asset API、WASM、外语绑定的二进制接口、C 外部函数接口（FFI）或第二个 Engine Adapter。

现行入口是 `TrafficWorld` 与 `LaneFlowSession`。

## 8. 生命周期命令、原子替换与原子移除

#475 冻结 `TrafficWorld::replace_completed_vehicle` 与 Bevy typed replace-and-rebind。
#540 修订 ADR 0016：保留真正移除用的原子 `despawn_vehicle`，并要求 Adapter 提供 typed
despawn-and-unbind；它不得被拼成 replacement 的“先删后建”，也不恢复已拆除的全局事件
backlog。

```rust
TrafficWorld::replace_completed_vehicle(
    old: VehicleHandle,
    input: VehicleSpawnInput,
) -> Result<VehicleReplaceRecord, ReplaceError>

replace_completed_vehicle(
    world: &mut bevy_ecs::world::World,
    old: VehicleHandle,
    input: VehicleSpawnInput,
) -> Result<LaneFlowVehicleReplaceOutcome, LaneFlowAdapterError>

TrafficWorld::despawn_vehicle(
    vehicle: VehicleHandle,
) -> Result<VehicleDespawnRecord, ParkingError>

despawn_vehicle(
    world: &mut bevy_ecs::world::World,
    vehicle: VehicleHandle,
) -> Result<LaneFlowVehicleDespawnRecord, LaneFlowAdapterError>
```

- 预检失败则已提交世界不变；成功则一次提交旧结束与新开始。
- `ReplaceError::Blocked` 仅入口占用/重叠，可重试；Adapter 此时映射与 Transform 不变。
- 已绑定：成功则同一 Entity 轮换到新句柄。未绑定保持未绑定。
- 到达路线终点写成 `Completed`，保留句柄与容量，不进 pose、不占车道。
- Runtime despawn 对每个 live `VehicleStatus`（`Active | Parked | Completed`）都是真正
  移除，并按合法状态矩阵原子释放可选 `Reserved | Occupied` parking binding；parking
  release 由 typed record 回显，不能在 handle stale 后反查拼装。
- Adapter despawn 先预检映射/清理容量，再提交 Runtime removal，并以不可失败路径恰好
  一次清除映射和销毁/回收宿主对象；预检或 Runtime 失败时两侧均不变。
- virtual Parked 无 pose 不是 despawn；只有 typed removal outcome 可以触发上述映射清理。

当前 parking typed mapping 保留完整 caller-selected payload：reserve 回显 bound
route/entry occurrence/virtual entry selector，leave 回显恢复 route/exit occurrence/virtual
exit selector，rebind 回显 old/new route 与 current/entry occurrence/selector。Adapter 不从
facility、LaneEdge 或无 pose 状态反推被省略的 route/anchor，也不把不同 payload 的调用
合并成 `NoChange`。

当前 Bevy 生命周期边界是 `LaneFlowFixedSet::Lifecycle`。原子替换只走 typed
`replace_completed_vehicle`，已绑定车辆移除只走 typed `despawn_vehicle`。Adapter 不复制
跟车、信号或停车规则。

Population 的 seed、portal/lane 抽样、pending/retry queue 仍是 engine-neutral caller-owned authority，不进入 Adapter 或 Bevy ECS；初始人口在 Session 创建前完成。
