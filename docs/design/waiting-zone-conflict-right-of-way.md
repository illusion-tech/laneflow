# WaitingZone、ConflictZone 与通行权分层

**文档状态**: Accepted（#235 联合架构与 #282 WaitingZone G1，2026-09-01）<br>
**适用范围**: WaitingZone 本地准入、Conflict 路线出现项、车辆级通行权、下游净空、
Parking 生命周期、持久化与 Runtime/Spatial/Adapter 边界<br>
**交付边界**: WaitingZone 本地动态 authority 属于 #282；§6 的联合算法已由 #235 接受，
#284 实施细化另处 Review，downstream-clearance、Conflict 仲裁与组合 ledger 尚未交付<br>

**关联文档**:

- `traffic-runtime-waiting-zone.md`
- `traffic-runtime-conflict-occurrence.md`
- [`traffic-runtime-right-of-way-policy.md`](traffic-runtime-right-of-way-policy.md)（#284 实施细化，Review）
- `traffic-runtime-shared-consumption.md`
- `traffic-runtime-integer-geometry.md`
- `traffic-runtime-snapshot.md`
- `traffic-runtime-revision-cutover.md`
- `parking-system.md`
- `signal-system.md`
- `vehicle-following.md`
- `../adr/0019-waiting-zone-conflict-right-of-way-authority.md`
- GitHub: #235、#281、#282、#283、#284、#541、#559

## 1. 当前基线与用途

本文描述 Waiting、Conflict 和 downstream clearance 在同一车辆 traversal 中如何组合，
并给各实施切片划定唯一 ownership。#282 的字段、固定步进、持久化和验收细节见
`traffic-runtime-waiting-zone.md`；本文不另造一套实现合同。

当前生产基线为：

- LFCA `formatVersion = 5`，受检 LFCA 构造唯一 `SharedNetworkRevision`；
- `TrafficWorld` 是唯一运行世界；
- 已提交纵向 authority 是整数毫米、`mm/s` 与 `carry_um`；
- WaitingZone identity、entry/release Gate、`maxOccupancy` 和 route
  `WaitingOccurrence` 已存在；
- `ParkingBinding`、reserve/park/leave/rebind/despawn 生命周期已存在；
- `ConflictPassageOccurrence`、`route_conflict_occurrence_capacity`、路线 conflict
  Gate ranges，以及生命周期/restore/cutover 的 3A 保护已存在；
- 当前持久化轴是 LFRS 4、runtime state 4、deterministic digest 6。

#282 只新增 WaitingZone 本地动态能力。#284 才新增正式 Conflict/right-of-way 与组合
资源能力。

## 2. Authority 地图

| 对象或职责                                                    | 唯一 owner                     | 本阶段合同                                  |
| ------------------------------------------------------------- | ------------------------------ | ------------------------------------------- |
| WaitingZone 静态 identity、entry/release Gate、`maxOccupancy` | LFCA / `SharedNetworkRevision` | Runtime 只消费受检 ordinal 和局部整数距离   |
| Waiting membership、occupancy、counter、queue、phase          | `TrafficWorld` / #282          | 本地动态 authority                          |
| zone-local `WaitingAdmissionClaim`                            | `TrafficWorld` / #282          | 只证明一个 WaitingZone 的 admission/storage |
| `ConflictPassageOccurrence` 与 conflict Gate ranges           | 路线编译器 / #559              | route-local 派生热表                        |
| `ConflictRuntimeUnavailable` 3A 保护                          | `TrafficWorld` / #559          | #284 前不可绕过                             |
| downstream-clearance claim                                    | `TrafficWorld` / #284          | 通用物理下游资源                            |
| `ConflictArbiter`、grant、reservation                         | `TrafficWorld` / #284          | 车辆级冲突 authority                        |
| Waiting/Conflict/downstream 组合 ledger                       | `TrafficWorld` / #284          | 原子取得、未提交选择与 cycle prevention     |
| signal indication                                             | Signal Runtime                 | 许可输入，不是最终车辆 grant                |
| canonical 3D 几何与 pairing validation                        | Spatial                        | 不反推行为或修改 claim                      |
| 只读状态与事件消费                                            | Adapter / Scenario / caller    | 不获得 mutation authority                   |

## 3. 静态区间与路线出现项

### 3.1 WaitingZone

WaitingZone 是同一 ManeuverPath 上 entry/release Gate 之间的有界区域。相邻 zone 可以
共享 release/entry boundary；内部重叠或嵌套由静态验证拒绝。

路线注册按 route order 编译：

```text
WaitingOccurrence
  zone
  maneuverOccurrenceIndex
  entryHop
  releaseHop
  storageLengthMm
```

`storageLengthMm` 是 zone-local 有界整数毫米距离，不使用可能因更早 route prefix
溢出而错误拒绝合法局部区间的全路线 `u32` 前缀。路线注册保持
profile-agnostic；实际车型能否放入空 zone 在 Active 候选绑定时验证。

### 3.2 Conflict passage

#559 已按稳定 stream/passage identity 编译：

```text
ConflictPassageOccurrence
  conflictZone
  participantStream
  maneuverOccurrenceIndex
  conflictGateRange
  entry/clearance route-local anchors
```

具体字段以 `traffic-runtime-conflict-occurrence.md` 为准。出现项数量由
`route_conflict_occurrence_capacity` 独立约束，不能借用 Waiting 数量、route edge
数量或 `vehicle_capacity` 猜测。

### 3.3 绑定时能力检查

`spawn_vehicle`、`replace_completed_vehicle` 与 `leave_parking` 产生的无既有 Waiting
authority 的 Active 候选必须依次通过：

1. 现有 Access、route/cursor/progress/speed 与 Parking 合法性；
2. future Waiting occurrence 的 empty-zone 车型可行性与 stateful interior guard；
3. #559 对全部 route conflict occurrence 的整车身 3A 能力保护；
4. existing occupancy/no-overlap 等提交前验证。

任一步失败都保持旧 world、旧 lifecycle 状态和 command cursor。Waiting 可行不代表
Conflict Runtime 可用；`ConflictRuntimeUnavailable` 仍须按 #559 返回。

`rebind_parking_route` 先执行 #541 的 current occurrence 与完整 physical footprint
等价检查。已有 traversal/membership 时，目标 route 必须按稳定 ManeuverPath、Gate 与
WaitingZone identity 精确映射当前 phase、crossing side、membership、admission sequence
与 release Gate；queue/occupancy/counter 原样保留。没有既有 authority 时才执行上述
interior guard。任一映射失败都不改变旧 route、Parking 或 Waiting 状态。

`reserve_parking` 与 `rebind_parking_route` 还必须对实际选择的 Parking entry 执行
route-specific Waiting 检查。按精确 route occurrence/hop 和 segmented route anchor，
entry 落在任一 Waiting occurrence 的
`[waitingEntryBoundary, waitingReleaseBoundary]` 时拒绝；entry 正好在 Waiting entry
boundary 时虽没有 membership，仍有 `PreGate` traversal。绑定还必须证明 exact parking
arrival 提交后没有 traversal state，覆盖 entry 上游但仍处于 stateful maneuver 的锚点。
该规则同样用于 restore 与 same/cross-revision cutover，不让 route registration 全局
禁止未被选择的停车 entry。

## 4. 车辆 traversal 与正交状态

#282 的 committed traversal phase 为：

```text
PreGate -> Committed -> Waiting -> Committed -> ... -> completed traversal
```

- `PreGate`：尚未跨过当前 stateful entry Gate，无 Waiting membership；
- `Committed`：已经跨过 entry，可能仍在 zone 内运动并持有 membership；
- `Waiting`：member 已到 release Gate，且 release Gate 是最终最严格约束的归因；
- release 重新允许但车辆尚未 crossing 时，回到 `Committed`，membership 不变；
- successful release crossing 才移除 membership；
- shared release/next-entry boundary 在一个事务内先 leave、后 enter。

#282 不预置空的 `Clearing` 变体。Conflict reservation 和 tail-clear phase 必须由 #284
与正式状态、快照和恢复验证同时引入。

Waiting membership 与 `ParkingBinding` 正交：

- `Active + Reserved` 可继续拥有 Waiting membership；
- Parking 预约不改变 Waiting queue；
- 合法 Parking arrival 后不得仍有 traversal state，且 entry 不得落入相关 Waiting
  occurrence 的 `[entry, release]`；非法 reserve/rebind 原子失败，不通过停车自动清除
  traversal 或 membership；
- `park_vehicle` 前 traversal/membership 必须已清空；
- Parked 与 Completed 不得持有 Waiting membership；
- 离场创建的新 Active 候选从无 membership 开始；
- completion、replace、despawn、route rebind/remove 都不得留下悬空 link 或 occupancy。

## 5. #282 的本地 admission 与物理存储

### 5.1 每 tick 的有限 horizon

每辆车每 tick 只为 route order 中最早一个尚未持有的 Waiting occurrence 请求新
`WaitingAdmissionClaim`。即使 unconstrained motion 能在同 tick 到达第二个 Waiting
entry，也只能到该 boundary，下一 tick 再申请。

车辆 tick-start 已持有旧 membership 时，shared release/next-entry 上的 next entry 是
本 tick 唯一可请求的新 claim，并与旧 membership release 原子替换。

该规则解决的是 #282 的现实实施边界：本地 reducer 不产生半成功的多 zone claim。
它不是网络级 deadlock 算法；跨 zone 原子取得、未提交 bundle 选择和 prospective cycle
prevention 归 #284。

### 5.2 本地容量与存储

同一 zone reducer 只观察：

```text
tickStartOccupancy + earlierStagedAdmissionCount < maxOccupancy
```

同 tick staged release 不返还 capacity。claim 还必须证明实际车身能放入 zone，满足
队尾车辆作为 leader 时的 minimum gap，并与 committed/staged member no-overlap。

`maxOccupancy` 不等于固定 slot 数。同样两辆车在不同长度和 gap 下可能得到不同结果；
这是现实车辆组合，不是需要通过理论 worst-case 常驻预算解决的问题。

取得 claim 只移除对应 Waiting entry stop，不能越过 leader、signal、RouteEnd、
ParkingStop 或任何更严格 hard boundary。未 crossing 的 claim tick 末失效，不消费
admission sequence。

## 6. #284 的组合仲裁

本节以当前权威保存 #235 已接受、由 #284 生产化的 right-of-way/policy/arbiter 语义；
ownership 分离不重新打开这些算法选择，也不声称 #284 已实现。#282 只实现 §5，不能
因此删除本节或用简化 ledger 取代它。

### 6.1 authority、当前输入与交付边界

最终 vehicle-specific right-of-way authority 是 `TrafficWorld` 私有
`ConflictArbiter`：

- policy 只把已提交 signal/regulatory snapshot 解释为
  `DenyAndStop | Candidate(Protected | Permissive | Uncontrolled)`；
- `ConflictArbiter` 才结合 Waiting state、`ConflictPassageOccurrence`、approach
  frontier、gap、Conflict occupancy/reservation 与 downstream storage 决定 grant；
- candidate producer 只读 tick-start snapshot；唯一 single-writer reducer 拥有
  tick-local claim ledger mutation；
- Adapter、Scenario、SignalController、Spatial 与 caller 只能读取 committed decision、
  state 和 event，不能注入 winner、grant 或 reservation。

当前静态根仍是受检 LFCA / `SharedNetworkRevision`，route conflict operands 来自 #559
已经交付的 `ConflictPassageOccurrence`、Gate ranges 与
`route_conflict_occurrence_capacity`。#284 的 regulation/policy source 必须从届时 official
source 经唯一 compiler/LFCA/shared-root 路径原子引入；不得恢复 current JSON 或让
`TrafficWorld` 解析文件。具体来源、世界绑定与候选版本组合见
[`traffic-runtime-right-of-way-policy.md`](traffic-runtime-right-of-way-policy.md) §2–§8；
其 Review 状态不改变上述当前版本。实施时与正式 Runtime state/digest 同切片闭合。

#284 消费 #282 已提交的 Waiting membership、occupancy、counter 与 local admission
outcome，但 mutation owner 仍留在各 zone reducer。#284 可以把 zone-local claim 纳入
组合事务，不能重新定义 Waiting queue、admission sequence 或释放语义。

### 6.2 policy normalization

目标语义 shape 保持：

```text
RegulationIdentity
  jurisdiction
  version
  source?

RightOfWayPolicySet
  regulation
  evidenceRefs[]
  gapProfiles[]
  streamRules[]

StreamRightOfWayRule
  participantStream
  participantClasses[]? // omitted = fallback
  priority              // higher first
  yieldToStreams[]
  gapProfile?

GapAcceptanceProfile
  minimumLeadGapMs
  minimumLagGapMs
  clearanceBufferMs
```

normalization 必须在 world/policy 安装前完成并失败原子：

- Scenario/world 显式 pin policy identity；业务时间、历法与规则启用时机由宿主产品
  决定，覆盖游戏、数字孪生、仿真等场景。核心不按真实公历或宿主墙钟筛选规则，
  不按“最新”猜测，也不在 tick 热切换；
- 重复或 dangling evidence/rule/profile 引用拒绝；provenance 用于追溯，
  不触发网络抓取或按日期判定策略有效性；
- participant class specificity 使用已受检 class hierarchy；显式 class 规则优先于
  fallback，同 specificity/priority 的冲突拒绝，不在 tick 做字符串或祖先遍历；
- self-yield、duplicate yield edge、严格 priority cycle、未共享 passage cell 的 yield
  target 拒绝；同 priority all-way-stop 使用稳定 arrival ticket，不伪造 yield cycle；
- 对每个受静态 Access 允许、可能形成 active Conflict passage 的 `(stream, profile)`，
  必须恰好解析一个 `ResolvedStreamRule`；missing/ambiguous 不默认 allow、deny 或 priority；
- yield set 非空时 gap profile 必填，空时必须省略；target stream 的所有可进入 profile
  effective priority 必须严格高于 subject，否则需要显式 target-class 设计并返回 G1；
- 每个 subject static passage cell 按自身 `ConflictPassageAddress` 预编译 exact
  `yieldTargetCellRange`；target 不参加该 cell 所属 zone 时不合成 missing cell；
- target ranges 按 stable canonical rank 编译，wire declaration order 只参与
  normalization first-error；tick 不做 zone × stream、route × policy 笛卡尔查询；
- 一个 Gate coverage 包含多个 subject streams 时逐 cell 解析 rule，candidate policy
  priority 取 distinct resolved rules 的最小值；yield/gap 仍按各 cell 的 exact range
  求值，不能任选第一条、最后一条或最高 priority；
- pure Waiting request 没有 Conflict policy row，priority 必须用显式 absent rank 表达，
  不能造一个默认数值 priority。

`Protected` 只跳过 yield/gap，不跳过 rule resolution、Conflict occupancy/reservation、
downstream clearance 或 motion safety；`Permissive` 与 `Uncontrolled` 都执行 policy
yield/gap。任一 signal/Access/regulatory deny 都得到 `DenyAndStop`，任何 policy allow
不能覆盖另一个约束平面的 deny。

policy 安装还必须对全部 steady signal phase 做 protected coherence：两个 Gate coverage
共享 incompatible ConflictZone，且某对可进入 profile 在同一 phase 都会得到
`Candidate(Protected)` 时，安装失败；不能把错误 authoring 静默降级为运行时排队。

#### 6.2.1 中国机动车红灯右转的最小策略

#284 首版包含中国机动车红灯条件右转。法规依据是
[《中华人民共和国道路交通安全法实施条例》2017 年修订本](https://xzfg.moj.gov.cn/law/download?LawID=1614&type=pdf)
第 38、41、51、53 条；国家行政法规库将该版本列为现行有效，本次核验日期为
2026-09-02。圆形灯、方向指示灯及禁止右转标志的区别同时参照
[盐城市公安局 2025-11-26 公开说明](https://www.yancheng.gov.cn/art/2025/11/26/art_34214_4383922.html)。
本节将已核实规则映射为运行时合同，不把地方说明作为额外的全国性法规。

- 普通圆形红灯、右转且没有适用的禁止右转约束时，在不妨碍被放行车辆、行人的
  条件下可以通行。中国策略将其解析为 `Candidate(Permissive)`，仍须取得最终 grant。
- 适用于该右转方向的箭头红灯，或适用的“禁止右转”“红灯时禁止右转”约束，解析为
  `DenyAndStop`。不能因为车辆是右转就绕过该限制，也不能把其他方向的箭头灯误用到右转。
- 同车道前车正在等候放行时，右转车辆依次等候；没有方向指示灯时还须表达转弯让直行、
  相对方向右转让左转。前方路口交通阻塞时不得进入。这些分别由编译的让行关系、
  Waiting/leader 顺序和 mandatory downstream-clearance 共同落实。
- source 必须明确表示适用的灯态解释、机动方向和禁令条件，经唯一 compiler/LFCA/shared-root
  路径规范化。`signalControl:none` 不是“没有右转专用灯”的别名：普通圆形灯仍有真实
  信号组绑定。缺失或含糊的输入不能当成圆形灯，也不能推断不存在禁令。
- `SignalAspect::Red` 本身不预先生成不可撤销的 deny；先由已 pin 的地区策略解释灯态，
  再组合真实的 signal/Access/regulatory deny。已解析的 deny 仍不能被其他 allow 覆盖。
  不在 SignalController 或 Adapter 内硬编码中国右转特例。

全国规则、地区/标志限制与工程间隙参数分别保留来源和版本。法规中的行人让行义务不因
当前 Runtime 只实现机动车而消失；#284 的可运行参考场景和支持声明限定为已建模的
机动车冲突，不能将未实现的行人/非机动车交通当作已经验证安全。更广泛的中国复杂道路
法规矩阵和跨层场景由 #238、#285 承接。

#### 6.2.2 首版参考间隙参数

#284 提供一套由项目维护、具名且版本化的保守参考 `GapAcceptanceProfile`，调用方必须
显式选入 policy；缺失必需参数仍拒绝，不新增隐式 fallback。具体数值通过短长车型、
让行汇入、无保护转向、饱和主路和下游堵塞的受控场景确定，并记录适用范围、通行量、
等待分布与拒绝归因。这些数值是工程参考，不冒充法规规定或专业交通工程校准结果；
参数调整不得放宽本设计的安全不变量。

### 6.3 Gate evaluation frontier 与稳定 candidate

next Gate 已进入本 tick lookahead 的车辆都进入 Gate evaluation frontier；不能因 Waiting
已满、gap/downstream 不足或 request 构造失败而从 latest decision 消失：

- regulatory deny：`NotEvaluated`，保留 GateStop；
- candidate 的 coverage 没有 Waiting/Conflict/downstream resource：`NotRequired`；
- finite authoritative input 下 Waiting、gap、Conflict 或 downstream 不满足：
  `NoGrant(reason)` normal outcome；
- metadata/handle/状态不变量破坏：step error，整个 tick 不提交；
- 只有完整 request 与稳定 `firstEligibleTick` 的车辆进入 arbitration candidate set。

candidate 规范排序键为：

```text
(
  protectedRank ASC,                 // Protected = 0
  policyPriorityPresentRank ASC,     // conflict-backed = 0, pure Waiting = 1
  candidateRightOfWayPriority DESC,  // present 时为 coverage minimum
  firstEligibleTick ASC,
  waitingTicketPresentRank ASC,      // existing member = 0
  waitingAdmissionSequenceOrZero ASC,
  vehicleUpdateSequence ASC
)
```

任何需要新 `WaitingAdmissionClaim` 的 request 都必须先取得 zone-local
`WaitingAdmissionEntitlement`。它是 tick-local、不可提交、不可由 Adapter 构造的
eligibility token，不是第二套 Waiting authority：

1. 按 WaitingZone 分组，从同一 tick-start snapshot 冻结 §7.1 的
   `(approachDistanceMm, vehicleUpdateSequence, entryHop)`；
2. 在 staged release 不返还 capacity 的前提下，按该顺序预演本地 count/storage，只有
   物理队首到可用名额范围内的 request 取得 entitlement；
3. entitlement 进入 candidate bundle，但 occupancy、queue、counter 和最终 claim 仍只由
   `TrafficWorld` 的 Waiting reducer 在组合事务成功时提交；
4. §6.3 的法规/冲突全局键只排列已经通过本地 entitlement 的 bundle，不能让较高
   `candidateRightOfWayPriority`、较早 ticket 或 existing-member rank 把同 zone 物理后车
   提到前车之前。

若物理前车随后因 Conflict/gap/downstream no-grant 或更严格 motion constraint 未 crossing，
同 tick 不把 entitlement 转授给其物理后车：后车不能越过前车，转授只会恢复活锁。
不同 WaitingZone 的 `approachDistanceMm` 没有可比较的全局坐标，因此不得把它直接插入
§6.3 全局 comparator。

presence rank 不能用合法整数 sentinel 冒充 absent。`firstEligibleTick` 在车辆首次满足
同一 Gate occurrence arrival predicate 时建立；离开 lookahead、换 route/occurrence 或
crossing 后清除。raw vehicle/route/entity handle、worker、锁竞争、HashMap iteration 与
proposal 完成顺序都不得参与业务排序。

### 6.4 directed lower-bound ETA 与 top-two frontier

v1 保留性能优先、保守确定性的 current/upcoming/repeated-passage frontier，不引入概率
driver model、长时域 trajectory search 或 per-Junction solver dispatch。

policy 安装对每条 yield rule checked 计算：

```text
requiredLeadMs = fixedDeltaTimeMs + minimumLeadGapMs + clearanceBufferMs
maxRequiredLeadMs = max(all requiredLeadMs)
frontierProofHorizonMs = maxRequiredLeadMs + 1
```

所有加法必须在 portable integer-ms 上 checked；超界使安装失败。route compiler 复用
#559 occurrence，并构造从任意 cursor 开始有界访问 current/upcoming/repeated passage 的
forward range/index；不能为每个 cursor 复制 remainder list，也不能只看当前 maneuver。

Committed longitudinal authority 仍是 integer `progress_mm / speed_mm_s / carry_um`。对
cursor 到 passage entry 的 segmented route distance，先得到 exact integer millimetres，
再把 `carry_um` 作为 1/1000 mm 的已前进余数扣除，形成非负 directed lower enclosure：

```text
dLowerMm = lower_sub(
  exactF64(segmentedDistanceMm),
  upper_div(exactF64(carry_um), 1000.0))
vUpperMmPerSecond = exactF64(speed_mm_s)
aUpperMmPerSecondSquared =
  upper_mul(exactF64(maxAccelerationMetersPerSecondSquared_f32), 1000.0)

if dLowerMm == 0:
  etaLowerSeconds = 0
else if aUpperMmPerSecondSquared > 0:
  etaLowerSeconds = lower_div(
    lower_mul(2, dLowerMm),
    upper_add(
      upper_sqrt(upper_add(
        upper_mul(vUpperMmPerSecond, vUpperMmPerSecond),
        upper_mul(upper_mul(2, aUpperMmPerSecondSquared), dLowerMm))),
      vUpperMmPerSecond))
else if vUpperMmPerSecond > 0:
  etaLowerSeconds = lower_div(dLowerMm, vUpperMmPerSecond)
else:
  OutsideHorizon

foeEarliestEntryMs = floor(max(0, lower_mul(etaLowerSeconds, 1000.0)))
```

canonical `f32` acceleration 转 `f64` 是 exact，随后显式向上乘 1000；整数 mm、mm/s 与
`carry_um` 转换不先走 round-to-nearest 米制状态。`lower_* / upper_*` 对 `+ - * / sqrt`
按指定方向扩一 representable value；NaN、非法除数、upper infinity、checked floor 转换
失败或无法证明运行环境 primitive 契约时得到 `Unprovable`，不是 optimistic finite。
公式、operation order 与上下界方向属于合同，不能换成普通“数学等价”式后直接 floor。

Outside proof 使用同一 upper speed/acceleration 与
`frontierProofHorizonMs` 的 upper travel enclosure；只有严格证明最大可达距离仍小于
`dLowerMm` 才是 `OutsideHorizon`。无法严格证明时继续 ETA；仍失败才是
`Unprovable`。`OutsideHorizon` 不贡献，`Unprovable` 是保守 normal no-grant。

frontier cell 使用 static `ConflictPassageAddress`/zone cell，不按动态 Route occurrence
建表。每个 vehicle 对同一 cell 的 current/upcoming/repeated contributions 先 owner-local
归约：任一 `Unprovable` 则 owner 为 `Unprovable`，否则取最小 `Finite(ms)`。cell 只保存
按下列顺序最保守的 top-two distinct owners：

```text
Unprovable before Finite
Finite(ms) by ms ASC
exact tie by vehicleUpdateSequence ASC
```

candidate 查询 `valueExcluding(subject)`：first owner 不是 subject 时取 first，是 subject
时取 second；两者均无才是 `OutsideHorizon`。因此 looping/repeated Route 的 subject 自己
不会阻塞自己，其他 vehicle 仍保留；retained cells 不随动态 Route 数复制。

### 6.5 gap acceptance

对 subject coverage 的每个 exact cell 按 compiled target-cell range 求值：

1. incompatible occupant、committed reservation 或 earlier staged grant 存在时
   no-grant；
2. 若 target cell 有滞后基准，要求
   `currentTimeMs - referenceTimeMs >= minimumLagGapMs + clearanceBufferMs`；
   实际清空时 referenceTimeMs 即 `lastClearTimeMs`，跨修订无法继承历史时使用下面的
   保守切换基准；
3. target 的 `valueExcluding(subject)` 为 `OutsideHorizon` 时通过 lead check；为
   `Finite(ms)` 时必须严格大于 `requiredLeadMs`，等于仍 no-grant；`Unprovable`
   no-grant；
4. coverage 全部 cells 通过，才允许进入组合 resource acquisition。

`fixedDeltaTimeMs` 覆盖 subject 可能在本 interval 末尾才 crossing 的最坏时点。
stationary foe 只要 max acceleration 为正仍产生 finite earliest ETA；不能因当前红灯、
leader 或 ParkingStop 推断未来不动。clear 发生在 `[T,T+D)` 时，`lastClearTimeMs` 记录
post-step `T+D`；下一 tick elapsed 从 0 开始。future timestamp、状态不一致或非法 finite
input 是 step error，lead equality、任一 gap 不足与 `Unprovable` 是 normal no-grant；
lag equality 通过该项检查。

#284 实施候选对跨修订新 cell 补充保守初始化：只有真正无既往运行历史的新世界可以
直接使用无历史；切换新增或无法证明语义连续的无 occupant/reservation cell，以最终
静默提交的模拟时刻作为 lag 基准，不从 Prepare 起算，也不生成实际 clear 事件。
具体 tagged value、持久化和独立迁移期望值见
[`traffic-runtime-right-of-way-policy.md`](traffic-runtime-right-of-way-policy.md) §6.1–§6.2
（Review）。ActualClear 与 CutoverFloor 使用同一间隙比较边界，lead 检查不变。

### 6.6 mandatory downstream-clearance

grant 前必须从 tick-start committed occupancy、route-local hard boundaries 与 actual
vehicle profile 证明：subject 不依赖任何前车继续移动，也能在下一未获准 Gate/RouteEnd
前让车尾清空本 Gate coverage 的全部 passage，并原子取得对应物理 span。

route compiler 对每个 Gate occurrence 复用 #559 ordered coverage range/farthest clearance，
并编译 next Gate/route terminal fast boundary；Runtime 对具体车辆计算：

```text
clearanceFrontTarget = advance(farthestPassageClearance, vehicle.length_mm)

storageUpperBound = min(
  next leader rear - actual follower minimum_gap_mm,
  next ManeuverGate boundary,
  RouteEnd,
  ParkingStop,
  Waiting/other committed hard boundary)

downstreamClearanceAvailable =
  clearanceFrontTarget is finite
  AND storageUpperBound >= clearanceFrontTarget
  AND tryClaim(clearanceSpan) against committed + earlier staged owners
```

比较复用规范 integer route position 与 segmented distance，不恢复米制 tolerance：target
相等通过，提前 1 `carry_um` 失败。coverage 有多个 passages 时取 route order 最远
clearance。next Gate 当前即使 allow 也仍是 boundary；一个 grant 不预借后续 Gate。

`clearanceSpan` 从本 Gate crossed side 贯穿全部 coverage 到
`clearanceFrontTarget`，规范化成有序 physical edge/progress intervals；不能只 claim 最终
vehicle footprint。不同 Route/Junction 汇入同一 physical edge 时仍冲突。同一 owner 的
Waiting/downstream overlap 在 bundle 内去重，不得自阻塞；对不同 owner：

- interval overlap 直接冲突；
- 同向同 edge 的相邻 intervals 先确定 physical follower，再使用实际 follower profile
  的 `minimum_gap_mm`，不能无条件使用 candidate gap；
- committed 与 earlier staged claims 都可见；同 tick staged clear/release 不返还资源；
- 未落位 staged claim 不能作为可依赖其继续移动的虚拟 leader；必经 span 冲突即
  no-grant；不共享 physical span 的 candidates 互不阻塞。

route registration 只编译 profile-agnostic operands。spawn/replacement/leave/rebind 与
restore/cutover 在 static access 之后，对实际 `(profile, route, cursor)` 的 pending
resource occurrences 验证 empty storage 与 finite clearance；不相关长车型不能毒化整条
route。steady tick 的 finite storage 不足是 normal no-grant；operand/identity/invariant
损坏是 error。hot path 使用 route index/leader query，不扫描全 Route、全 vehicle 或全
Conflict catalog。

### 6.7 single-writer 组合 ledger

每个 candidate 的 staged capability 为：

```text
GrantResourceBundle
  WaitingDependencyFootprint?
    retainedMembership?
    releaseOnCrossing?
    newAdmissionEntitlement?
    downstreamDependencies[]
  WaitingAdmissionClaim?
  ConflictZoneClaims[]
  DownstreamClearanceClaim?
```

reducer 按 §6.3 candidate order，对 tick-start committed authority 加 earlier successful
staged claims 顺序执行一次 `tryAcquireGrantBundle`：

- 每辆车的 `WaitingAdmissionClaim?` 仍至多一个，只能对应 route order 中最早一个尚未
  持有的 Waiting occurrence；`downstreamDependencies[]` 是只读 cycle-proof operands，
  不是更晚 occurrence 的 claim 或 reservation；
- 一个 candidate 要么取得所需全部资源，要么一项都不提交；
- 完整 preflight 后一次 stage，失败 ledger 不变；不允许 partial acquisition、
  hold-and-wait 或“先占 Waiting 再等 Conflict/downstream”；
- bundle 内 resources 按 stable canonical rank/physical interval order preflight；跨
  WaitingZone 相反 route order 仍使用同一规范读取顺序；该顺序只防止实现层锁顺序
  漂移，业务 cycle prevention 必须通过下述 dependency graph 证明；
- grant 已取得也只移除对应 Gate resource stop，不能覆盖 leader、minimum gap、
  no-overlap、safe speed、ParkingStop 或 RouteEnd；
- motion 最终未 crossing 时整份 tick-local bundle 失效，不提交 membership、reservation
  或 downstream owner；
- proposal 可以并行，resource mutation 必须单 writer；Mutex/CAS/worker completion
  不得决定 winner。

`WaitingDependencyFootprint` 让 #284 在提交新 membership 前证明不会形成 committed
hold-and-wait cycle。route compiler 为每个 Waiting occurrence 编译车辆在释放该
membership 之前必须取得的后继 Waiting occurrences；当前禁止 overlap/nesting 后，现实
热路径主要是 shared release/next-entry，但算法不能按 LaneEdge 名称猜测。reducer 对
candidate 的 tentative admission、committed/earlier-staged memberships 与这些依赖建立
owner/resource wait-for graph：

- resource 只有在 count/storage 对该依赖不足时才产生 wait edge；
- tentative owner 与其 retained membership/release intent 全部进入同一次 checked graph；
- 加入 candidate 后若出现含两个或以上 distinct owner 的 SCC，candidate 得到 normal
  `NoGrant(WaitingCycle)`，ledger、entitlement 与 world 均不改变；
- 同 tick 后续 candidate 只观察此前成功 staged 的无环结果，因此任何 successful tick
  都不能创建 committed Waiting cycle；
- SCC 的 node/edge、扫描与首个拒绝原因按 WaitingZone stable canonical rank、
  `vehicleUpdateSequence`、route occurrence/hop 总序，不能依赖容器顺序。

cycle prevention 是提交前预防，不是把已经占位的车辆原子换位。same-tick staged release
仍不返还 capacity，不把 committed membership 转给另一车辆，也不为同一车辆取得多个新
Waiting claim。所谓重分配只允许在 committed mutation 前改变尚未提交的 candidate
bundle 选择；#284 安装、restore 或 cross-revision cutover 若发现输入世界已经含 wait-for
cycle，整体失败关闭，由宿主在旧 world 清空后重试，不通过 teleport、capacity 透支或
忽略 leader/no-overlap 修复。

同一次 evaluation 的 normal no-grant attribution 固定为：Waiting capacity、Waiting
physical storage、Waiting cycle、Conflict occupied/reserved/staged、lag gap、approach
`Unprovable`、lead gap、downstream storage boundary、claim conflict。同类实体按
canonical rank，physical intervals 按 route/edge order。该顺序只决定审计 record，不
允许“先报错”留下部分 claim。

### 6.8 grant、reservation、Clearing 与 lifecycle

`ConflictGrant` 是当前 tick、私有、不可伪造/复制的 staged capability。只有 successful
Gate crossing 消费它并原子提交：vehicle traversal、可选 Waiting leave/enter、non-empty
`ConflictReservation` 与 committed downstream claim。没有 Conflict passage 的 pure
Waiting admission 不创建空 reservation。

non-empty reservation 保存 vehicle、route/maneuver/Gate occurrence、exact passage range、
downstream owner 与 acquired tick。front crossing passage entry 建立 zone occupancy；
actual vehicle rear 到达/越过 exact clearance 才 clear。全部 coverage clear 后释放
reservation/downstream claim；post-step occupancy 已成为下一 tick 权威后才释放，不能出现
无 owner 空窗。

#284 与正式 reservation 同时为 `ManeuverTraversalState` 增加真实 `Clearing`：

```text
PreGate | Committed | Waiting
  -> Clearing { reservation }
Clearing
  -> Committed | PreGate | completed traversal
```

crossing release Gate 时先移除旧 Waiting membership，再提交可选下一-zone membership 与
reservation；`Clearing` 不携带 Waiting membership。未 crossing 的 grant 不改变 committed
phase。route completion 前必须清空 reservation；despawn 原子释放 Waiting、Conflict 与
downstream authority。active reservation 的 arbitrary route replace/rebind 除非完整证明
同一 stable passages/claims/footprint，否则失败关闭；不能只迁移 enum。

#559 `ConflictRuntimeUnavailable` 只在本节 policy、arbiter、grant/reservation、组合 ledger、
snapshot/cutover 与测试同切片安装后原子移除。禁止先解除 3A，再补任一资源 owner。

### 6.9 fixed step、观察、事件与 first-error

#284 扩展 staged step，但不改变“只收紧 motion”：

1. 冻结 signal、vehicle、route、Waiting/Conflict/downstream committed state；
2. 重建 route-relative occupancy、leaders 与 top-two approach frontier；
3. 解析 Gate regulatory decisions，构造完整 evaluation records/requests；
4. 对 claim/decision/transition/event 的 actual checked count 在 mutation 前
   `try_reserve`；
5. stable-sort candidates，single-writer acquire all-or-nothing bundles；
6. 把缺失 grant 加为 Gate hard stop，与 Parking/RouteEnd/leader/no-overlap 共同归约；
7. stage motion、crossing、Waiting/Conflict/reservation/claim transitions；
8. 原子提交 state、latest batches、events、tick/time。

latest decision 至少区分 `NotEvaluated | NotRequired | Granted | NoGrant(reason)`，只描述
刚完成 successful tick，不是下一 tick lease。normal no-grant 不 spam error/event；
projection 只在首次 hard boundary contact transition 产生。

统一 transition event batch 继续以
`(vehicleUpdateSequence, routeAnchor, eventKindRank, staticCanonicalRank)` 总序。#284 在不
改变 #282 Waiting 相对顺序的前提下扩展同 anchor rank：projection、Gate crossing、
Waiting leave、Waiting enter、reservation acquire、Conflict enter、Conflict clear、
reservation release、maneuver completion。route anchor 使用 exact occurrence/hop/position；
raw handle、producer 或 arbitration completion 不参与。

下列是 normal outcome：regulatory deny、Waiting capacity/storage 不足、prospective
Waiting cycle、Conflict occupied、lag/lead gap 不足、approach `Unprovable`、downstream
storage/claim conflict、priority loser、grant 因更严格 motion constraint 未 crossing。
下列使 step 零提交失败：unknown/invalid
occurrence、non-canonical/non-finite authoritative input、occupancy/membership/reservation
不一致、duplicate/stale claim、partial bundle、no-grant Gate 被 crossing、completion 留下
authority、counter/tick/time overflow。

新增 policy/source normalization 的 first-error 必须追加在届时 current compiler/LFCA
既有规范 prefix 之后，不能重排无关现行错误。policy 内按 declaration phase，再按 wire
order；derived pair/cycle 按 owner/member canonical tuple。Runtime 多错误候选按 logical
phase、stable vehicle/route/entity key 选择首错，不能依赖 scan/worker 完成顺序。

### 6.10 snapshot、cutover、容量与验证矩阵

#284 从届时 current snapshot/runtime/digest 版本一次性升级，不预占 #282 的 4/4/6：

- 持久化 pinned policy identity、`firstEligibleTick`、Clearing/reservation、committed
  downstream claims、Conflict 滞后基准与必要 semantic owner；occupancy 从 reservation
  和实际位置重建，基准类别及保守切换起点见实施候选 §6.1，不伪造实际 clear 事件；
- tick-local grants、approach frontier、target ranges、dense handles 与 scratch 是派生状态，
  restore 从 stable policy/route/vehicle identity 重建；
- restore/same/cross-revision cutover 必须重新编译 policy/route operands、核对 capacity、
  重建 ledger/occupancy/Waiting dependency graph，并验证 owner 闭合且不存在 committed
  wait-for SCC；任一失败零发布；
- 不保留旧 reader、双写、迁移 shim 或 feature flag；#284 正式能力与 3A 移除必须处在
  同一可恢复/可切换切片。

容量按现实 retained payload 收费：static frontier cell/top-two 按共享根 passage cell 数，
route occurrences 继续由独立 route conflict capacity 约束；reservation/owner 由
`vehicle_capacity` 约束；entitlement 与 wait-for node/edge scratch 按 actual checked
request/dependency count 扩至 high-water mark，不保留 `O(V²)` 邻接矩阵。dynamic
Route/repeated occurrence 只增加 route metadata/contribution visits，不能复制 static
frontier cells。warm-up 后 steady tick 零 heap allocation。

#284 targeted validation 至少覆盖：

- policy provenance、specificity、totality、cycle、protected coherence、multi-subject
  coverage-min priority 与 exact target-cell ranges；
- current/upcoming/repeated passage、cursor `carry_um`、directed helper predecessor/equal/
  successor、subnormal/overflow oracle、Finite/OutsideHorizon/Unprovable；
- looping subject self contribution 与 top-two distinct-owner exclusion；dynamic Route 数
  增长不增加 static cell count；
- lead strict equality 拒绝、lead +1 ms、lag equality 接受、post-step last-clear time、
  protected/permissive/uncontrolled；
- downstream target equality/提前 1 `carry_um`、variable length/minimum gap、next Gate/
  RouteEnd/ParkingStop、跨 Route/Junction physical span conflict 与 disjoint span；
- bundle all-or-nothing、相反 Waiting route order、committed/earlier staged visibility、
  zone-local entitlement 不受 policy priority 覆盖、prospective wait-for SCC rejection、
  same-tick release 不返还、每车一个新 claim、unused grant expiry、crossing commit 与
  tail-clear/despawn release；
- proposal/worker/container/handle/static declaration permutation 下 winner、state、decision、
  event 与 digest 相同；allocation/counter/invariant failure 全事务回滚；
- restore/replay、same/cross-revision cutover、policy/route drift、malformed owner 与 3A
  原子接管；
- 10k 产品档报告 p50/p95、zero allocation 与 retained bytes；100k scaling 档报告
  contributions、visited passages、static cells、top-two bytes、claim/collision/query counts。

这些是 #235 已接受的 #284 实现合同，不是 #282 的验收项；若不再保留上述行为，必须在
#284 实现前显式重新打开 G1。

#282 不实现上述组合 ledger 的简化副本。#284 也不得重新定义 Waiting counter、queue
或 membership。

## 7. 确定性规则

### 7.1 Waiting candidate

同 zone candidate 从 tick-start committed snapshot 计算，并按：

```text
(approachDistanceMm ASC, vehicleUpdateSequence ASC, entryHop ASC)
```

`approachDistanceMm` 在 Waiting claim 改变 GateStop 前冻结。这样物理前车不会因后
spawn 或较大 update sequence 被物理后车长期抢走唯一容量。

#284 必须把该顺序的 zone-local entitlement 作为 bundle 前置条件；§6.3 的全局法规/
冲突 priority 只决定不同本地 entitlement 之间的组合仲裁，不能重新裁决同 zone winner。

### 7.2 admission sequence

successful entry 在 motion staging 后按 post-step 物理 front-to-back 顺序分配
admission sequence；`vehicleUpdateSequence`、`entryHop` 仅作防御性总序。
counter checked overflow 使整 tick 失败，不 rollover、不饱和、不重编号。

### 7.3 decision、phase 与事件

latest decision batch 按稳定 route order
`(vehicleUpdateSequence, entryHop)`。release Gate 与 leader/minimum-gap 得到相同
`finalTravelMm` 时，release Gate attribution 胜出；该规则只影响 phase/event 归因。

committed Waiting transition event batch 按
`(vehicleUpdateSequence ASC, routeAnchor ASC, eventKindRank ASC)` 形成全序；
`routeAnchor` 包含精确 route occurrence/hop，不能使用 raw handle 或 producer 完成顺序。
同一 anchor 的 rank 固定为 projection、Waiting leave、Waiting enter、maneuver
completion，因此同一 shared boundary 仍先 leave、后 enter，最后 completion。
per-vehicle horizon 首次把车辆从上游投影到更晚 entry boundary 时，产生
`EvaluationHorizon` projection event；下一 tick 的 capacity/storage no-grant 进入 latest
decision，不重复产生 boundary projection event。

## 8. 固定步进组合

#282 的 Waiting slice 接入现有 staged step：

1. 冻结 tick-start signal、vehicle、occupancy 与 route state；
2. 验证 Waiting state、Parking 正交状态及已编译 route operands；
3. checked 计数并预留 claim/decision scratch；
4. 按 zone stable order stage 本地 claims；
5. 将 Waiting GateStop 加入现有整数毫米 hard projection；
6. stage motion、crossing、phase、membership 与 queue transition；
7. checked 计数并预留 transition/event scratch；
8. 按 post-step 物理顺序分配 admission sequence；
9. stage snapshot/journal 可见增量并原子提交。

每个 scratch batch 都以实际 checked count 在 committed mutation 前 `try_reserve`。
不能把“每车最多一个新 claim”误用为 decision/transition/event 也必然每车一条：
same-tick enter+leave、shared boundary 与 traversal completion 都可能增加条目。

## 9. #559 保护与 #284 接管

#559 的临时 3A 保护要求每个 committed Active 车辆的 route-local rear 能清除路线中
全部 `ConflictPassageOccurrence`。保护覆盖 spawn、completed replacement、Parking
离场/路线重绑定、restore 和 same/cross-revision cutover。

#282 必须保持这套检查和 `ConflictRuntimeUnavailable` 原样生效。它不能因为车辆将
等待在某个 WaitingZone、因为 signal 当前为红灯，或因为本地 claim 尚未取得而推断
“暂时不会进入 conflict”并放宽能力检查。

#284 交付正式 conflict grant/reservation 与组合 ledger 时，才在同一切片原子移除 3A
保护。禁止先移除保护、后补仲裁。

## 10. 快照、摘要与修订切换

#282 已随本地 Waiting 逻辑状态完成一次性升级，当前唯一生产版本为：

| 权威轴                  | 当前版本 |
| ----------------------- | -------: |
| LFRS `formatVersion`    |        4 |
| `runtime_state_version` |        4 |
| deterministic digest    |        6 |

只保留当前 writer/reader；旧 v3 输入明确失败关闭，不保留双读、双写或迁移 shim。

持久化保存 vehicle traversal/membership、zone occupancy、next admission counter 与稳定
queue order；稠密 handle/link 在 restore 重建。空 zone 只要 counter 非零就仍有逻辑
历史，跨修订不得因 occupancy 为零而丢弃其 stable identity。

restore/cutover 必须同时完成：

- Waiting stable identity、route occurrence、phase、capacity、物理顺序与 Parking
  正交状态验证，包括 `Active + Reserved` 的 parking arrival 后不得仍有 traversal，且
  entry 不得落入 `[waitingEntryBoundary, waitingReleaseBoundary]`；
- `ConflictPassageOccurrence` 重编译和
  `route_conflict_occurrence_capacity` 核对；
- 全部 Active 车辆的 #559 3A 能力检查；
- journal replay 后 semantic digest 一致。

任一步失败都不发布候选 world，不清空旧 world 的 Waiting、Parking 或 Conflict 相关
状态。

验证必须覆盖 parking entry 位于 stateful maneuver 前、PreGate 区间、Waiting entry
boundary、内部、release boundary、maneuver exit 与 exit 后方，以及 repeated route
occurrence、reserve/rebind 原子失败和 restore/cutover 失败关闭；已有 Waiting authority
的 rebind 还要验证精确映射成功与任一 identity/phase/footprint 漂移失败。多车辆
producer/staging/container 顺序置换必须得到逐项相同 transition event batch。

## 11. 只读观察

#282 提供只读：

- vehicle traversal state；
- zone occupancy/`maxOccupancy` 与 admission order member batch；
- successful tick 的 latest Waiting decision batch；
- Waiting transition event batch。

Waiting outcome 至少区分 `NotEvaluated`、`NotRequired`、`Granted`、
`NoGrant(Capacity)` 与 `NoGrant(PhysicalStorage)`。它们是刚完成 tick 的审计记录，
不是下一 tick 的 lease。

#284 后续可扩展 Conflict/downstream outcome，但不得改变上述 Waiting reason 的语义。
Adapter 和 Scenario 不获得 claim、queue、counter 或 phase mutation API。

## 12. 容量与性能

- zone state 按 `SharedNetworkRevision` 的静态 WaitingZone 数量稠密构造；
- 不新增调用方 WaitingZone capacity 轴；
- member/link 由 `vehicle_capacity` 约束；
- conflict occurrence 继续使用独立 `route_conflict_occurrence_capacity`；
- claim、decision、transition、event scratch 使用当 tick actual checked count；
- warm-up 后 steady Waiting phase 应为零 heap allocation；
- hot path 不扫描静态 catalog、不做 external string lookup、不依赖 HashMap iteration；
- 当前不把 fixed step 改为多 worker。

验证以现实产品规模为主：10k 道路机动车报告 Waiting 增量 phase p50/p95 与硬预算，
100k 报告 scaling、retained bytes 与 visit counts。不会为理论上全路线、全车辆、全
occurrence 同 tick 笛卡尔积预留常驻内存。

## 13. 实施与验收边界

### #282

必须覆盖：

- `PreGate / Committed / Waiting`、membership、queue 与 counter；
- variable vehicle length、minimum gap、no-overlap、满容量和物理空间不足；
- 同车同 tick 仅最早一个新 Waiting claim；
- stable candidate order、post-step admission order、release Gate tie；
- Parking lifecycle、completion、replace、despawn 同步 release record 与 route guard；
- snapshot 4/4、digest 6、restore、journal 与 cutover；
- #559 3A 保护和 `ConflictRuntimeUnavailable` 不回退；
- checked exact-count scratch、失败原子性和 10k/100k 证据。

#282 不交付 downstream-clearance、Conflict arbitration、组合 ledger 或 cycle
prevention，也不记录完整冲突能力已经生产化。

### #284

后续实现直接消费 §6 的 Accepted 合同，必须覆盖：

- regulation/policy normalization、multi-subject coverage-min priority 与 protected
  coherence；
- current/upcoming/repeated passage 的 directed lower-bound ETA、top-two distinct-owner
  frontier 与 gap acceptance；
- 通用 downstream-clearance 与 physical-span claim；
- stable candidate、conflict grant/reservation/Clearing/tail-clear；
- Waiting/Conflict/downstream single-writer 组合 ledger；
- zone-local physical entitlement 不受法规全局 priority 覆盖；
- 跨 WaitingZone 原子取得、未提交 bundle 选择与 prospective wait-for SCC cycle
  prevention；不交换 committed membership，不放宽每车一个新 claim；
- 与 #282 Waiting 状态、#559 occurrence、Parking、snapshot 和 cutover 的原子组合；
- first-error、ordered decision/event、10k/100k determinism/performance matrix；
- 正式能力安装与 3A 临时保护移除在同一切片完成。

### 共同不变量

任何 successful tick 或 lifecycle transaction 后：

- occupancy 等于 semantic memberships 数；
- queue 与 admission sequence 严格一致；
- Parked/Completed 无 Waiting membership；
- no-overlap 与 minimum gap 成立；
- grant 不覆盖更严格 motion constraint；
- digest/replay 与稳定事件顺序不依赖本地 handle 或执行完成顺序。
