+++
schema_version = 1
id = "design-traffic-runtime-snapshot"
kind = "design"
status = "accepted"
revised_date = "2026-11-27"
summary = "定义运行时快照、恢复、摘要与跨修订迁移。"
scope = "版本化 Runtime Snapshot 的设计原则、绑定集、保存/恢复语义、回放、确定性状态摘要与跨修订迁移入口"
applicability = "#applicability"
issues = ["https://github.com/illusion-tech/laneflow/issues/282", "https://github.com/illusion-tech/laneflow/issues/283", "https://github.com/illusion-tech/laneflow/issues/302", "https://github.com/illusion-tech/laneflow/issues/540", "https://github.com/illusion-tech/laneflow/issues/1509", "https://github.com/illusion-tech/laneflow/issues/1512", "https://github.com/illusion-tech/laneflow/issues/1526"]
related = ["adr-0020", "adr-0021", "adr-0028", "adr-0029", "design-parking-system", "design-retire-precompiled-static-route", "design-shared-static-network", "design-traffic-observation-and-routing-integration", "design-traffic-runtime-revision-cutover"]
+++

# 运行时快照

<a id="applicability"></a>
## 适用边界与依据

快照绑定覆盖停车、路线冲突出现记录与等待区；精确状态和版本以正文合同为准。

关联文档：
[`../adr/0020-compiler-owned-static-network-and-static-image.md`][adr-0020]（§12；static image / receipt 条款已被 ADR 0025 §8 取代，origin 以 LFCA 为准）、
[`../adr/0021-traffic-infrastructure-and-host-boundary.md`][adr-0021]、
[`../adr/0028-integer-millimeter-traffic-geometry.md`][adr-0028]、
[`../adr/0029-retire-precompiled-static-route.md`][adr-0029]、
[`traffic-runtime-revision-cutover.md`][revision-cutover]、
[`traffic-observation-and-routing-integration.md`][routing-integration]、
[`retire-precompiled-static-route.md`][retire-precompiled-static-route]、
[`shared-static-network.md`][shared-static-network]、
[`parking-system.md`][parking-system]

本文是 ADR 0020 §12（按 ADR 0025 §8 修订后语义）的 #302 G1 设计。路线与车辆的
快照表示沿用 ADR 0029 §6 已冻结形状；容器 schema、字段映射与测试构造由 G2
落定。

本文的 #303 观测/Routing 恢复与回放接缝已由 #303 G1 接受，与 #302 合同共同构成
当前唯一实现正本。

> 实现状态：当前唯一生产合同与实现均为 Runtime Snapshot v7；来源绑定封闭为发布 / 编辑两种
> 形状（编辑来源只绑定道路编辑状态摘要、总长度与模块数）；停车使用 tagged
> `ExplicitSpace | VirtualPool` binding，并保存 Reserved/Occupied 状态、所有 Reserved
> binding 的精确 entry route occurrence 和 virtual reservation 的 semantic entry anchor；
> Waiting 保存 traversal、semantic membership 与非零历史 admission counter；
> Conflict 保存 first eligibility、Clearing Gate marker、reservation/passages/downstream authority 与
> 非 `NoHistory` lag reference，occupancy 从整车位置和 passage 锚点重建；
> `WorldConfig` 保存独立的路线边出现记录与路线冲突出现记录容量。旧 schema、reader 与
> writer 不属于当前生产入口，不提供双读或自动迁移。

## 1. 问题与设计立场

#284 的策略绑定、Conflict reservation 与历史状态增量见
[`traffic-runtime-right-of-way-policy.md`][right-of-way] §6、§8
（Accepted）。当前已实现策略绑定、Conflict eligibility/reservation/Clearing、
downstream authority 与 lag history 的持久化、同/跨修订迁移和生产 fixed-step 接线。

宿主应用需要存档、恢复与回放。Runtime Snapshot 是每世界可变状态的独立版本化
制品：不进入 LFCP 发布链，真实性由宿主存档清单在对象外绑定（ADR 0021）。
设计立场：

- **保存逻辑状态，不保存机器**：持久的是稳定引用、快照局部标识与各自合同规定的运动量
  （边上纵向位置见 ADR 0028，横向交接见 §2.1）；进程句柄、槽位、generation、密集序号、数组布局不持久，恢复
  时新分配句柄。
- **派生即不存**：凡确定性派生的状态不入快照——信号灯色由 `time_ms` 与共享静态路网
  program 派生，车道占用索引每 tick 从已提交状态重建。
- **单一时点**：保存在固定步进安全边界原子捕获一个快照点，编码只读该不可变
  捕获，世界随后可恢复步进；不存在跨提交状态的混合捕获。

## 2. 版本轴与绑定集

[执行配置设计][execution-config]将 worker 移出快照。当前版本轴为
LFRS 7 / runtime state 5 / digest 8；执行配置分离与编辑来源绑定仅改变容器布局，#1035
仅更换摘要算法与摘要版本，交通状态及摘要原像布局不变。
安装与 fresh restore 都显式接收 `ExecutionConfig`，完整交通准备后才校验执行能力。

版本轴分离：容器 `formatVersion` 与被绑定事实的版本（runtime 版本、
static-contract versions、`networkRevisionDerivationVersion`、identity registry
revision）不混用单一数字；未知版本值失败关闭。

必绑：LFCA origin；`CommittedNetworkSource`（editable 世界 = committed
`RoadEditingState` 的道路编辑状态摘要、精确总长度与模块数，不内嵌 `LFRE` 字节；
runtime-only 世界 = `PublishedLfcaReference`；编辑来源绑定形状见
[`committed-road-editing-state.md`][committed-editing] §6）；上列版本；
宿主安装时指定的世界身份 `world_id`；`tick` / `time_ms` / 输入命令与已提交事件双游标（切换
事务的 `worldBinding` 需要双游标基线）；全部每世界可变状态（§3）；
`WorldConfig` 与 `WorldPolicySelection`。必填 `world_policy` 使用封闭 tag：
`NotRequired = 1` 且无 policy，或 `Pinned = 2` 且带 policy StableId；0、未知 tag、
缺失/多余身份及未知 table 字段都拒绝。恢复把选择传入与安装共用的交通准备入口，
拒绝修订内未知策略与不合法 NotRequired；规则内容及法规来源由绑定的 LFCA 确定，
不保存 dense ordinal、解析表或步长派生间隙。

`WorldConfig` 只保存行为语义配置：`fixed_delta_time_ms` 必须精确匹配，vehicle、
route、route edge occurrence 与 route conflict occurrence 四项目标容量分别不得小于
保存容量，即使当前 live state 放得下也不能缩容。允许放大，但**精确回放的对拍
前提是语义容量一致**；容量差异会改变后续生命周期命令的成功条件，也进入摘要。
`ExecutionConfig` 由宿主另行显式提供，不从旧快照读取，不按 CPU 数猜测；当前支持
worker 1–16，超出后端能力的请求在完整交通恢复后返回执行能力错误。合法的执行配置
差异不得改变精确交通结果（ADR 0021、0030；执行配置合同 §2）。

`route_edge_occurrence_capacity` 与 `route_conflict_occurrence_capacity` 都是行为语义
容量：前者统计全部存活动态路线边序列 occurrence，后者统计路线重编译所得全部
`ConflictPassageOccurrence`；两者都保留重复出现。恢复配置必须分别容纳快照路线表
重建出的 exact 总数，检查点后精确回放还要求与原配置一致。

世界世代用于使活动进程内的会话失效，`WorldGeneration` 不进入快照。原位恢复时，运行时对活动
世界的当前世代执行受检递增。从快照新建世界时，运行时沿用快照中的 `world_id`，世代从零开始。
同一宿主进程须先结束该身份的旧世界和旧会话。

禁绑（出现即拒绝）：runtime handle / slot / entity generation、`WorldGeneration`、
密集序号、共享静态数组、`EditableDiffBase`、partition / worker assignment、数组地址 / layout /
capacity、调用方自有 seed/随机流（宿主存档清单绑定；Runtime 无自有随机流）。

### 2.1 任意位置换道与路口跟入预约的联合升版（目标合同）

LFRS 8、runtime state 6、deterministic digest 9 是同一次交付的唯一组合。容器 8 保存
路线登记的横向交接窗口、车辆已提交的任意位置换道及其组合预约，以及普通路口每份
冲突预约的下游授权终点 `E_f`（联合设计 §6.11）。状态 6 和摘要 9 覆盖同一组事实。
发布时写入、读回、直接与捕获摘要、恢复、迁移日志和回放一起切换。联合读方拒绝旧组合
及混用版本，包括空路线、无车或没有预约的世界。实施前生产组合仍为 7/5/8，固定预制过渡的保存限制见 §3。

**登记窗口。** 摘要 9 在 §6 每条路线记录的有序边向量后追加 `u64` 窗口数，普通路线写零。
各窗口按源出现下标严格递增，每项依次写以下字段。整数用小端，各稳定引用沿用 §6 的
16 字节规范次序。登记日志携带完整窗口，移除日志指认已登记的同一路线实例。

| 字段顺序     | 表示与恢复约束                                                                                                                                                                                                 |
| ------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 源出现       | `source_index:u32`。目标为受检的 `source_index + 1`，两边的有向身份取自这两个出现。两处出现必须存在，同一源出现只能登记一次。                                                                                  |
| 结构边界     | `ownerKind:u16`，继以所有者、左成员、右成员各一个 `StableId128`。沿用横断面表 7 的种类值。1 为走廊及其相邻两区段，2 为区段及其相邻两编制车道。左右按走廊参考方向，须重验直接邻接、切点归属和两条有向边的资格。 |
| 允许交接窗口 | `low_nm:u64, high_nm:u64`，所属走廊局部站位闭区间，`low_nm ≤ high_nm`。读回须证明两边的有向站位域与窗口有非空共同范围。                                                                                        |

验收使用两条尚无车辆、边序列相同的路线，仅改变允许窗口或重复边的源出现下标。
两者的摘要原像须可区分，各自恢复后的直接摘要须与捕获摘要一致。

**已提交候选及组合预约。** 在车辆既有字段（含下文普通冲突预约）之后，摘要 9 写
`u64` 条目数，再按源出现下标严格递增写各项。每项以 `source_index:u32` 引用所属
路线中唯一的登记窗口；车辆、路线由 §6 的实例分组指认。写方保留仍在执行或仍持有保护的
交接，直到相应义务均已结束。P3/P4 尚未提交的试算、申请和当拍工作计数不入档。
每项的字段顺序如下。

1. 保存候选的轨迹配置版本（`u16`）、求高与投影窗口（`low_nm:u64, high_nm:u64`）、
   沿行驶方向的后轴起点与终点（各 `u64` 纳米）。投影窗口按数值递增；反向行驶的后轴
   起点大于终点，不能交换。路线的允许交接窗口约束前保险杠过缝，这里的投影窗口还须容纳整车求高。
   同一修订与结构边界确定框架和走廊，所属车辆的有效车型稳定引用确定轴距、车体及转向限值。
2. 依次保存求证转向角速率时的后轴速度、轮迹预算 `tracks:u32`、扫过预算 `sweep:u32`。
   现行试算允许以米每秒输入任意有限非负 binary64，联合版按求证时的规范 binary64 位型
   写 `u64`，零统一为正零。预算取与各自生产硬上限相交后的有效值。恢复重用该速度和两个逻辑上限。车辆现速另用 `speed_mm_s`
   保存，制动后两者可以不同。几何算法与确定性工作顺序按轨迹配置版本解释。
3. 保存从候选起点沿后轴三维轨迹累计的已提交行程 `rear_travel_um:u64`，再写前保险杠
   是否已交接（`u8`，0 为尚在源出现，1 为已进入目标出现）。行程含不足一毫米的余量，
   按下文从单一整数读回。当前边的纵向进度与余量仍随前保险杠游标独立保存。
4. 依次写预约获得刻度 `u64`、仍持有的物理区间数 `u64`、各区间。每个区间依次写
   角色 `u8`（1 源侧、2 目标侧、3 扫过、4 下游）、物理边 `StableId128`、
   `start_mm:u32, end_mm:u32`。采用沿边前向的非空半开区间，在边长内保守覆盖该角色须保护的范围。
   同角色同边取规范并集，按角色、边稳定字节、再按起止数值升序。获得刻度不得晚于存档刻度。
   同一辆车触及两侧时，两侧实际占用都指向该车。占用按车体重建，预约仅表示保护义务。

直接与捕获摘要共用上述字段和次序；局部车辆号、路线号、修订内 ordinal 和临时句柄不进入原像。
LFRS 的集合仍用 FlatBuffers 向量及其自身的长度，`u64` 集合计数属于摘要原像。读方拒绝未知
标签或配置、错型或悬空引用、速度负零或非法、超过硬上限的预算、同组重复出现、逆序或非规范区间、
超出对应格式/世界容量的计数和截断的条目，先验证计数、乘加与余下字节再分配。

**后轴累计与单拍预检。** 候选起点的 `rear_travel_um` 为零，两个行驶方向均按非负实际行程累加。
毫米部分为 `rear_travel_um / 1000`，余量为 `rear_travel_um % 1000` 微米。每拍取纵向裁决及停车约束
共同确定的实际行程。常规分支将瞬时 `f32` 米精确提升到 `f64`，乘 `1_000_000` 后就近取整，
正中时取偶数微米。跟车安全量化向下，
停车可行性保持采用该分支最终收紧的整数行程。单拍预检直接消费已分配给这段候选的整数微米增量，
只加一次。零增量保留累计余量。前保险杠跨边时，后轴累计仍从原候选起点量取。[整数几何 §5–5.1][integer-geometry]

对累计加法和乘 `1000` 转为查询纳米作受检运算。`rear_travel_um` 的编码及反查输入共同限定范围
为 `0..=18_446_744_073_709_551` 微米，即 `floor(u64::MAX / 1000)`。边长上限为
`10_000_000` 毫米，仅约束单条边。合法步长为 4 至 1000 毫秒，100,000 毫米/秒的上限乘最长
步长为 100,000,000 微米，用于验收最大一步加法。拍内制动、停稳或空间裁剪仍用实际行程。
边界用例覆盖加上最大一步恰到可编码上限、再多一微米以及单拍零增量。候选全长须另向几何查询求界。

正反向验收取水平直路上纵向前进 30 米、横向移动 3.5 米的同一五次曲线。站位从 20 米到 50 米或从 50 米到 20 米，
到 35 米时两向后轴三维行程均落在 15.14456 至 15.14458 米内。两向若从 15,144,999 微米再走
2 微米，结果同为 15,145 毫米加 1 微米。实际站位分别递增、递减，相邻两次反查的外包可以重叠。
反查显式给定正的纳米容差与逻辑工作上限（单次至多 4,096），成功结果为按数值递增的保守站位闭区间。
容差约束区间宽度，整段区间均须满足要提交的判定。
超出候选、可能跨越未唯一确定的候选末端、加法或换算越界、预算耗尽或几何未证时，本次预检不得发布推进
或释放保护。跨端点分配同拍剩余行程须另证两段的消费与交接次序，不得用弧长外包中点或截断到终点来分配。

**联合版的恢复首拍。** 命令侧可显式传入本拍行程、反查容差和工作上限验证一次预检。接入运行世界时，
须把影响选速、反查容差和后续裁决的有效参数、单位、顺序、字宽及版本纳入 §6 世界行为配置，
由版本唯一指认的固定常数可从该版本重建。启用动态换道的世界显式提供[横断面设计 §5][clearance-speed]所规定的净空限速曲线。
联合版的世界行为配置在固定步长之后、刻度之前写入以下摘要原像（整数小端）。`mode:u8` 为 0 表示关闭动态换道，无后续字段；
为 1 表示显式启用，依次写 `policy_version:u16=1`、`max_span_nm:u64`、`travel_tolerance_nm:u64`、
`point_count:u64`，再按原序逐项写 `clearance_um:u32, speed_mm_s:u32`。LFRS 8 按同样的标签、标量宽度及节点内容保存，
节点数由 FlatBuffers 向量给出。先验证模式、版本、正的跨度和容差、2 至 8 个节点及字节可用性，再验证净空和速度的顺序与端值。
关闭模式不得携带策略载荷或已登记交接窗口；启用模式即使世界为空，也必须完整保存实际配置。
出现缺项、未知模式或版本、非规范节点或目标世界行为不匹配时，恢复整体拒绝。
直接摘要与捕获摘要均写入上述值。不同配置在空世界也有不同的原像。跨修订接管继承配置，重新取得目标修订的几何证据。
限速计划属于派生值，在相同修订按保存的配置及候选预算重建；不写缓存、指针或宿主预设名。
联合版须覆盖实际支持的最长候选，并在非零余量、反向行驶、前保险杠恰好过缝、车尾恰好清空和工作预算触限时
对拍恢复首拍。完成这些合同及联版验收前，8/6/9 不能写入或作为已交付版本公布。

恢复在候选世界重建同一轨迹，从保存的行程复核车体位姿、前保险杠对应的路线游标、
两侧实际占用与车尾清空条件，再核对预约归属、获得刻度及所需物理区间，全部成立后一次发布。
以路线交接合同 §4.1 的 30 米横移为例，在前保险杠已通过约 15.19516 米的交接截面、车尾仍在源侧时保存。
同修订恢复后，游标应已进入目标出现，两侧仍计为同一辆车，首拍从原预约开始。
与连续运行对照，车尾清空与释放预约的拍、事件序列、游标和逐拍摘要均须一致。保留该阶段却删去源侧保护，
或将游标回退到源出现，须零发布。活动横移或任一相关预约尚存时，跨修订接管整次拒绝；
其余已登记窗口逐个证明与目标修订的物理截面对应。[路线交接合同 §4.1][lateral-handoff]

**冲突预约的下游终点**。每份非空冲突预约必带一个规范终点。以预约已有的
`maneuver_entry_route_edge_index` 为锚，按顺序编码
`relative_hop_offset:u32`、`progress_mm:u32`、`carry_um:u16`。
以受检加法还原路线边出现下标；同一边的第二次、第三次经过各有独立位置。
非末边接缝唯一编码为下一边的零进度，只有路线末端可用末边全长。
当前清空锚点、车长及叠放间距均为整毫米，本版本合法授权终点的 `carry_um`
精确等于零。非零输入拒绝；车辆位置和空间上界仍按完整微米坐标与终点比较。
writer 生成规范坐标，reader 拒绝越界、非规范接缝、相对下标溢出和缺失终点。

快照保留预约既有的车、路线、Gate、passages 与获得刻度。原下游物理区间向量
从该预约的 wire 与摘要原像移除。摘要 9 在原车辆记录的 reservation passages 段
后按上述字段顺序和宽度写入小端整数；捕获与直接摘要使用同一原像，保留现行域
分隔与算法。两份终点不同而投影相同的预约须有可区分的原像。

capture 自 Gate 正向重建至终点的规范并集，与该车在当前冲突账本中的区间逐项比对
后写出终点。restore 先解析路线并验证终点和逐车义务，在候选世界正向构建账本，
再检验同链共享、链外互斥与车身阶段。仅更换为另一份满足全部谓词、且投影相同的
合法终点会改变状态语义和摘要；识别与既有检查点的偏离依靠可信的检查点摘要。
字段非法、capture 的热账本不符或 restore 的组合状态非法时，分别不写出、零发布。
重复边验收使用 `A → B → A`。单车与跟入车在第二次经过 A 时获得不同终点，
即使正向重建的物理并集相同，恢复仍须保留各自的授权终点，包括前车已经清空或移除时。

**联合验收。** 分别保存仅有换道、仅有路口冲突预约的世界，再保存同一世界不同车辆
各持一类预约的检查点。对照连续运行，逐车核验获得刻度、交接阶段、实际保护区间、
`E_f` 的路线出现及正向重建的账本。直接摘要、捕获摘要与同修订恢复后的摘要相等；
从该点重放登记、移除与已提交步进，核对每拍的状态、事件及游标。分别破坏任一类
预约时，恢复整批失败。活动换道与可直移的路口预约并存时，跨修订接管整次拒绝。

## 3. 每世界可变状态

下表列出 LFRS 7 的生产字段。LFRS 8 的路线窗口、活动横移与两类预约见 §2.1。

| 状态        | 快照表示                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| ----------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 路线表      | ADR 0029 §6 形状：`snapshot_route_id` + 有序边 `StableId128` 序列（允许重复边）；机动、等待区与冲突 passage 出现记录均由目标共享路网修订重新编译，不入快照                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| 车辆        | ADR 0029 §6 形状 + 每车唯一 `snapshot_vehicle_id`：所属 `snapshot_route_id`、`route_edge_index`、`progress_mm` / `carry_um` / `speed_mm_s` / `status`；profile / class 等静态绑定用 `StableId128`                                                                                                                                                                                                                                                                                                                                                                                                                       |
| 停车状态    | 保存 `Reserved | Occupied` + tagged target；显式 target 保存 `ParkingSpace StableId128`，虚拟 target 保存 `ParkingFacility StableId128`；Reserved 保存 entry route occurrence，所属 route 即同一车辆的 `snapshot_route_id`，Reserved virtual 另存 `(entry LaneEdge StableId128, progress_mm)`；counts/capacity 不作为第二 authority 入档                                                                                                                                                                                                                                                                                |
| Waiting     | 每车保存可选 maneuver traversal 与 semantic membership；每个有逻辑历史的 zone 保存 `WaitingZone StableId128`、occupancy 与单调 `nextAdmissionSequence`。queue link、tick-local claim 与 latest output batch 不入档                                                                                                                                                                                                                                                                                                                                                                                                      |
| Conflict    | 每车保存可选 exact Gate occurrence eligibility（含 `firstEligibleTick`），或 `Clearing` Gate marker 与该车的 reservation wire row（acquired tick、passage stable locator/route occurrence 证明、committed downstream 物理区间并集）；restore 后 eligibility 必须同时满足 exact locator/位置与所选 policy 在已恢复时刻对 admission Gate 的 `Candidate` 决策，`DenyAndStop` 整体拒绝；完整 reservation 只安装进 `ConflictArbiter`，vehicle 不保留副本。每个非 `NoHistory` cell 保存 tagged `ActualClear | CutoverFloor` 与时间。occupant/cleared、profile 派生的跟车间隙、frontier、tick-local grant 和内部 serial 不入档 |
| live 顺序   | 车辆 `snapshot_vehicle_id` 的规范排序序列                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| tick / 时钟 | `tick` / `time_ms` / 输入命令游标 / 已提交事件游标                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |

车辆是运行时实体，没有 `StableId128`：它以 `snapshot_vehicle_id` 持存并被
停车、live 序引用；静态实体（边、profile、class、停车位和停车设施）用 `StableId128`。
快照局部标识只在单个快照内稳定，恢复经 `SharedIdentityIndex` 与局部标识表
重建。LFRS 7 的边上纵向进度、余量、车速分别为整数毫米、微米与 `u32` 毫米每秒。
LFRS 8 沿用这些字段；窗口的走廊站位、候选后轴的走廊站位、准入速度与沿轨迹行程分别按 §2.1 解释。

**固定预制过渡退役期间。** 在固定过渡边的运行时入口和存量用途迁移完成前，生产
保存与直接摘要沿用 #1217 的 `UnsupportedTransitionState`。活动过渡、预约及当前
格式不能忠实表达的过渡清空历史均按既有边界返回，失败后旧世界可继续运行。
联合版只为 §2.1 的路线交接和预约登记新记录。固定过渡资源的身份及历史不可改写为
普通冲突区、普通路口 `E_f` 或任意位置换道状态。[固定预制过渡合同][lane-change-contract]

arrival 不保存独立布尔值。恢复后只从同一 Reserved binding 与已保存车辆状态派生：
route handle 必须是该 binding 所属 route，`route_edge_index` 必须等于 exact entry
occurrence，`progress_mm` 必须精确等于解析后的整数毫米 anchor，且
`speed_mm_s == 0`、`carry_um == 0`。任一项不同都不是 Arrived；reader 不允许用同一
LaneEdge、距离容差或“已经越过”补成到达。virtual entry 的 semantic anchor 恢复为目标
修订内的 typed selector后，仍必须满足上述同一谓词。

Reserved entry 还必须从保存的 vehicle cursor 前向可达。对同一 route，entry occurrence
更大即合法；occurrence 相同时，entry progress 必须更大，或 progress 相同且
`carry_um == 0`。因此“同一 `progress_mm` 但保存了非零 carry”既不是 Arrived，也不是
仍可到达的 reservation，reader 必须拒绝，不能清零 carry 修复。

**#303 G1 已接受合同**：观测导出/admission session/基线、动态成本快照、未注册
候选与成本 provenance 是调用方拥有或可重建的交付状态，不进入快照。候选注册
成功后形成的普通路线只按上表路线表示保存。任何成功恢复建立新的世界世代/
观测 stream；恢复前的 session 与
未注册候选全部 stale，不因同修订恢复而复活；新 stream 从初始
`observationStateSequence` 开始（Routing 合同 §5）。路线表恢复还必须经唯一
`register_admitted_route` 规范化入口核对修订、稳定标识、
两个路线 occurrence 容量与共同路线编译器；超限零部分恢复。

输入命令游标只统计成功应用的生命周期命令；direct / candidate / admitted 三类路线
注册在共同提交点恰好计数一次，保存不计数。游标以 checked arithmetic 推进；耗尽时
所有路线、生成、替换与停车入口都在修改状态前失败关闭，不允许回绕或饱和伪装成功。

## 4. 容器

封闭契约：size-prefixed FlatBuffers、file identifier `LFRS`、`formatVersion = 7`；
schema 位于 `schemas/runtime-snapshot/v7`；生成物隔离于私有 wire package（沿
`laneflow-road-editing-wire` 先例）。读取 verifier-first：语义 lowering 前完成
长度、基数与版本预检。确认 `formatVersion = 7` / `runtime_state_version = 5` 后，
reader 还逐 table 拒绝超过现行 schema 对该 table 登记字段数的 vtable 槽；该上界只在
容器 7 和状态 5 两个版本 gate 成功后选择。FlatBuffers verifier
本身允许旧 reader 忽略未知字段，不能替代禁绑字段的封闭性检查。发布链的自定义
规范制品仍只有 LFCA / LFSM / LFSD / LFCP；
快照不是发布对象，不要求跨实现字节规范序，只要求逻辑确定性（§6）。
G2 writer 入口为 `encode_lfrs(&CapturedSnapshot)`：它只读边界捕获，不回读活动
world；输出为带 `LFRS` file identifier 的 size-prefixed buffer，必需空表也编码为
存在的空 vector。

当前实现只读取 `schemas/runtime-snapshot/v7`；旧 reader/writer 不保留。来源种类与宿主提供的
目标来源种类必须一致；`Published` 只能携带发布来源表，`Editable` 只能携带编辑来源表，
缺失、多余或未知种类都在交通状态 lowering 之前拒绝。恢复先解析
全部 target/anchor StableId，再验证
显式排他性和每设施 `reserved + occupied <= virtual_capacity`，成功后才发布 world。

## 5. 保存与恢复

- 保存只接受活动聚合的 `CommittedNetworkSource`；working / candidate 与
  `EditableDiffBase` 不入档。候选准备期的保存仍只捕获旧聚合（切换文档 §8）。
- **共享路网修订认证（#955 G1，见
  [`traffic-runtime-state-integrity.md`][state-integrity] §4.2）**：
  宿主在恢复与跨修订迁移把重建 / 重新认证的修订投入使用之前执行共享路网修订认证；结论非
  `Pass` 时不恢复、不发布 world。保存路径不加完整性门控；检测滞后内被保存的检查点
  可能包含坏状态（完整性设计 §5 第 7 条）；对审计重放检出的分歧，宿主按完整性设计
  §4.3 判因并决定是否丢弃相关检查点。若最早保留的检查点本身已含失真，重放可能自洽
  而不产生分歧，不能据此检出或确定应丢弃的检查点。
- **恢复对称原则**：editable 与 published 两类来源都先重建可信根再恢复——
  editable 从 committed `RoadEditingState` 重编译，published 经
  `PublishedLfcaReference` 重新认证。同修订判据 = `NetworkRevisionId` +
  `networkRevisionDerivationVersion` + 契约版本精确相等（与切换文档 §3 一致）；
  origin 字节差异仅承担来源审计，published 重发布的摘要错配在判据满足时允许
  恢复。资产缺失、重编译失败、版本不匹配或缺 `EditableDiffBase` 对应关系失败
  关闭。editable 恢复由宿主按快照中的道路编辑状态摘要取回状态并重编译；Runtime 检查
  状态摘要与快照绑定一致、差异基线与目标修订逐字节对应
  （`committed-road-editing-state.md` §4–§6）。
- 发布资产启用道路编辑须另带 committed 道路状态并完成同修订
  root / source / diff-base rebase；成功前不得启用编辑。
- **捕获与编码的失败关闭分工（#532）**：`capture_snapshot` 的全部容量按计数
  可失败预留（路线/车辆/边表、句柄查表 HashMap 与 source 克隆的 asset key），
  预留失败返回 `SnapshotCaptureError`，世界无感知、宿主清点后可直接重试；
  `LFRS` wire 编码含第三方 FlatBufferBuilder 的内部分配，失败关闭化不可达，
  保存路径的失败关闭由捕获侧承载（编码输入是已物化的有界快照）。
- **完整性原则**：恢复的状态必须满足与 `install` / spawn 命令路径同一的不变量
  集。语义 lowering 拒绝：重复局部标识、悬空引用、live 序不是活跃车辆的精确排列、
  parking 状态矩阵不一致（只允许 `Active + None/Reserved`、`Parked + Occupied`、
  `Completed + None`）、Reserved binding 所属 route/entry occurrence 与 vehicle record
  不一致或 entry 不再前向可达、车辆值不变量破坏（`carry_um` 越界、进度超
  边长、超速、profile 与 class 不一致、活跃车辆物理重叠），以及时钟不变量
  破坏（`time_ms` 必须等于 `tick × fixed_delta_time_ms`，checked arithmetic
  验证——不可达时钟对会派生出错误的信号与回放分歧）。任一违反零部分恢复。
- 路线从有序稳定边序列重编译一次，联合版一并验证已登记的交接窗口。staging world 先累计 exact edge/conflict 两类出现记录
  总数并核对快照与目标容量，再以保存的 `carry_um` 直接提交最终车辆状态。每个
  `Active` 都必须满足冲突仲裁能力缺席期的 3A 车尾清除谓词；`Parked/Completed`
  不经过瞬时 `Active`。任一失败不发布 world。
- 重叠准入复用命令路径的物理边候选索引，比较路线起点截断后的非空车身并拒绝
  重合的零进度前杠入口点（`vehicle-following.md` §7.4）。索引从恢复结果建立，
  不写入快照；Completed / Parked 不登记为道路占用。
- 普通恢复严格核对已保存的 Waiting traversal/membership；缺失状态不得自动补成
  `PreGate`。首次 Waiting 覆盖的零历史初始化只属于显式跨修订切换（切换文档 §3.3）。
- Conflict eligibility 的 committed authority 谓词由 aggregate 校验、普通恢复、跨修订
  迁移与独立期望投影共用：Active、无 reservation、route/occurrence locator 精确、车辆
  位于 admission Gate、且当前所选 policy 决策为 `Candidate`。任一边界不得只复用其中
  的位置子谓词。
- 恢复成功后世界处于快照 `tick` 边界的一致状态；`install` 核对与
  `register_admitted_route` 规范化路线重建沿现行消费契约执行。
- 恢复出的 Conflict eligibility/reservation 可继续进入正式 fixed tick、保存、同修订
  换根、跨修订迁移或显式 despawn。首拍由同一 single-writer 组合路径推进
  `Clearing`、tail-clear 与 lag history，不丢失或重新授予既有 authority。
- G2 fresh restore 入口 `restore_lfrs` 的顺序不可绕过：调用方 wire / asset-key
  上限 → size prefix / file identifier → 有界 FlatBuffers verifier → 版本/绑定/配置与
  表基数预检 → 标识、引用、排列、停车和值不变量 lowering → 局部 world 路线/车辆/
  占用重建 → 执行能力校验。任一失败只丢弃 staging；成功才返回 world 与快照局部路线/车辆 ID 到新
  句柄的映射。fresh restore 从初始 `WorldGeneration` / 初始观测 stream 建立；调用方
  不得让同一 `world_id` 的旧 world 或旧 session 并存。Published 目标允许同修订
  重发布的 digest / length 不同，但目标 source 与目标修订、快照 source 与快照修订各自的
  `NetworkRevisionId` 必须闭合。

## 6. 回放与确定性状态摘要

- 输入命令序列由宿主记录并按序重放（术语表：输入命令序列）；Runtime 不新增
  隐藏输入来源。
- **回放身份**：命令目标用耐久身份（快照局部标识或宿主自有 ID），不用进程
  句柄；恢复经「局部标识 → 新分配句柄」映射重提交；检查点之后由命令新建的
  实体，其耐久 ID 由命令载荷自带。
- **恢复等价**：交通命令与步进的结果、事件和错误（含其中句柄所指的车辆）只由
  已提交状态决定，不依赖槽位分配、generation 或派生索引的内部顺序；同一已提交状态
  恢复后给出相同结果，句柄按新世界映射。管理接口中以槽位序号标识车辆的进程内诊断
  （如路网切换的重验证失败）不在此列。
- **G2 身份接缝**：`CapturedRoute` / `CapturedVehicle` 以只读访问器公开局部 ID 与
  逻辑字段；`RestoredSnapshot::route_mappings` / `vehicle_mappings` 返回按局部 ID
  升序的完整新句柄表，单 ID 查询只作便利入口。宿主据此把自己保存的耐久 ID
  重绑到本次进程句柄；Runtime 不保存或解释宿主 ID。
- **#303 G1 已接受合同——候选路线回放**：`register_candidate_route` 的世界世代、
  观测与成本绑定不进入命令序列。成功准入后只记录规范化路线注册命令：宿主耐久
  路线 ID、当时的路网修订标识/派生版本和有序 LaneEdge `StableId128`。恢复后核对
  当前修订、解析稳定标识并经同一 `route_edge_occurrence_capacity` 与唯一路线编译器
  的 `register_admitted_route` 入口重建，再把新句柄映射回宿主 ID；不重新执行
  Routing，不重放 stale admission。
  跨修订不匹配必须经 #302 受信任迁移策略显式迁移命令稳定引用，否则拒绝。
  联合版中，注册带横向交接的路线时，规范命令还须携带 §2.1 的全部登记窗口，
  通过同一出现对绑定检查。这样重复边上的第二次交接仍由原下标定位。
- **确定性状态摘要（术语表）**：唯一算法为 BLAKE3-256。按
  [ADR 0033][adr-0033]，它属于内部
  完整性摘要；#1035 将算法由 SHA-256 更换为 BLAKE3-256，并把摘要版本由 `7` 递增为
  `8`，规范原像的内容、排序与字节布局不变；不保留 SHA-256 计算或比对路径。公开入口
  `deterministic_state_digest` 与 `TrafficWorld::direct_state_digest` 返回按角色命名的
  32 字节类型 `StateDigest`，不再复用 `Sha256Digest`。域分隔字节保持
  `laneflow:runtime-state-digest:v1\0`，摘要版本字段已在原像内区分算法更换；随后写
  当前生产组合的 `u16` 摘要版本 `8` 与 `u16` `runtime_state_version` `5`；整数均小端。
  下文给出该组合的原像。联合版在同一位置依次写 `9:u16` 和 `6:u16`，保留域分隔与
  §6 的分组、live 序和记录长度规则，路线和车辆记录的新增字段见 §2.1。顶层依次写：
  `world_id`、语义
  `NetworkRevisionId`、六轴静态契约版本、行为语义配置（vehicle/route、edge/conflict
  两类 occurrence 容量与 fixed dt；不写 worker）、tick/时间/双游标、策略选择
  （`u8`：NotRequired=0，Pinned=1 后紧跟 16 字节 policy StableId）、按 stable
  WaitingZone identity 排序的 `occupancy + nextAdmissionSequence` 记录，再写路线实例
  分组多重集和 live 更新序列。每条记录写 `u64 byteLength + bytes`，每个集合先写
  `u64` 数量。路线实例
  分组 = 路线记录 + `u64` 绑定车辆数 + 按记录字节升序的绑定车辆记录多重集；路线记录 =
  `u64 edgeCount` + 有序边 `StableId128`。车辆记录依次写 route index / progress / carry /
  speed、status 封闭 `u8`（Active=1/Parked=2/Completed=3）、profile/class
  `StableId128`，随后写下列 parking binding：

  ```text
  parkingBindingPresence:u8       // 0=Absent, 1=Present
  if Present:
    bindingState:u8               // 1=Reserved, 2=Occupied
    targetKind:u8                 // 1=ExplicitSpace, 2=VirtualPool
    targetStableId:StableId128    // ParkingSpace 或 ParkingFacility
    if bindingState == Reserved:
      entryRouteEdgeIndex:u32     // vehicle 所属 route 内的精确 edge occurrence
    semanticEntryPresence:u8      // 0=Absent, 1=Present
    if semanticEntry Present:
      entryLaneEdgeStableId:StableId128
      entryProgressMillimetres:u32
  ```

  `entryRouteEdgeIndex` 当且仅当 Reserved 时存在，并按 `u32` 数值区分同一 LaneEdge 在
  route 中的不同 occurrence；它必须解析到该 target 的 entry。`semanticEntry` 当且仅当
  `Reserved + VirtualPool` 时存在；ExplicitSpace 和 Occupied VirtualPool 均禁止该字段。
  parking binding 后继续写可选 maneuver traversal（route occurrence、ManeuverPath
  stable identity、`PreGate/Committed/Waiting` phase 与 phase Gate stable identity）
  和可选 Waiting membership（WaitingZone、maneuver occurrence、entry/release Gate
  stable identity 与 admission sequence）。parking binding 不重复写 route handle/ID：车辆所在 route group 就是 Reserved binding
  的 route，改变 vehicle 的 route group 已会改变摘要。Parked 车辆必须是 Occupied
  binding，Reserved 车辆必须是 Active，Completed 必须 unbound。counts/capacity 从
  绑定与共享修订派生，不进入摘要。这样只改变 target tag、Reserved/Occupied 状态、
  route occurrence 或所选 virtual entry 的两个世界一定产生不同摘要。

  路线内容由所属分组承载，车辆记录不内嵌。分组多重集按完整分组字节升序；绑定分布
  形状因此可区分（分挂与同挂的 remove 语义不同），内容与绑定均相同的实例可交换
  （全部后续命令行为一致）。
  摘要计算的全部缓冲按计数可失败预留（#532：记录表/分组表用「按局部 ID
  排序的扁平表 + 二分查找」承载原 `BTreeMap` 语义——`BTreeMap` 无可失败预留
  API，输出字节不变）；预留失败返回 `SnapshotDigestError`，无副作用、可重试。
  已知大小的记录表和单条记录一次精确预留；分组及顶层规范字节缓冲的连续追加使用
  可失败的摊销增长，不逐记录精确扩容。容量与增长策略不进入规范原像或摘要版本。
  路线分组先建立按路线实例 ID 和车辆记录字节排序的扁平成员表，再按路线消费连续
  区间，不逐路线扫描全部车辆。空路线仍形成分组，最后按完整分组字节排序；该算法
  不改变规范原像或摘要版本。成本包含成员排序、规范记录字节与哈希，不承诺全流程线性。
  等价类边界：同内容实例间的**归属**差异（哪条实例拥有哪份绑定集合）是实例
  重标记轨道上的同一逻辑状态，摘要不区分——区分它只能依赖槽位派生的局部
  ID，与下述「局部 ID、句柄与表排列变化不影响摘要」直接冲突（restore 的
  Parked-先 staging 本身就会重排局部 ID）。其可观测面（如对特定实例的
  `remove_route` InUse/成功）由宿主耐久 ID 映射与命令结果比对承接，检查点
  比对可纳入身份映射；失同步显形因此可能延迟到首个路线目标命令之后，首个
  分歧区间未必包含归属置换时刻，根因定位归宿主映射比对。
  live 序按运行时更新顺序逐位写「车辆记录 + 所属路线记录」：跨路线换序可
  区分（pose 批次按 live 序产出，顺序可观测）；所属路线内容相同的同值车辆
  换序保持等价（有序对内容相同，全部后续命令行为一致）。这样局部 ID、句柄
  与表排列变化不影响摘要，但多重度、车辆-路线绑定与 live 顺序仍被保留。LFCA exact-byte digest/length、Published 审计来源、
  worker、`WorldGeneration`、观测 session 不入摘要。同一逻辑状态在任何合法容器编码
  与恢复句柄分配下摘要相等；切换事务的期望值从源捕获独立计算，只允许 target origin
  替换与首次 Waiting 覆盖的零历史 `PreGate` 规范化（切换文档 §3.3/§5），不省略
  traversal 摘要字段，也不从候选动态状态取值。
- 检查点（checkpoint）是回放序列中的快照锚点，与命令游标共同界定重放区间；
  G2 对拍点冻结为 `(command_cursor, tick, deterministic_state_digest)`；宿主从检查点
  依序重放并比较各点，首个不等点与上一个相等点构成首个分歧区间。失同步只诊断，
  不自动纠偏、不重启世界，处置归宿主。
- **完整性接缝（#955 G1，见
  [`traffic-runtime-state-integrity.md`][state-integrity]）**：
  C(T) 的完整性检测由宿主承担：宿主检查点对拍与审计重放（在独立世界、独立进程或
  另一台机器上从较早检查点重放并比对三元组；被审计世界仍在运行时只在独立进程或另一台
  机器上重放，因为同进程不得并存同 `world_id` 的世界；审计结果分完整一致 / 分歧 / 未完成，
  未完成区间与跨切换、恢复边界的区间不计入覆盖，见完整性设计 §4.3）在起点检查点不含失真时覆盖进入 C(T) 且
  改变摘要等价类的失真（同一等价类内的归属置换见上文「等价类边界」，由宿主身份映射
  比对承接）；换机器重放是判因证据，不单独构成判因，原始分歧在判因前按失同步诊断。Runtime 不登记检查点锚或命令摘要。完整性失配与失同步共用同一
  处置裁决权（归宿主），区别只在是否停用：失同步状态仍是合法确定性状态，只诊断不停用；
  完整性失配的对象可信度已被破坏，宿主先停用再按检查点 + 输入命令序列处置。共享路网修订
  不在 C(T) 内（ADR 0030），其失配不属回滚 / 重演可修复面，只能修订重建。完整性不进入
  本快照、确定性状态摘要原像与 tick 路径，digest 7 / runtime state 5 版本轴不变。

## 7. 跨修订迁移入口

跨修订恢复必须由受信任 `NetworkRevisionCutoverDescriptor` 驱动、经 LFSD 显式
迁移（切换文档 §3）；旧密集序号不得直接解释为新修订实体，稳定引用经
`SharedIdentityIndex` 完整重建。同修订换根（含重发布制品）走切换事务的
`same_revision_restore` 路径。

跨修订恢复的停车重绑与在线 cutover 使用同一规则，正本见
[修订切换 §3.1][revision-cutover-parking]。本文不另写一套入口、前移或失败条件。

## 8. G1 预算与度量

度量协议与切换文档 §9 相同：同机描述性基线；1.0 前可依据可复现证据收紧、放宽
或重构，影响合同或验收结论时返回 G1。维度：快照制品
exact bytes、save 停顿（墙钟与主线程）、load 停顿（editable 重编译与
published 认证分别度量）、恢复峰值内存、保存期间对稳态 tick 的干扰。

证据按仪器隔离：`snapshot_budget_evidence.rs`（`stats_alloc` 账本，随 CI 运行，
硬断言保存前后稳态 tick 分配账本相等与保存后确定性）、`snapshot_peak_evidence.rs`
（独立 DHAT integration binary，量恢复调用新增堆块高水位）、
`snapshot_wall_clock_evidence.rs`（未插桩 `#[ignore]`，量 release 墙钟中位与后台
编码竞争）。复现命令：

```text
cargo test --release --locked -p laneflow-runtime --test snapshot_budget_evidence -- --nocapture
cargo test --release --locked -p laneflow-runtime --test snapshot_peak_evidence -- --nocapture
cargo test --release --locked -p laneflow-runtime --test snapshot_wall_clock_evidence -- --ignored --nocapture
```

### 历史 B 部分初值（v1 published fresh restore）

切换前 v1 的同机描述性初值（`LF-P100-REF-01`，2026-08-28，rustc 1.98.0，release；
workload `signalized-corridor-v1`，LFCA exact `420,332` bytes、28 条 catalog 路线、
2 车辆、`4 ms` 固定步进，快照点 `tick = 64`、`command_cursor = 30`）：LFRS exact
`3,248` bytes；主线程 `capture_snapshot` 中位 `0.002 ms`、`encode_lfrs` 中位
`0.0072 ms`；published `restore_lfrs` 整次中位 `0.063 ms`；恢复增量堆高水位
`19,240` bytes / 272 blocks；保存前后稳态 tick 分配账本相等（实测均 0），后台
4,096 次 encode 竞争下 tick 墙钟与无竞争基线相同。不得作为当前版本（v7）exact
bytes 或性能结论复用；仪器隔离、逐项分配账本与完整表格见基线提交 3f80b65f 的
本文副本。editable load 的现行初值见下文「2026-10-28 editable load 初值」。
### 2026-10-19 再登记（#537，exact head `6cf9056365dcf106bedeba85035863c4fda5a8c1`，v6）

同机 LF-P100-REF-01、rustc 1.98.0、Windows build 29683；fixture 为当前 corridor
（LFCA exact `425,418 B`，28 路线、2 车、4 ms 步进），故与B 部分 初值（v1、`420,332 B`）
不作回归比较；本节为 v6 当期登记基线，现行版本为 v7（见 §9），v6 数值保持原身份。未插桩墙钟中位：`capture_snapshot` 主线程
`0.0016 ms`；`encode_lfrs` 无竞争主线程中位 `0.0029 ms`（后台化可行性沿 §8 初值节
口径，竞争影响由竞争 tick 行覆盖）；published `restore_lfrs` 整次
`0.0324 ms`；基线 tick `0.0012 ms` / 后台 4,096 次 encode 竞争下 `0.0027 ms`。
账本：capture `37` 次分配 / `5,391 B`（净 live `3,563`）；encode `34` 次分配 + `13` 次
重分配 / `9,672 B`（净 `7,640`）；restore `562+12` / `50,368 B`（净 `43,960`）。DHAT 恢复
增量同时存续峰值 `45,472 B / 418 blocks`、返回 live `44,152 B / 412 blocks`。LFRS
exact `3,312 B`。保存前后稳态 tick 账本**相等**的硬断言通过（实测两侧均 `0/0`；
断言对象是相等性，不是 0 值本身）。已实现的 §9 义务的回归面随 CI 保持；本节未新增
覆盖，墙钟类证据为手动取证；Editable 来源与 editable load 见下文 #529 节。完整记录、
原始输出与复现命令见固定提交中的
[#537 研究报告][target-path-baseline]（结果表为数值正本）。

### 2026-10-28 editable load 初值（#529，v7）

同机 LF-P100-REF-01（AMD Ryzen 9 9955HX）、rustc 1.99.1、release、Windows build 29695。
工作负载为 `laneflow-urban-generator` 按 `examples/config/cn-urban.toml` 生成的城市路网，
编辑来源为两份 `LFRE` 模块；走廊 fixture 只有 LFCA、没有 `LFRE` 来源，不能用于编辑来源，
因此本节与发布来源初值不作回归比较。宿主以 harness mixed-peak 计划运行 64 拍后，经同修订
换根切到编辑来源并保存。宿主重编译（逐模块准入、编译、发射与后发射检查）与生成器产物
逐字节一致；修订构建为 headless（`SpatialBuildOption::Omit`），与 harness 宿主同一组参数。
数值为未插桩墙钟中位，只描述本机，不是产品阈值。

| 项目                               | fixture（9 样本） | 10k（5 样本） |
| ---------------------------------- | ----------------- | ------------- |
| 保存时在途车辆                     | 2,000             | 10,000        |
| `LFRE` exact bytes（2 模块）       | 1,670,208         | 8,345,128     |
| LFCA exact bytes                   | 8,868,299         | 44,328,051    |
| `LFRS` exact bytes                 | 241,200           | 1,219,856     |
| 宿主重编译                         | `1,002.2 ms`      | `8,298.2 ms`  |
| 修订构建                           | `32.9 ms`         | `176.2 ms`    |
| `restore_lfrs`（编辑来源）         | `3.8 ms`          | `29.6 ms`     |
| 编辑来源修订认证（另列，含重编译） | `1,058.9 ms`      | `8,422.0 ms`  |

editable load 停顿 = 宿主重编译 + 修订构建 + `restore_lfrs`，由重编译主导；恢复之前的
修订认证（§5）另列，它在内部再做一次重编译与候选修订构建。复现命令（`LF_EDITABLE_LOAD_SCALE`
取 `fixture` / `10k` / `100k`，`LF_EDITABLE_LOAD_SAMPLES` 取样本数）：

```text
cargo test --release --locked -p laneflow-urban-harness --lib editable_load_evidence -- --ignored --nocapture
```

## 9. Runtime Snapshot v7 的 G2 边界与必测义务

v7 的 schema 与版本轴到字段映射已在
`schemas/runtime-snapshot/v7/README.md` clean-generate 并逐项绑定本文 §3–§6；摘要输入的
精确规范化序列化见 §6。历史版本的 G2 证据不能自动替 v7 通过；不受当前字段集改变的
义务继续作为回归 oracle。

必测义务：save → load exact oracle（逻辑状态含双游标与 `time_ms` 全等，句柄
不比对）；检查点 + 命令重放逐点相等与失同步定位；`fixed_delta_time_ms` 不一致
拒绝；语义容量按 §2 判据（恢复只许放大，精确回放对拍要求容量一致，不一致时
分歧按失同步信号处理）；worker 数差异不影响 exact 语义；容器拒绝面（未知版本 / 长度 / 基数 / 禁绑
字段）；完整性拒绝面（标识 / 引用 / 排列 / 停车绑定 / 值不变量 / 时钟不变量）；两类来源
恢复端到端与 published 同修订重发布恢复；边界捕获拒绝跨提交状态混合；候选
准备期保存只捕获旧聚合。路线 edge/conflict occurrence 容量 max/max+1 均须恢复原子；
检查点后成功候选注册按规范化命令重放且不调用 Routing/旧 cost binding。parking
状态矩阵、Reserved route ownership、entry occurrence/edge 闭合与
前向可达单项反例（含同 `progress_mm` 非零 carry）；跨修订 virtual semantic entry exact
重绑，以及 explicit entry 前移允许/移到 cursor 后方失败；任一失败零部分恢复。冲突
出现记录不得入档；恢复须证明 exact 重建计数、3A 的一微米边界、保存 carry 生效以及
Completed 不经过瞬时 Active。捕获与摘要的预留失败注入须证明失败关闭（世界无感知）、清点后重试得到
同一快照/摘要；save 路径的失败关闭由捕获侧承载（编码边界见 §5）。

Conflict downstream 区间不是 route occurrence 的第二份持久索引。writer 先从
reservation 的 exact route/Gate/passages、车辆全长与当前边长重建物理区间并集，要求与
committed 热状态相等，再按 `(edge StableId, startMm, endMm)` 写入。reader 先检查这一
wire 规范序，解析到当前修订后按 ordinal 排序，再从 reservation 证明独立重建并逐项比较。
以上为 v7 的单车记录。v8 的预约终点、正向重建及负向验收以 §2.1 和
`waiting-zone-conflict-right-of-way.md` §6.11 为准。
循环路线中同一物理边的重叠 occurrence 可以合并；合并区间没有唯一 route hop，禁止用
“Gate 后第一个同边 occurrence”补造来源。wire owner 由所属车辆记录给出；恢复后完整
reservation 只由 `ConflictArbiter` 持有，车辆 `Clearing` 只保存 Gate route anchor。
最小跟车间隙由 profile 派生。reservation passages 必须精确等于重编译所得 Gate range，
vehicle front 必须位于 Gate crossed side。已清 reservation cell 必须存在同地址
`ActualClear`，且时间不得早于 `acquiredTick × fixedDeltaTimeMs`；缺行、过早时间和
`CutoverFloor` 均拒绝。

writer 在进入逐车编码前，从 `ConflictArbiter` 的 committed reservation 与 downstream
ledger 一次建立按 vehicle slot 寻址的临时只读视图；同一 reservation 的 claims 必须连续、
计数和内部 serial 必须闭合。此后每车只读取自己的 claim slice，不得为每份 reservation
重新扫描完整 ledger。该冷路径总工作量为
`O(vehicleCapacity + liveReservations + downstreamClaims)`，临时索引的预留失败仍使用
快照捕获错误轴失败关闭，不进入 fixed-step 热状态。

### 当前测试覆盖与遗留边界

下表说明当前功能测试覆盖；tagged target、Reserved route occurrence、
virtual semantic entry、两类路线 occurrence 容量、资源守恒、未知 parking 枚举和
Waiting traversal/membership/counter、Conflict eligibility/reservation/lag、digest 往返均纳入
现行回归测试。

| 义务                        | 当前事实                                                                                                                                                                                                                                                                                                                           |
| --------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| save → load exact oracle    | 已覆盖：完整逻辑状态、双游标、`time_ms` 与局部 ID → 新句柄映射；句柄值不作 oracle                                                                                                                                                                                                                                                  |
| 检查点回放 / 首个失同步区间 | 已覆盖：宿主耐久 ID 重绑、检查点后新实体 ID、已准入路线稳定边序列重放；逐点 `(command_cursor, tick, digest)` 相等，偏移 spawn 命令定位首个分歧区间                                                                                                                                                                                 |
| 配置判据                    | 已覆盖：fixed dt 不等拒绝，vehicle/route/edge occurrence/conflict occurrence 四类语义容量缩小拒绝/放大允许，执行配置另行显式提供，交通错误先于执行能力错误；容量不同不冒充 exact replay                                                                                                                                            |
| 容器与完整性拒绝面          | 已覆盖：framing / identifier / verifier / wire 与 asset-key 上限、format/runtime/静态版本、未知 vtable 槽/枚举、过短 vtable（verifier 只保证缓冲区内且偶对齐，显式拒绝先于字段计数）、必需字段、标识/引用/live 排列/停车/数值/时钟/Active 重叠；错误只返回失败，不暴露 staging                                                     |
| Published 来源              | 已覆盖：端到端 fresh restore；同语义修订、不同 asset key / exact-byte digest / length 的已认证重发布来源允许恢复                                                                                                                                                                                                                   |
| Editable 来源               | 已覆盖：安装、快照恢复、同修订换根与跨修订直移的差异基线对应关系，四种来源转换，编辑来源绑定往返与拒绝面；宿主重编译后先认证再恢复的端到端 exact oracle（逻辑状态摘要全等、继续运行逐 tick 相等），含同修订不同构建字节的恢复（#529）                                                                                              |
| 边界捕获 / 候选准备期保存   | 现行实现保持结构闭合：`capture_snapshot(&self)` 与所有提交入口 `&mut self` 不能在 safe Rust 中交错。切换候选在同步 `&mut self` 调用内局部持有、无可并发观测的半提交 world，调用前捕获旧聚合、成功返回后捕获新聚合；未来异步候选形态必须补可交错定向测试                                                                            |
| occurrence max / max+1      | 已覆盖：edge 与 conflict 两类总 occurrence 正好等于保存/目标上限时恢复成功；max+1 分别以 `RouteEdgeOccurrences` / `RouteConflictOccurrences` 上限错误失败；实际 conflict 总数由完整 staging 路线表重建后核对                                                                                                                       |
| 冲突能力保护                | 已覆盖：保存的微米 carry 参与车尾位置；clearance 前一微米拒绝、相等允许；Completed 直接恢复最终态，不经过瞬时 Active；失败零发布                                                                                                                                                                                                   |
| Conflict 持久状态           | 已覆盖：firstEligibleTick 的 None/tick 0 区分、Clearing owner/passages 与可重建 downstream 物理并集往返、1 mm 改写/缺区间拒绝、ActualClear tick 0、cleared cell 历史类别闭合、悬空 locator/错误 occurrence/重复及 future history 拒绝；same-revision 原样保持，cross-revision 使用最终 T_commit floor 并在连续再次切换时保留原基准 |
| 快照五维预算                | Published 初值与 editable load 初值均已登记于 §8                                                                                                                                                                                                                                                                                   |

[adr-0020]: ../adr/0020-compiler-owned-static-network-and-static-image.md
[adr-0021]: ../adr/0021-traffic-infrastructure-and-host-boundary.md
[adr-0028]: ../adr/0028-integer-millimeter-traffic-geometry.md
[adr-0029]: ../adr/0029-retire-precompiled-static-route.md
[revision-cutover]: traffic-runtime-revision-cutover.md
[revision-cutover-parking]: traffic-runtime-revision-cutover.md#31-停车-target-的重绑合同
[routing-integration]: traffic-observation-and-routing-integration.md
[retire-precompiled-static-route]: retire-precompiled-static-route.md
[shared-static-network]: shared-static-network.md
[parking-system]: parking-system.md
[right-of-way]: traffic-runtime-right-of-way-policy.md
[lateral-handoff]: cross-section-stationing-and-lateral-geometry.md#41-从换道意图到一次路线交接
[clearance-speed]: cross-section-stationing-and-lateral-geometry.md#5-分段余量与速度
[integer-geometry]: traffic-runtime-integer-geometry.md
[execution-config]: traffic-runtime-execution-config.md
[committed-editing]: committed-road-editing-state.md
[lane-change-contract]: lane-change-minimal-contract.md
[state-integrity]: traffic-runtime-state-integrity.md
[adr-0033]: ../adr/0033-digest-role-and-algorithm-selection-authority.md
[target-path-baseline]: https://github.com/illusion-tech/laneflow/blob/e267d4c9eead22d14f7111555a70c70637cd9f04/research/issue-537-target-path-baseline/README.md
