# Lane Graph 设计

**文档状态**: Accepted  
**最后更新**: 2026-11-09

**适用范围**: Lane Graph + Route 的 Traffic Runtime 模型、edge / connection / topology 与 per-edge 基础道路限速

**关联文档**:

- `core-id-handles.md`
- `road-junction-model.md`
- `route-system.md`
- `shared-static-network.md`
- `traffic-runtime-integer-geometry.md`
- `traffic-runtime-revision-cutover.md`
- `../adr/0003-runtime-tick-and-determinism.md`
- `../adr/0005-core-identity-and-handle-model.md`
- `../adr/0017-static-road-junction-maneuver-and-gate-identity.md`
- `../adr/0025-checked-canonical-network-and-shared-static-network.md`
- `../adr/0028-integer-millimeter-traffic-geometry.md`
- `../roadmap.md`

## 1. 目标

本文定义 Traffic Runtime 消费的车道图模型：edge、connection、拓扑校验与 traversal 输入。

非目标：

- 不冻结完整道路几何、mesh、样条曲线或坐标系统。
- 不设计 pathfinding、route planner、lane change planner 或随机路线选择。
- 不设计 signals、parking、vehicle following、intersection priority 或 conflict zone。
- 不支持在共享路网修订内新增、删除或修改车道图拓扑。

## 2. 术语

- **Lane graph**：Traffic Runtime 用于验证 route 和进行 route traversal 的有向图。
- **Lane edge**：Traffic Runtime route following 的最小有向行驶段。
- **Lane connection**：从一个 lane edge 到另一个 lane edge 的显式可通行关系。
- **边键**（`laneEdgeKey`）：编制来源中显式、稳定的边标识，编译器据此派生 `StableId128`。
- **`LaneEdgeOrdinal`**：共享路网修订内的有类型边序号，Runtime 公开 API 与热路径使用它。
- **SpeedLimit**：lane edge 不可变的基础道路限速；已提交值为 `u32` mm/s。
- **最小 edge length**：边长输入的下限，属于输入语义；已提交值见 D5。

## 3. 设计决策

### D1. lane graph 是有向 edge graph

状态：已接受。

lane graph 使用 directed edge graph。每个 lane edge 表示一个方向上的可行驶段；双向道路必须建模为两条或多条方向相反的 edge。

概念模型：

```text
LaneGraph
  edges: LaneEdge[]

LaneEdge
  ordinal: LaneEdgeOrdinal
  stableId: StableId128
  length_mm: u32
  speed_limit_mm_s: u32
  connections: LaneEdgeOrdinal[]
```

说明：

- `length_mm` 是 Traffic Runtime route following 的正本长度。
- `speed_limit_mm_s` 是道路静态事实；当前的 effective speed ceiling 与该值相等，未来驾驶风格或临时 maneuver allowance 不得修改它。
- `connections` 只表达可从当前 edge 进入哪些下游 edge。
- 没有 outgoing connection 的 edge 是合法 terminal edge。
- route 的实际行驶顺序由 route edge sequence 决定，不由 connection 顺序隐式选择。
- 两个物理上不同但连接同一对 edge 的通行方式，应拆成独立中间 edge；不引入 connection ID 或多重 edge-to-edge connection。

### D2. 稳定标识与边序号分层

状态：已接受，依据 ADR 0005 与 ADR 0020。

编制来源、诊断和跨修订翻译使用稳定标识；编译器与共享静态路网分配 `LaneEdgeOrdinal`。

- `LaneEdgeOrdinal` 是有类型的密集序号，不携带 generation，因为拓扑在修订内不可变。
- `LaneEdgeOrdinal` 只在所属共享路网修订内有效，不写入编制来源。
- 稳定标识与序号经修订内的冷双向索引互查（`core-id-handles.md` §2）。

### D3. lane graph topology 在修订内不可变

状态：已接受。

Runtime 允许在步进之间增删车辆和路线定义，但 lane graph edge 和 connection 拓扑在共享路网修订内不可变。

原因：

- 动态拓扑会影响 route validity、vehicle occupancy、Adapter geometry、debug mesh 和增量 validation。
- `LaneEdgeOrdinal` 采用无 generation 的稳定密集序号，依赖拓扑不变。

拓扑变化只通过整份新修订的切换事务进入运行世界，见 `traffic-runtime-revision-cutover.md`。

### D4. route traversal 只消费 edge length 与连接关系

状态：已接受。

Traffic Runtime route following 只需要：

- 当前 route edge。
- 当前 edge-local progress。
- 当前 edge length。
- 当前 edge 的基础道路限速，以及 route 预计算的下游降限速 occurrence metadata。
- route sequence 中的下一 edge。
- lane graph 是否允许当前 edge 连接到下一 edge。

Traffic Runtime 不负责把 edge progress 转换成世界坐标。道路几何由 Spatial 按同一修订的边序号提供。

### D5. topology validation 必须稳定且可诊断

状态：已接受。

编译器必须执行以下校验：

- 边键在 edge domain 内唯一。
- edge length：编制米制先量化再检查；已提交值为 `u32` mm，`100..=10_000_000`（ADR 0028）。
- edge speed limit：编制米制先量化再检查；已提交值为 `u32` mm/s，
  `1..=100_000`（ADR 0028）。
- 每个 connection 的目标必须引用已存在 edge。
- 同一个 source edge 内不得重复声明同一个 target edge connection。
- self connection 合法，但必须显式声明。
- terminal edge 合法。
- disconnected graph component 合法；route validation 负责保证单条 route 连通。

校验结果必须稳定可复现，诊断应携带 source edge 与 target edge 的稳定键，便于工具和数据作者定位。

### D6. connection 顺序不是 public routing contract

状态：已接受。

connection 顺序可以用于稳定校验和 debug 展示，但不得作为 route choice、priority、random choice 或 lane change 策略的公开合同。

后续若引入 pathfinding 或 lane change planner，必须显式设计：

- 候选 connection 的排序依据。
- cost / priority / restriction 字段。
- deterministic tie-breaker。
- 与 signals、intersection rules 和 vehicle following 的交互边界。

## 4. Runtime API 影响

- 编制来源使用边键，共享静态路网与 Runtime 使用 `LaneEdgeOrdinal`。
- route definition 保存 `LaneEdgeOrdinal` 序列或等价紧凑表示，以及预计算 metadata，不在热路径读取稳定标识。
- 公开 API 不暴露内部表布局或连接数组的可变引用。
- 边长、限速和后继关系从 `SharedTrafficNetwork` 只读。

## 5. 持久化影响

路网制品（LFCA）保存边长、`mm/s` 限速与有向连接，字节形状见 `portable-canonical-artifact.md`。terminal edge 的连接列表为空。

`RouteHandle`、`VehicleHandle` 等每世界句柄不进入路网制品。

## 6. Adapter 影响

Adapter 不应复制 Traffic Runtime topology validation，也不应自行决定 route 是否连通。

Adapter 可以：

- 读取 Traffic Runtime 输出的边序号 / route edge index / progress。
- 通过共享路网修订的冷标识索引获取稳定标识。
- 使用同一修订的 Spatial 数据把 progress 映射到 transform。
- 在 debug draw 中显示 lane graph 和 route。

Adapter 不应：

- 把引擎对象 ID 当作 Traffic Runtime 句柄或序号。
- 依赖边序号的数值顺序表达业务含义。
- 在运行期直接修改 lane graph topology。

## 7. 测试与验证输入

实现至少应覆盖：

- duplicate edge key。
- invalid edge length：`NaN`、`Infinity`、`-Infinity`、`0`、负数，以及量化后的上下界与相邻值。
- invalid speed limit：`NaN`、`Infinity`、`-Infinity`、`0`、负数，以及量化后的上下界。
- unknown connection target。
- duplicate connection target。
- terminal edge。
- explicit self connection。
- disconnected graph component 与合法 route / 非法 route 的区别。
- 稳定标识与边序号之间可稳定互查。
- event / traversal 不依赖 connection 输入顺序做隐式 route choice。

## 8. Junction/Maneuver 层

ADR 0017 不改变 LaneGraph 对 edge、length、speed limit 和 directed connection 的裁决。共享静态路网在 LaneGraph 之上提供不可变的 `Junction -> Movement -> ManeuverPath` 层级：

- ManeuverPath 以 `entry + ordered internal[] + exit` 引用 LaneEdges；
- LaneEdge 不被 Junction/Movement/ManeuverPath 重新拥有；
- internal edge 的 Junction owner 由 path membership 派生，同一 Junction 内可共享，
  跨 Junction 不得复用；
- entry/exit boundary edge 可以连接相邻 Junction；
- Traffic Runtime 不从 geometry、connector 名称或 edge pair 推导 Junction/Movement identity。

完整 owner、validation、Route occurrence 与性能契约见
[`road-junction-model.md`](road-junction-model.md)。
