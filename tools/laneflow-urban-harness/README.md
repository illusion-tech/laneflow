# LF-CN-URBAN 无界面运行程序

#544 的无界面实现消费 #542 的正式制品目录，经 `TrafficWorld` 公共 API 运行七套封闭
case。库和命令行共用 `Artifacts`、`ResolvedPlan`、`Harness`；`adapter` feature
复用同一需求调度和提交态校验，提供 #545 的跨层观测。
设计依据为 [需求计划与无界面验证](../../docs/design/urban-demand-harness.md)。

## 本段验收

当前实现提供七套计划展开、实际初态、有限命令调度、逐 tick 校验和独立运行比较。
10k 正确性窗口是暖机 7656、观察 15312，共 22968 ticks。两次运行使用同一份预先
冻结的计划。fixture/短 probe 只验证实现；不提供正式 case 通过结论。

`GARAGE-EGRESS` 使用 25% Active/75% Parked；`GARAGE-INGRESS` 固定满池和显式排他
拒绝；三类交通 case 使用目标 cell 的有限角色脉冲并直接聚合 Waiting/Conflict 决策；
`BOUNDARY-BURST` 在相邻提交边界执行 park/leave/replace 及独立 despawn/spawn。
Waiting 的周期占用脉冲只在末端仍保留完整释放相位时重放，既覆盖迟入队后的下游
storage 拒绝，也要求终点脉冲实际 Completed 且无残留占用；时间余量本身不是通过证据。
所有路径都不增加 Traffic Runtime、共享静态路网或 Adapter 的公共接口。
实现和 fixture probe 不替代正式正确性及性能取证；后续跨层验证引用已交付的
[#544 结果](https://github.com/illusion-tech/laneflow/pull/614)，保留原始提交和范围。

两档七场景正确性与两档 Mixed 性能计划摘要保存在 [输入冻结记录](fixtures/v3/plans.json)。
当前 `urban-demand-v3` 固定全部路线的实际 edge 序列、观察期入场链及边界角色命令；
旧记录保留为历史诊断，不能作为当前计划的通过证据，不提供兼容转换。

## 运行

先用 `laneflow-urban-generator` 从 `examples/config/cn-urban.toml` 生成对应规模制品，
详见 [生成器](../laneflow-urban-generator/README.md)。以下命令从仓库根执行：

```text
cargo +1.98.0 build -p laneflow-urban-harness --release --locked
target/release/laneflow-urban-harness plan <artifact-directory> <plan.toml>
target/release/laneflow-urban-harness plan <artifact-directory> <plan.toml> --case GARAGE-EGRESS
target/release/laneflow-urban-harness run <artifact-directory> <plan.toml> <run-a>
target/release/laneflow-urban-harness run <artifact-directory> <plan.toml> <run-b>
target/release/laneflow-urban-harness compare <run-a> <run-b> <comparison.json>
```

`run` 接受 `--workers N`（缺省 1，合法域 1..=16，与 Runtime 执行配置上限一致）。
worker 属于执行配置：不进计划文件、不改变计划摘要、世界逻辑摘要或快照内容。
同一计划可用不同 worker 各跑一轮后做跨臂语义对拍：

```text
target/release/laneflow-urban-harness run <artifact-directory> <plan.toml> <run-1w> --workers 1
target/release/laneflow-urban-harness run <artifact-directory> <plan.toml> <run-4w> --workers 4
target/release/laneflow-urban-harness compare <run-1w> <run-4w> <comparison.json>
```

双目录 `compare` 支持 probe、correctness 与正式 performance 臂（status
`performance-match`）。performance 跨臂只接受同源码、同工具链、同硬件/电源
角色、仅 worker 不同的两臂：每臂先独立校验完成状态、文件摘要、测量封套
版本、执行编号、样本数与观测窗、worker 合法性及固定协议 provenance，再比较
除 `measurements.toml` 测量封套外的完整交通语义（计划摘要、逐拍日志、检查点、
角色见证与计数精确相等）；两臂 provenance 除 workers 外必须全等，diagnostics
与 measurements 的 worker 记录矛盾即拒绝。比较报告在 `ComparedRun.workers`
记录两臂各自 worker（自本切片起）。比较成功只表示跨臂语义匹配与来源条件
成立，不代表性能达标；正式性能结论仍以同 worker 三轮聚合
（`compare <performance-a> <performance-b> <performance-c>`，provenance 全等
含 workers）为准。

`--case` 只接受 `MIXED-PEAK`、`GARAGE-EGRESS`、`GARAGE-INGRESS`、
`WAITING-RELEASE`、`PERMISSIVE-LEFT`、`UNCONTROLLED-YIELD` 和
`BOUNDARY-BURST`。省略时为 `MIXED-PEAK`。

Windows 可为可执行文件加 `.exe`。计划文件与结果目录必须是新路径，避免覆盖证据。
`plan ... --probe-ticks 128` 生成短试跑；`--probe-warm-up N` 可在 fixture 上覆盖
“暖机后重新提交角色”的诊断路径。probe 不得冒充正式验收。
默认的 correctness 计划只接受 10k/100k 制品；fixture 必须显式使用 `--probe-ticks`。
库入口遵守同一准入规则，手工构造 correctness 窗口也不能为 fixture 创建正式计划。
载入时复用生成器 `Scale` 核对规模、tile 数、个体数和步长：fixture/2/2000/16 ms、
10k/10/10000/16 ms、100k/100/100000/33 ms；改写标签不能改变实际规模要求。
计划经读取后重新核对固定展开规则及来源文件摘要，不接受手工删减事件或车辆。
已加载的目录和共享路网通过 `Artifacts::catalog()` / `revision()` 只读借用；
改变源输入需要重新载入并展开计划，调用方不能替换已绑定来源摘要的内部字段。

正式性能计划只允许 Mixed 的 10k/100k 制品：

```text
target/release/laneflow-urban-harness plan <artifact-directory> <plan.toml> --performance
```

运行前必须设置 `LANEFLOW_HARDWARE_ROLE` 和 `LANEFLOW_POWER_ROLE`。每轮只统计观察
窗口，分别记录公共生命周期命令调用、`TrafficWorld::step` 和观测开销的 p50/p95/p99/max，实际
Active/intent 分布及进程 peak resident bytes 写入 `measurements.toml`。每档仍须按治理
流程启动三个新进程；单轮文件不代表三轮合并结论或产品预算认证。
三轮完成后仍通过 `compare` 入口做文件摘要、计划摘要和独立执行编号校验，并精确合并
三轮保存的观察窗口样本：p50/p95/p99 先逐轮计算，再取三个轮次值的中位数，max 取
三个轮次的最坏值；不将样本池化后求分位。每轮提交、干净状态、Rust/Cargo/target、构建参数、OS/架构、
硬件/电源角色、worker 与计时口径必须可用且一致；缺字段、脏工作树、样本数与窗口不符
均拒绝。正式运行在初始化前和窗口结束后核对来源，变化或无法读取时不产出性能通过包：

```text
target/release/laneflow-urban-harness compare <performance-a> <performance-b> <performance-c> <performance-comparison.toml>
```

三轮还须保持相同完整语义轨迹：除 `measurements.toml` 与 `diagnostics.json` 两个
执行封套外的结果字段、文件摘要、检查点、角色见证及状态/计数均精确相等。两个执行
封套均已逐轮独立校验后才排除（摘要绑定入各轮 `result.files`，worker 计数做
measurements↔diagnostics 交叉核对）；各包重新计算了自身摘要但彼此轨迹不同，
仍拒绝合并。运行目录应使用 Git 忽略的 `target/` 或 checkout 外目录；未来输出目录
若会使工作树变脏，在世界初始化前拒绝，不到长测结束才发现。

合并状态 `performance-three-rounds-complete` 只表示协议完整，不表示达到 #539/#305 的
产品预算。当前合并报告为 `urban-performance-comparison-v3`（v3 起记录三轮
已验证的共同 worker 数，执行配置可归属；v2 及更早报告不转换），显式记录统计
合并口径。

当前测量载荷为 `urban-performance-measurements-v3`（v3 起 workers 合法域为 1..=16；v2 固定单 worker 口径），旧计时载荷拒绝合并，不补写或转换。
`command_ns` 是该 tick 内六类公共生命周期调用（spawn/despawn/replace/leave/reserve/park）
的耗时之和，含实际调用后的拒绝，不含调用方延期；无调用时为 0。输入准备、排队、
完整快照、诊断断言及日志记账均在此计时外。`observation_ns` 累计 step 前的
`step_before` 采集、Active/intent 计数、红灯等待扫描，以及 step 后的信号采集、
事件/状态摘要和校验，两段均不包含 step 调用；三项仍不相加冒充整轮墙钟成本。
来源校验用于防止错用/混合记录，不证明保存的描述等于二进制的真实构建来源；正式取证
仍须冻结构建及输入，不能在运行中更改工作树。

观测摘要用固定大小的栈缓冲合批提交编码字节，字节顺序、长度前缀和摘要域不变；
不减少逐拍安全检查。编码专项对照仅供诊断，不替代真实城市前缀或正式性能取证。

## Mixed 的具体输入

每 tile 1000 个体、750 Active 和 250 个实际占位的 Parked；稳定身份为
`(tile, slot, incarnation)`，profile 比例为 30:60:10。离场、入场保持身份，Completed
原子替换使用请求序号派生的新 incarnation。

两名入场角色使用 slot 741/743，对应 `c08.bay1` 与 `c09.mixed`。先在实际入口臂上
取小于停车锚点的最大候选位置，分别为 `c08.e.out` 的 66500 mm 与 `c09.w.out`
的 32500 mm，路线使用该入口目录中的实际 occurrence。角色前方在同一臂上的候选
位置留空，其余 748 个 Active 按 `(位置层, edge key)` 补齐，保持每 tile 750 Active。

角色在边界 0 reserve；实际 arrival 后，park 在下一边界与预定最早边界两者的较晚者
执行。最早边界为观察窗口起点后 1/3 个量子，分别写入 `arrivals` 计划。暖机到达后
仍按真实 Active/Reserved 占用道路，直到观察期 park 成功；暖机到达不计作观察期事件。
两名角色保持原 incarnation，入场后保持 Parked；背景替换请求如实记录 `role-held`。
角色 slot 在显式 despawn/spawn 的相邻边界之间暂时 absent 时仍先按计划所有权记录
`role-held`，不会把 caller 推迟误报为 Runtime 命令失败。
车库 slot 880..899 仍在观察窗口起始的两个固定边界请求离场。
位置、身份、路线、目标和计划边界均从目录展开，不使用运行结果挑选输入。

每条离场/替换请求最多八次，间隔四个 528 ms 量子。未完成、入口受阻或角色保留均不
强制插车；耗尽请求继续记录。逐 tick 分开记录未来请求、可重试 pending 和 exhausted，
三者不算 live。70:30 检查以观察窗口内首次计划请求为分母，实际获准数另外报告。

## 证据及校验边界

- `resolved-plan.toml` 固定来源文件摘要、初态与有限请求；正式运行前提交展开规则和
  计划摘要，可从相同提交及 #542 制品重建完整文件。
- `ticks.jsonl` 用 `domain=road_motor_vehicle` 标识执行域，显式记录 `N_individual`、
  `N_active`、`N_intent`、`N_presented`、`N_aggregate_records`、`N_aggregate_equivalent`，
  后三项在本无界面路径恒为零；`intent_basis=exact_active_before_step`。
  另有 Parked/Completed、命令前后游标与状态/事件/命令摘要。
- `commands.jsonl` 保存实际请求、尝试次数、成功或拒绝、阻塞者稳定身份。
- `events.jsonl` 保存完整交通转移、生命周期和停车 arrival；每 tick 的 decision-batch
  摘要覆盖公共 Waiting/Conflict 决定的全部顺序、身份、锚点和结果，包括 NotEvaluated。
  typed ordinal 由同一份受检 LFCA 绑定，摘要不包含随机句柄或 Debug 文本。
  生命周期以 `phase=command/step` 区分命令边界和 step 提交。成功 park/leave/replace
  在命令提交时记录 before/after 状态及前后稳定身份；拒绝和 reserve 不产生生命周期
  变化事件。成功原子替换各累计一次出生和移除；拒绝及延期不增加这两项。
  `step_before` 仍在命令后采集，保持意图计数、红灯与跨 tile 观测的语义。
  命令观察边界为 `[warm_up,end)`；step 完成后的决策/事件用 `(warm_up,end]`，
  不计入最后一个暖机 step，但包含最后一个观察 step。
- `result.json` 记录窗口、实际提交、逐 tile 触发和完整快照摘要；初态、暖机结束、
  每观察周期末捕获完整快照。计划与结果均携带 `required_per_tile` 的冻结下限，
  对照逐 tile 的实际计数；载荷版本为 `urban-result-v5`（v5 起 `diagnostics.json`
  摘要纳入 `result.files` 完整性封套，worker 计数绑定证据封套），旧 v4 运行目录
  因缺该封套条目被拒绝、不转换；Failed 行不能通过 compare。
  `committed_role_commands`、`parking_arrivals`、`right_of_way` 和 `garage_exit_clearance` 保存具体身份及提交
  时序；计数不能替代缺失的角色准入、观察期入场链、让行因果或指定边界命令。
- `comparison.json` 由 compare 写到指定新路径，使用 `urban-comparison-v2`（v2 起
  `ComparedRun` 记录两臂各自 worker 数；v1 报告布局不含该字段，不转换），记录
  `case-pass`、`probe-match` 或 `performance-match`、case/scale、计划摘要、完成 tick 数，
  以及两个执行编号和两份 `result.json` 的 SHA256/字节数。原始运行文件保持不变；
  失败比较不生成通过报告。
  库的 `compare_runs` 返回同一结构，可用 `ComparisonReport::write` 保存。
- `diagnostics.json` 保存执行编号、环境和诊断耗时；step 范围只包围公共 step 调用，
  调用返回即停止计时，随后信号组采集与相位比较计入 observation，不含在 step 中。
  step 不含命令、oracle 或快照。它不是正式三轮性能协议，未测量的内存不填零。
  初始化完成后的受控执行或校验失败另留 `failure.json`。

结果包以完成初始化（世界安装、路线注册、初态校验和初始检查点）为起点。
初始化失败由库返回错误，CLI 输出错误并非零退出；本段接受目录只留下计划等部分
准备文件，不提供结构化初始化失败报告或半成品世界状态。缺少结果的目录不能通过 compare。

每次 `run` 根据进程号、执行开始时间和进程内序号生成非语义 `execution_id`。
compare 要求两份诊断记录中的编号存在且不同，用于拦截误复制同一次运行的结果目录。
编号不进入计划、`result.json` 或确定性摘要；缺少编号的旧记录需重新运行，不补号迁移。
不同编号只是防误用检查，不证明执行独立或抵御人为改写；两次各自创建新世界仍是
取证流程的责任，compare 的结果以此为前提。

每 tick 核对身份/生命周期守恒、实际停车 binding 和容量、Waiting membership 和容量、
按静态设施归属汇总显式绑定，与虚拟绑定一起核对每个设施的 reserved/occupied/total；
按路线向后展开的 Active 车身区间不重叠。通过 reservation acquire、passage clear 与
reservation release 的公开事件记录剩余 claim，核对资源区互斥和当前 reservation owner。
`urban-observation-v2` 状态摘要包含 live/absent 稳定 slot、车辆、遍历、Waiting、停车、
公开 Conflict reservation 和灯色，周期性完整快照补充隐藏权威状态。
失败命令逐次检查主体和游标，并对每类前八个候选调用中的第一次实际拒绝比较完整
快照；Ingress 的 `exclusive-occupied` / `virtual-full` 各自取证，预先的非 Active
拒绝不消耗这两类候选预算。未触发的原子性样本不宣称已测量。

Mixed 正式行要求每 tile 在观察窗口中实际完成跨 tile 行程、红灯前停车后过门、
至少一次 park 或 leave，离场与两类入场分别报告。完整观察期入场链由 Ingress 行验证。
小试发现输入错误必须更新冻结输入和原因后重新取证，
不以缩小必要触发数或无限增加重试换取通过。

`examples/boundary_probe.rs` 是从城市试跑缩小得到的单车诊断入口：在固定相位的边界
静止起步，核对下一次提交的位置和该次快照的恢复结果。它不调用 harness 调度或 oracle，
可供 Runtime 修复直接复用；step、恢复错误或游标越界均使程序非零退出：

```text
cargo +1.98.0 run -p laneflow-urban-harness --release --example boundary_probe -- <10k-artifact-directory>
```

## 跨层有限证据

`evidence ... adapter --presentation-config <config.json>` 显式选择宿主表现模式。
省略配置使用原 `FullValidation`。例如以下配置选择 live 个体的 10%，从稳定身份
排序后的第 53 项开始，每次成功采样移动 137 项，循环到列表开头：

```json
{"mode":"SelectedPresentation","selection":{"percent":10,"offset":53,"stride":137,"reverse":false}}
```

`percent` 为 0–100 的整数，分母是本次具有当前句柄的 live 个体数，数量向下取整。
`stride: 0` 保持稳定窗口；`reverse: true` 反转窗口内输入顺序。状态过滤由 Runtime
来源查询执行，因此 requested 与 extracted 不必相等。Selected 应用全部提取结果。
配对测量把 mode 改为 `FullValidationSelected`，保留相同 selection：它仍提取、转换
全量位姿，但只应用同一选择集合。`FullValidation` 配置不含 selection。

模式和完整选择配置进入 `evidence.json` 与每帧记录。计时分别记录选择收集、完整
提取调用（含校验/查询/采样/提交）、转换、绑定进出、应用及验证；`presentation_ns`
直接测选择到应用的整体时间。绑定记录 created/reused/hidden/shown/retired_bindings，
retired_bindings 包括真正移除与 replacement 后旧宿主身份退出。退出选择只隐藏，
仍保留绑定。`presentable` 由全量生命周期验证统计，不额外做全量 Spatial 采样。

完整链路性能使用 `plan ... --performance` 产生的原始窗口，并以 `evidence` 无限时
运行到末尾；汇总排除暖机，全部 tick 仍执行交通 oracle。`performance-case-pass`
只表示该轮完整执行，不代表已经完成三轮对照或产品认证。至少三个独立进程、相同
输入/计划和最终应用集合的比较及分配构建另行取证，probe 前缀不能替代。

独立取证构建：`--features allocation` 使用已有锁定的 `stats_alloc 0.1.10`
（crates.io，MIT/Apache-2.0）记录选择至应用区间的实际 alloc/realloc 和分配字节。
计数为进程 allocator 增量，不是峰值驻留内存；不混入随后验证、序列化和日志。
`--features pose-profiling` 在 stderr 输出成功批次的上下文/查重、来源查询、Spatial
采样与提交、Adapter 提交时间，以及输入、候选、输出、查重容器的 len/capacity。
Vec 元素大小用于容量字节账，Hash 表容量单位仍是元素；交换后分别计入各当前所有者，
逻辑复制字节为零不表示内存总线没有流量。插桩构建的墙钟不参与正常构建的性能结论。
默认 `adapter` 构建不含 allocator 或内部阶段插桩；三种构建须分别记录二进制摘要。
每帧 `applied_digest` 按稳定个体身份及实际 Transform 数值生成，配对运行须逐帧一致；
`requested` 表示提取需求（全量模式为 live 数），`host_selected` 另列宿主选择数量。

`adapter` feature 使用真实 `LaneFlowPlugin`、`LaneFlowSession` 和同根 `SpatialSession`。
每次 Bevy update 必须恰好提交一个固定步进且无积压。性能中的 Adapter `step` 是 Bevy
Step 阶段（含调度边界），headless `step` 是公共 `TrafficWorld::step` 调用，报告分别标明。
全量位姿提取、稳定身份排序与实体绑定、Transform 写入分别计时；逐帧校验和日志开销单列，
分位数不相加冒充帧时间。此入口未启动 GPU 渲染器，Transform 应用数不是实际绘制数。

```text
cargo +1.98.0 build -p laneflow-urban-harness --features adapter --release --locked
target/release/laneflow-urban-harness evidence <artifacts> <correctness-plan.toml> <new-output> adapter
target/release/laneflow-urban-harness variant <artifacts> <new-variant-directory>
target/release/laneflow-urban-harness transitions <artifacts> <variant-directory> MIXED-PEAK <new-output>
target/release/laneflow-urban-harness transitions <artifacts> <variant-directory> GARAGE-EGRESS <new-output>
```

功能矩阵是 10k 的 Mixed、Egress、Ingress、Waiting、Permissive、Uncontrolled 六行以及
100k Mixed 一行，消费已有完整 correctness 窗口。100k 是暖机 3712、观察 7424，共
11136 ticks。每帧全量提取 Active 和显式 Parked，排除虚拟 Parked 和 Completed。
10k 应用全量；100k 按 `(tile, slot, incarnation)` 升序取前 `floor(可表现数/10)` 个。
`N_presented` 包含完整提取集合；`applied` 单独表示实际写入 Transform 的数量。
失去表现资格的实体移除 Transform，停车期间保留身份和 Session 绑定；失败命令不创建
虚假位姿。`preview.json` 保存最终选集的真实 Transform 位置及数量口径，用于带标注预览。

增容变体按 base 的 ParkingFacility StableId128 字节序选择第一个 virtual-only 且容量
在 `(0,u32::MAX)` 内的设施，重用来源生成器，仅将其容量加 1，重新编译并生成 LFSD。
基线来源必须可逐字节重建，变体目录保存所选身份、两端容量、修订和所有输入摘要。
基线 #542 制品不被改写。

恢复与切换见证使用独立的 160-tick 固定 probe，两档各运行 Mixed 和 Egress。第 32 tick
保存实际 LFRS，以快照局部身份重绑调用方身份、队列和路线，并重建 Bevy 宿主。两次恢复
续跑与未中断运行的逐 tick 摘要和最终检查点相等。它验证保存点后的有限后缀，不替代完整
correctness 行。在线路径在边界 0 命令前 Prepare，真实步进并 pump 至边界 8 后提交；
必须实际增加命令游标。日志上限固定 128 MiB、滞后上限 8 ticks、每泵 4096 条记录，
报告实际日志占用。Adapter 在边界 8 维护暂停，先执行同修订换根，再拒绝错误
Spatial 配对，最后执行增容直移。两条路径分别重复两次，检查事件、世代、旧消费上下文
失效、句柄和停车 Reserved/Occupied 保留，并验证后续提交。更广协议和失败组合由
#538 按自身范围认证，本见证不声明该专项通过。

新增性能观测先冻结两个 `--probe-ticks 4096` 的 Mixed 计划，再执行四行串行整批：

```powershell
pwsh -NoProfile -File tools/laneflow-urban-harness/run-bounded.ps1 -Artifacts10k <10k-artifacts> -Artifacts100k <100k-artifacts> -Plan10k <10k-probe.toml> -Plan100k <100k-probe.toml> -Output <new-batch-directory>
```

构建和静态生成单独记录。脚本把启动、安装、运行、退出、日志和结果写出计入 600 秒
整批，按剩余时间和剩余行数分配进程预算，每行预留 10 秒写出，在完整提交边界停止。
每行最多 4096 ticks；硬终止、空样本、缺失结果或超时均失败。直接 `evidence ... --wall-ms N`
只限制一行，不能单独宣称整批达标。`batch.json` 和每行 `evidence.json` 记录实际范围、
停止原因、工具链、二进制/来源摘要、峰值驻留内存和各段分位数。此观测无三轮要求，也不
替代 #544 原性能记录或 #539 产品认证。运行前设置两项硬件/电源角色，并保持源工作树干净。

对同一份计划做固定前缀的性能对照时，可在单行有界调用中加 `--ticks N`：

```text
target/release/laneflow-urban-harness evidence <artifacts> <frozen-plan.toml> <new-output> headless --wall-ms 300000 --ticks 512
```

前缀只限制执行到哪个提交边界，不重新展开或改写需求计划；报告保留原始 `window`
和 `plan_digest`，另外记录 `prefix_ticks` 与实际 `target_ticks`。前缀必须在计划内，
且仅接受有墙钟上限的 Mixed probe，不能用来缩短 correctness 窗口。前后对照应预先
冻结同一前缀，并确认各轮均因 `tick-limit` 完成目标拍数；提前墙钟停止不能参与等长
样本比较。该入口本身不执行多轮合并，也不提供产品认证结论。
