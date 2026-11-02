# Issue #1218 方案稿：R1b 常规化 + F1/F2/F3 换道接入

**状态**：阶段 1（R1b 常规化 + 本方案稿）已完成；阶段 2（F 系列接入实现）待 lead 批准
**基线**：`main@d5f28317`（worktree `research/1218-r1b-f-matrix`，#1216/#1217 修复已合入）
**红线**：R1a/R1b/R2/R3 断言不弱化；F 系列既有比较谓词不放松；比对真实资源身份与历史，
不得只比错误编码摘要（#1217 口径）；YAML 变更仅限 `evidence.yml` 过滤器条目。

---

## 0. 阶段 1a 已完成（R1b 常规化，判决依据：lead 显式执行 1 passed）

| # | 变更 | 文件 | 状态 |
| --- | --- | --- | --- |
| 1 | 移除 `r1b_s12_pinned_two_changers_exactly_one_live_order` 的 `#[ignore]`，**R 断言零改动** | `crates/laneflow-runtime/src/kernel/tables.rs`（R1b 用例 doc 区） | ✅ |
| 2 | doc 注释改写：删除「暂时忽略/待通」叙事，如实记录「2026-11-03 显式执行通过于 `main@d5f28317`（#1216/#1217 合入后，未做归因实验）；#1218 移除 `#[ignore]` 入常规回归，断言不变」 | 同上 | ✅ |
| 3 | Evidence 工作流 `-E` 过滤器追加 `test(r1b_s12_pinned_two_changers_exactly_one_live_order)`（仅过滤器条目，其余 YAML 未动） | `.github/workflows/evidence.yml` | ✅（语义注记见 §10-D5） |
| 验证 | `cargo test --locked -p laneflow-runtime --features placement-fixtures --lib r1b_s12_pinned_two_changers_exactly_one_live_order -- --nocapture` → **1 passed / 0 ignored**（不再需要 `--ignored`） | — | ✅ |

---

## 1. 现状梳理（F 系列三矩阵 as-is）

三矩阵共用「生成器正式四文件输入」这一夹具入口：`tools/laneflow-synth-generator`
产出 `network.lfca / state.json / input-plan.json / manifest.json` 目录
（身份 `10000-W1-16-a` … `100000-W4-33-a`），消费端由
`tools/laneflow-synth-harness/src/install.rs` `Sample::new`（LFCA 校验 →
`build_shared_network_revision` → `TrafficWorld::install`（policy pin）→ 逐路线
`register_route`（每路线唯一信号 Gate）→ active/parked 双路径 `spawn_*` → 初始状态逐项核对）
安装成真实 `TrafficWorld`。W1/W2 只调用 `step` + 只读查询；W3/W4 走受检命令计划
（停车/生命周期命令，`commands.bin` 收据 + `batches.json` 批次）。窗口口径：
预热 14508 拍（16 ms）/ 7044 拍（33 ms），W4 观察窗 2 拍、其余 2×预热。

### 1.1 F1 跨 worker 确定性（`worker-matrix`）

- 入口：`worker-matrix <root> <commit> <environment.json> <out>`，逐运行
  `worker-run <input> <commit> <workers> <out>`；`ExecutionConfig` 显式传入
  （1/2/4/8/16 worker，不降级）。
- 矩阵：8 身份（2 规模步长组合 × W1–W4）× 5 worker × 3 全新进程轮 = **120 次**；
  每身份以第一轮单 worker 为参考，其余 14 次逐一对照 = **112 对**；每轮配置序左移 7；
  ≤4 并发、运行中配置 worker 之和 ≤32；首错停止新启动、不补跑拼接。
- 比较谓词（`src/workers.rs`）：
  - `check_report`（84–146 行）：身份/`input_manifest`/`complete`/format/
    `implementation_commit`/实际 `workers`/窗口（`warm_up_ticks`、`observation_ticks`、
    `validated_boundaries`）+ 固定 tick 检查点必须携带 64-hex `state_digest`；
  - `compare`（148–173 行）：回执 JSON **仅排除 `workers`** 后全字段精确相等
    （#1184：worker 是执行配置）；`ticks.bin`、`events.bin` 逐字节相等（W3/W4 另加
    `commands.bin`、`batches.json`）。
- 逐拍公开观测（`src/observe.rs`，`lf-synth-public-boundary-1` 帧）：tick/时间/游标/
  人口 + 逐车 8 字段（rank、route_rank、`route_edge_index`、progress、carry、speed、
  status、changed）+ 事件计数；伴随强宿主语义检查（身份/停车绑定/精确整数运动/防重叠
  超车/信号因果/GateCrossed+ManeuverTraversalCompleted 成对事件）。`ticks.bin` 为逐拍
  BLAKE3 摘要。

### 1.2 F2 失败重试（`worker-retry-matrix`）

- 入口：`worker-retry-matrix <runtime-test-binary> <root> <commit> <environment.json> <out>`；
  先以普通二进制生成 4 个完整单 worker 正常参考（一万/16 ms、十万/33 ms × W3/W4），
  再调用 optimized Runtime 测试二进制执行
  `kernel::lf_synth_contract::formal_retry::formal_input_failed_step_retry`
  （`src/retry.rs:12` 常量；驱动源 `crates/laneflow-runtime/src/kernel/tests/lf_synth_contract/formal_retry.rs`，
  `#[ignore]` + `LF_SYNTH_INPUT/OUTPUT/IMPLEMENTATION/WORKERS` 环境变量）。
- 矩阵：4 身份 × 5 worker × 3 轮 = **60 次**失败重试；比较 **56 对**。每进程安装
  失败/fresh 两世界（预算按 worker×2 计）、完整预热 + 前两拍 guardrail；注入点固定
  `StepFailpoint::AfterTransitions`（`kernel/tick.rs:620`，测试专用 thread-local）→
  `StepError::ParkingObservationAllocFailed`；失败零发布、只重试原 `TickInput`。
- 比较谓词（`src/worker_retry.rs` + `src/retry.rs`）：
  - `verify_reports`：与同源正常参考核对命令批次、step 输入、失败边界、游标、
    重试/最终检查点（`state_digest` 与正常参考对应检查点相等）；
  - `check_local_files`（144–157 行）：`post-command.lfrs == failed.lfrs ==
    fresh-post-command.lfrs`（失败零发布）、`retry.lfrs == fresh.lfrs`，共 **7 份规范
    `.lfrs` 快照**逐字节比较；
  - `check_observations`（58–142 行）：`lf-synth-retry-observations-1` 边界观测——
    11 字段事件行（**种类 1 = GateCrossed、8 = ManeuverTraversalCompleted，其他种类拒绝**）、
    2 字段信号行、outcome（tick/时间/有序 parking arrivals）、失败边界 outcome 必须为
    null（不得伪造成功）；
  - `compare`（159–180 行）：回执仅排除 `workers` 后精确相等 + 8 文件（7×lfrs +
    observations.json）逐字节相等。

### 1.3 F3 新进程恢复（`restore-matrix`）

- 入口：`restore-reference / restore-run / restore-probe / restore-matrix`
  （`src/burst/restore.rs`、`src/restore_matrix.rs`、`src/burst/restore/state.rs`）。
- 矩阵：8 个完整单 worker 参考 × 每参考 2 检查点 = 16 检查点 × 5 worker × 3 轮 =
  **240 次**全新进程恢复。检查点位置（`restore.rs:217-224` `positions`）：
  W1/W2 = 预热结束 `W` 与 `2W`；W3 = `W+1` 第一批 atomic replacement 命令前/后；
  W4 = `W`（B0 命令前）与 `W+1`（B0 成功后）。
- 恢复流程（`state.rs:264-481` `restore`）：重验四文件与 **exact LFCA**（宿主资产准入
  与原输入逐字节一致，`state.rs:279-282`）→ 重建修订 → `attest_published_shared_network_revision`
  必须 `Pass` → `restore_lfrs` → `live_order` 重建宿主身份映射（车辆/路线完整唯一、
  路线边稳定标识序列核对）→ `projection == host.projection`（规范状态摘要、时钟、双游标）。
- 比较谓词（`restore.rs:411-539`）：`ticks.bin`/`events.bin`/`commands.bin` 按宿主记录的
  **精确字节偏移取完整后缀**逐字节相等（`suffix_equal`，不按共同前缀判等）；
  `batches.json` 批次后缀、检查点后缀（含 `state_digest`）、`final.json` 末态相等
  （`state::projection`：canonical 摘要 + 逐车投影 + 宿主累计记录）。

### 1.4 共同持久化触点（#1217 交互面，关键）

F 系列的「状态面」全部落在 `capture_snapshot` + `deterministic_state_digest` 上：

- F1 `check_report` 检查点 `state_digest`（`lib.rs:99-110` `checkpoint`：`capture_snapshot`
  → `deterministic_state_digest`）；
- F2 七份 `.lfrs` 快照 + 重试/最终检查点 `state_digest`（`formal_retry.rs` 同源捕获）；
- F3 检查点 LFRS（`state.rs:128-235` `capture`）+ 末态 `projection`（`state.rs:51-118`）。

而 **#1217 已封闭的持久化边界**（`research/issue-1217-persistence-boundary/findings.md`）：
含过渡资源格（T）状态或历史的世界，`capture_snapshot` 与 `direct_state_digest`
**整入口结构化拒绝**（`UnsupportedTransitionState::{AdmissionHopWithoutGate,
TransitionPassageRow, TransitionLagHistory}`，`kernel/conflict.rs:254`），零部分产物、
世界照常可运行。拒绝窗口 = **首个 T 权威瞬间起、永久**（权威 = T `Clearing` 遍历或
覆盖 T 出现项的 committed 预约；释放后 lag 历史行仍落账 → S3「仅 T 历史」同样被拒）。
**结论：任何含 T 穿越的换道场景，其穿越之后的全部状态面（快照/摘要/末态投影）在
S5 #1200 完整格式交付前均不可捕获——这不是缺陷，是 #1217 裁定的预期行为。**

---

## 2. 换道标称场景：来源与身份（D-S 决策点，推荐 S1）

**推荐 S1：扩展 `laneflow-synth-generator`，新增换道 case 族 `L1`**，产出同布局
四文件输入（身份 `10000-L1-16` / `100000-L1-33`）：

- LFCA 发射**恰一横向过渡段 T**（分类判据走注册期表驱动矩阵，
  合同 §6.5；来源保真、不冒充 SUMO connection）+ 流绑定 + `RightOfWayPolicySet`
  声明（#1217 §7.3 教训：无策略声明的 T 网络无法安装世界）；单过渡段与 R1b 钉版
  载体口径一致（`stream_bearing` 夹具「恰一过渡段、不可扩改」，不复用不扩改）；
- 场景语义 = **R1b s12 钉版换道竞争**（`docs/design/lane-change-minimal-contract.md` §7）：
  两换道者（距 T 入口 19 m/27 m、10 m/s、`d_stop ≈ 15.61 m` 前提成立双方均可让行）
  + 标称背景交通；钉版保证：层① tick 0 稳定序恰一优先不建预约、层② 前车 crossing
  恰一建立预约、败者 no-grant 停让、`rear >= clearance` 后进入、逐拍 T 独占恰一；
- 窗口与拍号由生成器钉版冻结（probe-vectors 扩展）：**首 crossing 拍必须晚于两个
  恢复切面（§6），终点必须晚于败者清空进入拍**；窗口为 L 族自有数值（不沿用
  14508/7044 预热口径），具体拍号阶段 2 试跑钉定。
- 比较面要求的 T 预约/清空字段（§4）由公开只读查询供给（§7）。

备选 S2（运行时夹具载体 + 测试二进制驱动）**不推荐**：R 夹具在
`#[cfg(test)]` 内（`kernel/tables.rs` `stream_bearing_revision`，恰一过渡段、明文
不可扩改），F1/F3 的 harness 二进制路径消费的是生成器四文件入口；S2 需另造
harness 入口/夹具 bundle 格式并复制编译管线，改动面反而更大。

R1a（s6 换道对直行让行）与 R3（s8 长车尾清空）场景形状**不进本轮 F 矩阵**
（其验收保留在 `tables.rs` 钉版用例）；换道族只以 R1b 语义开矩阵，边界声明见 §9。

---

## 3. 持久化拒绝面协议（F3 与 #1217 拒绝语义的交互——本方案关键设计点）

换道族在三矩阵中统一采用**双面协议**，显式编码 #1217 边界：

- **面 A（可捕获面，T-free 时刻）**：沿用既有规范快照/摘要**逐字节**比较，谓词零放松。
  T-free 判定 = 该拍前世界从未出现 T 权威（无 T `Clearing` 遍历、无覆盖 T 出现项的
  committed 预约、无 T lag 历史）。
- **面 B（拒绝面，T 权威/历史时刻）**：保存/摘要**必须**结构化拒绝（`UnsupportedTransitionState`
  变体），断言三件事合成相等：
  1. **拒绝变体**相等（`AdmissionHopWithoutGate` / `TransitionPassageRow` /
     `TransitionLagHistory` 逐拍按预期归口）；
  2. **真实资源身份与历史投影**逐字节相等（§7 定义的 `t_state` 投影：资源种类 +
     过渡段稳定身份 + 预约生命周期 acquired/entered/cleared/released 拍号 + 逐车
     `PreEntry|InTransition|Clearing` 相位时间线 + `conflict_reservation` 终态）；
  3. **拒绝零部分产物 + 世界照常步进**（拒绝后下一拍 tick/双游标不变、`step()` 继续）。
- **双向钉边**：T-free 时刻捕获必须**成功**、T 触及时捕获必须**拒绝**——两侧任一漂移
  都是失败（防「为了过矩阵而把捕获改成静默跳过」或 #1217 边界回退）。
- **红线落实**：面 B 的相等判据是「拒绝变体 + 身份/历史投影」三元组，**绝不用同一
  错误编码的摘要互相印证**；投影字段携带身份（资源种类 tag + edge 稳定标识/ordinal +
  occurrence + 拍号），不以归属 `(stream, zone)` 冒充身份（#1217 P1-4 口径）。
  另钉 `conflict_passage_locator(T) == None`（归属≠身份派生源切断的公开面回声）。

**F3 换道场景与拒绝语义的具体交互**（承接 lead 提示的方案 b）：

1. **恢复续跑用非 T 时段切面**：L 族两个恢复检查点均落在首 crossing 前的 T-free 拍
   （LFRS 无 T 行，#1217 恢复读端 fail-closed 不触碰）；恢复后缀从下一拍跑至原终点，
   **换道全程（授予、crossing、清空、败者进入）在恢复续跑的后缀里发生**——
   这正是「恢复后续跑换道」的语义证据。
2. **断言保存被结构化拒绝本身成为 F3 换道场景期望行为之一**：参考运行与恢复运行
   在穿越后的同一组检查点拍（沿用 F3 的每 1024 拍 + 最终边界节奏）**显式调用保存/
   摘要并断言面 B**（拒绝变体 + 投影 + 零部分产物 + 照常步进）；
3. **末态（`final.json`）**：L 族末态记录为「面 B 拒绝记录 + `t_state` 投影 + 逐车公开
   投影 + 宿主累计记录」，参考/恢复两侧逐字节相等；`final.json` 的 L 族形制与
   W1–W4 形制分字段命名，互不混用。
4. **S5 翻转点**：#1200 完整格式（LFRS 7→8 / state 5→6 / digest 8→9）交付后，面 B
   翻转为按新格式写行 + 恢复续跑含 T 历史世界；本方案的双面协议保留为版本迁移的
   回归锚（翻转后面 B 断言改为「成功捕获 + 行内容身份保真」）。

---

## 4. F1 接入设计（完整有序输出比较，除 `workers` 外全语义）

- **矩阵扩展**：L1 两身份（`10000-L1-16`、`100000-L1-33`）并入 `jobs()` 网格：
  +2 身份 × 5 worker × 3 轮 = **+30 次运行**、**+28 对比较**（合计 150 次 / 140 对）；
  轮内左移 7、并发与 worker 预算口径不变；`summary.json` 的 scope 字符串登记
  L 族覆盖与边界。
- **回执比较**：`compare` 不动——L 族回执同样「仅排除 `workers` 全字段精确相等」。
  新增 T 字段全部落在回执/文件内部，自动进入精确比较。
- **新增 T 预约/清空状态字段（怎么比）**：观察帧为 L 族另立域
  **`lf-synth-public-boundary-lc-1`**（`lf-synth-public-boundary-1` 逐字不动，
  W1–W4 帧字节零变化）。帧内新增（全部小端定长、逐字节比较）：
  - 逐车 T 字段：过渡相位码（0 无 / 1 `PreEntry` / 2 `InTransition` / 3 `Clearing`）、
    所持预约身份（资源种类 tag + 过渡段 edge 身份 + passage occurrence + acquired 拍；
    无预约为 0 哨兵）；
  - 逐 T 段字段：预约生命周期状态（0 无 / 1 committed / 2 clearing）、owner rank、
    acquired 拍、actual-clear 拍（来自公开事件史，断言 ≤ 世界时钟）；
  - 事件行扩展：放行种类 4/5/6/7（`ReservationAcquired` / `ConflictEntered` /
    `ConflictCleared` / `ReservationReleased`，`kernel/transitions.rs:60`），payload 携带
    passage 身份（资源种类 + edge raw + occurrence）；行宽随新域固定（如 15 字段）。
- **宿主语义检查扩展**（逐拍、新域内）：T 独占恰一（R1b 层②）、预约生命周期闭合
  （acquired ≤ entered ≤ cleared = released、`rear >= clearance` 判据）、败者 no-grant
  停让在入口外、层①非穿越拍无存续预约、`conflict_passage_locator(T) == None`、
  静态行/过渡行身份不互覆。
- **检查点**：`check_report` 对 L 族按 §3 双面——T-free 拍照旧 64-hex `state_digest`，
  T 触拍为 `{"persistence_refused": <variant>, "t_state": <投影>}`；`state_digest` 在
  T 触拍出现即失败、`persistence_refused` 在 T-free 拍出现即失败（双向钉边）。

## 5. F2 接入设计（失败零发布与 retry/fresh 对照）

- **矩阵扩展**：L1 × 注入断点维度。失败边界 = **前车预约建立拍**（P4 授予/crossing
  提交同拍）；注入断点覆盖换道预约相关断点：**`AfterGrants`**（staged 授予后、提交前
  ——断言 §5.1「失败拍丢弃 staged、不建预约」）与 **`AfterTransitions`**（crossing
  转移产生后——断言零发布、事件不外泄）。2 断点 × 2 身份 × 5 worker × 3 轮 =
  **60 次失败重试**、56 对比较（每「身份×断点」组合以第一轮单 worker 为参考）；
  正常参考 +2（L1 两身份，断点间共享）。合计 F2 = 6 正常 / 120 重试 / 112 对。
  失败/fresh 世界预算口径（worker×2、≤32、≤4 并发）沿用。
- **失败零发布**：`post-command.lfrs == failed.lfrs == fresh-post-command.lfrs` 保持
  **逐字节**比较——失败边界拍为 T-free（crossing 未提交），三份快照走面 A，谓词不放松。
- **retry/fresh 对照**：重试成功后世界进入 T 权威/历史期，`retry.lfrs == fresh.lfrs`
  的 L 族版本为面 B 比较（拒绝变体 + `t_state` 投影 + 零部分产物 + 照常步进）；
  重试续跑至 L 族终点（覆盖败者清空后进入），`final` 同走面 B。跨 worker 的
  8 文件比较中，T-free 文件逐字节、T 触文件比面 B 记录。
- **观测面**：`observations.json` 为 L 族另立域（如 `lf-synth-retry-observations-lc-1`），
  事件行放行换道生命周期种类（4/5/6/7）并携带 passage 身份；失败边界 outcome 仍必须
  null、`post_command == failed == fresh_post_command`、`retry == fresh` 的相等口径
  改为「面 B 记录 + 投影」相等。
- **guardrail**：完整预热 + 前两拍 guardrail 沿用；失败后不重放命令、只重试原
  `TickInput`，重试后 failpoint 消耗断言沿用（`formal_input_failed_step_retry` 同款
  形制，L 族新增测试驱动 `lf_synth_contract` 家族成员）。

## 6. F3 接入设计（恢复后缀比较：轨迹/事件/命令后缀/末态）

- **矩阵扩展**：L1 两参考 + 2 检查点 × 5 worker × 3 轮 = **+60 次恢复**
  （合计 10 参考 / 20 检查点 / 300 次恢复）；轮内左移 7、≤4 并发、Σworker ≤32 沿用。
- **检查点位置**：`positions` 为 L 族返回两个 T-free 拍（首 crossing 前），
  生成器钉版保证 crossing 落在检查点 1 与终点之间；`restore-probe` 不扩至 L 族
  （预算探针口径保持 W1–W4，边界登记）。
- **恢复后缀比较**（`resume` 的 L 族版本）：
  - **轨迹后缀**：`ticks.bin`（`lf-synth-public-boundary-lc-1` 帧摘要）按宿主记录精确
    偏移取完整后缀逐字节相等——换道全程（含 T 字段）在后缀中逐拍相等；
  - **事件后缀**：`events.bin` 完整有序后缀逐字节相等（含换道生命周期事件与身份）；
  - **命令后缀**：L 族无命令批次则 `commands.bin`/`batches.json` 后缀恒等为空后缀
    （登记声明）；若阶段 2 采用命令进场备选（§2/§10-D3），则同 W3/W4 口径比较收据与批次；
  - **末态**：§3 第 3 点——面 B 记录 + `t_state` 投影 + 逐车公开投影 + 宿主累计记录，
    与参考 `final.json` 逐字节相等；
  - **检查点后缀**：每 1024 拍/最终边界的检查点记录按 §3 双面（T-free 拍 `state_digest`
    与参考相等；T 触拍面 B 记录与参考相等），且恢复侧对同拍保存显式断言拒绝（§3-F3-2）。
- **身份映射与来源**：`checked_reference` / `restore` 的来源、exact LFCA、认证 `Pass`、
  恢复映射（live order、路线边稳定标识、逐车归属）核对全部沿用，不放松。

## 7. 真实资源身份比对口径（横切，承接 #1217）

- **比较谓词不得只比同一错误编码的摘要**。面 B 的比较对象 = 拒绝变体 **AND**
  `t_state` 身份/历史投影 **AND** 零部分产物 **AND** 照常步进，四者缺一即失败。
- `t_state` 投影的字段来源（全部公开只读查询，`install.rs` 同一安装路径可达）：
  - `TrafficWorld::conflict_reservation(vehicle)`（`kernel/world.rs:2391`）→ committed
    预约的 passage 身份（`ConflictPassageAddress` 布局保持编码：资源种类高位标记 +
    edge raw）；
  - `TrafficWorld::latest_transition_events()`（`kernel/world.rs:2943`）→ 有序
    `ReservationAcquired/ConflictEntered/ConflictCleared/ReservationReleased` 事件携带
    `ConflictPassageRange` / `ConflictPassageOccurrenceLocator` 身份 → 换道历史
    （acquired/entered/cleared/released 拍号）；
  - `VehicleState::maneuver_traversal()` → `PreEntry|InTransition|Clearing` 相位；
  - `TrafficWorld::conflict_passage_locator`（`kernel/world.rs:482/2347`）对 T 返回
    `None`（#1217 派生源切断，作为身份保真断言钉住）。
- **身份字段携带资源种类 + 过渡段身份**（`ConflictResource::Transition(LaneEdgeOrdinal)`
    口径 + 网络稳定标识核对到 manifest 的 edge id），断言投影中的 T 行**不得**出现
  `(stream, zone)` 归属对形态（归属≠身份）。
- **缺口登记（可选加深，见 §10-D4）**：仲裁器 lag 行权威史（`persisted_lag_rows`/
  `lag_reference`，`kernel/conflict.rs:2737/2372`）目前 `pub(crate)`，harness 不可达；
  若 lead 要求把「lag 行内部史」纳入比较面，需新增只读查询 API（G1 小缺口，另行立项）；
  默认方案以上述公开事件史 + 预约查询为历史面（已是提交域权威历史）。

## 8. worker 配置覆盖

沿用 F 系列既有口径扩展到换道场景，逐矩阵不新增例外：

- worker 网格 **1/2/4/8/16** × **3 个全新进程轮**；轮内配置序左移 `7×(轮-1)`；
- 最多 4 个并发进程、运行中配置 worker 之和 ≤ 32（F2 含 fresh 世界按 worker×2 计）；
- 回执核对**实际** worker（`ExecutionConfig` 显式传入、不按 CPU 推断、不降级）；
  比较基线恒为每「身份×（F2：注入断点）」组合的第一轮单 worker；
- R1a/R1b 合同验收的「worker 配置无关一致 / 重放与不同 worker 配置结果一致」由
  L 族矩阵行提供矩阵级证据（与 `tables.rs` 钉版用例互补，不替代）。

**运行数汇总（阶段 2 预算）**：

| 矩阵 | 现有 | L 族新增 | 合计 |
| --- | --- | --- | --- |
| F1 运行 / 比较对 | 120 / 112 | +30 / +28 | 150 / 140 |
| F2 正常参考 / 重试 / 比较对 | 4 / 60 / 56 | +2 / +60 / +56 | 6 / 120 / 112 |
| F3 参考 / 检查点 / 恢复运行 | 8 / 16 / 240 | +2 / +4 / +60 | 10 / 20 / 300 |

## 9. 边界声明（如实延续 + 换道族新增）

保持 `tools/laneflow-synth-harness/README.md` 既有边界原文（不覆盖所有失败点、
不覆盖所有 worker 两两组合、非标称交叉组合/强制调度置换/快照解码恢复/Routing/
表现接线/完整五轴各自取证、`Product TBD / Uncertified` 不变），换道族追加：

1. **单过渡段**：L 族仅覆盖恰一横向过渡段网络；多过渡段/多 T 历史的矩阵覆盖受阻于
   世界级多 T 夹具缺口（#1217 §7.3），归 S5 #1200；
2. **场景形状**：F 矩阵只以 R1b（两换道者稳定序竞争）语义开换道族；R1a（对直行让行）、
   R3（长车尾清空）形状不在 F 矩阵，验收保留在 `tables.rs` 钉版用例；
3. **失败点**：换道族失败重试只覆盖 `AfterGrants`/`AfterTransitions` 两个注入断点的
   单次注入、单错误首错；不覆盖多错误首错、其他 failpoint（`AllocationAfterGrants`）、
   强制调度置换；
4. **恢复切面**：换道族恢复检查点仅取 T-free 切面（首 crossing 前）；含 T 状态/历史
   世界的保存/恢复/跨修订迁移在 S5 #1200 完整格式交付前一律以结构化拒绝为预期行为
   （#1217 边界），不得把面 B 断言写成「恢复认证」；
5. **观察面**：逐拍公开观察只覆盖设计列出的公开字段 + 新增 T 预约/清空字段，
   不宣称逐拍内部状态全部受检；`t_state` 投影是身份/历史比较面，不冒充完整 lag
   权威行（若 D4 选做只读 API 后另述）；
6. **性能**：换道族矩阵只做语义对照，不产生延迟/加速比/恢复性能认证。

## 10. 待 lead 裁定清单

- **D1（场景来源）**：推荐 S1（生成器新增 `L1` case 族，四文件入口全复用）；
  S2（运行时夹具载体）不推荐（理由见 §2）。
- **D2（F3/#1217 交互）**：§3 双面协议（面 A 逐字节 + 面 B 拒绝变体+身份历史投影+
  零部分产物+照常步进、双向钉边；恢复切面取 T-free、保存被拒本身入期望行为）。
  这是本方案关键设计点，请重点复核。
- **D3（L 族窗口机制）**：推荐「出生位钉版（s12 口径）+ 检查点钉在首 crossing 前」，
  L 族自有窗口数值阶段 2 试跑钉定并冻结进 probe-vectors；备选「命令批次在检查点后
  进场」（窗口可更大、命令后缀进比较面，但增加命令面复杂度）。
- **D4（历史比较深度）**：默认以公开事件史 + `conflict_reservation` + 相位时间线为
  历史面；是否要求新增仲裁器 lag 行只读查询 API（G1 小缺口）纳入比较面，请裁定。
- **D5（evidence.yml 过滤器语义注记）**：已按验收把 R1b 加入 `-E` 过滤名单；但该
  工作流以 `--run-ignored ignored-only` 运行，R1b 移除 `#[ignore]` 后在该工作流的
  交集为空（实际由常规门禁的 `cargo test --features placement-fixtures` 覆盖）。
  过滤条目现为惰性记录（若将来 R1b 被重新 `#[ignore]` 则自动回到 nightly 覆盖）。
  如 lead 期望 R1b 在 Evidence 工作流实际执行，需要另行调整运行模式（超出
  「仅改过滤器条目」授权范围，已停下待示下）。
- **D6（F2 注入断点数）**：推荐 2 断点（`AfterGrants` + `AfterTransitions`，合计
  60 次/56 对与现有 F2 同量级）；如需压缩矩阵可减为 1 断点（推荐保 `AfterGrants`，
  对换道预约最具区分度），运行数减半。
