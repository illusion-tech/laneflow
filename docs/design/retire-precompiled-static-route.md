# 退役预编译静态路线

**文档状态**: Accepted（#498 G1）<br>
**最后更新**: 2026-08-26<br>
**适用范围**: 从路网产品删除 `StaticRoute` 后的制品表、身份空位、运行时编译、
场景 catalog 0.3 与每世界路线表形状<br>
**关联文档**: [`../adr/0029-retire-precompiled-static-route.md`](../adr/0029-retire-precompiled-static-route.md)、
[`traffic-runtime-shared-consumption.md`](traffic-runtime-shared-consumption.md)、
[`portable-canonical-artifact.md`](portable-canonical-artifact.md)、
[`shared-static-network.md`](shared-static-network.md)、
[`route-system.md`](route-system.md)、
[`compiler-foundation.md`](compiler-foundation.md)、
[`example-scenarios.md`](example-scenarios.md)

本文是 ADR 0029 的实现级合同。为什么删除见 ADR。G2 决定 Rust 拼写。

## 1. 单一路线入口

`TrafficWorld` 安装后没有路线。调用方提交边序号序列：

```text
register_route([e0, e1, …, eN]) → RouteHandle
remove_route(handle)
spawn_vehicle(profile, handle, route_edge_index, progress_mm, speed_mm_s)
```

`RouteHandle` 只有槽位下标与 generation。没有静态种类、没有共享根序号别名。
`remove_route` 不再区分「静态句柄」；stale / in-use / 容量仍然失败关闭。

注册期编译器（`compile_route`）是**唯一**出现项编译器。
输入是共享根交通视图 + 边序号切片；输出只进入本世界表。共享根不保存路线。

匹配规则与现行动态路径相同，且必须覆盖现行静态 seal 已接受的语义：

| 检查                                      | 失败       |
| ----------------------------------------- | ---------- |
| 空序列                                    | 空序列     |
| 边序号越界                                | 未知边     |
| 相邻边不连通                              | 不连通     |
| 首/末边是路口内部边，或末边带停止线       | 机动不匹配 |
| 入口候选对不上完整 `ManeuverPath.edges()` | 机动不匹配 |
| 两条不同 path 都完整匹配                  | 机动歧义   |
| 路口内部边无出现项覆盖                    | 机动不匹配 |
| 出现项 hop 区间相交                       | 机动不匹配 |

hop 是否受控：用已编译机动出现项定位 path，再在共享根
`transition_candidates(from)` 上找 `successor == to && path && transition_index`
对应的门。无门则该 hop 不受信号限制。禁止再读已删除的
`StaticRoute.transitionGates`。

`register_route` 必须在本世界 compiled 表物化下列 **tick 索引**（不进共享根、不进
磁盘快照；切修订原地重编译时一并重生；形状与现行
`RouteDistanceIndexView` 相同）：

- 分段 `u32` 前缀（段下标、段内偏移、段合计）：下一条边长会让当前段溢出时封段、
  开新段。Finite 侧不上 `u64`。城市一趟行程（Spatial 单 frame 约 32 km，通勤/
  过境几十公里）落在约 4295 km 的 `u32` 窗口内；为「理论最长边序列 × 10 km」加宽
  会改查询合同（ADR 0028）。占用 `i64` 只服务有符号空隙，不是前缀先例。
- 每条边的后缀 `BoundedDistance`（到路终）。`remaining_to_route_end` 必须 O(1)
  读该列（扣当前边内进度），不得扫描剩余边，也不得用饱和起点前缀相减。从起点
  跨段总和仍可 `BeyondFinite`；靠近终点后缀可再 `Finite`，`Completed` 只在
  `Finite(0)`。
- 每个 hop 的下一受控门（绑定 `SignalGroup` 的门）及其沿路线的有界距离。这是
  **拓扑链**（下一盏有信号的门），不是「当前红灯」。信号停车沿该链走到第一盏
  **当前**限制的门；不得扫全部剩余 hop，也不得在槽位里存相位相关的红灯列。
- hop → 门：按 hop 下标定位，不得对机动出现项线性扫描。
- 限速下降转换：与现行共享根 `speed_limit_transitions` 同形（`from` / 目标边 /
  目标限速）。tick 读该列做下游制动包络，不得扫剩余边。边限速**值**仍读共享根
  边热列。

等待区出现项按所属机动出现项 + 入口门在注册时物化到本世界表。等待运行时行为仍
归独立切片；本切片不得把「静态有、动态无」的出现项缺口留在生产路径。

共享根不再为路线预计算 `distance_to_end`、`next_controlled_transition` 或
`speed_limit_transitions`。这些索引改由本世界 compiled 在 `register_route` 时物化。

编制 `StaticRoute.canvas_selection` 随 table 删除，不迁入 Runtime。走廊生成器停止
`add_static_route`，把有序边键写进 catalog 0.3。LFSM 历史实体代码 34 与 LFSD
关系角色 13–16 禁止出现。

## 2. LFCA / 关系 / 身份删除清单

### 2.1 实体表

`CanonicalEntityTables` 必选表从 22 张改为 21 张。下列 **必须存在**：
`0x0001..=0x0014` 与 `0x0016`（CanonicalFrame）。

| tableKind | 名称           | 本切片后              |
| --------- | -------------- | --------------------- |
| `0x0015`  | StaticRoute    | **禁止出现**          |
| `0x0016`  | CanonicalFrame | 保留，代码与 tag 不变 |

出现 `0x0015`、缺 `0x0016`、或 `formatVersion != 3`，读器失败关闭。format 3 对象
精确 TableV1 总数为 `30`（`1 + 1 + 21 + 1 + 3 + 1 + 1 + 1`）。历史 format 1/2 为
`35`。现行生产 `table_count()` 为 `30`。

StaticRoute 行上的 `3:edges`、`4:transitionGates` 一并消失。

### 2.2 出现项与关系角色

从 LFCA 关系 / 派生执行索引删除，角色代码改保留空位：

| 角色 | 历史名称                           | 本切片后           |
| ---: | ---------------------------------- | ------------------ |
|   13 | `StaticRouteEdge`                  | 保留空位，禁止出现 |
|   14 | `StaticRouteManeuverOccurrence`    | 保留空位，禁止出现 |
|   15 | `StaticRouteGateOccurrence`        | 保留空位，禁止出现 |
|   16 | `StaticRouteWaitingZoneOccurrence` | 保留空位，禁止出现 |

派生索引（路线距离前缀、下一受控转换、限速下降转换、边/路径/门/等待区反向路线
出现项）随实体删除。LaneEdge / ManeuverPath / ManeuverGate / WaitingZone 自身表
保留；它们不再持有「属于哪条静态路线」的反向向量。

其余关系角色代码不重编号。

### 2.3 Identity 登记表修订 2

`identityRegistryRevision = 2`。

| 代码 | 历史                | 修订 2                         |
| ---: | ------------------- | ------------------------------ |
|   21 | `StaticRoute`       | 保留空位；`from_code(21)` 失败 |
|   22 | `CanonicalFrame`    | 不变                           |
|   30 | `RouteKey`          | 保留空位；不得解码为字段       |
|   31 | `CanonicalFrameKey` | 不变                           |

`EntityKind` 可构造集合为种类 1–20 与 22，共 21 项。`from_code(21)` 失败，不发射、
不解码。`EntityKind::ALL` **长度仍为 22**：代码 21 占保留空槽，`CanonicalFrame`
仍是代码 22。共享身份表按 `kind_index = code() - 1` 寻址，backing 必须是 22 格，
不得把 `ALL` 缩成 21 项后再用代码减一索引。字段标签可构造集合去掉 30，保留既有
空位 23。身份编码版本仍为 1。

`CanonicalIdentityTable` 不得再出现 `entityKind = 21` 行。

### 2.4 编制来源与 IR

道路编辑 FlatBuffers：`format_version = 2`；删除 `StaticRoute` table 与
`RoadEditingSource.static_routes`；顶层声明向量 21 个（可构造 Identity 种类）。
schema 为 `schemas/road-editing/v2/road-editing.fbs`（G2 创建）。
`format_version = 1` / 含 `static_routes` 的旧 buffer 失败关闭。
`format_version = 2` 下根表 vtable 出现 member 25（历史 `static_routes`）同样失败关闭，
不得当未知槽忽略。
`frontendVersion = 2`。file identifier 仍 `LFRE`。

合成 DSL / typed AST / HIR / MIR / LIR：不再有静态路线声明或出现项表。
首批支持矩阵「静态路线」行改为明确拒绝。

生产 `CompileLimits` 不再包含 `RouteOccurrenceCount`。该限额不改挂到运行时。
G2 删除 `add_static_route` 之前，该入口只走通用 relation / reference / source-byte
限额，不为该死路径恢复 1920。

## 3. 共享静态路网

`SharedTrafficNetwork` 不再提供：

- 静态路线计数与 `StaticRouteOrdinal` 域
- `static_route_edges` / `static_route_transition_gates` / `static_route_reverse`
- `route_maneuver_*` / `route_gate_*` / `route_waiting_*` / `route_distance_to_end`

仍提供机动路径、转移候选、门、等待区、停止线、边长、后继 CSR、准入平面。
`register_route` 只读这些。

seal 不再为路线做 owner-local 分区或出现项闭合。空路线不是合法可选；根本不存在
该实体。

## 4. 场景 catalog 0.3

`catalog_version = "0.3"`。拒绝 `0.2`。

`RouteCatalogEntry` 增加有序边键（拼写由 G2 定，语义如下）：

```text
route_id: string
exit_portal_id: string
edge_ids: [laneEdgeKey, ...]   # 非空；允许同一边多次出现
```

- `edge_ids` 使用走廊编制 Identity v1 的 `laneEdgeKey`，与 LFCA 边身份同一套键。
- bind 经 `SharedIdentityIndex` 把每个键解析为 `LaneEdgeOrdinal`；未知键失败。
- 对 catalog 中每条路线恰好 `register_route` 一次；失败则整个 bind 失败，不留半份句柄。
- 本世界 `route_capacity >= 28`。
- 人口 / 回流继续用 `route_id` 交叉引用，运行时只持 `RouteHandle`。
- 生成器写 catalog 边键，不写 LFCA 路线，不在编制来源声明路线。

封闭规模不变：6 个 portal、32 条 ManeuverPath、28 条路线。路线仍是有限显式边
序列；受保护转向覆盖由注册期编译器证明，不靠共享根预编译出现项。

`runtime_min` / 集成夹具同样改为显式边序号 + `register_route`。`lfca-full-spatial`
不再含静态路线行。

## 5. 每世界热表、磁盘快照与在线切换

三者不是同一份布局。

**内存热表**（G2 tick 只读这一套 + 共享根机动/信号/边长）：

```text
slot.generation: u32
slot.compiled:
  edges: [LaneEdgeOrdinal]
  maneuvers: [{path, entry_route_edge_index, exit_route_edge_index}]
  hop_gate[hop]                      # 受控门或缺失；O(1)
  distance segments/offsets/totals   # 分段 u32 前缀；不上 u64
  remaining_to_end[i]                # BoundedDistance 后缀；O(1) 路终剩余
  next_controlled[hop]               # 下一有信号的门及有界距离；tick 沿链找当前限制
  speed_limit_drop[k]                # from / 目标边 / 目标限速；同形于 speed_limit_transitions
  waiting: 注册时必须能编译；#282 未消费前仍不得静默丢弃
slot.live_vehicles: u32
RouteHandle = { slot_index, generation }   # 只在产生它的 TrafficWorld 内有效；不编码 world
```

**磁盘快照**（ADR 0029 §6）：

```text
snapshot_route_id                # 仅该快照内指名
edges: [LaneEdge StableId128]    # 有序；允许重复边
vehicle:
  snapshot_route_id
  route_edge_index               # 序列下标
  progress_mm / carry_um / speed_mm_s / status
profile / class / parking: StableId128
```

不存槽位、`generation`、任何密集序号、compiled 出现项、catalog `route_id`。
读档经身份索引解析边身份，再 `register_route`，**新分配** 句柄。exact oracle 比对
边稳定身份序列与毫米游标，不比对 `RouteHandle`。

**同进程在线切修订**：允许原地改现有槽位的 compiled（新序号 + 重编译出现项），
当期 `RouteHandle` 保持到该进程结束。不得把该布局写进快照。走廊 catalog
controller 绑定的是 `(世界令牌, NetworkRevisionId)`：修订变化后 controller **失效**，
调用方按新修订重新 bind。重绑不得新分配句柄，也不得丢掉切修订已保住的句柄。本切片
不设计 catalog 原子热切换，也不让人口层在切修订后继续用旧修订句柄。

## 6. 必测项（G2）

- 含 `StaticRoute` 或 `formatVersion = 2` 的历史 LFCA 失败关闭，诊断可区分版本与未知表。
- format 3 LFCA 精确 TableV1 总数为 30（历史 format 1/2 为 35）；含禁止实体/关系表
  失败关闭。
- 身份 `entityKind = 21` 或字段标签 30 失败关闭。
- `EntityKind::ALL.len() == 22`，`kind_index(CanonicalFrame)` 可寻址；不得把 `ALL`
  缩成 21 项后再用 `code() - 1` 索引。
- 道路编辑 `format_version = 1` 或含 `static_routes`（含 format 2 根表 vtable
  member 25）的来源失败关闭；现行只接受 `2`。其它未知槽仍忽略。
- 三边 `entry → middle → exit` 夹具：`register_route` 后两车跟车，行为不弱于原
  `static_route(0)`。
- 走廊 28 条路线全部注册成功；受保护左转/直行/右转覆盖与现行静态夹具同等。
- 内部边缺口、停止线末端、歧义 path 在 `register_route` 失败，不留槽位。
- `remove_route` 在有车时失败；无车时旧句柄 stale。
- tick 源码路径不再按句柄种类分支（G2 可用测试或结构约束证明）。
- 前缀和溢出仍 `BeyondFinite`，注册成功。
- 快照夹具（G2 可不实现完整 #302）不得把 `RouteHandle` / 槽位 / 边序号写成耐久主键。
- tick 对路终剩余不扫描剩余边；compiled 索引在 `register_route` 后可查。
  索引是分段 `u32` + 后缀 `BoundedDistance`，不上 `u64`。
- 信号停车沿受控 hop 链走到第一盏当前限制的门，不扫全部剩余 hop，也不在 compiled
  里存当前红灯。
- 下游限速下降读本世界 `speed_limit_drop`，不扫剩余边；限速值仍读共享根边热列。
- 同一修订上两个 `TrafficWorld` 各自 `register_route`；catalog / scenario bind 把句柄
  钉在该 world 的 `install` 令牌上，不得用指针比较。`RouteHandle` 只有槽位与
  generation，不编码 world。spawn 只查本世界表。跨 world 把句柄塞进另一个 world
  是调用方错误，不作为运行时比特必测。
- 同进程切修订后走廊 catalog controller 不得继续 `consume_world` / `apply_pending`；
  必须按新修订重新 bind。重绑不得新分配句柄、不得丢掉已保住的句柄。不测 catalog
  原子热切换。
