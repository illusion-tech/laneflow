# LF-CN-URBAN 无界面运行程序

#544 的第一段实现，消费 #542 的正式制品目录，经 `TrafficWorld` 公共 API 运行
`MIXED-PEAK`。库和命令行共用 `Artifacts`、`ResolvedPlan`、`Harness`，供 #545 后续复用。
设计依据为 [需求计划与无界面验证](../../docs/design/urban-demand-harness.md)。

## 本段验收

本段提供 Mixed 展开、实际初态、有限命令调度、逐 tick 校验和独立运行比较。
10k 正确性窗口是暖机 7656、观察 15312，共 22968 ticks。两次运行使用同一份预先
冻结的计划。fixture/短 probe 只验证实现；不提供正式 case 通过结论。

其他六套 case、全部 100k 正确性行、正式性能协议仍未交付，#544 保持开放。
不增加 Traffic Runtime、共享静态路网、Adapter 的公共接口。

10k 展开计划摘要保存在 [输入冻结记录](fixtures/v1/mixed-10k-plan.json)。当前城市试跑
在 tick 172 被 [#609](https://github.com/illusion-tech/laneflow/issues/609) 的 Runtime
边界起步缺陷阻塞，尚未取得正式 Mixed 通过证据。

## 运行

先用 `laneflow-urban-generator` 从 `examples/config/cn-urban.toml` 生成对应规模制品，
详见 [生成器](../laneflow-urban-generator/README.md)。以下命令从仓库根执行：

```text
cargo +1.98.0 build -p laneflow-urban-harness --release --locked
target/release/laneflow-urban-harness plan <artifact-directory> <plan.toml>
target/release/laneflow-urban-harness run <artifact-directory> <plan.toml> <run-a>
target/release/laneflow-urban-harness run <artifact-directory> <plan.toml> <run-b>
target/release/laneflow-urban-harness compare <run-a> <run-b>
```

Windows 可为可执行文件加 `.exe`。计划文件与结果目录必须是新路径，避免覆盖证据。
`plan ... --probe-ticks 128` 生成短试跑；probe 不得冒充正式验收。
计划经读取后重新核对固定展开规则及来源文件摘要，不接受手工删减事件或车辆。

## Mixed 的具体输入

每 tile 1000 个体、750 Active 和 250 个实际占位的 Parked；稳定身份为
`(tile, slot, incarnation)`，profile 比例为 30:60:10。离场、入场保持身份，Completed
原子替换使用请求序号派生的新 incarnation。

背景按设计的道路臂、位置层和路线优先级展开。两名入场角色使用 slot 741/743：
其初态位于 `c00.e.out` / `c00.n.out` 的 92000 mm，初始路线取在该臂结束的 junction
路线，以便作为 Completed 等待预先指定的观察窗口请求。角色占用普通 slot 和位置，
不额外生成个体。角色不会提前接受背景替换请求；这些请求如实记录 `role-held`。

每 tile 的组 74 在观察窗口第一次到期时，组内 i=1/i=3 分别选择
`c08.bay1` 显式入场和 `c09.mixed` 虚拟入场路线；其余请求保持七东三西。
替换成功后在同一边界 reserve，公共 arrival observation 后的下一边界 park。
两名角色入场后保持 Parked。车库 slot 880..899 在观察窗口起始的两个固定边界请求
离场。上述位置、身份、路线、目标和 due tick 全部写进展开计划，不根据运行结果选角色。

每条离场/替换请求最多八次，间隔四个 528 ms 量子。未完成、入口受阻或角色保留均不
强制插车；耗尽请求继续记录。逐 tick 分开记录未来请求、可重试 pending 和 exhausted，
三者不算 live。70:30 检查以观察窗口内首次计划请求为分母，实际获准数另外报告。

## 证据及校验边界

- `resolved-plan.toml` 固定来源文件摘要、初态与有限请求；正式运行前提交展开规则和
  计划摘要，可从相同提交及 #542 制品重建完整文件。
- `ticks.jsonl` 记录 live/Active/Parked/Completed、命令前后游标与状态/事件/命令摘要。
  唯一执行域为 `road_motor_vehicle`，`N_individual=live`；`N_intent=intent` 来源为
  `exact_active_before_step`。headless 的 presented、两个 aggregate 恒为零。
- `commands.jsonl` 保存实际请求、尝试次数、成功或拒绝、阻塞者稳定身份。
- `events.jsonl` 保存完整交通转移、生命周期和停车 arrival；每 tick 的 decision-batch
  摘要覆盖公共 Waiting/Conflict 决定的全部顺序、身份、锚点和结果，包括 NotEvaluated。
  typed ordinal 由同一份受检 LFCA 绑定，摘要不包含随机句柄或 Debug 文本。
- `result.json` 记录窗口、实际提交、逐 tile 触发和完整快照摘要；初态、暖机结束、
  每观察周期末捕获完整快照。Failed 行不能通过 compare。
- `diagnostics.json` 仅记录诊断耗时；step 范围只包围公共 step 调用，不含命令、oracle
  或快照。它不是正式三轮性能协议，未测量的内存不填零。运行失败另留 `failure.json`。

每 tick 核对身份/生命周期守恒、实际停车 binding 和容量、Waiting membership 和容量、
按路线向后展开的 Active 车身区间不重叠。通过 reservation acquire、passage clear 与
reservation release 的公开事件记录剩余 claim，核对资源区互斥和当前 reservation owner。
状态摘要包含车辆、遍历、Waiting、停车、
公开 Conflict reservation 和灯色，周期性完整快照补充隐藏权威状态。
失败命令逐次检查主体和游标，并对每类前八个候选调用中的第一次实际拒绝比较完整
快照；未触发的原子性样本不宣称已测量。

Mixed 正式行要求每 tile 在观察窗口中实际完成跨 tile 行程、红灯前停车后过门、
离场、显式入场、虚拟入场。小试发现输入错误必须更新冻结输入和原因后重新取证，
不以缩小必要触发数或无限增加重试换取通过。

`examples/boundary_probe.rs` 是从城市试跑缩小得到的单车诊断入口：在固定相位的边界
静止起步，核对下一次提交的位置和该次快照的恢复结果。它不调用 harness 调度或 oracle，
可供 Runtime 修复直接复用：

```text
cargo +1.98.0 run -p laneflow-urban-harness --release --example boundary_probe -- <10k-artifact-directory>
```
