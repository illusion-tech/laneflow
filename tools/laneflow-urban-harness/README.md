# LF-CN-URBAN 无界面运行程序

#544 的无界面实现消费 #542 的正式制品目录，经 `TrafficWorld` 公共 API 运行七套封闭
case。库和命令行共用 `Artifacts`、`ResolvedPlan`、`Harness`，供 #545 后续复用。
设计依据为 [需求计划与无界面验证](../../docs/design/urban-demand-harness.md)。

## 本段验收

当前实现提供七套计划展开、实际初态、有限命令调度、逐 tick 校验和独立运行比较。
10k 正确性窗口是暖机 7656、观察 15312，共 22968 ticks。两次运行使用同一份预先
冻结的计划。fixture/短 probe 只验证实现；不提供正式 case 通过结论。

`GARAGE-EGRESS` 使用 25% Active/75% Parked；`GARAGE-INGRESS` 固定满池和显式排他
拒绝；三类交通 case 使用目标 cell 的有限角色脉冲并直接聚合 Waiting/Conflict 决策；
`BOUNDARY-BURST` 在相邻提交边界执行 park/leave/replace 及独立 despawn/spawn。
所有路径都不增加 Traffic Runtime、共享静态路网或 Adapter 的公共接口。
实现和 fixture probe 不替代当前提交上的十四行正式正确性及六轮性能取证；完整证据完成前
#544 保持开放。

10k 展开计划摘要保存在 [输入冻结记录](fixtures/v2/mixed-10k-plan.json)。
`urban-demand-v2` 将停车角色从入口竞争改为预先安排的既有 Active 个体；旧 v1
结果保留为失败诊断，不能作为当前计划的通过证据。

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
窗口，分别记录调用方命令、`TrafficWorld::step` 和观测开销的 p50/p95/p99/max，实际
Active/intent 分布及进程 peak resident bytes 写入 `measurements.toml`。每档仍须按治理
流程启动三个新进程；单轮文件不代表三轮合并结论或产品预算认证。
三轮完成后仍通过 `compare` 入口做文件摘要、计划摘要和独立执行编号校验，并精确合并
三轮保存的观察窗口样本：

```text
target/release/laneflow-urban-harness compare <performance-a> <performance-b> <performance-c> <performance-comparison.toml>
```

合并状态 `performance-three-rounds-complete` 只表示协议完整，不表示达到 #539/#305 的
产品预算。

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
- `result.json` 记录窗口、实际提交、逐 tile 触发和完整快照摘要；初态、暖机结束、
  每观察周期末捕获完整快照。计划与结果均携带 `required_per_tile` 的冻结下限，
  对照逐 tile 的实际计数；载荷版本为 `urban-result-v3`，Failed 行不能通过 compare。
- `comparison.json` 由 compare 写到指定新路径，使用 `urban-comparison-v1`，记录
  `case-pass` 或 `probe-match`、case/scale、计划摘要、完成 tick 数，以及两个执行编号和
  两份 `result.json` 的 SHA256/字节数。原始运行文件保持不变；失败比较不生成通过报告。
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
快照；未触发的原子性样本不宣称已测量。

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
