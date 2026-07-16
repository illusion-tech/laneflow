# Core ID 与 Handle 模型

**文档状态**: Accepted  
**最后更新**: 2026-07-16  
**适用范围**: v0.2 Lane Graph + Route 的 Core identity、typed handle、registry / resolver、动态 vehicle / route 生命周期和事件 payload 边界  
**关联文档**:

- `../architecture.md`
- `../roadmap.md`
- `../adr/0001-project-scope.md`
- `../adr/0003-runtime-tick-and-determinism.md`
- `../adr/0004-core-implementation-language.md`
- `../adr/0005-core-identity-and-handle-model.md`
- `../adr/0010-parking-binding-and-vehicle-lifecycle-authority.md`
- `core-runtime.md`
- `parking-system.md`

## 1. 目标

本文定义 LaneFlow Core 在 v0.2 阶段的 external ID 与 internal handle 边界，作为 #24 的设计输入。

目标：

- 外部数据、Adapter、debug、日志和用户工具继续使用稳定、可读的 external ID。
- Core runtime 热路径使用 typed handle / compact id，不在每个 tick 中克隆、排序或事件输出字符串 ID。
- 在 `CoreWorld` 初始化阶段完成 external ID 到 handle 的一次性归一化。
- 允许 Core runtime 以确定性方式动态 spawn / despawn vehicle，并支持注册 / 移除 route definition 的最小生命周期边界。
- 通过 registry / resolver 让 Adapter 和调试工具在需要时从 handle 解析回 external ID。
- 为 #29、#30、#32 提供 v0.2 lane graph / route 设计、data format 和 Core 对齐输入。

## 2. 背景

v0.1 Core runtime 中，`VehicleState`、`Route`、`LaneEdge` 和 `CoreEvent` 使用 `String` 表示 `vehicle_id`、`route_id`、`edge_id`。

这对 v0.1 原型可接受，但会在目标规模扩大后形成明确热点：

- `CoreWorld::step` 为保持失败原子性每 tick 克隆 `vehicles`，会连带克隆 `VehicleState.id` 和 `VehicleState.route_id`。
- 每 tick 重新按 `vehicle_id` 字符串排序，以稳定 event order。
- route transition events 拥有多个 `String` payload，大量车辆同 tick 跨 edge 时会放大分配与拷贝。

如果车辆 external ID 高概率是 UUID 字符串，并且目标规模包含 10k vehicles / 60 tick/s，那么这些字符串操作不应留在 runtime 热路径中。

## 3. 术语

- **External ID**：外部边界使用的稳定 ID，例如 `vehicle-001`、UUID 字符串、route id、edge id。用于数据文件、错误信息、日志、debug UI 和 Adapter 绑定。
- **Handle**：Core 内部和 public runtime API 使用的 typed compact id，例如 `VehicleHandle`。handle 只在同一个 `CoreWorld` / simulation session 内有效。
- **Generation**：handle 内部用于识别槽位复用后的过期引用。generation 不暴露为 public API 字段。
- **Registry**：维护 external ID 与 handle 双向映射的结构。
- **Resolver**：从 handle 解析 external ID，或从 external ID 查找 handle 的只读 API。
- **Active handle**：当前 world 中仍指向存活实体的 handle。
- **Stale handle**：曾经有效，但对应实体已经被移除或槽位已复用的 handle。
- **Runtime hot path**：`CoreWorld::step` 及其直接调用链，包括 vehicle update order、route traversal、event payload 构造。

## 4. 设计决策

### D1. external ID 与 runtime handle 必须分层

状态：已接受（ADR 0005）。

v0.2 Core 将 external ID 保留在初始化输入、data-format 边界、validation error、debug 和 resolver API 中。归一化后的 runtime state、route traversal 和 `CoreEvent` payload 应使用 typed handle。

概念模型：

```text
external traffic data
  vehicleId / routeId / edgeId as string
        |
        | CoreWorld initialization + validation
        v
CoreWorld normalized runtime state
  VehicleHandle / RouteHandle / EdgeHandle
        |
        | resolver on demand
        v
Adapter / debug / log external ID
```

Core 不应在 step 中通过 external string ID 做实体定位、事件 payload 构造或车辆更新排序。

### D2. handle 是不透明 typed handle

状态：已接受（ADR 0005）。

v0.2 public API 使用三个互不兼容的 handle 类型。示例写法如下，字段不加 `pub`，因此调用方不能直接构造或读取内部表示：

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

handle 应至少实现：

```rust
fn _assert_handle_traits<T>()
where
    T: Clone + Copy + std::fmt::Debug + PartialEq + Eq + std::hash::Hash,
{
}
```

规则：

- 调用方不得自行构造 handle。
- public API 不暴露稳定的数值含义。
- handle 不可跨 `CoreWorld` 混用或持久化为外部数据格式。
- handle 只表达当前 world 中的实体引用，不表达外部业务身份。
- handle 不提供 public ordering；稳定遍历、事件顺序或 debug 排序必须使用显式 update sequence、external ID 或 resolver 后的业务字段，而不是依赖 handle 的 `Ord`。

v0.2 实现应优先使用 generation-ready 的私有表示。推荐内部模型：

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

public contract 不暴露 `index` 或 `generation` 字段。调用方只能把 handle 作为 typed token 传回 Core 或用于 resolver 查询。

分层原因：

- Vehicle 是运行时动态实体。v0.2 应允许 spawn / despawn vehicle，因此 `VehicleHandle` 必须能识别 stale handle。
- Route definition 可能在运行时按需求注册或移除。若 route registry 支持复用槽位，`RouteHandle` 也应使用 generation。
- Lane graph / edge 在 v0.2 仍按初始化后稳定处理。动态道路拓扑会影响 occupancy、route validity、Adapter mesh 和增量 validation，应单独设计，不塞进 #24 的最小交付。

如果实现阶段为了更小步交付暂不复用空槽，仍不得把 public API 暴露成裸 `u32`。handle opaque 是允许后续从 dense index 升级为 generation handle 的硬性边界。

### D3. external ID 继续使用字符串，不引入 UUID 专用类型

状态：已接受（ADR 0005）。

v0.2 Core 不直接依赖 `uuid` crate，也不把 vehicle ID 特化成 `Uuid` 或 `u128`。

原因：

- route id、edge id、vehicle id 需要统一处理，字符串 external ID 对数据格式和调试最直观。
- 是否使用 UUID 是调用方或 data-format 的策略，不应强行进入 Core runtime 热路径。
- `String` 只保留在 registry 和 validation / debug 边界，不再进入 step 热路径。
- 避免过早引入额外运行时依赖和许可证评审成本。

external ID 规则由 #30 冻结。#24 的默认输入是：

- external ID 是 UTF-8 string。
- 同一 domain 内唯一，例如 vehicle id 集合内唯一、route id 集合内唯一、edge id 集合内唯一。
- 不同 domain 可以使用相同字符串，因为 `VehicleHandle`、`RouteHandle`、`EdgeHandle` 的类型空间不同。
- 比较默认大小写敏感，不做 trim、case fold 或 Unicode normalization。

### D4. registry / resolver 是 CoreWorld 的一部分

状态：已接受（ADR 0005）。

`CoreWorld` 初始化时应构建 domain-specific registry。registry 同时负责 active lookup 与 handle validation：

```text
VehicleRegistry
  slots: Vec<VehicleSlot>
  lookup: externalId -> active VehicleHandle
  freeList: reusable slot indices

RouteRegistry
  slots: Vec<RouteSlot>
  lookup: externalId -> active RouteHandle
  freeList: reusable slot indices

EdgeRegistry
  externalIds: Vec<String>
  lookup: externalId -> EdgeHandle
```

推荐 public resolver API：

```rust
impl CoreWorld {
    pub fn vehicle_external_id(&self, _handle: VehicleHandle) -> Option<&str> {
        todo!()
    }

    pub fn route_external_id(&self, _handle: RouteHandle) -> Option<&str> {
        todo!()
    }

    pub fn edge_external_id(&self, _handle: EdgeHandle) -> Option<&str> {
        todo!()
    }

    pub fn vehicle_handle(&self, _external_id: &str) -> Option<VehicleHandle> {
        todo!()
    }

    pub fn route_handle(&self, _external_id: &str) -> Option<RouteHandle> {
        todo!()
    }

    pub fn edge_handle(&self, _external_id: &str) -> Option<EdgeHandle> {
        todo!()
    }
}
```

`Option` 只表示该 handle 或 external ID 在当前 world 中无法解析为 active entity。调用方仍不得把其他 world 的 handle 当作可验证输入；wrong-world handle 不属于 v0.2 的稳定行为边界。

对需要区分错误原因的修改型 API，应返回明确错误，而不是只返回 `None`：

```rust
pub enum HandleLookupError {
    Unknown,
    Stale,
    WrongDomain,
}
```

`WrongDomain` 只能在类型系统无法防止的 FFI / binding 层出现。Rust API 应通过 `VehicleHandle`、`RouteHandle`、`EdgeHandle` 的类型隔离避免 domain 混用。

### D5. runtime state 使用 handle 引用

状态：已接受（ADR 0005）。

归一化后的 Core runtime state 不应保存 external route / edge string 引用。

概念模型：

```text
VehicleRuntimeState
  handle: VehicleHandle
  route: RouteHandle
  routeEdgeIndex
  edgeProgress
  speed
  status

RouteRuntime
  handle: RouteHandle
  edgeHandles: Vec<EdgeHandle>

LaneEdgeRuntime
  handle: EdgeHandle
  length
  nextEdgeHandles: Vec<EdgeHandle>
```

初始化输入可以继续使用 external IDs：

```text
VehicleInput
  externalId
  routeExternalId
  routeEdgeIndex
  edgeProgress
  speed
  status

RouteInput
  externalId
  edgeExternalIds

LaneEdgeInput
  externalId
  length
  nextEdgeExternalIds
```

具体类型命名由 #29、#30 和 #32 固化。本文只要求初始化输入与 runtime state 分层。

### D6. vehicle / route 生命周期属于 Core runtime 契约

状态：已接受（ADR 0005）。

运行时增加和移除 vehicle 是 LaneFlow 的基础能力。v0.2 设计应定义最小生命周期 API，而不是把所有 vehicle 固定在初始化输入里。

推荐最小 API：

```rust
impl CoreWorld {
    pub fn spawn_vehicle(&mut self, _input: VehicleSpawnInput) -> Result<VehicleHandle, CoreError> {
        todo!()
    }

    pub fn despawn_vehicle(
        &mut self,
        _vehicle: VehicleHandle,
    ) -> Result<VehicleDespawnRecord, CoreError> {
        todo!()
    }

    pub fn register_route(&mut self, _input: RouteInput) -> Result<RouteHandle, CoreError> {
        todo!()
    }

    pub fn remove_route(&mut self, _route: RouteHandle) -> Result<RouteRemoveRecord, CoreError> {
        todo!()
    }
}
```

生命周期规则：

- lifecycle API 只能在 `step` 调用之间执行，不允许在一次 `step` 中间隐式修改实体集合。
- 单条 lifecycle command 必须原子化：失败不得留下部分 registry 或 runtime state。
- 批量 command 若后续需要，应按输入顺序验证和应用，并明确是否 whole-batch atomic。
- `spawn_vehicle` 必须分配 active `VehicleHandle`，并把 external ID 注册进 `VehicleRegistry`。
- `despawn_vehicle` 必须让旧 `VehicleHandle` 变为 stale；若槽位复用，新 handle 必须拥有不同 generation。
- `despawn_vehicle` 应返回 `VehicleDespawnRecord`，至少包含被移除 vehicle 的 handle 和 external ID。这样 debug / Adapter 可以记录生命周期结果，而 active resolver 不需要继续把 stale handle 当成可解析实体。
- `register_route` 必须把 route 的 external edge IDs 解析为 `EdgeHandle` 序列，并执行 route connectivity validation。
- `remove_route` 只能移除没有 active vehicle 引用的 route；否则返回明确错误，避免让运行中车辆悬空。
- `remove_route` 应返回 `RouteRemoveRecord`，至少包含被移除 route 的 handle 和 external ID，避免删除后依赖 active resolver 取回诊断信息。
- v0.2 不支持动态新增 / 删除 edge 或修改 lane graph 拓扑。动态道路网络属于单独设计范围。

`RouteHandle` 表示 world-level route definition。对于车辆临时路径，调用方可以选择先注册 route definition 再分配给 vehicle；是否需要 per-vehicle route plan 类型由 #29 / #32 在 route system 设计中细化。

### D7. CoreEvent payload 使用 handle

状态：已接受（ADR 0005）。

v0.2 `CoreEvent` payload 不再拥有 external ID 字符串。事件应携带 handle 和必要的 route edge index：

```rust
pub struct VehicleChangedEdgeEvent {
    pub tick_index: u64,
    pub vehicle: VehicleHandle,
    pub route: RouteHandle,
    pub from_edge: EdgeHandle,
    pub to_edge: EdgeHandle,
    pub from_route_edge_index: usize,
    pub to_route_edge_index: usize,
}

pub struct VehicleCompletedRouteEvent {
    pub tick_index: u64,
    pub vehicle: VehicleHandle,
    pub route: RouteHandle,
    pub edge: EdgeHandle,
    pub route_edge_index: usize,
}
```

Lifecycle command 可以返回专门的 command record，例如：

```rust
pub struct VehicleDespawnRecord {
    pub vehicle: VehicleHandle,
    pub external_id: String,
}

pub struct RouteRemoveRecord {
    pub route: RouteHandle,
    pub external_id: String,
}
```

这类 record 不属于每 tick route traversal event 的 hot-path payload。它用于命令调用方记录实体生命周期，避免要求 active resolver 对 stale handle 继续返回 external ID。

Adapter、debug UI 或日志如需 external ID，应通过 resolver 查询：

```rust
if let Some(vehicle_id) = world.vehicle_external_id(event.vehicle) {
    // 使用 external vehicle id
}
```

这会改变 v0.1 `CoreEvent` 的 public payload，属于 Core API breaking change。实现 PR 必须按 `docs/reference/commit-convention.md` 标记 breaking change，或明确记录 v0.1 prototype 迁移边界。

### D8. stable update order 使用确定性 update key

状态：已接受（ADR 0005）。

v0.1 每 tick 通过 `vehicle_id` 字符串排序稳定事件顺序。v0.2 应改为维护稳定的 update order，而不是每 tick 排序 external string。

```text
vehicleUpdateOrder: Vec<VehicleHandle>
```

默认规则：

- 初始化车辆按 vehicle external ID 的 Rust `str` 字典序分配初始 update sequence，保证输入顺序不影响结果。
- runtime spawn 的车辆按 lifecycle command 应用顺序分配单调递增 update sequence。
- `vehicleUpdateOrder` 按 update sequence 维护；每 tick 直接遍历 active handles。
- despawn 车辆必须从 active update order 中移除，或标记为 inactive 并在遍历时跳过；无论采用哪种实现，事件顺序必须稳定且有测试覆盖。
- 同一 vehicle 在同一 tick 内按实际 route transition 顺序输出事件。

这样保留 v0.1 的输入顺序独立性，同时支持 runtime spawn，而不会把 external ID 字符串排序留在 tick 热路径。

### D9. validation error 保留 external ID，runtime error 可使用 handle

状态：已接受（ADR 0005）。

初始化 validation error 面向数据作者和调用方，应继续携带 external ID，例如 duplicate vehicle id、unknown edge id、disconnected route edge。

runtime step error 如果发生在已归一化 state 上，可以携带 handle。调用方可在错误发生后通过同一个 `CoreWorld` resolver 查询 external ID。

推荐规则：

- 初始化阶段：错误携带 external ID。
- external command / lookup 阶段：错误携带调用方传入的 external ID。
- step 阶段：错误优先携带 handle，不在错误构造中分配 external string。
- `Display` 文案仍使用中文优先；如果错误只含 handle，应输出 handle 的 debug 表示，并提示可通过 resolver 查 external ID。

### D10. v0.2 暂不引入 handle 管理 crate

状态：已接受（ADR 0005）。

v0.2 优先使用自有的最小 typed handles 和 registry，不引入 `slotmap`、`generational-arena` 等 handle 管理 crate。

原因：

- LaneFlow 只需要少数 domain-specific typed handles、resolver 和生命周期规则，不需要通用 arena 的完整抽象面。
- 自有 opaque typed handle 可以保持 public API 极小，并避免过早冻结第三方 crate 的 key 类型、生命周期模型和许可证评审。
- Vehicle / route 的 generation 规则足够小，可以在 Core 内部实现并用 focused tests 锁定。

如果 v0.3+ 引入更复杂的动态道路拓扑、parking reservation、跨模块长期缓存或大量实体类型，再重新评估 `slotmap`、`generational-arena` 等成熟 crate。评估应以实际生命周期需求为输入，而不是提前把通用 arena 抽象带入 v0.2。

## 5. 初始化流程

推荐归一化流程：

1. 校验 external ID 非空、domain 内唯一。
2. 为 edge external IDs 分配 `EdgeHandle`。
3. 将 lane edge 的 `nextEdgeExternalIds` 解析为 `EdgeHandle`。
4. 为 route external IDs 分配 `RouteHandle`。
5. 将 route 的 edge sequence 解析为 `EdgeHandle`，并执行 route connectivity validation。
6. 为初始 vehicle external IDs 分配 generation-ready `VehicleHandle`。
7. 将 vehicle 的 route reference 解析为 `RouteHandle`。
8. 校验 vehicle 初始 `routeEdgeIndex`、`edgeProgress`、`speed` 和 `status`。
9. 按初始 vehicle external ID 字典序分配 update sequence，并生成 `vehicleUpdateOrder`。
10. 构造不含 external string hot-path 引用的 runtime state。

初始化失败不得返回部分可用 `CoreWorld`。

运行期 lifecycle command 流程：

1. 校验 command 输入的 external ID、route reference、edge sequence 或 handle。
2. 对 handle 输入执行 active / stale 检查。
3. 对 route 变更执行 connectivity validation。
4. 按 command 顺序分配 handle / generation / update sequence。
5. 更新 registry、runtime state 和 `vehicleUpdateOrder`。
6. 返回 handle 或 lifecycle record。

单条 command 失败不得改变 `CoreWorld`。批量 command 的原子性由后续 API 设计明确，不能隐式部分成功。

## 6. 性能边界

v0.2 的最低性能目标：

- `CoreWorld::step` 不按 external string 排序。
- `CoreWorld::step` 不为 event payload 克隆 external strings。
- `CoreWorld::step` 不因 route / edge / vehicle 定位执行 external string lookup。
- `CoreWorld::step` 不处理 hidden spawn / despawn side effect；动态生命周期必须通过显式 command 进入 Core。
- 为失败原子性保留的临时 state clone 只能克隆 compact runtime state，不得克隆 external strings。

v0.2 可以暂时接受每 tick 克隆 compact `VehicleRuntimeState` 来保持 step 原子性。若 10k vehicles / 60 tick/s 下 compact state clone 仍成为热点，应单独拆性能 issue 评估 patch / compute-then-apply 策略。

## 7. 测试策略

实现 #32 时至少覆盖：

- 同一 external ID 输入无论车辆输入顺序如何，event order 都按初始化生成的 stable update order 输出。
- runtime spawn 的 vehicle 按 command 应用顺序获得稳定 update sequence。
- despawn 后旧 `VehicleHandle` 变为 stale；槽位复用时新 handle 的 generation 不同。
- stale `VehicleHandle` 不能读取或修改 active vehicle state。
- `despawn_vehicle` 返回的 lifecycle record 可提供被移除 vehicle 的 external ID。
- `CoreEvent` payload 只包含 handles 和 route edge index，不包含 external ID strings。
- `world.vehicle_external_id(handle)`、`world.route_external_id(handle)`、`world.edge_external_id(handle)` 可解析合法 handle。
- `world.vehicle_handle(id)`、`world.route_handle(id)`、`world.edge_handle(id)` 可反查合法 external ID。
- unknown external ID lookup 返回 `None`。
- stale handle 的 active resolver 返回 `None`，修改型 API 返回明确 stale handle error。
- 初始化 validation error 仍携带 external ID，便于定位数据错误。
- route transition 和 completion 事件可通过 resolver 转回 v0.1 等价 external ID 断言。
- deterministic tests 不依赖 `HashMap` 等无稳定迭代顺序集合。

## 8. 对后续 issue 的要求

#29 lane graph / route system 设计：

- 应明确 edge / route external ID 的数据模型和拓扑引用方式。
- 应说明 route edge sequence 中重复 edge 是否继续通过 `routeEdgeIndex` 区分位置。
- 应区分 world-level route definition 与 per-vehicle route plan。如果 route 可运行期注册 / 移除，应定义 active vehicle 引用时的 remove 拒绝规则。

#30 data format：

- 应冻结 external ID 字段名、唯一性、字符约束、版本策略和兼容性边界。
- 应说明 external ID 与 runtime handle 不等价，handle 不进入数据文件。
- 应说明 initial traffic data 与 runtime lifecycle command data 是否分层；如果 v0.2 不冻结 command data format，必须明确不做范围。

#31 validation：

- 应按初始化 validation、data-format validation 和 runtime validation 分层。
- 应明确哪些错误保留 external ID，哪些错误可以使用 handle。
- 应新增 active / stale handle、duplicate external ID、route in use、unknown route / edge 等 lifecycle validation 错误。

#32 Core 对齐：

- 应新增 handle types、registry / resolver、normalized runtime state 和 handle-only events。
- 应记录 Core API breaking change 或 v0.1 prototype 迁移边界。
- 应实现或显式拆出最小 vehicle spawn / despawn 和 route register / remove API；若拆出，必须说明 #24 的 handle 设计如何保证后续兼容。

## 9. 设计审阅结论

本设计按以下标准审阅：

- Core 热路径不依赖 external string clone / sort / lookup。
- 动态 vehicle 生命周期不被延后到会破坏 handle contract 的后续变更。
- 动态 route definition 与动态 lane graph 拓扑分层处理。
- handle 对调用方保持 opaque，允许内部 representation 演进。
- stale handle 有明确错误语义，而不是误命中新实体。
- Adapter / debug 仍可通过 resolver 或 lifecycle record 获得 external ID。
- 确定性 update order 不依赖 `HashMap` 迭代顺序或输入数组顺序。

审阅结论：

- 对 dynamic vehicle：采用 generation-ready `VehicleHandle` 是正确做法。运行时 spawn / despawn 是 LaneFlow 基础能力，若只用裸 index 会在槽位复用和外部缓存场景下产生 stale handle 风险。
- 对 dynamic route：支持 route definition 运行期注册 / 移除是合理边界，但必须限制 `remove_route` 不得移除仍被 active vehicle 引用的 route。
- 对 dynamic lane graph：不放入 v0.2 是合理取舍。运行时修改道路拓扑会牵动 route validity、occupancy、Adapter mesh 和增量 validation，应作为独立高风险设计。
- 对第三方 crate：v0.2 暂不引入通用 arena crate 是可接受的最佳实践。当前 domain 少、规则明确，自有 opaque typed handle + generation 更小、更容易审计；如果后续实体类型和动态拓扑复杂度明显上升，再评估成熟 crate。

因此，#24 推荐方案是：**Vehicle / Route handle 从设计上支持 generation，Edge handle 暂按静态 lane graph dense handle 处理；动态 vehicle 和 route definition 属于 Core runtime 契约，动态道路拓扑另行设计。**

## 10. ADR 判断

Core identity / handle 模型影响 Core API、data-format 输入和后续 Adapter resolver 边界，属于高影响设计决策。本文配套 `../adr/0005-core-identity-and-handle-model.md`，ADR 0005 状态为 `Accepted`，作为 #24 的 G1 冻结决策依据。

## 11. Planned v0.5 Parking extension

`parking-system.md` 在不改变本文 external-ID/opaque-handle 原则的前提下，planned 增加 static dense `ParkingAreaHandle` / `ParkingSpaceHandle`、immutable registry/resolver 与 Parking lifecycle records。Parking handles 不持久化、没有 public ordering；dynamic vehicle 继续使用 generation handle。Parking binding 是 Core 私有 aggregate，不进入 handle本身、VehicleState或Adapter。#105 只冻结设计，实际 API 与 lifecycle substrate 分别由 #106-#109 交付。
