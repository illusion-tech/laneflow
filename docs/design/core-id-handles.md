# Traffic Runtime 标识与句柄模型

**文档状态**: Accepted<br>
**最后更新**: 2026-11-09<br>
**适用范围**: 稳定标识、共享路网修订内的有类型序号、每世界代际句柄、车辆与路线生命周期，以及热路径的标识边界<br>
**关联文档**:

- `../architecture.md`
- `../adr/0003-runtime-tick-and-determinism.md`
- `../adr/0005-core-identity-and-handle-model.md`
- `../adr/0010-parking-binding-and-vehicle-lifecycle-authority.md`
- `../adr/0016-scenario-population-and-recycle-lifecycle-authority.md`
- `../adr/0017-static-road-junction-maneuver-and-gate-identity.md`
- `../adr/0020-compiler-owned-static-network-and-static-image.md`
- `../adr/0029-retire-precompiled-static-route.md`
- `../reference/glossary.md`
- `network-compiler.md`
- `shared-static-network.md`
- `traffic-runtime-shared-consumption.md`
- `traffic-runtime-snapshot.md`
- `parking-system.md`
- `road-junction-model.md`

## 1. 目标

本文定义 `TrafficWorld` 与共享静态路网的标识分层：

- 编制来源、诊断和跨修订翻译使用稳定标识，运行时热路径只使用有类型序号与句柄，不在
  固定步进中克隆、排序或查找字符串标识。
- 静态路网的序号由编译器和共享静态路网确定，不在世界安装时重新分配。
- 车辆与路线可以在步进之间以确定性方式动态增删，由代际句柄识别过期引用。

## 2. 标识分层

| 层级           | 形式                                                                | 有效范围               | 分配者                |
| -------------- | ------------------------------------------------------------------- | ---------------------- | --------------------- |
| 稳定标识       | `StableId128`，由规范标识元组派生                                   | 跨修订；不证明语义兼容 | 编译器                |
| 修订内序号     | 有类型 `u32` 序号（如 `LaneEdgeOrdinal`、`VehicleProfileOrdinal`）  | 单个共享路网修订       | 编译器 / 共享静态路网 |
| 所有者局部键   | 有类型 `(ownerOrdinal, role, localIndex)`                           | 所属序列的当前快照     | 编译器                |
| 每世界代际句柄 | `VehicleHandle`、`RouteHandle`（`{ index, generation }`，字段私有） | 单个 `TrafficWorld`    | `TrafficWorld`        |

规则：

- 所有共享静态路网表行使用修订内序号；只有稳定声明实体与可寻址派生实体另有
  `StableId128`。
- 静态与共享标识合同位于 `laneflow-static-contract`，不由 Runtime 拥有。
- 所有共享修订都必须保留冷的共享标识索引（`SharedIdentityIndex`），为快照恢复、
  动态路线重建和修订切换提供 `StableId128` 与序号之间的双向翻译。它不进入稳态
  tick，也不能证明同一 `StableId128` 在两个路网修订间语义兼容。
- 跨修订状态迁移必须使用由可信切换输入绑定的语义差异，不能采用仅凭索引的回退。
- 显示名、来源位置和完整规范元组等诊断数据由可移植制品、源映射或独立诊断制品外置
  提供，不进入共享静态路网。
- 路线与车辆的可读标识归调用方。`register_route` 只接收边序号，catalog `route_id`
  由调用方持有；Runtime 没有路线或车辆的外部字符串解析器（ADR 0029）。

## 3. 代际句柄

`VehicleHandle` 与 `RouteHandle` 是互不兼容的不透明类型，实现
`Clone + Copy + Debug + Eq + Hash`。

- 调用方不得自行构造句柄；公开 API 不暴露 `index` 或 `generation`，也不赋予数值稳定
  含义。
- 句柄只在产生它的 `TrafficWorld` 内有效，不编码世界身份；跨世界混用是调用方错误。
  修订切换期间句柄的延续规则见 `traffic-runtime-shared-consumption.md`。
- 句柄不进入编制来源与路网制品。运行时快照的保存边界见 `traffic-runtime-snapshot.md`。
- 句柄不提供公共排序；稳定遍历与事件顺序必须使用显式更新序，不得依赖句柄的数值或
  `Ord`。
- 槽位复用时新句柄必须拥有不同 generation，使旧句柄成为 stale。

继续使用项目自有的最小句柄，不引入 `slotmap`、`generational-arena` 等通用句柄管理
crate，也不依赖 `uuid` 或把标识特化成 UUID。

## 4. 车辆与路线生命周期

生命周期命令（`register_route`、`remove_route`、`spawn_vehicle`、
`replace_completed_vehicle`、停车生命周期与 `despawn_vehicle`）的现行签名见
`traffic-runtime-shared-consumption.md`。

- 生命周期命令只能在 `step` 调用之间执行，不允许在一次 `step` 中隐式修改实体集合。
- 单条命令必须原子：失败不得改变已提交世界。批量命令若后续需要，必须明确是否整批
  原子，不能隐式部分成功。
- 成功移除后旧句柄 stale；若槽位复用，新句柄必须拥有不同 generation。
- `remove_route` 只能移除没有 live 车辆引用的路线，否则返回明确错误，不让运行中车辆
  悬空。
- `RouteHandle` 表示世界级路线定义。
- 车道图拓扑在共享路网修订内不可变；拓扑变化只通过修订切换事务进入运行世界。

## 5. 确定性更新序

`live_order` 是车辆更新的确定性顺序：

- `spawn_vehicle` 按命令应用顺序在尾部追加，`update_sequence` 由此得出。
- 每 tick 直接遍历 `live_order`，不按字符串排序，也不依赖 `HashMap` 迭代或输入数组
  偶然顺序。
- 车辆移除后必须从更新序中摘除，事件顺序保持稳定。
- `replace_completed_vehicle` 尽量保留旧车在 `live_order` 中的位置，不产生 tombstone。

## 6. 错误边界

- 静态不变量由编译器诊断报告，携带来源位置与稳定键。
- 运行时错误可以携带句柄或序号，不在错误构造路径中为定位而分配字符串。
- 需要区分原因的修改型 API 返回明确错误；无法解析的只读查询返回 `None`。

## 7. 性能边界

- `step` 不按外部字符串排序，不为事件载荷克隆字符串，不因路线、边或车辆定位执行
  字符串查找。
- `step` 不处理隐藏的生成或移除；动态生命周期必须通过显式命令进入。

## 8. 静态对象标识

路口、机动路径、机动门、信号与停车等静态对象都是共享路网修订内的有类型序号
（`JunctionOrdinal`、`MovementOrdinal`、`ManeuverPathOrdinal`、`ManeuverGateOrdinal`、
`StopLineOrdinal`、`SignalGroupOrdinal`、`ParkingSpaceOrdinal` 等），不需要 generation
生命周期，也不提供具有业务含义的公共排序。路口层细节见 `road-junction-model.md` §4，
停车绑定与生命周期记录见 `parking-system.md`。

路线注册期把机动路径和机动门编译为路线共享的出现记录；稳态 tick 不查字符串标识、
不按序号数值排序，也不扫描全局表。

## 9. 完成车辆的原子替换

ADR 0016 规定的回流入口钉在 `TrafficWorld`：

```rust
replace_completed_vehicle(
    old: VehicleHandle,
    input: VehicleSpawnInput,
) -> Result<VehicleReplaceRecord, ReplaceError>
```

命令只接受 live、未绑定 Parking 的 `Completed` 车辆，并创建 `Active` 替换车。
`VehicleSpawnInput` 携带 profile、`RouteHandle`、`route_edge_index`、edge progress 与
initial speed，因此调用方可在 typed `Blocked` 后复用同一输入。Runtime 没有外部字符串
标识，不把 `Preserve | ReplaceWith` 搬进生产入口。

成功返回 `VehicleReplaceRecord { old, new }`，old 立即 stale，new handle 必须不同；公开
合同不保证相同 slot index。到达路线终点写成 `Completed`，保留槽位与句柄直到 replace；
不进 pose、不占车道，占车辆容量。不得用移除后再 `spawn` 充当回流。

只有物理 overlap 返回可恢复的 typed `ReplaceError::Blocked`，载荷含 old/blocker handle、
前后关系和 bumper gap。stale、非 Completed、Parking 占用、profile、route、
`route_edge_index`、progress、speed、限速、准入和容量错误均为致命 `ReplaceError`。任一
blocked/fatal 结果都保持已提交世界不变。

重叠准入使用路线起点截断后的非空车身区间，并拒绝重合的零进度前杠入口点；多个 blocker
按几何规则选择（离前杠最近者），与槽位分配无关、快照恢复前后一致。该规则不增加句柄公共
排序能力，详见 `vehicle-following.md` §7.4。

`TrafficWorld` 不选择回流 portal、lane 或 route，不拥有目标人口、seed、PRNG、pending
retry 或车辆数量上限，也不接触 Entity。走廊、城市游戏等调用方拥有 lifecycle policy；
成功 record 足以让 Adapter 事务原子切换 binding。详细职责见
`../adr/0016-scenario-population-and-recycle-lifecycle-authority.md` 与
`example-scenarios.md`。

## 10. 测试要求

- 输入顺序不同但命令序列相同时，更新序与事件顺序保持确定。
- 移除后旧句柄 stale；槽位复用时新句柄 generation 不同；stale 句柄不能读取或修改活动
  车辆状态。
- `remove_route` 拒绝仍被 live 车辆引用的路线。
- 确定性测试不依赖 `HashMap` 等无稳定迭代顺序的集合。
