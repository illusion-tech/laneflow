# 中国特色城市工作负载 v1

**文档状态**: Accepted（#304 G1；§4 保留 #540 Accepted 停车合同）<br>
**最后更新**: 2026-09-07<br>
**适用范围**: `LF-CN-URBAN-v1` 的 topology / demand / runtime 分层、计数口径、
首批场景边界与阶段依赖<br>
本文定义工作负载的交付合同，不以设计接受代替生成器、运行程序或产品认证。
G1 判断、交付进度与本次测量结果保存在 GitHub Issue / PR。
**关联文档**:

- `parking-system.md`
- `core-runtime-performance-baseline.md`
- `waiting-zone-conflict-right-of-way.md`
- `cross-section-access.md`
- `signalized-corridor-protected-turning.md`
- `real-road-workloads.md`
- `traffic-runtime-right-of-way-policy.md`
- `traffic-runtime-snapshot.md`
- `traffic-runtime-revision-cutover.md`
- `../adr/0010-parking-binding-and-vehicle-lifecycle-authority.md`
- `../adr/0021-traffic-infrastructure-and-host-boundary.md`

## 1. 目的和边界

`LF-CN-URBAN-v1` 将中国城市常见的交通规则和工作负载组织为可复核的通用技术
验证场景，用于检验交通闭环、规模口径和边界成本。场景选择不指定具体产品，
也不代表完整行为域或规模目标已经交付。

工作负载（Workload）分三层，唯一执行入口为 `laneflow-runtime` / `TrafficWorld`：

1. **topology artifact**：道路、路口、信号、准入、停车等不可变静态事实；
2. **demand artifact**：可重放的出行请求、精确停车目标、生成/离开序列与 provenance；
3. **runtime artifact**：实际 live/active/parked/presented 状态、命令序列、摘要和性能证据。

三层必须独立计数和版本化。城市人口、停车容量、静态表行数、道路 Active vehicle 和
引擎 presented entity 不是同一个数字，不能相互代替。

首版交付一套可生成的连通路网、七套互补需求计划、无界面运行程序（Headless Harness）
和有限的跨层验证。首批行为限于现有道路机动车、跟驰、信号、WaitingZone、
ConflictArbiter 与停车合同。生产组件不得识别工作负载名称、case ID 或 seed 决定行为。

需求、路线选择和生命周期编排属于调用方。七套计划改变出发方向、时刻、停车命令和
观察窗口，复用同一档静态路网；不为早晚高峰复制 LFCA，不在运行程序中重做通行权。
首版使用仓库自有的合成编制输入；不引入外部地图下载或新的法规研究依赖。

## 2. 规模和计数口径

每个交通执行域 `d` 至少报告：

- `N_individual[d]`：仍保留完整 identity 和 committed state 的个体；
- `N_active[d]`：当前 tick 参与该执行域运动/约束的个体；
- `N_intent[d]`：该固定步进实际重新计算昂贵行为或控制意图的个体数；
- `N_presented[d]`：当前交给引擎表现的个体；
- `N_aggregate_records[d]`：真实存在的聚合记录；
- `N_aggregate_equivalent[d]`：聚合记录代表的等价规模，仅在定义了转换口径时报告。

停车另报告：

```text
N_parking_facility
N_parking_space_explicit
N_parking_virtual_anchor
C_parking_virtual_declared
B_parking_explicit_reserved / occupied
B_parking_virtual_reserved / occupied
```

必须满足：

```text
N_individual[road_motor_vehicle]
  = N_active[road_motor_vehicle]
  + N_parked_explicit
  + N_parked_virtual
  + N_completed_live

N_presented[road_motor_vehicle]
  不包含 virtual Parked，但 virtual Parked 仍计入 N_individual
```

`C_parking_virtual_declared = 100_000` 不意味着有 100,000 个 `ParkingSpace`、LFCA 行、
Runtime slot 或 presented entity。

`Completed` 的句柄在移除或原子替换前仍 live，必须计入个体数。待提交的出发/离场
请求另报调用方队列长度，不得写入 `N_intent`。首版不使用聚合表示，两项 aggregate
计数为零；未提供真实意图更新计数时报告未测量，不能用请求数或活动数冒充实测值。

## 3. 规模与静态生成合同

### 3.1 两档输入

| 参数                                        |    10k |    100k |
| ------------------------------------------- | -----: | ------: |
| `N_individual[road_motor_vehicle]` 名义规模 | 10,000 | 100,000 |
| 250 m 布局单元（cell）                      |    100 |   1,000 |
| 每块含 `2 × 5` cells 的布局块（macro-tile） |     10 |     100 |
| 主运行 fixed step                           |  16 ms |   33 ms |

两档只改变规模参数，复用同一套道路模板、需求规则和行为精度。cell 是编制网格，
不是独立世界、运行分区或车型类别。观察期间逐 tick 报告真实 live/active/parked/
completed 数量；名义规模不能由车辆容量配置、空停车容量或累计生成数代替。

信号编制统一使用 `528 ms = lcm(16 ms, 33 ms)` 时间量子。版本化模板为每个相位声明
正整数 `duration_quanta`，为 controller 声明非负整数 `offset_quanta`；生成器以受检
整数乘法生成 `duration_ms = 528 × duration_quanta` 与对应 offset，并验证 offset
小于完整周期。相位顺序、灯态和量子数由 #542 的正式模板固定，两档复用，不由
Runtime 取整。这样每个相位均是两档 fixed step 的正整数倍；完整周期是已生成相位
时长之和，#544 直接据此确定观察窗口。

### 3.2 连通性与模板

生成器按稳定的 tile/cell 顺序进行整数铺设；坐标与几何遵守 Spatial 的规范框架
边界和整数毫米交通长度合同。布局块列数取不小于 `sqrt(2 × tile_count)` 的最小整数，
逐行填充实际 tile；不物化末行空余 tile。边界端口、相邻 tile 的连接规则与模板
参数写入版本化生成配置，不为 100k 添加特殊拓扑分支。

每个完整 tile 至少包含：受保护多阶段信号及左转待转区、信号无保护左转、主支路
让行 T 形路口、错位双 T、混合停车设施、地下多门车库及真实可见泊位。各模板通过
公共干支路连接；不得把信号、Waiting 与 Conflict 夹具摆在互不连通的小岛上。

拓扑验收必须证明：

- 全部用于需求的入口、出口与停车锚点接入同一弱连通道路网络；每条计划路线另作
  有向可达、合法 successor、出现项与几何连续性检查，不能以弱连通代替可行驶。
- 路线目录包含实际跨 cell、跨 tile 的行程，以及经过停车入口/出口和受控路口的
  行程；目录是调用方输入，在每世界通过 `register_route` 注册，不写入 LFCA。
- 受保护、`Candidate(Permissive)`、`Candidate(Uncontrolled)` 都有真实的门、流、
  冲突出现项与策略来源；`signalControl:none` 不代表自由通行。
- 地下停车容量通过设施与少量锚点表达；不生成内部道路、等容量伪泊位或虚拟位姿。

首版参与者类别与车辆配置在共同来源模块中共享，不按 cell/tile 复制。`ParticipantClass`
表达准入和路权分类，`VehicleProfile` 表达车长、速度、加减速度和跟车安全参数；仅模型、
颜色或材质不同的外观由宿主表现层处理，不因此新增类别或车辆配置。仅车长相同不能证明
全部运动参数相同。类别、车辆配置与外观数量分别报告；扩大地图不扩大同义目录。

真实不同的物理参数档必须保留并写入固定生成配置，不能为了编译通过合并不同配置。
同义声明共享与策略解析粒度是独立问题：前者避免编制复制，后者按路权专题合同处理。
容量验收不能只凭单配置样本通过，就认定多个不同参数档的正式输入已通过。

### 3.3 精确计数与容量

#542 交付规范生成配置、来源、LFCA/manifest、路线目录与精确对象报告。每个规模在
同一工具链上 clean-regenerate 两次，逐字节比较制品，并核对摘要与 `NetworkRevisionId`。
小型规范夹具可入库，大型制品通过生成命令和摘要复核；远端发布由发布任务另行执行。

报告至少列出：来源声明/引用与导入边、公开 LIR 计数、逐 LFCA 表/关系行数及分块、
道路和车道总长度、设施/泊位/锚点/容量、Gate/Waiting/Conflict/Policy 数量、车型目录
大小、路线数、坐标边界、制品字节、构建/检查/安装结果与内存。
未暴露的 HIR/MIR 内部计数明确记为未测量，以既有准入结果说明是否通过对应上限；
不为报表新增公共编译器 API，不把推导数写成实测数。

G1 冻结规模、语义和生成规则；#542 从正式生成器冻结逐对象计数与摘要。纯派生计数
变化由配置和报告解释并重算；改变场景覆盖、车型种类、布局规则或准入限制才返回
相应设计判断。不得在编写生成器前手工冻结一套无法从实现导出的 IR 总数。

#543 的容量研究只作为实现选择输入。策略、来源、模板或跨块连接变化后，正式生成器
必须重新走完整编译、后发射检查、共享根构建及世界安装。空世界安装不证明 10k/100k
车辆运行容量，未连通样本不证明城市行程。优先复用 `LF-COMP-SINGLE-NETWORK-1M-v2`；
发生拒绝时先记录实际阶段、维度、限值和输入形状，再判断编制问题或独立编译器缺陷，
不得直接放宽上限、减少规模或删除场景。

## 4. 停车切片（#540 G1 合同；#541 当前实现）

### 4.1 代表性场景

首批 topology 必须至少含以下可复核场景：

| 场景              | 静态表达                                                                                                        | 验证意义                                         |
| ----------------- | --------------------------------------------------------------------------------------------------------------- | ------------------------------------------------ |
| 工厂混合停车      | 一个 `ParkingFacility`；多个可见地面 `ParkingSpace`；非零 virtual capacity；地面与室内可共用同一外部道路 anchor | 验证同一外部入口下地面可见与室内不可见两种停驻   |
| 住宅/商业地下车库 | virtual-only `ParkingFacility`，至少一个入口和一个出口                                                          | 验证不可见容量不生成内部路网/泊位                |
| 多门设施          | virtual facility 有多个 entry 和/或 exit                                                                        | 验证 caller 精确选门、路线 occurrence 与离场安全 |
| 路侧显式泊位      | 独立或设施内 `ParkingSpace`                                                                                     | 保留可观察的 exact space/pose 语义               |

多门不是要求每个设施都必须有多个门；单入口/单出口仍是常见合法实例。混合工厂中，
显式泊位与 virtual pool 是两个独立 admission pool，设施总容量仅作报表。

### 4.2 Topology artifact

停车 topology 只物化：

- `F` 个 `ParkingFacility`；
- `S` 个真实可见 `ParkingSpace`；
- `A` 个 virtual entry/exit anchor；
- 每个设施一个 `u32 virtual_capacity`。

不得物化：

- `C_parking_virtual_declared` 个伪泊位；
- 不可观察车库内部 LaneEdge/Route；
- virtual parked pose 或空容量 slot；
- 与车辆数等长的静态停车对象。

### 4.3 Demand artifact

每个停车意图必须指定：

```text
vehicle / trip identity
exact ParkingTarget = ExplicitSpace | VirtualPool
entry route occurrence
virtual target 时显式携带 exact facility entry anchor selector
计划离场时的 route occurrence
virtual target 时显式携带 exact facility exit anchor selector
```

anchor selector 与 route occurrence 是两个正交输入：occurrence 选择动态 route 第几次
经过某条 LaneEdge，selector 选择该设施在这条 LaneEdge 上的哪个整数毫米 anchor。同一
LaneEdge 上存在多个入口/出口时，只有 facility + occurrence 的意图必须视为不完整。

demand/orchestration 层负责基于宿主策略选择 target。Runtime 只验证并执行
exact intent，不自动决定“停地面还是进室内”。满位后的换目标、等待或 reroute 也是
demand/Routing 策略，不在 #540 Runtime authority 内。

### 4.4 Runtime artifact

运行时必须分别记录：

- explicit/virtual reserved、occupied、vacant；
- 每个 live vehicle 的 exact tagged binding，以及只允许
  `Active + None/Reserved`、`Parked + Occupied`、`Completed + None` 的状态矩阵；
- Reserved binding 的 bound route、exact entry occurrence 与 virtual selected entry；
- park/leave/cancel/despawn 的 command result；
- exact arrival 的 occurrence、整数毫米 anchor、`speed_mm_s=0`、`carry_um=0` 以及
  `SignalStop -> ParkingStop -> RouteEnd` 同值归因；
- `N_active`、`N_individual` 和 `N_presented` 的状态变化；
- stable digest/order 和失败零副作用证据。

Reserve/rebind 只接受从 committed cursor 前向可达的 entry；同一 `progress_mm` 但
`carry_um > 0` 视为已越过。合法 reservation 由 ParkingStop 保持，不通过 route completion
自动释放。leave 必须携带恢复 route，并通过 route-aware overlap 与 direct-follower
emergency-envelope admission 后才能提交；该 admission 同时扣除 Following 承诺保留的
gap，不能让下一 tick geometry hard projection 补救。rebind 在车身跨 predecessor 时还
必须保持新旧完整 physical occupancy footprint 相同。

成功 virtual park 后：

```text
N_individual 不变
N_active -= 1
N_presented -= 1 当且仅当该车辆提交前属于 presented set；否则不变
B_parking_virtual_occupied += 1
```

成功 virtual leave 后 `N_individual` 不变、`N_active += 1`；只有新的 committed lane pose
被 Adapter presentation/LOD 策略实际纳入 presented set 时，`N_presented` 才增加 1，否则
保持不变。unsafe exit 时以上计数和表现都不变。验收必须比较提交前后真实集合成员关系和
完整状态分解，不能把 park/leave 当成无条件 `N_presented -1/+1`。

成功 `despawn_vehicle` 才使 `N_individual` 减一，并在同一提交释放 parking binding 与
route reference；virtual Parked 无 pose 或 Adapter 隐藏不改变 `N_individual`，也不能
替代 removal observation。

### 4.5 规模验证

10k/100k 至少区分两种压力：

1. **大容量、稀疏占用**：证明 static/Runtime retained 与 `F+S+A+B` 相关，不与空容量
   线性相关；
2. **高实际停驻**：证明 live vehicle/binding 按 `B` 线性增长，Parked 不进入 tick 的
   lane/following/motion 热路径。

必须记录 declaration/IR/LFCA/shared-static/runtime/Adapter 各层计数，不能只报进程总内存
或帧率。理论 `u32` 极限不属于首批验证；exact/exact+1 容量边界复用领域测试，
工作负载另完成现实 10k/100k 取证。

#541 的 Runtime 稀疏证据已经证明只改变 10k/100k virtual capacity 时，shared-static
retained 与单 binding 每世界分配形状不变；它没有构造 #304 exact topology，也不替代
#542 的正式拓扑/编译计数、#544 的实际车辆运行或 #545 的 Adapter 证据。

## 5. 七套需求计划与行为验收

| Case ID              | 调用方输入与必要触发                                                         | 验收重点                                                       |
| -------------------- | ---------------------------------------------------------------------------- | -------------------------------------------------------------- |
| `MIXED-PEAK`         | 初始 75% active / 25% parked；观察窗口内计划出发方向为 70:30，固定一个主方向 | 连续跨块行程、信号/冲突约束与停车并存；报告实际准入比例和积压  |
| `GARAGE-EGRESS`      | 初始 25% active / 75% parked；固定离场队列，包含多门选择与受阻出口           | safe leave 才恢复活动与位姿；失败保留身份、绑定和计数          |
| `GARAGE-INGRESS`     | 初始 75% active / 25% parked；指定显式泊位和虚拟池的到达、预留及满位请求     | 两种容量池分别守恒，精确到达、预留、停驻与拒绝无部分提交       |
| `WAITING-RELEASE`    | 初始 75% active / 25% parked；指定进入、容量耗尽、FIFO 排队和后继相位释放    | 实际经历等待与释放，受容量及下游 storage 约束                  |
| `PERMISSIVE-LEFT`    | 初始 75% active / 25% parked；对向主流脉冲与左转请求，含可用间隙             | 实际出现 no-grant 与 grant，等值边界不放行；不能以全程停车通过 |
| `UNCONTROLLED-YIELD` | 初始 75% active / 25% parked；主路脉冲、支路/车库出口请求与脉冲后的空窗      | 主路优先、占用与清空约束，空窗后确实放行                       |
| `BOUNDARY-BURST`     | 继承混合初态；固定相邻提交边界上的信号变化、离场和 lifecycle 请求            | 命令/事件顺序、计数守恒、失败无副作用与安全拒绝后重试          |

各计划使用同一拓扑档和固定的有限输入序列。75:25 是初始生命周期构成，70:30 是
计划出发方向比例，二者不是同一口径。完成车辆由调用方明确移除/原子替换；因安全
准入而未成功的出发请求保留为待提交队列，不能静默丢弃或强行生成重叠车辆。

#544 将每个 case 的 seed、profile mix、起终点、路线选择、初态、命令排序、最大
重试次数和观察长度写成可审阅的版本化计划。具体序列与正向触发计数在正式取证前
固定，不根据一次运行结果反向调整期望；未达到必需的触发不能标为通过。所有规模
使用同一规则，报告观察窗口内的 active 分布、完成行程、等候分布及拒绝原因。

使用既有正式策略源与 `PolicyPin`。信号无保护左转使用 `PermissiveGroup`；主支路
让行使用有明确优先级及共享冲突区让行关系的 `Uncontrolled`。间隙参数引用
`traffic-runtime-right-of-way-policy.md` §7 的 `urban-conservative-v1`，由实际
16/33 ms 步长派生所需间隙，不照抄 100 ms 校准结果。它是工程参考参数，不宣称
法定数值。#543 仅供安装的空让行关系策略不能成为这些行为用例的默认策略。

结果校验复用领域合同的有限断言：安全不重叠、信号/Waiting/Conflict 权威、停车
绑定与个体守恒、稳定事件顺序，以及重复执行和失败后重试的一致性。等值间隙、
容量满位等精确断言可复用 owner 的小型测试；城市行同时必须证明相应行为被实际
触发。不枚举任意路网、任意输入或全部边界的组合，也不新建一个交通求解器当预言机。

明确延后：公交运营、乘客、出租派单、上下客与动态路侧摩擦、非机动车、行人、轨道、
并行分区和聚合/多频率行为。显式泊位只覆盖现有停车合同，不代表上述行为已实现。

## 6. 有限验证矩阵与结果包

### 6.1 工作负载本体的完成条件

| 验证行            | 必需范围                                                               | 交付责任                   |
| ----------------- | ---------------------------------------------------------------------- | -------------------------- |
| 静态制品          | 10k/100k 两档各完整生成、检查、构建、安装；每档两次 clean regeneration | #542                       |
| 无界面行为        | 七个 case × 两档规模；每行两个独立同输入运行，比较状态、事件和输入摘要 | #544                       |
| 10k Adapter 观测  | `MIXED-PEAK` 及五个停车/Waiting/Conflict case                          | #545，消费 #285 的领域观测 |
| 100k Adapter 观测 | `MIXED-PEAK`，选择可表现集合的 10%                                     | #545                       |
| 保存、恢复与切换  | `MIXED-PEAK` 加一个停车转换窗口；复用已有公开来源恢复与受支持切换路径  | #545；协议专项归 #538      |

Headless 行必须执行全部道路活动个体的相同精度求解；Adapter 减少呈现不降低
交通精度。10k 观测选择全部可表现个体，100k 混合行选择其中 10%，按稳定身份
确定集合并报告实际分母、数量和比例；virtual Parked 与 Completed 不进入该集合。
这不是把 `N_presented` 写成 `N_individual` 的固定百分比。

首轮正确性窗口由计划声明，至少覆盖一个完整暖机信号周期、两个完整观察周期和
本 case 的必需转换；信号周期按 §3.1 已生成并合法安装的整数相位时长求和。无信号时按有限
tick 窗口及转换完成条件声明。有限输入耗尽、
持续受阻或重试用尽均如实报告，不能通过无限等待使样本变成成功。`BOUNDARY-BURST`
单列触发前后边界与安全拒绝后的重试结果，不照搬普通窗口的平均值。

正常性能行使用 release binary。正式性能基线采用
`core-runtime-performance-baseline.md` §8 的暖机/观察长度、三个独立进程轮次与
分位统计方法，工具链采用受检提交的仓库固定版本；先对 `MIXED-PEAK` 记录两档
headless 与 Adapter 主行。短正确性窗口、带断言或诊断计时不能冒充该性能基线。
额外呈现比例、更多 seeds 和完整平台矩阵不属于本体关闭条件，由具体性能问题或
#539 的正式认证合同决定，不能在审阅期间自动做笛卡尔积扩展。

Snapshot restore 比较同一已提交状态恢复后继续执行的事件/状态摘要；跨修订切换
比较相同切换输入的重复运行和合同规定的迁移结果，不要求不同修订 ID 的前后摘要
相等。Runtime 在线日志/追赶与 Adapter 维护暂停式切换分别使用各自已交付入口；
不向 Bevy 增加在线调度器，也不依赖尚未交付的 Editable 存档入口。

### 6.2 可复核输出与通过状态

结果包包含：workload/version/scale/case、生成配置与工具链、source/artifact/input
摘要及来源、共享根与 policy identity、实际硬件/OS/电源角色、固定步长与观察窗口、
路线和命令序列、逐域/lifecycle/停车计数、必要行为触发、状态/事件摘要、计时及内存。
命令行和目录布局由交付切片统一实现；manifest/plan 只作为版本化验证输入，不新增
Runtime wire、发布格式或兼容层。

缺失证据必须显式区分失败、未测量、依赖未交付与不适用。当前唯一路径是
`TrafficWorld`；旧 `CoreWorld` 不参与对照，不恢复 current/target 双轨。

功能正确性必须通过，性能测量必须可复现并如实暴露超预算；产品预算与硬件角色
仍由性能基线和 #539/#305 裁决。本体交付不等待 P10 设备选型或把本地研究机结果
升级为 Product Pass，也不能掩盖不满足性能目标的结果。新增缺陷按影响组件跟踪，
不为通过工作负载而改写安全、事件、快照或切换合同。

## 7. 阶段和完成边界

稳定交付顺序为：#304 设计接受 → #542 正式拓扑 → #544 需求/运行/校验 → #545 跨层
证据。停车复用 #540/#541，Waiting/Conflict 复用 #282/#283/#284；#543 提供容量研究
输入，#285 提供复杂路口跨层观测与领域验证。子切片的实际依赖与状态从 GitHub 读取。

#304 的完成条件是三层制品、上述必需验证行和复现入口全部交付，未支持能力清楚
登记。设计 PR、10k 单档或空世界容量测量都只能完成相应子范围。

#537 拥有全目标路径成本测量，#538 拥有快照/切换协议专项，#539/#305 拥有三套
工作负载的统一认证；它们复用 #304 的输入和运行入口，避免在每个 Issue 重建生成器
或重复整套矩阵。LuST 转换与 Editable 恢复分别由其 owner 交付，不成为本体的新范围。

任一子切片完成都不能把 #304 父任务标为完成。报告必须明确区分：已满足、依赖绑定、
显式延后和 unsupported；不得以“停车 100k 容量可表达”推导“100k 城市交通已通过”。
