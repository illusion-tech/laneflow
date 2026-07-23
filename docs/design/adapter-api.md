# 适配器应用程序接口（API）

**文档状态**: 已接受（Accepted）

**最后更新**: 2026-07-23（#187 Bevy lifecycle bridge）

**适用范围**: Core、Spatial 与引擎适配器（Engine Adapter）之间的只读位姿与 typed lifecycle 契约；具体 Bevy 0.19 specialization 见 `bevy-reference-adapter.md`

**关联文档**:

- `../architecture.md`
- `../adr/0001-project-scope.md`
- `../adr/0012-core-numeric-authority-and-presentation-precision.md`
- `../adr/0013-engine-neutral-spatial-geometry-and-length-authority.md`
- `../adr/0015-bounded-f32-canonical-spatial-frames.md`
- `../adr/0016-scenario-population-and-recycle-lifecycle-authority.md`
- `core-runtime.md`
- `spatial-geometry.md`
- `bevy-reference-adapter.md`
- `example-scenarios.md`

## 1. 目标与术语

适配器应用程序接口（API）让宿主引擎驱动 LaneFlow 的固定步长推进（fixed tick）、同步车辆生命周期，并把 Core 已提交的交通状态转为宿主表现。它不复制 Core 的交通规则，也不把引擎专用类型泄漏到 Core、Data 或 Spatial。

本文中的“宿主”指接入 LaneFlow 的 Bevy、Unity、Unreal、Godot 或 Web 运行环境；“位姿（pose）”指位置和朝向基向量；“批量（batch）”指按稳定顺序一次处理多辆车。组件名 Core、Data、Spatial、Adapter 和 Presentation 分别表示核心层、数据层、空间层、适配层和表现层。

## 2. 权威职责

“权威职责（authority）”表示某项状态由哪一层定义并最终裁决。

| 关注点                               | 权威层               | 适配器职责                               |
| ------------------------------------ | -------------------- | ---------------------------------------- |
| 固定步长、车辆、路线、信号和停车状态 | Core                 | 调度并消费已提交的快照与事件             |
| 中心线、弧长和位姿采样               | Spatial              | 提供快照记录与 frame 放置，消费批量位姿  |
| 交通与空间制品的解析                 | Data/Spatial 加载器  | 提供调用方已读取的字节；管理引擎资源周期 |
| 实体、预制体、变换、动画和细节层次   | Adapter/Presentation | 作为唯一的宿主表现事实源                 |

适配器不得把宿主变换（Transform）反写为 Core 进度，也不得用引擎样条曲线长度覆盖 Core/Spatial 的长度绑定。

## 3. 生命周期顺序

```text
读取引擎资源
  -> 加载并绑定交通包、空间包和场景清单
  -> 构造 CoreWorld
  -> 建立车辆与宿主实体的绑定
  -> 提交固定步长命令和输入
  -> Core 完成并提交一次推进
  -> 读取快照和事件
  -> Spatial 批量提取位姿
  -> 提交宿主生命周期、变换和表现结果
```

适配器只能从已提交状态生成表现结果。Core 推进、Spatial 提取或宿主转换任一步失败时，都不能留下只完成一部分的车辆映射或变换批次。

## 4. 位姿输入与输出

面向适配器的生产位姿输入由 #136 固定为：

```text
PoseInputRecord {
  vehicle: VehicleHandle
  source: Lane { edge: EdgeHandle, progress: EdgeProgress }
        | Parking { space: ParkingSpaceHandle }
}
```

- 行驶中或停止中的车道车辆使用 `Lane`；已停放车辆使用 `Parking`。位置权威判别由 source enum 表达，不增加可互相矛盾的 status 字段。
- Adapter 从同一已提交 Core snapshot 构造调用方拥有的稳定序列；Spatial 不接收 `CoreWorld`、不遍历宿主实体组件系统（ECS），也不重新判断车辆生命周期。
- 已完成或已移除车辆不产生有效位姿记录，由具体生命周期事件决定是否清理宿主实体。
- 输入和输出顺序必须稳定，不能依赖引擎实体组件系统（ECS）或散列表的遍历顺序。

Spatial 提供 LaneFlow 自有的有界 `f32` canonical 位姿。生产输出为：

```text
CanonicalPoseBatchF32 {
  frame_id: CanonicalFrameId
  placement_token: FramePlacementToken
  records: Vec<CanonicalPoseRecordF32>
}

CanonicalPoseRecordF32 { vehicle: VehicleHandle, pose: CanonicalPoseF32 }
```

`frame_id` 和 `placement_token` 只在 batch header 保存一次，不逐车辆重复。批次内位置每轴位于 `±16_384 m`；点、切向量和上方向都不暴露宿主或第三方类型。LaneFlow 不维护默认 canonical `f64` 位姿作为第二套运行时权威。

适配器拥有 frame 到宿主场景的放置和生命周期映射，可以在宿主末端使用 double world placement、tile 或相机相对原点，但不得把转换后的宿主位置反写到 Spatial/Core。`FramePlacementToken(u64)` 是调用方颁发、只比较相等性的 opaque token；Spatial 原样回显，Adapter 在提交 Transform 前必须复核 token 仍是当前值。同一 frame 重新放置、切 tile 或 rebase 时必须换 token，因此旧批次不能在 placement 切换后提交。token 不包含世界坐标、origin value 或宿主 Transform。

## 5. 宿主转换

适配器必须显式完成：

- 从 LaneFlow 标准的右手、Y 轴向上坐标系映射到宿主的手性、上方向和前方向约定；
- 从 LaneFlow 的切向量和上方向向量构造宿主旋转与变换；
- canonical frame 的宿主放置、分块或相机相对定位；
- 引擎标量类型与数值范围检查；
- 实体生命周期、插值、细节层次（LOD）和调试绘制。

Bevy/glam、Unity `Vector3`、Unreal `FVector`、Godot `Vector3` 以及 JavaScript/Web 向量类型只能出现在对应适配器的末端。LaneFlow 不承诺宿主 `Transform` 的二进制接口（ABI）或序列化布局。

## 6. 批量处理与错误语义

- `SpatialRegistry::extract_pose_batch` 接收调用方拥有的 input slice、committed `CanonicalPoseBatchF32` 与 `CanonicalPoseBatchScratch`。
- output frame 与 registry frame 不同会在读取 records 前失败；任一无效 edge、space、progress、朝向基或 canonical 范围记录都会使整个批次失败，并报告稳定输入序号、车辆句柄和结构化 source。
- 全部 records 先写 scratch；只有全部成功后才 swap 到 output 并更新 placement token。失败时旧 output 的 frame、token 和 records 逐项不变，scratch 清空但保留容量。
- 调用方可以同时预留并跨 tick 复用 output/scratch；稳定容量下不要求 per-batch allocation。#137 已验证精确零 allocation 与 10k/100k 性能，固定机结果和适用边界见 `../reference/v0.6-spatial-validation.md`。
- canonical frame 与宿主坐标之间的转换不得修改 `CoreWorld`、Spatial 注册表或快照。
- 单记录查询可以用于调试，但不能作为 1 万或 10 万车辆的默认同步路径。

## 7. v0.7 Bevy specialization

#121 已在 `bevy-reference-adapter.md` 冻结 v0.7 的 Bevy 0.19 支持线、最小 modular dependency graph、专用 fixed schedule、单活动 Session、Vehicle/Entity 部分双射、frame-root/child Transform、placement token 复核、两阶段原子批量提交、可选 Gizmos、最小 native example 与 10k/100k 验证 Gate。

该 specialization 不改变本文的跨引擎权威职责、`f32` canonical 精度边界、稳定批量顺序、失败原子性和宿主类型隔离。v0.7 仍不冻结 presentation interpolation、LOD/pooling、glTF/prefab/scene asset API、WASM、外语绑定的二进制接口、C 外部函数接口（FFI）或第二个 Engine Adapter。

## 8. v0.8 typed lifecycle transaction

#184/ADR 0016 要求 Adapter 为持续场景提供 typed replace-and-rebind 入口，但继续不公开 `&mut CoreWorld`。#187 的 Bevy specialization 公开独占 boundary helper：

```rust
replace_completed_vehicle(
    world: &mut bevy_ecs::world::World,
    old: VehicleHandle,
    input: &VehicleReplaceInput,
) -> Result<LaneFlowVehicleReplaceOutcome, LaneFlowAdapterError>
```

helper 在 Core 提交前以 O(1) 检查既有正向/反向映射和已绑定 Entity 存活性，然后把完整 replacement validation 委托给 `CoreWorld::replace_completed_vehicle`，不在 Adapter 复制 Core preview 或规则。Core 返回 `Replaced` 后，已绑定 old handle 沿稳定容量、不可失败路径轮换到 new handle，并返回包含 `old`、`new`、`Option<Entity>` 的 `LaneFlowVehicleReplaceRecord`；未绑定 old handle 只替换 Core vehicle 并继续保持未绑定。公共 API 不暴露 Core 已成功但 mapping 仍可任意失败的两步协议。

`LaneFlowVehicleReplaceOutcome::Blocked` 是可恢复结果：Core、映射、Transform 和 `last_error` 均不变，调用方可在后续 boundary 复用相同 borrowed input 重试，并继续处理同一 boundary 的其他计划。映射、Entity 或 Core validation 的 fatal error 会写入 Session，停止当前及后续 catch-up step 并保留 backlog；原子性按单条 command 保证，不回滚此前已经成功的其他 command。

Completed vehicle 等待入口期间不进入 pose batch，proxy 保留最后一次合法 Transform；成功 replace 不立即写 Transform，下一次正常 presentation batch 才用 new handle 的入口 pose 更新同一 Entity。Population 的 seed、portal/lane 抽样、pending/retry queue、runtime spawn/despawn 与初始人口仍是 engine-neutral caller-owned authority，不进入 Adapter 或 Bevy ECS；初始人口在 Session 创建前完成。
