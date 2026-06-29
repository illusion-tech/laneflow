# Lane Graph 设计

**文档状态**: Accepted  
**最后更新**: 2026-06-29  
**适用范围**: v0.2 Lane Graph + Route 的 Core lane graph 模型、edge / connection / topology 约束和 data-format 输入边界  
**关联文档**:

- `core-runtime.md`
- `core-id-handles.md`
- `route-system.md`
- `../adr/0003-runtime-tick-and-determinism.md`
- `../adr/0005-core-identity-and-handle-model.md`
- `../roadmap.md`

## 1. 目标

本文固化 v0.2 阶段 Core 可消费的 lane graph 设计，作为 #29 的 G1 冻结输入。

目标：

- 定义最小正式 lane graph model。
- 明确 edge、connection、topology validation 和 traversal 输入。
- 对齐 #24 已接受的 external ID / typed handle / registry / resolver 决策。
- 为 #30 data format、#31 validation 和 #32 Core 对齐提供可引用输入。

非目标：

- 不冻结完整道路几何、mesh、样条曲线或坐标系统。
- 不设计 pathfinding、route planner、lane change planner 或随机路线选择。
- 不设计 signals、parking、vehicle following、intersection priority 或 conflict zone。
- 不支持运行时新增、删除或修改 lane graph 拓扑。
- 不把 `IndexMap`、具体 Rust 容器或序列化字段名冻结为长期 data spec。

## 2. 术语

- **Lane graph**：Core 用于验证 route 和进行 route traversal 的有向图。
- **Lane edge**：Core route following 的最小有向行驶段。
- **Lane connection**：从一个 lane edge 到另一个 lane edge 的显式可通行关系。
- **External edge ID**：数据文件、工具、日志、debug 和 Adapter 使用的稳定字符串 ID。
- **EdgeHandle**：Core runtime 内部和 public runtime API 使用的不透明 typed handle。
- **Distance unit**：引擎无关距离单位。示例可按 meter 理解，但 Core 不绑定具体单位制。
- **Boundary epsilon**：edge boundary、最小 edge length 和 progress snap 使用的统一容差。

## 3. 设计决策

### D1. lane graph 是有向 edge graph

状态：已接受。

v0.2 lane graph 使用 directed edge graph。每个 lane edge 表示一个方向上的可行驶段；双向道路必须建模为两条或多条方向相反的 edge。

概念模型：

```text
LaneGraphInput
  edges: LaneEdgeInput[]

LaneEdgeInput
  externalId: string
  length: distance_units
  connections: LaneConnectionInput[]

LaneConnectionInput
  toEdgeExternalId: string
```

说明：

- `length` 是 Core route following 的权威长度。
- `connections` 只表达可从当前 edge 进入哪些下游 edge。
- 没有 outgoing connection 的 edge 是合法 terminal edge。
- route 的实际行驶顺序由 route edge sequence 决定，不由 connection 顺序隐式选择。
- 两个物理上不同但连接同一对 edge 的通行方式，应拆成独立中间 edge；v0.2 不引入 connection ID 或多重 edge-to-edge connection。

### D2. external ID 与 EdgeHandle 分层

状态：已接受，依据 ADR 0005。

lane graph 输入、data format、debug、日志和 Adapter 绑定使用 external edge ID。CoreWorld 初始化时把 external edge ID 归一化为 `EdgeHandle`。

v0.2 中：

- `EdgeHandle` 是不透明 typed handle，不暴露内部 index。
- `EdgeHandle` 可以使用 dense index，因为 lane graph topology 在初始化后稳定。
- `EdgeHandle` 不能跨 `CoreWorld` 混用，也不能持久化到 data format。
- `CoreWorld` 必须提供 resolver，从 `EdgeHandle` 查询 external edge ID，并从 external edge ID 查询 active `EdgeHandle`。

### D3. lane graph topology 在 v0.2 运行期不可变

状态：已接受。

v0.2 允许动态 spawn / despawn vehicle，并允许 register / remove route definition；但 lane graph edge 和 connection 拓扑在 `CoreWorld` 初始化后不可变。

原因：

- 动态拓扑会影响 route validity、vehicle occupancy、Adapter geometry、debug mesh 和增量 validation。
- `EdgeHandle` 采用无 generation 的稳定 dense handle，依赖拓扑不变。
- #29 的目标是固化 route 和 lane graph 的最小长期输入，不把动态 road network 编辑塞进同一切片。

如果后续需要运行时道路封闭、临时改道或动态地图编辑，应新建设计 Issue，并重新评估 `EdgeHandle` generation、route invalidation、车辆迁移和 Adapter 同步策略。

### D4. route traversal 只消费 edge length 与连接关系

状态：已接受。

Core route following 只需要：

- 当前 route edge。
- 当前 edge-local progress。
- 当前 edge length。
- route sequence 中的下一 edge。
- lane graph 是否允许当前 edge 连接到下一 edge。

Core 不负责把 edge progress 转换成世界坐标。Adapter 或 Presentation 需要的道路几何，应由 data layer / Adapter 通过 external edge ID 或 resolver 关联到自己的几何数据。

这意味着：

- Core state / query 输出可以包含 `EdgeHandle`、route edge index 和 progress；route transition event 默认只携带 handle 与 route edge index，不把 progress 固化为事件契约。
- Adapter 通过 resolver 得到 external edge ID，再在自己的几何数据中做插值。
- #30 data format 可以包含几何字段，但 Core lane graph 设计不强制 Rust runtime 消费几何。

### D5. topology validation 必须稳定且可诊断

状态：已接受。

CoreWorld 初始化或 lane graph 构建时必须执行以下校验：

- edge external ID 在 edge domain 内唯一。
- edge length 是 finite number，并且严格大于 `EDGE_BOUNDARY_EPSILON`。
- 每个 connection 的 `toEdgeExternalId` 必须引用已存在 edge。
- 同一个 source edge 内不得重复声明同一个 target edge connection。
- self connection 合法，但必须显式声明。
- terminal edge 合法。
- disconnected graph component 合法；route validation 负责保证单条 route 连通。

校验结果必须稳定可复现：

- 若实现采用 fail-fast，错误发现顺序必须基于输入顺序和 connection 顺序。
- 若实现采用批量收集错误，错误列表也必须按输入顺序稳定排序。
- 错误信息应包含 source edge external ID 和 target edge external ID，便于工具和数据作者定位。

### D6. connection 顺序不是 public routing contract

状态：已接受。

connection 输入顺序可以用于稳定校验和 debug 展示，但不得作为 route choice、priority、random choice 或 lane change 策略的 public contract。

后续若引入 pathfinding 或 lane change planner，必须显式设计：

- 候选 connection 的排序依据。
- cost / priority / restriction 字段。
- deterministic tie-breaker。
- 与 signals、intersection rules 和 vehicle following 的交互边界。

## 4. Runtime API 影响

v0.2 Core API 应从字符串 edge 引用迁移到 handle 引用：

- `LaneGraph` 初始化输入使用 external edge ID。
- `CoreWorld` 归一化后使用 `EdgeHandle`。
- route definition 保存 `Vec<EdgeHandle>` 或等价 compact representation。
- event payload 使用 `EdgeHandle`，调用方通过 resolver 转为 external edge ID。
- public API 不暴露 `IndexMap`、内部 index 或连接数组的可变引用。

v0.1 的 `LaneEdge::new(id, length, next_edge_ids)` 可以作为迁移起点，但 v0.2 不应把 `next_edge_ids: Vec<String>` 留在 runtime hot path。

## 5. Data Format 影响

#30 data format 应至少能表达：

- edge external ID。
- edge length。
- directed connection 列表。
- route 使用的 edge external ID sequence。

#30 可以决定具体序列化字段名，例如 `nextEdgeIds` 或 `connections`。无论序列化名称如何，语义必须等价于本文的 directed connection。

data format 不应持久化 `EdgeHandle`。`EdgeHandle` 只在单个 `CoreWorld` / simulation session 内有效。

## 6. Adapter 影响

Adapter 不应复制 Core topology validation，也不应自行决定 route 是否连通。

Adapter 可以：

- 读取 Core 输出的 edge handle / route edge index / progress。
- 通过 resolver 获取 external edge ID。
- 使用自己的 geometry 数据把 progress 映射到 transform。
- 在 debug draw 中显示 lane graph 和 route。

Adapter 不应：

- 把引擎对象 ID 当作 Core runtime handle。
- 依赖 `EdgeHandle` 的内部数值排序。
- 在 CoreWorld 运行期直接修改 lane graph topology。

## 7. ADR 评估

本设计不新增 ADR。

原因：

- fixed tick、determinism、runtime mutation 边界已由 ADR 0003 冻结。
- external ID / typed handle / registry / resolver 已由 ADR 0005 冻结。
- 本文是对 lane graph 的 v0.2 可执行设计展开，没有新增新的跨层不可逆架构取舍。

若后续引入动态 lane graph 拓扑、pathfinding、connection identity 或 geometry-driven Core collision / occupancy，应重新评估是否需要新增 ADR。

## 8. 测试与验证输入

后续实现 issue 至少应覆盖：

- duplicate edge ID。
- invalid edge length：`NaN`、`Infinity`、`-Infinity`、`0`、负数、`EDGE_BOUNDARY_EPSILON` 和小于 epsilon 的正数。
- unknown connection target。
- duplicate connection target。
- terminal edge。
- explicit self connection。
- disconnected graph component 与合法 route / 非法 route 的区别。
- resolver 能在 handle 与 external edge ID 之间稳定转换。
- event / traversal 不依赖 connection 输入顺序做隐式 route choice。
