# 0005 Core Identity and Handle Model

**状态**: Accepted  
**日期**: 2026-06-24  
**适用范围**: LaneFlow Core 的 external ID、typed handle、registry / resolver、动态 vehicle / route 生命周期和事件 payload 边界  
**关联文档**:

- 上游决策:
  - `0001-project-scope.md`
  - `0002-dependency-and-licensing-constraints.md`
  - `0003-runtime-tick-and-determinism.md`
  - `0004-core-implementation-language.md`
- 相关设计:
  - `../design/core-runtime.md`
  - `../design/core-id-handles.md`

## 背景

v0.1 Core runtime 使用 `String` 表示 `vehicle_id`、`route_id` 和 `edge_id`。这让原型容易阅读和测试，但会在 runtime 热路径中引入字符串 clone、字符串排序和事件 payload 分配。

LaneFlow 需要支持 10k vehicles / 60 tick/s 这类目标规模。若车辆 ID 是 UUID 字符串，继续在每个 tick 中复制和排序外部字符串，会把 identity 表达方式变成性能热点。

同时，LaneFlow 仍需要外部数据、Adapter、debug UI 和日志保留稳定、可读的 external ID。Core 不能只暴露裸整数而丢失可诊断性。

运行时 spawn / despawn vehicle 是 NPC 车流 runtime 的基础能力。类似园区、停车场、数字孪生或城市片区模拟的场景通常需要持续生成车辆、车辆完成路线后离开系统，以及按工具或 Adapter 命令注入车辆。若 handle 只采用裸 dense index，删除后复用槽位会让旧 handle 误命中新实体。

运行时注册 route definition 也是常见能力，但动态修改 lane graph / road network 拓扑会进一步影响 route validity、occupancy、Adapter mesh 和增量 validation。该问题应与 vehicle 生命周期分层处理。

## 决策

LaneFlow Core 采用双层 identity 模型：

- 外部边界使用稳定、可读的 external ID string。
- Core runtime state、route traversal、event payload 和 hot path entity reference 使用 typed handle。
- `CoreWorld` 初始化阶段构建 registry，把 external ID 归一化为 handle。
- Adapter、debug 和日志通过 resolver 从 handle 查询 external ID。

v0.2 定义的 handle 类型包括：

```rust
pub struct VehicleHandle {
    index: u32,
    generation: u32,
}

pub struct RouteHandle {
    index: u32,
    generation: u32,
}

pub struct EdgeHandle {
    index: u32,
}
```

handle 是不透明 typed handle。上述字段保持私有，只表达推荐内部表示；调用方不得自行构造，也不得跨 `CoreWorld` 混用或持久化到数据文件。

v0.2 的 `VehicleHandle` 和 `RouteHandle` 应采用 generation-ready 的内部表示，用于识别 stale handle。`EdgeHandle` 在 v0.2 仍可采用 dense index，因为 lane graph / edge 拓扑按初始化后稳定处理。所有字段保持私有，不暴露 `index` 或 `generation`。

v0.2 定义最小动态生命周期边界：

- Core runtime 支持 spawn / despawn vehicle。
- Core runtime 支持 register / remove route definition。
- `remove_route` 不得移除仍被 active vehicle 引用的 route。
- vehicle / route removal 应返回 lifecycle record，保留被移除实体的 handle 与 external ID，避免删除后依赖 active resolver 取回诊断信息。
- 动态新增 / 删除 edge 或修改 lane graph 拓扑不纳入本 ADR，应单独设计。

v0.2 不直接引入 `slotmap`、`generational-arena` 等 handle 管理 crate。LaneFlow 当前只需要少数 domain-specific typed handles、resolver 和生命周期规则，自有 opaque typed handle + generation 足以覆盖 #24 的需求。若后续引入动态道路拓扑、parking reservation、更多实体类型或跨模块长期缓存，再基于实际生命周期需求评估成熟 arena crate。

Core 不引入 `uuid` crate。UUID 可以作为 external ID 字符串存在，但 Core runtime hot path 不依赖 UUID 解析或 UUID 专用类型。

## 后果

正向后果：

- `CoreWorld::step` 不再因车辆更新顺序而每 tick 排序 external string。
- `CoreEvent` payload 不再拥有多个 `String` 字段。
- route / edge / vehicle runtime reference 可以使用 compact Copy 类型。
- 运行时 vehicle spawn / despawn 可以识别 stale handle，避免旧 handle 误命中新实体。
- route definition 可以在运行期注册 / 移除，同时通过引用检查避免 active vehicle 悬空。
- removal lifecycle record 可以保留可诊断 external ID，而不要求 active resolver 对 stale handle 继续返回实体。
- Adapter 仍可通过 resolver 获得可读 external ID。
- data-format 可以继续使用稳定字符串 ID，不暴露 runtime handle。

成本和风险：

- 这是 Core API breaking change。v0.1 直接读取 `VehicleState.id` 或 `CoreEvent.vehicle_id` 的调用方需要改用 handle + resolver。
- handle 是 world-scoped，调用方必须避免跨 world 混用。
- generation 可以解决同一 world 内槽位复用导致的 stale handle 问题，但不单独解决跨 `CoreWorld` 混用 handle；wrong-world handle 仍属于调用方错误或后续 binding 层问题。
- 动态 lane graph / edge 拓扑仍未解决；相关能力需要后续设计。
- 初始化阶段需要 registry 构建和 external ID 唯一性校验。
- lifecycle command 需要新增 active / stale handle、duplicate external ID、route in use 等验证路径。

## 替代方案

### 继续在 runtime 中使用 String

优点是最简单，debug 直接可读。

缺点是保留当前 clone、sort 和 event allocation 热点，不适合 10k vehicles / 60 tick/s 目标。

### 把 UUID / u128 作为 Core ID

优点是 vehicle ID 可能更紧凑，且适合部分业务系统。

缺点是 route / edge 不一定是 UUID，Core 会过早绑定调用方 ID 策略，也无法解决每 tick 排序和事件 payload 需要 typed domain 的问题。

### 只使用裸 dense index handle

优点是最简单，内存最小。

缺点是无法安全支持 runtime despawn / slot reuse。旧 `VehicleHandle(42)` 可能在车辆删除后误命中新车辆，不适合作为 LaneFlow 的基础 runtime 契约。

### 立即采用 generational arena crate

优点是动态插入、删除和 stale handle 检测能力成熟。

缺点是 v0.2 的实体 domain 少，Core 还需要 external ID registry、route 引用检查和 Adapter resolver 等项目特定规则。直接引入会扩大依赖面并过早冻结 key 模型。LaneFlow 应先用最小 opaque typed handle + generation 满足 v0.2 的动态 vehicle / route 生命周期。

## 状态说明

本 ADR 为 #24 的 G1 冻结决策依据，覆盖 Core identity、typed handle、registry / resolver 和动态 vehicle / route lifecycle。
