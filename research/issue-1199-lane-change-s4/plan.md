# Issue #1199 S4 实现方案：能力落地与 R1–R3 运行时验收

状态：**G2 已记录**（lead 裁定 2026-11-01）：①资源模型 = 复用 staged→committed
（过渡段作独占冲突资源入 staged claims，不新增平行预约系统）；②PR 标题不加
`!`，PR body 增「行为变化（授权语义演化）」专节（5 条 S3 测试断言迁移对照 +
能力翻转前后语义）；③准入拒绝面**新钉**（S4 专属独立钉版，A5 交叉处仅引用不
复用，S5 统一核对）。维护者 2026-11-01 批准 S4/S5 立项（#1199 G0/G1 记录）。
依据：`docs/design/lane-change-minimal-contract.md`（Accepted，§5/§6/§7）、
`research/issue-1142-route-design/sample/minimal-closed-loop.json` v5 钉版、
PR #1188 S3/S4 验收边界条目、#237 G1 冻结包（A1–A6 裁定）。

## 1. 范围与非目标

- 范围：能力落地｜R1a（s6）｜R1b（s12，两层断言）｜R2（s4/tick 0 钉拍）｜
  R3（s8 车尾清空 + §5.1 撤销分支）｜world 级注册表原子性｜准入/恢复拒绝面｜
  g1-decision-chapter A6 引用对齐。
- 非目标：S5 持久化交叉（s10 三捕获点）；S6 LuST 全量发射；合同语义变更。

## 2. 工作项与落点

### 2.1 能力落地（实现属性、无配置）

- `crates/laneflow-runtime/src/kernel/tables.rs`：`transition_execution_supported()`
  → `true`（A6 选项 3：实现能力，无配置字段，无 WorldConfig/ExecutionConfig 扩义）。
- 语义变化（合同 §6.3「能力落地后（S4）」授权）：`is_lateral_transition_segment`
  分类表保留（段级/横向坐标判据不动），但分类命中**不再拒绝**——转入过渡预约
  执行路径（§2.2）。`TransitionUnsupported` 结构化拒绝面保留给准入/恢复入口的
  不支持形态（§2.6）。
- S3 既有 5 测试的演化（如实记录，非修绿）：分类矩阵断言保留（判据不变）；
  「拒绝」断言改为「进入过渡预约路径」断言；`capability_rejection_*` 语义由
  「注册拒绝」迁移为「注册成功 + 预约语义」。**旧输入零新拒绝**：639 条无过渡
  语义测试零改动。
- 等值：converter 318 基线零改动照跑。

### 2.2 过渡预约执行（R1–R3 的运行时主体）

- **资源模型**（提议，待 G2 确认）：过渡段（`T01`）作为**独占冲突资源**接入
  现行 staged→committed 预约机制（`kernel/conflict_tick.rs`：staged claims →
  P7 提交 / 失败丢弃；`ConflictClearOutcome::ReservationReleased` 清空释放），
  不新增平行预约系统；过渡区间 = 整边保守投影（A2）+ §6.4 重叠分类。
- **P3/P4**：换道申请按 §5.2——枚举全部**潜在进入者**（过渡区及上/下游当前
  车辆，实际车体记录），逐个证明可让行：具名四项 `d_stop = v·(r·Δt) +
  v²/(2·a_service) + min_gap`（r=2 拍、Δt=0.1 s、a_service=4500 mm/s²、
  min_gap=2500 mm；a_emerg=9000 仅作不可迫使急刹的界）；`MotionReach` 仅为
  本拍位移上界（13.9 m/s × 0.1 s = 1.39 m/tick），当拍不可达**只可跳过当拍、
  存续期重证**。任一潜在进入者不可证明 → no-grant。
- **仲裁（R1）**：普通直行车**不进入 P4 候选队列**（不提交过渡预约 bundle），
  约束经「潜在进入者证明（P4）+ P5 遵守」两条路径覆盖（§7 仲裁模型冻结）。
  换道申请之间：安全证明先行；可证安全的资源冲突按 **live 序**（`committed
  .live_order`，现行稳定序）裁决恰一优先。
- **§5.1 撤销分支**：P4 授予（staged）后 P5 未产生实际 crossing → 整 bundle
  到期（不留痕）；「同拍进入并清空」事件只在实际 crossing 且清空条件提交后
  产生（P7 原子）。
- **P5 遵守**：全体进入者对当拍预约集（committed + granted）遵守；尾随车不得
  进入被预约区间；P5 运动从 C(T) 计算（预览无提交权）。
- **车尾清空（R3）**：预约保持到**真实车尾清空**（`rear >= clearance`），不得
  车头离开即释放；源/目标/过渡区间均纳入占用投影（长车 12 m 扫掠全区）。

### 2.3 R1a（sample s6，钉版）

构造：`v-change-1`（B0_up@0，10 m/s，过渡路线）× `v-lane1-straight`
（B1_up@0，10 m/s，直行路线），对称同拍，拍初过渡区为空。
断言：①直行车不进 P4 候选队列（不产生过渡预约 bundle——断言其不在候选集）；
②换道 P4 证明覆盖该直行车（d_stop ≈ 15.6 m > 距被预约区间 8 m，不可安全
让行）→ no-grant、换道让行；③直行通过后换道进入；④全程无物理重叠；
⑤结果与 worker 配置无关（遍历序/调度不得定序）。
失败面（照录）：以「拍初为空」放行两车；换道抢行迫使直行急刹；向直行车发
预约 bundle。

### 2.4 R1b（sample s12，两层断言）

构造：`v-change-1`（A0@19000）× `v-change-2`（A0@11000），同速 10 m/s；
距 T01 入口 19 m / 27 m，均 ≥ d_stop ≈ 15.61 m（**双方均可安全让行前提**）；
本拍位移上界 1.39 m → 双方均**不可能本拍进入**。
断言分层：
- **层①（tick 0，前摄证明/优先裁决）**：前提成立时双方 P4 证明均通过；T01
  独占冲突按 live 序稳定裁决**恰一优先**；**不建立预约**（§5.1 撤销分支：
  无 crossing 即到期——断言 staged 到期、无 committed 残留）。
- **层②（实际进入）**：优先方到达入口拍 **crossing 恰一次**并建立预约；
  败者同拍申请 no-grant、`sl-t01` 前等待，`rear >= clearance` 后进入；每
  crossing 拍**恰一车进入**；重放与不同 worker 配置结果一致。
失败面（照录）：在距 T01 > 1.39 m 处断言「本拍进入」；把前摄优先序当作已
建立预约；前提不成立（任一方 d_stop > 距离）时硬套断言（应退回证明先行
语义）；以遍历先后/worker 调度定序；两车同 crossing 拍进入；败者被跳过/补位。

### 2.5 R2（sample s4，关键拍 tick 0 钉定）+ R3（sample s8）

- R2 构造：`v-change-1`（B0_up@7000，10 m/s）× `v-target-follower`
  （B1_up@0，13.9 m/s）。关键拍 tick 0：后车距被预约区间入口 8 m，d_stop =
  13.9×0.2 + 13.9²/(2×4.5) + 2.5 ≈ 26.75 m > 8 m。
  断言：P4 no-grant（**具名参数**计算，不引用笼统 profile 制动能力）；换道车
  `sl-t01` 前停让；后车通过后（MotionReach 当拍跳过的各拍在存续期重证）进入；
  后车不被迫紧急制动（a_emerg 仅界）。**对照分支**：后车处于 `d_stop +
  min_gap` 之外且存续期各拍均可让行 → 可授予（防过保守恒拒绝）。
- R3 构造：`v-long`（truck-long 12 m，B0_up@6000，8 m/s）× `v-source-
  follower`（A0@18000）。断言：整车扫掠占用投影（源/目标/过渡区间）；预约保持
  到 `rear >= clearance`；清空前跟随车不得进入被预约区间（P5 + P4 制动证明）；
  清空后释放、无残留占用/预约。失败面（照录）：车头离开即释放；只查过渡区边
  不查源/目标区间；长车跨边占用漏算。
- 同拍进入并清空语义（§5.1）：实际 crossing 且清空条件提交后产生事件；
  P7 状态/预约/事件单次原子提交。

### 2.6 world 级注册表原子性 + 准入/恢复拒绝面

- world 级：`TrafficWorld::install` + `register_route`（含过渡路线，能力翻转后
  成功）；失败注入（非法注册）→ 注册表不变（D5 失败原子，逐字段断言）；
  补 compile 级纯度证明之外的 world 级缺口（S3 留档项）。
- 准入：绑定期复核入口保留结构化拒绝（失败关闭、零部分状态）——不支持形态
  （如跨修订 cutover v1，A5）注入夹具断言 `TransitionUnsupported`/对应错误类
  可达；过渡路线本身在能力落地后放行。
- 恢复：staging 零部分丢弃路径断言（部分恢复不得留痕）；与 S5 持久化交叉的
  边界注明（s10 捕获点断言留 S5）。

### 2.7 g1-decision-chapter A6 引用对齐

- `research/issue-1177-route-g1-freeze/g1-decision-chapter.md` A6 条目引用
  更新为最终判据（合同 §6.5 表驱动接缝矩阵：横向坐标 =（section 走廊元素位,
  section 内 lane 序数），同走廊且跨度 ≥ 2 → 横向过渡段）与能力语义（实现
  属性、无配置、三入口结构化拒绝面）。

## 3. 测试载体

- 夹具：S3 合成样本网络（20 边，含 `T01`/`T0a…T0b`/纵向 `X`/接缝 `SP`）即
  sample v5 拓扑超集——s4/s6/s8/s12 场景按 actors 钉版 spawn（edge+progress_mm+
  route_edge_index+initial_speed_mm_s）在 `laneflow-runtime` 测试内直接构造
  （显式边序列路线，route-system D1）；fixed_delta_time_ms=100 固定步进。
- 新增验收测试（`kernel` 测试模块，命名锚 R1a/R1b/R2/R3 + scenario id）：
  每用例含接受断言 + 失败面断言（合同 §7 照录）；确定性断言（重放一致、
  worker 配置无关）。
- 等值计划：converter 318 基线零改动；runtime 639 无过渡基线零改动；S3 的 5
  条过渡测试按 §2.1 演化（变化逐条记录理由）；clippy/fmt/format-md-tables/
  git diff --check 全量。

## 4. 纪律

- 等值失败如实报不修绿；验收失败面照录；不改合同语义；术语中文权威。
- 流程：方案报审 → G2 → 实现+等值验证 → 报告 → PR（不入列，报「就绪待入队」）。

## 5. 风险与待 G2 确认点

1. **资源模型**：过渡段作为独占冲突资源复用 staged→committed 机制（§2.2）
   ——**G2 裁定：照准（复用，不新增平行预约系统）**。
2. **S3 测试演化口径**：**G2 裁定：PR 标题不加 `!`；PR body 增「行为变化
   （授权语义演化）」专节（5 条断言迁移对照 + 能力翻转前后语义）**。
3. **准入拒绝面注入方式**：**G2 裁定：新钉（S4 专属独立钉版；A5 交叉处仅
   引用不复用，S5 统一核对）**。

## 6. 现状勘察结论（实现落点依据）

- 冲突预约机制已具（`kernel/conflict.rs` + `kernel/conflict_tick.rs`）：
  `ConflictZone`（静态稳定 id）× passage/stream 单元格、staged grants →
  P7 提交 / 失败丢弃、`ConflictClearOutcome::ReservationReleased` 清空释放、
  `evaluate_yield_target` 让行评估（`ConflictYieldOutcome`：
  Accepted/Occupied/LagGap/LeadGap/ApproachUnprovable，时间间隙粒度）。
- **缺件 = 合同 §5.2 具名四项距离判据**：现有 check_gap 为时间间隙口径，
  A4 冻结要求 `d_stop = v·(r·Δt) + v²/(2·a_service) + min_gap` 具名参数——
  已落 `kernel/lane_change.rs`（`stop_distance_millimetres` /
  `yieldable_by_stopping` + 三钉版测试：26747 mm（R2/s4）、15611 mm（R1/s6、
  s12 双方前提）、对照分支防过保守）。
- 集成路径（下一步）：过渡段（分类表）→ 静态侧 ConflictZone 资源（独占）→
  换道申请入 staged claims（G2①）；P4 潜在进入者证明 = 枚举（实际车体记录）
  × `yieldable_by_stopping`（具名四项）× MotionReach 本拍上界跳拍重证；仲裁
  = 证明先行 + live 序（`committed.live_order`）；§5.1 撤销分支复用 staged
  到期语义；车尾清空复用 `ConflictClearOutcome` 清空条件（`rear >= clearance`）。

## 7. 差异记录 / 发现记录

- **隐式布局/代码形态敏感摘要面（发现记录，S5 版本轴域地雷）**：S4 增量1
  诊断中，phase 等值 digest 的 `conflict=`/`conflict-journal=` 两分量对**行为
  逐值等价**的代码改动持续漂移（fixture `phase-protocol-e011745e.txt` 钉死），
  而全部显式预映像路径（`canonical_state_preimage`/`push_conflict_locator`/
  稳定 ID 序列化）逐行审查与布局/形态无关；两项首嫌均已实验排除——
  ① `zone_span` 结构字段改懒派生（零结构增量）后仍漂；② `ConflictPassageAddress`
  布局 12→16B 后复原（32 位身份编码实验 A）仍漂，且 digest 值随等价改动
  变化（d835→3e27）——**存在未定位的隐式摘要面**（size_of/retained/原始
  内存核算或哈希形态类）。**S5 布局/格式变更（digest 9/前像扩展）必须显式
  审计该面**，勿以"逐值等价"推定 digest 稳定。
- **A 实验（布局保持身份编码，裁定批准并执行）**：地址身份 32 位编码
  `raw = z.raw()` / `0x8000_0000 | edge.raw()`（与 `resource_order_raw` 同
  编码），accessor 解码保真、身份来源 = 分类表派生不变、结构 12B 复原。
  **容量注记**：编码假定 `edge.raw() < 2^31`（网络规模上限保证）；突破须换
  64 位身份并走 S5 版本轴。A 后等值仍红 → 按裁定转 B 深挖（未提交、不猜）。
- **B 深挖定位结果（根因已锁定并修复）**：byte-diff 探针证明 `canonical_state_
  preimage` 全时点（k=0…640）与 restore+续跑指纹**逐字节全同**——状态序列化
  无恙；泄漏面 = **`{:?}` Debug 形状即摘要面**：phase 轨迹摘要以
  `checkpoint`/`{outcome:?}` 的 Debug 串为预映像（`latest_conflict_decisions`
  等含 `ConflictPassageAddress`），其派生 Debug 形状随字段改名漂移
  （zone/resource/raw 三形状 = 三指纹，全部观察闭合）。修复：手工 `Debug`
  **逐字复原历史形状**（`zone` 字段名 + Ordinal Debug 值），零 digest/fixture
  变化、phase 等值 5/5 回绿。**S5 审计要点（升级）**：凡进入摘要/轨迹预映像
  的 `{:?}` 输出都是格式面——结构重命名/字段调整/derive→manual 切换都会
  漂移指纹；digest 9/前像扩展须把 **Debug 形状**列入显式审计清单。

## 8. 增量2a 执行简报（裁定 B；供即刻执行，零再推导）

- **流绑定规则（B）**：过渡出现项 = `ConflictPassageOccurrence { resource:
  ConflictResource::Transition(edge), stream: <跨越流>, passage_local_index: 0, … }`；
  跨越流 = 该路线中过渡段**之后首个 maneuver 出现项的 stream**（无后继时取
  前驱 maneuver 流，确定性钉死）；独占由 zone-head 所有权承担
  （`zone_owned_by_other`；s12"恰一优先"即该语义，多 cell 形态正确性集中）。
- **站点清单**（`ConflictPassageOccurrence.zone` 资源化；注意与
  `WaitingOccurrence.zone: WaitingZoneOrdinal` 同名干扰项区分，后者勿动）：
  tables.rs 结构/`address()`/发射点 :835/:1100/测试构 :2952/:3060/:3304；
  conflict_tick.rs :1244/:4111（`occurrence.zone.raw()` →
  `resource_order_raw(occurrence.resource)`）/:6894/:6912（静态校验比较 →
  `ConflictResource::Zone(z)`/`static_zone` 语义）；tick.rs :2669/:2692 同。
  **Debug 纪律（B 教训）**：改结构前先查 checkpoint/{outcome:?} 预映像是否
  含 occurrence Debug；若有 → 手工 Debug 复原历史形状再改。
- **不变量验收（lead 附条）**：两不同流争同段 → 恰一 staged 授予、败者
  no-grant、同拍至多一 claim；与 R1b s12 两层断言互证（zone 级独占承载）。
- **P4 具名判据接线**：潜在进入者枚举（实际车体记录）×
  `yieldable_by_stopping`（具名四项）× MotionReach 本拍上界跳拍重证（存续期
  重证）；任一不可证 → no-grant。撤销分支 = staged 无 crossing 到期（§5.1）；
  清空 = `rear >= clearance`（ConflictClearOutcome 现成语义）。
- **能力翻转次序**：P3-P7 接线全绿后方可翻 `transition_execution_supported`
  （防无预约穿越——翻转与预约路径同切片落）。
