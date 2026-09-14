# #285 复杂路口有界性能报告

该程序从正式 LFCA/catalog 进入一个 `TrafficWorld`，运行复用相同 policy、车型、
几何与生产绑定入口的参考路口网格。32/320 个参考单元分别容纳 10,000/100,000 辆车，
每单元 312/313 辆，初速为零；每单元有一个中心路口和两个外环汇合路口。64 段有限
路线保留真实机动门与 Waiting/Conflict 资源。它是独立参考负载，不命名为
`LF-SYNTH-v1` 或 `LF-CN-URBAN-v1`。

## 有界协议

每档运行一次 integrated process，最多 **4,096 个总 tick**，两档测量批次合计
**不超过 600 秒**，先到即停。没有额外仿真预热；统计来自实际冷启动观察窗口。
runner 将剩余批次时间按剩余档数均分，避免第一档耗尽时间；初始化、150 个零步
渲染准备帧、截图和结果收尾都在测量墙钟内。子进程为收尾预留最多 10 秒（短预算为
10%），在帧边界停止；父进程硬截止保护失去响应的执行。时间截断可能留下 backlog，
最终截图暂停仿真，报告保留未消化的量子数。

到时限正常退出且完成正数 tick、有效呈现与完整实际窗口校验，属于有效有界证据。
初始化失败、零 tick、校验失败、崩溃或被硬截止杀死不算可运行。实际 ticks、仿真时长、
帧数、进程与批次耗时、`tick-limit` / `time-limit` 均写入结果；不把短窗口写成
完成 4,096 ticks。不存在样本的帧类别输出 `null`。

运行前先提交来源；`freeze.ps1` 只接受干净提交和工作树外的新目录。它执行受控
release 构建、复制 Cargo 所报告的二进制，再用冻结生成器从检入配置生成两档输入。
构建、输入准备、摘要与环境预检在测量批次外；不可把其耗时混入组件样本。完整初态
快照、配置、LFCA/catalog、车辆／路线计划、seed、来源提交、二进制摘要、构建日志
与环境均保留。runner 和分析器拒绝输入或二进制变化、快照缺失及进程／结果绑定错误。
OS、硬件、驱动、电源及其变化如实记录，不作为该专项的认证条件。

```powershell
./research/issue-285-junction-scale/freeze.ps1 -OutputDirectory <new-evidence-dir>
./research/issue-285-junction-scale/run.ps1 -EvidenceDirectory <evidence-dir>
<evidence-dir>/bin/junction_scale_analyze.exe <evidence-dir>
```

`run.ps1 -MaxWallMilliseconds <1..600000>` 可进一步缩短整批预算。每个结果目录只运行
一批，分析器只创建新的 `summary.json`；失败记录保留，不覆盖或续跑旧长测。当前
协议使用 freeze/evidence/process v2，旧长测包不符合本协议。正式测量期间不重建
二进制、修改输入或并行运行其他重负载测量。

## 测量与验证

冻结输入使用 16 ms 步长，源配置信号周期为 72,048 ms；有界窗口不保证覆盖完整信号
周期。外层输入量子序列为 `[0,1,2,4,0]`，每帧最多推进两步，逐帧核对真实步数与
backlog。一万全量提取并应用 Transform；十万全量提取后按稳定调用方身份选取前
10,000 辆应用。1600×1000 offscreen renderer 使用无光照车辆方块，核对全部应用
对象进入视图；渲染耗时包括提取、提交与同步 GPU 完成，不是 GPU timestamp 或道路
美术成本。`junction_debug` 另提供可交互的参考路口展示。

每份报告区分 Runtime tick 与驱动、领域观测、pose 提取、Transform apply、同帧
Spatial+Adapter、renderer、完整帧及取证成本，报告 p50/p95/p99/max。
`laneflow_frame_without_evidence` 从同帧墙钟扣除实测 renderer 和取证子区间，保留
领域观测、ECS 调度及驱动；原始逐拍／逐帧样本保留。不同运行的 percentile 不相加。
资源持有、申请、事件及候选数量按实测报告；零活动不外推为领域失败或长期覆盖。

示例私有检查器继续检查实际每拍全部车辆的整数几何、重叠与净距、信号停止线、身份／
路线／生命周期、停车绑定、tick/time、领域事件顺序与因果、Conflict 排他性及观测
Projection；记录 12 类违规数、完整逐拍轨迹和决定／事件摘要。Adapter 领域观测逐拍
对照同一已提交世界；每帧全量 pose 对照直接 committed-state 输入的 exact Spatial
基线，全部应用 Transform 对照冻结身份映射。分析器按实际 ticks 与帧数核对覆盖行数。
首次失败保留 `.validation-failure.json` 和可捕获的失败快照，并使进程失败。
准入算法、全部拒绝数值理由、生命周期命令与失败分支的独立期望值继续复用
[有限正确性矩阵](../../docs/design/junction-observation-and-validation.md#5-有限正确性矩阵)，
本窗口不重写求解器，也不宣称重新覆盖所有矩阵场景。

runner 通过采样与退出后保留句柄记录工作集峰值、private bytes 采样峰值及进程 commit
峰值，包含校验器和收尾成本。这不是 Runtime 自有内存账本。该协议不运行额外
allocation、组件账本或三轮重放；不声称稳态零分配、重复性或产品硬件认证。

## 预算比较与验收

一万 Runtime p95 2 ms、十万 16 ms、Spatial+Adapter 4 ms，以及现行 p99/max
参考值继续展示；一万单步 LaneFlow 帧比较 6 ms p95，十万比较 16.667 ms observation
阈值。**超预算不阻止 #285 的可运行验收**。报告明确列出未测量项与样本范围，
不把参考短窗口称作产品认证；统一产品性能基线的验收仍归相应任务。

`summary.json` 是可复核结果，最终 PR 按修订后的 #285 验收、已有回归、普通 review、
required checks 与 Merge Queue 收口；不另加三轮长测或独立认证关卡。
