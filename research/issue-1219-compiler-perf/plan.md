# Issue #1219 方案稿：审计 P2×2 编译器 + 性能双对象 + 工具层两项

- 基线：`main@d5f28317`（worktree `research/1219-compiler-perf`，干净）。
- 本阶段范围：方案稿 + 轻量 P2 修复（①②④）；③ 只出测量方案，实施等 lead 绿灯与
  #1218 矩阵让出算力。重构建避让：cargo 构建轻、短，长测试批次延后。
- 红线：①拒绝面只准不松；②预算不得低估（宁可高估入账）；注释只写契约
  （rust-code-style §5）；不 commit、不 push、不开 PR。

## 0. 现场核对（与 issue 描述的差异，详见 findings.md）

- ②的 `conflict_dispatch_hops` 实际位于 `crates/laneflow-runtime/src/kernel/tables.rs`
  **757-780**（issue 记 696-722，行号漂移，内容一致：`gate_hops.clone()` +
  `contains`/`push` + `sort_unstable`）。
- `hir/junction.rs:887` 的路径连通检查已用 `candidate.target == successor.target`
  typed 比较——typed 比较在同一 arena 内是既有惯例，①的修法与之一致。
- `hir/junction.rs:772-773` 有死代码（`let edge_stable = &edge.stable_key; let _ = edge_stable;`），
  随①同点清理，不另开重构。

## 1. ① P2-5 跨模块身份比较（junction.rs 两处内层判断）

### 1.1 现状与缺陷

`build_junction_hir` 的两处内层成员判断只比较 `stable_key` 字符串：

- 判定 1（约 777-788，`InternalHasSuccessors` 的 owner 侧）：找 internal 边的
  "路径外后继"时，`lane_edges.get(member.target).stable_key.to_string() ==
  lane_edges.get(successor.target).stable_key.to_string()`；
- 判定 2（约 827-839，`InternalReferencedBySuccessor` 的引用侧）：判断引用边所在
  候选路径是否也含目标 internal 边时，同样只比较 `stable_key`。

`stable_key` 仅模块内唯一（`HirLaneEdge::stable_key` 注释即如此定义）。跨模块同名边
（如 A::exit 与 B::exit，stable_key 均为 "exit"）会被误判为同一实体：

- 反例：A 模块路径 `entry → internal → exit`，为 A::internal 声明后继 B::exit。
  判定 1 中 B::exit 与成员 A::exit 字符串相同 → 被当成"路径内" → 不拒绝
  （fail-open：路径外后继被放行）。
- 判定 2 对称存在同类误放行（候选路径含同名不同模块成员即误判 same-path）。

### 1.2 修法

两处内层比较统一改为 **typed key 相等**：`member.target == successor.target`。

- `member.target`（`HirManeuverPathEdge.target`）与 `successor.target`
  （`HirLaneEdgeReference.target`）都是 `HirLaneEdgeKey`，指向**同一**
  `HirUnit::lane_edges` arena（base.rs 注释即声明"当前 `HirUnit::lane_edges`
  中的目标键"）。同一 arena 内 typed key 相等 ⇔ 同一记录 ⇔ 同一实体；
  `(module, stable_key)` 由 Typed AST 去重保证唯一，故字符串比较命中的同名
  跨模块边恰是 typed 比较要拒绝的面。
- 判定口径收紧（拒绝面变准、不变松）：旧比较命中 ⇒ 要么同一记录（新旧同判
  合法），要么同名不同记录（旧放行、新拒绝）。不存在旧拒新放的输入。
- 后继 `find` 顺序、"显式 internal owner 跳过第二轮检查"的分支语义
  （约 806-818，`owner_is_explicit_internal` → `continue`）保持不变；只改成员
  身份判定。
- 同点把成员索引的键从 `(u32, String)` 改为 typed 边键（见 ②），消除查询期
  `String` 临时分配；注释重写为契约：同空间 typed key 即实体身份，跨空间比较
  才走 `stable_id`（删除 772-773 死代码与"跨空间一律 stable_key"的过时理由段）。

### 1.3 测试设计（crates/laneflow-compiler/src/hir/tests.rs）

1. **owner 侧反例（issue 指定）**：
   `explicit_junction_internal_edge_rejects_cross_module_same_name_successor`。
   双模块夹具：A（`city/junction-cross`，import base）含 entry/internal/exit 与
   路径 `entry→internal→exit`、junction 显式声明 approach=[entry,exit]、
   internal=[internal]；A::internal 后继 = **imported base "exit"**（B::exit）。
   期望：`build_hir` 失败且含 `JunctionEdgeSetMismatch { edge_key: "internal",
   violation: InternalHasSuccessors }`。旧代码下该夹具 `Ok`（A::exit 字符串掩护
   B::exit），故本测试即钉住反例。
2. **引用侧反例（第二处判定对称钉）**：
   `explicit_junction_internal_edge_rejects_cross_module_same_name_inbound_reference`。
   A 同上但 A::internal 后继为空；B（import A）含 x/"internal"/exit-b 与合成
   路径 `x→internal→exit-b`，且 x 的后继含 **A::internal**（跨模块入向引用）。
   期望：`InternalReferencedBySuccessor`。旧代码下 B 路径成员 B::internal（同名
   "internal"）掩护 A::internal，误判 same-path 放行。
3. **回归面**：既有 `explicit_junction_internal_edge_*` 四例
   （无后继/路径自身后继/路径外后继/路径外入向引用）全保持——同模块合法面不收紧。

## 2. ② P2-6 预算补账 + runtime fallible allocation

### 2.1 member_index 布局取舍结论（按此实现）

候选三方案（查询：按边取"含该边路径键集"；构建：全部路径成员出现项 M、车道边 N）：

| 方案 | 分配 | 查询 | 共享长前缀时的预留 |
| --- | --- | --- | --- |
| 现状 `HashMap<(u32,String), Vec<Key>>`，`with_capacity(M)` | M 桶 × (String+Vec) + 每键 String + 每 Vec 增长余量 | 哈希 + String 构造 | **显著过量**：M 桶按出现项预留，而不同边数 ≤ N，路径共享边时 M ≫ N |
| typed key + **连续布局（CSR）**：`starts: [usize; N+1]` + 构造游标 `[usize; N]` + `path_keys: [Key; M]` | 精确 3 块，(2N+1)×usize + M×4B | O(1) 区间取值 | 精确：每出现项 4 B + 每边 16 B，无桶、无字符串 |
| typed key + 排序出现项 `(edge, Key)[M]` + 二分 | 1 块 M×8B | O(log M) | 精确，但查询二分、下标不连续 |

**结论：采用 CSR（typed key + 连续布局）**。理由：
- 预算可精确入账（3 个定长分配，无哈希桶 8× 保守上界、无 String/Vec 增长余量），
  "宁可高估"只需按 `counts.maneuver_path_edges` 计 M 上界；
- 查询 O(1) 且无哈希/字符串构造——第二轮检查在每个 (边, 后继) 对上查询，热路径；
- N 项 (2N+1)×usize 与同函数既有的 `internal_claims`/`boundary_claims` 等 O(N)
  数组同量级，不是新增量级风险；
- 排序出现项省一半常数但换 O(log M) 查询与间接寻址，收益不值复杂度；HashMap
  typed 化（方案 0.5）不解决按 M 预留的过量，也保留哈希成本。

### 2.2 预算补账（hir/plan.rs `junction_budget`，与①同批）

`scratch_bytes` 增加（成员索引 CSR 三块 + 构造 scratch）：

```text
requested_bytes::<usize>(lane_edge_count + 1)          // member_starts
requested_bytes::<usize>(lane_edge_count)              // member_cursor（构造 scratch）
requested_bytes::<HirManeuverPathKey>(maneuver_path_edges)  // path_keys，M 上界（越界成员被跳过，只高估）
```

现状该索引整块漏账（HashMap 桶/键/Vec 与查询临时 String 均未入账）；改 CSR 后
临时分配归零、结构分配全部入账。`sequence_index`（`ManeuverPathSequence` 哈希表）
等既有条目不动。`persistent_bytes`/`lookup_bytes` 不受影响（索引是互斥 scratch）。

### 2.3 runtime `conflict_dispatch_hops` 纳回 fallible allocation（tables.rs 757-780）

现状：`gate_hops.clone()`（隐式分配，绕开 `try_reserve_route_exact` 及其 failpoint）
+ `contains`/`push`（push 可再增长，同绕约定）。修法（**收敛在该构造块内**，避开
#1218 在途面）：

```rust
let mut conflict_dispatch_hops = try_route_vec(gate_hops.len().saturating_add(hop_count))?;
conflict_dispatch_hops.extend_from_slice(&gate_hops);
// ... 含过渡出现项的 hop 仍按原规则 push（容量上界 = gate_hops + hop_count，不再增长）
conflict_dispatch_hops.sort_unstable();
```

- 语义不变：并集内容与排序结果逐值相同；每 hop 至多入列一次的 `contains` 守卫保留。
- `try_route_vec` 预留上界覆盖全部可能 push（hop 数上界），构造期零增长 ⇒ 全程
  受 `try_reserve_route_exact` 约束（含测试 failpoint 轴）。
- 不抽新函数、不动 `refresh_hops`/`compile_*` 其他分配（那些各有受检路径）。

### 2.4 测试设计

1. **预算公式钉（hir/plan.rs tests）**：
   `junction_budget_accounts_member_index_layout`——同计数下 `maneuver_path_edges`
   +1 ⇒ scratch 增量 == `size_of::<HirManeuverPathKey>()`；`lane_edge_count` +1 ⇒
   增量 == `2 × size_of::<usize>()`（starts+cursor）。锁"不得低估"的边际口径。
2. **刚好够／差一单位（hir/tests.rs）**：
   `junction_scratch_budget_is_exact_at_the_limit_boundary`——显式 junction 夹具，
   `HirBuildPlan::analyze` 取 `memory.stage_scratch_bytes`：
   - limit == 计划值 ⇒ `build_hir` 成功（刚好够）；
   - limit == 计划值 − 1 ⇒ `CompileLimitExceeded { StageScratchBytes, limit:
     计划值−1, observed: 计划值 }`（差一单位）。
   与 `portable_fixture_tests.rs` 的 LIR 边界口径一致。
3. **runtime**：既有 route 注册分配失败测试（occupancy/cutover/facade 的
   `with_route_allocation_failure_after` 系列）覆盖 failpoint 语义；构造块改后
   全量回归由 lead 绿灯后的 `cargo test -p laneflow-runtime` 承接（本阶段只跑
   targeted：`kernel::tables`、`kernel::occupancy`）。

## 3. ④ 工具层两项（与①②同批，改动小）

### 3.1 #1170 测量入口：oracle 实现提交核验（tools/laneflow-synth-harness/src/measurement/runner.rs）

- 现状 251-257 的 `require` 校验 `complete`、`input_manifest`、
  `ordered_events_digest`，**未核对 `oracle.implementation_commit == implementation`**
  ——换实现跑旧 oracle 可静默通过（manifest 虽记 `oracle_report_sha256`，但不拒绝）。
- 修法：把 oracle 预检收敛为纯函数 `check_oracle(oracle, input_manifest, implementation)
  -> Result<(), Error>`（含新条件 `oracle["implementation_commit"] == implementation`），
  `run_inner` 改为调用；错误文案同步（"oracle 未完成、缺少事件、实现提交或输入
  身份不符"）。纯函数化对齐 `workers::check_report` 既有测试形态。
- 测试（runner.rs `mod tests`）：
  1. 合法 oracle 通过；
  2. **"只改 oracle SHA"负测试**：其余字段不动、仅 `implementation_commit` 换成
     另一 40 位提交 ⇒ 必须拒绝；
  3. 顺带钉 `complete`/`input_manifest`/`ordered_events_digest` 各字段单独破坏
     亦拒绝（与 workers 的 `receipts_reject_*` 同形态）。

### 3.2 #1207 Seam 注入参数域（tools/laneflow-lust-converter/src/output/geom/s1/grid.rs 154-173）

- 现状：`Seam` 分支对左右参数统一 `assert!((0.0..1.0).contains(&at))`（即
  `[0,1)`）。合同（research/issue-1141-internal-diagnosis/plan.md §P3）：接缝对
  `[s,1]│[0,t]` 自由域 **`(s,t) ∈ [0,1)×(0,1]`**，`s=0`/`t=1` 是该侧整 cubic
  单弦边界情形（合法）。现状后果：合法 `t=1` panic、非法 `t=0` 通过。
- 修法：按侧分开校验——左（`matched == control`，参数 `s`）：`s ∈ [0,1)`；
  右（`matched == right_control`，参数 `t`）：`t ∈ (0,1]`。NaN 两式都拒绝。
  网格并集构造（∪{s}/∪{t}、排序去重保端点）与 `witness_pair` 下标语义不动：
  s=0 ⇒ 左见证弦 `[0,1]`、t=1 ⇒ 右见证弦 `[0,1]`，恰为单弦边界记录形态。
- 测试（grid.rs 既有内联 `#[test]` 区）四端点：
  1. `s=0` 合法：左网格含 0.0、保端点、见证弦 `[0,1]`；
  2. `s=1` 非法：`#[should_panic]`；
  3. `t=0` 非法：`#[should_panic]`；
  4. `t=1` 合法：右网格含 1.0、保端点、见证弦 `[0,1]`。

## 4. ③ 性能归因（两独立对象，方案先行）

两对象分开交付、分开结论，互不代替；沿用 #1170（research/issue-1166-lf-synth-measurement）
测量口径：整数纳秒、nearest-rank 百分位逐轮计算再取三轮中位、max 取三轮最坏；
分量不相加为整帧；W4 不套 W1 预算、不从两样本生成 p95/p99；lifecycle 区间含
命令执行与校验、非 exclusive CPU。

### 4.1 对象 a：lifecycle 秒级突发拆分（W4）

- 目标事实：#1170 W4 十万 33 ms 每轮 burst max 中位 3503.3509 ms、最坏
  4295.3141 ms；每帧 2000 条 observation 命令所在的 lifecycle 实测区间占帧
  98.98%–99.48%。现只有整段区间，无内部归因。
- 拆分维度（互斥、合计≈lifecycle 区间）：命令执行 / 准入检查 / 索引维护 /
  实体映射 / 校验成本（validate_population、checkpoint 对比）/ 其余杂项（残差单列）。
- 测量口径：lifecycle 区间内分段计时（`Instant` → 整数纳秒），逐帧输出
  `{segment, ns, commands, alloc}`；分段和与区间总长对账（守恒断言，残差
  显式列示，不静默吸收）。与 #1170 相同的帧分类（lifecycle_burst 单列）。
- 场景矩阵：`100000-W4-33-p10` × 3 fresh-process rounds（主）+
  `10000-W4-16-p100` × 3（对照，验证拆分口径在小突发同样守恒）。每轮 B0/B1 两帧。
- 指标采集点（复用 #1170 入口）：`measurement/runner.rs` 的 W4 帧 bracket、
  `checkpoint`/`state_digest` 校验、`resources::start/finish`（allocation 诊断
  binary）、manifest 记账（分段表进 `frames.json` 同级新字段，不改公共 API）。
- 预算与运行成本：6 次子进程（≈每 10 分钟内，W4 窗口极短：预热 + 2 quantum 帧，
  成本在准备与证据）+ oracle 准备；allocation 诊断 2 次单列。合计 ≈ 1.5–2 机时
  小时、单 worker 串行、与 #1218 矩阵不并发。新计数零依赖、零 API 面。

### 4.2 对象 b：S4 新成本量化（#1216 修复后实现）

- 目标事实：S4 过渡执行路径的成本自变量未量化；已知移交点必须可归因：
  `entrant_neighborhood` 的 `labels.iter().position` 与 `first_ring.contains`
  线性扫描、`entrant_stop_reach_mm` 每次申请全 profile 遍历。
- 指标（逐 tick/逐操作计数 + 纳秒）：过渡密度（过渡出现项/边数）、候选密度
  （候选数/过渡出现项）、路线长度 × 访问边数、唯一车辆数、路线查询次数、
  分配次数/字节（allocation feature）、以及三个已知移交点各自的调用次数与
  累计扫描长度（labels 扫描、first_ring 扫描、profile 遍历步数）——使任何
  尾值都能归因到具体线性扫描或全遍历。
- 测量口径：注册期（route 编译）与 tick 期（conflict dispatch、过渡邻域）分开；
  计数与时间分别记账，不用时间反推次数。输入自变量（密度三元组）随 manifest
  冻结，输出成本模型按自变量回归列示，不外推未测规模。
- 场景矩阵：过渡密度 3 档（低/中/高，合成输入 generator 参数化）× 规模
  （一万/十万）× 表现比例（10%/100%）= 12 配置 × 3 rounds；每配置先跑语义
  oracle（复用 #1170 `prepare-oracles` 口径）再跑计数/计时。
- 指标采集点：`laneflow-runtime` 注册期计数（route 查询、分配 failpoint 轴同源）
  + `measurement/runner.rs` manifest 记账；新增计数仅测试/测量 feature 可见，
  不进公共 API、不改运行快路径（计数器每 tick 聚合，不做每事件原子操作）。
- 预算与运行成本：12 配置 × 3 rounds + oracle ≈ 40 次子进程，十万行单次
  数分钟级，合计 ≤ 6 机时小时；分两批（先 4 配置冒烟校准计数口径，再全量）。

### 4.3 阶段安排

对象 a/b 的 harness 改动与跑数均等 lead 绿灯 + #1218 矩阵结束另行安排；本阶段
只冻结口径、场景与采集点，防止实施期口径漂移。

## 5. 验证计划

- 本阶段（轻量）：`cargo fmt --check`；targeted 测试
  （laneflow-compiler `hir::`、laneflow-synth-harness `measurement`、
  laneflow-lust-converter `grid`、laneflow-runtime `kernel::tables`）；
  `cargo run -p xtask -- check-runtime-architecture`。
- lead 绿灯后长批次：四项交付清单全量（`cargo clippy --all-targets -D warnings`
  触及 crate、`cargo test` 触及 crate）。未跑项在报告中如实列出。
- 不 commit / 不 push / 不开 PR；差异记录进 findings.md。

## 6. 风险与遗留

1. `runtime tables.rs` 与 #1218 在途分支潜在小冲突：改动收敛在
   `conflict_dispatch_hops` 构造块（约 20 行），冲突面最小；若 #1218 先合，
   以语义（并集内容不变）为准做文本 rebase。
2. ①收紧判定可能暴露此前被误放行的真实输入（拒绝面变准的预期后果）；无
   既有 fixture 应命中（既有测试均为单模块）。
3. ②预算按 M=`maneuver_path_edges` 上界入账，越界成员跳过时实际更小——方向
   安全（宁可高估）。
4. ③两对象的计数器设计以"零运行快路径开销"为约束，若实施期发现必须侵入
   tick 热路径，先回报 lead 再动。
