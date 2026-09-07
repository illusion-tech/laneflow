# LF-CN-URBAN 需求计划与无界面验证

**文档状态**: Accepted（#544 G1；#608 合入后在 Issue 记录接受）<br>
**最后更新**: 2026-09-08<br>
**适用范围**: `LF-CN-URBAN-v1` 的调用方需求、无界面运行程序、有限行为校验和结果包<br>
**关联文档**: [工作负载合同](chinese-style-city-workload.md)、
[停车](parking-system.md)、[Waiting](traffic-runtime-waiting-zone.md)、
[路权策略](traffic-runtime-right-of-way-policy.md)、
[快照](traffic-runtime-snapshot.md)、[性能基线](core-runtime-performance-baseline.md)

## 1. 交付边界

#304 已冻结七套 case、两档规模、初态比例和有限验收矩阵。本文细化 #544 的实现输入，
不重新裁决产品定位、车型分类或性能达标条件。#542 的正式 LFCA 与路线目录是共同输入；
七套 case 改变调用方计划，使用同一档路网和正式 `PolicyPin`。

实现放在非发布的 `tools/laneflow-urban-harness`：复用 `laneflow-urban-generator`
的配置与目录类型，只有一个运行库，由命令行和后续 #545 驱动调用。运行库拥有稳定编号、
需求队列、命令排序和观测结果；`TrafficWorld` 拥有全部道路行为与提交状态。
不增加 Runtime、SharedNetwork、Adapter 公共接口、工作负载特判、通用插件框架或新 wire。
计划和结果是版本化验证文件，不作 1.0 后的兼容承诺。

交付分三项分别验收，任何一项不能替代另一项：

| 交付项   | 完成条件                                                                     |
| -------- | ---------------------------------------------------------------------------- |
| 计划冻结 | 七套版本化规则与两档展开计划可复核；具体命令、目标与必要触发在正式运行前提交 |
| 正确性   | 七个 case × 两档规模，每行两个独立运行，全部必需行为通过且重复结果一致       |
| 性能测量 | `MIXED-PEAK` 两档各三个独立进程轮次，记录正式窗口、分位值与实际规模          |

G1 接受不关闭 #544。小型试跑只用于检验计划可实施性；#544 关闭需要上述三项。
正式 correctness 计划只允许 10k/100k 制品；fixture 只允许 probe，CLI 与运行库共用
该准入规则。制品载入复用生成器 `Scale` 的固定 tile 数、个体数与步长定义，校验
标签和实际 manifest 数量一致。
#545 拥有 Adapter、保存/恢复和固定增容切换证据；#539/#305 拥有产品预算认证。

## 2. 共同输入与稳定身份

计划版本使用 `urban-demand-v2`，固定 `seed=544`、单 worker、`world_id=544`。
seed 属于调用方；不改 LFCA 或 TrafficWorld 的规则。所有选择使用目录 key 的 UTF-8
字节序，禁止依赖哈希表迭代、墙钟或平台随机数。实体绑定使用目录 StableId，安装后
通过共享根得到 typed ordinal；虚拟入口/出口使用目录中的实际 anchor selector。

每 tile 固定 1000 个初始个体，编号为 `(tile, slot, incarnation)`，初始 incarnation
为 0；按此元组升序提交初态。后续请求另有递增的序号，成功原子替换 Completed 个体时
使用新 incarnation，并保存新旧编号与句柄的对应关系。离场、入场不改变个体编号；
句柄及分配槽位不是跨运行的稳定身份。#545 直接复用这个编号与映射。

每组十个 slot 的 profile 顺序固定为 `compact` 三个、`car` 六个、`van` 一个，
即 30:60:10；只有共同的 `road-vehicle` class。三种长度分别为 4000、4500、6000 mm，
外观不参与 profile、class 或需求数量。case 角色从这些 slot 选择，不另外改变车型总量。

初态与请求展开产生 `resolved-plan.toml`，至少含：来源/config/catalog 摘要、case/scale、
身份与 profile、实际边序列及 route occurrence、初始位置和停车目标、每条请求的 due tick、
目标、角色、序号、重试预算和预期观测种类。目录中出现的同一条边在不同路线上的 occurrence
分别保留，不按 LaneEdge 去重。路线按 key 升序注册，容量由实际路线长度和冲突出现项计算。

展开只用配置和已编译目录，不运行交通求解器挑选“会成功”的输入。展开文件在正式取证前
落盘并固定摘要；小试发现输入错误时修改计划版本及原因，废弃旧版本作为当前证明的资格。
禁止拿一次正式运行的结果回填 expected 或缩小必要触发数。

## 3. 初态分配

普通六个 case 每 tile 为 750 Active、250 Parked；`GARAGE-EGRESS` 为 250 Active、
750 Parked。两档分别使用 10/100 个 tile；Completed 和待提交请求初始均为 0。

Active 只从目录中长度 95000 mm 的外部 `.in` / `.out` 道路臂生成。每臂最多 11 个
候选车头位置：`7000 + 8500 * j` mm，`j=0..10`，初速为 0。全车身留在道路臂上；
即使相邻都是最长 profile，也留有 2500 mm 净距。每 tile 74 条臂共 814 个候选位置。
先为 case 角色分配有限位置，再按 `(slot 层, edge key)` 补齐背景；角色和背景共享同一
位置占用表，不能重复放置或通过跳过生成凑人数。

初态路线选择包含该臂的目录路线，优先 `cross-tile`、其次 `cross-cell`、最后
`junction`，同级取 key 最小者；记录实际 cursor 和剩余路段。不能把已经位于末段的
车辆统计为完成跨块行程。所有候选最终都经 `spawn_vehicle`；任一拒绝使初态校验失败，
不能把缺少的 Active 转成 Parked 或待提交请求。

Parked 分配固定到逐 tile 的合法目标，表中数字是实际 Occupied 数：

| 初态                            |                    显式泊位 | 十个 mixed 虚拟池                 | virtual-only 车库 | 合计 |
| ------------------------------- | --------------------------: | --------------------------------- | ----------------: | ---: |
| 普通六 case                     | 每 cell 的 bay0 各 1，共 10 | 每池 12，共 120                   |               120 |  250 |
| `GARAGE-EGRESS`                 |                 同上，共 10 | 每池 24，共 240                   |               500 |  750 |
| `GARAGE-INGRESS` 覆盖上述普通行 |                 同上，共 10 | c09 为 100；其余九池各 12，共 208 |                32 |  250 |

bay1 保留给实际到达；Ingress 的 c09 mixed 虚拟池从初态就真实满位。车库每 tile 有
1000 个虚拟位置，mixed 每池 100 个；不同设施和显式/虚拟池的空位不能互相借用。
所有 Parked 通过 `spawn_parked_vehicle` 建立 Occupied binding；retained cursor
使用该目标目录首个 departure anchor，它不表示道路 arrival。

初始化后读取每个个体状态和 binding，以及每个 facility 的分池计数，核对上述精确分配。
两档 Egress 必须实际得到 7500/75000 个 Parked binding。容量总数、预留内存和未提交
需求均不能代替这些绑定。

## 4. 有限需求与七套 case

### 4.1 背景需求和命令顺序

背景车完成短跨块行程后，由调用方提交下一次出发请求，使用 `replace_completed_vehicle`。
每组十个初始 Active slot 为一个出发组，周期为 64 个 528 ms 量子；组的偏移为
`(group_index + seed) mod 64` 个量子。同组请求同 tick 到期，编号升序处理。
只展开本次运行终点之前的有限周期，不运行无限需求循环。

`MIXED-PEAK` 每组十条计划出发使用七条向东、三条向西路线，依次取本 tile 目录的
`.cross.e` / `.cross.w` 路线并按 key 循环。起点使用首边 7000 mm、初速 0；终点为
目录路线的末端。其他 case 保持相同的分组、周期、profile 和方向比例。三类
Waiting/Conflict 行及 Burst 的背景路线从同一有序目录中排除“剩余后缀会穿过角色
cell”的候选，避免有限角色脉冲被持续背景流替代；这只是调用方输入选择，不改变
Runtime 规则。其余跨 tile 流量仍由真实跨 tile 路线提供，不创建独立世界，也不因
tile 边界切断 Runtime 更新。

Mixed 另在第一个观察周期提交每 tile 20 条车库离场请求，同样按七东三西的十条组编排。
入场从既有 Active 个体选择并预留前向可达目标；不额外生成车辆。70:30 的分母是观察
窗口内到期的计划出发请求，重试不重复计入；真实准入数及其方向比单独报告。

Mixed 的到达角色在初态中预先占用目标入口臂上的合法候选位置，前向有限空间不再
分配背景个体，其余 Active 按共同候选表补齐。首个提交边界 reserve；实际到达可以
发生在暖机期，但保持 Active/Reserved 等待预先冻结的观察期 park 边界。park 仍要求
真实 arrival，且最早在 arrival 后的下一个提交边界；只有观察期实际 park/leave 计作
该行停车转换。暖机 reserve/arrival 不计为观察期入场链证据。

普通可重试请求最多尝试 8 次，间隔 4 个量子；超出窗口不追加尝试。未 Completed、
无可安全准入的位置、离场受阻均保留原个体和请求。用尽预算的请求保留为 exhausted
积压记录，不继续调度、不改 due tick、不清账冒充成功。后续到期请求也不能覆盖旧记录。

每个提交边界依次处理：到期的 park/cancel、离场、Completed 原子替换、reserve，
最后调用一次 `step`。同类按 `(due_tick, tile, stable_id, request_sequence, attempt)`
排序。Step 返回的 arrivals 和领域事件在本边界记录，派生的 park 最早在下个边界提交。
`BOUNDARY-BURST` 的显式 despawn/spawn 作为独立命令组放在原子替换位置，不将两条命令
包装成 Runtime 原子事务。背景替换请求若命中角色拥有的 slot，由 caller 在读取 live
handle 前记录为 `role-held` 并按共同有限预算重试；因此相邻 despawn/spawn 边界之间的
暂时 absent 不会向 Runtime 下发命令，也不改变角色所有权。每次命令都保留结果、提交
游标和映射变化。

### 4.2 场景角色与必要观测

每 tile 使用现有对应模板安排下表角色。展开计划必须给出精确个体、路线、锚点和 due tick；
角色不是 Runtime 标签。信号相位的候选边界由已编译目录的 phase duration、cycle、offset
计算，时间位置为 `(time_ms + offset_ms) mod cycle_ms`；实际通过与拒绝仍由 Runtime 判定。
脉冲和释放安排在观察窗口内，不能只在暖机发生一次便记为观察通过。

| Case                 | 固定角色和输入                                                                                     | 每 tile 必须出现的有限观测                                                                          |
| -------------------- | -------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------- |
| `MIXED-PEAK`         | 背景跨块行程、20 条车库离场，以及 bay1 和未满虚拟池各一个到达角色                                  | 至少一个真实跨 tile 完成行程、受信号约束的等待及随后通行、一次停车转换；统计方向、准入、完成和积压  |
| `GARAGE-EGRESS`      | 车库 20 个已停个体，交替使用两个已编译出口；其中一个出口安排有限占用脉冲                           | 两个出口各至少一次成功离场；至少一次安全拒绝及同一请求的成功重试，身份与原绑定在拒绝后保留          |
| `GARAGE-INGRESS`     | bay1 到达、未满 mixed 虚拟池到达、向 bay0 和已满 c09 池的预留请求                                  | 显式和虚拟各一次实际 reserve→arrival→park，以及各一次真实满位/排他拒绝                              |
| `WAITING-RELEASE`    | c00 的同一待转方向三个 FIFO 个体，配合有限下游占用脉冲                                             | entry、容量拒绝、下游 storage 拒绝、后继相位 release 各至少一次；入队者按权威队列顺序释放           |
| `PERMISSIVE-LEFT`    | c01 左转角色和同绿对向直行脉冲，脉冲结束后保留可用间隙                                             | 与对向流相关的 no-grant 和后续 grant/通过各至少一次                                                 |
| `UNCONTROLLED-YIELD` | c04 主路脉冲、支路转入角色及相邻车库出口请求，主路脉冲有限终止                                     | 主路通过、支路因真实让行关系等待、空窗后的支路通过各至少一次                                        |
| `BOUNDARY-BURST`     | 在已选相位边界的前/后相邻提交边界编排 leave、reserve、park、Completed replace 和显式 despawn/spawn | 实际相位变化、至少一次 lifecycle 成功、一次安全拒绝及有限重试成功；分别记录两个边界，不用平均值代替 |

每个城市行按 tile 给出 required/observed 表，只有对应实例的实际观测可以满足该行；
不能由另一个 tile 的成功或 owner 小型测试补足。角色准入失败、脉冲未清空、窗口结束时
必要事件缺失都判失败，不延长窗口直到碰巧通过。背景个体仍逐 tick 执行同精度道路求解。

Mixed 的“一次停车转换”是至少一次成功 park 或 leave，三种计数仍分别报告；不额外
要求该行同时成功离场、显式入场和虚拟入场。观察期内完整 reserve→arrival→park 与
两类拒绝由 `GARAGE-INGRESS` 对应行证明，不能用 Mixed 的分阶段角色代替。

#542 的待转区容量为 1，城市行固定三个候选并记录其 admission 顺序；在有限两周期
窗口内，实际 release 序列必须是 admission 序列的有序前缀，且至少有一次 release。
不声称验证多成员同时在区内的排列；完整多成员 FIFO 精确断言复用 owner 测试。本体
不为增加这类组合而改写共同 LFCA。

安装必须读取 `urban-conservative-v1` 的实际 policy pin 和派生间隙：16 ms 的 lead/lag
为 5516/2500 ms，33 ms 为 5533/2500 ms。不能使用 #543 的空让行关系策略。
等值间隙不放行等精确边界复用领域 owner 小型测试；城市行证明真实让行和放行，不另行
制造任意几何/时刻/车型的笛卡尔积。

## 5. 观测与校验边界

运行程序只读取公共已提交状态、`StepOutcome`、Parking 命令结果、Waiting/Conflict
决策、transition events 和快照。校验以下有限不变量：

1. 每次提交后，`live = Active + Parked + Completed`；初始 live 加成功出生减成功移除
   等于当前 live，原子替换同时计一次移除和一次出生。稳定编号、句柄映射一一对应。
2. Active 车身在实际 route occurrence 上投影到一维物理边区间，同边区间不得重叠；
   这是几何区间核对，不实现跟车、信号、Waiting 或冲突求解器。
3. 显式停车排他，虚拟池 Reserved + Occupied 不超容量，Parked 与 Occupied 对应；
   按静态设施归属汇总显式/虚拟绑定，与设施分池 reserved/occupied 和 total 对账；
   Waiting membership/容量/FIFO 和 grant/no-grant 的实际后果符合所选角色合同。
   按已提交 Conflict reservation 的资源身份核对互斥 claim，不另算间隙或决定 grant。
4. 每种必需拒绝选一个代表请求，比较拒绝前后完整 deterministic state digest；
   其余重试核对该个体、绑定、分池计数和提交游标。不对每个拒绝重复捕获整个城市快照。
5. 每个成功 tick 按稳定顺序摘要车辆状态、命令结果、决策和事件；在初态、暖机末及每个
   完整观察周期末，另用 `capture_snapshot` / `deterministic_state_digest` 核对完整状态。
   不把 Debug 文本、内存地址、计时或运行目录写进语义摘要。

命令成功引发的 park/leave/replace 生命周期变化在命令边界记录前后状态、稳定身份及
命令序号；step 引发的变化在 step 提交时记录，以 phase 区分。命令后仍独立采集
`step_before`，供意图计数、红灯等待和路线跨越观测使用，不能将它移到命令前。

两个独立运行从同一 LFCA 和同一展开计划各自创建新世界，不用首轮快照启动第二轮。
比较输入摘要、逐 tick 状态/事件序列与完整状态检查点；首个差异保留 tick、稳定个体和
命令信息，运行失败返回非零。完整快照检查点不能代替逐 tick 摘要，反之亦然。
运行程序将非语义执行编号写入诊断记录，比较时要求编号存在且不同，以拦截误复制
同一次执行的目录。编号不进入确定性摘要，也不构成独立执行或防伪证明；真实独立运行
由取证流程保证。缺少编号的旧记录需重新取证，不补号转换。

逐域记录唯一已运行的 `road_motor_vehicle`：`N_individual=live`，`N_active` 为提交后
Active 实数，headless 的 `N_presented=0`，两个 aggregate 计数均为 0。Completed 仍属于
live；未来尚未到期和已到期 pending/exhausted 请求分别计数，均不进入 N_individual。

`N_intent` 对成功 tick 使用命令执行完、step 开始前的 Active 实数，并记录依据
`exact_active_before_step`：现有完整路径对每个 Active 调用运动控制更新，即使被阻停也
已计算控制意图。它不等于容量、初始人数或 step 后的 Active 数。step 失败时不声称所有
个体已经更新，记录 actual intent 为未测量，并保留失败；将来 Runtime 改成多频率路径后
本推导失效，需要重新审定观测口径。

## 6. 运行长度与结果包

令 C 为输入中最长完整信号周期的 tick 数。当前目录最长周期 122496 ms，因此
10k 的 C=7656，100k 的 C=3712。所有 phase/offset/需求量子必须能被实际 fixed step
整除，输入变更导致不能整除时拒绝，不能由 harness 舍入相位。

| 运行             | 暖机         | 观察          |              独立次数 |
| ---------------- | ------------ | ------------- | --------------------: |
| 七套正确性行     | C            | 2C            |    每 case/scale 两次 |
| Mixed 正常性能行 | max(512, 4C) | max(4096, 8C) | 每 scale 三个独立进程 |

对应正确性总长为 22968/11136 ticks，性能每轮总长为 91872/44544 ticks。
Burst 仍保留共同有限窗口，但重点另列两个提交边界的 raw 结果和安全重试；不从两个
样本伪造 percentile。正确性只做一个 seed、两档、当前固定工具链，不追加平台组合。

实现提供 plan、run、compare 三种操作；CLI 与 #545 使用同一运行库。输出采用新目录，
输入、命令日志与语义证据固定后，计时、环境和错误单独保存。结果包以完成初始化
（世界安装、路线注册、初态校验和初始检查点）为起点。初始化失败由库返回错误，
CLI 输出错误并非零退出；本段接受部分准备文件，不提供结构化初始化失败记录或
半成品世界状态。完成初始化后的运行及受控执行/校验失败，其结果包最少包含：

- `resolved-plan.toml`：上述实际输入和预期触发；其摘要与来源制品四联进入结果。
- `commands`、`events`、`ticks`：实际顺序、结果和逐域计数，及用于重复比较的摘要。
- `result.json`：工作负载/计划/结果版本、case/scale、LFCA/config/catalog
  摘要、world/policy identity、实际窗口、逐 tile 触发、检查点及本次运行结论。
- `comparison.json`：比较结论、case/scale、计划摘要、完成 tick 数、两个执行编号和
  两份 result 的 SHA256/字节数。CLI 必须指定新报告路径，成功比较才写入；两份原始
  result 保持不可变。当前载荷为 `urban-result-v3` / `urban-comparison-v1`，不转换旧记录。
- 正式性能阶段的 `measurements.toml`：git commit、`rustc -Vv`、`cargo -V`、target、构建参数、硬件/OS/电源角色、
  命令行、phase 耗时、计时范围、实际 Active/intent 分布、内存值及测量方法。

首段 Mixed 实现的具体角色、命令与摘要口径见
[运行程序说明](../../tools/laneflow-urban-harness/README.md)。短试跑和正确性运行的
`diagnostics.json` 提供执行编号、环境与诊断计时，不替代正式性能结果；其他 case 与
100k 仍需各自取证。

性能使用 CI 固定 Rust 版本的正常 release binary；正确性摘要、完整快照捕获和诊断
断言耗时不混入正常 tick latency。分别记录 TrafficWorld step、调用方命令与观测开销。
按性能合同合并三轮 p50/p95/p99 和最坏 max，不能把两个正确性运行当性能轮次；工具链
变化须重跑用于当前判断的三个轮次。未测内存项写明未测量，不填 0。

每项证据显式记录通过、失败、未测量、依赖未交付或不适用，只有通过项参与整体通过。
性能超预算如实报告，不改变正确性预期；实际 Active 衰减或积压必须可见，不能继续用
初始 75% Active 给测量行命名。#544 的交付不包含新性能门禁、产品达标声明或 owner
组件的大范围优化；真实缺陷按组件形成可复现的问题，再决定相应修复范围。
