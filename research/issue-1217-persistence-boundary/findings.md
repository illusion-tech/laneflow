# Issue #1217 差异记录：换道持久化边界封闭（审计 P1-3/P1-4）

**状态**：实现完成（阶段 2；未 commit/push/PR，改动留工作树）<br>
**基线**：`main@45263887`（worktree `research/1217-persistence-boundary`）<br>
**方案**：[`plan.md`](./plan.md)（lead 已批准，D1–D4 裁定见 §4）

---

## 1. 变更总览

| 面 | 变更 |
| --- | --- |
| 错误轴 | 新增 `UnsupportedTransitionState`（三变体，`kernel/conflict.rs`——与 `ConflictResource::Transition` 同域的 kernel 域概念，admin 持久化边界反向消费；公开路径 `crate::UnsupportedTransitionState` 不变），作为 `SnapshotCaptureError` 与 `SnapshotDigestError` 的**独立新变体**（区分于 `InvariantViolation` 族）；`CutoverError` 同名变体承载迁移域拒绝 |
| P1-3 | `persisted_admission_gate` 统一解析器替换 `snapshot.rs` 三处准入位点 `expect`（923/1029/994-999）；`catch_unwind` 只兜真 unwind，过渡拒绝走 `Result` 直返 |
| P1-4 | 归属≠身份派生源切断（`conflict_passage_locator` 对 T 返回 `None`）；三个持久写端按资源种类整入口拒绝；恢复端按资源种类 fail closed；迁移域入口 fail closed（D3） |
| D4 | `VehicleDelta::journalable` 入口判据 + 全部 5 个 `from_state` 调用点入口级拒绝（迁移入口结构化错误 / 日志粘性终止） |
| 测试 | 场景集 `admin/tests/transition_persistence_boundary.rs`（6 用例，见 §5） |

## 2. P1-3：结构化拒绝替代 panic

- **错误形状**（D1 归口裁定落实）：`UnsupportedTransitionState::{AdmissionHopWithoutGate, TransitionPassageRow, TransitionLagHistory}`；**按检查位点语义归口**——准入 hop 解析位点（traversal/reservation/eligibility，含混合 hop）一律 `AdmissionHopWithoutGate`；持久 passage/occurrence 行 → `TransitionPassageRow`；lag 行 → `TransitionLagHistory`。混合 hop 的理由写入类型与解析器注释：错绑风险来自准入身份槽本身（混合 hop 可能巧合解析到其他机动的 Gate），与 S5 翻转点一一对应。
- **统一解析器**：`persisted_admission_gate`（`admin/snapshot.rs`）→ 委托 `CompiledRoute::admission_hop_transition_covered`（`kernel/tables.rs`，#1217 归属≠身份统一判定的单一事实源）。判定键是**覆盖该 (maneuver occurrence, 准入 hop) 的出现项资源种类**，不是「hop 能否解析出 Gate」。只覆盖静态出现项而 hop 无 Gate = 编译期已排除的真不变量破坏，保留 `ConflictInvariantViolation`（宿主处置不同，未混同）。
- **入口行为**：`capture_snapshot` 与 `direct_state_digest` 对不支持过渡状态**整入口拒绝、零部分产物**；`direct_capture_error` 1:1 映射；`catch_unwind` 保留为纯 panic 兜底（测试断言错误变体 ≠ `InvariantViolation`/`Panicked`，证明未经 panic 转译）。
- **世界继续可运行**：捕获/摘要均为 `&self` 只读；场景测试逐拍断言 tick/双游标不变、`step()` 照常推进、重试同错、历史不被改写。
- **`# Errors`/`# Panics` 契约更新**：`WorldState::capture_snapshot`、`TrafficWorld::capture_snapshot`、`captured_vehicle_at`、`WorldState::direct_state_digest`、`TrafficWorld::direct_state_digest`、`SnapshotDigestError` 枚举文档。

## 3. P1-4：归属≠身份通道切断

**通道解剖**（1 派生源 + 3 持久写端 + 1 恢复读端 + 迁移面旁证）：

1. **派生源（已切断）**：`StepReadView::conflict_passage_locator`（`kernel/world.rs`）对过渡资源格（`static_zone() == None`）**返回 `None`**，删除「经 stream passage 取归属 zone」分支；契约注释改写为「T 格无忠实持久 locator；归属信息不得充当身份（#1217）」。`WorldState`/`TrafficWorld` 两层包装同步文档（公开 API `None` 语义扩展）。
2. **写端①**：`admin/snapshot.rs` capture lag 行——先判 `address.resource()`，`Transition` → `TransitionLagHistory`；静态格照旧派生。
3. **写端②**：`admin/snapshot_digest.rs` 直算 lag 行——同口径，错误独立于 `InvariantViolation`。
4. **写端③**：reservation/eligibility 的 passage 行（`admin/snapshot.rs`）——`compiled.conflicts[idx].resource` 为 `Transition` → `TransitionPassageRow`。
5. **恢复读端（可检测面 fail closed）**：`admin/format_admission.rs`——eligibility 与 reservation 逐 passage 先按出现项资源种类拦截（`Transition` → `InvalidConflictAuthority`），再进 locator 交叉核对（混合 hop 下归属派生对恰与静态格重合，必须按资源种类拦截）。
6. **迁移面（D3）**：`cutover_migration.rs`——(a) 目标 T 格的 `CutoverFloor` 写入前拒绝（`TransitionLagHistory`，准备期入口）；(b) 期望投影 lag 行同口径兜底；(c) `VehicleDelta` 入口判据拒绝 T 遍历增量（`AdmissionHopWithoutGate`）。`conflict_passage_locator` 切断后 `cutover_migration.rs` 的 target locator 派生对 T 恒 `None`，旧的归属行通道同步失效。

**保真口径**：不替换身份（不得以归属 `(stream, zone)` 冒充静态 passage 身份）、不静默丢弃历史（跳行会让仅 T 历史两世界摘要相等）；v7 表达不了即**保存前结构化拒绝**。完整格式（资源种类 tagged 身份 `Zone(ConflictZoneId) | Transition(LaneEdgeId)`、LFRS 7→8 / state 5→6 / digest 8→9）按合同 §9 版本轴归 S5 #1200。

## 4. D1–D4 裁定落实

- **D1**：三变体 + 错误轴形状照批；归口按检查位点语义定死（§2）。
- **D2**：eligibility 位点统一走 `persisted_admission_gate`；**且构造出了触发测试**（`d2_eligibility_on_transition_occurrence_rejects_at_admission_slot`：构造态 eligibility 指向 T 出现项 → 钉住归口 `AdmissionHopWithoutGate`）。unreachable 论证：committed eligibility 只带真实 Gate hop（无门 hop 的 eligibility 在同 tick 校验中被 `conflict_eligibility_valid_with_signals` 清除，`kernel/world.rs` 站位谓词对无门 hop 返回 false），故生产态触不到；构造态测试钉统一口径。
- **D3**：fail-closed 落地（§3 第 6 条）。回归测试 `d3_cross_revision_migration_of_t_cell_network_fails_structurally_without_residue` 钉住：含 T 格网络跨修订迁移 → `CutoverError::UnsupportedTransitionState(TransitionLagHistory)`、源世界状态逐项相等、照常步进（无部分状态残留）。**行为变化 + G-E 缺口登记见 §7.2**；#1217 issue 留言草稿见 §7.4。
- **D4**：**可达**（证据链）：`VehicleDelta::from_state` 的 phase Gate `expect` 在 5 个生产调用点可达——
  1. `kernel/tick.rs` 武装期 TICK 记录（日志武装时逐变化车辆，正常运行 T-Clearing 车触发——先于冲突格投影到达）；
  2. `kernel/world.rs` replace 命令（替换可携带 T-`Clearing` 遍历）；
  3. `kernel/parking.rs` `record_parking_update`（T-Clearing 车的停车绑定更新）；
  4. `kernel/world.rs` spawn / 5. parked spawn（当前构造下 traversal 恒可编码，防御性同口径）。
  处理：新增 `VehicleDelta::journalable` 入口判据（与 `from_state` 前置条件同界 + 资源种类拦截），5 个调用点全部入口级结构化拒绝——迁移入口（`cutover_migration::migrate_structural_clone`）返回 `CutoverError::UnsupportedTransitionState`；tick/命令路径（提交段无错误出口）粘性终止日志（`MigrationDeltaJournal::refuse_unsupported_transition_state`），事务 `check_health` 映射为 `CutoverError::UnsupportedTransitionState` 并整体放弃。`from_state` 签名未动；原裸 `expect` 改为前置断言（`# Panics` 契约注明入口保证 + 逐调用点守卫），**不再是唯一防线**。若 lead 要求物理消除该 `expect`，需把 `from_state` 改为 fallible（D4 明令不动签名），建议作为 S5 跟进项。

## 5. 场景集结果（真实资源身份与历史比对）

| # | 场景 | 期望 | 结果 |
| --- | --- | --- | --- |
| S1/S2 | 活动过渡 + 车尾清空（`Clearing`），**真实 tick 生命周期**（80 拍驱动换道车全程） | 持 T 权威窗口：capture/digest = `AdmissionHopWithoutGate`（独立变体，非 `Panicked`）；拒绝零状态污染 | ✅ `s1_s2_s3_real_tick_transition_lifecycle_rejects_structurally`（入 T 前 capture/digest 逐字节一致为控制点；每拍 tick/双游标不变） |
| S3 | 仅 T 历史（预约释放、车已离开 T） | capture = `TransitionLagHistory`；T 格真身份历史 = `ActualClear(真实清空时间 ≤ 世界时钟)`，拒绝不改写；`conflict_passage_locator(T) == None`；世界继续步进、拒绝持续稳定 | ✅ 同上用例后半程 |
| S4 | T + 静态 Zone 混合历史 | 整入口拒绝（静态行不得单独放出）；T 格与静态格**各自**历史保真（1_000 / 2_000 逐项相等）；通道切断断言 | ✅ `s4_mixed_t_and_static_history_rejects_whole_entry_with_distinct_identity` |
| S5 | 多 T 历史 | 两 T 格身份互不覆盖、历史各自保真、`persisted_lag_rows` 两行俱在（旧通道坍缩同一键） | ✅ `s5_multi_t_history_keeps_distinct_resource_identity`（仲裁器级；世界级载体缺口见 §7.3） |
| S6 | 控制组（零变化红线） | 无 T 静态冲突世界：捕获/编码确定性、直算与捕获摘要逐字节一致 | ✅ `s6_static_conflict_persistence_control_zero_change`（全量套件 = 字节零变化域的最终判据） |
| D2 | eligibility 指向 T 出现项 | 归口 `AdmissionHopWithoutGate` | ✅ |
| D3 | 含 T 格网络跨修订迁移 | 结构化失败 + 无部分状态残留 | ✅ |
| S7（计划中追加） | 恢复面手工 wire 的过渡行拒绝 | — | ⏸ 未构造（§7.3：防御面 + 不可达论证） |

断言面遵守红线：全部以**真身份**（`ConflictPassageAddress::new_transition/new`、`lag_reference`、`persisted_lag_rows`、`conflict_reservation` 的出现项资源种类）逐项比对，不以同一套错误编码的摘要互相印证。

## 6. 验证结果

本地交付清单已**固定为四项**（#1217 流程教训固化——CI 专属门禁曾两次咬人：先 `cargo fmt --check`、后 `xtask check-runtime-architecture`）：`cargo run -p xtask -- check-runtime-architecture`、`cargo fmt --check`、`cargo clippy -p laneflow-runtime --all-targets --locked --features placement-fixtures -- -D warnings`、`cargo test -p laneflow-runtime --locked --features placement-fixtures`。**四项全过才允许 push**。精确结果见文末「验证补记」。

## 7. 遗留风险与缺口

### 7.1 外来/旧构建 v7 制品的 T 历史行不可检测（残余风险）

v7 wire 的 lag 行没有资源种类字段，旧构建 writer 产出的 T 历史行（错绑为 `(stream 0, 归属 zone)`）与真实静态格行**在恢复端不可区分**。本版本 writer 由保存侧切断保证不再产出此类行；对旧制品不做猜测式拒绝（不得误伤真实静态格历史）。处置归 S5 #1200：完整格式带资源种类后提供迁移/拒绝策略。已在 `decode_conflict_locator` 注释登记。

### 7.2 D3 行为变化登记（G-E 更新缺口）

- **行为变化**：含 T 资源格网络的**跨修订迁移**（含从未使用 T 的网络）由「成功但产出错误身份行」改为 `CutoverError::UnsupportedTransitionState` 结构化失败；含 T 状态/历史的世界同时不可保存/摘要（S5 前的诚实边界）。同修订恢复/换根同样受保存侧拒绝约束（其 quiescent capture 走同一入口）。
- **G-E 更新建议**：合同 §9/G-E 的「活动过渡或持预约世界拒绝跨修订切换」需扩展登记为「含过渡资源格网络跨修订迁移/持久化在 S5 完整格式交付前一律结构化拒绝」，S5 #1200 承接放开条件（资源种类 + T 稳定边身份入档 + 逐修订映射）。
- **S5 翻转点清单**（届时由拒绝翻转为按新格式写行）：`admin/snapshot.rs` `persisted_admission_gate` 三调用点 + lag 行 + passage 行；`admin/snapshot_digest.rs` lag 行；`kernel/world.rs` `conflict_passage_locator` T 分支；`admin/format_admission.rs` 两处资源种类拦截 + `decode_conflict_locator`；`admin/cutover_migration.rs` floor 写入前拦截 + 期望投影；`admin/migration_journal.rs` `VehicleDelta::journalable`/`from_state` 相位门；`admin/cutover_transaction.rs` `check_health` 映射。错误变体三枚届时逐项退役。

### 7.3 测试构造缺口（不造假测试）

- **世界级多 T 场景**：样本网络（`sample/minimal-closed-loop.json` v5 派生，含 T01/T0a/T0b 多段过渡链）无 `RightOfWayPolicySet` 声明，`WorldPolicyBinding::install` 对带门/冲突/流网络拒绝 `NotRequired`，无法安装世界；`stream_bearing` 夹具恰一过渡段（钉版 R1–R3 载体，不可扩改）。S5 因此在仲裁器级钉多格身份不互覆；世界级拒绝口径与 S4 共用同一代码路径（任一 T 行即整入口拒绝）。建议 S5 #1200 为多 T 世界夹具补策略声明后升格为世界级用例。
- **恢复面过渡行检查（防御面）**：`format_admission` 的两处资源种类拦截（eligibility/reservation passage）触发需「同一 wire 内手工构造覆盖 T 出现项的行且先通过前置合法性检查」——现有夹具无法单点构造（T 预约行在准入门解析处已被既有拒绝面拦截，混合 hop 无现成夹具）。留待 S5 完整格式的 crafted-wire 负例族一并覆盖；本单以保存侧切断 + 真身份回归兜底。

### 7.4 #1217 issue 留言草稿（D3(c) + 红线教训，供 lead 同步）

> 持久化边界封闭（P1-3/P1-4）实现记录一条行为变化：含过渡资源格（横向过渡段）网络的跨修订迁移现在**结构化拒绝**（`CutoverError::UnsupportedTransitionState`），不再产出归属对错绑的 lag/出现项行（原行为属 P1-4 同类身份错绑）；同状态世界在 S5 完整格式交付前不可保存/摘要（`SnapshotCaptureError::SnapshotDigestError::UnsupportedTransitionState`，世界照常可运行）。放开条件与翻转点归 S5 #1200：持久行携带**资源种类 + T 稳定边身份**（LFRS 7→8 / runtime state 5→6 / digest 8→9，合同 §9 版本轴）。G-E 建议同步更新为「含 T 资源格网络跨修订迁移/持久化在完整格式前一律拒绝」。
>
> 实现教训一条（供后续切片）：**pub 结构的 `Debug` 形状即摘要面**——`MigrationJournalStats` 等进入等值轨迹摘要冻结 fixture（`phase-protocol-e011745e.txt`）的结构，新增可观测字段前先查冻结 fixture 覆盖；本单曾在 `MigrationJournalStats` 加观测字段、被冻结摘要拦下，经裁定改为只经结构化错误路径观测，未动任何断言与 fixture。

### 7.5 其他登记

- `migration_journal.rs` `WaitingMembershipReleaseDelta::from_state` 的 `expect("Waiting release hop resolves Gate")` 属等待域（等待区 release hop 由注册期编译校验为真实 Gate，且 `Clearing` 与 waiting membership 互斥），不涉过渡资源；未改动，论证如上。
- 迁移日志的 `ConflictOccurrenceJournalLocator.zone` 以 `resource_order_raw` 编码（高位标记区分 `Transition`，含 edge raw）——**迁移日志域的冲突出现项编码保留了资源种类与 T 边身份**（与 v7 快照相反）；其消费侧 LFSD 重绑对 T 的语义由 S5 核对，本单未动（D3 已使 T 网络走不到该路径）。
- 错误枚举新增变体会使宿主/下游的穷尽 `match` 编译失败——这是期望的记账行为；`cargo check --all-targets` 已确认工作区内无受影响消费方。

---

## 8. 红线事件记录（零变化断言冲突，已停下报裁）

- **事件**：全量测试触发 3 个冻结摘要失败（`phase_equivalence::exact_baseline_trace_and_retry_match` / `occupancy_candidate_preserves_exact_trace_and_first_error_priority` / `parallel_worker_matrix_trace_matches_fixed_fixture`），按红线「零变化断言冲突立即停下报 lead」停工上报。
- **诊断**：三测试把 `world.migration_journal_stats()` 的 `{:?}` Debug 形状并入等值轨迹摘要，与冻结 fixture `tests/fixtures/phase-protocol-e011745e.txt`（pre-refactor 锚定，**不得由被测实现重生成**）逐字比对。失败模式精确吻合根因：仅**日志武装**场景（waiting-journal / waiting-overflow / conflict-journal / signals-clock）子摘要变化、非武装场景不变——冲突 100% 来自实现中自选的 `MigrationJournalStats.unsupported_transition_state` 可观测字段（D4 粘性拒绝的观测面），与 D1–D4 行为面**零冲突**（冻结场景均无 T 状态；迁移日志字节、摘要前像、capture/digest 字节全部未动）。另 12 个 `waiting::tests`/`execution::tests` 失败为 `RESOURCE_TEST_LOCK` 毒化连锁（上述意外 panic 持锁所致），非独立缺陷。
- **裁定（lead）**：**方案 B**——删除该字段，`MigrationJournalStats` Debug 形状逐字复原（以冻结 fixture 通过为证，不新增形状断言）；粘性拒绝仅经 `MigrationDeltaJournal::unsupported_transition_state()` + `check_health` → `CutoverError::UnsupportedTransitionState` 观测（结构化错误即观测面）。方案 C（手写 Debug 隐字段）不取：字段本身不必要，"加了再藏"同时背冗余字段与 Debug 不诚实两项代价。
- **教训（写入 #1217 差异评论）**：**pub 结构的 `Debug` 形状即摘要面**——给进入等值轨迹摘要的 pub 结构新增可观测字段前，先查冻结 fixture 覆盖；确需改动冻结 fixture/摘要前像时再次停下报 lead，走独立的 fixture 再生成治理（附正当理由记录），绝不静默重生成。实现中已在 `stats()` 契约注释固化该形状禁令（锚 #1217）。

## 验证补记（Rust 1.99.1 钉版；CI nextest 以 `cargo test` 等价执行）

**本地交付清单（固定四项，四项全过才允许 push）**：

```text
cargo run -p xtask -- check-runtime-architecture
cargo fmt --check
cargo clippy -p laneflow-runtime --all-targets --locked --features placement-fixtures -- -D warnings
cargo test -p laneflow-runtime --locked --features placement-fixtures
```

| 检查 | 结果 |
| --- | --- |
| `cargo run -p xtask -- check-runtime-architecture` | ✅ 通过（「显式模块、格式接口与默认/全特性生产依赖规则通过」；本轮修复：`UnsupportedTransitionState` 由 `admin/snapshot.rs` 移入 `kernel/conflict.rs`——kernel 层不得依赖 admin 层，该类型是与 `ConflictResource::Transition` 同域的 kernel 域概念，admin 反向消费合法；公开路径 `crate::UnsupportedTransitionState` 不变） |
| `cargo fmt --check` | ✅ 无差异 |
| `cargo clippy -p laneflow-runtime --all-targets --locked --features placement-fixtures -- -D warnings` | ✅ 零告警 |
| `cargo test -p laneflow-runtime --locked --features placement-fixtures` | ✅ 全绿（exit 0）：lib `675 passed / 0 failed / 22 ignored`（含 6 个新增场景集）；集成与 evidence 目标全部通过（`compiled_networks` 121、`tick` 18、`lifecycle` 14、`audit_replay` 10、`snapshot_replay` 2、`parallel_preview_equivalence` 7 等）；doc-test 1 通过 |
| `cargo check --all-targets --locked --features placement-fixtures` | ✅ 通过（补充项：含全部测试目标与下游 crate：urban-harness/bevy/scenario 等） |

**CI flake 归因（勿动代码）**：`integrity_soak::tests::safe_pause_resumes_the_resident_world_and_excludes_paused_time` 抛
`Validation("无法测得看护或存活驻留进程的工作集（Incomplete）")`——工作集采样瞬时失败（Evidence fixture tests）。归因：本分支本地跑该用例 **1 passed（7.46s）**，同 run 的 Urban harness tests 全绿，本 diff 不触碰任何采样面；等新 push 触发重跑即可，不改代码。

**红线事件恢复证据（§8 的 16 个失败逐一确认恢复，方案 B 落地后）**：

| 失败（方案 B 前） | 恢复 |
| --- | --- |
| `tick::phase_equivalence::exact_baseline_trace_and_retry_match` | ✅ `ok`（冻结 fixture `phase-protocol-e011745e.txt` 原样通过，未重生成） |
| `tick::phase_equivalence::occupancy_candidate_preserves_exact_trace_and_first_error_priority` | ✅ `ok` |
| `tick::phase_equivalence::parallel_worker_matrix_trace_matches_fixed_fixture` | ✅ `ok` |
| `execution::tests::try_dispatch_error_keeps_done_prefix_and_pending_suffix_within_chunk` | ✅ `ok`（`RESOURCE_TEST_LOCK` 毒化解除） |
| `waiting::tests::{motion_dispatch_natural_thread_ids_match_stats, parallel_non_entry_decisions_match_serial_and_fall_back_on_cache_miss, parking_arrival_injection_requires_growth_per_step, phase_combination_matrix_matches_across_all_arms, preview_error_before_identity_failure_keeps_serial_first_error, preview_first_error_is_stable_across_workers_and_positions, preview_input_reserve_failure_falls_back_to_fused, preview_panic_invalidates_world_and_rejects_queries, preview_slot_reserve_failure_falls_back_to_fused, preview_workset_shapes_match_fused_reference, reserve_probe_records_site_reachability_and_growth, sparse_basis_payloads_merge_canonically_and_reuse_attempt_buffers}`（12 项） | ✅ 逐名 `ok`（定向重跑 `waiting::tests::` 族 73 passed / 0 failed） |

另注：`tests/spawn_overlap_allocation.rs` 运行输出中的两条 `assertion ... warm overlap queries allocated/reallocated` 文本是 `allocation_guard_rejects_real_heap_operations` 的**预期负例 panic**（`catch_unwind` 断言 `is_err()` 验证配额守卫），目标整体 `2 passed / 0 failed`，与本单无关。
