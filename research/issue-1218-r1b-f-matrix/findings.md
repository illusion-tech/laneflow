# Issue #1218 差异记录：R1b 常规化 + F1/F2/F3 换道接入

**状态**：阶段 2 实现完成（F1/F2/F3 换道接入 + dev 态 L 族矩阵切片；正式冻结构建全矩阵
待 lead 本地 commit 后按裁定分步走——见 §7；未 commit/push/PR，改动留工作树）
**基线**：`main@d5f28317f6a741f7845827635b8a3d6b8161a145`（worktree `research/1218-r1b-f-matrix`）
**方案**：[`plan.md`](./plan.md)（lead 已批准 D1–D6；D2 硬要求=面 B 身份投影只用既有公开观测面）

---

## 1. 阶段 1（已完成）：R1b 常规化 + 方案稿

| 变更 | 文件 | 结果 |
| --- | --- | --- |
| 移除 R1b `#[ignore]`（R 断言零改动）+ doc 注释如实记录显式执行出处（#1216/#1217 合入后、未做归因实验） | `crates/laneflow-runtime/src/kernel/tables.rs` | ✅ `1 passed / 0 ignored`（常规模式） |
| Evidence 过滤器追加 R1b 条目 + D5 注释（条目仅在其回归 ignored 状态时生效，常规门禁=Placement fixture tests 每 PR 必跑） | `.github/workflows/evidence.yml` | ✅ 仅过滤器条目 + 一行注释 |
| F 系列接入方案稿（双面协议关键设计点、D1–D6 待裁定） | `research/issue-1218-r1b-f-matrix/plan.md` | ✅ 已批 |

## 2. 阶段 2 已完成：生成器 L1 case（D1/D3）

`tools/laneflow-synth-generator`：新增 `Case::L1`、`network_lane_change.rs`（独立发射面）、
`state.rs`/`plan.rs`/`bundle.rs`/`main.rs` L1 支持。

- **网络**：cell 背景（车道边 + 25 停车位/路线，无机动/门/信号）+ 换道簇（恰一横向过渡段
  `lc-t`，R1b s12 钉版载体形制逐项保持：边长 30/5/30/8/10/30/30/30 m、lane 链、帧跨度、
  冲突区多边形）+ 编辑侧 junction/冲突区/两条流/无信号准入门 + 唯一策略集（gap profile
  500/500/100 + 流规则 + 全门规则）。
- **状态**：s12 钉版两换道者（e0@11000 / e0@3000，10 m/s，`with_open_entrance`）+ W1 型背景；
  车型钉版承接 R1b 权威口径（a_service 4.5、min_gap 2.5、d_stop(10 m/s)=15.61 m 前提）。
- **窗口钉版（D3）**：预热 16 拍（F3 检查点 0/1 = 16/32，早于首 crossing）；观察窗
  16 ms = 304 拍（终 320）、33 ms = 144 拍（终 160）。探针
  `lane_change_lifecycle_probe_freezes_window_pinning` 冻结拍号并双向断言。

探针实测（2-cell 世界，1200 拍长程）：

| 步长 | 前车入 T | 前车出 T | 前车释放 | 前车完成 | 败者入 T | 败者出 T | 败者释放 | 败者完成 | 拒绝面起点 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 16 ms | 110 | 136 | 160 | 370 | 286 | 447 | 508 | 822 | 110（=首 T 权威拍） |
| 33 ms | 53 | 66 | 78 | 180 | 139 | 217 | 247 | 399 | 53（=首 T 权威拍） |

窗口不变量（探针断言）：检查点 16/32 早于首 crossing；前车完整生命周期收束在窗口内；
败者清空后进入拍落在窗口内且晚于前车释放；窗口内无换道者路线完成。正式十万规模实测同形
（10000-L1-16：前车 110/160、败者 286——与探针一致）。

## 3. 阶段 2 已完成：F1 换道接入（harness）

`tools/laneflow-synth-harness`：

- **install**：L 族容量（车辆=规模+2、边出现项按实长累计、冲突出现项 16）、簇路线
  无信号组（`signal_group_id=None`）、换道者 `with_open_entrance`、逐路线真实边长
  前缀和位置表（`route_position_offsets`）。
- **观察域 `lf-synth-public-boundary-lc-1`**（W 域 `lf-synth-public-boundary-1` 逐字不动）：
  W 族 8 字段 + 逐车 T 字段（遍历相位 0-4、所持预约标记/身份、acquired 拍）；语义检查含
  T 独占恰一、败者在前车 **T 预约**释放后进入（按资源种类钉判据，前车可合法持有下游
  zone passage 预约）、换道预约生命周期事件序（4→5→6→7）。
- **事件编码**：L 族放行事件种类 1/4/5/6/7/8；4/5/6/7 的 payload 携带**真实资源身份**
  （`ConflictPassageAddress::static_zone` 判资源种类；T 格 → `T_RESOURCE_IDENTITY`
  标记 + manifest 绑定的簇路线 T 边稳定标识进 `t_state` 投影；Zone 格 → zone ordinal）。
- **双面检查点（D2/面 A/面 B）**：T-free 拍（0/16/32）必须成功捕获（64-hex
  `state_digest`）；终态拍（T 历史）必须结构化拒绝（`persistence_refused` =
  `AdmissionHopWithoutGate` 等变体，`SnapshotCaptureError::UnsupportedTransitionState`
  公开匹配）+ `t_state` 投影（换道预约生命周期史（tick/槽位/种类/资源身份）+ 逐车相位
  与预约终态）+ `state_digest` 必须缺席——**双向钉边**进 `check_report`。
- **矩阵口径（D6/预算核准）**：F1 网格扩为 10 身份（W1–W4 + L1）× 5 worker × 3 轮 =
  150 次 / 140 对（回执仅排除 `workers`、`ticks.bin`/`events.bin` 逐字节比较谓词不变）。

真实运行（`10000-L1-16`，dev 态声明 `implementation_commit=main 基线 SHA`，工作树含本单
改动未 commit——非正式冻结构建，**不构成正式矩阵证据**）：见 §6。

## 4. 关键发现（冻结面/编译器约束——后续切片必读）

1. **LFCA 规范制品嵌入发射调用点行号**：`network.rs` 任何行移（纯注释行实验证实）都会改变
   `network.lfca` 字节（等长异值），从而破坏 `fixtures/probe-vectors.json` 冻结摘要。
   因此 L1 发射面**独立成 `network_lane_change.rs` 新文件**、dispatch 放在 `bundle.rs`，
   `network.rs` 逐字保持基线。**教训**：改 `network.rs`（哪怕纯注释）都会翻 W 族冻结向量；
   再生成 probe-vectors 属冻结 fixture 再生成治理，须停下报 lead。
2. **策略覆盖约束**：编译期策略校验要求**每个策略集覆盖全网每道门与每条流**
   （`mir/policy/validation.rs` `select`/MissingRule）；而合成侧（module-scoped）门**无法**
   被编辑侧规则引用（`RoadEditingReference::imported` 门深度=3 vs 合成门地址深度 0，
   `UnknownReferenceTarget`）。故 L1 背景**不设 cell 门/信号**（L 族自有背景语义 D1），
   全部门/流（换道簇）由编辑侧唯一策略集覆盖。W 族形制不动。
3. **D2 身份通道（不新增产品 API）**：`ConflictResource` 未从运行时根导出、
   `ConflictPassageAddress::new_transition` 为 `pub(crate)`——但真实身份**可**由既有公开面
   表达：资源种类 = `ConflictPassageAddress::static_zone()`（T 格 `None`）；过渡段稳定边身份
   = 正式输入 manifest 中簇路线 `edge_ids[1]`（单过渡段钉版，进 `t_state` 投影）。
   不以归属 `(stream, zone)` 冒充身份；不解析 Debug 形状当身份。若 S5 多 T 需逐边区分，
   届时按完整格式（资源种类 tagged 身份）走 #1200，无需现版 API。
4. **运行时遍历相位实名**：`ManeuverTraversalPhase::{PreGate,Committed,Waiting,Clearing}`
   （plan 稿写 wire 形制 `PreEntry|InTransition|Clearing`，S5 #1200 交付时对齐）。
5. **L 族位置编码**：簇路线边长非均匀，`observe` 的 W 位置公式（`edge_index*5_130_000`）
   不适用；L 域改用真实边长前缀和。W 域不动。
6. **事件面**：T crossing 产生 `GateCrossed`（准入门）+ 预约生命周期事件（kind 4/5/6/7）；
   败者 no-grant 等待不产生 waiting-zone 事件。

## 5. 红线遵守

- R1a/R1b/R2/R3 断言零改动（R1b 仅去 ignore + doc 注释，见阶段 1）。
- F 系列既有比较谓词零放松：回执「仅排除 `workers` 全字段相等」、`ticks.bin`/`events.bin`
  逐字节、W 族检查点 64-hex 摘要校验逐字保持；新增仅限 L 族行与面 B 记录。
- 面 B 不以错误编码摘要互相印证：比较三元组 = 拒绝变体 + 真实资源身份/历史投影（生命周期
  史 + 逐车相位 + 预约终态，均来自公开观测面）+ 零部分产物/照常步进（`check_report` 双向钉边）。
- 未新增任何产品侧只读 API（D2 硬要求）；`SnapshotCaptureError::UnsupportedTransitionState`
  的公开匹配与既有访问器均为既有公开面。
- YAML 变更仅 `evidence.yml`（过滤条目 + D5 注释）。

## 6. 运行记录（dev 态；非正式冻结构建，不构成正式矩阵证据）

- 生成：`10000-L1-16-a` 四文件（10002 个体 / 401 路线 / 2 门 / 0 控制器）。
- F1 L 族切片（`worker-run`，1/2/4/8/16 worker × 3 全新进程轮）：**15 次运行**，每次
  321 拍全语义核验通过（含面 A/面 B 检查点、T 独占恰一、败者前车 T 释放后进入、
  生命周期事件序）；以第一轮单 worker 为参考逐对比较（回执除 `workers` 外全字段 +
  `ticks.bin`/`events.bin` 字节）= **14 对，0 失败**。
- 终态拍（320）面 B 记录实例：`persistence_refused: "AdmissionHopWithoutGate"` +
  `t_state.journal` = 前车 [110 acquire(T)→110 enter(T)→160 clear(T)→160 release(T)]、
  285 前车 zone passage [acquire/enter(Zone#0)]、败者 [286 acquire(T)→286 enter(T)]；
  两换道者相位 `Clearing`、各持 1 条预约——真实资源身份（Zone ordinal / T 标记 +
  manifest 绑定的 `lc-t` 稳定标识）与历史全在比较面内。
- 未运行（原因见 §7）：`100000-L1-33` 身份；F2/F3 换道行；W 族正式矩阵（正式协议要求冻结
  构建/干净检出/环境声明——本单红线「不 commit」，正式矩阵作为独立交付动作待 lead 安排）。

## 7. 阶段 2 其余实现（F2/F3）与遗留

### 7.1 F2 换道接入（已实现）

- **运行时驱动** `crates/laneflow-runtime/src/kernel/tests/lf_synth_contract/lane_change_retry.rs`
  （`lane_change_failed_step_retry`，`#[ignore]` + 五环境变量含 `LF_SYNTH_FAILPOINT`）：
  探针世界定失败边界 = 前车首次入 T 拍（确定性）；失败/fresh 两世界推进至边界前一拍，
  T-free 边界四份 `.lfrs` 逐字节（零发布）；注入 `AfterGrants`（staged 丢弃不建预约）或
  `AfterTransitions`（crossing 零发布）→ `ParkingObservationAllocFailed`、failpoint 消耗、
  状态/事件/游标不变；只重试原 step；retry/fresh 逐拍等价；续跑至窗口终点；
  T 触时刻（retry/fresh/final）输出面 B 记录（`persistence_refused` + `vehicles`/`journal`
  身份历史投影）；边界观测 `lf-synth-retry-observations-lc-1`（事件行携带资源身份 payload）。
- **harness `worker_retry.rs`**：网格 = W3/W4 4 身份 + L1（2 身份 × 2 注入断点）=
  8「身份×断点」组合 × 5 worker × 3 轮 = **120 次重试 / 112 对**（回执仅排除 `workers` +
  文件逐字节；L 文件集 `L_FILES`：T-free 四份 `.lfrs` + `retry.json`/`fresh.json`/`final.json`
  面 B 记录 + `observations.json`）；同源正常参考 6 个；`verify_lane_change_reports` 把
  末态面 B 历史（`journal`，含真实资源身份）与正常参考终检查点 `t_state.journal` 逐项核对。

### 7.2 F3 换道接入（已实现）

- **面 B 恢复续跑**：L 恢复检查点（16/32 拍，T-free 切面）LFRS 捕获/恢复照旧；
  `Host` 记录新增 `lane_change_journal`（空则省略，W 形制逐字不动），恢复端续接使
  参考/恢复末态同跨度可比；续跑后缀比较照旧（lc-1 轨迹/事件后缀 + 命令后缀）；
  检查点与 `final.json` 双面（T-free 拍 `state_digest`、T 触拍 `persistence_refused` +
  `t_state`），`lane_change_refusal_variant` 显式断言保存被结构化拒绝（成功捕获亦失败，
  双向钉边）。
- **`restore-matrix` 网格**：+L1 → 10 参考 / 20 检查点 / **300 次恢复**（probe 口径保持
  W1–W4 = 32，换道族不进 probe）。
- **容量一致性**：恢复重建 `WorldConfig` 与安装同式（`install::world_config`；L 族车辆
  容量=规模+2、边出现项按实长、冲突出现项 16）——否则 `restore_lfrs` 容量校验拒绝。

### 7.3 dev 态 L 族矩阵切片（非正式冻结构建，不构成正式矩阵证据）

| 矩阵 | 运行 | 比较对 | 结果 |
| --- | --- | --- | --- |
| F1 `10000-L1-16`（1/2/4/8/16 worker × 3 全新进程轮） | 15 | 14（回执除 `workers` + `ticks.bin`/`events.bin` 字节） | **0 失败** |
| F2 `10000-L1-16`（2 注入断点 × 5 worker × 3 轮） | 30 | 28（同上 + 8 文件字节） | **0 失败** |
| F3 `10000-L1-16`（2 检查点 × 5 worker × 3 轮 + 1 参考） | 30 恢复 | 30 次「后缀+末态」对参考比较 | **0 失败** |

- F2 末态面 B 历史与 F1 同源正常参考 `t_state.journal` 逐项核对：**2/2 相等**
  （[110 acquire(T), 110 enter(T), 160 clear(T), 160 release(T), 285 前车
  GateCrossed/Zone#0 acquire/enter, 286 败者 acquire(T)/enter(T)]，payload 携带
  T 标记 `2147483648`/Zone ordinal `0`）。
- F3 参考检查点 tick 0 的 `state_digest` 与 F1 同输入 tick 0 摘要逐字节一致
  （`4a2ee207…`，跨入口一致性旁证）；tick 320 检查点与 `final.json` 均为面 B 记录
  （`AdmissionHopWithoutGate` + 身份历史投影）。

### 7.4 遗留（正式证据步骤，按 lead 分步裁定）

1. 正式冻结构建全矩阵：lead 本地 commit 后，在该 commit 跑 `100000-L1-33` 与 W 族全网格
   （F1 150 / F2 120 / F3 300），manifest `implementation commit` 与被测树一致；
   结果回填本文件后 lead push+PR。
2. `100000-L1-33` 身份的 dev 态生成/运行未做（同形代码路径，无独立逻辑）。
3. S5 #1200 前：多 T 世界、含 T 恢复续跑（面 B 翻转）照旧登记。

## 8. 验证补记

**本地交付清单（固定四项，四项全过才允许 push；本单红线不 push，停在「待交付」等 lead
本地 commit 后跑正式矩阵）**：

| 检查 | 结果 |
| --- | --- |
| `cargo run -p xtask -- check-runtime-architecture` | ✅ 通过（显式模块、格式接口与默认/全特性生产依赖规则） |
| `cargo fmt --check` | ✅ 无差异 |
| `cargo clippy -p laneflow-runtime --all-targets --locked --features placement-fixtures -- -D warnings` | ✅ 零告警 |
| `cargo test -p laneflow-runtime --locked --features placement-fixtures` | ✅ 全绿（exit 0） |

补充检查（tools 改动补跑）：

| 检查 | 结果 |
| --- | --- |
| `cargo test -p laneflow-synth-generator --locked` | ✅ 7 passed / 0 failed（1 ignored = `full_scale` 冻结矩阵，默认忽略） |
| `cargo test -p laneflow-synth-harness --locked` | ✅ 22 + 13 + 5 passed / 0 failed（1 ignored 预存） |
| `cargo clippy -p laneflow-synth-generator -p laneflow-synth-harness --all-targets --locked -- -D warnings` | ✅ 零告警 |

> 注意：生成器 schema 扩展（`VehicleRecord.parking_space_id`/`RouteRecord.signal_group_id`
> 改 `Option`、`edge_ids` 改 `Vec`）是 W 字节等价的（W 族 probe-vectors 冻结摘要原样通过），
> 但**编译期面**触及 runtime 测试消费方 `formal_retry.rs`（Option 解引用适配，行为零变化）。

## 9. 正式矩阵首错与修复（2026-11-03）

**首错事实**：F1 `worker-matrix` 全新目录重跑至 **43/150** 时按协议首错停止：
`100000-L1-33-w1-r1`、`100000-L1-33-w2-r1` 子进程退出码 1，日志均为
`语义验证失败：进度/速度/加减速 tick=1 rank=24`；此时已完成 **34/140** 对比较全部相等。
失败与 worker 数无关（w1/w2 同点位复现）；材料保留于 `lf1218-formal2/F1`，不拼接、
不补跑。

**根因**：L 族生成器车型 `max_acceleration_meters_per_second_squared = 1.5`
（`tools/laneflow-synth-generator/src/network_lane_change.rs`），而观测器
`motion_valid` 把加速度上界硬编码为 `1_400`（1.4 m/s²，沿用 W 族口径）。dt=33 ms
时合法加速产生 Δv=49 mm/s，超过硬上限 `ceil(1400×33/1000)+1 = 48`；dt=16 ms 时
Δv=24 恰等于上限，故 dev 态 `10000-L1-16` 切片未暴露，`100000-L1-33`（§7.4 遗留
2）首次运行即命中。

**修法**：`Sample` 新增 `max_accel_mm_s2`，安装与恢复两条路径均从共享路网
`vehicle_profile(profile).max_accel()` 派生（米/秒²→毫米/秒²）；`motion_valid`
改收车型上界，两处调用点（W/L）与 burst 观测同步传入；新增单元测试钉住
1.4/1.5 m/s² × dt=16/33 的边界组合。修复后 debug 单跑 `100000-L1-33` w1
`complete=true`（面 A/B 检查点齐备，仅实现验证、不作正式证据）。

**证据处置**：d28a8fbc 冻结运行的局部/首错材料全部保留；本修复提交为新的实现提交，
随后按该提交重建冻结构建并**全新目录**重跑 F1/F2/F3。

## 10. F2 矩阵死锁与修复（2026-11-03）

**现象**：F2 `worker-retry-matrix` 正式运行在 **4/6 正常参考**完成后停滞——重试
**0/120**、`normal_runs=4`、无子进程、父进程空转 **33 分钟**，`errors=0`。

**根因**：`worker_retry::jobs()` 的 L1 行全部携带注入断点（`AfterGrants`/
`AfterTransitions`），任务序列只构造出 **4 个正常参考**（W3/W4 × 2 尺度）；而
`run_matrix` 的启动闸门与完成条件写死 `normals.len() == 6`
（`expected_normal_runs: 6`，设计口径 = W3/W4 × 2 + L1 × 2）。闸门永不开启，
120 个重试无法启动。dev 态切片只跑过 L1 重试本身，未端到端运行
`worker-retry-matrix` 入口，故未暴露。

**修法**：抽出 `tasks()`，显式补齐 L1 × 2 尺度正常参考（共 6 个，置于 120 个
重试之前）；新增回归测试 `retry_tasks_have_six_normal_references_before_retries`
钉住任务序列前 6 个为正常参考。harness 全量测试 23+14+5 passed / 0 failed，
fmt、clippy 干净。

**证据处置**：f6629bc6 冻结下的 F1（70/150，0 错误）与 F2（4/6 正常参考、0 重试）
材料全部保留；修复提交 `537b98b0` 为新的实现提交，按协议以新冻结构建**全新目录**
重跑 F1/F2/F3。

## 11. F2 L1 游标字段占位与修复（2026-11-03）

**现象**：F2 死锁修复后重试启动成功（6/6 正常参考），但 `AfterGrants-10000-L1-16`
w1/w2/w4/w8 首错停止——`check_observations` 报“边界观测时间或游标不符”（4 错误）。

**根因**：`lane_change_retry.rs` 报告把 `cursor_before_commands`/
`cursor_after_commands`/`cursor_after_retry`/`final_cursor` 硬编码为 0（占位），
而 `observations.json` 记录真实 `world.command_cursor()`（本输入恒为 10403）；
W 族 `formal_retry.rs` 早已正确记录这些游标，L 族测试漏了。dev 态切片未经过
harness 的 `check_observations`，故未暴露。

**修法**：在失败前/失败后/重试后/终态四个检查点捕获真实游标写入报告。手动复跑
`10000-L1-16` AfterGrants w1：`1 passed`，报告与观测游标逐字段一致。

**证据处置**：537b98b0 冻结下的 F1（31/150，0 错误）、F2（6 正常参考 + 10 重试、
4 错误）、F3（5 参考，0 错误）材料全部保留；修复提交 `4cb74402` 为新的实现提交，
按协议统一以新冻结构建**全新目录**重跑 F1/F2/F3。

## 12. F2 L1 final 观测缺终态步进结果与修复（2026-11-03）

**现象**：F2 游标修复后仍首错停止——`AfterGrants-10000-L1-16` w1/w2/w4/w8
报“缺少完整步进结果”（4 错误）。

**根因**：L1 测试终态观测用 `observation(&failure, None)`（outcome=null），
而 harness `check_observations` 与 W 族一致要求 final 携带终态 `StepOutcome`
（tick/time/parking_arrivals）。续跑循环未捕获最后一步 outcome。

**修法**：续跑循环记录最后一步 outcome，final 观测改传 `Some`。手动复跑
`10000-L1-16` AfterGrants w1：`1 passed`，`final outcome={tick:320,
time_ms=5120,parking_arrivals:[]}`。

**证据处置（方案 B）**：修复只触及 runtime 测试目标（`#[cfg(test)]`，不进入
harness 二进制），F1/F3 不受影响。F1/F3 继续在 `4cb74402` 上跑完；F2 以
`0b49c172` 重建冻结（`bin5`/`environment5.json`）在全新目录单独重跑。
结果文档按矩阵分别登记 implementation commit。
