# Issue #1217 方案稿：审计 P1-3/P1-4 —— 换道持久化边界封闭

**状态**：方案稿（阶段 1，待 lead 批准后进入实现）<br>
**作者**：fix-1217（实现工程师）<br>
**基线**：`main@45263887`，worktree `research/1217-persistence-boundary`<br>
**范围**：`laneflow-runtime` 持久化边界的保存 / 摘要 / 恢复三链；不做完整持久化格式（S5 #1200），不动 #1216 在途的冲突枚举面

---

## 0. 结论摘要

P1-3（快照 panic）与 P1-4（身份错绑）的根因是同一个：**v7 持久形制只有
`(participant_stream, conflict_zone)` 定位对与 Gate 稳定身份槽，表达不了
「资源种类 + 横向过渡段（T）稳定边身份」**，而实现却把 T 的**归属信息**（其
stream passage 的 `(stream, zone)` 对）当成**身份**写进持久面：

- P1-3：T 出现项的 `admission_hop` 是前驱边下标、不是 Gate hop
  （`tables.rs:1216-1218`），快照代码却按「hop 必解析 Gate」`expect`
  （`admin/snapshot.rs:923`、`admin/snapshot.rs:1029`），正常运行态（车辆入 T
  后、车尾清空前）直接 panic 传播宿主。
- P1-4：T 格的清空历史经 `conflict_passage_locator` 的归属派生
  （`kernel/world.rs:2990-3003`）写成静态 Zone 对（`admin/snapshot.rs:724-744`），
  恢复端 `unique_address(zone, stream)` 重绑回**静态 Zone 格**
  （`admin/format_admission.rs:728-747`）——仅 T 历史 = 身份丢失错绑；T+静态
  混合 / 多 T 历史 = 重复 locator → `InvalidConflictHistory`。

**修案（本单）**：保存/摘要入口对「不支持的过渡状态」返回**显式结构化错误**
（新错误变体，与 `InvariantViolation` 族严格区分）；**保存前结构化拒绝**，不替换
身份、不静默丢弃历史；「归属≠身份」通道在持久面切断；世界拒绝后无状态污染、继续
可运行。完整格式（资源种类 + T 稳定边身份入档）按合同 §9 版本轴归 S5 #1200。

---

## 1. 现状调用链梳理（函数级）

### 1.1 保存链（capture → encode）

```
TrafficWorld::capture_snapshot                    admin/snapshot.rs:1192（&self，assert_usable）
└─ WorldState::capture_snapshot                   admin/snapshot.rs:604
   ├─ ConflictRead::persistence_view              kernel/conflict.rs:2745（reservation 权威视图）
   ├─ captured_route_at ×N                        admin/snapshot.rs:770（live 槽位序 → 局部 ID + 边 StableId）
   ├─ captured_vehicle_at ×N                      admin/snapshot.rs:821
   │  ├─ parking binding 捕获                     admin/snapshot.rs:839-892
   │  ├─ maneuver_traversal 捕获                  admin/snapshot.rs:893-936
   │  │    └─ ★P1-3 位点①：hop_gate[phase_hop].expect("traversal phase hop resolves Gate")
   │  │                                          admin/snapshot.rs:918-923
   │  ├─ waiting_membership 捕获                  admin/snapshot.rs:937-973
   │  ├─ conflict_eligibility 捕获                admin/snapshot.rs:979-1011
   │  │    └─ expect("eligible admission hop resolves Gate")   admin/snapshot.rs:994-999（防御性同型）
   │  └─ conflict_reservation 捕获                admin/snapshot.rs:1012-1148
   │     ├─ ★P1-3 位点②：hop_gate[admission_gate_hop].expect("reservation admission hop resolves Gate")
   │     │                                          admin/snapshot.rs:1024-1029
   │     ├─ ★P1-4 通道：逐 passage
   │     │    conflict_passage_occurrence_locator  admin/snapshot.rs:1052-1054
   │     │    （T 出现项 → 归属 (stream,zone) 写入 CapturedConflictPassage.passage）
   │     └─ downstream 区间重建与逐项比对          admin/snapshot.rs:1064-1135
   ├─ waiting_zones 捕获                          admin/snapshot.rs:699-722
   ├─ ★P1-4 通道：conflict lag 行                  admin/snapshot.rs:724-744
   │  ├─ ConflictRead::persisted_lag_rows         kernel/conflict.rs:2705-2723（含 T 格非 NoHistory 行）
   │  └─ StepReadView::conflict_passage_locator   kernel/world.rs:2983-3011
   │     （T 格 static_zone()=None → 经 stream passage 取归属 (stream,zone)；
   │      expect("installed conflict address resolves stable locator") admin/snapshot.rs:728-730）
   └─ CapturedSnapshot → encode_lfrs              admin/snapshot.rs:506 → admin/format_admission.rs:178
```

错误轴：`SnapshotCaptureError`（`ReservationFailed | ConflictInvariantViolation |
LimitExceeded`，`admin/snapshot.rs:30-41`）。捕获只读 `&self`，失败即世界无感知。

### 1.2 摘要链（直算；与捕获路径前像逐字节一致）

```
TrafficWorld::direct_state_digest                admin/snapshot_digest.rs:1081
└─ WorldState::direct_state_digest               admin/snapshot_digest.rs:718
   └─ catch_unwind(direct_state_preimage)         admin/snapshot_digest.rs:722-737
      ├─ panic → SnapshotDigestError::InvariantViolation(StateInvariantViolation::Panicked)
      │        ★P1-3 现状：expect 的 panic 被吞成 InvariantViolation——不出摘要但
      │        错误类误导（健康世界被报成不变量破坏，可能误触宿主停用处置）
      └─ direct_state_preimage                    admin/snapshot_digest.rs:740-1026
         ├─ captured_route_at / captured_vehicle_at（同 1.1 两个 ★P1-3 位点，
         │    经 direct_capture_error 映射 admin/snapshot_digest.rs:1043-1059）
         ├─ ★P1-4 通道：conflict lag 行            admin/snapshot_digest.rs:935-962
         │    （同一 conflict_passage_locator 归属派生 + 同型 expect 941-942）
         └─ canonical 前像 → StateDigest::of_preimage（BLAKE3-256，ADR 0033 内部完整性摘要）
```

另有 `deterministic_state_digest(&CapturedSnapshot)`（`admin/snapshot_digest.rs:198`）：
只归一化**已捕获**快照，捕获侧拒绝即天然受保护，无需改动。

错误轴：`SnapshotDigestError`（`ReservationFailed | LimitExceeded |
InvariantViolation(StateInvariantViolation)`，`admin/snapshot_digest.rs:36-46`）。

### 1.3 恢复链（restore）

```
restore_lfrs                                     admin/snapshot_restore.rs:468（公开入口）
└─ 语义 lowering（admin/format_admission.rs:583 起，顺序不可绕过，快照合同 §5）
   ├─ restore_vehicle（逐车；非 Active 先、Active 后）  admin/format_admission.rs:2144
   │  └─ traversal 恢复：phase_gate StableId → resolve_maneuver_anchor → phase hop
   │     admin/format_admission.rs:1868-1920（v7 只存 Gate 身份 → T Clearing 保存侧
   │     必然拒绝后，恢复侧天然不可达）
   └─ restore_conflict_aggregate                 admin/format_admission.rs:772-1265
      ├─ eligibility 恢复：decode_conflict_locator + occurrence 交叉核对
      │                                          admin/format_admission.rs:798-872
      ├─ reservation 恢复：admission_gate → hop 解析（918-937，无 Gate 即拒）；
      │  逐 passage decode_conflict_locator + occurrence 交叉核对（1041-1057）
      │  → ★P1-4：T 出现项行经 unique_address 重绑到**静态 Zone 格**（混合 hop 可达）
      │  restore_reservation + Clearing 重建      admin/format_admission.rs:1195-1226
      ├─ ★P1-4：conflict lag 行恢复               admin/format_admission.rs:1229-1259
      │  ├─ decode_conflict_locator               admin/format_admission.rs:728-747
      │  │    unique_address(zone, stream) → 静态 Zone 格（kernel/conflict.rs:2815-2836）
      │  │    + world.conflict_passage_locator(address) == Some(locator) 等值核对
      │  └─ locator 键严格递增检查（仅 T 历史 → 静默错绑；T+静态/多 T → 重复键
      │     → InvalidConflictHistory）
      └─ normalize_conflict_eligibility + conflict_state_valid  admin/format_admission.rs:1260-1263
```

### 1.4 通道旁证：跨修订迁移（同型归属派生，本单范围决策点见 §4.5）

- `cutover_migration.rs:2091-2118`：切换事务期望投影对**每个**目标格（含 T 格）
  调 `conflict_passage_locator` 生成 lag 行——同一归属≠身份通道的第三个写端；
- `cutover_migration.rs:1764-1769`：跨修订迁移给全部非继承目标格（T 格恒不继承，
  `mapped_conflict_address` 经 `source_address.static_zone()?` 在 1101 失败关闭）
  写 `CutoverFloor` → 迁移后 T 格必然带 lag 参照；
- `migration_journal.rs:240-245`：`VehicleDelta::from_state` 有同型
  `expect("Waiting phase hop resolves Gate")`（切换日志写端，T-`Clearing` 车辆触发）。

---

## 2. P1-3 修案：结构化拒绝替代 panic

### 2.1 错误类型形状

**原则**：显式结构化、与 `InvariantViolation` 族严格区分。理由（写入 rustdoc 契约）：
T 过渡态是**健康世界的正常状态**，不是 C(T) 失真；若并入 `InvariantViolation`
（或被 `catch_unwind` 吞成 `Panicked`），会误触宿主按 ADR 0032 的停用/隔离处置，
且违背 `SnapshotDigestError` 既有的「不变量破坏 = 比较点记 `Incomplete`」语义
（`admin/snapshot_digest.rs:30-46`）。

草案（命名可由 lead 调整；`Display` 中文、变体名机器身份，rust-code-style §5.3）：

```rust
/// 持久形制无法忠实表达的过渡状态种类（#1217；S5 #1200 完整格式交付前的边界拒绝）。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub enum UnsupportedTransitionState {
    /// 遍历/预约的准入 hop 属过渡资源出现项（无真实 Gate），v7 的 phase Gate 槽无法忠实表达。
    #[error("过渡准入 hop 无真实 Gate，当前持久形制无法忠实表达")]
    AdmissionHopWithoutGate,
    /// 持久化 passage/occurrence 行覆盖 `ConflictResource::Transition` 出现项。
    #[error("过渡资源出现项缺资源种类与稳定边身份，当前持久形制无法忠实表达")]
    TransitionPassageRow,
    /// `ConflictResource::Transition` 资源格携带非 `NoHistory` 清空历史。
    #[error("过渡资源清空历史缺资源种类与稳定边身份，当前持久形制无法忠实表达")]
    TransitionLagHistory,
}
```

- `SnapshotCaptureError` 新增变体 `UnsupportedTransitionState(UnsupportedTransitionState)`
  ——保持 `Clone, Copy`（payload 为 Copy 枚举）；
- `SnapshotDigestError` 新增**独立变体** `UnsupportedTransitionState(UnsupportedTransitionState)`
  ——**不**挂在 `InvariantViolation(StateInvariantViolation)` 下；
- `direct_capture_error`（`admin/snapshot_digest.rs:1043-1059`）1:1 映射新变体。

### 2.2 插入点与入口行为

**统一处理口径（两处 expect 及同型点共用一个解析器）**：判定键是
**覆盖该 hop 的出现项的资源种类**，不是「hop 能否解析出 Gate」——T 的
`admission_hop`（前驱边下标，`tables.rs:1216-1218`）在混合 hop 下可能**巧合解析到
真实 Gate**（那是别的机动的门，写进去就是身份错绑），必须按资源种类拒绝。

新增 crate-private 助手（不分配、纯读，`Result` 返回）：

```text
resolve_persisted_admission_gate(compiled, maneuver_occurrence_index, phase_hop)
  → Result<ManeuverGateOrdinal, UnsupportedTransitionState>
```

- 扫 `compiled.conflicts` 中 `maneuver_index == maneuver_occurrence_index &&
  admission_hop == phase_hop` 的出现项（不分配的迭代器扫描，冷路径）：
  - 只含 `ConflictResource::Zone` → 取 `compiled.hop_gate[phase_hop]`
    （编译期已校验 = 该 passage 的 admission gate，`tables.rs:1254-1261`）；
    解析失败属真不变量破坏，保留 `ConflictInvariantViolation` 语义（**不得**降级为
    新变体——两类错误的宿主处置不同）；
  - 含任何 `Transition`（含混合 hop）→ `AdmissionHopWithoutGate`（混合 hop 场景
    的变体归入 `TransitionPassageRow` 亦可，实施时择一并保持测试与文档一致）。

应用点（全部替换为结构化错误，路径上不再有 panic）：

| 位点 | 现状 | 改后 |
| --- | --- | --- |
| `admin/snapshot.rs:918-923`（traversal phase hop） | `expect("traversal phase hop resolves Gate")` | `resolve_persisted_admission_gate` → `UnsupportedTransitionState` |
| `admin/snapshot.rs:1024-1029`（reservation admission hop） | `expect("reservation admission hop resolves Gate")` | 同上，统一口径 |
| `admin/snapshot.rs:994-999`（eligibility admission hop） | `expect("eligible admission hop resolves Gate")` | 同口径防御（见 §6 决策点 D2：已核实 committed eligibility 只带真实 Gate hop，`conflict_eligibility_valid_with_signals` 在 `kernel/world.rs:3104-3111` 对无门 hop 返回 false 并清表；仍建议统一走 helper 免未来破防） |
| `admin/snapshot.rs:728-730`、`admin/snapshot_digest.rs:940-942`（lag 行 locator） | `expect("installed conflict address resolves stable locator")` | 先判 `address.resource()`：`Transition` → `TransitionLagHistory`；否则照常派生，派生失败归 `ConflictInvariantViolation` |

**`capture_snapshot` 入口行为**：整入口拒绝（fail closed，零部分产物）——发现任一
不支持过渡状态立即返回该错误，不产出 `CapturedSnapshot`；不做部分捕获、不跳行、
不改世界（`&self` 只读，游标不动）。`TrafficWorld::capture_snapshot`
（`admin/snapshot.rs:1192`）与 `WorldState::capture_snapshot`（604）的 `# Errors`
按 rust-code-style §5.2 更新（新增失败族）；`captured_vehicle_at` 的 `# Panics`
（817-820）同步改写为「过渡态不 panic，返回结构化错误；其余 live 状态引用前提不变」。

**`direct_state_digest` 入口行为**：同判定、同错误族（经 `direct_capture_error`
映射）。`catch_unwind`（722-737）保留为**纯 panic 兜底**（真不变量 unwind 仍报
`Panicked`），但支持域内的过渡态拒绝必须走 `Result` 直返、不得经 panic 转译——
测试断言错误变体 ≠ `InvariantViolation(_)`。

### 2.3 「世界继续可运行」的验证方式（测试断言，见 §5）

1. 拒绝后 `world.step(...)` 正常推进（换道继续、清空继续、历史照常落账）；
2. `command_cursor` / `event_cursor` / `tick` / `time_ms` 与拒绝前逐项相等（无状态污染）；
3. 同一状态下重试 `capture_snapshot` / `direct_state_digest` 得到**同一**结构化错误
   （确定性、可重试）；状态自然演化到「可表达」（本单内 T 相关态不会自愈——历史
   不消失，断言错误持续稳定）；
4. 非 T 世界的保存/摘要/恢复**字节零变化**（控制组，见 §5）。

---

## 3. P1-4 修案：身份保真优先，通道切断 + 保存前结构化拒绝

### 3.1 身份模型与「暂时不能忠实表达」的判定

**持久身份要求**（审计红线）：持久行必须保留**资源种类 + T 稳定边身份**。T 格的身份
是分类表派生的横向过渡段 `LaneEdge`（`kernel/conflict.rs:223-238`、
`ConflictPassageAddress` 的布局保持编码 `0x8000_0000 | edge.raw()`，239-347）；
`(participant_stream, conflict_zone)` 只是其**归属**（经 stream passage 派生），
本体≠身份（`kernel/world.rs:2990-2994` 注释已声明「不得串成静态 passage 身份」）。

**为什么本单不能忠实表达**：v7 wire 的冲突定位只有
`ConflictPassageLocatorBinding{ participant_stream, conflict_zone }`
（`CapturedConflictPassageLocator`，`admin/snapshot.rs:242-247`），traversal/
reservation/eligibility 的准入身份槽只有 `phase_gate` / `admission_gate`（Gate
`StableId128`，217-221 / 251-258）。给这些槽加「资源种类 + T 稳定边身份」= 改
wire schema = 合同 §9 的三轴版本升级（LFRS 7→8、runtime state 5→6、digest 8→9）
= **S5 #1200 的完整格式交付**，属本单非目标（且数据格式语义变更须同步 design 文档）。

因此「暂时不能忠实表达」的**判定条件（保存前结构化拒绝，整入口 fail closed）**：

| 判定 | 触发条件 | 错误变体 |
| --- | --- | --- |
| 拒绝 A | 任一车辆的 traversal/reservation/eligibility 的准入 hop 覆盖 `Transition` 出现项（含混合 hop） | `AdmissionHopWithoutGate` / `TransitionPassageRow` |
| 拒绝 B | 任一 reservation/eligibility 的持久 passage 行覆盖 `Transition` 出现项 | `TransitionPassageRow` |
| 拒绝 C | 任一 `ConflictResource::Transition` 资源格的 lag 参照 ≠ `NoHistory`（`persisted_lag_rows` 行） | `TransitionLagHistory` |

- **不替换身份**：任何路径不得再把归属 `(stream, zone)` 写入持久行；
- **不静默丢弃历史**：不得跳过 T 行继续出快照/摘要（两世界仅 T 历史不同必须可区分
  ——拒绝本身就是区分手段；静默丢弃会让二者摘要相等）；
- 拒绝 C 的覆盖含 `ActualClear` **与** `CutoverFloor`（两者都影响恢复后行为等价）。

### 3.2 「归属≠身份」通道切断

通道共有 **1 个派生源 + 3 个持久写端 + 1 个恢复读端**：

1. **派生源**：`StepReadView::conflict_passage_locator`（`kernel/world.rs:2983-3011`）
   及其包装（`WorldState`/`TrafficWorld::conflict_passage_locator`，
   `kernel/world.rs:2320-2325`、`478-485`）。
   **切断动作**：T 资源格（`static_zone() == None`）**返回 `None`**，删除「经
   stream passage 取归属 zone」的派生分支；文档合同改写为「T 资源格无忠实持久
   locator，返回 `None`；归属信息不得充当身份（#1217）」。公开 API 返回语义变化
   写入 rustdoc（`None` 语义扩展）。
   - 影响面核实（实施时逐一确认）：调用方仅 `admin/snapshot.rs:729`、
     `admin/snapshot_digest.rs:941`（本单改造）、`admin/format_admission.rs:744`
     （恢复等值核对，地址来自 `unique_address` → 恒为静态格，行为不变）、
     `admin/cutover_migration.rs:2102`（迁移期望投影 → fail closed，见 §4.5）。
   - **不动** occurrence 级派生 `conflict_passage_occurrence_locator`
     （`kernel/world.rs:3018-3048`）的共享实现——它被 `conflict_tick.rs:412` 等
     运行态热路径与 #1216 在途面使用（红线禁改）；改为在**持久写端**按资源种类
     fail closed（下条）。
2. **写端①**：`admin/snapshot.rs:724-744`（capture lag 行）——先判
   `address.resource()`，`Transition` → `TransitionLagHistory`；静态格照旧。
3. **写端②**：`admin/snapshot_digest.rs:935-962`（直算 lag 行）——同上，经
   `direct_capture_error`/新变体直返。
4. **写端③**：reservation/eligibility 的 passage 行
   （`admin/snapshot.rs:1045-1063`、`1000-1010`）——循环内已有
   `compiled.conflicts[idx].resource`（1048-1051），`Transition` → `TransitionPassageRow`。
5. **恢复读端**：`admin/format_admission.rs` ——
   - reservation 逐 passage（1025-1057）与 eligibility（798-872）：解析出的
     occurrence 资源种类为 `Transition` → `SnapshotRestoreError::InvalidConflictAuthority`
     （**可检测**面 fail closed；混合 hop 的错绑在此拦截）；
   - lag 行（1229-1259）：v7 wire **无法区分**外来 T 历史行与真实静态格行（行内
     无资源种类），恢复端不可检测——由保存端切断保证本版本 writer 不再产出此类
     行；旧构建/外来制品的残余风险登记（§4.4、findings.md）。

### 3.3 S5 版本轴衔接（不交付完整格式）

- 本单**不动**版本轴（LFRS 7 / runtime state 5 / digest 8），不改 wire、不改摘要
  前像字节（受支持域零变化，见 §5 红线）。
- S5 #1200 按合同 §9 / 快照合同 §3 衔接注记交付：车辆 tagged 过渡状态
  （`PreEntry | InTransition | Clearing`）+ 预约 wire 行（acquired tick、源/目标/
  过渡区边 `StableId128` + 整数毫米区间并集、route occurrence 证明）；冲突定位行
  升级为**资源种类 tagged 身份**（`Zone(ConflictZoneId) | Transition(LaneEdgeId)`，
  participant stream 降为归属注记或删除），三轴 LFRS 7→8 / state 5→6 / digest 8→9。
- 本单的错误变体即 S5 的**翻转点清单**：S5 落地时，§2.2 表中各插入点由「拒绝」翻
  转为「按新格式写行」，`UnsupportedTransitionState` 三变体逐项退役；findings.md
  记录精确位点。
- 恢复端 `unique_address(zone, stream)` 重绑定（`kernel/conflict.rs:2815-2836`）在
  S5 变为「按资源种类 + 稳定身份重绑」（world.rs:2994 注释所称「S5=unique_address
  重绑定域」）；本单保持其静态格语义不变。

### 3.4 恢复侧行为（本单可达面）

| 输入 | 期望 |
| --- | --- |
| 本单 writer 产出的快照 | 不含任何 T 相关行（保存侧已拒绝），恢复照旧 |
| 覆盖 T 出现项的 reservation/eligibility 行（可检测） | `InvalidConflictAuthority`（fail closed，新增断言） |
| 重复 locator 键（多 T 历史 / T+静态同键） | `InvalidConflictHistory`（严格递增检查现状保持，`format_admission.rs:1233-1239`） |
| 单条外来 T 历史行（旧 writer 产物） | **不可检测**（行内无资源种类）→ 残余风险，登记 findings.md 与 S5 迁移问题；不做猜测式拒绝（不得误伤真实静态格历史） |

### 3.5 跨修订迁移连带面（决策点 D3，见 §6）

`cutover_migration.rs:2102` 在派生源切断后对 T 格返回 `None` →
`CutoverError::ConflictRevalidationFailed`（迁移 fail closed）；叠加迁移给 T 格写
`CutoverFloor`（1764-1769）+ 拒绝 C，**跨修订迁移后的含 T 网络世界将不可保存**，
直到 S5 交付。这与冲突资源文档「静态面（安装/迁移/重绑定/恢复）遇 `Transition`
一律失败关闭」（`kernel/conflict.rs:229-231`）和合同 §9 首版限制同向，但会改变
「含 T 网络跨修订迁移今天能成功」的现状行为——需要 lead 裁定口径（推荐：fail
closed 并登记 G-E 更新；备选：迁移面暂缓、仅登记风险）。

---

## 4. 场景集测试设计

放置：`crates/laneflow-runtime/src/admin/tests/transition_persistence_boundary.rs`
（新模块，`#[cfg(test)]`；夹具复用 `kernel/tables.rs` 测试模块的
`stream_bearing_revision()` / `install()` 合成网络形态——含横向过渡段 T、真实
participant stream，`tables.rs:3620-3650`；实施时把夹具提取为 `#[cfg(test)]`
跨模块可复用助手或复制最小形态，不改生产代码）。全部在 `placement-fixtures` 下运行。

**真实资源身份与历史的断言方式**（贯穿全部场景，禁止只比摘要）：
- 用真身份查询仲裁器：`ConflictRead::lag_reference(ConflictPassageAddress::new_transition(edge, …))`
  vs `lag_reference(ConflictPassageAddress::new(zone, …))`，逐格断言参照值
  （`ActualClear(t)` / `CutoverFloor(t)` / `NoHistory`）与 tick/时间戳；
- `persisted_lag_rows()` 的行集 = 期望的真实行集（按 address 对齐），断言拒绝后
  **未被改写**（与拒绝前逐项相等）；
- 通道切断断言：`world.conflict_passage_locator(t_address) == None`；
- 保存/摘要入口的错误**变体**精确断言（`UnsupportedTransitionState(...)`，
  ≠ `InvariantViolation(_)` ≠ `Panicked`）；
- 恢复面断言用真身份重查恢复后世界的 `lag_reference` / reservation cells。

| # | 场景 | 构造（合成网络，换道车 route `[e0, T, e2, …]`） | 期望行为 | 核心断言 |
| --- | --- | --- | --- | --- |
| S1 | 活动过渡（持 T 预约、车在 T 内） | 换道车步进至 `route_edge_index == 1`（T）中途停拍 | `capture_snapshot` 与 `direct_state_digest` 均返回 `UnsupportedTransitionState`（traversal 分支先行 → `AdmissionHopWithoutGate`）；**不 panic** | 错误变体精确匹配；`step()` 继续推进；游标/tick 不动；重试同错；T 格与静态格参照不变 |
| S2 | 车尾清空（`Clearing`） | 继续步进至车头离 T（`route_edge_index >= 2`）但车尾未清（`rear < clearance`，车长 > 剩余） | 同 S1（traversal 位点① + reservation 位点②都走结构化拒绝；旧代码此处 panic = 回归锚） | 同 S1；断言拒绝来自结构化错误而非 `Panicked`（`catch_unwind` 不被触发） |
| S3 | 仅 T 历史 | 步进至车尾清空（预约释放，T 格落 `ActualClear`） | 保存/摘要返回 `TransitionLagHistory`（拒绝 C）；整入口无产物 | T 格（`new_transition(edge,…)`）历史 = `ActualClear(真实时间)`；全部静态格历史不变（多为 `NoHistory`）；`conflict_passage_locator(t_addr) == None`；世界继续步进、可 despawn |
| S4 | T + 静态 Zone 混合历史 | S3 后再制造一条静态格历史（普通机动清空，或测试直写 `restore_lag_reference` 于静态格） | 保存/摘要返回 `TransitionLagHistory`（整入口拒绝，静态行不得单独放出） | 两个真身份格各自历史独立保真（值与时间逐项相等）；`encode_lfrs` 不被调用（无产物）；旧代码此场景恢复端 = 重复 locator → `InvalidConflictHistory`（回归对照注记） |
| S5 | 多 T 历史 | 两条换道边（网络含两个横向过渡段或两世界分别制造后合并不可行 → 用双 T 网络夹具/两辆车跨两个 T）各落历史 | 保存/摘要返回 `TransitionLagHistory` | 两个 `ConflictResource::Transition(edge)` 各自历史**分别**保真（旧通道把它们坍缩成同一 `(stream0, zoneX)` 键）；逐格比对真身份 |
| S6 | 控制组（零变化红线） | 无 T 的静态冲突世界：eligibility → reservation → `Clearing` → `ActualClear` 全生命周期 | 保存/恢复/摘要**行为与字节零变化**：`encode_lfrs` 字节、`StateDigest` 与基线相等 | `capture_snapshot` 两次相等；直算摘要 = 捕获路径摘要逐字节；save→load exact oracle 照旧 |
| S7 | 恢复面拒绝（可检测项） | 手改 wire（沿 `admin/tests/format_admission.rs` 的 tamper 手法）：reservation/eligibility 行覆盖 T 出现项（混合 hop 可构造） | `InvalidConflictAuthority`（fail closed） | 零部分恢复；错误精确匹配；不写入任何格 |

补充断言（S1–S5 共用）：拒绝后 `capture_snapshot` 无任何分配泄漏要求（只读路径），
且 `command_cursor` 与拒绝前相等（同 `capture_is_deterministic_and_side_effect_free`
的既有口径，`admin/snapshot.rs:1284-1295`）。

---

## 5. 不变量与红线

1. **合同 §5.1/§5.3 语义不变**：预约四态生命周期、P5 无门 hop 遵守义务、车尾
   真实清空判据均不动；本单只封闭持久化边界，不动 `conflict_tick.rs` 的 P4 搜索
   与出现项查询（#1216 在途面，红线禁改）。
2. **digest/state 零变化红线（按新裁定调整口径：身份保真优先于零变化）**：
   - 受支持域（无 T 参与的状态）**必须**零变化：`encode_lfrs` 字节、摘要前像与
     `StateDigest` 与基线逐字节相等（S6 控制组 + 既有回归全绿）；
   - T 相关态以**结构化拒绝**取代旧的错误编码产物（panic → `Panicked`、错绑行 →
     伪摘要/伪快照）——此即「身份保真优先于零变化」的适用点；
   - **实施中若与既有测试的零变化断言冲突（例如既有摘要往返测试以 T 世界为输入
     并断言字节相等），立即停止并报 lead 裁定，不得自行弱化断言。**
3. **合同 §9 版本轴**：LFRS 7 / runtime state 5 / digest 8 本单不升；不改 wire
   schema、不改摘要前像布局；完整格式归 S5 #1200。
4. **新增分配遵守 fallible allocation（#532）**：本单预期**零新增缓冲**（判定为
   迭代器扫描与只读查询）；若实现中确需新 Vec/HashMap，capture 侧走
   `capture_try_reserve_exact`/`capture_map_try_reserve`，digest 侧走
   `digest_try_reserve_exact`/`NormalizationBudget`，迁移侧走 `try_staging_vec`。
5. **穷尽解构记账（rust-code-style §6）**：预期不新增 Core owning struct 的
   heap 字段（仅错误枚举与纯函数助手）；若实现中意外新增，同步 owner-local
   `cfg(test)` 穷尽解构计账与 smoke fixture，不得遗漏。
6. **注释只写契约（rust-code-style §5）**：新注释写不变量/失败语义/治理锚点
   （`#1217`、`#1199`、快照合同 §3/§5、合同 §9）；公开可失败 API 补 `# Errors`，
   有 panic 路径的补 `# Panics`（标题英文、正文中文）；不留无锚点 TODO。
7. **非目标**：完整持久化格式（S5 #1200）；`conflict_tick.rs` 的 P4 搜索/出现项
   查询两处（#1216 在途）；`ExecutionConfig`/容量语义；跨修订迁移策略设计
   （G-E 更新只登记不实现）。
8. **不留情面的诚实**：不 commit、不 push、不开 PR；验证结果（含未运行项与原因）
   如实报告。

---

## 6. 待 lead 裁定的决策点

- **D1 错误变体命名与粒度**：`UnsupportedTransitionState` 三变体
  （`AdmissionHopWithoutGate` / `TransitionPassageRow` / `TransitionLagHistory`）
  作为 `SnapshotCaptureError` / `SnapshotDigestError` 的新变体（保持 `Copy`）。
- **D2 eligibility 位点（`admin/snapshot.rs:994-999`）是否随两处 expect 同口径改
  造**：已核实 committed eligibility 只带真实 Gate hop（无门 hop 的 eligibility 在
  同 tick 校验中被清除，`kernel/world.rs:3104-3111`、`conflict_tick.rs:3661-3670`），
  该 expect 现状不可达 panic；建议仍统一走 helper（防御性、口径一致），成本极小。
- **D3 跨修订迁移连带口径**（§3.5）：推荐迁移面随派生源切断 fail closed
  （`ConflictRevalidationFailed`），并把「含 T 网络跨修订迁移后不可保存」登记为
  G-E 更新缺口；备选是本单不动迁移面（`cutover_migration.rs:2102` 保留归属派生），
  仅登记风险。此点会改变现有行为面，须 lead 拍板。
- **D4 `migration_journal.rs:240-245` 同型 expect**：切换日志写端对 T-`Clearing`
  车辆会 panic（P1-3 同类、第三入口）。合同 §9 首版限制已要求活动过渡/持预约世界
  拒绝跨修订切换，但该拒绝门是否已实现需核实；若未实现，建议本单顺手在切换入口
  加结构化拒绝（不动 `from_state` 签名）或明确登记遗留风险。

---

## 7. 实施序与记账点

1. **错误轴**：`UnsupportedTransitionState` 定义 + `SnapshotCaptureError` /
   `SnapshotDigestError` 新变体 + `direct_capture_error` 映射 + rustdoc
   `# Errors`/`# Panics` 更新（`admin/snapshot.rs`、`admin/snapshot_digest.rs`）。
2. **P1-3**：`resolve_persisted_admission_gate` 助手 + 替换 `snapshot.rs:918-923`、
   `1024-1029`（+ D2 批准则含 `994-999`）；`captured_vehicle_at` 与
   `capture_snapshot` 契约注释改写。
3. **P1-4 写端**：lag 行资源种类判定（`snapshot.rs:724-744`、
   `snapshot_digest.rs:935-962`）+ reservation/eligibility passage 行判定
   （`snapshot.rs:1045-1063`、`1000-1010`）。
4. **通道切断**：`conflict_passage_locator`（`kernel/world.rs:2983-3011` + 两层
   包装）对 T 返回 `None` + 契约注释改写；核对全部调用方（§3.2 列表）。
5. **恢复面**：`format_admission.rs` reservation/eligibility 的 occurrence 资源
   种类 fail closed（1025-1057、798-872）；lag 行注释注明残余风险指向。
6. **迁移面**：按 D3 裁定执行或登记。
7. **场景集测试**（§4 S1–S7）+ 夹具提取。
8. **findings.md**：差异记录（归属≠身份通道切断说明、逐位点改动、场景集结果、
   测试结果清单、遗留风险含 v7 外来 T 历史行不可检测、G-E 更新建议、S5 翻转点）。

**验证命令（阶段 2，Rust 1.99.1 钉版；无 cargo-nextest，CI nextest 一律以
`cargo test` 等价执行）**：

```text
cargo check --all-targets --locked --features placement-fixtures
cargo clippy -p laneflow-runtime --all-targets --locked --features placement-fixtures -- -D warnings
cargo test -p laneflow-runtime --locked --features placement-fixtures
```

改动波及 `format_admission`/`snapshot` 系持久化格式测试（均在 `laneflow-runtime`
crate 内），上列 `cargo test -p laneflow-runtime` 已覆盖；若迁移面（D3）改动落地，
`cutover_migration`/`cutover_transaction` 测试同在该 crate。若 grep 发现
`laneflow-urban-harness` 等 crate 消费 `capture_snapshot`/`direct_state_digest`
错误变体（穷尽 match），一并跑该 crate 测试。

**记账点清单**：fallible allocation（若新增缓冲，见红线 4）；穷尽解构（若新增
heap 字段，见红线 5）；错误枚举穷尽 match 的下游消费方（`direct_capture_error`、
宿主可见 `Display`；新增变体会使外部穷尽 match 编译失败——这正是期望的记账行为，
在 findings.md 记录触点）。
