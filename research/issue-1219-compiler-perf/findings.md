# Issue #1219 findings：现场核对与差异记录

基线 `main@d5f28317`。本文件记录方案/实现相对 issue #1219 描述与现场代码的差异、
实现取舍与验证结果。

## 1. 现场核对差异（相对 issue 描述）

1. **行号漂移**：issue 记 runtime `tables.rs:696-722` 的 `conflict_dispatch_hops`
   构造；实际位于 `crates/laneflow-runtime/src/kernel/tables.rs:754-780`
   （注释块 + `gate_hops.clone()` + `contains`/`push` + `sort_unstable`）。
   内容与 issue 描述一致，按实际位置修。
2. **typed 比较既有惯例**：`hir/junction.rs:887`（路径连通检查）已用
   `candidate.target == successor.target` typed 相等；①改为 typed key 与既有
   惯例一致，不引入新的身份口径。
3. **死代码**：`hir/junction.rs:772-773`（`let edge_stable = &edge.stable_key;
   let _ = edge_stable;`）无消费，随①同点清理。
4. **过时注释**：763-768、823-826 的"跨空间比较一律 stable_key"理由段与两处
   实际比较（同一 `HirUnit::lane_edges` 空间内比较）不符——正是 P2-5 的误判
   来源；按 rust-code-style §5.4 重写为契约注释（不留考古叙述）。
5. **oracle 报告确有 `implementation_commit` 字段**（`measurement/matrix.rs:228`、
   `workers.rs:102` 等多处已按该字段核验），measurement 入口是唯一漏检处，
   补齐后与其它入口同口径。
6. **Seam 合同出处**：`research/issue-1141-internal-diagnosis/plan.md` §P3
   （31 行）明确 `(s,t) ∈ [0,1)×(0,1]`、`s=0`/`t=1` 为该侧整 cubic 单弦边界
   情形单列记录；grid.rs 的统一 `[0,1)` 断言与合同不符。

## 2. 实现差异与取舍

### ① P2-5（hir/junction.rs）

- 两处内层比较统一 `member.target == successor.target`；成员索引键同步 typed 化。
- **附加反例测试**：issue 只列 owner 侧（A::exit vs B::exit）；第二处判定
  （引用侧 `InternalReferencedBySuccessor`）存在对称误放行面，同批补对称反例
  （B 路径成员 B::internal 同名掩护 A::internal）。理由：两处同缺陷同修法，
  只钉一处会留下回归缺口。
- 拒绝面单调性论证见 plan.md §1.2：旧命中 ⇒ 新命中或同名不同记录（新拒绝），
  不存在旧拒新放。

### ② P2-6（hir/plan.rs + hir/junction.rs + runtime tables.rs）

- **布局取舍结论：CSR（typed key + 连续布局）**，三方案对比见 plan.md §2.1：
  按出现项预留的 HashMap 桶在共享长前缀时显著过量，typed HashMap 不解决过量
  且保留哈希/字符串成本；排序出现项省常数但查询降为 O(log M)。CSR 精确定长
  3 块、查询 O(1)。
- 预算新增三笔（`junction_budget` scratch）：starts `(N+1)×usize`、构造游标
  `N×usize`、path_keys `M×size_of::<HirManeuverPathKey>`（M 按
  `maneuver_path_edges` 上界入账，越界成员跳过时只高估——宁可高估方向）。
  旧实现整块漏账（HashMap 桶/键/Vec/查询临时 String 均未入账）。
- runtime `conflict_dispatch_hops`：`gate_hops.clone()` + `push` 增长改为
  `try_route_vec(gate_hops.len() + hop_count)` 一次预留 + `extend_from_slice` +
  受守卫的 `push`；并集内容与排序逐值不变；改动收敛在构造块内（避 #1218）。

### ④ 工具层

- measurement runner：oracle 预检抽为纯函数 `check_oracle`（对齐
  `workers::check_report` 既有测试形态），新增
  `oracle.implementation_commit == implementation`；"只改 oracle SHA"负测试 +
  逐字段破坏负测试。
- grid.rs Seam：按侧分开校验（左 `s∈[0,1)`、右 `t∈(0,1]`）；四端点测试
  （s=0 合法、s=1 非法、t=0 非法、t=1 合法）。`witness_pair` 与网格并集语义
  不动——s=0/t=1 的见证弦恰为 `[0,1]` 单弦边界形态，与 §P3"单列记录"一致。

## 3. 验证记录（targeted + 四项交付清单全量）

- 反例真伪钉证：临时回退两处 `member.target == successor.target` 为旧字符串比较后
  重跑——两个跨模块反例均 **FAILED**（旧代码 `Ok`，fail-open 实证），恢复 typed
  比较后全部通过。拒绝面收紧为实证结论，非推断。
- `cargo test -p laneflow-compiler hir::`：34 passed（含四个新增：
  owner 侧/引用侧跨模块反例、`junction_scratch_budget_is_exact_at_the_limit_boundary`、
  `hir::plan::tests::junction_budget_accounts_member_index_layout`）。
- `cargo test -p laneflow-lust-converter grid::`：15 passed（含 Seam 四端点：
  s=0 合法、s=1 拒、t=0 拒、t=1 合法）；`seam` 过滤 10 passed。
- `cargo test -p laneflow-synth-harness measurement::`：10 passed（含
  `oracle_identity_rejects_only_changed_implementation_commit`）。
- `cargo test -p laneflow-runtime kernel::tables`：57 passed（1 ignored 为既有
  #1199 WIP 钉，与本次无关）；`allocation_failure` failpoint 家族 11 passed
  （含 `route_compilation_allocation_failure_leaves_world_unchanged`、
  `route_allocation_failure_aborts_without_partial_cutover`）。
- `cargo fmt --check`：clean。
- `cargo clippy -p laneflow-compiler -p laneflow-runtime -p laneflow-synth-harness
  -p laneflow-lust-converter --all-targets -- -D warnings`：**clean**（exit 0）。
- `cargo run -p xtask -- check-runtime-architecture`：**通过**（显式模块、格式接口、
  默认/全特性生产依赖规则）。
- 回退实验后已恢复 typed 比较并重跑 `cargo test -p laneflow-compiler hir::`：
  34 passed（恢复态确认）。
- **全量交付清单已跑（2026-11-03，lead 绿灯后；工作树 `research/1219-compiler-perf`）**：

| 检查 | 结果 |
| --- | --- |
| `cargo run -p xtask -- check-runtime-architecture` | ✅ exit 0 |
| `cargo fmt --check` | ✅ exit 0 |
| `cargo clippy -p laneflow-compiler -p laneflow-runtime -p laneflow-synth-harness -p laneflow-lust-converter --all-targets --locked -- -D warnings` | ✅ exit 0 |
| `cargo test -p laneflow-compiler --locked` | ✅ exit 0 |
| `cargo test -p laneflow-runtime --locked --features placement-fixtures` | ✅ exit 0 |
| `cargo test -p laneflow-synth-harness --locked` | ✅ exit 0 |
| `cargo test -p laneflow-lust-converter --locked` | ✅ exit 0 |

全批合计 **1810 passed / 0 failed / 120 ignored**（67 个测试结果行，含单元、集成与
doctest）；ignored 均为既有 release/manual 夹具（如 `waiting_lookup_release_ab`、
S0F/LUST 外部目录依赖），与本单改动无关。

### ②记账结论定稿（scratch 分类为正确分类）

CSR 三块（`member_starts`、构造游标、`member_path_keys`）均为**校验期暂存**：
只在 `build_junction_hir` 内部使用，不进入 `JunctionHir` 输出，retained live=0
（与 `internal_claims`/`boundary_claims` 同类）。因此三笔全入
`junction_budget.scratch_bytes`（互斥工作集轴）是**正确分类**，不是兜底：

- 构造后显式 `drop(member_cursor)`，检查后显式 `drop(member_starts)`/
  `drop(member_path_keys)`，峰值收紧到"starts+path_keys"而非三块同时存活
  （同 `drop(sequence_index)` 惯例）；
- starts/游标为精确入账（`(N+1)×usize` + `N×usize`）；路径键按
  `maneuver_path_edges` 上界入账，越界成员跳过时只高估（方向安全）；
- "刚好够/差一单位"边界测试钉住该口径。

契约注记（代码站点同注，#1219）：**若未来该索引保留进输出，必须迁入 live 轴重
记账**。

## 4. 遗留风险

- runtime tables.rs 与 #1218 在途分支潜在文本冲突（改动 ~20 行，语义锚定
  "并集内容不变"）。
- ③性能归因两对象仅出方案，未实施；计数器侵入面待实施前复核。
