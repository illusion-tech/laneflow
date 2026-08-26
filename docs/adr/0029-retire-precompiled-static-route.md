# 0029 退役预编译静态路线，路线入口只留 `register_route`

**状态**: Accepted（#498 G1）<br>
**日期**: 2026-08-26<br>
**适用范围**: 路网产品中的路线实体、LFCA / 共享静态路网 / 编制来源、`TrafficWorld`
路线入口、信号化走廊场景 catalog、以及 #302 快照字段可消费的每世界路线表形状<br>
**部分取代**: ADR 0017 中「目标态由编译器预编译初始路线出现项」；ADR 0020 /
`network-compiler.md` 把 `StaticRoute` 列为 Identity v1 必选声明种类；ADR 0025 /
共享静态路网把 `StaticRoute` 列为 Traffic 必需关系。`network-compiler.md` §9.7
对动态通行定义的耐久指名改为：快照局部 ID + 共享根实体 `StableId128`，不把运行时
槽位 / generation / 密集序号写入存档。路口 / 通行流向 / 机动路径 / 机动门 /
出现项匹配规则 / 热路径无字符串继续有效。<br>
**不取代**: ADR 0021 出行编排与路径规划分层；ADR 0018 `register_route` 不做
`(ParticipantClass, Route)` 判断；ADR 0028 整数毫米一维几何；#303 Routing 契约。<br>
**关联 Issue**: [#498](https://github.com/illusion-tech/laneflow/issues/498)<br>
**关联文档**:

- `0017-static-road-junction-maneuver-and-gate-identity.md`
- `0020-compiler-owned-static-network-and-static-image.md`
- `0021-city-simulation-game-traffic-foundation.md`
- `0025-checked-canonical-network-and-shared-static-network.md`
- `0028-integer-millimeter-traffic-geometry.md`
- `../design/retire-precompiled-static-route.md`
- `../design/traffic-runtime-shared-consumption.md`
- `../design/route-system.md`
- `../design/portable-canonical-artifact.md`
- `../design/shared-static-network.md`
- `../design/example-scenarios.md`

## 背景

LaneFlow 的第一长期消费者是上层类 Cities: Skylines 2 的城市模拟游戏。市民出行、
玩家新画的公交线、临时货运，都是运行时才决定走哪条边序列。`TrafficWorld::register_route`
已能在本世界编译出现项并生成车辆。

LFCA / 共享根里的 `StaticRoute` 把边序列和灯/门/待行出现项冻进不可变路网修订。
当前走廊、Bevy smoke 和 `runtime_min` 主要靠 `static_route(ordinal)`，因为还没有
#303 规划器，加载后要立刻放车。

这造成两套路线权威：共享根上的预编译目录，加上每世界动态表。tick 按句柄种类取边、
取门、做准入后缀。过境/BRT 等「已知线路」也可以在启动时 `register_route`，不必占
共享根。仓库未发布 1.0、没有外部用户。现在从封闭登记表拿掉 `StaticRoute`，比 1.0
之后再破坏 LFCA 更便宜。

代码洁净（单一路线编译路径、单一句柄种类）高于「多留一条可选已知线路」的收益。

## 决策

G1 冻产品合同、删除清单、版本闸口、场景 catalog 权威和每世界路线表形状。G2 决定
Rust 方法名、错误变体拼写和夹具字节。合入本文不授权改生产代码。

### 1. 路网产品不再拥有路线

静态路网（编制来源、compiler IR、LFCA、共享静态路网）不再声明、发射或安装路线。
路网拥有车道边、路口、机动路径、门、等待区、信号、停车、准入和车辆配置。
**通行计划属于每世界可变状态**，由调用方在 `register_route` 提交有序边序列。

城市游戏主入口是规划器或出行编排产出边序列，再 `register_route`。Traffic Runtime
不拥有出行选择策略，也不在不可变修订里保存行程目录。

### 2. `TrafficWorld` 只留动态注册

公开路线入口只剩：

- `register_route`：共享根 `LaneEdgeOrdinal` 有序非空序列 → 本世界编译出现项 →
  代际感知 `RouteHandle`
- `remove_route`：只移除本世界路线；仍有 live 车辆引用则失败

删除 `static_route`、`RouteHandle` 的静态分支，以及 tick 上按句柄种类取边/门/准入
的分叉。`RouteHandle` 不再区分静态/动态；只保留槽位下标与 generation。

出现项编译规则与现行动态路径相同，也必须覆盖现行静态 seal 已证明的语义，不得弱化：

- 相邻边连通（车道后继或机动转移候选）
- 机动路径完整匹配；只认 `transition_index == 0` 的入口候选；多条不同 path 都匹配则歧义
- 路口内部边必须被出现项覆盖
- 首边、末边不得落在路口内部边上；末边不得带 StopLine
- hop 半开区间不得相交
- hop 门从已编译机动出现项 + 共享根机动转移候选解析，不读共享根路线表
- 等待区出现项在注册时一并编译，供后继等待运行时消费；本切片不生产化 #282
- 本世界 compiled 表物化前缀毫米距离、下一受控 hop 与 hop→门索引；tick 对这些
  查询 O(1)，不把索引写进共享根或磁盘快照

`register_route` 仍不做 `(ParticipantClass, Route)` 判断；绑定期准入在 spawn。
前缀累计超出 `u32` mm 仍是查询侧 `BeyondFinite`，**不得**因此拒绝注册。

运行时不按边序列内容做内部去重。调用方若要复用同一序列，自己保留句柄。

### 3. 版本闸口与身份空位

破坏制品形状，升拒绝闸口；不重编号其余种类。

| 闸口                               | 现行 | 本切片后 |
| ---------------------------------- | ---: | -------: |
| 对象 `formatVersion`               |    2 |        3 |
| `canonicalFormatVersion`           |    2 |        3 |
| `identityEncodingVersion`          |    1 |        1 |
| `identityRegistryRevision`         |    1 |        2 |
| `networkRevisionDerivationVersion` |    1 |        1 |
| `constraintContractVersion`        |    2 |        2 |
| `staticExecutionContractVersion`   |    2 |        3 |

读器拒绝 `formatVersion != 3`。含 `StaticRoute` 表或种类代码 21 的对象失败关闭。
不恢复制品 v2 读器，不为旧字节提供迁移器。

Identity 种类代码 21（历史 `StaticRoute`）与字段标签 30（历史 `RouteKey`）改为
**保留空位**：不发射、不解码为合法种类/字段，不得分配给 `CanonicalFrame` 或其他
实体。`CanonicalFrame` 种类代码保持 22，字段标签 `CanonicalFrameKey` 保持 31。
关系角色 13–16（历史静态路线边/三类出现项）同样保留空位，不压缩后续角色代码。

LFSM `sourceMapFormatVersion` 与 LFSD `semanticDiffFormatVersion` 节形状不变，版本
保持 2；新对象不得再出现静态路线行。旧对象因 `formatVersion` 被拒，不单独兼容。

### 4. 场景 catalog 0.3 拥有示例边序列

仓库信号化走廊的 28 条路线是**场景层出行清单**，不是路网产品。

catalog 升到 `0.3`：每条 `route` 在既有 `route_id` / 出口 portal 之外，列出有序
`laneEdgeKey`。走廊生成器把序列写进 catalog，不再写入 LFCA / 编制来源。
bind 把键解析为共享根边序号，对每条 catalog 路线 `register_route` 一次，并在本
世界生命周期内保留句柄。人口与回流政策继续按 catalog 选择，只是句柄来自注册而非
静态序号。

城市游戏与 #303 不使用本 catalog；规划器直接提交边序列。不得为示例另发明第三类
路网 sidecar。不得在 bind 时从机动路径「反推」边序列——那会变成第二套路线编译器。

### 5. 容量

编译器 `max_route_occurrence_count = 1920` 只服务预编译静态路线，随实体退役。
该数字不是寻路上限，也不是单条动态路线边数上限。

每世界同时存活的路线条数继续由调用方 `WorldConfig.dynamic_route_capacity` 约束。
单条边序列只受空序列、连通、机动匹配和分配失败约束，不另冻产品边数。走廊示例必须
把容量设为至少 28。

### 6. 磁盘快照与在线切换的路线表示

`RouteHandle`、槽位下标和 `generation` 是进程内能力句柄，不是存档主键。磁盘快照
与同进程在线切换不是同一种表示。#302 实现容器和切修订状态机，必须消费本合同。

**磁盘快照**（存档 / 读档 / 检查点）路线表：

- 每条路线一个 **快照局部 ID**，只在该快照内指名车辆引用；恢复后不得复用为
  `RouteHandle`。
- 边序列用每条 `LaneEdge` 的 `StableId128`，不用 `LaneEdgeOrdinal`。同修订下允许
  在已证明 origin digest 一致时用序号作压缩编码，那是编码，不是第二套身份。
- 车辆存快照局部路线 ID、`route_edge_index`（该序列下标，不是路网序号）、毫米
  进度 / 余数 / 速度 / 状态。
- **不存** 槽位、`generation`、`LaneEdgeOrdinal`、compiled 机动/门/等待出现项、
  catalog `route_id`、已删除的静态路线序号。
- `VehicleProfile`、`ParticipantClass`、`ParkingSpace` 同样：存档用
  `StableId128`，内存用序号。

读档：按 committed 来源重建共享根 → `SharedIdentityIndex` 把边身份解析为 **当前**
序号 → 对每条快照路线 `register_route`（或同一编译器）→ **新分配** 句柄 → 车辆
引用打到新句柄。身份缺失或编译失败则整事务失败关闭。exact oracle 比对已提交交通
事实（边稳定身份序列、游标、毫米状态），不比对 `RouteHandle` 数值。

**同进程在线修订切换**：Adapter 可能仍持有当期句柄。允许在现有槽位 **原地** 把
compiled 边序号换成映射后的新序号并重编译出现项，句柄保持到该进程结束。这不是
磁盘格式，不得把槽位布局写进存档。

出现项是 `(边序列 × 当前共享根机动网)` 的纯函数，只存在于内存热表。加载和切修订
都用唯一 `register_route` 编译器生成；不得在快照里保留第二份出现项权威。

### 7. 落地顺序

唯一 Delivery PR 实现合同、重生夹具并 `Closes #498`。建议在编译器 IR 交通一维
已与制品同一套整数权威之后再改发射路径，避免连续两次重生 LFCA；这是实施顺序，
不是第二套合同。G2 不得把米制 IR 或并行 v2 读器带回来。

## 明确不做

- 不实现 #302 快照容器、在线切换或存档编码。
- 不实现 #303 规划器、动态成本或出行编排。
- 不把出行选择策略放进 Runtime。
- 不把 `RouteHandle` / 槽位 / `generation` / 密集序号写成存档身份。
- 不把 compiled 出现项写入快照。
- 不在 Runtime 内按边序列内容建立规范身份或去重表。
- 不恢复 JSON 运行时入口或 `CoreWorld`。
- 不把编制曲线、规范折线或车辆配置改出当前权威。
- 不为过境/BRT 保留空的 `StaticRoute` 表。
- 不重编号 `CanonicalFrame` 或压缩身份/关系代码。

## 后果

- LFCA 必选实体表从 22 张变为 21 张（缺 `0x0015`，保留 `0x0016` CanonicalFrame）。
- 道路编辑来源 `format_version = 2`：删除 `StaticRoute` table 与根上的
  `static_routes`；声明向量与 Identity 可构造种类一一对应（21 个）。
  `format_version = 1` 的旧 `LFRE` 在语义读取前失败关闭。schema 路径
  `schemas/road-editing/v2/`（G2 落地；本 PR 不改生成绑定）。
  `frontendVersion` 同步为 `2`。file identifier 仍为 `LFRE`。
- 合成 DSL 不再接受静态路线声明。
- 编译器生产 `CompileLimits` 删除路线出现项维度；若具名配置档不能原地删维度，G2
  使用新配置档标识。
- 共享 Traffic 不再投影静态路线边序列、出现项、反向索引或 seal 派生的路线距离/
  下一受控转换表。机动路径、门、等待区、停止线仍在共享根，供注册期匹配。
- 公开 API 删除 `StaticRouteOrdinal` 消费面（Runtime/Adapter/scenario bind）。
  身份 crate 可保留「代码 21 非法」的失败路径，不保留可构造的 `StaticRoute` 种类。

## 已考虑的方案

1. **保留实体、允许空表，只改示例走 `register_route`。** 叙事干净，但 tick /
   句柄 / 登记表 / 编译限额仍双路径。否决。
2. **留静态路线给过境/发行走廊。** 功能可被启动时 `register_route` 替代；无外部
   用户时不抵双路径成本。否决。
3. **与整数毫米 Delivery 同一 PR 拆掉。** 一次重生 LFCA，但破坏已冻范围。否决。
4. **独立 G1/G2，从登记表删除 `StaticRoute`。** 采用。
5. **bind 时从机动路径反推走廊边序列，catalog 不列边。** 会把生成器编制逻辑移到
   运行时 bind，形成第二套路线 compiler。否决。
6. **另写非 LFCA 路线 sidecar。** 多一类产品制品。否决。
7. **压缩 Identity 种类，把 CanonicalFrame 改成 21。** 身份编码是 Identity v1
   契约，禁止为清单美观重编号。否决。
8. **把 1920 改挂为单条动态路线边数上限。** 把编译出现项预算误用为行程长度，
   限制城市游戏超长出行。否决。
9. **磁盘快照按运行时槽位 + 边序号落盘。** 把分配器布局和修订局部密集域提升为
   存档主键；跨修订必漂，句柄语义绑死 freelist。否决。
10. **快照同时保存 compiled 出现项。** 与边序列形成第二权威；跨修订几乎总要丢弃。
    否决。
11. **磁盘与在线切换共用一种表示。** 在线需要保留进程内句柄，磁盘应当作废旧句柄。
    否决合成。

## 落地顺序与验收

G2 唯一 Delivery 必须证明：

- 生产路径无 `StaticRoute` / `static_route`；旧 LFCA 含该实体或 `formatVersion != 3`
  则失败关闭。
- 走廊、Bevy smoke、`runtime_min`、替换/回流测试只通过 `register_route` 放车，
  受保护转向覆盖不弱于现行静态夹具。
- tick 热路径只有一套已编译边序列与出现项。
- catalog `0.3` 带边键；生成器不再往 LFCA 写路线。
- 文档、glossary、compiler-foundation、portable-canonical-artifact、道路编辑
  schema 不再把预编译静态路线写成路网必选产品。
- 磁盘快照合同：局部路线 ID + 边 `StableId128`；不落盘槽位 / generation / 密集序号 /
  出现项。
